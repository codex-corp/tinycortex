use std::collections::HashMap;
use std::sync::Mutex;

use super::*;

#[derive(Default)]
struct MemoryStateStore(Mutex<HashMap<String, serde_json::Value>>);

#[async_trait]
impl SyncStateStore for MemoryStateStore {
    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&format!("{namespace}:{key}"))
            .cloned())
    }

    async fn set(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(format!("{namespace}:{key}"), value.clone());
        Ok(())
    }
}

#[tokio::test]
async fn state_round_trips_cursor_dedup_and_budget() {
    let store = MemoryStateStore::default();
    let mut state = SyncState::new("gmail", "conn-1");
    state.advance_cursor("cursor-2");
    state.mark_synced("message-1");
    state.record_requests(3);
    state.save(&store).await.unwrap();

    let loaded = SyncState::load(&store, "gmail", "conn-1").await.unwrap();
    assert_eq!(loaded.cursor.as_deref(), Some("cursor-2"));
    assert!(loaded.is_synced("message-1"));
    assert_eq!(loaded.daily_budget.requests_used, 3);
}

#[test]
fn stale_budget_reports_full_and_resets_on_record() {
    let mut budget = DailyBudget {
        date: "2000-01-01".into(),
        requests_used: 499,
        limit: 500,
    };
    assert_eq!(budget.remaining(), 500);
    budget.record_requests(1);
    assert_eq!(budget.requests_used, 1);
    assert_eq!(budget.remaining(), 499);
}
