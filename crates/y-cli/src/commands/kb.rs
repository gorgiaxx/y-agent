//! `kb` CLI command — knowledge base management.
//!
//! Subcommands:
//! - `ingest` — ingest a document from a file path
//! - `collection` — collection CRUD (list, create, delete)
//! - `search` — search the knowledge base

use anyhow::Result;
use clap::Subcommand;

use crate::output::{self, OutputMode, TableRow};

/// Knowledge base subcommands.
#[derive(Debug, Subcommand)]
pub enum KbAction {
    /// Ingest a document into the knowledge base.
    Ingest {
        /// Source file path.
        #[arg(long)]
        source: String,

        /// Domain hint (optional).
        #[arg(long)]
        domain: Option<String>,

        /// Target collection (default: "default").
        #[arg(long, default_value = "default")]
        collection: String,
    },

    /// Collection management.
    Collection {
        #[command(subcommand)]
        action: CollectionAction,
    },

    /// Search the knowledge base.
    Search {
        /// Search query string.
        query: String,

        /// Domain filter (optional).
        #[arg(long)]
        domain: Option<String>,

        /// Maximum results.
        #[arg(long, default_value = "5")]
        limit: usize,
    },

    /// Entry management subcommands.
    Entry {
        #[command(subcommand)]
        action: EntryAction,
    },

    /// Rename a collection.
    Rename {
        /// Current collection name.
        #[arg(long)]
        old_name: String,

        /// New collection name.
        #[arg(long)]
        new_name: String,
    },
}

/// Collection subcommands.
#[derive(Debug, Subcommand)]
pub enum CollectionAction {
    /// List all collections.
    List,

    /// Create a new collection.
    Create {
        /// Collection name.
        name: String,

        /// Collection description.
        #[arg(long, default_value = "")]
        description: String,
    },

    /// Delete a collection.
    Delete {
        /// Collection name.
        name: String,
    },
}

/// Entry subcommands.
#[derive(Debug, Subcommand)]
pub enum EntryAction {
    /// List entries in a collection.
    List {
        /// Collection name.
        #[arg(long)]
        collection: String,
    },

    /// Get entry detail (L0/L1/L2 content).
    Detail {
        /// Entry ID.
        id: String,
    },

    /// Delete an entry.
    Delete {
        /// Entry ID.
        id: String,
    },

    /// Show global KB statistics.
    Stats,
}

