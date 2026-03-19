//! Vector indexer: Qdrant collection management (feature-gated).
//!
//! This module provides the interface for vector indexing.
//!
//! - Without `vector_qdrant` feature: no-op placeholder.
//! - With `vector_qdrant` feature: full Qdrant client integration.

use crate::error::KnowledgeError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A point to be stored in the vector index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorPoint {
    /// Unique point ID (must be a UUID string or numeric).
    pub id: String,
    /// The embedding vector.
    pub vector: Vec<f32>,
    /// Payload key-value pairs stored with the point.
    pub payload: HashMap<String, serde_json::Value>,
}

/// A search result from the vector index.
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// Point ID.
    pub id: String,
    /// Similarity score.
    pub score: f32,
    /// Payload retrieved with the point.
    pub payload: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// No-op placeholder (default, no feature)
// ---------------------------------------------------------------------------

/// Placeholder for vector indexing operations.
///
/// Without the `vector_qdrant` feature, all operations are no-ops.
/// Enable the feature for production Qdrant integration.
#[cfg(not(feature = "vector_qdrant"))]
#[derive(Debug, Default)]
pub struct VectorIndexer;

#[cfg(not(feature = "vector_qdrant"))]
impl VectorIndexer {
    pub fn new() -> Self {
        Self
    }

    /// Create a collection (no-op without Qdrant).
    pub fn create_collection(
        &self,
        _name: &str,
        _vector_size: u64,
    ) -> Result<(), KnowledgeError> {
        tracing::warn!(
            "VectorIndexer: create_collection is a no-op (enable vector_qdrant feature)"
        );
        Ok(())
    }

    /// Upsert points (no-op without Qdrant).
    pub fn upsert(
        &self,
        _collection: &str,
        _points: Vec<VectorPoint>,
    ) -> Result<(), KnowledgeError> {
        tracing::warn!("VectorIndexer: upsert is a no-op (enable vector_qdrant feature)");
        Ok(())
    }

    /// Delete points (no-op without Qdrant).
    pub fn delete(&self, _collection: &str, _ids: &[String]) -> Result<(), KnowledgeError> {
        tracing::warn!("VectorIndexer: delete is a no-op (enable vector_qdrant feature)");
        Ok(())
    }

    /// Search for nearest neighbors (no-op without Qdrant).
    pub fn search(
        &self,
        _collection: &str,
        _query_vector: Vec<f32>,
        _top_k: u64,
    ) -> Result<Vec<VectorSearchResult>, KnowledgeError> {
        tracing::warn!("VectorIndexer: search is a no-op (enable vector_qdrant feature)");
        Ok(vec![])
    }

    /// Check if a collection exists (no-op without Qdrant).
    pub fn collection_exists(&self, _name: &str) -> Result<bool, KnowledgeError> {
        Ok(false)
    }

    /// Delete a collection (no-op without Qdrant).
    pub fn delete_collection(&self, _name: &str) -> Result<(), KnowledgeError> {
        tracing::warn!(
            "VectorIndexer: delete_collection is a no-op (enable vector_qdrant feature)"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Qdrant implementation (vector_qdrant feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "vector_qdrant")]
use qdrant_client::{
    qdrant::{
        CreateCollectionBuilder, DeleteCollectionBuilder, DeletePointsBuilder, Distance, Filter,
        PointStruct, PointsIdsList, QueryPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
    },
    Qdrant,
};

/// Vector indexer backed by Qdrant.
///
/// Manages Qdrant collections for knowledge base vector storage.
/// Uses the Qdrant gRPC API via `qdrant-client` crate.
#[cfg(feature = "vector_qdrant")]
#[derive(Debug)]
pub struct VectorIndexer {
    client: Qdrant,
}

#[cfg(feature = "vector_qdrant")]
impl VectorIndexer {
    /// Create a new `VectorIndexer` connected to a Qdrant instance.
    ///
    /// # Arguments
    /// * `url` - Qdrant gRPC URL (e.g., `http://localhost:6334`)
    /// * `api_key` - Optional API key for authentication
    pub fn new_with_url(url: &str, api_key: Option<&str>) -> Result<Self, KnowledgeError> {
        let mut builder = Qdrant::from_url(url);
        if let Some(key) = api_key {
            builder = builder.api_key(key);
        }
        let client = builder
            .build()
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to connect to Qdrant: {e}"),
            })?;
        Ok(Self { client })
    }

