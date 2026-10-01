use super::*;

#[test]
fn persisted_edges_are_canonical_weighted_and_symmetric() {
    let temp = tempfile::tempdir().unwrap();
    let config = MemoryConfig::new(temp.path());
    let pairs = pairs_from_entities(&[
        "person:bob".into(),
        "person:alice".into(),
        "person:alice".into(),
    ]);
    assert_eq!(pairs, vec![("person:alice".into(), "person:bob".into())]);
    upsert_edges(&config, &pairs, 1).unwrap();
    upsert_edges(&config, &pairs, 2).unwrap();
    assert_eq!(
        edge_neighbors(&config, "person:bob").unwrap(),
        vec![("person:alice".into(), 2)]
    );
    assert_eq!(count_edges(&config).unwrap(), 1);
}
