//! Persistent prompt history for the interactive composer.

use std::fmt;
use std::path::PathBuf;

/// Bounded history store. `None` keeps history process-local.
#[derive(Debug, Clone)]
pub struct PromptHistoryStore {
    path: Option<PathBuf>,
    max_entries: usize,
}

#[derive(Debug)]
pub enum PromptHistoryError {
    Io(std::io::Error),
    Decode(serde_json::Error),
}

impl fmt::Display for PromptHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "prompt history I/O failed: {error}"),
            Self::Decode(error) => write!(formatter, "prompt history is invalid: {error}"),
        }
    }
}

impl std::error::Error for PromptHistoryError {}

impl From<std::io::Error> for PromptHistoryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PromptHistoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error)
    }
}

impl PromptHistoryStore {
    pub fn new(path: Option<PathBuf>, max_entries: usize) -> Self {
        Self {
            path,
            max_entries: max_entries.max(1),
        }
    }

    /// Load and bound persisted entries. A missing file is an empty history.
    pub fn load(&self) -> Result<Vec<String>, PromptHistoryError> {
        let Some(path) = &self.path else {
            return Ok(Vec::new());
        };
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut entries: Vec<String> = serde_json::from_str(&source)?;
        entries.retain(|entry| !entry.trim().is_empty());
        trim_to_limit(&mut entries, self.max_entries);
        Ok(entries)
    }

    /// Record one normalized prompt and atomically persist the bounded list.
    pub fn record(&self, entries: &mut Vec<String>, input: &str) -> Result<(), PromptHistoryError> {
        let input = input.trim();
        if input.is_empty() {
            return Ok(());
        }
        if entries.last().map(String::as_str) != Some(input) {
            entries.push(input.to_string());
            trim_to_limit(entries, self.max_entries);
        }
        self.persist(entries)
    }

    fn persist(&self, entries: &[String]) -> Result<(), PromptHistoryError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(entries)?;
        write_private(&temporary, &bytes)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

fn trim_to_limit(entries: &mut Vec<String>, max_entries: usize) {
    let overflow = entries.len().saturating_sub(max_entries);
    if overflow > 0 {
        entries.drain(..overflow);
    }
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_history_round_trips_across_store_instances() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.json");
        let store = PromptHistoryStore::new(Some(path.clone()), 50);
        let mut entries = store.load().unwrap();

        store.record(&mut entries, "first prompt").unwrap();
        store.record(&mut entries, "second prompt").unwrap();

        let restarted = PromptHistoryStore::new(Some(path), 50);
        assert_eq!(
            restarted.load().unwrap(),
            vec!["first prompt".to_string(), "second prompt".to_string()]
        );
    }

    #[test]
    fn test_prompt_history_deduplicates_consecutive_entries_and_trims_limit() {
        let directory = tempfile::tempdir().unwrap();
        let store = PromptHistoryStore::new(Some(directory.path().join("history.json")), 2);
        let mut entries = Vec::new();

        store.record(&mut entries, "first").unwrap();
        store.record(&mut entries, "first").unwrap();
        store.record(&mut entries, "second").unwrap();
        store.record(&mut entries, "third").unwrap();

        assert_eq!(entries, vec!["second".to_string(), "third".to_string()]);
    }

    #[test]
    fn test_prompt_history_ignores_blank_input_and_supports_memory_only_mode() {
        let store = PromptHistoryStore::new(None, 50);
        let mut entries = Vec::new();

        store.record(&mut entries, "  ").unwrap();
        store.record(&mut entries, "kept").unwrap();

        assert_eq!(entries, vec!["kept".to_string()]);
    }
}
