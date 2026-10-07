//! Offline regressions for resumable Codex ingestion.
use tempfile::tempdir;
struct ScopedChat(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl crate::memory::score::extract::ChatProvider for ScopedChat {
    fn name(&self) -> &str {
        "scoped-offline-fixture"
    }
    async fn chat_for_json(
        &self,
        prompt: &crate::memory::score::extract::ChatPrompt,
    ) -> anyhow::Result<String> {
        assert!(!prompt.user.contains("EXCLUDED_SECRET"));
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(r#"{"observations":[{"facet":"workflow","observation":"Run focused tests","quote":"tests","tier":"t2"}]}"#.into())
    }
}

fn write_codex(path: &std::path::Path, project: &std::path::Path, turns: usize) {
    let mut content =
        serde_json::json!({"type":"session_meta", "payload":{"cwd":project,"id":"fixture"}})
            .to_string();
    content.push('\n');
    for n in 0..turns {
        let value = serde_json::json!({"type":"response_item", "timestamp":"2026-10-01T00:00:00Z",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text",
            "text":format!("Run tests {n} {}", "abc ".repeat(400))}]}});
        content.push_str(&value.to_string());
        content.push('\n');
    }
    std::fs::write(path, content).unwrap();
}

#[tokio::test]
async fn codex_long_session_resumes_after_restart_without_resending_completed_windows() {
    use crate::memory::persona::{PersonaConfig, Pipeline, RunMode};
    let temp = tempdir().unwrap();
    let project = temp.path().join("project");
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let transcript = sessions.join("rollout-inside.jsonl");
    write_codex(&transcript, &project, 100);
    write_codex(&sessions.join("rollout-outside.jsonl"), temp.path(), 1);
    std::fs::write(sessions.join("rollout-missing.jsonl"), "{}\n").unwrap();
    let config = crate::memory::MemoryConfig::new(temp.path().join("memory"));
    let mut persona = PersonaConfig::with_home(temp.path(), "fixture");
    persona.codex_root = Some(sessions);
    persona.claude_code_root = None;
    persona.project_roots.clear();
    persona.global_instruction_files.clear();
    persona.codex_project_root = Some(project.clone());
    persona.run_budget.max_sessions = 5;
    persona.run_budget.max_llm_calls = 5;
    let chat = ScopedChat(std::sync::atomic::AtomicUsize::new(0));
    let mut done = false;
    for pass in 0..10 {
        let store =
            crate::memory::persona::state::FileStateStore::open_in_workspace(&config.workspace)
                .unwrap();
        let before = chat.0.load(std::sync::atomic::Ordering::SeqCst);
        let report = Pipeline {
            config: &config,
            persona: &persona,
            provider: &chat,
            summariser: &crate::memory::tree::summarise::ConcatSummariser,
            store: &store,
        }
        .run(RunMode::Incremental)
        .await
        .unwrap();
        assert_eq!(report.files_seen, 1);
        assert_eq!(report.sessions_excluded, 2);
        assert_eq!(report.sessions_failed, 0);
        assert!(chat.0.load(std::sync::atomic::Ordering::SeqCst) - before <= 5);
        if pass == 0 {
            assert_eq!(report.sessions_processed, 0);
            assert_eq!(report.checkpoints_advanced, 5);
        }
        if report.sessions_processed == 1 {
            done = true;
            break;
        }
        assert!(report.budget_hit);
        assert!(report.checkpoints_advanced > 0);
    }
    assert!(done);
    let before = chat.0.load(std::sync::atomic::Ordering::SeqCst);
    let store = crate::memory::persona::state::FileStateStore::open_in_workspace(&config.workspace)
        .unwrap();
    let report = Pipeline {
        config: &config,
        persona: &persona,
        provider: &chat,
        summariser: &crate::memory::tree::summarise::ConcatSummariser,
        store: &store,
    }
    .run(RunMode::Incremental)
    .await
    .unwrap();
    assert_eq!(report.sessions_skipped, 1);
    assert_eq!(chat.0.load(std::sync::atomic::Ordering::SeqCst), before);
    // An appended window reuses unchanged content-addressed pieces.
    write_codex(&transcript, &project, 101);
    let store = crate::memory::persona::state::FileStateStore::open_in_workspace(&config.workspace)
        .unwrap();
    let report = Pipeline {
        config: &config,
        persona: &persona,
        provider: &chat,
        summariser: &crate::memory::tree::summarise::ConcatSummariser,
        store: &store,
    }
    .run(RunMode::Incremental)
    .await
    .unwrap();
    assert_eq!(report.sessions_processed, 1);
    assert!(chat.0.load(std::sync::atomic::Ordering::SeqCst) - before <= 2);
}

