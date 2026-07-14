use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use clap::Subcommand;

use y_core::skill::SkillRegistry;
use y_service::{ServiceContainer, SkillService};
use y_skills::{
    FilesystemSkillStore, ManifestParser, SkillConfig, SkillRegistryImpl, SkillValidator,
};

use crate::output::{self, OutputMode, TableRow};

/// Skill subcommands.
#[derive(Debug, Subcommand)]
pub enum SkillAction {
    /// List all registered skills.
    List,

    /// Show detailed info about a skill.
    Inspect {
        /// Skill name.
        name: String,
    },

    /// Import a skill from a file (uses AI agent for transformation).
    Import {
        /// Path to the skill source file.
        path: String,

        /// Use legacy TOML parser instead of AI agent.
        #[arg(long)]
        legacy: bool,
    },

    /// Mark a skill as deprecated.
    Deprecate {
        /// Skill name to deprecate.
        name: String,
    },

    /// Create a new skill from a natural-language description (uses AI agent).
    Create {
        /// Natural-language description of the skill to create.
        request: String,

        /// Optional domain hints (comma-separated).
        #[arg(long)]
        domain: Option<String>,

        /// Optional target language (ISO 639-1).
        #[arg(long)]
        language: Option<String>,
    },

    /// Validate all registered skills.
    Validate,

    /// Rollback a skill to a previous version.
    Rollback {
        /// Skill name.
        name: String,

        /// Target version hash to rollback to.
        version: String,
    },

    /// Enable a skill.
    Enable {
        /// Skill name.
        name: String,
    },

    /// Disable a skill.
    Disable {
        /// Skill name.
        name: String,
    },

    /// Uninstall (delete) a skill.
    Uninstall {
        /// Skill name.
        name: String,
    },

    /// List files in a skill directory.
    Files {
        /// Skill name.
        name: String,
    },

    /// Read a file within a skill directory.
    ReadFile {
        /// Skill name.
        name: String,

        /// Relative path to the file within the skill directory.
        path: String,
    },

    /// Save a file within a skill directory.
    SaveFile {
        /// Skill name.
        name: String,

        /// Relative path to the file within the skill directory.
        path: String,

        /// File content to write.
        content: String,
    },
}

