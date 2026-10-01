use std::sync::{Arc, Mutex};

use super::*;
use crate::memory::sources::SourceKind;
use crate::memory::sync::state::SyncStateStore;
use crate::memory::sync::traits::{
    ExternalSourceReader, LocalDocumentSink, SkillDocSink, SkillDocument, SyncEventSink,
};

#[derive(Default)]
struct Host {
    documents: Mutex<HashMap<String, LocalDocument>>,
    state: Mutex<HashMap<String, serde_json::Value>>,
    events: Mutex<Vec<SyncEvent>>,
    deletes: Mutex<Vec<String>>,
    external_items: Mutex<Vec<crate::memory::sources::SourceItem>>,
    external_bodies: Mutex<HashMap<String, crate::memory::sources::SourceContent>>,
}

#[async_trait]
impl SkillDocSink for Host {
    async fn store(&self, _: SkillDocument) -> anyhow::Result<()> {
        Ok(())
    }
    async fn delete(&self, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl LocalDocumentSink for Host {
    async fn upsert(&self, document: LocalDocument) -> anyhow::Result<()> {
        self.documents
            .lock()
            .unwrap()
            .insert(document.source_id.clone(), document);
        Ok(())
    }

    async fn delete(&self, source_id: &str) -> anyhow::Result<()> {
        self.documents.lock().unwrap().remove(source_id);
        self.deletes.lock().unwrap().push(source_id.into());
        Ok(())
    }
}

#[async_trait]
impl SyncEventSink for Host {
    async fn emit(&self, event: SyncEvent) -> anyhow::Result<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[async_trait]
impl SyncStateStore for Host {
    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(self
            .state
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
        self.state
            .lock()
            .unwrap()
            .insert(format!("{namespace}:{key}"), value.clone());
        Ok(())
    }
}

#[async_trait]
impl ExternalSourceReader for Host {
    async fn list_items(
        &self,
        _: &MemorySourceEntry,
    ) -> anyhow::Result<Vec<crate::memory::sources::SourceItem>> {
        Ok(self.external_items.lock().unwrap().clone())
    }

    async fn read_item(
        &self,
        _: &MemorySourceEntry,
        item_id: &str,
    ) -> anyhow::Result<crate::memory::sources::SourceContent> {
        self.external_bodies
            .lock()
            .unwrap()
            .get(item_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing external item {item_id}"))
    }
}

fn folder_source(path: &std::path::Path) -> MemorySourceEntry {
    MemorySourceEntry {
        id: "folder-1".into(),
        kind: SourceKind::Folder,
        label: "Notes".into(),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: Some(path.to_string_lossy().into_owned()),
        glob: Some("**/*.md".into()),
        url: None,
        branch: None,
        paths: Vec::new(),
        max_commits: None,
        max_issues: None,
        max_prs: None,
        query: None,
        since_days: None,
        max_items: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
}

#[tokio::test]
async fn folder_pipeline_tracks_create_update_noop_and_remove() {
    let temp = tempfile::tempdir().unwrap();
    let notes = temp.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let file = notes.join("daily.md");
    std::fs::write(&file, "first").unwrap();
    let config = MemoryConfig::new(temp.path().join("workspace"));
    let pipeline = WorkspaceSourcePipeline::new(folder_source(&notes)).unwrap();
    let host = Arc::new(Host::default());
    let context = SyncContext {
        events: host.clone(),
        documents: host.clone(),
        state: host.clone(),
        local_documents: Some(host.clone()),
        external_sources: None,
        summariser: None,
    };

    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        1
    );
    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        0
    );
    std::thread::sleep(std::time::Duration::from_millis(5));
    std::fs::write(&file, "second").unwrap();
    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        1
    );
    assert_eq!(
        host.documents.lock().unwrap()["mem_src:folder-1:daily.md"].body,
        "second"
    );
    std::fs::remove_file(&file).unwrap();
    let removed = pipeline.tick(&config, &context).await.unwrap();
    assert_eq!(removed.records_ingested, 0);
    assert_eq!(removed.note.as_deref(), Some("1 removed"));
    assert!(host.documents.lock().unwrap().is_empty());
    assert_eq!(
        host.deletes.lock().unwrap().as_slice(),
        ["mem_src:folder-1:daily.md"]
    );
}

#[tokio::test]
async fn external_pipeline_tracks_versions_without_destructive_absence() {
    use crate::memory::sources::{ContentType, SourceContent, SourceItem};

    let config = MemoryConfig::new(tempfile::tempdir().unwrap().path().join("workspace"));
    let mut source = folder_source(std::path::Path::new("unused"));
    source.id = "rss-1".into();
    source.kind = SourceKind::RssFeed;
    source.path = None;
    source.glob = None;
    source.url = Some("https://example.test/feed.xml".into());
    let pipeline = WorkspaceSourcePipeline::new(source).unwrap();
    let host = Arc::new(Host::default());
    host.external_items.lock().unwrap().push(SourceItem {
        id: "post-1".into(),
        title: "Post".into(),
        updated_at_ms: Some(1),
    });
    host.external_bodies.lock().unwrap().insert(
        "post-1".into(),
        SourceContent {
            id: "post-1".into(),
            title: "Post".into(),
            body: "first".into(),
            content_type: ContentType::Plaintext,
            metadata: serde_json::Value::Null,
        },
    );
    let context = SyncContext {
        events: host.clone(),
        documents: host.clone(),
        state: host.clone(),
        local_documents: Some(host.clone()),
        external_sources: Some(host.clone()),
        summariser: None,
    };

    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        1
    );
    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        0
    );
    host.external_items.lock().unwrap()[0].updated_at_ms = Some(2);
    host.external_bodies.lock().unwrap().remove("post-1");
    assert!(pipeline.tick(&config, &context).await.is_err());
    assert_eq!(
        host.documents.lock().unwrap()["mem_src:rss-1:post-1"].body,
        "first"
    );
    host.external_bodies.lock().unwrap().insert(
        "post-1".into(),
        SourceContent {
            id: "post-1".into(),
            title: "Post".into(),
            body: "second".into(),
            content_type: ContentType::Plaintext,
            metadata: serde_json::Value::Null,
        },
    );
    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        1
    );
    host.external_items.lock().unwrap()[0].updated_at_ms = None;
    host.external_bodies
        .lock()
        .unwrap()
        .get_mut("post-1")
        .unwrap()
        .body = "timestamp-less refresh".into();
    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        1
    );
    host.external_items.lock().unwrap().clear();
    assert_eq!(
        pipeline
            .tick(&config, &context)
            .await
            .unwrap()
            .records_ingested,
        0
    );
    assert!(host
        .documents
        .lock()
        .unwrap()
        .contains_key("mem_src:rss-1:post-1"));
}
