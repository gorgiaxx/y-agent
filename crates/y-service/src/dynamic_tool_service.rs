//! Durable service-layer lifecycle for runtime-created script tools.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use y_core::types::ToolName;
use y_tools::dynamic::{make_dynamic_tool, DynamicToolDef, DynamicToolKind};
use y_tools::{DynamicToolManager, ToolRegistryImpl};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DynamicToolCreateRequest {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub interpreter: String,
    pub source: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DynamicToolUpdateRequest {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<serde_json::Value>,
    pub interpreter: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalOperation {
    Upsert,
    Delete,
    Execute,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JournalEntry {
    operation: JournalOperation,
    tool_name: ToolName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    definition: Option<DynamicToolDef>,
    actor: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Coordinates exact-version persistence, manager state, and live registry state.
pub struct DynamicToolService {
    manager: DynamicToolManager,
    journal_path: PathBuf,
    mutation_lock: Mutex<()>,
    journal_lock: Mutex<()>,
}

impl DynamicToolService {
    /// Open the append-only journal and rehydrate active tools when enabled.
    pub async fn open(
        journal_path: impl AsRef<Path>,
        registry: &ToolRegistryImpl,
    ) -> Result<Self, DynamicToolServiceError> {
        let journal_path = journal_path.as_ref().to_path_buf();
        if let Some(parent) = journal_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if !journal_path.exists() {
            tokio::fs::File::create(&journal_path)
                .await?
                .sync_all()
                .await?;
        }
        let entries = load_and_repair_journal(&journal_path)?;
        let mut active = HashMap::<ToolName, DynamicToolDef>::new();
        for entry in entries {
            match entry.operation {
                JournalOperation::Upsert => {
                    if let Some(definition) = entry.definition {
                        active.insert(entry.tool_name, definition);
                    }
                }
                JournalOperation::Delete => {
                    active.remove(&entry.tool_name);
                }
                JournalOperation::Execute => {}
            }
        }

        let service = Self {
            manager: DynamicToolManager::new(),
            journal_path,
            mutation_lock: Mutex::new(()),
            journal_lock: Mutex::new(()),
        };
        let mut definitions: Vec<_> = active.into_values().collect();
        definitions.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        for definition in definitions {
            service.manager.restore_tool(definition.clone()).await?;
            if registry.config().allow_dynamic_tools {
                if registry.get_definition(&definition.name).await.is_some() {
                    return Err(DynamicToolServiceError::NameCollision {
                        name: definition.name.as_str().to_string(),
                    });
                }
                registry
                    .register_tool(
                        make_dynamic_tool(definition.clone()),
                        definition.to_tool_definition(),
                    )
                    .await
                    .map_err(|error| registry_error(&error))?;
            }
        }
        Ok(service)
    }

    pub async fn create(
        &self,
        registry: &ToolRegistryImpl,
        request: DynamicToolCreateRequest,
        actor: &str,
    ) -> Result<DynamicToolDef, DynamicToolServiceError> {
        Self::ensure_enabled(registry)?;
        let _guard = self.mutation_lock.lock().await;
        let name = ToolName::from_string(request.name.trim());
        if registry.get_definition(&name).await.is_some() {
            return Err(DynamicToolServiceError::NameCollision {
                name: name.as_str().to_string(),
            });
        }
        let definition = DynamicToolDef {
            name: name.clone(),
            description: request.description,
            parameters: request.parameters,
            kind: DynamicToolKind::Script {
                interpreter: request.interpreter,
                source: request.source,
            },
            created_by: actor.to_string(),
            created_at: chrono::Utc::now(),
            version: 1,
        };
        let tool_definition = self.manager.create_tool(definition.clone()).await?;
        if let Err(error) = registry
            .register_tool(make_dynamic_tool(definition.clone()), tool_definition)
            .await
        {
            self.manager.forget_tool(&name).await;
            return Err(registry_error(&error));
        }
        if let Err(error) = self
            .append(JournalEntry::upsert(definition.clone(), actor))
            .await
        {
            registry.unregister_tool(&name).await;
            self.manager.forget_tool(&name).await;
            return Err(error);
        }
        Ok(definition)
    }

    pub async fn update(
        &self,
        registry: &ToolRegistryImpl,
        request: DynamicToolUpdateRequest,
        actor: &str,
    ) -> Result<DynamicToolDef, DynamicToolServiceError> {
        Self::ensure_enabled(registry)?;
        let _guard = self.mutation_lock.lock().await;
        let name = ToolName::from_string(request.name.trim());
        let previous = self.manager.get_tool(&name).await.ok_or_else(|| {
            DynamicToolServiceError::NotFound {
                name: name.as_str().to_string(),
            }
        })?;
        let DynamicToolKind::Script {
            interpreter: previous_interpreter,
            source: previous_source,
        } = &previous.kind
        else {
            return Err(DynamicToolServiceError::UnsupportedKind);
        };
        let requested = DynamicToolDef {
            name: name.clone(),
            description: request
                .description
                .unwrap_or_else(|| previous.description.clone()),
            parameters: request
                .parameters
                .unwrap_or_else(|| previous.parameters.clone()),
            kind: DynamicToolKind::Script {
                interpreter: request
                    .interpreter
                    .unwrap_or_else(|| previous_interpreter.clone()),
                source: request.source.unwrap_or_else(|| previous_source.clone()),
            },
            created_by: previous.created_by.clone(),
            created_at: previous.created_at,
            version: previous.version,
        };
        self.manager.update_tool(requested).await?;
        let updated = self.manager.get_tool(&name).await.ok_or_else(|| {
            DynamicToolServiceError::NotFound {
                name: name.as_str().to_string(),
            }
        })?;
        registry.unregister_tool(&name).await;
        if let Err(error) = registry
            .register_tool(
                make_dynamic_tool(updated.clone()),
                updated.to_tool_definition(),
            )
            .await
        {
            self.restore_previous(registry, &previous).await;
            return Err(registry_error(&error));
        }
        if let Err(error) = self
            .append(JournalEntry::upsert(updated.clone(), actor))
            .await
        {
            self.restore_previous(registry, &previous).await;
            return Err(error);
        }
        Ok(updated)
    }

    pub async fn delete(
        &self,
        registry: &ToolRegistryImpl,
        name: &str,
        actor: &str,
        reason: &str,
    ) -> Result<DynamicToolDef, DynamicToolServiceError> {
        Self::ensure_enabled(registry)?;
        let _guard = self.mutation_lock.lock().await;
        let name = ToolName::from_string(name.trim());
        let previous = self.manager.get_tool(&name).await.ok_or_else(|| {
            DynamicToolServiceError::NotFound {
                name: name.as_str().to_string(),
            }
        })?;
        self.manager.delete_tool(&name, actor).await?;
        registry.unregister_tool(&name).await;
        if let Err(error) = self
            .append(JournalEntry::delete(
                name.clone(),
                actor,
                reason.trim().to_string(),
            ))
            .await
        {
            self.restore_previous(registry, &previous).await;
            return Err(error);
        }
        Ok(previous)
    }

    pub async fn get(&self, name: &str) -> Option<DynamicToolDef> {
        self.manager
            .get_tool(&ToolName::from_string(name.trim()))
            .await
    }

    pub async fn list(&self, query: Option<&str>) -> Vec<DynamicToolDef> {
        let query = query.map(str::trim).filter(|query| !query.is_empty());
        let mut tools = self.manager.list_tools().await;
        if let Some(query) = query {
            let query = query.to_lowercase();
            tools.retain(|tool| {
                tool.name.as_str().to_lowercase().contains(&query)
                    || tool.description.to_lowercase().contains(&query)
            });
        }
        tools.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        tools
    }

    /// Record execution in the in-memory audit and append-only journal.
    pub async fn record_execution(&self, name: &ToolName, actor: &str) {
        if self.manager.get_tool(name).await.is_none() {
            return;
        }
        self.manager.record_execution(name, actor).await;
        if let Err(error) = self
            .append(JournalEntry::execute(name.clone(), actor))
            .await
        {
            tracing::warn!(tool = %name, %error, "failed to persist dynamic-tool execution audit");
        }
    }

    fn ensure_enabled(registry: &ToolRegistryImpl) -> Result<(), DynamicToolServiceError> {
        if registry.config().allow_dynamic_tools {
            Ok(())
        } else {
            Err(DynamicToolServiceError::Disabled)
        }
    }

    async fn restore_previous(&self, registry: &ToolRegistryImpl, previous: &DynamicToolDef) {
        registry.unregister_tool(&previous.name).await;
        if let Err(error) = self.manager.restore_tool(previous.clone()).await {
            tracing::error!(tool = %previous.name, %error, "failed to restore dynamic-tool manager state");
            return;
        }
        if let Err(error) = registry
            .register_tool(
                make_dynamic_tool(previous.clone()),
                previous.to_tool_definition(),
            )
            .await
        {
            tracing::error!(tool = %previous.name, %error, "failed to restore dynamic tool in registry");
        }
    }

    async fn append(&self, entry: JournalEntry) -> Result<(), DynamicToolServiceError> {
        let _guard = self.journal_lock.lock().await;
        let mut bytes = serde_json::to_vec(&entry)?;
        bytes.push(b'\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        Ok(())
    }
}

impl JournalEntry {
    fn upsert(definition: DynamicToolDef, actor: &str) -> Self {
        Self {
            operation: JournalOperation::Upsert,
            tool_name: definition.name.clone(),
            definition: Some(definition),
            actor: actor.to_string(),
            timestamp: chrono::Utc::now(),
            reason: None,
        }
    }

    fn delete(tool_name: ToolName, actor: &str, reason: String) -> Self {
        Self {
            operation: JournalOperation::Delete,
            tool_name,
            definition: None,
            actor: actor.to_string(),
            timestamp: chrono::Utc::now(),
            reason: Some(reason),
        }
    }

    fn execute(tool_name: ToolName, actor: &str) -> Self {
        Self {
            operation: JournalOperation::Execute,
            tool_name,
            definition: None,
            actor: actor.to_string(),
            timestamp: chrono::Utc::now(),
            reason: None,
        }
    }
}

fn load_and_repair_journal(path: &Path) -> Result<Vec<JournalEntry>, DynamicToolServiceError> {
    let bytes = std::fs::read(path)?;
    let mut entries = Vec::new();
    let mut offset = 0;
    let mut valid_end = 0;
    for segment in bytes.split_inclusive(|byte| *byte == b'\n') {
        let segment_end = offset + segment.len();
        let text = std::str::from_utf8(segment).map_err(|error| {
            DynamicToolServiceError::CorruptJournal {
                message: error.to_string(),
            }
        })?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            valid_end = segment_end;
            offset = segment_end;
            continue;
        }
        match serde_json::from_str::<JournalEntry>(trimmed) {
            Ok(entry) => {
                entries.push(entry);
                valid_end = segment_end;
            }
            Err(_error) if segment_end == bytes.len() && !segment.ends_with(b"\n") => {
                let file = std::fs::OpenOptions::new().write(true).open(path)?;
                file.set_len(u64::try_from(valid_end).map_err(|conversion| {
                    DynamicToolServiceError::CorruptJournal {
                        message: conversion.to_string(),
                    }
                })?)?;
                file.sync_all()?;
                break;
            }
            Err(error) => {
                return Err(DynamicToolServiceError::CorruptJournal {
                    message: format!("invalid record at byte {offset}: {error}"),
                });
            }
        }
        offset = segment_end;
    }
    Ok(entries)
}

fn registry_error(error: &y_tools::ToolRegistryError) -> DynamicToolServiceError {
    DynamicToolServiceError::Registry {
        message: error.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DynamicToolServiceError {
    #[error("dynamic tools are disabled by tools.allow_dynamic_tools")]
    Disabled,
    #[error("tool name collides with an existing registry entry: {name}")]
    NameCollision { name: String },
    #[error("dynamic tool not found: {name}")]
    NotFound { name: String },
    #[error("only script dynamic tools are currently supported")]
    UnsupportedKind,
    #[error("dynamic-tool registry error: {message}")]
    Registry { message: String },
    #[error("dynamic-tool journal is corrupt: {message}")]
    CorruptJournal { message: String },
    #[error(transparent)]
    Tool(#[from] y_core::tool::ToolError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}
