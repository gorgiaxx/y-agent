//! Knowledge service — orchestrates ingestion, indexing, and retrieval.
//!
//! Provides a high-level API for knowledge base operations, bridging
//! the `y-knowledge` crate with the service layer.

use std::collections::HashMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use y_core::embedding::EmbeddingProvider;
use y_knowledge::config::KnowledgeConfig;
use y_knowledge::ingestion::IngestionPipeline;
use y_knowledge::middleware::{
    EntryMetadata, InjectKnowledge, InjectKnowledgeConfig, KnowledgeContextItem,
};
use y_knowledge::models::{KnowledgeCollection, KnowledgeEntry};
use y_knowledge::quality::QualityFilter;
use y_knowledge::retrieval::HybridRetriever;
use y_knowledge::tokenizer::SimpleTokenizer;
use y_knowledge::tools::{
    KnowledgeIngestParams, KnowledgeIngestResult, KnowledgeSearchParams, KnowledgeSearchResult,
    SearchResultItem,
};
use y_knowledge::{
    classifier::RuleBasedClassifier,
    ingestion::{markdown::MarkdownConnector, text::TextConnector, SourceConnector},
};

use y_knowledge::metadata::DocumentMetadata;

/// Snapshot of entry data used during re-indexing to avoid borrow conflicts.
struct ReindexEntryData {
    entry_id: String,
    chunks: Vec<String>,
    source_uri: String,
    title: String,
    quality_score: f32,
    summary: Option<String>,
    section_titles: Vec<String>,
    tags: Vec<String>,
    metadata: DocumentMetadata,
}

