//! Durable, content-addressed digest windows for Codex imports.
//! Successful results and recovery splits survive cancellation and restart.
//! Only a complete session is folded; the file cursor never hides failed pieces.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::distill::{
    digest_window, split_window, windows, CallBudget, DigestError, SessionOutcome,
    MAX_RESPLIT_DEPTH, MIN_WINDOW_CHARS,
};
use super::readers::RawSession;
use super::state::PersonaStateStore;
use super::types::{DigestObservation, SessionDigest};
use crate::memory::score::extract::ChatProvider;

const NAMESPACE: &str = "codex-digest-windows-v1";

#[derive(Serialize, Deserialize)]
enum SavedWindow {
    Complete(Vec<DigestObservation>),
    Split,
}

/// Digest cached pieces without re-sending completed content. `completed`
/// counts newly persisted pieces, including valid empty responses.
pub(super) async fn digest(
    provider: &dyn ChatProvider,
    session: &RawSession,
    budget: &CallBudget,
    store: &dyn PersonaStateStore,
    completed: &mut usize,
) -> Result<SessionOutcome> {
    let mut observations = Vec::new();
    for window in windows(session) {
        let mut stack = vec![(window, MAX_RESPLIT_DEPTH)];
        while let Some((piece, depth)) = stack.pop() {
            let mut hash = Sha256::new();
            hash.update(serde_json::to_vec(&session.source)?);
            hash.update(provider.name().as_bytes());
            hash.update(piece.as_bytes());
            let key = format!("{:x}", hash.finalize());
            let saved = store
                .get(NAMESPACE, &key)
                .await?
                .map(serde_json::from_value::<SavedWindow>)
                .transpose()?;
            match saved {
                Some(SavedWindow::Complete(obs)) => {
                    observations.extend(obs);
                    continue;
                }
                Some(SavedWindow::Split) => {}
                None => match digest_window(provider, session, &piece, budget).await {
                    Ok(obs) => {
                        store
                            .set(
                                NAMESPACE,
                                &key,
                                &serde_json::to_value(SavedWindow::Complete(obs.clone()))?,
                            )
                            .await?;
                        *completed += 1;
                        observations.extend(obs);
                        continue;
                    }
                    Err(DigestError::Unparseable(error)) => {
                        if depth == 0 || piece.chars().count() / 2 < MIN_WINDOW_CHARS {
                            return Err(DigestError::Unparseable(error).into());
                        }
                        store
                            .set(NAMESPACE, &key, &serde_json::to_value(SavedWindow::Split)?)
                            .await?;
                        *completed += 1;
                    }
                    Err(error) => return Err(error.into()),
                },
            }
            let parts = split_window(&piece, piece.chars().count() / 2);
            for part in parts.into_iter().rev() {
                stack.push((part, depth - 1));
            }
        }
    }
    Ok(SessionOutcome {
        digest: SessionDigest {
            source: session.source.clone(),
            observations,
        },
        windows_lost: 0,
    })
}

/// Share the extraction call ceiling with tree reduction. Once exhausted,
/// reduction uses the existing deterministic fallback and keeps every leaf.
pub(super) struct BudgetedSummariser<'a> {
    pub inner: &'a dyn crate::memory::tree::Summariser,
    pub budget: CallBudget,
}

#[async_trait::async_trait]
impl crate::memory::tree::Summariser for BudgetedSummariser<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn summarise(
        &self,
        inputs: &[crate::memory::tree::SummaryInput],
        context: &crate::memory::tree::SummaryContext<'_>,
    ) -> Result<crate::memory::tree::SummaryOutput> {
        if !self.budget.try_acquire() {
            return Ok(crate::memory::tree::summarise::fallback_summary(
                inputs,
                context.token_budget,
            ));
        }
        self.inner.summarise(inputs, context).await
    }
    async fn summarise_with_usage(
        &self,
        inputs: &[crate::memory::tree::SummaryInput],
        context: &crate::memory::tree::SummaryContext<'_>,
    ) -> Result<crate::memory::tree::SummaryCall> {
        if !self.budget.try_acquire() {
            return Ok(crate::memory::tree::SummaryCall {
                output: crate::memory::tree::summarise::fallback_summary(
                    inputs,
                    context.token_budget,
                ),
                ..Default::default()
            });
        }
        self.inner.summarise_with_usage(inputs, context).await
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
