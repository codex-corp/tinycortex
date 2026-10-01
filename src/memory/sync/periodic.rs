//! Host-neutral periodic synchronization cadence policy.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::memory::sources::{MemorySourceEntry, SourceKind};
use crate::memory::sync::audit::SyncAuditEntry;

pub const DEFAULT_SYNC_INTERVAL_SECS: u64 = 24 * 60 * 60;

pub fn effective_interval_secs(configured: Option<u64>) -> Option<u64> {
    match configured {
        Some(0) => None,
        Some(seconds) => Some(seconds),
        None => Some(DEFAULT_SYNC_INTERVAL_SECS),
    }
}

pub fn due_workspace_sources(
    sources: &[MemorySourceEntry],
    audit: &[SyncAuditEntry],
    configured_interval_secs: Option<u64>,
    now: DateTime<Utc>,
) -> Vec<MemorySourceEntry> {
    let Some(interval) = effective_interval_secs(configured_interval_secs) else {
        return Vec::new();
    };
    let successes = last_success_by_source(audit);
    sources
        .iter()
        .filter(|source| source.enabled && is_periodic_workspace_kind(&source.kind))
        .filter(|source| {
            successes
                .get(&source.id)
                .is_none_or(|last| elapsed_since(*last, now) >= Duration::from_secs(interval))
        })
        .cloned()
        .collect()
}

fn is_periodic_workspace_kind(kind: &SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::GithubRepo | SourceKind::Folder | SourceKind::RssFeed | SourceKind::WebPage
    )
}

fn last_success_by_source(audit: &[SyncAuditEntry]) -> HashMap<String, DateTime<Utc>> {
    let mut result = HashMap::new();
    for entry in audit.iter().filter(|entry| entry.success) {
        if !matches!(
            entry.source_kind.as_str(),
            "github_repo" | "folder" | "rss_feed" | "web_page"
        ) {
            continue;
        }
        result
            .entry(entry.source_id.clone())
            .and_modify(|current: &mut DateTime<Utc>| *current = (*current).max(entry.timestamp))
            .or_insert(entry.timestamp);
    }
    result
}

fn elapsed_since(timestamp: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    Duration::from_secs((now - timestamp).num_seconds().max(0) as u64)
}

#[cfg(test)]
#[path = "periodic_tests.rs"]
mod tests;
