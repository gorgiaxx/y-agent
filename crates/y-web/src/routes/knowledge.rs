//! Knowledge base management endpoints.
//!
//! Mirrors all knowledge-related Tauri commands from the GUI.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::routes::events::SseEvent;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CollectionInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entry_count: usize,
    pub chunk_count: usize,
    pub total_bytes: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
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
    pub document_type: Option<String>,
    pub industry: Option<String>,
    pub subcategory: Option<String>,
    pub interpreted_title: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
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
    pub document_type: Option<String>,
    pub industry: Option<String>,
    pub subcategory: Option<String>,
    pub interpreted_title: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SectionInfo {
    pub index: usize,
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Serialize)]
pub struct ChunkInfo {
    pub id: String,
    pub content: String,
    pub token_estimate: usize,
    pub section_index: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchResultItem {
    pub chunk_id: String,
    pub title: String,
    pub content: String,
    pub relevance: f64,
    pub domains: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IngestResult {
    pub success: bool,
    pub entry_id: Option<String>,
    pub chunk_count: usize,
    pub domains: Vec<String>,
    pub quality_score: f32,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BatchIngestResult {
    pub succeeded: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct KbStats {
    pub collections: usize,
    pub entries: usize,
    pub chunks: usize,
    pub hits: u64,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameCollectionRequest {
    pub new_name: String,
}

#[derive(Debug, Deserialize)]
pub struct EntryDetailQuery {
    pub resolution: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub domain: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub source: String,
    pub domain: Option<String>,
    pub collection: String,
    pub use_llm_summary: Option<bool>,
    pub extract_metadata: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct BatchIngestRequest {
    pub sources: Vec<String>,
    pub domain: Option<String>,
    pub collection: String,
    pub use_llm_summary: Option<bool>,
    pub extract_metadata: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMetadataRequest {
    pub document_type: Option<String>,
    pub industry: Option<String>,
    pub subcategory: Option<String>,
    pub interpreted_title: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ExpandFolderRequest {
    pub path: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_knowledge(state: &AppState) -> Result<&Arc<crate::state::KnowledgeState>, ApiError> {
    state
        .knowledge
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Knowledge service not configured".into()))
}

fn entry_to_info(e: &y_knowledge::KnowledgeEntry) -> EntryInfo {
    EntryInfo {
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
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/knowledge/collections`
async fn collection_list(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let service = ks.service.lock().await;
    let collections: Vec<CollectionInfo> = service
        .list_collections()
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
        .collect();
    Ok(Json(collections))
}

/// `POST /api/v1/knowledge/collections`
async fn collection_create(
    State(state): State<AppState>,
    Json(body): Json<CreateCollectionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let mut service = ks.service.lock().await;
    service.create_collection(&body.name, &body.description);

    let collections = service.list_collections();
    let c = collections
        .iter()
        .find(|c| c.name == body.name)
        .ok_or_else(|| ApiError::Internal("Failed to find created collection".into()))?;

    let info = CollectionInfo {
        id: c.id.to_string(),
        name: c.name.clone(),
        description: c.description.clone(),
        entry_count: usize::try_from(c.stats.entry_count).unwrap_or(usize::MAX),
        chunk_count: usize::try_from(c.stats.chunk_count).unwrap_or(usize::MAX),
        total_bytes: c.stats.total_bytes,
        created_at: c.created_at.to_rfc3339(),
    };

    Ok((StatusCode::CREATED, Json(info)))
}

/// `DELETE /api/v1/knowledge/collections/:name`
async fn collection_delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let service = Arc::clone(&ks.service);
    tokio::task::spawn_blocking(move || {
        let mut guard = service.blocking_lock();
        if guard.delete_collection(&name) {
            Ok(Json(serde_json::json!({"message": "deleted"})))
        } else {
            Err(ApiError::NotFound(format!("Collection '{name}' not found")))
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("delete task failed: {e}")))?
}

/// `POST /api/v1/knowledge/collections/:name/rename`
async fn collection_rename(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<RenameCollectionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let mut service = ks.service.lock().await;
    if service.rename_collection(&name, &body.new_name) {
        Ok(Json(serde_json::json!({"message": "renamed"})))
    } else {
        Err(ApiError::BadRequest(format!(
            "Failed to rename '{name}' (not found or name taken)"
        )))
    }
}

/// `GET /api/v1/knowledge/collections/:name/entries`
async fn entry_list(
    State(state): State<AppState>,
    Path(collection): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let service = ks.service.lock().await;
    let entries: Vec<EntryInfo> = service
        .list_entries(&collection)
        .iter()
        .map(|e| entry_to_info(e))
        .collect();
    Ok(Json(entries))
}

/// `GET /api/v1/knowledge/entries/:id`
async fn entry_detail(
    State(state): State<AppState>,
    Path(entry_id): Path<String>,
    Query(_query): Query<EntryDetailQuery>,
) -> Result<impl IntoResponse, ApiError> {
    const MAX_L2_CHUNKS: usize = 200;

    let ks = require_knowledge(&state)?;
    let service = ks.service.lock().await;
    let entry = service
        .get_entry(&entry_id)
        .ok_or_else(|| ApiError::NotFound(format!("Entry '{entry_id}' not found")))?;

    let l0_summary = entry.summary.clone().unwrap_or_default();
    let l1_sections: Vec<SectionInfo> = entry
        .l1_sections
        .iter()
        .map(|s| SectionInfo {
            index: s.index,
            title: s.title.clone(),
            summary: s.content.clone(),
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

    Ok(Json(EntryDetail {
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
    }))
}

/// `DELETE /api/v1/knowledge/entries/:id`
async fn entry_delete(
    State(state): State<AppState>,
    Path(entry_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let service = Arc::clone(&ks.service);
    tokio::task::spawn_blocking(move || {
        let mut guard = service.blocking_lock();
        if guard.delete_entry(&entry_id) {
            Ok(Json(serde_json::json!({"message": "deleted"})))
        } else {
            Err(ApiError::NotFound(format!("Entry '{entry_id}' not found")))
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("delete task failed: {e}")))?
}

/// `PATCH /api/v1/knowledge/entries/:id/metadata`
async fn entry_update_metadata(
    State(state): State<AppState>,
    Path(entry_id): Path<String>,
    Json(body): Json<UpdateMetadataRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let mut service = ks.service.lock().await;
    let entry = service
        .get_entry_mut(&entry_id)
        .ok_or_else(|| ApiError::NotFound(format!("Entry '{entry_id}' not found")))?;

    if let Some(dt) = body.document_type {
        entry.metadata.document_type = Some(dt);
    }
    if let Some(ind) = body.industry {
        entry.metadata.industry = Some(ind);
    }
    if let Some(sub) = body.subcategory {
        entry.metadata.subcategory = Some(sub);
    }
    if let Some(title) = body.interpreted_title {
        entry.metadata.interpreted_title = Some(title);
    }
    if let Some(new_tags) = body.tags {
        entry.tags.clone_from(&new_tags);
        entry.metadata.topics = new_tags;
    }

    service.save_entries_public();
    Ok(Json(serde_json::json!({"message": "updated"})))
}

/// `POST /api/v1/knowledge/search`
async fn kb_search(
    State(state): State<AppState>,
    Json(body): Json<SearchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let service = ks.service.lock().await;
    let params = y_knowledge::tools::KnowledgeSearchParams {
        query: body.query,
        domain: body.domain,
        resolution: "l0".to_string(),
        limit: body.limit.unwrap_or(10),
        collection: None,
    };
    let result = service.search(&params).await;

    let items: Vec<SearchResultItem> = result
        .results
        .iter()
        .map(|r| SearchResultItem {
            chunk_id: r.chunk_id.clone(),
            title: r.title.clone(),
            content: r.content.clone(),
            relevance: r.relevance,
            domains: r.domains.clone(),
        })
        .collect();

    Ok(Json(items))
}

/// `POST /api/v1/knowledge/ingest`
async fn kb_ingest(
    State(state): State<AppState>,
    Json(body): Json<IngestRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let mut service = ks.service.lock().await;
    let params = y_knowledge::tools::KnowledgeIngestParams {
        source: body.source,
        domain: body.domain,
        collection: body.collection,
        use_llm_summary: body.use_llm_summary.unwrap_or(false),
        extract_metadata: body.extract_metadata.unwrap_or(false),
    };

    let result = match service.ingest(&params, "default").await {
        Ok(r) => IngestResult {
            success: r.success,
            entry_id: r.entry_id,
            chunk_count: r.chunk_count,
            domains: r.domains,
            quality_score: r.quality_score,
            message: r.message,
        },
        Err(e) => IngestResult {
            success: false,
            entry_id: None,
            chunk_count: 0,
            domains: vec![],
            quality_score: 0.0,
            message: e.to_string(),
        },
    };

    Ok(Json(result))
}

/// `POST /api/v1/knowledge/ingest-batch`
async fn kb_ingest_batch(
    State(state): State<AppState>,
    Json(body): Json<BatchIngestRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let total = body.sources.len();
    let mut succeeded = 0usize;
    let mut errors = Vec::<String>::new();

    let llm_summary = body.use_llm_summary.unwrap_or(false);
    let metadata_flag = body.extract_metadata.unwrap_or(false);
    let service_handle = Arc::clone(&ks.service);

    for (i, source) in body.sources.iter().enumerate() {
        let _ = state.event_tx.send(SseEvent::KbBatchProgress {
            current: i + 1,
            total,
            source: source.clone(),
        });

        let params = y_knowledge::tools::KnowledgeIngestParams {
            source: source.clone(),
            domain: body.domain.clone(),
            collection: body.collection.clone(),
            use_llm_summary: llm_summary,
            extract_metadata: metadata_flag,
        };

        let (result, entry_info) = {
            let mut guard = service_handle.lock().await;
            let r = guard.ingest(&params, "default").await;
            let info = if let Ok(ref res) = r {
                res.entry_id
                    .as_ref()
                    .and_then(|eid| guard.get_entry(eid).map(entry_to_info))
            } else {
                None
            };
            (r, info)
        };

        match result {
            Ok(r) if r.success => {
                succeeded += 1;
                let _ = state
                    .event_tx
                    .send(SseEvent::KbEntryIngested(serde_json::json!({
                        "entry_id": r.entry_id.unwrap_or_default(),
                        "source": source,
                        "collection": body.collection,
                        "current": i + 1,
                        "total": total,
                        "entry": entry_info,
                    })));
            }
            Ok(r) => {
                errors.push(format!("{source}: {}", r.message));
            }
            Err(e) => {
                errors.push(format!("{source}: {e}"));
            }
        }
    }

    Ok(Json(BatchIngestResult {
        succeeded,
        failed: errors.len(),
        errors,
    }))
}

/// `POST /api/v1/knowledge/expand-folder`
async fn kb_expand_folder(
    Json(body): Json<ExpandFolderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let root = std::path::PathBuf::from(&body.path);
    if !root.exists() {
        return Err(ApiError::NotFound(format!(
            "Path does not exist: {}",
            body.path
        )));
    }

    if root.is_file() {
        if y_knowledge::supported_formats::is_supported(&root) {
            return Ok(Json(vec![body.path]));
        }
        return Ok(Json(Vec::<String>::new()));
    }

    let files = y_knowledge::supported_formats::expand_directory(&root)
        .map_err(|e| ApiError::Internal(format!("Failed to scan folder: {e}")))?;

    Ok(Json(
        files
            .into_iter()
            .filter_map(|p| p.to_str().map(String::from))
            .collect::<Vec<String>>(),
    ))
}

/// `GET /api/v1/knowledge/stats`
async fn kb_stats(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let ks = require_knowledge(&state)?;
    let service = ks.service.lock().await;
    let collections = service.list_collections();

    let total_entries: u64 = collections.iter().map(|c| c.stats.entry_count).sum();
    let total_chunks: u64 = collections.iter().map(|c| c.stats.chunk_count).sum();

    Ok(Json(KbStats {
        collections: collections.len(),
        entries: usize::try_from(total_entries).unwrap_or(usize::MAX),
        chunks: usize::try_from(total_chunks).unwrap_or(usize::MAX),
        hits: 0,
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Knowledge route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/knowledge/collections",
            get(collection_list).post(collection_create),
        )
        .route(
            "/api/v1/knowledge/collections/{name}",
            delete(collection_delete),
        )
        .route(
            "/api/v1/knowledge/collections/{name}/rename",
            post(collection_rename),
        )
        .route(
            "/api/v1/knowledge/collections/{name}/entries",
            get(entry_list),
        )
        .route(
            "/api/v1/knowledge/entries/{id}",
            get(entry_detail).delete(entry_delete),
        )
        .route(
            "/api/v1/knowledge/entries/{id}/metadata",
            patch(entry_update_metadata),
        )
        .route("/api/v1/knowledge/search", post(kb_search))
        .route("/api/v1/knowledge/ingest", post(kb_ingest))
        .route("/api/v1/knowledge/ingest-batch", post(kb_ingest_batch))
        .route("/api/v1/knowledge/expand-folder", post(kb_expand_folder))
        .route("/api/v1/knowledge/stats", get(kb_stats))
}
