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
use tokio::sync::Mutex;
use y_service::knowledge_service::{KnowledgeMetadataUpdate, KnowledgeService};

use crate::error::ApiError;
use crate::routes::events::SseEvent;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

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
    pub collection: Option<String>,
    pub resolution: Option<String>,
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

fn knowledge_service(state: &AppState) -> &Arc<Mutex<KnowledgeService>> {
    &state.container.knowledge_service
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/knowledge/collections`
async fn collection_list(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let service = knowledge_service(&state).lock().await;
    Ok(Json(service.collection_infos()))
}

/// `POST /api/v1/knowledge/collections`
async fn collection_create(
    State(state): State<AppState>,
    Json(body): Json<CreateCollectionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ks = knowledge_service(&state);
    let mut service = ks.lock().await;
    let info = service.create_collection_info(&body.name, &body.description);

    Ok((StatusCode::CREATED, Json(info)))
}

/// `DELETE /api/v1/knowledge/collections/:name`
async fn collection_delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let service = Arc::clone(knowledge_service(&state));
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
    let mut service = knowledge_service(&state).lock().await;
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
    let service = knowledge_service(&state).lock().await;
    Ok(Json(service.entry_infos(&collection)))
}

/// `GET /api/v1/knowledge/entries/:id`
async fn entry_detail(
    State(state): State<AppState>,
    Path(entry_id): Path<String>,
    Query(_query): Query<EntryDetailQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let service = knowledge_service(&state).lock().await;
    let detail = service
        .entry_detail_info(&entry_id)
        .ok_or_else(|| ApiError::NotFound(format!("Entry '{entry_id}' not found")))?;
    Ok(Json(detail))
}

/// `DELETE /api/v1/knowledge/entries/:id`
async fn entry_delete(
    State(state): State<AppState>,
    Path(entry_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let service = Arc::clone(knowledge_service(&state));
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
    let mut service = knowledge_service(&state).lock().await;
    service
        .update_entry_metadata(
            &entry_id,
            KnowledgeMetadataUpdate {
                document_type: body.document_type,
                industry: body.industry,
                subcategory: body.subcategory,
                interpreted_title: body.interpreted_title,
                tags: body.tags,
            },
        )
        .then(|| Json(serde_json::json!({"message": "updated"})))
        .ok_or_else(|| ApiError::NotFound(format!("Entry '{entry_id}' not found")))
}

/// `POST /api/v1/knowledge/search`
async fn kb_search(
    State(state): State<AppState>,
    Json(body): Json<SearchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let service = knowledge_service(&state).lock().await;
    let params = y_knowledge::tools::KnowledgeSearchParams {
        query: body.query,
        domain: body.domain,
        resolution: body.resolution.unwrap_or_else(|| "l0".to_string()),
        limit: body.limit.unwrap_or(10),
        collection: body.collection,
    };
    Ok(Json(service.search_items(&params).await))
}

/// `POST /api/v1/knowledge/ingest`
async fn kb_ingest(
    State(state): State<AppState>,
    Json(body): Json<IngestRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut service = knowledge_service(&state).lock().await;
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
    let total = body.sources.len();
    let mut succeeded = 0usize;
    let mut errors = Vec::<String>::new();

    let llm_summary = body.use_llm_summary.unwrap_or(false);
    let metadata_flag = body.extract_metadata.unwrap_or(false);
    let service_handle = Arc::clone(knowledge_service(&state));

    for (i, source) in body.sources.iter().enumerate() {
        let _ = state.event_tx.send(
            SseEvent::KbBatchProgress {
                current: i + 1,
                total,
                source: source.clone(),
            }
            .into(),
        );

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
                res.entry_id.as_ref().and_then(|eid| guard.entry_info(eid))
            } else {
                None
            };
            (r, info)
        };

        match result {
            Ok(r) if r.success => {
                succeeded += 1;
                let _ = state.event_tx.send(
                    SseEvent::KbEntryIngested(serde_json::json!({
                        "entry_id": r.entry_id.unwrap_or_default(),
                        "source": source,
                        "collection": body.collection,
                        "current": i + 1,
                        "total": total,
                        "entry": entry_info,
                    }))
                    .into(),
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
    KnowledgeService::expand_supported_sources(&body.path)
        .map(Json)
        .map_err(|error| ApiError::BadRequest(error.to_string()))
}

/// `GET /api/v1/knowledge/stats`
async fn kb_stats(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let service = knowledge_service(&state).lock().await;
    Ok(Json(service.stats()))
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
