//! Ingestion pipeline: fetch → parse → chunk → classify → filter.
//!
//! Provides `SourceConnector` trait for fetching raw documents from various
//! sources, and `IngestionPipeline` for orchestrating the full ingestion flow.

pub mod encoding;
pub mod markdown;
pub mod text;

use crate::chunking::extract_section_title;
use crate::chunking::{coalesce_chunks, ChunkMetadata, ChunkerType, ChunkingStrategy};
use crate::classifier::Classifier;
use crate::config::KnowledgeConfig;
use crate::error::KnowledgeError;
use crate::models::{EntryState, KnowledgeEntry, L1Section, SourceRef, SourceType};
use crate::quality::QualityFilter;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Raw Document
// ---------------------------------------------------------------------------

/// A raw document fetched from a source before processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawDocument {
    /// The raw text content.
    pub content: String,
    /// Source URI (file path, URL, etc.).
    pub uri: String,
    /// Title extracted from the document (if available).
    pub title: String,
    /// SHA-256 hash of the content.
    pub content_hash: String,
    /// Source type.
    pub source_type: SourceType,
}

// ---------------------------------------------------------------------------
// Source Connector trait
// ---------------------------------------------------------------------------

/// Trait for fetching raw documents from various sources.
#[async_trait]
pub trait SourceConnector: Send + Sync {
    /// Fetch a raw document from the given URI.
    async fn fetch(&self, uri: &str) -> Result<RawDocument, KnowledgeError>;

    /// Return the source type this connector handles.
    fn source_type(&self) -> SourceType;
}

// ---------------------------------------------------------------------------
// Ingestion Pipeline
// ---------------------------------------------------------------------------

/// Orchestrates the ingestion flow: fetch → parse → chunk → classify → filter.
///
/// The pipeline takes a raw document and produces a `KnowledgeEntry` ready
/// for embedding and indexing.
pub struct IngestionPipeline {
    config: KnowledgeConfig,
    chunker_type: ChunkerType,
}

impl IngestionPipeline {
    /// Create a new pipeline with default settings.
    pub fn new(config: KnowledgeConfig) -> Self {
        Self {
            config,
            chunker_type: ChunkerType::SentenceBoundary,
        }
    }

    /// Set the chunker type for the pipeline.
    #[must_use]
    pub fn with_chunker(mut self, chunker_type: ChunkerType) -> Self {
        self.chunker_type = chunker_type;
        self
    }

    /// Run the full ingestion pipeline on a raw document.
    ///
    /// Steps: parse → chunk → classify → filter → produce `KnowledgeEntry`.
    pub fn ingest(
        &self,
        doc: RawDocument,
        workspace_id: &str,
        collection: &str,
        classifier: Option<&dyn Classifier>,
        quality_filter: Option<&QualityFilter>,
    ) -> Result<KnowledgeEntry, KnowledgeError> {
        // 1. Create entry in Fetched state.
        let source = SourceRef {
            source_type: doc.source_type,
            uri: doc.uri,
            content_hash: doc.content_hash,
            title: doc.title.clone(),
            author: None,
            fetched_at: Utc::now(),
            connector_id: None,
        };

        let mut entry = KnowledgeEntry::new(workspace_id, collection, &doc.content, source);

        // 2. Transition to Parsed.
        entry.transition(EntryState::Parsed);

        // 3. Chunk the content and cache.
        let strategy = ChunkingStrategy::with_chunker(self.config.clone(), self.chunker_type);
        let metadata = ChunkMetadata {
            source: entry.source.uri.clone(),
            domain: String::new(),
            title: doc.title,
            section_index: 0,
            collection: collection.to_string(),
            ..Default::default()
        };
        let chunks = strategy.chunk(
            &entry.id.to_string(),
            &entry.content,
            crate::chunking::ChunkLevel::L2,
            &metadata,
        );
        entry.chunks = chunks.into_iter().map(|c| c.content).collect();

        // Enforce max chunks per entry — merge adjacent chunks if over budget.
        let max_chunks = self.config.max_chunks_per_entry;
        if max_chunks > 0 && entry.chunks.len() > max_chunks {
            tracing::info!(
                before = entry.chunks.len(),
                max = max_chunks,
                "coalescing chunks to stay within max_chunks_per_entry"
            );
            entry.chunks = coalesce_chunks(entry.chunks, max_chunks);
        }

        // Generate L0 summary.
        let l0_chunks = strategy.chunk(
            &entry.id.to_string(),
            &entry.content,
            crate::chunking::ChunkLevel::L0,
            &metadata,
        );
        if let Some(l0) = l0_chunks.first() {
            entry.summary = Some(l0.content.clone());
        }

        // Generate L1 sections.
        let l1_chunks = strategy.chunk(
            &entry.id.to_string(),
            &entry.content,
            crate::chunking::ChunkLevel::L1,
            &metadata,
        );
        entry.l1_sections = l1_chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| L1Section {
                index: i,
                title: extract_section_title(&chunk.content, i),
                content: chunk.content.clone(),
            })
            .collect();

