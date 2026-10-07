//! Persona pipeline orchestration (doc 06 §6.5–§6.8): drives readers → digest →
//! facet-tree reduce → compile, with incremental cursors and run budgets.
//!
//! Two modes (§6.7): `Backfill` walks everything oldest-first so trees fold
//! chronologically; `Incremental` skips files/repos whose cursor is unchanged.
//! Both honour the run budget (max sessions / LLM calls); provider cost is
//! bounded by the provider's own per-run ceiling. Because evidence ids are
//! content-addressed, re-runs dedupe naturally — cursors are a fast-skip, not a
//! correctness gate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use super::compile::{write_pack, PackInputs};
use super::config::PersonaConfig;
use super::distill::{digest_session, is_budget_exhausted, CallBudget, SessionOutcome};
use super::readers::{claude_code, codex, instruction, RawSession};
use super::reduce::{fold_digest, fold_directives, seal_and_collect, FacetAsks, ReduceState};
use super::state::PersonaStateStore;
use super::state::{self, file_key, file_unchanged, record_file};
use super::types::PersonaFacet;
use crate::memory::config::MemoryConfig;
use crate::memory::score::extract::ChatProvider;
use crate::memory::tree::Summariser;
use futures::stream::{self, StreamExt};

/// Run mode (§6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Walk everything, oldest-first.
    Backfill,
    /// Cursor-forward only: skip unchanged files/repos.
    Incremental,
}

impl RunMode {
    fn as_str(self) -> &'static str {
        match self {
            RunMode::Backfill => "backfill",
            RunMode::Incremental => "incremental",
        }
    }
}

/// What a run did — printed by the harness `status`/run output.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunReport {
    /// The mode that ran.
    pub mode: String,
    /// Transcript/instruction files discovered.
    pub files_seen: usize,
    /// Sessions/batches actually digested.
    pub sessions_processed: usize,
    /// Files skipped because their cursor was unchanged.
    pub sessions_skipped: usize,
    /// Instruction-file rules folded (verbatim T0).
    pub directives_folded: usize,
    /// Total evidence units extracted this run.
    pub evidence_units: usize,
    /// Digest calls that produced at least one observation.
    pub digests: usize,
    /// Sessions whose digest hit a hard **provider** failure — transport or auth
    /// (cursor NOT committed; the whole session is retried next run). A spent call
    /// budget is a clean checkpoint reported in [`Self::budget_hit`], not here, and
    /// truncated/unparseable windows are recovered or counted in
    /// [`Self::windows_lost`], not here.
    pub sessions_failed: usize,
    /// Newly persisted Codex digest pieces or recovery splits; resumable progress.
    pub checkpoints_advanced: usize,
    /// Safe, bounded per-session failure information.
    pub failures: Vec<SessionFailure>,
    /// Codex files excluded by configured project provenance.
    pub sessions_excluded: usize,
    /// Recovery leaf sub-windows dropped because their digest stayed unparseable
    /// even after truncation-recovery re-splitting (one truncated 12k window can
    /// contribute several, so this counts sub-windows, not top-level windows). A
    /// session's cursor is committed on this path only when the run produced
    /// observations elsewhere (near-deterministic at temperature 0.0); a run that
    /// lost windows and yielded nothing withholds instead — see
    /// [`Self::systemic_digest_failure`]. Surfaced so the drop is never silent.
    pub windows_lost: usize,
    /// Observations distilled.
    pub observations: usize,
    /// Per-facet observation counts (facet wire-string → count).
    pub facet_counts: BTreeMap<String, usize>,
    /// True when a run budget stopped the run early (checkpointed).
    pub budget_hit: bool,
    /// True when every digested session yielded zero observations *and* at least
    /// one window was lost — the signature of a systemic provider failure (a
    /// wrong/non-instruct model, refusal mode, a proxy returning prose). The
    /// fully-lost sessions' cursors are then **withheld** so the backlog is
    /// retried once the cause is fixed, rather than silently committed and skipped.
    pub systemic_digest_failure: bool,
    /// Path of the compiled pack, if written.
    pub pack_path: Option<String>,
}

/// Sanitised failure metadata; never contains transcript text or provider errors.
#[derive(Debug, Clone, Serialize)]
pub struct SessionFailure {
    /// Stable machine-readable cause.
    pub code: String,
    /// Opaque content-derived correlation identifier.
    pub session_id: String,
    /// Safe remediation text.
    pub summary: String,
}

fn failure(session: &RawSession, error: &anyhow::Error) -> SessionFailure {
    use sha2::{Digest, Sha256};
    let is_parse = error
        .downcast_ref::<super::distill::DigestError>()
        .is_some_and(|e| matches!(e, super::distill::DigestError::Unparseable(_)));
    SessionFailure {
        code: if is_parse {
            "parse_failure"
        } else {
            "provider_failure"
        }
        .into(),
        session_id: format!(
            "{:x}",
            Sha256::digest(session.source.source_id().as_bytes())
        ),
        summary: if is_parse {
            "The digest response was invalid. Check the summarisation provider and retry."
        } else {
            "The summarisation call failed. Check provider access and retry."
        }
        .into(),
    }
}

