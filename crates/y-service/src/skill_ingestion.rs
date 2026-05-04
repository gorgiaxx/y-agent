//! Skill ingestion service — delegates third-party skill transformation to the
//! `skill-ingestion` agent and registers the result in the skill registry.
//! Security screening is handled separately by the `skill-security-check` agent.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use tempfile::TempDir;

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_core::skill::{
    SkillClassification, SkillClassificationType, SkillConstraints, SkillManifest, SkillRegistry,
    SkillState, SkillVersion, SubDocumentRef,
};
use y_core::types::SkillId;
use y_skills::{
    FilesystemSkillStore, FormatDetector, IngestionFormat, ManifestParser, SkillConfig,
    SkillRegistryImpl,
};

// ---------------------------------------------------------------------------
// Import result
// ---------------------------------------------------------------------------

/// Result of a skill import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Whether the import was accepted, rejected, optimized, or partially accepted.
    pub decision: ImportDecision,
    /// Classification assigned by the agent.
    pub classification: String,
    /// Skill ID if registered.
    pub skill_id: Option<String>,
    /// Rejection reason if applicable.
    pub rejection_reason: Option<String>,
    /// Redirect suggestion if rejected.
    pub redirect_suggestion: Option<String>,
    /// Security issues found.
    pub security_issues: Vec<String>,
    /// Notes describing what optimizations were applied (for `optimized`/`partial_accept`).
    pub optimization_notes: Option<String>,
}

/// Import decision outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDecision {
    Accepted,
    Rejected,
    PartialAccept,
    Optimized,
}

/// Permissions a skill requests according to the security screening agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionsNeeded {
    #[serde(default)]
    pub files_read: Vec<String>,
    #[serde(default)]
    pub files_write: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Presentation-friendly result for an end-to-end skill import request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillImportOutcome {
    pub decision: String,
    pub classification: String,
    pub skill_id: Option<String>,
    pub error: Option<String>,
    pub security_issues: Vec<String>,
    pub permissions_needed: Option<PermissionsNeeded>,
}