    /// Create a new Qdrant collection with cosine distance.
    ///
    /// If the collection already exists, this is a no-op.
    pub async fn create_collection(
        &self,
        name: &str,
        vector_size: u64,
    ) -> Result<(), KnowledgeError> {
        // Check if collection already exists.
        let exists = self.collection_exists(name).await?;
        if exists {
            tracing::info!("Collection '{}' already exists, skipping creation", name);
            return Ok(());
        }

        self.client
            .create_collection(
                CreateCollectionBuilder::new(name)
                    .vectors_config(VectorParamsBuilder::new(vector_size, Distance::Cosine)),
            )
            .await
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to create collection '{name}': {e}"),
            })?;

        tracing::info!("Created Qdrant collection '{}' (dim={})", name, vector_size);
        Ok(())
    }

    /// Upsert points into a collection.
    pub async fn upsert(
        &self,
        collection: &str,
        points: Vec<VectorPoint>,
    ) -> Result<(), KnowledgeError> {
        let qdrant_points: Vec<PointStruct> = points
            .into_iter()
            .map(|p| {
                let payload: HashMap<String, qdrant_client::qdrant::Value> = p
                    .payload
                    .into_iter()
                    .map(|(k, v)| (k, json_to_qdrant_value(v)))
                    .collect();

                PointStruct::new(p.id, p.vector, payload)
            })
            .collect();

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection, qdrant_points))
            .await
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to upsert points: {e}"),
            })?;

        Ok(())
    }

    /// Delete points by ID from a collection.
    pub async fn delete(&self, collection: &str, ids: &[String]) -> Result<(), KnowledgeError> {
        let point_ids: Vec<qdrant_client::qdrant::PointId> = ids
            .iter()
            .map(|id| qdrant_client::qdrant::PointId::from(id.clone()))
            .collect();

        self.client
            .delete_points(
                DeletePointsBuilder::new(collection).points(PointsIdsList { ids: point_ids }),
            )
            .await
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to delete points: {e}"),
            })?;

        Ok(())
    }

    /// Search for nearest neighbors in a collection.
    pub async fn search(
        &self,
        collection: &str,
        query_vector: Vec<f32>,
        top_k: u64,
    ) -> Result<Vec<VectorSearchResult>, KnowledgeError> {
        let response = self
            .client
            .query(
                QueryPointsBuilder::new(collection)
                    .query(query_vector)
                    .limit(top_k)
                    .with_payload(true),
            )
            .await
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to search: {e}"),
            })?;

        let results = response
            .result
            .into_iter()
            .map(|point| {
                let id = match point.id {
                    Some(pid) => format!("{pid:?}"),
                    None => String::new(),
                };
                let payload: HashMap<String, serde_json::Value> = point
                    .payload
                    .into_iter()
                    .map(|(k, v)| (k, qdrant_value_to_json(v)))
                    .collect();

                VectorSearchResult {
                    id,
                    score: point.score,
                    payload,
                }
            })
            .collect();

        Ok(results)
    }

    /// Check if a collection exists.
    pub async fn collection_exists(&self, name: &str) -> Result<bool, KnowledgeError> {
        self.client
            .collection_exists(name)
            .await
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to check collection existence: {e}"),
            })
    }

    /// Delete a collection.
    pub async fn delete_collection(&self, name: &str) -> Result<(), KnowledgeError> {
        self.client
            .delete_collection(DeleteCollectionBuilder::new(name))
            .await
            .map_err(|e| KnowledgeError::VectorStoreError {
                message: format!("failed to delete collection '{name}': {e}"),
            })?;

        tracing::info!("Deleted Qdrant collection '{}'", name);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// JSON ↔ Qdrant Value conversion helpers (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "vector_qdrant")]
fn json_to_qdrant_value(value: serde_json::Value) -> qdrant_client::qdrant::Value {
    use qdrant_client::qdrant::value::Kind;

    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Kind::IntegerValue(i)
            } else {
                Kind::DoubleValue(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Kind::StringValue(s),
        serde_json::Value::Array(arr) => {
            let values: Vec<qdrant_client::qdrant::Value> =
                arr.into_iter().map(json_to_qdrant_value).collect();
            Kind::ListValue(qdrant_client::qdrant::ListValue { values })
        }
        serde_json::Value::Object(map) => {
            let fields: HashMap<String, qdrant_client::qdrant::Value> = map
                .into_iter()
                .map(|(k, v)| (k, json_to_qdrant_value(v)))
                .collect();
            Kind::StructValue(qdrant_client::qdrant::Struct { fields })
        }
    };

    qdrant_client::qdrant::Value { kind: Some(kind) }
}

#[cfg(feature = "vector_qdrant")]
fn qdrant_value_to_json(value: qdrant_client::qdrant::Value) -> serde_json::Value {
    use qdrant_client::qdrant::value::Kind;

    match value.kind {
        Some(Kind::NullValue(_)) | None => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
        Some(Kind::IntegerValue(i)) => serde_json::json!(i),
        Some(Kind::DoubleValue(d)) => serde_json::json!(d),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s),
        Some(Kind::ListValue(list)) => {
            let arr: Vec<serde_json::Value> =
                list.values.into_iter().map(qdrant_value_to_json).collect();
            serde_json::Value::Array(arr)
        }
        Some(Kind::StructValue(st)) => {
            let obj: serde_json::Map<String, serde_json::Value> = st
                .fields
                .into_iter()
                .map(|(k, v)| (k, qdrant_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_point_creation() {
        let mut payload = HashMap::new();
        payload.insert("domain".to_string(), serde_json::json!("rust"));
        payload.insert("chunk_id".to_string(), serde_json::json!("doc-1-L1-0"));

        let point = VectorPoint {
            id: "point-1".to_string(),
            vector: vec![0.1, 0.2, 0.3],
            payload,
        };

        assert_eq!(point.id, "point-1");
        assert_eq!(point.vector.len(), 3);
        assert_eq!(point.payload["domain"], serde_json::json!("rust"));
    }

    #[test]
    fn test_vector_point_serialization() {
        let mut payload = HashMap::new();
        payload.insert("key".to_string(), serde_json::json!("value"));

        let point = VectorPoint {
            id: "p1".to_string(),
            vector: vec![1.0, 2.0],
            payload,
        };

        let json = serde_json::to_string(&point).expect("serialize");
        let deserialized: VectorPoint = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.id, "p1");
        assert_eq!(deserialized.vector, vec![1.0, 2.0]);
    }

    #[cfg(not(feature = "vector_qdrant"))]
    #[test]
    fn test_noop_indexer_operations() {
        let indexer = VectorIndexer::new();
        // All operations should succeed as no-ops.
        assert!(indexer.create_collection("test", 128).is_ok());
        assert!(indexer.upsert("test", vec![]).is_ok());
        assert!(indexer.delete("test", &[]).is_ok());
        let results = indexer.search("test", vec![0.1], 5).unwrap();
        assert!(results.is_empty());
        assert!(!indexer.collection_exists("test").unwrap());
        assert!(indexer.delete_collection("test").is_ok());
    }
}
