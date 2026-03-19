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

/// Run a kb subcommand.
pub async fn run(action: &KbAction, _mode: OutputMode) -> Result<()> {
    use y_knowledge::config::KnowledgeConfig;
    use y_service::knowledge_service::KnowledgeService;

    // Create a service instance.
    // In production, this would come from the ServiceContainer.
    let mut service = KnowledgeService::new(KnowledgeConfig::default());

    match action {
        KbAction::Ingest {
            source,
            domain,
            collection,
        } => {
            let params = y_knowledge::tools::KnowledgeIngestParams {
                source: source.clone(),
                domain: domain.clone(),
                collection: collection.clone(),
            };

            output::print_info(&format!("Ingesting from '{source}'..."));

            match service.ingest(&params, "default").await {
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
                let collections = service.list_collections();
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

                if rows.is_empty() {
                    output::print_info("No collections found");
                } else {
                    output::print_info(&format!("{} collection(s):", rows.len()));
                    let table = output::format_table(headers, &rows);
                    print!("{table}");
                }
            }

            CollectionAction::Create { name, description } => {
                service.create_collection(name, description);
                output::print_success(&format!("Collection '{name}' created"));
            }

            CollectionAction::Delete { name } => {
                if service.delete_collection(name) {
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
            let params = y_knowledge::tools::KnowledgeSearchParams {
                query: query.clone(),
                domain: domain.clone(),
                resolution: "l0".to_string(),
                limit: *limit,
                collection: None,
            };

            let result = service.search(&params);

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
    }

    Ok(())
}