/// Run a skill subcommand.
pub async fn run(
    action: &SkillAction,
    services: &ServiceContainer,
    mode: OutputMode,
) -> Result<()> {
    // Determine store path from config
    let config = SkillConfig::default();
    let store_path = &config.store_path;

    match action {
        SkillAction::List => {
            let store = FilesystemSkillStore::new(store_path)?;
            let registry = SkillRegistryImpl::with_store(store).await?;

            let skills = registry.search("", 1000).await?;

            let headers = &["Name", "Tags", "Version", "Tokens"];
            let rows: Vec<TableRow> = skills
                .iter()
                .map(|s| TableRow {
                    cells: vec![
                        s.name.clone(),
                        s.tags.join(", "),
                        String::new(), // Version not in summary
                        s.token_estimate.to_string(),
                    ],
                })
                .collect();

            match mode {
                OutputMode::Json => {
                    let json = serde_json::to_string_pretty(&skills)?;
                    println!("{json}");
                }
                _ => {
                    if rows.is_empty() {
                        output::print_info("No skills registered");
                    } else {
                        output::print_info(&format!("{} skill(s) registered:", rows.len()));
                        let table = output::format_table(headers, &rows);
                        print!("{table}");
                    }
                }
            }
        }

        SkillAction::Inspect { name } => {
            inspect_skill(store_path, name, mode)?;
        }

        SkillAction::Import { path, legacy } => {
            if *legacy {
                // Legacy path: manual TOML parsing
                let content = std::fs::read_to_string(path)?;
                let parser = ManifestParser::new(config.clone());
                let manifest = parser.parse(&content)?;

                let store = FilesystemSkillStore::new(store_path)?;
                let registry = SkillRegistryImpl::with_store(store).await?;

                let version = registry.register(manifest.clone()).await?;
                output::print_success(&format!(
                    "Imported skill '{}' (version: {}) [legacy]",
                    manifest.name, version
                ));
            } else {
                // Agent-based ingestion via SkillIngestionService
                let store = FilesystemSkillStore::new(store_path)?;
                let registry = std::sync::Arc::new(tokio::sync::RwLock::new(
                    SkillRegistryImpl::with_store(store).await?,
                ));
                let ingestion_service = services.skill_ingestion_service(registry);

                output::print_info(&format!(
                    "Importing skill from '{path}' (agent-assisted)..."
                ));

                match ingestion_service.import(Path::new(path)).await {
                    Ok(result) => match result.decision {
                        y_service::ImportDecision::Accepted => {
                            output::print_success(&format!(
                                "Skill imported successfully\n  ID: {}\n  Classification: {}\n  Decision: accepted",
                                result.skill_id.unwrap_or_default(),
                                result.classification
                            ));
                        }
                        y_service::ImportDecision::Optimized => {
                            let notes = result.optimization_notes.as_deref().unwrap_or("(none)");
                            output::print_success(&format!(
                                "Skill optimized and imported\n  ID: {}\n  Classification: {}\n  Decision: optimized\n  Optimizations: {}",
                                result.skill_id.unwrap_or_default(),
                                result.classification,
                                notes
                            ));
                        }
                        y_service::ImportDecision::PartialAccept => {
                            output::print_info(&format!(
                                "Skill partially accepted\n  ID: {}\n  Classification: {}\n  Security issues: {:?}",
                                result.skill_id.unwrap_or_default(),
                                result.classification,
                                result.security_issues
                            ));
                        }
                        y_service::ImportDecision::Rejected => {
                            output::print_error(&format!(
                                "Skill rejected\n  Classification: {}\n  Reason: {}\n  Suggestion: {}",
                                result.classification,
                                result.rejection_reason.unwrap_or_default(),
                                result.redirect_suggestion.unwrap_or_default()
                            ));
                        }
                    },
                    Err(e) => {
                        output::print_error(&format!("Import failed: {e}"));
                    }
                }
            }
        }

        SkillAction::Deprecate { name } => {
            // For now, deprecation is informational — just removes from store
            let store = FilesystemSkillStore::new(store_path)?;
            store.delete_skill(name)?;
            output::print_success(&format!("Skill '{name}' has been deprecated and removed"));
        }

        SkillAction::Create {
            request,
            domain,
            language,
        } => {
            let store = FilesystemSkillStore::new(store_path)?;
            let registry = std::sync::Arc::new(tokio::sync::RwLock::new(
                SkillRegistryImpl::with_store(store).await?,
            ));
            let creation_service = services.skill_creation_service(registry);

            output::print_info("Creating skill (agent-assisted)...");

            let domain_hints: Option<Vec<String>> = domain
                .as_deref()
                .map(|d| d.split(',').map(|s| s.trim().to_string()).collect());

            match creation_service
                .create(request, domain_hints.as_deref(), language.as_deref())
                .await
            {
                Ok(result) => match result.decision {
                    y_service::CreationDecision::Created => {
                        let notes = result.optimization_notes.as_deref().unwrap_or("(none)");
                        output::print_success(&format!(
                            "Skill created successfully\n  ID: {}\n  Notes: {}",
                            result.skill_id.unwrap_or_default(),
                            notes
                        ));
                    }
                    y_service::CreationDecision::Rejected => {
                        output::print_error(&format!(
                            "Skill creation rejected\n  Reason: {}\n  Suggestion: {}",
                            result.rejection_reason.unwrap_or_default(),
                            result.redirect_suggestion.unwrap_or_default()
                        ));
                    }
                },
                Err(e) => {
                    output::print_error(&format!("Creation failed: {e}"));
                }
            }
        }

        SkillAction::Validate => {
            validate_skills(store_path, &config)?;
        }

        SkillAction::Rollback { name, version } => {
            let store = FilesystemSkillStore::new(store_path)?;
            let registry = SkillRegistryImpl::with_store(store).await?;

            let skill_id = y_core::types::SkillId::from_string(name);
            let target_version = y_core::skill::SkillVersion(version.clone());

            registry.rollback(&skill_id, &target_version).await?;
            output::print_success(&format!("Rolled back skill '{name}' to version {version}"));
        }

        SkillAction::Enable { name } => {
            let skills_dir = services
                .skills_dir
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("skills directory not configured"))?;
            let svc = SkillService::new(skills_dir);
            svc.set_enabled(name, true)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            output::print_success(&format!("Skill '{name}' enabled"));
        }

        SkillAction::Disable { name } => {
            let skills_dir = services
                .skills_dir
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("skills directory not configured"))?;
            let svc = SkillService::new(skills_dir);
            svc.set_enabled(name, false)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            output::print_success(&format!("Skill '{name}' disabled"));
        }

        SkillAction::Uninstall { name } => {
            let skills_dir = services
                .skills_dir
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("skills directory not configured"))?;
            let svc = SkillService::new(skills_dir);
            svc.uninstall(name).await.map_err(|e| anyhow::anyhow!(e))?;
            output::print_success(&format!("Skill '{name}' uninstalled"));
        }

        SkillAction::Files { name } => {
            let skills_dir = services
                .skills_dir
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("skills directory not configured"))?;
            let dir = skills_dir.join(name);
            if !dir.exists() {
                anyhow::bail!("Skill directory not found: {}", dir.display());
            }
            let entries = list_skill_files(&dir, &dir)?;

            match mode {
                OutputMode::Json => {
                    let json = serde_json::to_string_pretty(&entries)?;
                    println!("{json}");
                }
                _ => {
                    if entries.is_empty() {
                        output::print_info(&format!("No files in skill '{name}'"));
                    } else {
                        let headers = &["Path", "Type", "Size"];
                        let rows: Vec<TableRow> = entries
                            .iter()
                            .map(|e| TableRow {
                                cells: vec![
                                    e.path.clone(),
                                    if e.is_dir {
                                        "dir".to_string()
                                    } else {
                                        "file".to_string()
                                    },
                                    if e.is_dir {
                                        "-".to_string()
                                    } else {
                                        e.size.to_string()
                                    },
                                ],
                            })
                            .collect();
                        let table = output::format_table(headers, &rows);
                        print!("{table}");
                    }
                }
            }
        }

        SkillAction::ReadFile { name, path } => {
            let skills_dir = services
                .skills_dir
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("skills directory not configured"))?;
            let dir = skills_dir.join(name);
            let target = y_service::resolve_skill_read_path(&dir, Path::new(path))
                .map_err(|e| anyhow::anyhow!(e))?;
            let content = std::fs::read_to_string(&target)?;

            match mode {
                OutputMode::Json => {
                    let json = serde_json::json!({
                        "path": path,
                        "content": content,
                    });
                    println!("{json}");
                }
                _ => {
                    print!("{content}");
                }
            }
        }

        SkillAction::SaveFile {
            name,
            path,
            content,
        } => {
            let skills_dir = services
                .skills_dir
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("skills directory not configured"))?;
            let dir = skills_dir.join(name);
            let target = y_service::resolve_skill_write_path(&dir, Path::new(path))
                .map_err(|e| anyhow::anyhow!(e))?;
            std::fs::write(&target, content)?;
            output::print_success(&format!("Saved file '{path}' in skill '{name}'"));
        }
    }

    Ok(())
}

