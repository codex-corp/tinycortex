//! Local workspace source synchronization through crate-owned readers.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use crate::memory::config::MemoryConfig;
use crate::memory::sources::{reader_for, MemorySourceEntry, SourceReader};
use crate::memory::sync::state::SyncState;
use crate::memory::sync::traits::{
    LocalDocument, SyncContext, SyncEvent, SyncOutcome, SyncPipeline, SyncPipelineKind, SyncStage,
};

pub struct WorkspaceSourcePipeline {
    id: String,
    source: MemorySourceEntry,
    reader: Option<Box<dyn SourceReader>>,
}

impl WorkspaceSourcePipeline {
    pub fn new(source: MemorySourceEntry) -> anyhow::Result<Self> {
        source.validate().map_err(anyhow::Error::msg)?;
        let reader = reader_for(&source.kind);
        Ok(Self {
            id: format!("workspace:{}:{}", source.kind.as_str(), source.id),
            source,
            reader,
        })
    }

    fn source_id(&self, item_id: &str) -> String {
        format!("mem_src:{}:{item_id}", self.source.id)
    }

    async fn event(&self, context: &SyncContext, stage: SyncStage, message: Option<String>) {
        let _ = context
            .events
            .emit(SyncEvent {
                source_id: self.id.clone(),
                toolkit: self.source.kind.as_str().into(),
                connection_id: Some(self.source.id.clone()),
                stage,
                message,
            })
            .await;
    }
}

#[async_trait]
impl SyncPipeline for WorkspaceSourcePipeline {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> SyncPipelineKind {
        SyncPipelineKind::Workspace
    }
    async fn init(&self, _: &MemoryConfig, _: &SyncContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn tick(
        &self,
        config: &MemoryConfig,
        context: &SyncContext,
    ) -> anyhow::Result<SyncOutcome> {
        if !self.source.enabled {
            return Ok(SyncOutcome {
                note: Some("source disabled".into()),
                ..SyncOutcome::default()
            });
        }
        self.event(context, SyncStage::Fetching, None).await;
        let state_toolkit = format!("workspace:{}", self.source.kind.as_str());
        let mut state =
            SyncState::load(context.state.as_ref(), &state_toolkit, &self.source.id).await?;
        let local_documents = context
            .local_documents
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("workspace pipeline requires a local document sink"))?;
        let items = match &self.reader {
            Some(reader) => reader
                .list_items(&self.source, config)
                .await
                .map_err(anyhow::Error::msg)?,
            None => {
                context
                    .external_sources
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "source kind requires an external reader: {}",
                            self.source.kind.as_str()
                        )
                    })?
                    .list_items(&self.source)
                    .await?
            }
        };
        let current_ids: HashSet<_> = items.iter().map(|item| item.id.clone()).collect();
        let mut versions = HashMap::with_capacity(items.len());
        let mut ingested = 0u32;

        for item in items {
            let version = item
                .updated_at_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into());
            versions.insert(item.id.clone(), version.clone());
            if item.updated_at_ms.is_some() && state.item_versions.get(&item.id) == Some(&version) {
                continue;
            }
            let source_id = self.source_id(&item.id);
            let content = match &self.reader {
                Some(reader) => reader
                    .read_item(&self.source, &item.id, config)
                    .await
                    .map_err(anyhow::Error::msg)?,
                None => {
                    context
                        .external_sources
                        .as_ref()
                        .expect("external reader checked before item loop")
                        .read_item(&self.source, &item.id)
                        .await?
                }
            };
            local_documents
                .upsert(LocalDocument {
                    source_id,
                    path_scope: None,
                    owner: "user".into(),
                    tags: vec!["memory_sources".into(), self.source.kind.as_str().into()],
                    title: content.title,
                    body: content.body,
                    modified_at: item
                        .updated_at_ms
                        .and_then(chrono::DateTime::from_timestamp_millis)
                        .unwrap_or_else(chrono::Utc::now),
                    source_ref: Some(format!("{}:{}", self.source.id, item.id)),
                })
                .await?;
            ingested = ingested.saturating_add(1);
        }

        let removed: Vec<_> = if self.source.kind == crate::memory::sources::SourceKind::Folder {
            state
                .item_versions
                .keys()
                .filter(|id| !current_ids.contains(*id))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        for item_id in &removed {
            local_documents.delete(&self.source_id(item_id)).await?;
        }
        state.item_versions = versions;
        state.last_sync_at_ms = Some(chrono::Utc::now().timestamp_millis() as u64);
        state.save(context.state.as_ref()).await?;
        self.event(
            context,
            SyncStage::Stored,
            Some(format!("{ingested} stored, {} removed", removed.len())),
        )
        .await;
        self.event(context, SyncStage::Completed, None).await;
        Ok(SyncOutcome {
            records_ingested: ingested,
            more_pending: false,
            actions_called: 0,
            provider_cost_usd: 0.0,
            note: (!removed.is_empty()).then(|| format!("{} removed", removed.len())),
        })
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
