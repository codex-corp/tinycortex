use super::*;
use crate::memory::graph::{pairs_from_entities, upsert_edges};

#[test]
fn bounded_bfs_finds_two_hop_pair() {
    let temp = tempfile::tempdir().unwrap();
    let config = MemoryConfig::new(temp.path());
    upsert_edges(
        &config,
        &pairs_from_entities(&["alice".into(), "bob".into()]),
        1,
    )
    .unwrap();
    upsert_edges(
        &config,
        &pairs_from_entities(&["bob".into(), "carol".into()]),
        1,
    )
    .unwrap();
    assert!(
        pair_distances(&config, &["alice".into(), "carol".into()], 1)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        pair_distances(&config, &["alice".into(), "carol".into()], 2).unwrap(),
        vec![PairDistance {
            a: "alice".into(),
            b: "carol".into(),
            dist: 2,
        }]
    );
}