/// Display detailed information about a single skill.
fn inspect_skill(store_path: &str, name: &str, mode: OutputMode) -> Result<()> {
    let store = FilesystemSkillStore::new(store_path)?;
    let manifest = store.load_skill(name)?;

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&manifest)?;
        println!("{json}");
        return Ok(());
    }

    println!("Skill: {}", manifest.name);
    println!("Description: {}", manifest.description);
    println!("Version: {}", manifest.version);
    println!("Tags: {}", manifest.tags.join(", "));
    println!("Token Estimate: {}", manifest.token_estimate);

    if let Some(ref cls) = manifest.classification {
        println!("\nClassification:");
        println!("  Type: {}", cls.skill_type);
        println!("  Domain: {}", cls.domain.join(", "));
        println!("  Atomic: {}", cls.atomic);
    }

    if let Some(ref con) = manifest.constraints {
        println!("\nConstraints:");
        if let Some(max_in) = con.max_input_tokens {
            println!("  Max Input Tokens: {max_in}");
        }
        if let Some(max_out) = con.max_output_tokens {
            println!("  Max Output Tokens: {max_out}");
        }
        if let Some(ref lang) = con.requires_language {
            println!("  Requires Language: {lang}");
        }
    }

    if let Some(ref security) = manifest.security {
        println!("\nSecurity:");
        println!(
            "  External Calls: {}",
            if security.allows_external_calls {
                "allowed"
            } else {
                "blocked"
            }
        );
        println!(
            "  File Operations: {}",
            if security.allows_file_operations {
                "allowed"
            } else {
                "blocked"
            }
        );
        println!(
            "  Code Execution: {}",
            if security.allows_code_execution {
                "allowed"
            } else {
                "blocked"
            }
        );
        println!("  Max Delegation Depth: {}", security.max_delegation_depth);
    }

    if let Some(ref refs) = manifest.references {
        println!("\nReferences:");
        if !refs.tools.is_empty() {
            println!("  Tools: {}", refs.tools.join(", "));
        }
        if !refs.skills.is_empty() {
            println!("  Skills: {}", refs.skills.join(", "));
        }
        if !refs.knowledge_bases.is_empty() {
            println!("  Knowledge Bases: {}", refs.knowledge_bases.join(", "));
        }
    }

    if !manifest.sub_documents.is_empty() {
        println!("\nSub-Documents:");
        for sd in &manifest.sub_documents {
            println!(
                "  - {} ({}): {} tokens",
                sd.id, sd.load_condition, sd.token_estimate
            );
        }
    }

    if let Some(ref author) = manifest.author {
        println!("\nAuthor: {author}");
    }

    let lineage_path = std::path::Path::new(store_path)
        .join(&manifest.name)
        .join("lineage.toml");
    if lineage_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&lineage_path) {
            if let Ok(record) = y_skills::LineageRecord::from_toml(&content) {
                println!("\nLineage:");
                println!("  Source: {}", record.source_path);
                println!("  Format: {}", record.source_format);
                println!("  Date: {}", record.transform_date);
                if let Some(ref model) = record.transform_model {
                    println!("  Model: {model}");
                }
                println!("  Steps: {}", record.transform_steps.len());
            }
        }
    }

    Ok(())
}