struct FailingChat(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl crate::memory::score::extract::ChatProvider for FailingChat {
    fn name(&self) -> &str {
        "failure-fixture"
    }
    async fn chat_for_json(
        &self,
        prompt: &crate::memory::score::extract::ChatPrompt,
    ) -> anyhow::Result<String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if prompt.user.contains("FAIL_ME") {
            anyhow::bail!("private provider detail SECRET");
        }
        Ok(r#"{"observations":[]}"#.into())
    }
}

#[tokio::test]
async fn failed_codex_session_retains_retry_and_does_not_block_healthy_session() {
    use crate::memory::persona::{PersonaConfig, Pipeline, RunMode};
    let temp = tempdir().unwrap();
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let bad = sessions.join("rollout-bad.jsonl");
    write_codex(&bad, temp.path(), 1);
    let text = std::fs::read_to_string(&bad)
        .unwrap()
        .replace("Run tests 0", "FAIL_ME");
    std::fs::write(&bad, text).unwrap();
    write_codex(&sessions.join("rollout-good.jsonl"), temp.path(), 1);
    let config = crate::memory::MemoryConfig::new(temp.path().join("memory"));
    let mut persona = PersonaConfig::with_home(temp.path(), "fixture");
    persona.codex_root = Some(sessions);
    persona.claude_code_root = None;
    persona.project_roots.clear();
    persona.global_instruction_files.clear();
    persona.codex_project_root = Some(temp.path().to_path_buf());
    persona.run_budget.max_sessions = 5;
    persona.run_budget.max_llm_calls = 5;
    let chat = FailingChat(std::sync::atomic::AtomicUsize::new(0));
    for pass in 0..2 {
        let store =
            crate::memory::persona::state::FileStateStore::open_in_workspace(&config.workspace)
                .unwrap();
        let report = Pipeline {
            config: &config,
            persona: &persona,
            provider: &chat,
            summariser: &crate::memory::tree::summarise::ConcatSummariser,
            store: &store,
        }
        .run(RunMode::Incremental)
        .await
        .unwrap();
        assert_eq!(report.sessions_failed, 1);
        assert_eq!(report.sessions_processed, usize::from(pass == 0));
        assert_eq!(report.failures[0].code, "provider_failure");
        assert!(!serde_json::to_string(&report.failures)
            .unwrap()
            .contains("SECRET"));
    }
    assert_eq!(chat.0.load(std::sync::atomic::Ordering::SeqCst), 3);
}

struct InvalidChat;
#[async_trait::async_trait]
impl crate::memory::score::extract::ChatProvider for InvalidChat {
    fn name(&self) -> &str {
        "invalid-fixture"
    }
    async fn chat_for_json(
        &self,
        _: &crate::memory::score::extract::ChatPrompt,
    ) -> anyhow::Result<String> {
        Ok("not JSON PRIVATE_TEXT".into())
    }
}

#[tokio::test]
async fn invalid_digest_is_reported_and_never_commits_a_completed_cursor() {
    use crate::memory::persona::{PersonaConfig, Pipeline, RunMode};
    let temp = tempdir().unwrap();
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    write_codex(&sessions.join("rollout-bad.jsonl"), temp.path(), 1);
    let config = crate::memory::MemoryConfig::new(temp.path().join("memory"));
    let mut persona = PersonaConfig::with_home(temp.path(), "fixture");
    persona.codex_root = Some(sessions);
    persona.claude_code_root = None;
    persona.project_roots.clear();
    persona.global_instruction_files.clear();
    persona.codex_project_root = Some(temp.path().to_path_buf());
    persona.run_budget.max_llm_calls = 5;
    for _ in 0..2 {
        let store =
            crate::memory::persona::state::FileStateStore::open_in_workspace(&config.workspace)
                .unwrap();
        let report = Pipeline {
            config: &config,
            persona: &persona,
            provider: &InvalidChat,
            summariser: &crate::memory::tree::summarise::ConcatSummariser,
            store: &store,
        }
        .run(RunMode::Incremental)
        .await
        .unwrap();
        assert_eq!(report.sessions_failed, 1);
        assert_eq!(report.sessions_processed, 0);
        assert_eq!(report.failures[0].code, "parse_failure");
        assert!(!serde_json::to_string(&report.failures)
            .unwrap()
            .contains("PRIVATE_TEXT"));
    }
}
