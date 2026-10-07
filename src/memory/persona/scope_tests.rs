use super::*;
use tempfile::tempdir;

#[test]
fn accepts_project_and_children_but_not_sibling_prefix() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("api")).unwrap();
    assert!(matches_project(Some(root.to_str().unwrap()), &root));
    assert!(matches_project(
        Some(root.join("api").to_str().unwrap()),
        &root
    ));
    assert!(!matches_project(
        Some(temp.path().join("project-old").to_str().unwrap()),
        &root
    ));
}

#[test]
fn missing_relative_or_unresolvable_scope_is_rejected() {
    let temp = tempdir().unwrap();
    assert!(!matches_project(None, temp.path()));
    assert!(!matches_project(Some("project"), temp.path()));
    assert!(!matches_project(
        Some("/nonexistent-project-0193"),
        temp.path()
    ));
}

#[cfg(unix)]
#[test]
fn symlink_outside_project_is_rejected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    assert!(!matches_project(
        Some(root.join("escape").to_str().unwrap()),
        &root
    ));
}

#[test]
fn codex_metadata_is_read_without_accepting_prompt_cwd() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"response_item","payload":{"cwd":"/fake"}}
{"type":"session_meta","payload":{"cwd":"/project","originator":"codex_cli_rs"}}
"#,
    )
    .unwrap();
    assert_eq!(
        transcript_scope(&path, "codex").unwrap().as_deref(),
        Some("/project")
    );
    std::fs::write(
        &path,
        r#"{"type":"session_meta","payload":{"cwd":"/project","originator":"codex_desktop"}}"#,
    )
    .unwrap();
    assert_eq!(
        transcript_scope(&path, "codex").unwrap().as_deref(),
        Some("/project")
    );
}

#[test]
fn a_file_is_not_a_working_directory_scope() {
    let temp = tempdir().unwrap();
    let file = temp.path().join("source.rs");
    std::fs::write(&file, "fixture").unwrap();
    assert!(!matches_project(file.to_str(), temp.path()));
    assert!(!matches_project(temp.path().to_str(), &file));
}
