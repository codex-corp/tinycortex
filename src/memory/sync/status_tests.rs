use chrono::{TimeZone, Utc};

use super::*;
use crate::memory::chunks::{upsert_chunks, Chunk, Metadata, SourceKind};

fn chunk(id: &str, source_id: &str, created_ms: i64) -> Chunk {
    let timestamp = Utc.timestamp_millis_opt(created_ms).unwrap();
    Chunk {
        id: id.into(),
        content: "content".into(),
        token_count: 1,
        seq_in_source: 0,
        created_at: timestamp,
        partial_message: false,
        metadata: Metadata {
            source_kind: SourceKind::Document,
            source_id: source_id.into(),
            path_scope: None,
            source_ref: None,
            owner: "test".into(),
            timestamp,
            time_range: (timestamp, timestamp),
            tags: Vec::new(),
        },
    }
}

#[test]
fn status_groups_provider_and_tracks_active_wave_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let config = MemoryConfig::new(temp.path());
    let now = 1_777_000_000_000i64;
    upsert_chunks(
        &config,
        &[
            chunk("a", "gmail:conn", now - 2_000),
            chunk("b", "gmail:conn", now - 1_000),
            chunk("c", "slack:conn", now - 600_000),
        ],
    )
    .unwrap();
    with_connection(&config, |connection| {
        connection.execute(
            "INSERT INTO mem_tree_chunk_embeddings (chunk_id, model_signature, vector, dim, created_at) VALUES ('a', 'test', X'00', 1, 0)",
            [],
        )?;
        connection.execute("UPDATE mem_tree_chunks SET lifecycle_status = 'dropped' WHERE id = 'c'", [])?;
        Ok(())
    }).unwrap();

    let statuses = list_sync_statuses_at(&config, now).unwrap();
    let gmail = statuses
        .iter()
        .find(|status| status.provider == "gmail")
        .unwrap();
    assert_eq!(gmail.chunks_synced, 2);
    assert_eq!(gmail.chunks_pending, 1);
    assert_eq!(gmail.batch_total, 2);
    assert_eq!(gmail.batch_processed, 1);
    assert_eq!(gmail.freshness, FreshnessLabel::Active);
    let slack = statuses
        .iter()
        .find(|status| status.provider == "slack")
        .unwrap();
    assert_eq!(slack.chunks_pending, 0);
    assert_eq!(slack.batch_total, 0);
    assert_eq!(slack.freshness, FreshnessLabel::Idle);
}
