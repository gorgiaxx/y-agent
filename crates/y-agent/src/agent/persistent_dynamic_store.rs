//! Append-only durable storage for runtime-created agent definitions.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::agent::dynamic_agent::{
    validate_definition, AgentStatus, DynamicAgentDefinition, DynamicAgentStoreBackend,
};
use crate::agent::error::MultiAgentError;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct DynamicAgentEvent {
    schema_version: u32,
    agent: DynamicAgentDefinition,
}

/// JSONL-backed dynamic-agent store.
///
/// Every mutation appends a complete definition snapshot and synchronizes it
/// before updating in-memory state. Reopening replays the latest snapshot for
/// each agent. A final non-newline-terminated record is treated as an
/// interrupted write and ignored.
pub struct PersistentDynamicAgentStore {
    path: PathBuf,
    agents: RwLock<HashMap<String, DynamicAgentDefinition>>,
}

impl PersistentDynamicAgentStore {
    /// Open or create a durable store at `path` and replay committed events.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MultiAgentError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(storage_error)?;
        }

        let mut agents = HashMap::new();
        if path.exists() {
            let bytes = fs::read(&path).map_err(storage_error)?;
            for record in bytes.split_inclusive(|byte| *byte == b'\n') {
                if !record.ends_with(b"\n") {
                    break;
                }
                let line = record.strip_suffix(b"\n").unwrap_or(record);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line.is_empty() {
                    continue;
                }
                let event: DynamicAgentEvent =
                    serde_json::from_slice(line).map_err(storage_error)?;
                if event.schema_version != SCHEMA_VERSION {
                    return Err(MultiAgentError::Other {
                        message: format!(
                            "unsupported dynamic-agent journal schema version {}",
                            event.schema_version
                        ),
                    });
                }
                validate_definition(&event.agent).map_err(storage_error)?;
                agents.insert(event.agent.id.clone(), event.agent);
            }
        }

        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(storage_error)?;

        Ok(Self {
            path,
            agents: RwLock::new(agents),
        })
    }

    fn append(&self, agent: &DynamicAgentDefinition) -> Result<(), MultiAgentError> {
        let event = DynamicAgentEvent {
            schema_version: SCHEMA_VERSION,
            agent: agent.clone(),
        };
        let mut encoded = serde_json::to_vec(&event).map_err(storage_error)?;
        encoded.push(b'\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(storage_error)?;
        file.write_all(&encoded).map_err(storage_error)?;
        file.sync_all().map_err(storage_error)
    }

    /// Restore an active historical snapshot as a new monotonically increasing version.
    pub fn rollback(
        &self,
        id: &str,
        target_version: u64,
    ) -> Result<DynamicAgentDefinition, MultiAgentError> {
        let mut agents = self.agents.write().map_err(lock_error)?;
        let Some(current) = agents.get(id) else {
            return Err(MultiAgentError::NotFound { id: id.to_string() });
        };
        if target_version >= current.version {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!(
                    "rollback target version {target_version} must be older than current version {}",
                    current.version
                ),
            });
        }

        let mut restored = self
            .historical_snapshot(id, target_version)?
            .ok_or_else(|| MultiAgentError::NotFound {
                id: format!("{id}@{target_version}"),
            })?;
        restored.version = current.version.saturating_add(1);
        restored.status = AgentStatus::Active;
        restored.deactivated_at = None;
        restored.deactivation_reason = None;
        validate_definition(&restored).map_err(storage_error)?;
        self.append(&restored)?;
        agents.insert(id.to_string(), restored.clone());
        Ok(restored)
    }

    /// Load a committed active snapshot for a specific historical version.
    pub fn get_version(
        &self,
        id: &str,
        version: u64,
    ) -> Result<Option<DynamicAgentDefinition>, MultiAgentError> {
        self.historical_snapshot(id, version)
    }

    fn historical_snapshot(
        &self,
        id: &str,
        version: u64,
    ) -> Result<Option<DynamicAgentDefinition>, MultiAgentError> {
        let bytes = fs::read(&self.path).map_err(storage_error)?;
        let mut found = None;
        for record in bytes.split_inclusive(|byte| *byte == b'\n') {
            if !record.ends_with(b"\n") {
                break;
            }
            let line = record.strip_suffix(b"\n").unwrap_or(record);
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            let event: DynamicAgentEvent = serde_json::from_slice(line).map_err(storage_error)?;
            if event.agent.id == id
                && event.agent.version == version
                && event.agent.status == AgentStatus::Active
            {
                found = Some(event.agent);
            }
        }
        Ok(found)
    }

    fn read_agents(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashMap<String, DynamicAgentDefinition>> {
        self.agents
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl DynamicAgentStoreBackend for PersistentDynamicAgentStore {
    fn create(
        &self,
        def: DynamicAgentDefinition,
    ) -> Result<DynamicAgentDefinition, MultiAgentError> {
        validate_definition(&def).map_err(storage_error)?;
        let mut agents = self.agents.write().map_err(lock_error)?;
        if agents.contains_key(&def.id) {
            return Err(MultiAgentError::DelegationFailed {
                message: format!("agent '{}' already exists", def.id),
            });
        }
        self.append(&def)?;
        agents.insert(def.id.clone(), def.clone());
        Ok(def)
    }

    fn update(
        &self,
        mut def: DynamicAgentDefinition,
    ) -> Result<DynamicAgentDefinition, MultiAgentError> {
        validate_definition(&def).map_err(storage_error)?;
        let mut agents = self.agents.write().map_err(lock_error)?;
        let Some(existing) = agents.get(&def.id) else {
            return Err(MultiAgentError::DelegationFailed {
                message: format!("agent '{}' not found", def.id),
            });
        };
        def.version = existing.version.saturating_add(1);
        self.append(&def)?;
        agents.insert(def.id.clone(), def.clone());
        Ok(def)
    }

    fn deactivate(&self, id: &str, reason: &str) -> Result<(), MultiAgentError> {
        let mut agents = self.agents.write().map_err(lock_error)?;
        let Some(existing) = agents.get(id) else {
            return Err(MultiAgentError::DelegationFailed {
                message: format!("agent '{id}' not found"),
            });
        };
        let mut deactivated = existing.clone();
        deactivated.status = AgentStatus::Deactivated;
        deactivated.deactivated_at = Some(Utc::now().to_rfc3339());
        deactivated.deactivation_reason = Some(reason.to_string());
        self.append(&deactivated)?;
        agents.insert(id.to_string(), deactivated);
        Ok(())
    }

    fn search(&self, query: &str) -> Vec<DynamicAgentDefinition> {
        let query = query.to_lowercase();
        let mut matches: Vec<_> = self
            .read_agents()
            .values()
            .filter(|agent| {
                agent.status == AgentStatus::Active
                    && (agent.definition.name.to_lowercase().contains(&query)
                        || agent.definition.description.to_lowercase().contains(&query)
                        || agent
                            .definition
                            .capabilities
                            .iter()
                            .any(|capability| capability.to_lowercase().contains(&query)))
            })
            .cloned()
            .collect();
        matches.sort_by(|left, right| left.id.cmp(&right.id));
        matches
    }

    fn get(&self, id: &str) -> Option<DynamicAgentDefinition> {
        self.read_agents().get(id).cloned()
    }

    fn list_active(&self) -> Vec<DynamicAgentDefinition> {
        let mut active: Vec<_> = self
            .read_agents()
            .values()
            .filter(|agent| agent.status == AgentStatus::Active)
            .cloned()
            .collect();
        active.sort_by(|left, right| left.id.cmp(&right.id));
        active
    }

    fn count(&self) -> usize {
        self.read_agents().len()
    }
}

fn storage_error(error: impl std::fmt::Display) -> MultiAgentError {
    MultiAgentError::Other {
        message: format!("dynamic-agent store error: {error}"),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> MultiAgentError {
    MultiAgentError::Other {
        message: "dynamic-agent store lock poisoned".to_string(),
    }
}