        // Sync overview field for backward compat.
        entry.overview = if entry.l1_sections.is_empty() {
            None
        } else {
            Some(
                entry
                    .l1_sections
                    .iter()
                    .map(|s| s.title.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
            )
        };

        entry.transition(EntryState::Chunked);

        // 4. Classify domains.
        if let Some(cls) = classifier {
            entry.domains = cls.classify(&entry.content);
            entry.transition(EntryState::Classified);
        }

        // 5. Quality filter.
        if let Some(filter) = quality_filter {
            let (accepted, score) = filter.evaluate(&entry);
            entry.quality_score = score;
            if !accepted {
                return Err(KnowledgeError::IngestionError {
                    message: format!("entry rejected by quality filter (score={score:.2})"),
                });
            }
            entry.transition(EntryState::Filtered);
        }

        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SourceType;

    fn test_raw_doc() -> RawDocument {
        RawDocument {
            content: "This is a test document with enough content to pass quality checks. \
                It contains multiple sentences and meaningful information about Rust programming. \
                The document covers advanced topics including cargo build configuration, rustc compiler flags, \
                clippy lints, and strategies for robust software development.".to_string(),
            uri: "/tmp/test.txt".to_string(),
            title: "Test Document".to_string(),
            content_hash: "abc123".to_string(),
            source_type: SourceType::File,
        }
    }

    #[test]
    fn test_pipeline_basic_ingestion() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);
        let entry = pipeline
            .ingest(test_raw_doc(), "ws-1", "default", None, None)
            .expect("ingestion should succeed");

        assert_eq!(entry.workspace_id, "ws-1");
        assert_eq!(entry.collection, "default");
        assert_eq!(entry.state, EntryState::Chunked);
        assert!(!entry.chunks.is_empty());
        assert!(entry.is_active);
        // L0 summary should be generated.
        assert!(entry.summary.is_some(), "L0 summary should be generated");
        // L1 sections should be generated.
        assert!(
            !entry.l1_sections.is_empty(),
            "L1 sections should be generated"
        );
    }

    #[test]
    fn test_pipeline_with_classifier() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);

        let classifier = crate::classifier::RuleBasedClassifier::default_taxonomy();
        let entry = pipeline
            .ingest(test_raw_doc(), "ws-1", "default", Some(&classifier), None)
            .expect("ingestion should succeed");

        assert_eq!(entry.state, EntryState::Classified);
        // "Rust" keyword should trigger rust domain classification.
        assert!(
            entry.domains.iter().any(|d| d.contains("rust")),
            "expected rust domain, got {:?}",
            entry.domains
        );
    }

    #[test]
    fn test_pipeline_with_quality_filter() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);
        let filter = QualityFilter::new();

        let entry = pipeline
            .ingest(test_raw_doc(), "ws-1", "default", None, Some(&filter))
            .expect("ingestion should succeed");

        assert_eq!(entry.state, EntryState::Filtered);
        assert!(entry.quality_score > 0.0);
    }

    #[test]
    fn test_pipeline_rejects_short_content() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);
        let filter = QualityFilter::new();

        let mut doc = test_raw_doc();
        doc.content = "Too short.".to_string();

        let result = pipeline.ingest(doc, "ws-1", "default", None, Some(&filter));
        assert!(result.is_err(), "short content should be rejected");
    }

    #[test]
    fn test_pipeline_with_sentence_boundary_chunker() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config).with_chunker(ChunkerType::SentenceBoundary);

        let entry = pipeline
            .ingest(test_raw_doc(), "ws-1", "default", None, None)
            .expect("ingestion should succeed");

        assert!(!entry.chunks.is_empty());
    }

    #[test]
    fn test_pipeline_generates_l0_summary() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);
        let entry = pipeline
            .ingest(test_raw_doc(), "ws-1", "default", None, None)
            .expect("ingestion should succeed");

        let summary = entry.summary.expect("L0 summary should be Some");
        assert!(!summary.is_empty(), "L0 summary should not be empty");
        // Summary should be shorter than the original content.
        assert!(
            summary.len() <= entry.content.len(),
            "L0 summary should be truncated"
        );
    }

    #[test]
    fn test_pipeline_generates_l1_sections() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);
        let entry = pipeline
            .ingest(test_raw_doc(), "ws-1", "default", None, None)
            .expect("ingestion should succeed");

        assert!(
            !entry.l1_sections.is_empty(),
            "L1 sections should be generated"
        );
        // Each section should have content.
        for section in &entry.l1_sections {
            assert!(
                !section.content.is_empty(),
                "L1 section content should not be empty"
            );
            assert!(
                !section.title.is_empty(),
                "L1 section title should not be empty"
            );
        }
        // Overview should be synced.
        assert!(
            entry.overview.is_some(),
            "overview should be set from L1 sections"
        );
    }

    #[test]
    fn test_pipeline_l1_section_titles_from_headings() {
        let config = KnowledgeConfig::default();
        let pipeline = IngestionPipeline::new(config);
        let doc = RawDocument {
            content: "# Introduction\nThis is the intro.\n\n## Methods\nWe use Rust.\n\n## Results\nGreat results.".to_string(),
            uri: "/tmp/headings.md".to_string(),
            title: "Headings Doc".to_string(),
            content_hash: "headings123".to_string(),
            source_type: SourceType::File,
        };
        let entry = pipeline
            .ingest(doc, "ws-1", "default", None, None)
            .expect("ingestion should succeed");

        // At least some sections should have heading-derived titles.
        let has_heading_title = entry
            .l1_sections
            .iter()
            .any(|s| s.title != format!("Section {}", s.index + 1));
        assert!(
            has_heading_title,
            "at least one L1 section should have a heading-derived title, got: {:?}",
            entry
                .l1_sections
                .iter()
                .map(|s| &s.title)
                .collect::<Vec<_>>()
        );
    }
}
