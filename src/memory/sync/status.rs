//! Pull-based synchronization status derived from the authoritative chunk store.

use serde::{Deserialize, Serialize};

use crate::memory::chunks::with_connection;
use crate::memory::config::MemoryConfig;

const WAVE_WINDOW_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessLabel {
    Active,
    Recent,
    Idle,
}

impl FreshnessLabel {
    pub fn from_age_ms(last_chunk_at_ms: Option<i64>, now_ms: i64) -> Self {
        match last_chunk_at_ms {
            None => Self::Idle,
            Some(timestamp) => match now_ms.saturating_sub(timestamp) {
                age if age <= 30_000 => Self::Active,
                age if age <= 5 * 60_000 => Self::Recent,
                _ => Self::Idle,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySyncStatus {
    pub provider: String,
    pub chunks_synced: u64,
    pub chunks_pending: u64,
    pub batch_total: u64,
    pub batch_processed: u64,
    pub last_chunk_at_ms: Option<i64>,
    pub freshness: FreshnessLabel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusListResponse {
    pub statuses: Vec<MemorySyncStatus>,
}

pub fn list_sync_statuses(config: &MemoryConfig) -> anyhow::Result<Vec<MemorySyncStatus>> {
    list_sync_statuses_at(config, chrono::Utc::now().timestamp_millis())
}

fn list_sync_statuses_at(
    config: &MemoryConfig,
    now_ms: i64,
) -> anyhow::Result<Vec<MemorySyncStatus>> {
    with_connection(config, |connection| {
        let mut statement = connection.prepare(
            "WITH provider_chunks AS ( \
                SELECT CASE WHEN INSTR(source_id, ':') > 0 \
                    THEN SUBSTR(source_id, 1, INSTR(source_id, ':') - 1) \
                    ELSE source_kind END AS provider, \
                    created_at_ms, \
                    CASE WHEN EXISTS (SELECT 1 FROM mem_tree_chunk_embeddings e WHERE e.chunk_id = c.id) \
                      OR c.lifecycle_status = 'dropped' \
                      OR EXISTS (SELECT 1 FROM mem_tree_chunk_reembed_skipped s WHERE s.chunk_id = c.id) \
                    THEN 1 ELSE 0 END AS resolved, timestamp_ms \
                FROM mem_tree_chunks c \
             ), provider_max AS ( \
                SELECT provider, MAX(created_at_ms) AS max_created FROM provider_chunks GROUP BY provider \
             ), provider_pending AS ( \
                SELECT p.provider, SUM(CASE WHEN p.resolved = 0 AND p.created_at_ms >= m.max_created - ?1 THEN 1 ELSE 0 END) AS pending \
                FROM provider_chunks p JOIN provider_max m ON p.provider = m.provider GROUP BY p.provider \
             ), wave_anchors AS ( \
                SELECT p.provider, MIN(p.created_at_ms) AS anchor \
                FROM provider_chunks p JOIN provider_max m ON p.provider = m.provider \
                JOIN provider_pending pp ON p.provider = pp.provider \
                WHERE pp.pending > 0 AND p.created_at_ms >= m.max_created - ?1 GROUP BY p.provider \
             ) SELECT p.provider, COUNT(*) AS chunks_synced, \
                SUM(CASE WHEN p.resolved = 0 THEN 1 ELSE 0 END) AS chunks_pending, \
                SUM(CASE WHEN w.anchor IS NOT NULL AND p.created_at_ms >= w.anchor THEN 1 ELSE 0 END) AS batch_total, \
                SUM(CASE WHEN w.anchor IS NOT NULL AND p.created_at_ms >= w.anchor AND p.resolved = 1 THEN 1 ELSE 0 END) AS batch_processed, \
                MAX(p.timestamp_ms) AS last_chunk_at_ms \
             FROM provider_chunks p LEFT JOIN wave_anchors w ON p.provider = w.provider \
             GROUP BY p.provider ORDER BY last_chunk_at_ms DESC",
        )?;
        let rows = statement.query_map([WAVE_WINDOW_MS], |row| {
            let last_chunk_at_ms = row.get(5)?;
            Ok(MemorySyncStatus {
                provider: row.get(0)?,
                chunks_synced: nonnegative(row.get(1)?),
                chunks_pending: nonnegative(row.get(2)?),
                batch_total: nonnegative(row.get(3)?),
                batch_processed: nonnegative(row.get(4)?),
                last_chunk_at_ms,
                freshness: FreshnessLabel::from_age_ms(last_chunk_at_ms, now_ms),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

fn nonnegative(value: i64) -> u64 {
    value.max(0) as u64
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
