use super::*;
use crate::memory::tree::{store::get_buffer, ConcatSummariser};

#[tokio::test]
async fn direct_summary_lands_at_l1_without_creating_chunks() {
    let temp = tempfile::tempdir().unwrap();
    let config = MemoryConfig::new(temp.path());
    let tree = TreeFactory::source("github:org/repo")
        .get_or_create(&config)
        .unwrap();
    let now = Utc::now();
    let outcome = ingest_summary(
        &config,
        &tree,
        SummaryIngestInput {
            content: "summary".into(),
            token_count: 2,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: now,
            time_range_end: now,
            score: 0.5,
            child_labels: vec!["100_issue-1".into()],
            child_basenames: Vec::new(),
        },
        &ConcatSummariser,
    )
    .await
    .unwrap();
    assert_eq!(crate::memory::chunks::count_chunks(&config).unwrap(), 0);
    let summary = store::get_summary(&config, &outcome.summary_id)
        .unwrap()
        .unwrap();
    assert_eq!(summary.level, 1);
    assert_eq!(summary.child_ids, vec!["100_issue-1"]);
    assert_eq!(get_buffer(&config, &tree.id, 1).unwrap().item_ids.len(), 1);
}
