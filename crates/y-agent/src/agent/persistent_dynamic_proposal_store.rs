//! Append-only persistence for governed dynamic-agent evolution proposals.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::agent::dynamic_agent_proposal::DynamicAgentEvolutionProposal;
use crate::agent::error::MultiAgentError;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct ProposalEvent {
    schema_version: u32,
    proposal: DynamicAgentEvolutionProposal,
}

pub struct PersistentDynamicAgentProposalStore {
    path: PathBuf,
    proposals: RwLock<HashMap<String, DynamicAgentEvolutionProposal>>,
}

impl PersistentDynamicAgentProposalStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MultiAgentError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(storage_error)?;
        }
        let mut proposals = HashMap::new();
        if path.exists() {
            for event in read_events(&path)? {
                proposals.insert(event.proposal.id.clone(), event.proposal);
            }
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(storage_error)?;
        Ok(Self {
            path,
            proposals: RwLock::new(proposals),
        })
    }

    pub fn create(
        &self,
        proposal: DynamicAgentEvolutionProposal,
    ) -> Result<DynamicAgentEvolutionProposal, MultiAgentError> {
        let mut proposals = self.proposals.write().map_err(lock_error)?;
        if proposals.contains_key(&proposal.id) {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!("dynamic-agent proposal '{}' already exists", proposal.id),
            });
        }
        if proposals.values().any(|existing| {
            existing.agent_id == proposal.agent_id
                && existing.current_version == proposal.current_version
                && existing.status.is_open()
        }) {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!(
                    "an open proposal already exists for '{}@{}'",
                    proposal.agent_id, proposal.current_version
                ),
            });
        }
        self.append(&proposal)?;
        proposals.insert(proposal.id.clone(), proposal.clone());
        Ok(proposal)
    }

    pub fn update(
        &self,
        mut proposal: DynamicAgentEvolutionProposal,
    ) -> Result<DynamicAgentEvolutionProposal, MultiAgentError> {
        let mut proposals = self.proposals.write().map_err(lock_error)?;
        let Some(existing) = proposals.get(&proposal.id) else {
            return Err(MultiAgentError::NotFound {
                id: proposal.id.clone(),
            });
        };
        proposal.revision = existing.revision.saturating_add(1);
        self.append(&proposal)?;
        proposals.insert(proposal.id.clone(), proposal.clone());
        Ok(proposal)
    }

    pub fn get(&self, id: &str) -> Option<DynamicAgentEvolutionProposal> {
        self.read_proposals().get(id).cloned()
    }

    pub fn list(&self) -> Vec<DynamicAgentEvolutionProposal> {
        let mut proposals: Vec<_> = self.read_proposals().values().cloned().collect();
        proposals.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        proposals
    }

    pub fn find_open(
        &self,
        agent_id: &str,
        current_version: u64,
    ) -> Option<DynamicAgentEvolutionProposal> {
        self.read_proposals()
            .values()
            .find(|proposal| {
                proposal.agent_id == agent_id
                    && proposal.current_version == current_version
                    && proposal.status.is_open()
            })
            .cloned()
    }

    pub fn find_for_version(
        &self,
        agent_id: &str,
        current_version: u64,
    ) -> Option<DynamicAgentEvolutionProposal> {
        self.read_proposals()
            .values()
            .filter(|proposal| {
                proposal.agent_id == agent_id && proposal.current_version == current_version
            })
            .max_by_key(|proposal| proposal.revision)
            .cloned()
    }

    fn append(&self, proposal: &DynamicAgentEvolutionProposal) -> Result<(), MultiAgentError> {
        let event = ProposalEvent {
            schema_version: SCHEMA_VERSION,
            proposal: proposal.clone(),
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

    fn read_proposals(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashMap<String, DynamicAgentEvolutionProposal>> {
        self.proposals
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn read_events(path: &Path) -> Result<Vec<ProposalEvent>, MultiAgentError> {
    let bytes = fs::read(path).map_err(storage_error)?;
    let mut events = Vec::new();
    for record in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !record.ends_with(b"\n") {
            break;
        }
        let line = record.strip_suffix(b"\n").unwrap_or(record);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let event: ProposalEvent = serde_json::from_slice(line).map_err(storage_error)?;
        if event.schema_version != SCHEMA_VERSION {
            return Err(MultiAgentError::Other {
                message: format!(
                    "unsupported dynamic-agent proposal schema version {}",
                    event.schema_version
                ),
            });
        }
        events.push(event);
    }
    Ok(events)
}

fn storage_error(error: impl std::fmt::Display) -> MultiAgentError {
    MultiAgentError::Other {
        message: format!("dynamic-agent proposal store error: {error}"),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> MultiAgentError {
    MultiAgentError::Other {
        message: "dynamic-agent proposal store lock poisoned".to_string(),
    }
}
