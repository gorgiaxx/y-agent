//! y-skills: Skill ingestion, transformation, registry, and versioning.
//!
//! Skills are LLM-instruction-only artifacts (no embedded tools or scripts).
//! They use a tree-indexed format with compact root documents (< 2,000 tokens)
//! and on-demand sub-document loading.
//!
//! # Components
//!
//! - [`manifest::ManifestParser`] — TOML parsing with token estimation
//! - [`version::VersionStore`] — content-addressable store with JSONL reflog
//! - [`search::SkillSearch`] — tag and trigger pattern matching
//! - [`registry::SkillRegistryImpl`] — full `SkillRegistry` trait implementation
//! - [`ingestion::IngestionPipeline`] — multi-format input processing

// ----- Core (always-on via `skill_core`) -----
pub mod config;
pub mod error;
pub mod manifest;
pub mod registry;
pub mod search;
pub mod store;
pub mod version;
pub mod version_meta;

// ----- Ingestion pipeline (feature: skill_ingestion) -----
#[cfg(feature = "skill_ingestion")]
pub mod analyzer;
#[cfg(feature = "skill_ingestion")]
pub mod classifier;
#[cfg(feature = "skill_ingestion")]
pub mod converter;
#[cfg(feature = "skill_ingestion")]
pub mod decomposer;
#[cfg(feature = "skill_ingestion")]
pub mod filter;
#[cfg(feature = "skill_ingestion")]
pub mod ingestion;
#[cfg(feature = "skill_ingestion")]
pub mod lineage;
#[cfg(feature = "skill_ingestion")]
pub mod separator;

// ----- Transformation (feature: skill_transformation) -----
#[cfg(feature = "skill_transformation")]
pub mod diff;
#[cfg(feature = "skill_transformation")]
pub mod state;
#[cfg(feature = "skill_transformation")]
pub mod validator;

// ----- Security screening (feature: skill_security_screening) -----
#[cfg(feature = "skill_security_screening")]
pub mod security;

// ----- Cross-resource linkage (feature: skill_linkage) -----
#[cfg(feature = "skill_linkage")]
pub mod linker;

// ----- Evolution capture (feature: evolution_capture) -----
#[cfg(feature = "evolution_capture")]
pub mod experience;

// ----- Evolution extraction (feature: evolution_extraction) -----
#[cfg(feature = "evolution_extraction")]
pub mod extractor;

// ----- Evolution refinement (feature: evolution_refinement) -----
#[cfg(feature = "evolution_refinement")]
pub mod evolution;
#[cfg(feature = "evolution_refinement")]
pub mod regression;

// ----- Fast-path extraction (feature: evolution_fast_path) -----
#[cfg(feature = "evolution_fast_path")]
pub mod fast_path;

// ----- Usage audit (feature: skill_usage_audit) -----
#[cfg(feature = "skill_usage_audit")]
pub mod usage_audit;

// ----- Garbage collection (always-on, needed by version store) -----
pub mod gc;

// ===== Re-exports =====
// Core
pub use config::SkillConfig;
pub use error::SkillModuleError;
pub use manifest::ManifestParser;
pub use registry::SkillRegistryImpl;
pub use search::SkillSearch;
pub use store::FilesystemSkillStore;
pub use version::PersistentVersionStore;
pub use version::VersionStore;
pub use version_meta::VersionMeta;

// Ingestion
#[cfg(feature = "skill_ingestion")]
pub use analyzer::{AnalysisReport, ContentAnalyzer};
#[cfg(feature = "skill_ingestion")]
pub use classifier::{SkillClassificationType, SkillClassifier};
#[cfg(feature = "skill_ingestion")]
pub use converter::FormatConverter;
#[cfg(feature = "skill_ingestion")]
pub use decomposer::{DecomposedSkill, DocumentDecomposer};
#[cfg(feature = "skill_ingestion")]
pub use filter::{FilterDecision, FilterGate};
#[cfg(feature = "skill_ingestion")]
pub use ingestion::{FormatDetector, IngestionFormat, IngestionPipeline};
#[cfg(feature = "skill_ingestion")]
pub use lineage::LineageRecord;
#[cfg(feature = "skill_ingestion")]
pub use separator::ToolSeparator;

// Transformation
#[cfg(feature = "skill_transformation")]
pub use diff::{diff_file, diff_texts, FileDiff, SkillDiff};
#[cfg(feature = "skill_transformation")]
pub use state::{SkillState, SkillStateMachine};
#[cfg(feature = "skill_transformation")]
pub use validator::SkillValidator;

// Security
#[cfg(feature = "skill_security_screening")]
pub use security::{SecurityScreener, SecurityVerdict};

// Linkage
#[cfg(feature = "skill_linkage")]
pub use linker::ResourceLinker;

// Evolution
#[cfg(feature = "evolution_refinement")]
pub use evolution::{ChangeType, SkillMetrics, SkillRefiner};
#[cfg(feature = "evolution_capture")]
pub use experience::{ExperienceRecord, ExperienceStore, TokenUsage, ToolCallRecord};
#[cfg(feature = "evolution_extraction")]
pub use extractor::{ExtractedPattern, PatternExtractor, PatternRegistry};
#[cfg(feature = "evolution_fast_path")]
pub use fast_path::FastPathExtractor;
#[cfg(feature = "evolution_refinement")]
pub use regression::RegressionDetector;

// Usage audit
#[cfg(feature = "skill_usage_audit")]
pub use usage_audit::SkillUsageAudit;

// GC
pub use gc::SkillGarbageCollector;
