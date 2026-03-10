//! Skill registry traits and associated types.
//!
//! Design reference: skills-knowledge-design.md, skill-versioning-evolution-design.md
//!
//! Skills are LLM-instruction-only artifacts (no embedded tools or scripts).
//! They use a tree-indexed proprietary format with compact root documents
//! (< 2,000 tokens) and on-demand sub-document loading.
//!
//! Version control uses a content-addressable store with JSONL reflog
//! (Git-like deduplication with trivial rollback).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{SkillId, Timestamp};

// ---------------------------------------------------------------------------
// Skill types
// ---------------------------------------------------------------------------

/// A skill manifest (the root document, capped at 2,000 tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub id: SkillId,
    pub name: String,
    pub description: String,
    pub version: SkillVersion,
    /// Domain tags for relevance matching.
    pub tags: Vec<String>,
    /// Task patterns this skill applies to.
    pub trigger_patterns: Vec<String>,
    /// Referenced knowledge base collections.
    #[serde(default)]
    pub knowledge_bases: Vec<String>,
    /// Root instruction content (the LLM-facing instructions).
    pub root_content: String,
    /// Sub-document references (loaded on demand).
    #[serde(default)]
    pub sub_documents: Vec<SubDocumentRef>,
    /// Estimated token count for the root content.
    pub token_estimate: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Reference to a sub-document within a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubDocumentRef {
    pub id: String,
    pub title: String,
    /// When to load this sub-document.
    pub load_condition: String,
    pub token_estimate: u32,
}

/// Skill version identifier (content-addressable hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SkillVersion(pub String);

impl std::fmt::Display for SkillVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A compact skill entry for context injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub id: SkillId,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub token_estimate: u32,
}

/// Sub-document content (loaded on demand).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubDocumentContent {
    pub id: String,
    pub title: String,
    pub content: String,
    pub token_estimate: u32,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from skill operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("skill not found: {id}")]
    NotFound { id: String },

    #[error("skill version not found: {version}")]
    VersionNotFound { version: String },

    #[error("sub-document not found: {doc_id} in skill {skill_id}")]
    SubDocumentNotFound { skill_id: String, doc_id: String },

    #[error("ingestion failed: {message}")]
    IngestionError { message: String },

    #[error("token budget exceeded: root content is {actual} tokens, max is {max}")]
    TokenBudgetExceeded { actual: u32, max: u32 },

    #[error("storage error: {message}")]
    StorageError { message: String },

    #[error("{message}")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Registry for managing skills with lazy loading and versioning.
#[async_trait]
pub trait SkillRegistry: Send + Sync {
    /// Get skill summaries relevant to a query (for context injection).
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SkillSummary>, SkillError>;

    /// Get the full manifest for a skill (root content).
    async fn get_manifest(&self, id: &SkillId) -> Result<SkillManifest, SkillError>;

    /// Load a sub-document on demand.
    async fn load_sub_document(
        &self,
        skill_id: &SkillId,
        doc_id: &str,
    ) -> Result<SubDocumentContent, SkillError>;

    /// Register a new skill or update an existing one (creates a new version).
    async fn register(&self, manifest: SkillManifest) -> Result<SkillVersion, SkillError>;

    /// Rollback a skill to a previous version.
    async fn rollback(&self, id: &SkillId, target_version: &SkillVersion)
        -> Result<(), SkillError>;

    /// List all version history for a skill (from reflog).
    async fn version_history(&self, id: &SkillId) -> Result<Vec<SkillVersion>, SkillError>;
}