/// Knowledge service error.
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeServiceError {
    #[error("knowledge error: {0}")]
    Knowledge(#[from] y_knowledge::KnowledgeError),

    #[error("collection not found: {name}")]
    CollectionNotFound { name: String },

    #[error("unsupported source type: {extension}")]
    UnsupportedSource { extension: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("embedding error: {0}")]
    Embedding(#[from] y_core::embedding::EmbeddingError),
}

/// High-level knowledge service for the y-agent platform.
///
/// Orchestrates:
/// - Collection management (create/list/delete)
/// - Document ingestion (text, Markdown)
/// - Knowledge retrieval and context injection
pub struct KnowledgeService {
    /// Configuration.
    #[allow(dead_code)]
    config: KnowledgeConfig,
    /// Workspace ID.
    workspace_id: String,
    /// Collections managed by this service.
    collections: HashMap<String, KnowledgeCollection>,
    /// Ingested entries keyed by entry ID.
    entries: HashMap<String, KnowledgeEntry>,
    /// Knowledge injection middleware (shared via Arc for tool/context integration).
    inject_knowledge: Arc<StdMutex<InjectKnowledge<SimpleTokenizer>>>,
    /// Ingestion pipeline.
    pipeline: IngestionPipeline,
    /// Domain classifier.
    classifier: RuleBasedClassifier,
    /// Quality filter.
    quality_filter: QualityFilter,
    /// Optional data directory for persisting collections.
    data_dir: Option<PathBuf>,
    /// Optional embedding provider for vector-based semantic search.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    /// Optional tag generator for LLM-driven auto-tagging.
    tag_generator: Option<Arc<dyn TagGenerator>>,
    /// Optional metadata extractor for multi-dimensional LLM classification.
    metadata_extractor: Option<Arc<dyn MetadataExtractor>>,
    /// Optional summary generator for LLM-driven L0/L1 summarization.
    summary_generator: Option<Arc<dyn y_knowledge::tagger::SummaryGenerator>>,
}

impl KnowledgeService {
    /// Create a new knowledge service with default settings (in-memory only).
    pub fn new(config: KnowledgeConfig) -> Self {
        let retriever = HybridRetriever::new(SimpleTokenizer::new());
        let inject_knowledge = Arc::new(StdMutex::new(InjectKnowledge::new(retriever)));
        let pipeline = IngestionPipeline::new(config.clone());
        let classifier = RuleBasedClassifier::default_taxonomy();
        let quality_filter = QualityFilter::new();

        let mut service = Self {
            config,
            workspace_id: "default".to_string(),
            collections: HashMap::new(),
            entries: HashMap::new(),
            inject_knowledge,
            pipeline,
            classifier,
            quality_filter,
            data_dir: None,
            embedding_provider: None,
            tag_generator: None,
            metadata_extractor: None,
            summary_generator: None,
        };

        // Create default collection.
        service.create_collection("default", "Default knowledge collection");
        service
    }

    /// Create a new knowledge service with persistence to the given data directory.
    ///
    /// Collections are loaded from `<data_dir>/knowledge_collections.json` on
    /// construction and saved back after every mutation.
    pub fn with_data_dir(config: KnowledgeConfig, data_dir: PathBuf) -> Self {
        let retriever = HybridRetriever::new(SimpleTokenizer::new());
        let inject_knowledge = Arc::new(StdMutex::new(InjectKnowledge::new(retriever)));
        let pipeline = IngestionPipeline::new(config.clone());
        let classifier = RuleBasedClassifier::default_taxonomy();
        let quality_filter = QualityFilter::new();

        let mut service = Self {
            config,
            workspace_id: "default".to_string(),
            collections: HashMap::new(),
            entries: HashMap::new(),
            inject_knowledge,
            pipeline,
            classifier,
            quality_filter,
            data_dir: Some(data_dir),
            embedding_provider: None,
            tag_generator: None,
            metadata_extractor: None,
            summary_generator: None,
        };

        // Try to load persisted data.
        service.load_collections();
        service.load_entries();

        // Re-index all loaded entries into the HybridRetriever so the
        // in-memory search index is populated on startup.
        service.reindex_all_entries();

        // Ensure the default collection exists.
        if !service.has_collection("default") {
            service.create_collection("default", "Default knowledge collection");
        }

        service
    }

    /// Create with custom injection config.
    pub fn with_inject_config(
        config: KnowledgeConfig,
        inject_config: InjectKnowledgeConfig,
    ) -> Self {
        let retriever = HybridRetriever::new(SimpleTokenizer::new());
        let inject_knowledge = Arc::new(StdMutex::new(InjectKnowledge::with_config(
            retriever,
            inject_config,
        )));
        let pipeline = IngestionPipeline::new(config.clone());
        let classifier = RuleBasedClassifier::default_taxonomy();
        let quality_filter = QualityFilter::new();

        let mut service = Self {
            config,
            workspace_id: "default".to_string(),
            collections: HashMap::new(),
            entries: HashMap::new(),
            inject_knowledge,
            pipeline,
            classifier,
            quality_filter,
            data_dir: None,
            embedding_provider: None,
            tag_generator: None,
            metadata_extractor: None,
            summary_generator: None,
        };

        service.create_collection("default", "Default knowledge collection");
        service
    }

    /// Hot-reload the knowledge configuration.
    ///
    /// Updates the stored config and recreates the ingestion pipeline so
    /// that subsequent ingestion operations use the new parameters (e.g.
    /// chunk sizes, max chunks per entry). In-flight operations are
    /// unaffected because they already hold their own pipeline reference.
    pub fn reload_config(&mut self, new_config: KnowledgeConfig) {
        tracing::info!("Knowledge config hot-reloaded");
        self.pipeline = IngestionPipeline::new(new_config.clone());
        self.config = new_config;
    }

    /// Set the embedding provider for vector-based semantic search.
    ///
    /// When set, document ingestion will generate embeddings for each chunk
    /// and store them for cosine similarity retrieval.
    pub fn set_embedding_provider(&mut self, provider: Arc<dyn EmbeddingProvider>) {
        self.embedding_provider = Some(provider);
    }

    /// Set the tag generator for LLM-driven auto-tagging.
    ///
    /// When set, document ingestion will generate semantic tags for each
    /// entry using the `knowledge-metadata` sub-agent (legacy path).
    /// Prefer `set_metadata_extractor` for full metadata extraction.
    pub fn set_tag_generator(&mut self, generator: Arc<dyn TagGenerator>) {
        self.tag_generator = Some(generator);
    }

    /// Set the metadata extractor for multi-dimensional LLM classification.
    ///
    /// When set, ingestion with `extract_metadata = true` will extract
    /// document type, industry, sub-category, and interpreted title.
    pub fn set_metadata_extractor(&mut self, extractor: Arc<dyn MetadataExtractor>) {
        self.metadata_extractor = Some(extractor);
    }

    /// Set the summary generator for LLM-driven L0/L1 summarization.
    ///
    /// When set, ingestion with `use_llm_summary = true` will generate
    /// high-quality L0 summary and L1 section overviews via LLM.
    pub fn set_summary_generator(
        &mut self,
        generator: Arc<dyn y_knowledge::tagger::SummaryGenerator>,
    ) {
        self.summary_generator = Some(generator);
    }

    /// Get a reference to the embedding provider (if configured).
    pub fn embedding_provider(&self) -> Option<&Arc<dyn EmbeddingProvider>> {
        self.embedding_provider.as_ref()
    }

    /// Get a cloneable handle to the knowledge injection middleware.
    ///
    /// Used to share the retriever with the `KnowledgeSearch` tool and
    /// `KnowledgeContextProvider` for chat integration.
    pub fn knowledge_handle(&self) -> Arc<StdMutex<InjectKnowledge<SimpleTokenizer>>> {
        Arc::clone(&self.inject_knowledge)
    }

    // -------------------------------------------------------------------
    // Collection CRUD
    // -------------------------------------------------------------------

    /// Create a new collection.
    pub fn create_collection(&mut self, name: &str, description: &str) -> &KnowledgeCollection {
        let collection = KnowledgeCollection::new(&self.workspace_id, name, description);
        self.collections.insert(name.to_string(), collection);
        self.save_collections();
        // Safety: the key was just inserted on the line above.
        &self.collections[name]
    }

    /// List all collections.
    pub fn list_collections(&self) -> Vec<&KnowledgeCollection> {
        self.collections.values().collect()
    }

    /// Delete a collection and all its entries.
    ///
    /// This cascades the deletion: every entry belonging to the collection
    /// is removed first (clearing search index chunks, embeddings, and
    /// persisted data), then the collection record itself is dropped.
    pub fn delete_collection(&mut self, name: &str) -> bool {
        if !self.collections.contains_key(name) {
            return false;
        }

        // Collect entry IDs belonging to this collection first to avoid
        // borrow conflict (we need &mut self for delete_entry).
        let entry_ids: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.collection == name)
            .map(|(id, _)| id.clone())
            .collect();

        for entry_id in &entry_ids {
            self.delete_entry(entry_id);
        }

        self.collections.remove(name);
        self.save_collections();
        true
    }

    /// Rename a collection.
    ///
    /// Updates the collection's key in the map, its internal `name` field,
    /// and the `collection` field on every associated entry so that queries
    /// remain consistent.  Returns `false` if `old_name` does not exist or
    /// `new_name` is already taken.
    pub fn rename_collection(&mut self, old_name: &str, new_name: &str) -> bool {
        if !self.collections.contains_key(old_name) || self.collections.contains_key(new_name) {
            return false;
        }

        // Re-key the collection.
        if let Some(mut coll) = self.collections.remove(old_name) {
            coll.name = new_name.to_string();
            coll.updated_at = chrono::Utc::now();
            self.collections.insert(new_name.to_string(), coll);
        }

        // Update every entry that referenced the old name.
        for entry in self.entries.values_mut() {
            if entry.collection == old_name {
                entry.collection = new_name.to_string();
            }
        }

        self.save_collections();
        self.save_entries();
        true
    }

    /// Check if a collection exists.
    pub fn has_collection(&self, name: &str) -> bool {
        self.collections.contains_key(name)
    }

    // -------------------------------------------------------------------
    // Ingestion
    // -------------------------------------------------------------------

    /// Ingest a document from a source path.
    pub async fn ingest(
        &mut self,
        params: &KnowledgeIngestParams,
        workspace_id: &str,
    ) -> Result<KnowledgeIngestResult, KnowledgeServiceError> {
        // Determine source type from extension.
        let path = Path::new(&params.source);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let connector: Box<dyn SourceConnector> = match extension.as_str() {
            // Markdown
            "md" | "markdown" | "mdx" => Box::new(MarkdownConnector::new()),
            // Plain text & docs
            "txt" | "text" | "rst" | "adoc" | "org" | "rtf"
            // Data / config
            | "json" | "jsonl" | "yaml" | "yml" | "toml" | "csv" | "tsv"
            | "xml" | "html" | "htm" | "svg"
            | "ini" | "cfg" | "conf" | "env" | "properties"
            // Source code
            | "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "go" | "java"
            | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "rb" | "php"
            | "swift" | "kt" | "kts" | "scala" | "lua" | "r" | "pl"
            | "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd"
            | "sql" | "graphql" | "gql"
            // Misc text
            | "log" | "diff" | "patch" | "tex" | "bib"
            | "css" | "scss" | "less" | "sass"
            | "vue" | "svelte" | "astro"
            | "dockerfile" | "makefile" | "cmake"
            => Box::new(TextConnector::new()),
            _ => {
                // As a last resort, check if the file is likely text by
                // attempting to read the first few bytes. For now, reject
                // cleanly with the extension that failed.
                return Err(KnowledgeServiceError::UnsupportedSource {
                    extension: if extension.is_empty() {
                        "(no extension)".to_string()
                    } else {
                        extension.clone()
                    },
                });
            }
        };

        // Fetch the document.
        let raw_doc = connector.fetch(&params.source).await?;

        // Check collection exists.
        if !self.has_collection(&params.collection) {
            self.create_collection(&params.collection, "Auto-created collection");
        }

        // Run ingestion pipeline.
        let entry = self.pipeline.ingest(
            raw_doc,
            workspace_id,
            &params.collection,
            Some(&self.classifier),
            Some(&self.quality_filter),
        )?;

        let chunk_count = entry.chunks.len();
        let domains = entry.domains.clone();
        let quality_score = entry.quality_score;
        let entry_id = entry.id.to_string();
        let content_size: u64 = entry.chunks.iter().map(|c| c.len() as u64).sum();

        // Generate LLM-driven L0/L1 summaries first so that the metadata
        // extractor can use them as input (it expects l0_summary + l1 sections).
        let mut entry = entry;
        let mut llm_summary_status: Option<&str> = None;
        if params.use_llm_summary {
            let has_gen = self.summary_generator.is_some();
            tracing::info!(
                use_llm_summary = params.use_llm_summary,
                summary_generator_configured = has_gen,
                "LLM summarization requested"
            );
            if let Some(ref summary_gen) = self.summary_generator {
                let original_filename = std::path::Path::new(&params.source)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let total_lines = entry.content.lines().count();
                match summary_gen
                    .generate_summary(&params.source, total_lines, original_filename)
                    .await
                {
                    Ok(llm_summary) => {
                        tracing::info!(
                            entry_id = %entry_id,
                            l1_count = llm_summary.l1_sections.len(),
                            "LLM-generated L0/L1 summaries for entry"
                        );
                        entry.summary = Some(llm_summary.l0_summary);
                        entry.l1_sections = llm_summary
                            .l1_sections
                            .into_iter()
                            .enumerate()
                            .map(|(i, s)| y_knowledge::models::L1Section {
                                index: i,
                                title: s.title,
                                content: s.summary,
                            })
                            .collect();
                        // Sync overview field.
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
                        llm_summary_status = Some("ok");
                    }
                    Err(e) => {
                        tracing::error!(
                            entry_id = %entry_id,
                            error = %e,
                            "Failed to generate LLM summary, degraded to text-based truncation"
                        );
                        llm_summary_status = Some("failed");
                    }
                }
            } else {
                tracing::debug!("use_llm_summary requested but no summary_generator configured");
                llm_summary_status = Some("not_configured");
            }
        }

        // Extract multi-dimensional metadata via LLM. Runs after the summarizer
        // so that entry.summary and entry.l1_sections contain the LLM-generated
        // content, giving the metadata agent higher-quality input.
        if params.extract_metadata {
            if let Some(ref extractor) = self.metadata_extractor {
                let section_titles: Vec<String> =
                    entry.l1_sections.iter().map(|s| s.title.clone()).collect();
                let original_filename = std::path::Path::new(&params.source)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(String::from);
                match extractor
                    .extract_metadata(
                        &entry.content,
                        entry.summary.as_deref(),
                        &section_titles,
                        original_filename.as_deref(),
                    )
                    .await
                {
                    Ok(mut meta) => {
                        meta.original_filename = original_filename;
                        tracing::info!(
                            entry_id = %entry_id,
                            doc_type = ?meta.document_type,
                            industry = ?meta.industry,
                            "Extracted metadata for entry"
                        );
                        // Sync topics -> tags for backward compat.
                        entry.tags = meta.topics.clone();
                        entry.metadata = meta;
                    }
                    Err(e) => {
                        tracing::warn!(
                            entry_id = %entry_id,
                            "Failed to extract metadata, continuing without: {e}"
                        );
                    }
                }
            } else {
                tracing::debug!("extract_metadata requested but no metadata_extractor configured");
            }
        }

        // Generate embeddings if an embedding provider is configured.
        let chunk_embeddings = if let Some(ref provider) = self.embedding_provider {
            // Truncate chunk text to fit the embedding model's context window.
            //
            // Different tokenizers (BPE vs WordPiece/BERT) produce vastly
            // different token counts for the same text. WordPiece/BERT can
            // average as low as ~1.1 chars per token, while BPE averages
            // ~3-4 chars per token. Use 1 char = 1 token as the worst-case
            // ceiling to guarantee no overflow regardless of tokenizer.
            let max_tokens = self.config.effective_chunk_max_tokens();
            let max_chars = max_tokens as usize;
            let texts: Vec<String> = entry
                .chunks
                .iter()
                .map(|c| {
                    if max_tokens == 0 {
                        return c.clone();
                    }
                    if c.chars().count() <= max_chars {
                        return c.clone();
                    }
                    tracing::debug!(
                        original_chars = c.chars().count(),
                        max_chars,
                        max_tokens,
                        "Truncating chunk to fit embedding model context window"
                    );
                    c.chars().take(max_chars).collect()
                })
                .collect();
            match provider.embed_batch(&texts).await {
                Ok(results) => {
                    let embeddings: Vec<Vec<f32>> = results.into_iter().map(|r| r.vector).collect();
                    tracing::info!(
                        chunks = embeddings.len(),
                        "Generated embeddings for ingested chunks"
                    );
                    Some(embeddings)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to generate embeddings, falling back to keyword-only: {e}"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Index chunks for retrieval (batch -- much faster than per-chunk).
        {
            use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};
            let mut knowledge = self
                .inject_knowledge
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            let domain = domains.first().cloned().unwrap_or_default();
            let chunks_for_index: Vec<Chunk> = entry
                .chunks
                .iter()
                .enumerate()
                .map(|(i, chunk_content)| Chunk {
                    id: format!("{entry_id}-{i}"),
                    document_id: entry_id.clone(),
                    level: ChunkLevel::L2,
                    content: chunk_content.clone(),
                    token_estimate: u32::try_from(chunk_content.len() / 4).unwrap_or(u32::MAX),
                    metadata: ChunkMetadata {
                        source: entry.source.uri.clone(),
                        domain: domain.clone(),
                        title: entry.source.title.clone(),
                        section_index: i,
                    },
                })
                .collect();

            if let Some(ref embeddings) = chunk_embeddings {
                knowledge.retriever_mut().index_batch_with_embeddings(
                    chunks_for_index,
                    embeddings.clone(),
                    quality_score,
                );
            } else {
                knowledge
                    .retriever_mut()
                    .index_batch_with_quality(chunks_for_index, quality_score);
            }

            // Register L0/L1 metadata for progressive context injection.
            knowledge.register_entry_metadata(
                &entry_id,
                EntryMetadata {
                    title: entry.source.title.clone(),
                    summary: entry.summary.clone(),
                    section_titles: entry.l1_sections.iter().map(|s| s.title.clone()).collect(),
                    tags: entry.tags.clone(),
                    document_type: entry.metadata.document_type.clone(),
                    industry: entry.metadata.industry.clone(),
                    subcategory: entry.metadata.subcategory.clone(),
                },
            );
        }

        // Update collection stats.
        if let Some(collection) = self.collections.get_mut(&params.collection) {
            collection.stats.entry_count += 1;
            collection.stats.chunk_count += chunk_count as u64;
            collection.stats.total_bytes += content_size;
        }
        self.save_collections();

        // Persist entry metadata (strip full content to save space).
        let mut stored_entry = entry;
        stored_entry.content = String::new(); // Don't persist full content
        stored_entry.content_size = content_size; // Persist computed content size for accurate deletion accounting
        self.entries.insert(entry_id.clone(), stored_entry);
        self.save_entries();

        // Persist embeddings if any were generated.
        if chunk_embeddings.is_some() {
            self.save_embeddings();
        }

        // Build result message with LLM pipeline status.
        let mut msg = format!("Ingested successfully: {chunk_count} chunks");
        match llm_summary_status {
            Some("ok") => msg.push_str(" [LLM summary: ok]"),
            Some("failed") => msg.push_str(" [LLM summary: FAILED, using text-based fallback]"),
            Some("not_configured") => msg.push_str(" [LLM summary: not configured]"),
            _ => {}
        }

        Ok(KnowledgeIngestResult {
            success: true,
            entry_id: Some(entry_id),
            chunk_count,
            domains,
            quality_score,
            message: msg,
        })
    }

    // -------------------------------------------------------------------
    // Search
    // -------------------------------------------------------------------

    /// Search the knowledge base.
    ///
    /// When an `EmbeddingProvider` is configured, the query is embedded
    /// before retrieval so that cosine similarity is used for the semantic
    /// component of the blend search.
    pub async fn search(&self, params: &KnowledgeSearchParams) -> KnowledgeSearchResult {
        // Embed the query for cosine similarity when a provider is available.
        let query_embedding = if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&params.query).await {
                Ok(result) => Some(result.vector),
                Err(e) => {
                    tracing::warn!("Failed to embed search query: {e}");
                    None
                }
            }
        } else {
            None
        };

        let domain = params.domain.as_deref();
        let knowledge = self
            .inject_knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let items =
            knowledge.retrieve_for_context(&params.query, query_embedding.as_deref(), domain);

        let results: Vec<SearchResultItem> = items
            .iter()
            .take(params.limit)
            .map(|item| SearchResultItem {
                chunk_id: item.chunk_id.clone(),
                document_id: item.document_id.clone(),
                content: item.content.clone(),
                relevance: item.relevance,
                domains: if item.domain.is_empty() {
                    vec![]
                } else {
                    vec![item.domain.clone()]
                },
                title: item.title.clone(),
            })
            .collect();

        let total_matches = items.len();

        KnowledgeSearchResult {
            results,
            total_matches,
            strategy: "hybrid".to_string(),
        }
    }

    // -------------------------------------------------------------------
    // Entry queries
    // -------------------------------------------------------------------

    /// List entries belonging to a collection.
    pub fn list_entries(&self, collection: &str) -> Vec<&KnowledgeEntry> {
        self.entries
            .values()
            .filter(|e| e.collection == collection)
            .collect()
    }

    /// Get a single entry by ID.
    pub fn get_entry(&self, entry_id: &str) -> Option<&KnowledgeEntry> {
        self.entries.get(entry_id)
    }

    /// Get mutable reference to a specific entry by ID.
    pub fn get_entry_mut(&mut self, entry_id: &str) -> Option<&mut KnowledgeEntry> {
        self.entries.get_mut(entry_id)
    }

    /// Public wrapper for persisting entry changes.
    ///
    /// Used by GUI commands after editing entry metadata fields.
    pub fn save_entries_public(&self) {
        self.save_entries();
    }

    /// Delete an entry by ID. Returns `true` if found and removed.
    pub fn delete_entry(&mut self, entry_id: &str) -> bool {
        if let Some(entry) = self.entries.remove(entry_id) {
            // Use the persisted content_size field (set at ingest time) for
            // accurate stats accounting.  Fall back to computing from chunks
            // only for entries created before the field was introduced.
            let content_size: u64 = if entry.content_size > 0 {
                entry.content_size
            } else {
                entry.chunks.iter().map(|c| c.len() as u64).sum()
            };

            // Update collection stats.
            if let Some(coll) = self.collections.get_mut(&entry.collection) {
                coll.stats.entry_count = coll.stats.entry_count.saturating_sub(1);
                coll.stats.chunk_count = coll
                    .stats
                    .chunk_count
                    .saturating_sub(entry.chunks.len() as u64);
                coll.stats.total_bytes = coll.stats.total_bytes.saturating_sub(content_size);
            }

            // Remove chunks from the in-memory retriever index.
            {
                let mut knowledge = self
                    .inject_knowledge
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let removed = knowledge.retriever_mut().remove_by_document(entry_id);
                knowledge.remove_entry_metadata(entry_id);
                tracing::debug!(entry_id, removed, "Removed chunks from retriever index");
            }

            self.save_collections();
            self.save_entries();
            self.save_embeddings();
            true
        } else {
            false
        }
    }

    // -------------------------------------------------------------------
    // Context injection
    // -------------------------------------------------------------------

    /// Retrieve knowledge items for context injection.
    ///
    /// Called by the context pipeline middleware.
    pub fn retrieve_for_context(
        &self,
        user_query: &str,
        domain_hint: Option<&str>,
    ) -> Vec<KnowledgeContextItem> {
        self.inject_knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retrieve_for_context(user_query, None, domain_hint)
    }

    /// Retrieve knowledge items with a pre-computed query embedding.
    pub fn retrieve_for_context_with_embedding(
        &self,
        user_query: &str,
        query_embedding: Option<&[f32]>,
        domain_hint: Option<&str>,
    ) -> Vec<KnowledgeContextItem> {
        self.inject_knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retrieve_for_context(user_query, query_embedding, domain_hint)
    }

    // -------------------------------------------------------------------
    // Re-indexing (rebuild in-memory index from persisted entries)
    // -------------------------------------------------------------------

    /// Re-index all loaded entries into the `HybridRetriever`.
    ///
    /// Called on startup after `load_entries()` to rebuild the in-memory
    /// BM25 index from persisted chunk data. Uses batch indexing for
    /// better performance with large knowledge bases.
    fn reindex_all_entries(&mut self) {
        use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};

        // Load persisted embeddings (if any).
        let persisted_embeddings = self.load_embeddings();

        let mut total_chunks = 0usize;
        let mut embedding_count = 0usize;

        // Collect entry data first to avoid borrow conflict.
        let entries_data: Vec<ReindexEntryData> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.chunks.is_empty())
            .map(|(entry_id, entry)| ReindexEntryData {
                entry_id: entry_id.clone(),
                chunks: entry.chunks.clone(),
                source_uri: entry.source.uri.clone(),
                title: entry.source.title.clone(),
                quality_score: entry.quality_score,
                summary: entry.summary.clone(),
                section_titles: entry.l1_sections.iter().map(|s| s.title.clone()).collect(),
                tags: entry.tags.clone(),
                metadata: entry.metadata.clone(),
            })
            .collect();

        let mut knowledge = self
            .inject_knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        for entry in &entries_data {
            let domain = self
                .entries
                .get(&entry.entry_id)
                .and_then(|e| e.domains.first().cloned())
                .unwrap_or_default();

            // Build all Chunk structs for this entry.
            let all_chunks: Vec<Chunk> = entry
                .chunks
                .iter()
                .enumerate()
                .map(|(i, chunk_content)| Chunk {
                    id: format!("{}-{i}", entry.entry_id),
                    document_id: entry.entry_id.clone(),
                    level: ChunkLevel::L2,
                    content: chunk_content.clone(),
                    token_estimate: u32::try_from(chunk_content.len() / 4).unwrap_or(u32::MAX),
                    metadata: ChunkMetadata {
                        source: entry.source_uri.clone(),
                        domain: domain.clone(),
                        title: entry.title.clone(),
                        section_index: i,
                    },
                })
                .collect();

            total_chunks += all_chunks.len();

            // Partition into chunks with/without embeddings.
            let mut chunks_with_emb = Vec::new();
            let mut embs = Vec::new();
            let mut chunks_without_emb = Vec::new();

            for chunk in all_chunks {
                if let Some(embedding) = persisted_embeddings.get(&chunk.id) {
                    chunks_with_emb.push(chunk);
                    embs.push(embedding.clone());
                    embedding_count += 1;
                } else {
                    chunks_without_emb.push(chunk);
                }
            }

            if !chunks_with_emb.is_empty() {
                knowledge.retriever_mut().index_batch_with_embeddings(
                    chunks_with_emb,
                    embs,
                    entry.quality_score,
                );
            }
            if !chunks_without_emb.is_empty() {
                knowledge
                    .retriever_mut()
                    .index_batch_with_quality(chunks_without_emb, entry.quality_score);
            }

            // Register L0/L1 metadata for progressive context injection.
            knowledge.register_entry_metadata(
                &entry.entry_id,
                EntryMetadata {
                    title: entry.title.clone(),
                    summary: entry.summary.clone(),
                    section_titles: entry.section_titles.clone(),
                    tags: entry.tags.clone(),
                    document_type: entry.metadata.document_type.clone(),
                    industry: entry.metadata.industry.clone(),
                    subcategory: entry.metadata.subcategory.clone(),
                },
            );
        }

        if total_chunks > 0 {
            tracing::info!(
                total_chunks,
                embedding_count,
                entries = entries_data.len(),
                "Re-indexed knowledge chunks into retriever"
            );
        }
    }

    // -------------------------------------------------------------------
    // Persistence
    // -------------------------------------------------------------------

    /// Persist collections to disk (if a data directory is configured).
    fn save_collections(&self) {
        let Some(dir) = &self.data_dir else { return };
        if let Err(e) = fs::create_dir_all(dir) {
            tracing::warn!("Failed to create knowledge data dir: {e}");
            return;
        }
        let path = dir.join("knowledge_collections.json");
        match serde_json::to_string_pretty(&self.collections) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    tracing::warn!("Failed to save knowledge collections: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("Failed to serialize knowledge collections: {e}");
            }
        }
    }

    /// Load collections from disk (if a data directory is configured).
    fn load_collections(&mut self) {
        let Some(dir) = &self.data_dir else { return };
        let path = dir.join("knowledge_collections.json");
        if !path.exists() {
            return;
        }
        match fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<HashMap<String, KnowledgeCollection>>(&json) {
                Ok(collections) => {
                    tracing::info!(
                        "Loaded {} knowledge collections from disk",
                        collections.len()
                    );
                    self.collections = collections;
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize knowledge collections: {e}");
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read knowledge collections file: {e}");
            }
        }
    }

    /// Persist entry metadata to disk.
    fn save_entries(&self) {
        let Some(dir) = &self.data_dir else { return };
        if let Err(e) = fs::create_dir_all(dir) {
            tracing::warn!("Failed to create knowledge data dir: {e}");
            return;
        }
        let path = dir.join("knowledge_entries.json");
        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    tracing::warn!("Failed to save knowledge entries: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("Failed to serialize knowledge entries: {e}");
            }
        }
    }

    /// Load entry metadata from disk.
    fn load_entries(&mut self) {
        let Some(dir) = &self.data_dir else { return };
        let path = dir.join("knowledge_entries.json");
        if !path.exists() {
            return;
        }
        match fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<HashMap<String, KnowledgeEntry>>(&json) {
                Ok(entries) => {
                    tracing::info!("Loaded {} knowledge entries from disk", entries.len());
                    self.entries = entries;
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize knowledge entries: {e}");
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read knowledge entries file: {e}");
            }
        }
    }

    // -------------------------------------------------------------------
    // Embedding persistence
    // -------------------------------------------------------------------
    //
    // Binary format (no external dependency):
    //   [entry_count: u32]
    //   for each entry:
    //     [key_len: u32] [key_bytes: u8 * key_len]
    //     [vec_len: u32] [floats: f32 * vec_len]

    /// Persist embedding vectors to `knowledge_embeddings.bin`.
    fn save_embeddings(&self) {
        let Some(dir) = &self.data_dir else { return };
        if let Err(e) = fs::create_dir_all(dir) {
            tracing::warn!("Failed to create knowledge data dir: {e}");
            return;
        }
        let path = dir.join("knowledge_embeddings.bin");

        let knowledge = self
            .inject_knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let embeddings = knowledge.retriever().embeddings();
        if embeddings.is_empty() {
            return;
        }

        let result = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&path)?;

            // Entry count.
            file.write_all(&u32::try_from(embeddings.len()).unwrap_or(0).to_le_bytes())?;

            for (key, vector) in embeddings {
                // Key.
                let key_bytes = key.as_bytes();
                file.write_all(&u32::try_from(key_bytes.len()).unwrap_or(0).to_le_bytes())?;
                file.write_all(key_bytes)?;

                // Vector.
                file.write_all(&u32::try_from(vector.len()).unwrap_or(0).to_le_bytes())?;
                for &f in vector {
                    file.write_all(&f.to_le_bytes())?;
                }
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                tracing::info!(
                    count = embeddings.len(),
                    "Persisted embedding vectors to disk"
                );
            }
            Err(e) => {
                tracing::warn!("Failed to save embeddings: {e}");
            }
        }
    }

    /// Load embedding vectors from `knowledge_embeddings.bin`.
    ///
    /// Returns an empty map if the file does not exist or cannot be read.
    fn load_embeddings(&self) -> HashMap<String, Vec<f32>> {
        let Some(dir) = &self.data_dir else {
            return HashMap::new();
        };
        let path = dir.join("knowledge_embeddings.bin");
        if !path.exists() {
            return HashMap::new();
        }

        let result = (|| -> std::io::Result<HashMap<String, Vec<f32>>> {
            let data = fs::read(&path)?;
            let mut cursor = &data[..];

            let mut buf4 = [0u8; 4];

            // Entry count.
            cursor.read_exact(&mut buf4)?;
            let entry_count = u32::from_le_bytes(buf4) as usize;

            let mut map = HashMap::with_capacity(entry_count);

            for _ in 0..entry_count {
                // Key.
                cursor.read_exact(&mut buf4)?;
                let key_len = u32::from_le_bytes(buf4) as usize;
                let mut key_buf = vec![0u8; key_len];
                cursor.read_exact(&mut key_buf)?;
                let key = String::from_utf8(key_buf)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Vector.
                cursor.read_exact(&mut buf4)?;
                let vec_len = u32::from_le_bytes(buf4) as usize;
                let mut vector = Vec::with_capacity(vec_len);
                for _ in 0..vec_len {
                    cursor.read_exact(&mut buf4)?;
                    vector.push(f32::from_le_bytes(buf4));
                }

                map.insert(key, vector);
            }

            Ok(map)
        })();

        match result {
            Ok(map) => {
                if !map.is_empty() {
                    tracing::info!(count = map.len(), "Loaded embedding vectors from disk");
                }
                map
            }
            Err(e) => {
                tracing::warn!("Failed to load embeddings: {e}");
                HashMap::new()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AgentTagGenerator — LLM-backed tag generation via delegation
// ---------------------------------------------------------------------------

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_knowledge::tagger::{
    ContentPreparator, MetadataExtractor, MetadataParser, PreparedContent, TagGenerator, TagMerger,
};

/// LLM-backed tag generator using the `knowledge-metadata` sub-agent.
///
/// Delegates to the built-in `knowledge-metadata` agent via [`AgentDelegator`].
/// Extracts topic tags from the metadata response for backward compatibility.
/// Handles large documents by preparing content through [`ContentPreparator`]
/// (summary + excerpts for medium docs, map-reduce for large docs).
pub struct AgentTagGenerator {
    delegator: Arc<dyn AgentDelegator>,
    preparator: ContentPreparator,
}

impl AgentTagGenerator {
    /// Create a new tag generator with an agent delegator.
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self {
            delegator,
            preparator: ContentPreparator::new(),
        }
    }

    /// Delegate a tagging request to the `knowledge-metadata` agent.
    ///
    /// Extracts topic tags from the structured metadata response.
    async fn call_tagger(&self, content: &str) -> Result<Vec<String>, KnowledgeServiceError> {
        let input = serde_json::json!({
            "l0_summary": "",
            "l1_overview": "",
            "content_excerpt": content,
            "original_filename": "unknown",
        });

        let result = self
            .delegator
            .delegate("knowledge-metadata", input, ContextStrategyHint::None, None)
            .await
            .map_err(|e| {
                KnowledgeServiceError::Knowledge(y_knowledge::KnowledgeError::IngestionError {
                    message: format!("tag generation delegation failed: {e}"),
                })
            })?;

        // Try to parse as structured metadata and extract topics.
        if let Ok(meta) = MetadataParser::parse(&result.text) {
            if !meta.topics.is_empty() {
                return Ok(meta.topics);
            }
        }

        // Fallback: parse as flat tag array.
        Ok(TagMerger::parse_tags(&result.text))
    }
}

#[async_trait::async_trait]
impl TagGenerator for AgentTagGenerator {
    async fn generate_tags(
        &self,
        content: &str,
        l0_summary: Option<&str>,
        l1_section_titles: &[String],
    ) -> Result<Vec<String>, y_knowledge::KnowledgeError> {
        let prepared = self
            .preparator
            .prepare(content, l0_summary, l1_section_titles);

        let tag_sets = match prepared {
            PreparedContent::Full(text) => {
                let tags = self.call_tagger(&text).await.map_err(|e| {
                    y_knowledge::KnowledgeError::IngestionError {
                        message: format!("tag generation failed: {e}"),
                    }
                })?;
                vec![tags]
            }
            PreparedContent::Summarized(text) => {
                let tags = self.call_tagger(&text).await.map_err(|e| {
                    y_knowledge::KnowledgeError::IngestionError {
                        message: format!("tag generation failed: {e}"),
                    }
                })?;
                vec![tags]
            }
            PreparedContent::MapReduce(windows) => {
                // Tag each window independently, then merge.
                let mut all_tags = Vec::new();
                for window in &windows {
                    match self.call_tagger(window).await {
                        Ok(tags) => all_tags.push(tags),
                        Err(e) => {
                            tracing::warn!("Tag generation for window failed: {e}");
                        }
                    }
                }
                all_tags
            }
        };

        Ok(TagMerger::merge(&tag_sets))
    }
}

// ---------------------------------------------------------------------------
// AgentMetadataExtractor -- LLM-backed metadata extraction via delegation
// ---------------------------------------------------------------------------

use y_knowledge::tagger::SummaryParser;

/// LLM-backed metadata extractor using the `knowledge-metadata` sub-agent.
///
/// Delegates to the built-in `knowledge-metadata` agent via [`AgentDelegator`].
/// Parses structured JSON output into [`DocumentMetadata`].
pub struct AgentMetadataExtractor {
    delegator: Arc<dyn AgentDelegator>,
}

impl AgentMetadataExtractor {
    /// Create a new metadata extractor with an agent delegator.
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self { delegator }
    }
}

#[async_trait::async_trait]
impl MetadataExtractor for AgentMetadataExtractor {
    async fn extract_metadata(
        &self,
        content: &str,
        l0_summary: Option<&str>,
        l1_section_titles: &[String],
        original_filename: Option<&str>,
    ) -> Result<y_knowledge::metadata::DocumentMetadata, y_knowledge::KnowledgeError> {
        // Prepare input from L0/L1 content (not raw content to save tokens).
        let preparator = ContentPreparator::new();
        let prepared = preparator.prepare(content, l0_summary, l1_section_titles);
        let input_text = match prepared {
            PreparedContent::Full(text) | PreparedContent::Summarized(text) => text,
            PreparedContent::MapReduce(windows) => {
                // For very large docs, use first + last windows.
                let mut combined = String::new();
                if let Some(first) = windows.first() {
                    combined.push_str(first);
                }
                if windows.len() > 1 {
                    if let Some(last) = windows.last() {
                        combined.push_str("\n\n[...]\n\n");
                        combined.push_str(last);
                    }
                }
                combined
            }
        };

        let input = serde_json::json!({
            "l0_summary": l0_summary.unwrap_or(""),
            "l1_overview": l1_section_titles.join("\n"),
            "content_excerpt": input_text,
            "original_filename": original_filename.unwrap_or("unknown"),
        });

        let result = self
            .delegator
            .delegate("knowledge-metadata", input, ContextStrategyHint::None, None)
            .await
            .map_err(|e| y_knowledge::KnowledgeError::IngestionError {
                message: format!("metadata extraction delegation failed: {e}"),
            })?;

        MetadataParser::parse(&result.text)
    }
}

// ---------------------------------------------------------------------------
// AgentSummaryGenerator -- LLM-backed summarization via delegation
// ---------------------------------------------------------------------------

/// LLM-backed summary generator using the `knowledge-summarizer` sub-agent.
///
/// The summarizer agent is tool-enabled: it uses `FileRead` with
/// `line_offset`/`limit` parameters to progressively read large files
/// without overflowing the context window. Between reads, the system
/// prunes previous tool results, and the agent detects truncation at
/// chunk boundaries to overlap reads.
pub struct AgentSummaryGenerator {
    delegator: Arc<dyn AgentDelegator>,
}

impl AgentSummaryGenerator {
    /// Create a new summary generator with an agent delegator.
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self { delegator }
    }
}

