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
use y_knowledge::middleware::{EntryMetadata, InjectKnowledge, InjectKnowledgeConfig, KnowledgeContextItem};
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
    ingestion::{text::TextConnector, markdown::MarkdownConnector, SourceConnector},
};

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
        let inject_knowledge = Arc::new(StdMutex::new(
            InjectKnowledge::with_config(retriever, inject_config),
        ));
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
        };

        service.create_collection("default", "Default knowledge collection");
        service
    }

    /// Set the embedding provider for vector-based semantic search.
    ///
    /// When set, document ingestion will generate embeddings for each chunk
    /// and store them for cosine similarity retrieval.
    pub fn set_embedding_provider(&mut self, provider: Arc<dyn EmbeddingProvider>) {
        self.embedding_provider = Some(provider);
    }

    /// Get a reference to the embedding provider (if configured).
    pub fn embedding_provider(&self) -> Option<&Arc<dyn EmbeddingProvider>> {
        self.embedding_provider.as_ref()
    }

    /// Get a cloneable handle to the knowledge injection middleware.
    ///
    /// Used to share the retriever with the `knowledge_search` tool and
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
                        extension.to_string()
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

        // Generate embeddings if an embedding provider is configured.
        let chunk_embeddings = if let Some(ref provider) = self.embedding_provider {
            // Truncate chunk text to fit the embedding model's context window.
            // The chunking heuristic uses ~4 chars/token but real BPE tokenizers
            // often produce more tokens (short common words like "the" = 1 token
            // but only 3 chars). Use ~2 chars/token as a safe worst-case estimate
            // for English text to guarantee we stay within the model's limit.
            let max_tokens = self.config.effective_chunk_max_tokens();
            let max_chars = if max_tokens > 0 {
                (max_tokens as usize) * 2
            } else {
                usize::MAX
            };
            let texts: Vec<String> = entry.chunks.iter().map(|c| {
                if c.chars().count() > max_chars {
                    c.chars().take(max_chars).collect()
                } else {
                    c.clone()
                }
            }).collect();
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
                    tracing::warn!("Failed to generate embeddings, falling back to keyword-only: {e}");
                    None
                }
            }
        } else {
            None
        };

        // Index chunks for retrieval (batch — much faster than per-chunk).
        {
            use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};
            let mut knowledge = self.inject_knowledge.lock().unwrap_or_else(|e| e.into_inner());

            let domain = domains.first().cloned().unwrap_or_default();
            let chunks_for_index: Vec<Chunk> = entry.chunks.iter().enumerate().map(|(i, chunk_content)| {
                Chunk {
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
                }
            }).collect();

            if let Some(ref embeddings) = chunk_embeddings {
                knowledge.retriever_mut().index_batch_with_embeddings(
                    chunks_for_index,
                    embeddings.clone(),
                    quality_score,
                );
            } else {
                knowledge.retriever_mut().index_batch_with_quality(
                    chunks_for_index,
                    quality_score,
                );
            }

            // Register L0/L1 metadata for progressive context injection.
            knowledge.register_entry_metadata(&entry_id, EntryMetadata {
                title: entry.source.title.clone(),
                summary: entry.summary.clone(),
                section_titles: entry.l1_sections.iter().map(|s| s.title.clone()).collect(),
            });
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

        Ok(KnowledgeIngestResult {
            success: true,
            entry_id: Some(entry_id),
            chunk_count,
            domains,
            quality_score,
            message: format!("Ingested successfully: {chunk_count} chunks"),
        })
    }

    // -------------------------------------------------------------------
    // Search
    // -------------------------------------------------------------------

    /// Search the knowledge base.
    pub fn search(
        &self,
        params: &KnowledgeSearchParams,
    ) -> KnowledgeSearchResult {
        let domain = params.domain.as_deref();
        let knowledge = self.inject_knowledge.lock().unwrap_or_else(|e| e.into_inner());
        let items = knowledge.retrieve_for_context(&params.query, None, domain);

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
                coll.stats.chunk_count = coll.stats.chunk_count.saturating_sub(entry.chunks.len() as u64);
                coll.stats.total_bytes = coll.stats.total_bytes.saturating_sub(content_size);
            }

            // Remove chunks from the in-memory retriever index.
            {
                let mut knowledge = self.inject_knowledge.lock().unwrap_or_else(|e| e.into_inner());
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
            .unwrap_or_else(|e| e.into_inner())
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
            .unwrap_or_else(|e| e.into_inner())
            .retrieve_for_context(user_query, query_embedding, domain_hint)
    }

    // -------------------------------------------------------------------
    // Re-indexing (rebuild in-memory index from persisted entries)
    // -------------------------------------------------------------------

    /// Re-index all loaded entries into the HybridRetriever.
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
        let entries_data: Vec<(String, Vec<String>, String, String, f32, Option<String>, Vec<String>)> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.chunks.is_empty())
            .map(|(entry_id, entry)| {
                (
                    entry_id.clone(),
                    entry.chunks.clone(),
                    entry.source.uri.clone(),
                    entry.source.title.clone(),
                    entry.quality_score,
                    entry.summary.clone(),
                    entry.l1_sections.iter().map(|s| s.title.clone()).collect(),
                )
            })
            .collect();

        let mut knowledge = self.inject_knowledge.lock().unwrap_or_else(|e| e.into_inner());

        for (entry_id, chunks, source_uri, title, quality_score, summary, section_titles) in &entries_data {
            let domain = self
                .entries
                .get(entry_id)
                .and_then(|e| e.domains.first().cloned())
                .unwrap_or_default();

            // Build all Chunk structs for this entry.
            let all_chunks: Vec<Chunk> = chunks.iter().enumerate().map(|(i, chunk_content)| {
                Chunk {
                    id: format!("{entry_id}-{i}"),
                    document_id: entry_id.clone(),
                    level: ChunkLevel::L2,
                    content: chunk_content.clone(),
                    token_estimate: u32::try_from(chunk_content.len() / 4).unwrap_or(u32::MAX),
                    metadata: ChunkMetadata {
                        source: source_uri.clone(),
                        domain: domain.clone(),
                        title: title.clone(),
                        section_index: i,
                    },
                }
            }).collect();

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
                    chunks_with_emb, embs, *quality_score,
                );
            }
            if !chunks_without_emb.is_empty() {
                knowledge.retriever_mut().index_batch_with_quality(
                    chunks_without_emb, *quality_score,
                );
            }

            // Register L0/L1 metadata for progressive context injection.
            knowledge.register_entry_metadata(entry_id, EntryMetadata {
                title: title.clone(),
                summary: summary.clone(),
                section_titles: section_titles.clone(),
            });
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
                    tracing::info!(
                        "Loaded {} knowledge entries from disk",
                        entries.len()
                    );
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

        let knowledge = self.inject_knowledge.lock().unwrap_or_else(|e| e.into_inner());
        let embeddings = knowledge.retriever().embeddings();
        if embeddings.is_empty() {
            return;
        }

        let result = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&path)?;

            // Entry count.
            file.write_all(&(embeddings.len() as u32).to_le_bytes())?;

            for (key, vector) in embeddings {
                // Key.
                let key_bytes = key.as_bytes();
                file.write_all(&(key_bytes.len() as u32).to_le_bytes())?;
                file.write_all(key_bytes)?;

                // Vector.
                file.write_all(&(vector.len() as u32).to_le_bytes())?;
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
                    tracing::info!(
                        count = map.len(),
                        "Loaded embedding vectors from disk"
                    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use y_knowledge::config::KnowledgeConfig;

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

    #[test]
    fn test_search_empty() {
        let service = KnowledgeService::new(KnowledgeConfig::default());
        let params = KnowledgeSearchParams {
            query: "anything".to_string(),
            domain: None,
            resolution: "l0".to_string(),
            limit: 5,
            collection: None,
        };
        let result = service.search(&params);
        assert!(result.results.is_empty());
    }
}
