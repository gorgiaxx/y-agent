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

/// Skill classification type per design.
///
/// Determines whether a skill is accepted, rejected, or partially accepted
/// during the ingestion pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillClassificationType {
    /// Pure LLM reasoning instructions (no external tools).
    LlmReasoning,
    /// Wraps an external API call.
    ApiCall,
    /// Wraps tool execution.
    ToolWrapper,
    /// Agent behavior instructions (delegation, multi-step).
    AgentBehavior,
    /// Mix of reasoning and tool/API usage.
    Hybrid,
}

impl std::fmt::Display for SkillClassificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::LlmReasoning => "llm_reasoning",
            Self::ApiCall => "api_call",
            Self::ToolWrapper => "tool_wrapper",
            Self::AgentBehavior => "agent_behavior",
            Self::Hybrid => "hybrid",
        };
        f.write_str(s)
    }
}

/// Skill lifecycle state per design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    /// External skill submitted for ingestion.
    Submitted,
    /// Ingestion pipeline analyzing content.
    Analyzing,
    /// Classification complete, awaiting filter decision.
    Classified,
    /// Rejected by filter gate or security screener.
    Rejected,
    /// Accepted, transformation in progress.
    Transforming,
    /// Transformation complete, awaiting registration.
    Transformed,
    /// Registered in skill registry.
    Registered,
    /// Actively selected for agent use.
    Active,
    /// Deprecated (replaced by newer version).
    Deprecated,
}

impl std::fmt::Display for SkillState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Submitted => "submitted",
            Self::Analyzing => "analyzing",
            Self::Classified => "classified",
            Self::Rejected => "rejected",
            Self::Transforming => "transforming",
            Self::Transformed => "transformed",
            Self::Registered => "registered",
            Self::Active => "active",
            Self::Deprecated => "deprecated",
        };
        f.write_str(s)
    }
}

/// Skill classification metadata (maps to `[skill.classification]` in manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillClassification {
    /// Classification type.
    #[serde(rename = "type")]
    pub skill_type: SkillClassificationType,
    /// Domain tags (e.g., `writing`, `chinese`, `editing`).
    #[serde(default)]
    pub domain: Vec<String>,
    /// Whether this skill is atomic (does one thing well).
    #[serde(default = "default_true")]
    pub atomic: bool,
}

/// Skill constraint metadata (maps to `[skill.constraints]` in manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConstraints {
    pub max_input_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub requires_language: Option<String>,
}

/// Skill security flags (maps to `[skill.security]` in manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSecurityConfig {
    #[serde(default)]
    pub allows_external_calls: bool,
    #[serde(default)]
    pub allows_file_operations: bool,
    #[serde(default)]
    pub allows_code_execution: bool,
    #[serde(default)]
    pub max_delegation_depth: u32,
}

/// Cross-resource references (maps to `[skill.references]` in manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillReferences {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub knowledge_bases: Vec<String>,
}

fn default_true() -> bool {
    true
}

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

    // --- Design-aligned extended fields (all optional for backward compat) ---
    /// Skill classification metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<SkillClassification>,
    /// Skill constraint metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<SkillConstraints>,
    /// Skill security configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<SkillSecurityConfig>,
    /// Cross-resource references.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<SkillReferences>,
    /// Author or generator of this skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Original source format (e.g., "markdown", "toml").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_format: Option<String>,
    /// SHA-256 hash of the original source content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    /// Lifecycle state of this skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SkillState>,
    /// Path to the root document file (design-aligned, relative to skill dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
}

/// Reference to a sub-document within a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubDocumentRef {
    /// Unique identifier for this sub-document.
    pub id: String,
    /// File path relative to skill directory (e.g., `details/tone-guidelines.md`).
    #[serde(default)]
    pub path: String,
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
