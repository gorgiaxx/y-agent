//! Persistent per-session composer draft storage.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DraftSnapshot {
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<y_core::types::Attachment>,
}

#[derive(Debug)]
pub struct DraftStore {
    path: Option<PathBuf>,
    drafts: HashMap<String, DraftSnapshot>,
}

impl DraftStore {
    pub fn new(path: Option<PathBuf>) -> Result<Self, String> {
        let drafts = match path.as_ref().map(std::fs::read) {
            Some(Ok(bytes)) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("could not parse composer drafts: {error}"))?,
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Some(Err(error)) => return Err(format!("could not read composer drafts: {error}")),
            None => HashMap::new(),
        };
        Ok(Self { path, drafts })
    }

    pub fn memory_only() -> Self {
        Self {
            path: None,
            drafts: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&DraftSnapshot> {
        self.drafts.get(key)
    }

    pub fn put(&mut self, key: String, snapshot: DraftSnapshot) -> Result<(), String> {
        self.drafts.insert(key, snapshot);
        self.persist()
    }

    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        self.drafts.remove(key);
        self.persist()
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create draft directory: {error}"))?;
        }
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.drafts)
            .map_err(|error| format!("could not encode composer drafts: {error}"))?;
        std::fs::write(&temporary, bytes)
            .map_err(|error| format!("could not write composer drafts: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("could not protect composer drafts: {error}"))?;
        }
        std::fs::rename(temporary, path)
            .map_err(|error| format!("could not commit composer drafts: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_persist_by_session_and_can_be_removed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("drafts.json");
        let mut store = DraftStore::new(Some(path.clone())).unwrap();
        store
            .put(
                "session-1".into(),
                DraftSnapshot {
                    text: "unfinished".into(),
                    attachments: Vec::new(),
                },
            )
            .unwrap();

        let mut reloaded = DraftStore::new(Some(path)).unwrap();
        assert_eq!(reloaded.get("session-1").unwrap().text, "unfinished");
        reloaded.remove("session-1").unwrap();
        assert!(reloaded.get("session-1").is_none());
    }
}
