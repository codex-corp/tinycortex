use super::*;

fn fresh() -> Connection {
    Connection::open_in_memory().unwrap()
}

#[test]
fn migrations_create_expected_tables() {
    let conn = fresh();
    run(&conn).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for expected in [
        "people",
        "handle_aliases",
        "interactions",
        "_people_migrations",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing {expected}: {names:?}"
        );
    }
}

#[test]
fn migrations_are_idempotent() {
    let conn = fresh();
    run(&conn).unwrap();
    run(&conn).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM _people_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, MIGRATIONS.len() as i64);
}