/// Validate all skills in the store.
fn validate_skills(store_path: &str, config: &SkillConfig) -> Result<()> {
    let store = FilesystemSkillStore::new(store_path)?;
    let all = store.load_all()?;
    let validator = SkillValidator::new(config.clone());

    let existing_names: HashSet<String> = all.iter().map(|m| m.name.clone()).collect();
    let empty_set: HashSet<String> = HashSet::new();

    let mut total_errors = 0;

    for manifest in &all {
        let skill_dir = std::path::Path::new(store_path).join(&manifest.name);
        let dir_errors = validator.validate_directory(&skill_dir);

        let manifest_errors = validator.validate_manifest(
            manifest,
            &existing_names,
            &empty_set,
            &empty_set,
            &empty_set,
        );

        let errors: Vec<_> = dir_errors.into_iter().chain(manifest_errors).collect();

        if errors.is_empty() {
            output::print_success(&format!("  [ok] {}", manifest.name));
        } else {
            total_errors += errors.len();
            output::print_error(&format!(
                "  [fail] {} ({} issues):",
                manifest.name,
                errors.len()
            ));
            for err in &errors {
                println!("    - {err}");
            }
        }
    }

    if all.is_empty() {
        output::print_info("No skills to validate");
    } else if total_errors == 0 {
        output::print_success(&format!("All {} skill(s) passed validation", all.len()));
    } else {
        output::print_error(&format!(
            "{total_errors} validation issue(s) found across {} skill(s)",
            all.len()
        ));
    }

    Ok(())
}

/// A file or directory entry within a skill directory.
#[derive(Debug, serde::Serialize)]
struct SkillFileEntry {
    path: String,
    is_dir: bool,
    size: u64,
}

/// Recursively list all files and directories under `dir`, returning entries
/// with paths relative to `base`.
fn list_skill_files(dir: &Path, base: &Path) -> Result<Vec<SkillFileEntry>> {
    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(dir)?;
    for entry in read_dir {
        let entry = entry?;
        let meta = entry.metadata()?;
        let abs_path = entry.path();
        let rel_path = abs_path
            .strip_prefix(base)
            .unwrap_or(&abs_path)
            .to_string_lossy()
            .to_string();

        if meta.is_dir() {
            entries.push(SkillFileEntry {
                path: rel_path,
                is_dir: true,
                size: 0,
            });
            entries.extend(list_skill_files(&abs_path, base)?);
        } else {
            entries.push(SkillFileEntry {
                path: rel_path,
                is_dir: false,
                size: meta.len(),
            });
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}