#[async_trait::async_trait]
impl y_knowledge::tagger::SummaryGenerator for AgentSummaryGenerator {
    async fn generate_summary(
        &self,
        file_path: &str,
        total_lines: usize,
        original_filename: &str,
    ) -> Result<y_knowledge::tagger::LlmSummary, y_knowledge::KnowledgeError> {
        let input = serde_json::json!({
            "file_path": file_path,
            "total_lines": total_lines,
            "original_filename": original_filename,
        });

        let result = self
            .delegator
            .delegate(
                "knowledge-summarizer",
                input,
                ContextStrategyHint::None,
                None,
            )
            .await
            .map_err(|e| y_knowledge::KnowledgeError::IngestionError {
                message: format!("summary generation delegation failed: {e}"),
            })?;

        SummaryParser::parse(&result.text)
    }
}

// ---------------------------------------------------------------------------
// Tag management methods on KnowledgeService
// ---------------------------------------------------------------------------

impl KnowledgeService {
    /// Update tags for a specific entry (manual editing).
    ///
    /// Replaces the entry's tags with the provided list and persists the change.
    /// Also updates the in-memory `EntryMetadata` so context injection reflects
    /// the new tags immediately.
    pub fn update_entry_tags(&mut self, entry_id: &str, tags: &[String]) -> bool {
        let Some(entry) = self.entries.get_mut(entry_id) else {
            return false;
        };

        // Normalize tags.
        entry.tags = tags
            .iter()
            .map(|t| TagMerger::normalize_tag(t))
            .filter(|t| !t.is_empty())
            .collect();

        let new_tags = entry.tags.clone();

        // Update in-memory metadata for context injection.
        {
            let mut knowledge = self
                .inject_knowledge
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            // Re-read existing metadata from the middleware to preserve
            // other fields (summary, section_titles).
            if let Some(entry) = self.entries.get(entry_id) {
                knowledge.register_entry_metadata(
                    entry_id,
                    EntryMetadata {
                        title: entry.source.title.clone(),
                        summary: entry.summary.clone(),
                        section_titles: entry.l1_sections.iter().map(|s| s.title.clone()).collect(),
                        tags: new_tags,
                        document_type: entry.metadata.document_type.clone(),
                        industry: entry.metadata.industry.clone(),
                        subcategory: entry.metadata.subcategory.clone(),
                    },
                );
            }
        }

        self.save_entries();
        true
    }