/// A run's session-count budget. The provider-call ceiling (`max_llm_calls`) is
/// enforced separately and precisely by [`CallBudget`] — per actual call, so
/// multi-window sessions and truncation-recovery re-tries all count — rather than
/// estimated here at one call per session.
struct Budget {
    max_sessions: usize,
    sessions: usize,
}

impl Budget {
    fn from(cfg: &PersonaConfig) -> Self {
        Self {
            max_sessions: cfg.run_budget.max_sessions,
            sessions: 0,
        }
    }
    /// True once the run has digested its full session allowance (stop cleanly).
    fn exhausted(&self) -> bool {
        self.sessions >= self.max_sessions
    }
    fn charge(&mut self) {
        self.sessions += 1;
    }
}

/// Digest-loop guards threaded through the ingest sources: the shared
/// provider-call ceiling and the deferred cursor commits awaiting the run-level
/// systemic-failure check in [`Pipeline::run`].
struct DigestGuards {
    call_budget: CallBudget,
    /// Commits for fully-lost sessions (zero observations, ≥1 window dropped),
    /// applied or withheld once the whole run's outcome is known.
    deferred: Vec<(String, serde_json::Value)>,
}

/// Git-history ingestion (`ingest_git`), feature-gated and split into a sibling
/// module to keep this file within the repo's size norm.
#[cfg(feature = "git-diff")]
#[path = "pipeline_git.rs"]
mod pipeline_git;

/// The pipeline binds the workspace, config, provider, summariser, and state
/// store; `run` executes one pass.
pub struct Pipeline<'a> {
    /// Memory workspace config (tree store, content root).
    pub config: &'a MemoryConfig,
    /// Persona config (roots, models, budgets, asks).
    pub persona: &'a PersonaConfig,
    /// Chat provider for the digest map step.
    pub provider: &'a dyn ChatProvider,
    /// Summariser for the facet-tree folds.
    pub summariser: &'a dyn Summariser,
    /// Incremental-run state store.
    pub store: &'a dyn PersonaStateStore,
}

