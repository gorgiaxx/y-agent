//! Knowledge management command handlers — collection CRUD, entry browsing,
//! search, ingestion, and statistics.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use serde::Serialize;
use tauri::State;

use y_knowledge::config::KnowledgeConfig;
use y_service::knowledge_service::KnowledgeService;

// ---------------------------------------------------------------------------
// Lazy knowledge service (stored alongside AppState)
// ---------------------------------------------------------------------------

/// Thread-safe wrapper for storing a lazily initialised `KnowledgeService`.
pub struct KnowledgeState {
    service: Arc<Mutex<KnowledgeService>>,
}

impl KnowledgeState {
    /// Create from a shared `KnowledgeService` (used when wiring with `ServiceContainer`).
    ///
    /// This is the preferred constructor for production use — it ensures the GUI
    /// knowledge panel, context pipeline, and `knowledge_search` tool all operate
    /// on the same `KnowledgeService` instance (with embedding if configured).
    pub fn from_shared(service: Arc<Mutex<KnowledgeService>>) -> Self {
        Self { service }
    }

    /// Create a new `KnowledgeState` with persistence to the given data directory.
    ///
    /// Creates an **independent** `KnowledgeService` with default config. Useful
    /// for standalone or test scenarios but does **not** share state with
    /// `ServiceContainer`. Prefer [`from_shared`] in production.
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            service: Arc::new(Mutex::new(KnowledgeService::with_data_dir(
                KnowledgeConfig::default(),
                data_dir,
            ))),
        }
    }

    /// Get a shared handle to the knowledge injection middleware.
    ///
    /// Used to share the retriever with `knowledge_search` tool and
    /// `KnowledgeContextProvider` for chat integration.
    #[allow(dead_code)]
    pub async fn knowledge_handle(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<y_knowledge::middleware::InjectKnowledge<y_knowledge::tokenizer::SimpleTokenizer>>>
    {
        self.service.lock().await.knowledge_handle()
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct CollectionInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entry_count: usize,
    pub chunk_count: usize,
    pub total_bytes: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct EntryInfo {
    pub id: String,
    pub title: String,
    pub source_uri: String,
    pub source_type: String,
    pub domains: Vec<String>,
    pub quality_score: f32,
    pub chunk_count: usize,
    pub content_size: u64,
    pub state: String,
    pub hit_count: u64,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct EntryDetail {
    pub id: String,
    pub title: String,
    pub source_uri: String,
    pub domains: Vec<String>,
    pub quality_score: f32,
    pub state: String,
    pub hit_count: u64,
    pub total_chunk_count: usize,
    pub l0_summary: String,
    pub l1_sections: Vec<SectionInfo>,
    pub l2_chunks: Vec<ChunkInfo>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SectionInfo {
    pub index: usize,
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChunkInfo {
    pub id: String,
    pub content: String,
    pub token_estimate: usize,
    pub section_index: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchResultItem {
    pub chunk_id: String,
    pub title: String,
    pub content: String,
    pub relevance: f32,
    pub domains: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct IngestResult {
    pub success: bool,
    pub entry_id: Option<String>,
    pub chunk_count: usize,
    pub domains: Vec<String>,
    pub quality_score: f32,
    pub message: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct KbStats {
    pub total_collections: usize,
    pub total_entries: usize,
    pub total_chunks: usize,
    pub total_hits: u64,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all knowledge collections.
#[tauri::command]
pub async fn kb_collection_list(
    kb: State<'_, KnowledgeState>,
) -> Result<Vec<CollectionInfo>, String> {
    let service = kb.service.lock().await;
    let collections = service.list_collections();

    Ok(collections
        .iter()
        .map(|c| CollectionInfo {
            id: c.id.to_string(),
            name: c.name.clone(),
            description: c.description.clone(),
            entry_count: c.stats.entry_count as usize,
            chunk_count: c.stats.chunk_count as usize,
            total_bytes: c.stats.total_bytes,
            created_at: c.created_at.to_rfc3339(),
        })
        .collect())
}

/// Create a new collection.
#[tauri::command]
pub async fn kb_collection_create(
    kb: State<'_, KnowledgeState>,
    name: String,
    description: String,
) -> Result<CollectionInfo, String> {
    let mut service = kb.service.lock().await;
    service.create_collection(&name, &description);

    // Return the new collection.
    let collections = service.list_collections();
    let c = collections
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| "Failed to find created collection".to_string())?;

    Ok(CollectionInfo {
        id: c.id.to_string(),
        name: c.name.clone(),
        description: c.description.clone(),
        entry_count: c.stats.entry_count as usize,
        chunk_count: c.stats.chunk_count as usize,
        total_bytes: c.stats.total_bytes,
        created_at: c.created_at.to_rfc3339(),
    })
}

/// Delete a collection and all its entries.
///
/// Uses `spawn_blocking` because `delete_collection` now cascades through
/// every entry (BM25 index cleanup, disk persistence) which can take
/// several seconds for large collections.
#[tauri::command]
pub async fn kb_collection_delete(
    kb: State<'_, KnowledgeState>,
    name: String,
) -> Result<(), String> {
    let service = Arc::clone(&kb.service);
    tokio::task::spawn_blocking(move || {
        let mut guard = service.blocking_lock();
        if guard.delete_collection(&name) {
            tracing::info!(name, "Collection deleted successfully");
            Ok(())
        } else {
            Err(format!("Collection '{name}' not found"))
        }
    })
    .await
    .map_err(|e| format!("delete task failed: {e}"))?
}

/// Rename a collection.
#[tauri::command]
pub async fn kb_collection_rename(
    kb: State<'_, KnowledgeState>,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    let mut service = kb.service.lock().await;
    if service.rename_collection(&old_name, &new_name) {
        Ok(())
    } else {
        Err(format!(
            "Failed to rename '{old_name}' → '{new_name}' (not found or name taken)"
        ))
    }
}

/// List entries in a collection.
#[tauri::command]
pub async fn kb_entry_list(
    kb: State<'_, KnowledgeState>,
    collection: String,
) -> Result<Vec<EntryInfo>, String> {
    let service = kb.service.lock().await;
    let entries = service.list_entries(&collection);

    Ok(entries
        .iter()
        .map(|e| EntryInfo {
            id: e.id.to_string(),
            title: e.source.title.clone(),
            source_uri: e.source.uri.clone(),
            source_type: e.source.source_type.to_string(),
            domains: e.domains.clone(),
            quality_score: e.quality_score,
            chunk_count: e.chunks.len(),
            content_size: if e.content_size > 0 {
                e.content_size
            } else {
                e.chunks.iter().map(|c| c.len() as u64).sum()
            },
            state: e.state.to_string(),
            hit_count: e.hit_num as u64,
            updated_at: e.refreshed_at.to_rfc3339(),
        })
        .collect())
}

/// Get entry detail with L0/L1/L2 content.
#[tauri::command]
pub async fn kb_entry_detail(
    kb: State<'_, KnowledgeState>,
    entry_id: String,
    _resolution: String,
) -> Result<EntryDetail, String> {
    let service = kb.service.lock().await;
    let entry = service.get_entry(&entry_id)
        .ok_or_else(|| format!("Entry '{}' not found", entry_id))?;

    let l0_summary = entry.summary.clone().unwrap_or_default();
    let l1_sections: Vec<SectionInfo> = entry.l1_sections.iter().map(|s| {
        SectionInfo {
            index: s.index,
            title: s.title.clone(),
            summary: s.content.clone(),
        }
    }).collect();

    let total_chunk_count = entry.chunks.len();
    // Cap at 200 chunks to avoid flooding the IPC channel / UI.
    const MAX_L2_CHUNKS: usize = 200;
    let l2_chunks: Vec<ChunkInfo> = entry.chunks.iter().enumerate()
        .take(MAX_L2_CHUNKS)
        .map(|(i, content)| {
            ChunkInfo {
                id: format!("{}-{}", entry.id, i),
                content: content.clone(),
                token_estimate: content.len() / 4,
                section_index: i,
            }
        }).collect();

    Ok(EntryDetail {
        id: entry.id.to_string(),
        title: entry.source.title.clone(),
        source_uri: entry.source.uri.clone(),
        domains: entry.domains.clone(),
        quality_score: entry.quality_score,
        state: entry.state.to_string(),
        hit_count: entry.hit_num as u64,
        total_chunk_count,
        l0_summary,
        l1_sections,
        l2_chunks,
    })
}

/// Search knowledge base.
#[tauri::command]
pub async fn kb_search(
    kb: State<'_, KnowledgeState>,
    query: String,
    domain: Option<String>,
    limit: usize,
) -> Result<Vec<SearchResultItem>, String> {
    let service = kb.service.lock().await;
    let params = y_knowledge::tools::KnowledgeSearchParams {
        query,
        domain,
        resolution: "l0".to_string(),
        limit,
        collection: None,
    };
    let result = service.search(&params);

    Ok(result
        .results
        .iter()
        .map(|r| SearchResultItem {
            chunk_id: r.chunk_id.clone(),
            title: r.title.clone(),
            content: r.content.clone(),
            relevance: r.relevance,
            domains: r.domains.clone(),
        })
        .collect())
}

/// Ingest a document.
#[tauri::command]
pub async fn kb_ingest(
    kb: State<'_, KnowledgeState>,
    source: String,
    domain: Option<String>,
    collection: String,
) -> Result<IngestResult, String> {
    let mut service = kb.service.lock().await;
    let params = y_knowledge::tools::KnowledgeIngestParams {
        source,
        domain,
        collection,
    };

    match service.ingest(&params, "default").await {
        Ok(r) => Ok(IngestResult {
            success: r.success,
            entry_id: r.entry_id,
            chunk_count: r.chunk_count,
            domains: r.domains,
            quality_score: r.quality_score,
            message: r.message,
        }),
        Err(e) => Ok(IngestResult {
            success: false,
            entry_id: None,
            chunk_count: 0,
            domains: vec![],
            quality_score: 0.0,
            message: e.to_string(),
        }),
    }
}

/// Delete an entry.
///
/// Uses `spawn_blocking` because the underlying `delete_entry` performs
/// CPU-intensive work (BM25 index cleanup, disk persistence) that can
/// take several seconds for entries with 100K+ chunks.
#[tauri::command]
pub async fn kb_entry_delete(
    kb: State<'_, KnowledgeState>,
    entry_id: String,
) -> Result<(), String> {
    let service = Arc::clone(&kb.service);
    tokio::task::spawn_blocking(move || {
        let mut guard = service.blocking_lock();
        if guard.delete_entry(&entry_id) {
            tracing::info!(entry_id, "Entry deleted successfully");
            Ok(())
        } else {
            Err(format!("Entry '{entry_id}' not found"))
        }
    })
    .await
    .map_err(|e| format!("delete task failed: {e}"))?
}

/// Get global knowledge base statistics.
#[tauri::command]
pub async fn kb_stats(
    kb: State<'_, KnowledgeState>,
) -> Result<KbStats, String> {
    let service = kb.service.lock().await;
    let collections = service.list_collections();

    let total_entries: u64 = collections.iter().map(|c| c.stats.entry_count).sum();
    let total_chunks: u64 = collections.iter().map(|c| c.stats.chunk_count).sum();

    Ok(KbStats {
        total_collections: collections.len(),
        total_entries: total_entries as usize,
        total_chunks: total_chunks as usize,
        total_hits: 0,
    })
}

/// Supported file extensions for knowledge ingestion.
const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Markdown
    "md", "markdown", "mdx",
    // Plain text & docs
    "txt", "text", "rst", "adoc", "org", "rtf",
    // Data / config
    "json", "jsonl", "yaml", "yml", "toml", "csv", "tsv",
    "xml", "html", "htm", "svg",
    "ini", "cfg", "conf", "env", "properties",
    // Source code
    "rs", "py", "js", "ts", "jsx", "tsx", "go", "java",
    "c", "h", "cpp", "hpp", "cc", "cs", "rb", "php",
    "swift", "kt", "kts", "scala", "lua", "r", "pl",
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd",
    "sql", "graphql", "gql",
    // Misc text
    "log", "diff", "patch", "tex", "bib",
    "css", "scss", "less", "sass",
    "vue", "svelte", "astro",
    "dockerfile", "makefile", "cmake",
];

/// Expand a folder path into a list of supported files (recursively).
///
/// If the given path is a file, returns it as-is (if its extension is supported).
/// If it is a directory, recursively walks it and collects all files with
/// supported extensions.
#[tauri::command]
pub async fn kb_expand_folder(path: String) -> Result<Vec<String>, String> {
    let root = PathBuf::from(&path);
    if !root.exists() {
        return Err(format!("Path does not exist: {path}"));
    }

    // If it's a single file, just check its extension.
    if root.is_file() {
        if is_supported_extension(&root) {
            return Ok(vec![path]);
        } else {
            return Ok(vec![]); // unsupported file
        }
    }

    // Recursively walk the directory.
    let mut files = Vec::new();
    collect_supported_files(&root, &mut files).map_err(|e| format!("Failed to scan folder: {e}"))?;

    // Sort for deterministic ordering.
    files.sort();

    Ok(files)
}

/// Check if a path has a supported extension for knowledge ingestion.
fn is_supported_extension(path: &std::path::Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Also support extensionless files named like "Dockerfile", "Makefile", etc.
    if ext.is_empty() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            return matches!(lower.as_str(), "dockerfile" | "makefile" | "cmakelists.txt" | "readme" | "license" | "changelog");
        }
        return false;
    }

    SUPPORTED_EXTENSIONS.contains(&ext.as_str())
}

/// Recursively collect files with supported extensions from a directory.
fn collect_supported_files(dir: &std::path::Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip hidden files/directories (starting with '.')
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        if path.is_dir() {
            collect_supported_files(&path, out)?;
        } else if path.is_file() && is_supported_extension(&path) {
            if let Some(s) = path.to_str() {
                out.push(s.to_string());
            }
        }
    }
    Ok(())
}