impl SkillImportOutcome {
    fn rejected(error: String) -> Self {
        Self {
            decision: "rejected".to_string(),
            classification: String::new(),
            skill_id: None,
            error: Some(error),
            security_issues: vec![],
            permissions_needed: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Agent output schema
// ---------------------------------------------------------------------------

/// Structured output from the `skill-ingestion` agent.
///
/// The agent writes all file content (root.md, sub-documents, tools,
/// companion transforms) to the `output_dir` via `FileWrite`. This struct
/// contains only lightweight metadata -- no content fields.
#[derive(Debug, Deserialize)]
struct AgentIngestionOutput {
    decision: String,
    classification: String,
    #[serde(default)]
    rejection_reason: Option<String>,
    #[serde(default)]
    redirect_suggestion: Option<String>,
    #[serde(default)]
    optimization_notes: Option<String>,
    #[serde(default)]
    manifest: Option<AgentManifestOutput>,
    #[serde(default)]
    sub_documents: Vec<AgentSubDocOutput>,
    #[serde(default)]
    extracted_tools: Vec<AgentExtractedTool>,
    #[serde(default)]
    companion_decisions: Vec<CompanionDecision>,
}

#[derive(Debug, Deserialize)]
struct AgentManifestOutput {
    name: String,
    #[serde(default = "default_version")]
    version: String,
    description: String,
    #[serde(default)]
    classification: Option<AgentClassificationOutput>,
    #[serde(default)]
    constraints: Option<AgentConstraintsOutput>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

#[derive(Debug, Deserialize)]
struct AgentClassificationOutput {
    #[serde(rename = "type", default)]
    skill_type: String,
    #[serde(default)]
    domain: Vec<String>,
    #[serde(default = "default_atomic")]
    atomic: bool,
}

fn default_atomic() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct AgentConstraintsOutput {
    #[serde(default)]
    max_input_tokens: Option<u32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    requires_language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentSubDocOutput {
    path: String,
    title: String,
    #[serde(default)]
    token_count: u32,
    #[serde(default)]
    load_condition: Option<String>,
}

/// A tool that the agent extracted from a hybrid skill.
///
/// The actual script content is written by the agent to
/// `{{OUTPUT_DIR}}/tools/{{NAME}}.{{EXT}}` -- this struct holds only metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentExtractedTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub tool_type: String,
}

/// Agent's decision on how to handle a companion file.
///
/// For "transform" actions, the agent writes the transformed content to
/// `{{OUTPUT_DIR}}/companions/{{PATH}}` via `FileWrite`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompanionDecision {
    /// Relative path within the skill directory.
    pub path: String,
    /// Action: "keep" (copy as-is), "transform" (agent wrote transformed
    /// content to `{{OUTPUT_DIR}}/companions/{{PATH}}`), or "discard" (skip).
    pub action: String,
    /// Reason for the decision.
    #[serde(default)]
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Import error
// ---------------------------------------------------------------------------

/// Errors during skill import.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("file not found: {path}")]
    FileNotFound { path: String },

    #[error("file read error: {0}")]
    IoError(String),

    #[error("agent delegation failed: {0}")]
    DelegationFailed(String),

    #[error("agent returned invalid JSON: {0}")]
    InvalidAgentOutput(String),

    #[error("skill registration failed: {0}")]
    RegistrationFailed(String),

    #[error("temp directory error: {0}")]
    TempDirError(String),
}

#[derive(Debug, Deserialize)]
struct SecurityOutput {
    verdict: String,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    risk_level: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    permissions_needed: Option<PermissionsNeeded>,
}

/// Import a skill from a path using the full service-layer workflow.
///
/// Direct TOML imports are registered deterministically when sanitization is
/// disabled. Sanitized imports and non-TOML imports use the agent-assisted
/// ingestion service, with optional security screening first.
pub async fn import_skill_from_path(
    delegator: Arc<dyn AgentDelegator>,
    store_path: &Path,
    source_path: &Path,
    sanitize: bool,
) -> Result<SkillImportOutcome, String> {
    std::fs::create_dir_all(store_path)
        .map_err(|e| format!("Failed to create skills directory: {e}"))?;

    if !source_path.exists() {
        return Err(format!("Path not found: {}", source_path.display()));
    }

    let format = FormatDetector::from_path(source_path);
    if !sanitize && format == IngestionFormat::Toml {
        return import_toml_skill_direct(store_path, source_path).await;
    }

    if sanitize {
        if let Some(rejection) = screen_skill_security(&delegator, source_path).await? {
            return Ok(rejection);
        }
    }

    let store = FilesystemSkillStore::new(store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| format!("Failed to create registry: {e}"))?,
    ));
    let ingestion_service = SkillIngestionService::new(delegator, registry);

    Ok(match ingestion_service.import(source_path).await {
        Ok(result) => SkillImportOutcome {
            decision: import_decision_label(&result.decision).to_string(),
            classification: result.classification,
            skill_id: result.skill_id,
            error: result.rejection_reason,
            security_issues: result.security_issues,
            permissions_needed: None,
        },
        Err(e) => SkillImportOutcome::rejected(e.to_string()),
    })
}

async fn import_toml_skill_direct(
    store_path: &Path,
    source_path: &Path,
) -> Result<SkillImportOutcome, String> {
    let content =
        std::fs::read_to_string(source_path).map_err(|e| format!("Failed to read file: {e}"))?;
    let parser = ManifestParser::new(SkillConfig::default());
    let manifest = parser
        .parse(&content)
        .map_err(|e| format!("Failed to parse skill: {e}"))?;

    let store = FilesystemSkillStore::new(store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;
    let registry = SkillRegistryImpl::with_store(store)
        .await
        .map_err(|e| format!("Failed to create registry: {e}"))?;

    let skill_name = manifest.name.clone();
    registry
        .register(manifest)
        .await
        .map_err(|e| format!("Failed to register skill: {e}"))?;

    Ok(SkillImportOutcome {
        decision: "accepted".to_string(),
        classification: "direct_import".to_string(),
        skill_id: Some(skill_name),
        error: None,
        security_issues: vec![],
        permissions_needed: None,
    })
}

async fn screen_skill_security(
    delegator: &Arc<dyn AgentDelegator>,
    source_path: &Path,
) -> Result<Option<SkillImportOutcome>, String> {
    let source_content = read_security_source_content(source_path)?;
    let security_input = serde_json::json!({
        "source_content": source_content,
        "source_format": security_format_label(source_path),
    });

    let security_result = delegator
        .delegate(
            "skill-security-check",
            security_input,
            ContextStrategyHint::None,
            None,
        )
        .await;

    match security_result {
        Ok(output) => {
            let Ok(security) = serde_json::from_str::<SecurityOutput>(&output.text) else {
                return Ok(None);
            };
            match security.verdict.as_str() {
                "insecure" => Ok(Some(SkillImportOutcome {
                    decision: "rejected".to_string(),
                    classification: String::new(),
                    skill_id: None,
                    error: Some(format!(
                        "Security check failed ({}): {}",
                        security.risk_level, security.summary
                    )),
                    security_issues: security.issues,
                    permissions_needed: security.permissions_needed,
                })),
                "caution" => {
                    warn!(
                        risk_level = %security.risk_level,
                        summary = %security.summary,
                        issues = ?security.issues,
                        "Security check returned caution -- proceeding with ingestion"
                    );
                    Ok(None)
                }
                _ => Ok(None),
            }
        }
        Err(e) => {
            warn!(error = %e, "Security check agent failed -- proceeding with ingestion");
            Ok(None)
        }
    }
}

fn read_security_source_content(source_path: &Path) -> Result<String, String> {
    if !source_path.is_dir() {
        return std::fs::read_to_string(source_path)
            .map_err(|e| format!("Failed to read file: {e}"));
    }

    let entries = std::fs::read_dir(source_path)
        .map_err(|e| format!("Failed to read directory: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    Ok(entries.join("\n"))
}

fn security_format_label(source_path: &Path) -> &'static str {
    if source_path.is_dir() {
        return "directory";
    }

    match source_path.extension().and_then(|e| e.to_str()) {
        Some("toml") => "toml",
        Some("md" | "markdown") => "markdown",
        Some("yaml" | "yml") => "yaml",
        Some("json") => "json",
        _ => "plaintext",
    }
}

fn import_decision_label(decision: &ImportDecision) -> &'static str {
    match decision {
        ImportDecision::Accepted => "accepted",
        ImportDecision::Rejected => "rejected",
        ImportDecision::PartialAccept => "partial_accept",
        ImportDecision::Optimized => "optimized",
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Orchestrates third-party skill ingestion by delegating to the
/// `skill-ingestion` agent and registering the result.
///
/// Flow:
/// 1. Read source file (deterministic)
/// 2. Format detection (deterministic — `FormatDetector`)
/// 3. Delegate to `skill-ingestion` agent (LLM-assisted, with tool calling)
/// 4. Parse structured output
/// 5. Register in `SkillRegistry`
/// 6. Handle companion files based on agent decisions
///
/// Security screening is NOT performed here; it is the caller's
/// responsibility to invoke the `skill-security-check` agent beforehand.
pub struct SkillIngestionService {
    delegator: Arc<dyn AgentDelegator>,
    registry: Arc<tokio::sync::RwLock<SkillRegistryImpl>>,
}

impl SkillIngestionService {
    /// Create a new ingestion service.
    pub fn new(
        delegator: Arc<dyn AgentDelegator>,
        registry: Arc<tokio::sync::RwLock<SkillRegistryImpl>>,
    ) -> Self {
        Self {
            delegator,
            registry,
        }
    }

    /// Import a skill from a file path.
    ///
    /// Creates a temporary output directory, delegates to the
    /// `skill-ingestion` agent (which writes files there via `FileWrite`),
    /// then reads back the generated files and registers the skill.
    pub async fn import(&self, path: &Path) -> Result<ImportResult, ImportError> {
        // 1. Validate source file exists
        if !path.exists() {
            return Err(ImportError::FileNotFound {
                path: path.display().to_string(),
            });
        }

        // 2. Resolve source path, format, and main file name
        let source_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let format = FormatDetector::from_path(path);
        let format_str = format_to_str(format);
        let main_file_name = path.file_name().and_then(|n| n.to_str()).map(String::from);

        // 3. Gather existing skills for dedup context
        let existing_skills: Vec<String> = self.registry.read().await.list_names().await;

        // 4. Create temp directory for agent output
        let output_dir = TempDir::new()
            .map_err(|e| ImportError::TempDirError(format!("failed to create temp dir: {e}")))?;
        let output_dir_path = output_dir.path().to_string_lossy().to_string();

        // 5. Build markdown instruction for the agent.
        //    The agent reads the source file itself via FileRead and detects
        //    the format from the file extension.
        let skills_list = if existing_skills.is_empty() {
            "(none)".to_string()
        } else {
            existing_skills.join(", ")
        };
        let input = serde_json::Value::String(format!(
            "## Skill Ingestion Request\n\n\
             - **Source path**: `{source_path}`\n\
             - **Main file name**: `{main_file}`\n\
             - **Output directory**: `{output_dir}`\n\
             - **Existing skills** (for dedup): {skills}\n\n\
             Read the source file and any companion files in its directory, \
             then analyze, classify, and transform the skill.",
            source_path = source_path.display(),
            main_file = main_file_name.as_deref().unwrap_or("unknown"),
            output_dir = output_dir_path,
            skills = skills_list,
        ));

        info!(
            path = %path.display(),
            output_dir = %output_dir_path,
            "Delegating skill ingestion to agent"
        );

        let delegation_output = self
            .delegator
            .delegate("skill-ingestion", input, ContextStrategyHint::None, None)
            .await
            .map_err(|e| ImportError::DelegationFailed(e.to_string()))?;

        // 6. Parse agent output (metadata-only JSON)
        let json_str = extract_json_from_response(&delegation_output.text);
        let agent_output: AgentIngestionOutput = serde_json::from_str(json_str).map_err(|e| {
            ImportError::InvalidAgentOutput(format!(
                "failed to parse agent response: {e}\nraw: {}",
                &delegation_output.text[..delegation_output.text.floor_char_boundary(500)]
            ))
        })?;

        // 7. Handle rejection (temp dir auto-drops)
        let decision = match agent_output.decision.as_str() {
            "accepted" => ImportDecision::Accepted,
            "optimized" => ImportDecision::Optimized,
            "partial_accept" => ImportDecision::PartialAccept,
            _ => ImportDecision::Rejected,
        };

        if decision == ImportDecision::Rejected {
            info!(
                classification = %agent_output.classification,
                reason = ?agent_output.rejection_reason,
                "Skill rejected by agent"
            );
            return Ok(ImportResult {
                decision,
                classification: agent_output.classification,
                skill_id: None,
                rejection_reason: agent_output.rejection_reason,
                redirect_suggestion: agent_output.redirect_suggestion,
                security_issues: vec![],
                optimization_notes: None,
            });
        }

        if decision == ImportDecision::Optimized {
            info!(
                classification = %agent_output.classification,
                notes = ?agent_output.optimization_notes,
                "Skill optimized by agent"
            );
        }

        // 8. Read root.md from the temp output directory
        let root_md_path = output_dir.path().join("root.md");
        let root_content = tokio::fs::read_to_string(&root_md_path)
            .await
            .map_err(|e| {
                ImportError::InvalidAgentOutput(format!(
                    "agent did not write root.md to output dir: {e}"
                ))
            })?;

        // 9. Build manifest and register
        let manifest = Self::build_manifest(&agent_output, &root_content, format_str)?;
        let skill_name = manifest.name.clone();
        let skill_id_str = manifest.id.to_string();

        let reg = self.registry.read().await;
        let version = reg
            .register(manifest)
            .await
            .map_err(|e| ImportError::RegistrationFailed(e.to_string()))?;

        info!(
            skill_id = %skill_id_str,
            version = %version,
            name = %skill_name,
            classification = %agent_output.classification,
            "Skill successfully imported and registered"
        );

        // 10. Store sub-document content (read from temp dir)
        for sub_doc in &agent_output.sub_documents {
            let sub_doc_path = output_dir.path().join(&sub_doc.path);
            match tokio::fs::read_to_string(&sub_doc_path).await {
                Ok(content) => {
                    if let Err(e) = reg
                        .store_sub_document(&skill_id_str, &sub_doc.path, &content)
                        .await
                    {
                        warn!(
                            skill_id = %skill_id_str,
                            path = %sub_doc.path,
                            error = %e,
                            "Failed to store sub-document"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        skill_id = %skill_id_str,
                        path = %sub_doc.path,
                        error = %e,
                        "Agent declared sub-document but file not found in output dir"
                    );
                }
            }
        }

        // 11. Log extracted tools (future: register in ToolRegistry)
        if !agent_output.extracted_tools.is_empty() {
            info!(
                count = agent_output.extracted_tools.len(),
                "Agent extracted tools (not yet auto-registered)"
            );
        }

        // 12. Handle companion files based on agent decisions
        Self::handle_companion_files(
            path,
            &skill_name,
            &agent_output.companion_decisions,
            main_file_name.as_deref(),
            output_dir.path(),
            &reg,
        )?;

        // output_dir (TempDir) is dropped here, cleaning up the temp directory

        Ok(ImportResult {
            decision,
            classification: agent_output.classification,
            skill_id: Some(skill_id_str),
            rejection_reason: None,
            redirect_suggestion: None,
            security_issues: vec![],
            optimization_notes: agent_output.optimization_notes,
        })
    }

    /// Build a `SkillManifest` from the agent's metadata and the root
    /// content read from the output directory.
    fn build_manifest(
        agent_output: &AgentIngestionOutput,
        root_content: &str,
        format_str: &str,
    ) -> Result<SkillManifest, ImportError> {
        let manifest_data = agent_output.manifest.as_ref().ok_or_else(|| {
            ImportError::InvalidAgentOutput("accepted skill missing manifest".to_string())
        })?;

        let token_estimate = u32::try_from(root_content.chars().count() / 4).unwrap_or(0);
        let now = chrono::Utc::now();

        let sub_doc_refs: Vec<SubDocumentRef> = agent_output
            .sub_documents
            .iter()
            .map(|sd| SubDocumentRef {
                id: sd.path.clone(),
                path: sd.path.clone(),
                title: sd.title.clone(),
                load_condition: sd
                    .load_condition
                    .clone()
                    .unwrap_or_else(|| "on_demand".to_string()),
                token_estimate: sd.token_count,
            })
            .collect();

        let tags = manifest_data
            .classification
            .as_ref()
            .map(|c| c.domain.clone())
            .unwrap_or_default();

        Ok(SkillManifest {
            id: SkillId::from_string(&manifest_data.name),
            name: manifest_data.name.clone(),
            description: manifest_data.description.clone(),
            version: SkillVersion(manifest_data.version.clone()),
            tags,
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content: root_content.to_string(),
            sub_documents: sub_doc_refs,
            token_estimate,
            created_at: now,
            updated_at: now,
            classification: manifest_data.classification.as_ref().map(|c| {
                let skill_type = match c.skill_type.as_str() {
                    "api_call" => SkillClassificationType::ApiCall,
                    "tool_wrapper" => SkillClassificationType::ToolWrapper,
                    "agent_behavior" => SkillClassificationType::AgentBehavior,
                    "hybrid" => SkillClassificationType::Hybrid,
                    _ => SkillClassificationType::LlmReasoning,
                };
                SkillClassification {
                    skill_type,
                    domain: c.domain.clone(),
                    atomic: c.atomic,
                }
            }),
            constraints: manifest_data
                .constraints
                .as_ref()
                .map(|c| SkillConstraints {
                    max_input_tokens: c.max_input_tokens,
                    max_output_tokens: c.max_output_tokens,
                    requires_language: c.requires_language.clone(),
                }),
            security: None,
            references: None,
            author: Some("skill-ingestion-agent".to_string()),
            source_format: Some(format_str.to_string()),
            source_hash: None,
            state: Some(SkillState::Registered),
            root_path: Some("root.md".to_string()),
        })
    }

    /// Process companion file decisions from the agent output.
    ///
    /// - "keep" copies from the original source directory
    /// - "transform" copies from `{{OUTPUT_DIR}}/companions/{{PATH}}` (written by
    ///   the agent via `FileWrite`)
    /// - "discard" logs and skips
    fn handle_companion_files(
        source_path: &Path,
        skill_name: &str,
        companion_decisions: &[CompanionDecision],
        main_file_name: Option<&str>,
        output_dir: &Path,
        reg: &SkillRegistryImpl,
    ) -> Result<(), ImportError> {
        let Some(dir) = source_path.parent() else {
            return Ok(());
        };
        let Some(store_base) = reg.store_base_path() else {
            return Ok(());
        };

        let store = FilesystemSkillStore::new(store_base)
            .map_err(|e| ImportError::IoError(format!("failed to open store: {e}")))?;

        for decision in companion_decisions {
            let source_file = dir.join(&decision.path);
            match decision.action.as_str() {
                "keep" => {
                    if source_file.exists() {
                        let target = store.base_path().join(skill_name).join(&decision.path);
                        if let Some(parent) = target.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::copy(&source_file, &target) {
                            warn!(
                                skill = %skill_name,
                                path = %decision.path,
                                error = %e,
                                "Failed to copy companion file"
                            );
                        }
                    }
                }
                "transform" => {
                    let transformed_file = output_dir.join("companions").join(&decision.path);
                    if transformed_file.exists() {
                        let target = store.base_path().join(skill_name).join(&decision.path);
                        if let Some(parent) = target.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::copy(&transformed_file, &target) {
                            warn!(
                                skill = %skill_name,
                                path = %decision.path,
                                error = %e,
                                "Failed to copy transformed companion file"
                            );
                        }
                    } else {
                        warn!(
                            skill = %skill_name,
                            path = %decision.path,
                            "Agent declared transform but file not found in output dir"
                        );
                    }
                }
                "discard" => {
                    info!(
                        skill = %skill_name,
                        path = %decision.path,
                        reason = %decision.reason,
                        "Discarded companion file per agent decision"
                    );
                }
                _ => {
                    warn!(
                        skill = %skill_name,
                        path = %decision.path,
                        action = %decision.action,
                        "Unknown companion decision action, skipping"
                    );
                }
            }
        }

        // Fallback: if agent provided no companion decisions, copy
        // all companion files (backwards compatibility).
        if companion_decisions.is_empty() {
            if let Err(e) = store.copy_companion_files(skill_name, dir, main_file_name) {
                warn!(
                    skill = %skill_name,
                    error = %e,
                    "Failed to copy companion files (fallback)"
                );
            }
        }

        Ok(())
    }

    /// Import multiple skills from paths.
    pub async fn import_batch(&self, paths: &[&Path]) -> Vec<Result<ImportResult, ImportError>> {
        let mut results = Vec::with_capacity(paths.len());
        for path in paths {
            results.push(self.import(path).await);
        }
        results
    }
}

fn format_to_str(format: IngestionFormat) -> &'static str {
    match format {
        IngestionFormat::Toml => "toml",
        IngestionFormat::Markdown => "markdown",
        IngestionFormat::Yaml => "yaml",
        IngestionFormat::Json => "json",
        IngestionFormat::PlainText => "plaintext",
        IngestionFormat::Directory => "directory",
    }
}

/// Extract a JSON object from a response that may contain surrounding text.
///
/// Multi-turn agent execution accumulates intermediate assistant content
/// (from tool-calling iterations like `ShellExec` / `FileRead`) before the
/// final JSON output. This finds the first `{` and last `}` to extract the
/// JSON object, discarding any prefix or suffix.
fn extract_json_from_response(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                return &trimmed[start..=end];
            }
        }
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use y_core::agent::{AgentDelegator, DelegationError, DelegationOutput};

    // Mock delegator that returns configurable JSON output and writes
    // files to the agent's output_dir (extracted from the markdown input).
    #[derive(Debug)]
    struct MockDelegator {
        response: String,
        /// Files to write to `output_dir`: relative_path -> content.
        output_files: HashMap<String, String>,
    }

    impl MockDelegator {
        fn with_response(json: &str) -> Self {
            Self {
                response: json.to_string(),
                output_files: HashMap::new(),
            }
        }

        fn with_response_and_files(json: &str, files: HashMap<String, String>) -> Self {
            Self {
                response: json.to_string(),
                output_files: files,
            }
        }

        /// Extract the output directory path from the markdown input string.
        fn extract_output_dir(input: &str) -> Option<String> {
            for line in input.lines() {
                if let Some(rest) = line.strip_prefix("- **Output directory**: `") {
                    return rest.strip_suffix('`').map(String::from);
                }
            }
            None
        }
    }

    #[async_trait::async_trait]
    impl AgentDelegator for MockDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            // Simulate agent writing files to output_dir
            let input_str = input.as_str().unwrap_or_default();
            if let Some(output_dir) = Self::extract_output_dir(input_str) {
                let output_path = Path::new(&output_dir);
                for (rel_path, content) in &self.output_files {
                    let file_path = output_path.join(rel_path);
                    if let Some(parent) = file_path.parent() {
                        std::fs::create_dir_all(parent).unwrap();
                    }
                    std::fs::write(&file_path, content).unwrap();
                }
            }

            Ok(DelegationOutput {
                text: self.response.clone(),
                tokens_used: 100,
                input_tokens: 80,
                output_tokens: 20,
                model_used: "mock".to_string(),
                duration_ms: 500,
            })
        }
    }