impl Pipeline<'_> {
    /// Execute one pass in `mode`, writing `persona/PERSONA.md`.
    pub async fn run(&self, mode: RunMode) -> Result<RunReport> {
        let asks = self.persona.asks();
        let mut state = ReduceState::default();
        let mut budget = Budget::from(self.persona);
        // Shared provider-call ceiling (so transcripts and git history draw from
        // the same `max_llm_calls`) plus the deferred fully-lost cursor commits,
        // resolved after all sources run.
        let mut guards = DigestGuards {
            call_budget: CallBudget::new(self.persona.run_budget.max_llm_calls as usize),
            deferred: Vec::new(),
        };
        let mut report = RunReport {
            mode: mode.as_str().to_string(),
            ..Default::default()
        };

        // 1. Instruction files (no LLM) — highest-confidence T0 directives.
        // Always re-read the full set every run (cheap, no LLM) so edits and
        // removals are reflected — directives are rebuilt fresh, never seeded
        // from a stale persisted copy.
        self.ingest_instructions(&mut state, &mut report).await?;

        // 2. Transcripts (Claude Code + Codex) — the digest map step.
        self.ingest_transcripts(
            mode,
            &asks,
            &mut state,
            &mut budget,
            &mut guards,
            &mut report,
        )
        .await?;

        // 3. Git history (feature-gated).
        #[cfg(feature = "git-diff")]
        self.ingest_git(
            mode,
            &asks,
            &mut state,
            &mut budget,
            &mut guards,
            &mut report,
        )
        .await?;

        // Resolve the deferred (zero-observation, ≥1-window-lost) cursor commits.
        // If the run produced observations anywhere, those sessions are localized
        // permanent failures — commit them so the queue advances (no starvation).
        // If the run yielded *nothing* despite dropping windows, treat it as a
        // systemic provider failure: withhold every deferred commit so the backlog
        // is retried once the cause is fixed, and flag it loudly.
        if !guards.deferred.is_empty() {
            if report.observations > 0 {
                for (key, value) in &guards.deferred {
                    // A single failed cursor write must not discard the whole run's
                    // reduce output (the pack, written below): a cursor is a
                    // fast-skip, not a correctness gate, so the session simply
                    // re-digests next run. Log and carry on rather than `?`-abort.
                    if let Err(e) = self.store.set(state::NAMESPACE, key, value).await {
                        log::warn!(
                            "[persona] deferred cursor commit failed for {key}; \
                             it will be re-digested next run: {e:#}"
                        );
                    }
                }
            } else {
                report.systemic_digest_failure = true;
                log::error!(
                    "[persona] {} session(s) digested to zero observations with {} window(s) \
                     lost and no observations anywhere this run — likely a systemic provider \
                     failure (wrong/non-instruct model, refusal, or a proxy returning prose). \
                     Withholding {} cursor commit(s) so the backlog is retried, not skipped.",
                    guards.deferred.len(),
                    report.windows_lost,
                    guards.deferred.len(),
                );
            }
        }

        // `budget_hit` is recorded precisely where a ceiling actually stops work:
        // the selection loop when it drops a pending session, and the call-budget
        // checkpoint on `BudgetExhausted`. It is deliberately NOT re-derived from
        // `budget.exhausted()` here — that is true whenever the run merely filled
        // its session allowance exactly, which drops nothing and is not a hit.

        // 4. Seal facet trees + compile the pack.
        let bounded = super::checkpoint::BudgetedSummariser {
            inner: self.summariser,
            budget: guards.call_budget.clone(),
        };
        let summariser = if self.persona.codex_project_root.is_some() {
            &bounded as &dyn Summariser
        } else {
            self.summariser
        };
        let bodies = seal_and_collect(self.config, &asks, summariser).await?;
        let pack_path = self.compile_and_write(bodies, &state)?;
        report.pack_path = Some(pack_path.display().to_string());
        for (facet, n) in &state.counts {
            report.facet_counts.insert(facet.as_str().to_string(), *n);
        }
        Ok(report)
    }

    /// Re-assemble the pack from the current facet-tree roots without any LLM
    /// calls (the `compile` subcommand).
    pub fn compile_only(&self) -> Result<PathBuf> {
        use super::reduce::strip_frontmatter;
        use crate::memory::tree::flavoured::compile_flavoured_root;
        use crate::memory::tree::TreeFactory;

        let asks = self.persona.asks();
        let mut bodies = BTreeMap::new();
        // Directives are reconstructed verbatim from the persisted store; the
        // rest of the reduction starts empty.
        let mut state = ReduceState {
            directives: super::compile::read_directives(self.config),
            ..Default::default()
        };
        for facet in PersonaFacet::ALL {
            let factory = TreeFactory::flavoured(facet.tree_scope(), asks.ask(facet));
            let tree = factory.get_or_create(self.config)?;
            let markdown = compile_flavoured_root(self.config, &tree.id)?;
            let body = strip_frontmatter(&markdown);
            if !body.trim().is_empty() {
                bodies.insert(facet, body);
                // Use leaves-folded as a rough observation count proxy.
                *state.counts.entry(facet).or_default() += tree.root_id.is_some() as usize;
            }
        }
        self.compile_and_write(bodies, &state)
    }

    /// Build [`PackInputs`] and write the pack.
    fn compile_and_write(
        &self,
        bodies: BTreeMap<PersonaFacet, String>,
        state: &ReduceState,
    ) -> Result<PathBuf> {
        // Persist the verbatim directives so a later `compile` can rebuild the
        // Directives section without re-reading instruction files.
        if !state.directives.is_empty() {
            super::compile::write_directives(self.config, &state.directives)?;
        }
        let mut inputs = PackInputs::new(self.persona.identity.clone());
        inputs.facet_bodies = bodies;
        inputs.directives = state.directives.clone();
        inputs.counts = state.counts.clone();
        inputs.scopes = state.scopes.iter().map(|(k, v)| (*k, v.len())).collect();
        inputs.per_facet_budget = self.persona.per_facet_token_budget;
        inputs.total_budget_max = self.persona.total_token_budget;
        write_pack(self.config, &inputs)
    }

    async fn ingest_instructions(
        &self,
        state: &mut ReduceState,
        report: &mut RunReport,
    ) -> Result<()> {
        let files = instruction::discover(
            &self.persona.project_roots,
            &self.persona.global_instruction_files,
        );
        for file in files {
            report.files_seen += 1;
            let key = file_key("instruction_file", &file.path);
            let bytes = match std::fs::read(&file.path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let sha = instruction::content_sha(&bytes);
            // Re-read every run so removed/edited rules drop out of the freshly
            // rebuilt directive set (dedup keeps repeats collapsed). No LLM cost.
            let session = match instruction::read_file(&file) {
                Ok(s) => s,
                Err(_) => continue,
            };
            report.evidence_units += session.evidence.len();
            report.directives_folded += session.evidence.len();
            fold_directives(&session.evidence, state);
            state::record_watermark(self.store, &key, &sha).await?;
        }
        Ok(())
    }
}

/// A read-but-not-yet-digested session plus an optional state commit (cursor or
/// watermark) applied only after the session is folded — so a budget-truncated
/// session is re-processed on the next run.
struct Pending {
    session: RawSession,
    commit: Option<(String, serde_json::Value)>,
}

/// Dispatch to the right transcript reader by source-kind tag.
fn read_transcript(kind: &str, path: &Path) -> Result<RawSession> {
    match kind {
        "claude_code" => claude_code::read_session(path),
        "codex" => codex::read_session(path),
        other => anyhow::bail!("unknown transcript kind {other}"),
    }
}

/// File mtime in millis for oldest-first ordering (0 when unknown).
fn file_mtime_ms(path: &Path) -> i64 {
    state::FileCursor::of(path).map(|c| c.mtime_ms).unwrap_or(0)
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

#[path = "pipeline_ingest.rs"]
mod ingest;
