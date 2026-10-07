//! Transcript selection, checkpointed digestion and chronological folding.
use super::*;

impl Pipeline<'_> {
    pub(super) async fn ingest_transcripts(
        &self,
        mode: RunMode,
        asks: &FacetAsks,
        state: &mut ReduceState,
        budget: &mut Budget,
        guards: &mut DigestGuards,
        report: &mut RunReport,
    ) -> Result<()> {
        let mut files: Vec<(PathBuf, &'static str)> = Vec::new();
        if let Some(root) = &self.persona.claude_code_root {
            for p in claude_code::discover(root) {
                files.push((p, "claude_code"));
            }
        }
        if let Some(root) = &self.persona.codex_root {
            for p in codex::discover(root) {
                files.push((p, "codex"));
            }
        }
        // Oldest-first for chronological folding.
        files.sort_by_key(|(p, _)| file_mtime_ms(p));

        // Read (cheap I/O, serial) into a pending list, then digest concurrently
        // + fold serially below. Cursors are committed only AFTER a session is
        // digested, so a budget-truncated file is re-processed on resume.
        let mut pending: Vec<Pending> = Vec::new();
        for (path, kind) in files {
            if let Some(project) = self
                .persona
                .codex_project_root
                .as_ref()
                .filter(|_| kind == "codex")
            {
                let scope = super::super::scope::transcript_scope(&path, kind)
                    .ok()
                    .flatten();
                if !super::super::scope::matches_project(scope.as_deref(), project) {
                    report.sessions_excluded += 1;
                    continue;
                }
            }
            report.files_seen += 1;
            let key = file_key(kind, &path);
            if mode == RunMode::Incremental && file_unchanged(self.store, &key, &path).await? {
                report.sessions_skipped += 1;
                continue;
            }
            // Do not repeatedly parse gigabytes of untouched history on every
            // five-session pass. Account for the backlog using provenance and
            // cursors, but parse only the selected batch's evidence.
            if pending.len() >= self.persona.run_budget.max_sessions {
                report.budget_hit = true;
                continue;
            }
            let session: RawSession = match read_transcript(kind, &path) {
                Ok(s) => s,
                Err(_) => {
                    report.sessions_failed += 1;
                    if report.failures.len() < 10 {
                        use sha2::{Digest, Sha256};
                        report.failures.push(SessionFailure {
                            code: "transcript_unreadable".into(),
                            session_id: format!("{:x}", Sha256::digest(key.as_bytes())),
                            summary: "The transcript could not be read. Check the file and retry."
                                .into(),
                        });
                    }
                    continue;
                }
            };
            if kind == "codex"
                && self
                    .persona
                    .codex_project_root
                    .as_ref()
                    .is_some_and(|project| {
                        !super::super::scope::matches_project(
                            session.source.scope.as_deref(),
                            project,
                        )
                    })
            {
                report.sessions_excluded += 1;
                continue;
            }
            if session.is_empty() {
                // Nothing to digest — record the cursor now so we don't re-read.
                record_file(self.store, &key, &path).await?;
                report.sessions_skipped += 1;
                continue;
            }
            report.evidence_units += session.evidence.len();
            let commit = state::FileCursor::of(&path)
                .and_then(|c| serde_json::to_value(c).ok())
                .map(|v| (key, v));
            pending.push(Pending { session, commit });
        }
        self.digest_and_fold(pending, asks, state, budget, guards, report)
            .await
    }

    /// Digest the `pending` sessions concurrently (the network-bound map step)
    /// and fold the results serially into the facet trees (SQLite writes must
    /// stay serial). Order is preserved (`buffered`, not `buffer_unordered`) so
    /// trees fold oldest-first. Selection honours the shared run budget; a
    /// selected session's cursor is committed after it is folded, except a
    /// fully-lost session (zero observations, ≥1 window dropped) whose commit is
    /// pushed to `deferred` for the run-level systemic check in [`Self::run`].
    pub(super) async fn digest_and_fold(
        &self,
        pending: Vec<Pending>,
        asks: &FacetAsks,
        state: &mut ReduceState,
        budget: &mut Budget,
        guards: &mut DigestGuards,
        report: &mut RunReport,
    ) -> Result<()> {
        // Select within the session-count budget up front; the provider-call
        // budget is enforced per call inside `digest_session`.
        let mut selected: Vec<Pending> = Vec::new();
        for p in pending {
            if budget.exhausted() {
                report.budget_hit = true;
                break;
            }
            budget.charge();
            selected.push(p);
        }
        if selected.is_empty() {
            return Ok(());
        }
        let concurrency = self.persona.digest_concurrency.max(1);
        let results: Vec<(Result<SessionOutcome>, usize)> = stream::iter(selected.iter())
            .map(|p| async {
                let mut completed = 0;
                let outcome =
                    if p.session.source.kind == super::super::types::PersonaSourceKind::Codex {
                        super::super::checkpoint::digest(
                            self.provider,
                            &p.session,
                            &guards.call_budget,
                            self.store,
                            &mut completed,
                        )
                        .await
                    } else {
                        digest_session(self.provider, &p.session, &guards.call_budget).await
                    };
                (outcome, completed)
            })
            .buffered(concurrency)
            .collect()
            .await;
        for (p, (result, completed)) in selected.iter().zip(results) {
            report.checkpoints_advanced += completed;
            let outcome = match result {
                Ok(o) => o,
                Err(e) => {
                    // Non-committable, cursor NOT committed so the session is
                    // re-attempted next run. Two shapes reach here, neither a
                    // silent drop: the run's call budget was spent mid-session (a
                    // clean checkpoint), or a hard provider failure
                    // (transport/auth). Truncated/unparseable windows never reach
                    // here — they are recovered or dropped-and-counted inside
                    // `digest_session`.
                    if is_budget_exhausted(&e) {
                        report.budget_hit = true;
                    } else {
                        log::warn!("[persona] digest failed; cursor retained for retry");
                        report.sessions_failed += 1;
                        if report.failures.len() < 10 {
                            report.failures.push(failure(&p.session, &e));
                        }
                    }
                    continue;
                }
            };
            report.sessions_processed += 1;
            report.windows_lost += outcome.windows_lost;
            let digest = outcome.digest;
            let session_observations = digest.observations.len();
            if !digest.is_empty() {
                report.digests += 1;
                report.observations += session_observations;
                let bounded = super::super::checkpoint::BudgetedSummariser {
                    inner: self.summariser,
                    budget: guards.call_budget.clone(),
                };
                let summariser = if self.persona.codex_project_root.is_some() {
                    &bounded as &dyn Summariser
                } else {
                    self.summariser
                };
                fold_digest(self.config, &digest, asks, summariser, state).await?;
            }
            let Some(commit) = &p.commit else { continue };
            if session_observations == 0 && outcome.windows_lost > 0 {
                // Yielded nothing but lost ≥1 window. In isolation this is a
                // localized permanent-garbage window that should commit so the
                // queue advances; run-wide it can instead be the symptom of a
                // systemic provider failure. Defer the commit — `run` applies it
                // only if the run produced observations somewhere, otherwise it
                // withholds and flags rather than silently skipping the backlog.
                guards.deferred.push(commit.clone());
            } else if let Err(e) = self.store.set(state::NAMESPACE, &commit.0, &commit.1).await {
                // Committed now that the session is folded: a recovered or
                // genuinely-empty (zero-loss) session reproduces on re-run, so
                // committing loses nothing. As on the deferred path, a failed
                // write is logged and skipped rather than `?`-aborting the run and
                // discarding the pack — the cursor is a fast-skip, so the session
                // simply re-digests next run.
                log::warn!(
                    "[persona] cursor commit failed for {}; it will be re-digested next run: {e:#}",
                    commit.0
                );
            }
        }
        Ok(())
    }
}