    fn test_registry() -> Arc<tokio::sync::RwLock<SkillRegistryImpl>> {
        Arc::new(tokio::sync::RwLock::new(SkillRegistryImpl::new()))
    }

    fn accepted_response() -> String {
        serde_json::json!({
            "decision": "accepted",
            "classification": "llm_reasoning",
            "manifest": {
                "name": "test-humanizer",
                "version": "1.0.0",
                "description": "Removes AI artifacts from text",
                "classification": {
                    "type": "llm_reasoning",
                    "domain": ["writing"],
                    "tags": ["humanize", "rewrite"],
                    "atomic": true
                },
                "constraints": {
                    "max_input_tokens": 8000,
                    "max_output_tokens": 8000
                }
            },
            "sub_documents": [],
            "extracted_tools": []
        })
        .to_string()
    }

    fn accepted_files() -> HashMap<String, String> {
        let mut files = HashMap::new();
        files.insert(
            "root.md".to_string(),
            "# Humanizer\n\nRemove AI artifacts from text.\n\n## Rules\n\n1. Detect exaggerated language.\n2. Replace vague statements.".to_string(),
        );
        files
    }

    fn rejected_response() -> String {
        serde_json::json!({
            "decision": "rejected",
            "classification": "api_call",
            "rejection_reason": "This skill describes API interactions",
            "redirect_suggestion": "Register as a Tool via y-agent tool register"
        })
        .to_string()
    }