    /// Re-tag all entries that have empty tags using the given tag generator.
    ///
    /// This is a batch operation for retroactive tagging of existing entries.
    /// Entries that already have tags are skipped.
    /// Returns the number of entries that were successfully re-tagged.
    pub async fn retag_all_entries(&mut self, tag_generator: &dyn TagGenerator) -> usize {
        // Collect entry IDs that need tagging.
        let entries_to_tag: Vec<(String, String, Option<String>, Vec<String>)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.tags.is_empty())
            .map(|(id, entry)| {
                (
                    id.clone(),
                    entry.content.clone(),
                    entry.summary.clone(),
                    entry.l1_sections.iter().map(|s| s.title.clone()).collect(),
                )
            })
            .collect();

        if entries_to_tag.is_empty() {
            tracing::info!("No entries need re-tagging");
            return 0;
        }

        tracing::info!(
            count = entries_to_tag.len(),
            "Starting batch re-tagging of entries"
        );

        let mut tagged_count = 0usize;

        for (entry_id, content, summary, section_titles) in &entries_to_tag {
            match tag_generator
                .generate_tags(content, summary.as_deref(), section_titles)
                .await
            {
                Ok(tags) => {
                    if !tags.is_empty() {
                        self.update_entry_tags(entry_id, &tags);
                        tagged_count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(entry_id, "Failed to generate tags for entry: {e}");
                }
            }
        }

        tracing::info!(
            tagged_count,
            total = entries_to_tag.len(),
            "Batch re-tagging complete"
        );

        tagged_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tempfile::TempDir;
    use y_knowledge::config::KnowledgeConfig;
    use y_knowledge::metadata::DocumentMetadata;
    use y_knowledge::tagger::{LlmL1Section, LlmSummary, SummaryGenerator};

    struct CountingTagGenerator {
        calls: Arc<AtomicUsize>,
        tags: Vec<String>,
    }

    #[async_trait::async_trait]
    impl TagGenerator for CountingTagGenerator {
        async fn generate_tags(
            &self,
            _content: &str,
            _l0_summary: Option<&str>,
            _l1_section_titles: &[String],
        ) -> Result<Vec<String>, y_knowledge::KnowledgeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.tags.clone())
        }
    }

    struct StaticMetadataExtractor {
        metadata: DocumentMetadata,
    }

    #[async_trait::async_trait]
    impl MetadataExtractor for StaticMetadataExtractor {
        async fn extract_metadata(
            &self,
            _content: &str,
            _l0_summary: Option<&str>,
            _l1_section_titles: &[String],
            _original_filename: Option<&str>,
        ) -> Result<DocumentMetadata, y_knowledge::KnowledgeError> {
            Ok(self.metadata.clone())
        }
    }

    struct CountingSummaryGenerator {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl SummaryGenerator for CountingSummaryGenerator {
        async fn generate_summary(
            &self,
            _file_path: &str,
            _total_lines: usize,
            _original_filename: &str,
        ) -> Result<LlmSummary, y_knowledge::KnowledgeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(LlmSummary {
                l0_summary: "LLM summary".to_string(),
                l1_sections: vec![LlmL1Section {
                    title: "Overview".to_string(),
                    summary: "Generated by summary generator.".to_string(),
                }],
            })
        }
    }

    fn write_test_doc(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).expect("write test doc");
        path
    }

    #[test]
    fn test_service_creation() {
        let service = KnowledgeService::new(KnowledgeConfig::default());
        assert!(service.has_collection("default"));
    }

    #[test]
    fn test_collection_crud() {
        let mut service = KnowledgeService::new(KnowledgeConfig::default());

        service.create_collection("test", "Test collection");
        assert!(service.has_collection("test"));
        assert_eq!(service.list_collections().len(), 2); // default + test

        assert!(service.delete_collection("test"));
        assert!(!service.has_collection("test"));
    }

    #[tokio::test]
    async fn test_search_empty() {
        let service = KnowledgeService::new(KnowledgeConfig::default());
        let params = KnowledgeSearchParams {
            query: "anything".to_string(),
            domain: None,
            resolution: "l0".to_string(),
            limit: 5,
            collection: None,
        };
        let result = service.search(&params).await;
        assert!(result.results.is_empty());
    }

    #[tokio::test]
    async fn test_ingest_without_metadata_flag_does_not_call_legacy_tagger() {
        let dir = TempDir::new().expect("temp dir");
        let content = [
            "# Safety Doc",
            "",
            "Functional safety guidance for automotive systems with hazard analysis,",
            "technical safety concepts, validation planning, and verification evidence.",
            "",
            "## Scope",
            "This document describes assumptions, safety goals, failure handling,",
            "and work products required for ISO 26262 aligned system development.",
            "",
            "## Requirements",
            "Teams shall define ASIL decomposition, safety mechanisms, traceability,",
            "change impact analysis, confirmation reviews, and production handoff notes.",
            "",
            "## Verification",
            "Evidence includes system tests, interface checks, design inspections,",
            "fault injection results, and safety case updates for every release.",
        ]
        .join("\n");
        let path = write_test_doc(&dir, "doc.md", &content);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut service = KnowledgeService::new(KnowledgeConfig::default());
        service.set_tag_generator(Arc::new(CountingTagGenerator {
            calls: Arc::clone(&calls),
            tags: vec!["functional-safety".to_string()],
        }));

        let params = KnowledgeIngestParams {
            source: path.display().to_string(),
            domain: None,
            collection: "default".to_string(),
            use_llm_summary: false,
            extract_metadata: false,
        };

        let result = service.ingest(&params, "default").await.expect("ingest");
        let entry_id = result.entry_id.expect("entry id");
        let entry = service.get_entry(&entry_id).expect("stored entry");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(entry.tags.is_empty());
        assert!(entry.metadata.is_empty());
    }

    #[tokio::test]
    async fn test_ingest_with_metadata_flag_persists_structured_metadata() {
        let dir = TempDir::new().expect("temp dir");
        let content = [
            "# ISO 26262-4",
            "",
            "Road vehicles functional safety part 4 covers product development",
            "at the system level, technical safety concepts, interface definitions,",
            "requirements allocation, verification strategy, and integration planning.",
            "",
            "## Product Development at the System Level",
            "System design shall refine functional safety requirements into",
            "technical safety requirements and define architectural assumptions.",
            "",
            "## Verification and Validation",
            "Development teams verify system requirements, analyze dependent failures,",
            "and validate the item against safety goals before release.",
        ]
        .join("\n");
        let path = write_test_doc(&dir, "iso-26262-part4.md", &content);
        let mut service = KnowledgeService::new(KnowledgeConfig::default());
        service.set_metadata_extractor(Arc::new(StaticMetadataExtractor {
            metadata: DocumentMetadata {
                document_type: Some("standards".to_string()),
                industry: Some("automotive".to_string()),
                subcategory: Some("functional_safety".to_string()),
                interpreted_title: Some(
                    "ISO 26262-4:2018 Road vehicles — Functional safety — Part 4".to_string(),
                ),
                title_language: Some("en".to_string()),
                original_filename: None,
                topics: vec!["functional-safety".to_string(), "iso-26262".to_string()],
            },
        }));

        let params = KnowledgeIngestParams {
            source: path.display().to_string(),
            domain: None,
            collection: "default".to_string(),
            use_llm_summary: false,
            extract_metadata: true,
        };

        let result = service.ingest(&params, "default").await.expect("ingest");
        let entry_id = result.entry_id.expect("entry id");
        let entry = service.get_entry(&entry_id).expect("stored entry");

        assert_eq!(entry.metadata.document_type.as_deref(), Some("standards"));
        assert_eq!(entry.metadata.industry.as_deref(), Some("automotive"));
        assert_eq!(
            entry.metadata.subcategory.as_deref(),
            Some("functional_safety")
        );
        assert_eq!(
            entry.metadata.interpreted_title.as_deref(),
            Some("ISO 26262-4:2018 Road vehicles — Functional safety — Part 4")
        );
        assert_eq!(
            entry.metadata.original_filename.as_deref(),
            Some("iso-26262-part4.md")
        );
        assert_eq!(
            entry.tags,
            vec!["functional-safety".to_string(), "iso-26262".to_string()]
        );
    }

    #[tokio::test]
    async fn test_ingest_with_llm_summary_flag_uses_summary_generator() {
        let dir = TempDir::new().expect("temp dir");
        let content = [
            "# Summary Target",
            "",
            "This document contains enough structure and detail to survive",
            "knowledge quality filtering during ingestion.",
            "",
            "## Overview",
            "The first section introduces the target system, expected behaviour,",
            "operational assumptions, and external interfaces.",
            "",
            "## Details",
            "The second section records failure modes, mitigations, verification",
            "activities, rollout notes, and follow-up actions for maintainers.",
        ]
        .join("\n");
        let path = write_test_doc(&dir, "summary-target.md", &content);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut service = KnowledgeService::new(KnowledgeConfig::default());
        service.set_summary_generator(Arc::new(CountingSummaryGenerator {
            calls: Arc::clone(&calls),
        }));

        let params = KnowledgeIngestParams {
            source: path.display().to_string(),
            domain: None,
            collection: "default".to_string(),
            use_llm_summary: true,
            extract_metadata: false,
        };

        let result = service.ingest(&params, "default").await.expect("ingest");
        let entry_id = result.entry_id.expect("entry id");
        let entry = service.get_entry(&entry_id).expect("stored entry");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(entry.summary.as_deref(), Some("LLM summary"));
        assert_eq!(entry.l1_sections.len(), 1);
        assert_eq!(entry.l1_sections[0].title, "Overview");
    }
}
