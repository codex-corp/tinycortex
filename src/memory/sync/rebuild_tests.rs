use super::*;

#[test]
fn coverage_is_incremental_and_ignores_source_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = MemoryConfig::new(temp.path());
    let custom_root = temp.path().join("custom-content");
    config.content_root = Some(custom_root.clone());
    let root = custom_root.join("raw/github-com-org-repo/issues");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("100_one.md"), "one").unwrap();
    std::fs::write(root.join("200_two.md"), "two").unwrap();
    std::fs::write(root.parent().unwrap().join("_source.md"), "metadata").unwrap();

    let first = raw_coverage(&config, "github:org/repo", "github.com/org/repo").unwrap();
    assert_eq!(first.total, 2);
    assert_eq!(first.pending.len(), 2);
    mark_raw_paths_ingested(&config, &[first.pending[0].rel.clone()]).unwrap();
    let second = raw_coverage(&config, "github:org/repo", "github.com/org/repo").unwrap();
    assert_eq!(second.covered, 1);
    assert_eq!(second.pending.len(), 1);
    assert!(needs_rebuild(
        &config,
        "github:org/repo",
        "github.com/org/repo"
    ));
}

#[tokio::test]
async fn rebuild_ingests_l1_summaries_marks_coverage_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = MemoryConfig::new(temp.path());
    config.tree.input_token_budget = 2;
    let root = config
        .workspace
        .join("memory_tree/content/raw/github-com-org-repo/issues");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("100_one.md"), "first body").unwrap();
    std::fs::write(root.join("200_two.md"), "second body").unwrap();

    let first = rebuild_tree_from_raw(
        &config,
        "github:org/repo",
        "github.com/org/repo",
        &crate::memory::tree::ConcatSummariser,
    )
    .await
    .unwrap();
    assert_eq!(first.files_read, 2);
    assert_eq!(first.batches, 2);
    assert!(!needs_rebuild(
        &config,
        "github:org/repo",
        "github.com/org/repo"
    ));
    assert_eq!(crate::memory::chunks::count_chunks(&config).unwrap(), 0);
    let tree = TreeFactory::source("github:org/repo")
        .get_or_create(&config)
        .unwrap();
    assert_eq!(
        list_summaries_at_level(&config, &tree.id, 1).unwrap().len(),
        2
    );

    let second = rebuild_tree_from_raw(
        &config,
        "github:org/repo",
        "github.com/org/repo",
        &crate::memory::tree::ConcatSummariser,
    )
    .await
    .unwrap();
    assert_eq!(second, RebuildOutcome::default());
    assert_eq!(
        list_summaries_at_level(&config, &tree.id, 1).unwrap().len(),
        2
    );
}