/// Run a kb subcommand.
pub async fn run(
    action: &KbAction,
    services: &y_service::ServiceContainer,
    _mode: OutputMode,
) -> Result<()> {
    let service_handle = &services.knowledge_service;

    match action {
        KbAction::Ingest {
            source,
            domain,
            collection,
        } => {
            let params = y_service::KnowledgeIngestParams {
                source: source.clone(),
                domain: domain.clone(),
                collection: collection.clone(),
                use_llm_summary: false,
                extract_metadata: false,
            };

            output::print_info(&format!("Ingesting from '{source}'..."));

            match service_handle.lock().await.ingest(&params, "default").await {
                Ok(result) => {
                    if result.success {
                        output::print_success(&format!(
                            "Ingested successfully\n  Entry ID: {}\n  Chunks: {}\n  Domains: {}\n  Quality: {:.2}",
                            result.entry_id.unwrap_or_default(),
                            result.chunk_count,
                            result.domains.join(", "),
                            result.quality_score
                        ));
                    } else {
                        output::print_error(&format!("Ingestion failed: {}", result.message));
                    }
                }
                Err(e) => {
                    output::print_error(&format!("Ingestion error: {e}"));
                }
            }
        }

        KbAction::Collection { action } => match action {
            CollectionAction::List => {
                let guard = service_handle.lock().await;
                let collections = guard.list_collections();
                let headers = &["Name", "Description", "Entries"];
                let rows: Vec<TableRow> = collections
                    .iter()
                    .map(|c| TableRow {
                        cells: vec![
                            c.name.clone(),
                            c.description.clone(),
                            c.stats.entry_count.to_string(),
                        ],
                    })
                    .collect();
                drop(guard);

                if rows.is_empty() {
                    output::print_info("No collections found");
                } else {
                    output::print_info(&format!("{} collection(s):", rows.len()));
                    let table = output::format_table(headers, &rows);
                    print!("{table}");
                }
            }

            CollectionAction::Create { name, description } => {
                service_handle
                    .lock()
                    .await
                    .create_collection(name, description);
                output::print_success(&format!("Collection '{name}' created"));
            }

            CollectionAction::Delete { name } => {
                if service_handle.lock().await.delete_collection(name) {
                    output::print_success(&format!("Collection '{name}' deleted"));
                } else {
                    output::print_error(&format!("Collection '{name}' not found"));
                }
            }
        },

        KbAction::Search {
            query,
            domain,
            limit,
        } => {
            let params = y_service::KnowledgeSearchParams {
                query: query.clone(),
                domain: domain.clone(),
                resolution: "l0".to_string(),
                limit: *limit,
                collection: None,
            };

            let result = service_handle.lock().await.search(&params).await;

            if result.results.is_empty() {
                output::print_info("No results found");
            } else {
                output::print_info(&format!(
                    "{} result(s) found (strategy: {}):",
                    result.total_matches, result.strategy
                ));
                for (i, r) in result.results.iter().enumerate() {
                    println!("\n  {}. {} (relevance: {:.2})", i + 1, r.title, r.relevance);
                    // Show first 200 chars of content.
                    let preview: String = r.content.chars().take(200).collect();
                    println!("     {preview}");
                }
            }
        }
        KbAction::Entry { action } => match action {
            EntryAction::List { collection } => {
                let guard = service_handle.lock().await;
                let entries = guard.list_entries(collection);
                let headers = &["ID", "Title", "Type", "Collection", "Chunks"];
                let rows: Vec<TableRow> = entries
                    .iter()
                    .map(|e| TableRow {
                        cells: vec![
                            e.id.to_string(),
                            e.source.title.clone(),
                            e.source.source_type.to_string(),
                            e.collection.clone(),
                            e.chunks.len().to_string(),
                        ],
                    })
                    .collect();
                drop(guard);

                if rows.is_empty() {
                    output::print_info("No entries found");
                } else {
                    output::print_info(&format!("{} entry(s):", rows.len()));
                    let table = output::format_table(headers, &rows);
                    print!("{table}");
                }
            }

            EntryAction::Detail { id } => {
                const MAX_L2_DISPLAY: usize = 50;
                let guard = service_handle.lock().await;
                let Some(entry) = guard.get_entry(id) else {
                    output::print_error(&format!("Entry '{id}' not found"));
                    return Ok(());
                };
                output::print_info(&format!("Entry: {}", entry.source.title));
                println!("  ID: {}", entry.id);
                println!("  Collection: {}", entry.collection);
                println!("  State: {:?}", entry.state);
                println!("  Quality: {:.2}", entry.quality_score);
                println!("  Domains: {}", entry.domains.join(", "));
                println!("  Chunks: {}", entry.chunks.len());
                println!("\n--- L0 Summary ---");
                if let Some(summary) = &entry.summary {
                    println!("{summary}");
                } else {
                    println!("(no summary)");
                }
                println!("\n--- L1 Overview ---");
                if let Some(overview) = &entry.overview {
                    println!("{overview}");
                } else {
                    println!("(no overview)");
                }
                if !entry.l1_sections.is_empty() {
                    println!("\n--- L1 Sections ---");
                    for s in &entry.l1_sections {
                        println!("\n  [{}] {}", s.index, s.title);
                        println!("  {}", s.content);
                    }
                }
                println!("\n--- L2 Chunks ---");
                for (i, chunk) in entry.chunks.iter().take(MAX_L2_DISPLAY).enumerate() {
                    println!("\n  [Chunk {}/{}]", i + 1, entry.chunks.len());
                    let preview: String = chunk.chars().take(200).collect();
                    println!("  {preview}");
                }
                if entry.chunks.len() > MAX_L2_DISPLAY {
                    println!(
                        "\n  ... and {} more chunks (showing first {MAX_L2_DISPLAY})",
                        entry.chunks.len() - MAX_L2_DISPLAY
                    );
                }
            }

            EntryAction::Delete { id } => {
                if service_handle.lock().await.delete_entry(id) {
                    output::print_success(&format!("Entry '{id}' deleted"));
                } else {
                    output::print_error(&format!("Entry '{id}' not found"));
                }
            }

            EntryAction::Stats => {
                let guard = service_handle.lock().await;
                let collections = guard.list_collections();
                let entry_count: usize = collections
                    .iter()
                    .map(|c| c.stats.entry_count as usize)
                    .sum();
                let chunk_count: usize = collections
                    .iter()
                    .map(|c| c.stats.chunk_count as usize)
                    .sum();

                output::print_info("Knowledge Base Statistics");
                println!("  Collections: {}", collections.len());
                println!("  Entries: {entry_count}");
                println!("  Chunks: {chunk_count}");
            }
        },

        KbAction::Rename { old_name, new_name } => {
            if service_handle
                .lock()
                .await
                .rename_collection(old_name, new_name)
            {
                output::print_success(&format!("Collection '{old_name}' renamed to '{new_name}'"));
            } else {
                output::print_error(&format!(
                    "Failed to rename '{old_name}' (not found or '{new_name}' already exists)"
                ));
            }
        }
    }

    Ok(())
}
