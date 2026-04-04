//! Knowledge management command handlers — collection CRUD, entry browsing,
//! search, ingestion, and statistics.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, State};

use y_knowledge::config::KnowledgeConfig;
use y_service::knowledge_service::KnowledgeService;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip a title prefix from the beginning of content text.
///
/// When L1 sections are chunked, the chunk content often starts with the
/// heading line that was also extracted as the section title. This helper
/// removes that redundant prefix so the GUI displays title and body
/// separately without duplication.
fn strip_title_prefix(content: &str, title: &str) -> String {
    let trimmed = content.trim_start();

    // Try stripping markdown heading markers (e.g. "## Title\n...")
    for prefix in ["#### ", "### ", "## ", "# "] {
        let heading = format!("{prefix}{title}");
        if let Some(rest) = trimmed.strip_prefix(&heading) {
            let rest = rest.trim_start();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }

    // Try stripping plain title at the start of content.
    if let Some(rest) = trimmed.strip_prefix(title) {
        let rest = rest.trim_start();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }

    content.to_string()
}

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
    /// knowledge panel, context pipeline, and `KnowledgeSearch` tool all operate
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
    /// Used to share the retriever with `KnowledgeSearch` tool and
    /// `KnowledgeContextProvider` for chat integration.
    #[allow(dead_code)]
    pub async fn knowledge_handle(
        &self,
    ) -> std::sync::Arc<
        std::sync::Mutex<
            y_knowledge::middleware::InjectKnowledge<y_knowledge::tokenizer::SimpleTokenizer>,
        >,
    > {
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
    /// Multi-dimensional metadata fields.
    pub document_type: Option<String>,
    pub industry: Option<String>,
    pub subcategory: Option<String>,
    pub interpreted_title: Option<String>,
    pub tags: Vec<String>,
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
    /// Multi-dimensional metadata fields.
    pub document_type: Option<String>,
    pub industry: Option<String>,
    pub subcategory: Option<String>,
    pub interpreted_title: Option<String>,
    pub tags: Vec<String>,
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
    pub relevance: f64,
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
    pub collections: usize,
    pub entries: usize,
    pub chunks: usize,
    pub hits: u64,
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
            entry_count: usize::try_from(c.stats.entry_count).unwrap_or(usize::MAX),
            chunk_count: usize::try_from(c.stats.chunk_count).unwrap_or(usize::MAX),
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
        entry_count: usize::try_from(c.stats.entry_count).unwrap_or(usize::MAX),
        chunk_count: usize::try_from(c.stats.chunk_count).unwrap_or(usize::MAX),
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
            hit_count: u64::from(e.hit_num),
            updated_at: e.refreshed_at.to_rfc3339(),
            document_type: e.metadata.document_type.clone(),
            industry: e.metadata.industry.clone(),
            subcategory: e.metadata.subcategory.clone(),
            interpreted_title: e.metadata.interpreted_title.clone(),
            tags: e.tags.clone(),
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
    // Cap at 200 chunks to avoid flooding the IPC channel / UI.
    const MAX_L2_CHUNKS: usize = 200;

    let service = kb.service.lock().await;
    let entry = service
        .get_entry(&entry_id)
        .ok_or_else(|| format!("Entry '{entry_id}' not found"))?;

    let l0_summary = entry.summary.clone().unwrap_or_default();
    let l1_sections: Vec<SectionInfo> = entry
        .l1_sections
        .iter()
        .map(|s| {
            // Strip the title from the beginning of the content to avoid
            // redundant display (title shown as heading, content as body).
            let summary = strip_title_prefix(&s.content, &s.title);
            SectionInfo {
                index: s.index,
                title: s.title.clone(),
                summary,
            }
        })
        .collect();

    let total_chunk_count = entry.chunks.len();
    let l2_chunks: Vec<ChunkInfo> = entry
        .chunks
        .iter()
        .enumerate()
        .take(MAX_L2_CHUNKS)
        .map(|(i, content)| ChunkInfo {
            id: format!("{}-{}", entry.id, i),
            content: content.clone(),
            token_estimate: content.len() / 4,
            section_index: i,
        })
        .collect();

    Ok(EntryDetail {
        id: entry.id.to_string(),
        title: entry.source.title.clone(),
        source_uri: entry.source.uri.clone(),
        domains: entry.domains.clone(),
        quality_score: entry.quality_score,
        state: entry.state.to_string(),
        hit_count: u64::from(entry.hit_num),
        total_chunk_count,
        l0_summary,
        l1_sections,
        l2_chunks,
        document_type: entry.metadata.document_type.clone(),
        industry: entry.metadata.industry.clone(),
        subcategory: entry.metadata.subcategory.clone(),
        interpreted_title: entry.metadata.interpreted_title.clone(),
        tags: entry.tags.clone(),
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
    let result = service.search(&params).await;

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
    _app: tauri::AppHandle,
    kb: State<'_, KnowledgeState>,
    source: String,
    domain: Option<String>,
    collection: String,
    use_llm_summary: Option<bool>,
    extract_metadata: Option<bool>,
) -> Result<IngestResult, String> {
    let llm_summary = use_llm_summary.unwrap_or(false);
    let metadata_flag = extract_metadata.unwrap_or(false);

    let mut service = kb.service.lock().await;
    let params = y_knowledge::tools::KnowledgeIngestParams {
        source,
        domain,
        collection,
        use_llm_summary: llm_summary,
        extract_metadata: metadata_flag,
    };

    let result = match service.ingest(&params, "default").await {
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
    };

    result
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
pub async fn kb_stats(kb: State<'_, KnowledgeState>) -> Result<KbStats, String> {
    let service = kb.service.lock().await;
    let collections = service.list_collections();

    let total_entries: u64 = collections.iter().map(|c| c.stats.entry_count).sum();
    let total_chunks: u64 = collections.iter().map(|c| c.stats.chunk_count).sum();

    Ok(KbStats {
        collections: collections.len(),
        entries: usize::try_from(total_entries).unwrap_or(usize::MAX),
        chunks: usize::try_from(total_chunks).unwrap_or(usize::MAX),
        hits: 0,
    })
}

/// Expand a folder path into a list of supported files (recursively).
///
/// Delegates to `y_knowledge::supported_formats` for extension checks and
/// recursive directory walking.
#[tauri::command]
pub async fn kb_expand_folder(path: String) -> Result<Vec<String>, String> {
    let root = PathBuf::from(&path);
    if !root.exists() {
        return Err(format!("Path does not exist: {path}"));
    }

    // Single file: check extension via the knowledge crate.
    if root.is_file() {
        if y_knowledge::supported_formats::is_supported(&root) {
            return Ok(vec![path]);
        }
        return Ok(vec![]);
    }

    // Directory: recursively collect supported files.
    let files = y_knowledge::supported_formats::expand_directory(&root)
        .map_err(|e| format!("Failed to scan folder: {e}"))?;

    Ok(files
        .into_iter()
        .filter_map(|p| p.to_str().map(String::from))
        .collect())
}

/// Progress event payload emitted during batch ingestion.
#[derive(Debug, Serialize, Clone)]
pub struct BatchProgressPayload {
    pub current: usize,
    pub total: usize,
    pub source: String,
}

/// Event payload emitted after each file is successfully ingested.
///
/// Includes the full `EntryInfo` so the frontend can merge the new entry
/// directly into its local state without making additional IPC calls
/// (which would compete for the same service mutex and block the UI).
#[derive(Debug, Serialize, Clone)]
pub struct EntryIngestedPayload {
    pub entry_id: String,
    pub source: String,
    pub collection: String,
    pub current: usize,
    pub total: usize,
    /// Full entry info for direct frontend state merge.
    pub entry: Option<EntryInfo>,
}

/// Result summary for a batch ingestion operation.
#[derive(Debug, Serialize, Clone)]
pub struct BatchIngestResult {
    pub succeeded: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

/// Ingest multiple files in a single backend call.
///
/// The service mutex is scoped tightly around each individual `ingest`
/// call so that other Tauri commands (entry list, entry detail, etc.)
/// can proceed between files instead of being blocked for the entire
/// batch.
///
/// Emits:
/// - `kb:batch_progress` before each file (counter update).
/// - `kb:entry_ingested` after each successful file with the full
///    `EntryInfo` payload so the frontend can merge the new entry
///    directly without additional IPC round-trips.
#[tauri::command]
pub async fn kb_ingest_batch(
    app: tauri::AppHandle,
    kb: State<'_, KnowledgeState>,
    sources: Vec<String>,
    domain: Option<String>,
    collection: String,
    use_llm_summary: Option<bool>,
    extract_metadata: Option<bool>,
) -> Result<BatchIngestResult, String> {
    let total = sources.len();
    let mut succeeded = 0usize;
    let mut errors = Vec::<String>::new();

    let llm_summary = use_llm_summary.unwrap_or(false);
    let metadata_flag = extract_metadata.unwrap_or(false);

    // Clone the Arc so we can re-lock per file without borrowing `kb`
    // across the entire loop.
    let service_handle = Arc::clone(&kb.service);

    for (i, source) in sources.iter().enumerate() {
        // Emit progress before each file.
        let _ = app.emit(
            "kb:batch_progress",
            BatchProgressPayload {
                current: i + 1,
                total,
                source: source.clone(),
            },
        );

        let params = y_knowledge::tools::KnowledgeIngestParams {
            source: source.clone(),
            domain: domain.clone(),
            collection: collection.clone(),
            use_llm_summary: llm_summary,
            extract_metadata: metadata_flag,
        };

        // Acquire the lock, ingest, read entry info, then DROP the guard
        // so other commands can access the service between files.
        let (result, entry_info) = {
            let mut guard = service_handle.lock().await;
            let r = guard.ingest(&params, "default").await;
            // If ingest succeeded, read the entry data while still
            // holding the lock so we can include it in the event
            // (avoids the frontend having to make a competing IPC call).
            let info = if let Ok(ref res) = r {
                res.entry_id.as_ref().and_then(|eid| {
                    guard.get_entry(eid).map(|e| EntryInfo {
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
                        hit_count: u64::from(e.hit_num),
                        updated_at: e.refreshed_at.to_rfc3339(),
                        document_type: e.metadata.document_type.clone(),
                        industry: e.metadata.industry.clone(),
                        subcategory: e.metadata.subcategory.clone(),
                        interpreted_title: e.metadata.interpreted_title.clone(),
                        tags: e.tags.clone(),
                    })
                })
            } else {
                None
            };
            (r, info)
        };
        // -- lock released here --

        match result {
            Ok(r) if r.success => {
                succeeded += 1;
                // Notify frontend with inline entry data so it can
                // update state directly without backend calls.
                let _ = app.emit(
                    "kb:entry_ingested",
                    EntryIngestedPayload {
                        entry_id: r.entry_id.unwrap_or_default(),
                        source: source.clone(),
                        collection: collection.clone(),
                        current: i + 1,
                        total,
                        entry: entry_info,
                    },
                );
            }
            Ok(r) => {
                errors.push(format!("{source}: {}", r.message));
            }
            Err(e) => {
                errors.push(format!("{source}: {e}"));
            }
        }
    }

    Ok(BatchIngestResult {
        succeeded,
        failed: errors.len(),
        errors,
    })
}

/// Update metadata fields for a knowledge entry.
#[tauri::command]
pub async fn kb_entry_update_metadata(
    kb: State<'_, KnowledgeState>,
    entry_id: String,
    document_type: Option<String>,
    industry: Option<String>,
    subcategory: Option<String>,
    interpreted_title: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<(), String> {
    let mut service = kb.service.lock().await;
    let entry = service
        .get_entry_mut(&entry_id)
        .ok_or_else(|| format!("Entry '{entry_id}' not found"))?;

    if let Some(dt) = document_type {
        entry.metadata.document_type = Some(dt);
    }
    if let Some(ind) = industry {
        entry.metadata.industry = Some(ind);
    }
    if let Some(sub) = subcategory {
        entry.metadata.subcategory = Some(sub);
    }
    if let Some(title) = interpreted_title {
        entry.metadata.interpreted_title = Some(title);
    }
    if let Some(new_tags) = tags {
        entry.tags.clone_from(&new_tags);
        entry.metadata.topics = new_tags;
    }

    // Persist changes.
    service.save_entries_public();
    Ok(())
}