    /// T-SK-A2-01: Accepted skill is registered.
    #[tokio::test]
    async fn test_import_accepted_skill() {
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &accepted_response(),
            accepted_files(),
        ));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, Arc::clone(&registry));

        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("humanizer.md");
        tokio::fs::write(&skill_path, "# Humanizer\nRemove AI artifacts.")
            .await
            .unwrap();

        let result = service.import(&skill_path).await.unwrap();
        assert_eq!(result.decision, ImportDecision::Accepted);
        assert_eq!(result.classification, "llm_reasoning");
        assert!(result.skill_id.is_some());
        assert!(result.rejection_reason.is_none());

        // Verify registration
        let reg = registry.read().await;
        let names = reg.list_names().await;
        assert!(names.contains(&"test-humanizer".to_string()));
    }

    /// T-SK-A2-02: Rejected skill is not registered.
    #[tokio::test]
    async fn test_import_rejected_skill() {
        let delegator = Arc::new(MockDelegator::with_response(&rejected_response()));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, Arc::clone(&registry));

        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("api-wrapper.yaml");
        tokio::fs::write(&skill_path, "openapi: 3.0.0\ninfo: {title: API}")
            .await
            .unwrap();

        let result = service.import(&skill_path).await.unwrap();
        assert_eq!(result.decision, ImportDecision::Rejected);
        assert_eq!(result.classification, "api_call");
        assert!(result.skill_id.is_none());
        assert!(result.rejection_reason.is_some());
        assert!(result.redirect_suggestion.is_some());
    }

    /// T-SK-A2-03: File not found returns error.
    #[tokio::test]
    async fn test_import_file_not_found() {
        let delegator = Arc::new(MockDelegator::with_response("{}"));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, registry);

        let result = service.import(Path::new("/nonexistent/file.md")).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImportError::FileNotFound { .. }
        ));
    }

    /// T-SK-A2-04: Invalid agent JSON output returns error.
    #[tokio::test]
    async fn test_import_invalid_agent_output() {
        let delegator = Arc::new(MockDelegator::with_response("not valid json"));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, registry);

        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("test.md");
        tokio::fs::write(&skill_path, "# Test skill").await.unwrap();

        let result = service.import(&skill_path).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImportError::InvalidAgentOutput(_)
        ));
    }

    /// T-SK-A2-05: Batch import processes multiple files.
    #[tokio::test]
    async fn test_import_batch() {
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &accepted_response(),
            accepted_files(),
        ));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, Arc::clone(&registry));

        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("skill1.md");
        let path2 = dir.path().join("skill2.md");
        tokio::fs::write(&path1, "# Skill 1").await.unwrap();
        tokio::fs::write(&path2, "# Skill 2").await.unwrap();

        let results = service.import_batch(&[&path1, &path2]).await;
        assert_eq!(results.len(), 2);
        // First should succeed; second registers same name (creates new version)
        assert!(results[0].is_ok());
    }

    // --- extract_json_from_response tests ---

    #[test]
    fn test_extract_json_clean() {
        let input = r#"{"decision": "rejected"}"#;
        assert_eq!(extract_json_from_response(input), input);
    }

    #[test]
    fn test_extract_json_with_prefix() {
        let input = "\n\nSome thinking...\n\n{\"decision\": \"rejected\"}";
        assert_eq!(
            extract_json_from_response(input),
            "{\"decision\": \"rejected\"}"
        );
    }

    #[test]
    fn test_extract_json_with_markdown_fence() {
        let input = "```json\n{\"decision\": \"rejected\"}\n```";
        assert_eq!(
            extract_json_from_response(input),
            "{\"decision\": \"rejected\"}"
        );
    }

    #[test]
    fn test_extract_json_no_braces() {
        let input = "no json here";
        assert_eq!(extract_json_from_response(input), input);
    }

    /// T-SK-A2-06: Agent response with accumulated content prefix parses correctly.
    #[tokio::test]
    async fn test_import_with_accumulated_content_prefix() {
        // Simulate multi-turn output: blank lines + thinking text before final JSON.
        let prefix = "\n\nLet me analyze this skill...\n\n";
        let json = rejected_response();
        let response_with_prefix = format!("{prefix}{json}");

        let delegator = Arc::new(MockDelegator::with_response(&response_with_prefix));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, Arc::clone(&registry));

        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("api-wrapper.yaml");
        tokio::fs::write(&skill_path, "openapi: 3.0.0\ninfo: {title: API}")
            .await
            .unwrap();

        let result = service.import(&skill_path).await.unwrap();
        assert_eq!(result.decision, ImportDecision::Rejected);
        assert_eq!(result.classification, "api_call");
    }

    fn optimized_response() -> String {
        serde_json::json!({
            "decision": "optimized",
            "classification": "api_call",
            "optimization_notes": "Transformed URL templates into natural-language rule tables. Decomposed into root + 2 sub-documents for lazy loading. Extracted WebFetch as tool reference.",
            "manifest": {
                "name": "multi-search-engine",
                "version": "1.0.0",
                "description": "Guide LLM to construct search queries for multiple search engines",
                "classification": {
                    "type": "api_call",
                    "domain": ["search", "web"],
                    "tags": ["search", "multi-engine"],
                    "atomic": false
                },
                "constraints": {
                    "max_input_tokens": 4000,
                    "max_output_tokens": 4000
                }
            },
            "sub_documents": [
                {
                    "path": "details/general-engines.md",
                    "title": "General search engines",
                    "token_count": 400,
                    "load_condition": "when searching general web"
                }
            ],
            "extracted_tools": [],
            "companion_decisions": []
        })
        .to_string()
    }

    fn optimized_files() -> HashMap<String, String> {
        let mut files = HashMap::new();
        files.insert(
            "root.md".to_string(),
            "# Multi-Search-Engine\n\nGuide for constructing search queries across multiple engines.\n\n## Core Rules\n\n1. Identify the user's search intent.\n2. Select the appropriate engine from the sub-document index.\n3. Construct the URL using the engine's pattern.\n4. Use the `WebFetch` tool to retrieve results.\n\n## Sub-Document Index\n\n| Document | Load Condition |\n|----------|----------------|\n| details/general-engines.md | When searching Google, Bing, or DuckDuckGo |\n| details/academic-engines.md | When searching academic sources |".to_string(),
        );
        files.insert(
            "details/general-engines.md".to_string(),
            "# General Search Engines\n\n| Engine | URL Pattern | Notes |\n|--------|------------|-------|\n| Google | `https://google.com/search?q={{QUERY}}` | Default engine |\n| Bing | `https://bing.com/search?q={{QUERY}}` | Alternative |".to_string(),
        );
        files
    }

    /// T-SK-A2-07: Optimized skill is registered successfully.
    #[tokio::test]
    async fn test_import_optimized_skill() {
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &optimized_response(),
            optimized_files(),
        ));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, Arc::clone(&registry));

        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("multi-search.md");
        tokio::fs::write(&skill_path, "# Multi Search Engine\nURL templates...")
            .await
            .unwrap();

        let result = service.import(&skill_path).await.unwrap();
        assert_eq!(result.decision, ImportDecision::Optimized);
        assert_eq!(result.classification, "api_call");
        assert!(result.skill_id.is_some());
        assert!(result.optimization_notes.is_some());
        assert!(result
            .optimization_notes
            .unwrap()
            .contains("natural-language"));

        // Verify registration
        let reg = registry.read().await;
        let names = reg.list_names().await;
        assert!(names.contains(&"multi-search-engine".to_string()));
    }

    /// T-SK-A2-08: Agent returns accepted but fails to write root.md -> error.
    #[tokio::test]
    async fn test_import_missing_root_md() {
        // No files written -- agent returned accepted but didn't write root.md
        let delegator = Arc::new(MockDelegator::with_response(&accepted_response()));
        let registry = test_registry();
        let service = SkillIngestionService::new(delegator, registry);

        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("broken-skill.md");
        tokio::fs::write(&skill_path, "# Broken skill")
            .await
            .unwrap();

        let result = service.import(&skill_path).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImportError::InvalidAgentOutput(_)
        ));
    }

    #[tokio::test]
    async fn test_import_skill_from_path_rejects_missing_source() {
        let delegator = Arc::new(MockDelegator::with_response("{}"));
        let store_dir = tempfile::tempdir().unwrap();

        let result = import_skill_from_path(
            delegator,
            store_dir.path(),
            Path::new("/missing/skill.md"),
            false,
        )
        .await;

        assert!(result.unwrap_err().contains("Path not found"));
    }

    #[tokio::test]
    async fn test_import_skill_from_path_maps_security_rejection() {
        let security_response = serde_json::json!({
            "verdict": "insecure",
            "risk_level": "high",
            "summary": "Attempts to read private keys",
            "issues": ["reads private key material"],
            "permissions_needed": {
                "files_read": ["~/.ssh"],
                "files_write": [],
                "network": [],
                "commands": ["cat ~/.ssh/id_rsa"]
            }
        })
        .to_string();
        let delegator = Arc::new(MockDelegator::with_response(&security_response));
        let store_dir = tempfile::tempdir().unwrap();
        let source_dir = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("unsafe.md");
        tokio::fs::write(&source_path, "# Unsafe\nRead private keys.")
            .await
            .unwrap();

        let result = import_skill_from_path(delegator, store_dir.path(), &source_path, true)
            .await
            .unwrap();

        assert_eq!(result.decision, "rejected");
        assert_eq!(result.security_issues, vec!["reads private key material"]);
        assert_eq!(
            result.permissions_needed.unwrap().files_read,
            vec!["~/.ssh"]
        );
    }
}
