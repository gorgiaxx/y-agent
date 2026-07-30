//! Knowledge management command handlers — collection CRUD, entry browsing,
//! search, ingestion, and statistics.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, State};

use y_knowledge::config::KnowledgeConfig;
use y_service::knowledge_service::{
    CollectionInfo, EntryDetail, EntryInfo, KnowledgeMetadataUpdate,
    KnowledgeSearchItem as SearchResultItem, KnowledgeService, KnowledgeStats as KbStats,
};

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
            y_knowledge::middleware::InjectKnowledge<y_knowledge::tokenizer::AutoTokenizer>,
        >,
    > {
        self.service.lock().await.knowledge_handle()
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct IngestResult {
    pub success: bool,
    pub entry_id: Option<String>,
    pub chunk_count: usize,
    pub domains: Vec<String>,
    pub quality_score: f32,
    pub message: String,
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
    Ok(service.collection_infos())
}

/// Create a new collection.
#[tauri::command]
pub async fn kb_collection_create(
    kb: State<'_, KnowledgeState>,
    name: String,
    description: String,
) -> Result<CollectionInfo, String> {
    let mut service = kb.service.lock().await;
    Ok(service.create_collection_info(&name, &description))
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
    Ok(service.entry_infos(&collection))
}

/// Get entry detail with L0/L1/L2 content.
#[tauri::command]
pub async fn kb_entry_detail(
    kb: State<'_, KnowledgeState>,
    entry_id: String,
    _resolution: String,
) -> Result<EntryDetail, String> {
    let service = kb.service.lock().await;
    service
        .entry_detail_info(&entry_id)
        .ok_or_else(|| format!("Entry '{entry_id}' not found"))
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
    Ok(service.search_items(&params).await)
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
    Ok(service.stats())
}

/// Expand a folder path into a list of supported files (recursively).
///
/// Delegates to `y_knowledge::supported_formats` for extension checks and
/// recursive directory walking.
#[tauri::command]
pub async fn kb_expand_folder(path: String) -> Result<Vec<String>, String> {
    KnowledgeService::expand_supported_sources(&path).map_err(|error| error.to_string())
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
                res.entry_id.as_ref().and_then(|eid| guard.entry_info(eid))
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
    service
        .update_entry_metadata(
            &entry_id,
            KnowledgeMetadataUpdate {
                document_type,
                industry,
                subcategory,
                interpreted_title,
                tags,
            },
        )
        .then_some(())
        .ok_or_else(|| format!("Entry '{entry_id}' not found"))
}
