//! Skill creation service -- delegates dynamic skill generation to the
//! `skill-creator` agent and registers the result in the skill registry.

use std::path::{Component, Path};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use tempfile::TempDir;

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_core::skill::{SkillManifest, SkillReferences, SkillRegistry};
use y_skills::SkillRegistryImpl;

use crate::skill_files::resolve_skill_write_path;
use crate::skill_ingestion::{
    extract_json_from_response, AgentClassificationOutput, AgentConstraintsOutput,
    AgentExtractedTool, AgentSubDocOutput,
};
use crate::skill_manifest_helper::{
    build_skill_manifest_from_agent_output, store_sub_documents, SkillManifestInput,
};

/// Maximum size of a single companion artifact copied into a skill directory.
const MAX_ARTIFACT_BYTES: u64 = 5 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Creation result
// ---------------------------------------------------------------------------

/// Result of a skill creation operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationResult {
    pub decision: CreationDecision,
    pub skill_id: Option<String>,
    pub rejection_reason: Option<String>,
    pub redirect_suggestion: Option<String>,
    pub optimization_notes: Option<String>,
}

/// Creation decision outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreationDecision {
    Created,
    Rejected,
}

/// Presentation-friendly result for a skill creation request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillCreateOutcome {
    pub decision: String,
    pub skill_id: Option<String>,
    pub error: Option<String>,
}

impl SkillCreateOutcome {
    fn rejected(error: String) -> Self {
        Self {
            decision: "rejected".to_string(),
            skill_id: None,
            error: Some(error),
        }
    }
}

// ---------------------------------------------------------------------------
// Agent output schema
// ---------------------------------------------------------------------------

/// Structured output from the `skill-creator` agent.
#[derive(Debug, Deserialize)]
struct AgentCreatorOutput {
    decision: String,
    #[serde(default)]
    rejection_reason: Option<String>,
    #[serde(default)]
    redirect_suggestion: Option<String>,
    #[serde(default)]
    optimization_notes: Option<String>,
    #[serde(default)]
    manifest: Option<AgentCreatorManifest>,
    #[serde(default)]
    sub_documents: Vec<AgentSubDocOutput>,
    #[serde(default)]
    extracted_tools: Vec<AgentExtractedTool>,
}

#[derive(Debug, Deserialize)]
struct AgentCreatorManifest {
    name: String,
    #[serde(default = "crate::skill_ingestion::default_version")]
    version: String,
    description: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    classification: Option<AgentClassificationOutput>,
    #[serde(default)]
    constraints: Option<AgentConstraintsOutput>,
    #[allow(dead_code)]
    #[serde(default)]
    root: Option<AgentRootOutput>,
    #[serde(default)]
    references: Option<AgentReferencesOutput>,
}

#[derive(Debug, Deserialize)]
struct AgentRootOutput {
    #[allow(dead_code)]
    #[serde(default)]
    path: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AgentReferencesOutput {
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    knowledge_bases: Vec<String>,
}

// ---------------------------------------------------------------------------
// Creation error
// ---------------------------------------------------------------------------

/// Errors during skill creation.
#[derive(Debug, thiserror::Error)]
pub enum CreationError {
    #[error("agent delegation failed: {0}")]
    DelegationFailed(String),

    #[error("agent returned invalid JSON: {0}")]
    InvalidAgentOutput(String),

    #[error("skill registration failed: {0}")]
    RegistrationFailed(String),

    #[error("temp directory error: {0}")]
    TempDirError(String),
}

// ---------------------------------------------------------------------------
// Public convenience function
// ---------------------------------------------------------------------------

/// Create a skill from a natural-language description using the full
/// service-layer workflow.
pub async fn create_skill_from_request(
    delegator: Arc<dyn AgentDelegator>,
    store_path: &std::path::Path,
    request: &str,
    domain_hints: Option<&[String]>,
    language: Option<&str>,
) -> Result<SkillCreateOutcome, String> {
    std::fs::create_dir_all(store_path)
        .map_err(|e| format!("Failed to create skills directory: {e}"))?;

    let store = y_skills::FilesystemSkillStore::new(store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| format!("Failed to create registry: {e}"))?,
    ));
    let service = SkillCreationService::new(delegator, registry);

    Ok(
        match service.create(request, domain_hints, language).await {
            Ok(result) => SkillCreateOutcome {
                decision: creation_decision_label(result.decision).to_string(),
                skill_id: result.skill_id,
                error: result.rejection_reason,
            },
            Err(e) => SkillCreateOutcome::rejected(e.to_string()),
        },
    )
}

fn creation_decision_label(decision: CreationDecision) -> &'static str {
    match decision {
        CreationDecision::Created => "created",
        CreationDecision::Rejected => "rejected",
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Orchestrates dynamic skill creation by delegating to the
/// `skill-creator` agent and registering the result.
///
/// Flow:
/// 1. Gather existing skill names for dedup
/// 2. Create temp output directory
/// 3. Delegate to `skill-creator` agent (LLM-assisted, with tool calling)
/// 4. Parse structured output
/// 5. Read generated files from temp directory
/// 6. Register in `SkillRegistry`
pub struct SkillCreationService {
    delegator: Arc<dyn AgentDelegator>,
    registry: Arc<tokio::sync::RwLock<SkillRegistryImpl>>,
}

impl SkillCreationService {
    pub fn new(
        delegator: Arc<dyn AgentDelegator>,
        registry: Arc<tokio::sync::RwLock<SkillRegistryImpl>>,
    ) -> Self {
        Self {
            delegator,
            registry,
        }
    }

    /// Create a new skill from a natural-language description.
    ///
    /// Creates a temporary output directory, delegates to the
    /// `skill-creator` agent (which writes files there via `FileWrite`),
    /// then reads back the generated files and registers the skill.
    pub async fn create(
        &self,
        request: &str,
        domain_hints: Option<&[String]>,
        language: Option<&str>,
    ) -> Result<CreationResult, CreationError> {
        let existing_skills: Vec<String> = self.registry.read().await.list_names().await;

        let output_dir = TempDir::new()
            .map_err(|e| CreationError::TempDirError(format!("failed to create temp dir: {e}")))?;
        let output_dir_path = output_dir.path().to_string_lossy().to_string();

        let skills_list = if existing_skills.is_empty() {
            "(none)".to_string()
        } else {
            existing_skills.join(", ")
        };

        let domain_section = match domain_hints {
            Some(hints) if !hints.is_empty() => {
                format!("- **Domain hints**: {}\n", hints.join(", "))
            }
            _ => String::new(),
        };

        let language_section = match language {
            Some(lang) if !lang.is_empty() => format!("- **Language**: {lang}\n"),
            _ => String::new(),
        };

        let input = serde_json::Value::String(format!(
            "## Skill Creation Request\n\n\
             - **Skill request**: {request}\n\
             - **Output directory**: `{output_dir_path}`\n\
             {domain_section}{language_section}\
             - **Existing skills** (for dedup): {skills_list}\n\n\
             Create a new skill based on the request above.",
        ));

        info!(
            output_dir = %output_dir_path,
            "Delegating skill creation to agent"
        );

        let delegation_output = self
            .delegator
            .delegate("skill-creator", input, ContextStrategyHint::None, None)
            .await
            .map_err(|e| CreationError::DelegationFailed(e.to_string()))?;

        let json_str = extract_json_from_response(&delegation_output.text);
        let agent_output: AgentCreatorOutput = serde_json::from_str(json_str).map_err(|e| {
            CreationError::InvalidAgentOutput(format!(
                "failed to parse agent response: {e}\nraw: {}",
                &delegation_output.text[..delegation_output
                    .text
                    .floor_char_boundary(500)
                    .min(delegation_output.text.len())]
            ))
        })?;

        let decision = match agent_output.decision.as_str() {
            "created" => CreationDecision::Created,
            _ => CreationDecision::Rejected,
        };

        if decision == CreationDecision::Rejected {
            info!(
                reason = ?agent_output.rejection_reason,
                "Skill creation rejected by agent"
            );
            return Ok(CreationResult {
                decision,
                skill_id: None,
                rejection_reason: agent_output.rejection_reason,
                redirect_suggestion: agent_output.redirect_suggestion,
                optimization_notes: None,
            });
        }

        let root_md_path = output_dir.path().join("root.md");
        let root_content = tokio::fs::read_to_string(&root_md_path)
            .await
            .map_err(|e| {
                CreationError::InvalidAgentOutput(format!(
                    "agent did not write root.md to output dir: {e}"
                ))
            })?;

        let manifest = Self::build_manifest(&agent_output, &root_content)?;
        let skill_name = manifest.name.clone();
        let skill_id_str = manifest.id.to_string();

        let reg = self.registry.read().await;
        let version = reg
            .register(manifest)
            .await
            .map_err(|e| CreationError::RegistrationFailed(e.to_string()))?;

        info!(
            skill_id = %skill_id_str,
            version = %version,
            name = %skill_name,
            "Skill successfully created and registered"
        );

        store_sub_documents(
            &reg,
            &skill_id_str,
            &agent_output.sub_documents,
            output_dir.path(),
        )
        .await;

        if !agent_output.extracted_tools.is_empty() {
            if let Some(store_base) = reg.store_base_path() {
                let skill_dir = store_base.join(&skill_name);
                Self::copy_artifacts(
                    &agent_output.extracted_tools,
                    output_dir.path(),
                    &skill_dir,
                    &skill_name,
                );
            } else {
                warn!(
                    skill = %skill_name,
                    "extracted tools declared but store has no base path; skipping artifact copy"
                );
            }
        }

        Ok(CreationResult {
            decision,
            skill_id: Some(skill_id_str),
            rejection_reason: None,
            redirect_suggestion: None,
            optimization_notes: agent_output.optimization_notes,
        })
    }

    /// Copy companion artifacts (e.g. scripts) into the registered skill
    /// directory.
    ///
    /// Two sources, in priority order, per declared tool:
    /// 1. `source_path` -- an existing absolute file the agent referenced;
    ///    copied verbatim (the agent never regenerates such content).
    /// 2. Otherwise, a file the agent wrote to `output_dir/<dest_path>` (or
    ///    `output_dir/tools/<name>`), mirroring the ingestion pipeline.
    ///
    /// All destinations are guarded against path traversal via
    /// [`resolve_skill_write_path`]. Failures are logged, never fatal -- the
    /// skill is already registered and a missing artifact must not abort it.
    fn copy_artifacts(
        tools: &[AgentExtractedTool],
        output_dir: &Path,
        skill_dir: &Path,
        skill_name: &str,
    ) {
        for tool in tools {
            let Some(dest_rel) = Self::artifact_dest_rel(tool) else {
                warn!(
                    skill = %skill_name,
                    tool = %tool.name,
                    "cannot determine destination path for artifact; skipping"
                );
                continue;
            };

            let source = match &tool.source_path {
                Some(src) if !src.is_empty() => Path::new(src).to_path_buf(),
                _ => output_dir.join(&dest_rel),
            };

            if !source.is_file() {
                warn!(
                    skill = %skill_name,
                    tool = %tool.name,
                    source = %source.display(),
                    "artifact source not found or not a regular file; skipping"
                );
                continue;
            }

            if let Ok(meta) = std::fs::metadata(&source) {
                if meta.len() > MAX_ARTIFACT_BYTES {
                    warn!(
                        skill = %skill_name,
                        tool = %tool.name,
                        bytes = meta.len(),
                        "artifact exceeds size limit; skipping"
                    );
                    continue;
                }
            }

            // `dest_rel` contains only normal components (enforced by
            // `artifact_dest_rel`), so joining it cannot escape `skill_dir`.
            // Create the parent first so the canonicalizing guard below can
            // resolve it.
            if let Some(parent) = skill_dir.join(&dest_rel).parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    warn!(
                        skill = %skill_name,
                        tool = %tool.name,
                        error = %e,
                        "failed to create artifact parent dir; skipping"
                    );
                    continue;
                }
            }

            let target = match resolve_skill_write_path(skill_dir, Path::new(&dest_rel)) {
                Ok(target) => target,
                Err(e) => {
                    warn!(
                        skill = %skill_name,
                        tool = %tool.name,
                        dest = %dest_rel,
                        error = %e,
                        "rejected artifact destination; skipping"
                    );
                    continue;
                }
            };

            match std::fs::copy(&source, &target) {
                Ok(_) => info!(
                    skill = %skill_name,
                    tool = %tool.name,
                    dest = %dest_rel,
                    "copied skill artifact"
                ),
                Err(e) => warn!(
                    skill = %skill_name,
                    tool = %tool.name,
                    error = %e,
                    "failed to copy skill artifact"
                ),
            }
        }
    }

    /// Resolve the safe skill-relative destination for an artifact.
    ///
    /// Uses `dest_path` when provided, else `tools/<source-file-name>`. Returns
    /// `None` when the path is absolute or escapes the skill directory.
    fn artifact_dest_rel(tool: &AgentExtractedTool) -> Option<String> {
        let rel = match &tool.dest_path {
            Some(dest) if !dest.is_empty() => dest.clone(),
            _ => {
                let file_name = tool
                    .source_path
                    .as_deref()
                    .map(Path::new)
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned())?;
                format!("tools/{file_name}")
            }
        };

        let path = Path::new(&rel);
        let safe = path.components().all(|c| matches!(c, Component::Normal(_)));
        if safe && path.file_name().is_some() {
            Some(rel)
        } else {
            None
        }
    }

    fn build_manifest(
        agent_output: &AgentCreatorOutput,
        root_content: &str,
    ) -> Result<SkillManifest, CreationError> {
        let manifest_data = agent_output.manifest.as_ref().ok_or_else(|| {
            CreationError::InvalidAgentOutput("created skill missing manifest".to_string())
        })?;

        // Creation merges classification domain and tags (sorted, de-duped)
        // and carries references plus their knowledge bases.
        let tags = manifest_data
            .classification
            .as_ref()
            .map(|c| {
                let mut t = c.domain.clone();
                t.extend(c.tags.clone());
                t.sort();
                t.dedup();
                t
            })
            .unwrap_or_default();

        let references = manifest_data.references.as_ref().map(|r| SkillReferences {
            tools: r.tools.clone(),
            skills: r.skills.clone(),
            knowledge_bases: r.knowledge_bases.clone(),
        });

        let knowledge_bases = manifest_data
            .references
            .as_ref()
            .map(|r| r.knowledge_bases.clone())
            .unwrap_or_default();

        let author = Some(
            manifest_data
                .author
                .clone()
                .unwrap_or_else(|| "skill-creator-agent".to_string()),
        );

        Ok(build_skill_manifest_from_agent_output(SkillManifestInput {
            name: &manifest_data.name,
            version: &manifest_data.version,
            description: &manifest_data.description,
            classification: manifest_data.classification.as_ref(),
            constraints: manifest_data.constraints.as_ref(),
            sub_documents: &agent_output.sub_documents,
            root_content,
            tags,
            references,
            knowledge_bases,
            author,
            source_format: Some("generated".to_string()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use y_core::agent::{AgentDelegator, DelegationError, DelegationOutput};

    #[derive(Debug)]
    struct MockDelegator {
        response: String,
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

    fn created_response() -> String {
        serde_json::json!({
            "decision": "created",
            "optimization_notes": "Single root document sufficient for this skill.",
            "manifest": {
                "name": "summarize-academic",
                "version": "1.0.0",
                "description": "Summarize academic papers into structured abstracts",
                "author": "skill-creator-agent",
                "classification": {
                    "type": "llm_reasoning",
                    "domain": ["academic", "summarization"],
                    "tags": ["summarize", "paper", "abstract"],
                    "atomic": true
                },
                "constraints": {
                    "max_input_tokens": 8000,
                    "max_output_tokens": 4000
                },
                "root": {
                    "path": "root.md",
                    "token_count": 600
                },
                "references": {
                    "tools": [],
                    "skills": [],
                    "knowledge_bases": []
                }
            },
            "sub_documents": [],
            "extracted_tools": []
        })
        .to_string()
    }

    fn created_files() -> HashMap<String, String> {
        let mut files = HashMap::new();
        files.insert(
            "root.md".to_string(),
            "# Summarize Academic\n\nSummarize academic papers.\n\n## Rules\n\n1. Extract the thesis.\n2. Identify methodology.\n3. State key findings.".to_string(),
        );
        files
    }

    fn rejected_response() -> String {
        serde_json::json!({
            "decision": "rejected",
            "rejection_reason": "This request describes a CLI wrapper, not an LLM reasoning task",
            "redirect_suggestion": "Tool System"
        })
        .to_string()
    }

    #[tokio::test]
    async fn test_created_skill_is_listable_via_skill_service() {
        // Regression: a skill created through the service must be immediately
        // visible via SkillService::list -- the exact read path the GUI skill
        // panel uses. Guards the register -> persist -> load_all round-trip
        // (including non-kebab names like the underscore form below).
        let store = tempfile::TempDir::new().unwrap();
        let response = serde_json::json!({
            "decision": "created",
            "manifest": {
                "name": "ufa_secrets_analysis",
                "version": "1.0.0",
                "description": "Analyze the Secrets sheet from UFA xlsx files",
                "author": "skill-creator-agent",
                "classification": {
                    "type": "llm_reasoning",
                    "domain": ["security"],
                    "tags": ["secrets"],
                    "atomic": true
                },
                "constraints": {},
                "root": { "path": "root.md", "token_count": 100 },
                "references": { "tools": ["ShellExec"], "skills": [], "knowledge_bases": [] }
            },
            "sub_documents": [],
            "extracted_tools": []
        })
        .to_string();
        let mut files = HashMap::new();
        files.insert(
            "root.md".to_string(),
            "# UFA Secrets Analysis\n\nAnalyze the Secrets sheet.".to_string(),
        );
        let delegator = Arc::new(MockDelegator::with_response_and_files(&response, files));

        let outcome =
            create_skill_from_request(delegator, store.path(), "Analyze UFA secrets", None, None)
                .await
                .unwrap();
        assert_eq!(outcome.decision, "created");

        let listed = crate::skill_service::SkillService::new(store.path())
            .list()
            .await
            .unwrap();
        assert!(listed.iter().any(|s| s.name == "ufa_secrets_analysis"));
    }

    #[tokio::test]
    async fn test_create_skill_success() {
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &created_response(),
            created_files(),
        ));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, Arc::clone(&registry));

        let result = service
            .create(
                "Summarize academic papers into structured abstracts",
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.decision, CreationDecision::Created);
        assert!(result.skill_id.is_some());
        assert!(result.rejection_reason.is_none());

        let reg = registry.read().await;
        let names = reg.list_names().await;
        assert!(names.contains(&"summarize-academic".to_string()));
    }

    #[tokio::test]
    async fn test_create_skill_rejected() {
        let delegator = Arc::new(MockDelegator::with_response(&rejected_response()));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, Arc::clone(&registry));

        let result = service
            .create("Wrap the curl command for API testing", None, None)
            .await
            .unwrap();

        assert_eq!(result.decision, CreationDecision::Rejected);
        assert!(result.skill_id.is_none());
        assert!(result.rejection_reason.is_some());
        assert!(result.redirect_suggestion.is_some());
    }

    #[tokio::test]
    async fn test_create_skill_invalid_agent_output() {
        let delegator = Arc::new(MockDelegator::with_response("not valid json"));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, registry);

        let result = service.create("Some skill request", None, None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CreationError::InvalidAgentOutput(_)
        ));
    }

    #[tokio::test]
    async fn test_create_skill_missing_root_md() {
        let delegator = Arc::new(MockDelegator::with_response(&created_response()));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, registry);

        let result = service.create("Some skill request", None, None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CreationError::InvalidAgentOutput(_)
        ));
    }

    #[tokio::test]
    async fn test_create_skill_with_sub_documents() {
        let response = serde_json::json!({
            "decision": "created",
            "manifest": {
                "name": "review-code-rust",
                "version": "1.0.0",
                "description": "Review Rust code for idiomatic patterns and common pitfalls",
                "author": "skill-creator-agent",
                "classification": {
                    "type": "llm_reasoning",
                    "domain": ["code-review", "rust"],
                    "tags": ["review", "rust", "idiomatic"],
                    "atomic": true
                },
                "constraints": {},
                "root": { "path": "root.md", "token_count": 800 },
                "references": { "tools": [], "skills": [], "knowledge_bases": [] }
            },
            "sub_documents": [
                {
                    "path": "details/ownership-patterns.md",
                    "title": "Ownership and borrowing patterns",
                    "token_count": 1200,
                    "load_condition": "when reviewing ownership or lifetime issues"
                }
            ],
            "extracted_tools": []
        })
        .to_string();

        let mut files = HashMap::new();
        files.insert(
            "root.md".to_string(),
            "# Rust Code Review\n\nReview Rust code.\n\n## Sub-Document Index\n\n| Document | Load Condition |\n|----------|----------------|\n| details/ownership-patterns.md | Ownership issues |".to_string(),
        );
        files.insert(
            "details/ownership-patterns.md".to_string(),
            "# Ownership Patterns\n\nCommon ownership pitfalls in Rust.".to_string(),
        );

        let delegator = Arc::new(MockDelegator::with_response_and_files(&response, files));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, Arc::clone(&registry));

        let result = service
            .create("Review Rust code for idiomatic patterns", None, None)
            .await
            .unwrap();

        assert_eq!(result.decision, CreationDecision::Created);
        assert!(result.skill_id.is_some());

        let reg = registry.read().await;
        let names = reg.list_names().await;
        assert!(names.contains(&"review-code-rust".to_string()));
    }

    #[tokio::test]
    async fn test_create_skill_with_domain_and_language() {
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &created_response(),
            created_files(),
        ));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, Arc::clone(&registry));

        let domain_hints = vec!["academic".to_string(), "writing".to_string()];
        let result = service
            .create("Summarize papers", Some(&domain_hints), Some("en"))
            .await
            .unwrap();

        assert_eq!(result.decision, CreationDecision::Created);
    }

    #[tokio::test]
    async fn test_create_skill_with_accumulated_content_prefix() {
        let prefix = "\n\nLet me analyze the request...\n\n";
        let json = rejected_response();
        let response_with_prefix = format!("{prefix}{json}");

        let delegator = Arc::new(MockDelegator::with_response(&response_with_prefix));
        let registry = test_registry();
        let service = SkillCreationService::new(delegator, Arc::clone(&registry));

        let result = service.create("Some request", None, None).await.unwrap();
        assert_eq!(result.decision, CreationDecision::Rejected);
    }

    /// Build a `created` agent response that declares one extracted tool.
    fn created_response_with_tool(skill_name: &str, tool: serde_json::Value) -> String {
        serde_json::json!({
            "decision": "created",
            "manifest": {
                "name": skill_name,
                "version": "1.0.0",
                "description": "A skill with a companion artifact",
                "author": "skill-creator-agent",
                "classification": {
                    "type": "llm_reasoning",
                    "domain": ["x"],
                    "tags": ["y"],
                    "atomic": true
                },
                "constraints": {},
                "root": { "path": "root.md", "token_count": 50 },
                "references": { "tools": [], "skills": [], "knowledge_bases": [] }
            },
            "sub_documents": [],
            "extracted_tools": [tool]
        })
        .to_string()
    }

    fn root_only_files() -> HashMap<String, String> {
        let mut files = HashMap::new();
        files.insert("root.md".to_string(), "# Skill\n\nBody.".to_string());
        files
    }

    #[tokio::test]
    async fn test_create_copies_declared_source_path_artifact() {
        let store = tempfile::TempDir::new().unwrap();
        // A companion script that already exists outside any skill directory.
        let ext = tempfile::TempDir::new().unwrap();
        let script = ext.path().join("analyze_secrets.py");
        std::fs::write(&script, "print('hi')\n").unwrap();

        let response = created_response_with_tool(
            "ufa-secrets",
            serde_json::json!({
                "name": "analyze",
                "description": "Analyze the Secrets sheet",
                "type": "python",
                "source_path": script.display().to_string(),
                "dest_path": "tools/analyze_secrets.py"
            }),
        );
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &response,
            root_only_files(),
        ));

        let outcome = create_skill_from_request(delegator, store.path(), "req", None, None)
            .await
            .unwrap();
        assert_eq!(outcome.decision, "created");

        let copied = store
            .path()
            .join("ufa-secrets")
            .join("tools")
            .join("analyze_secrets.py");
        assert!(
            copied.is_file(),
            "existing artifact must be copied verbatim"
        );
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "print('hi')\n");
    }

    #[tokio::test]
    async fn test_create_copies_agent_written_output_dir_artifact() {
        let store = tempfile::TempDir::new().unwrap();
        // No source_path: the agent wrote the script into the output dir itself.
        let response = created_response_with_tool(
            "gen-skill",
            serde_json::json!({
                "name": "gen",
                "description": "Generated helper",
                "type": "python",
                "dest_path": "tools/gen.py"
            }),
        );
        let mut files = root_only_files();
        files.insert("tools/gen.py".to_string(), "print('gen')\n".to_string());
        let delegator = Arc::new(MockDelegator::with_response_and_files(&response, files));

        let outcome = create_skill_from_request(delegator, store.path(), "req", None, None)
            .await
            .unwrap();
        assert_eq!(outcome.decision, "created");

        let copied = store.path().join("gen-skill").join("tools").join("gen.py");
        assert!(
            copied.is_file(),
            "agent-written output-dir artifact must be copied"
        );
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "print('gen')\n");
    }

    #[tokio::test]
    async fn test_create_rejects_traversal_dest_path() {
        let store = tempfile::TempDir::new().unwrap();
        let ext = tempfile::TempDir::new().unwrap();
        let script = ext.path().join("evil.py");
        std::fs::write(&script, "x").unwrap();

        let response = created_response_with_tool(
            "trav",
            serde_json::json!({
                "name": "evil",
                "description": "tries to escape",
                "type": "python",
                "source_path": script.display().to_string(),
                "dest_path": "../evil.py"
            }),
        );
        let delegator = Arc::new(MockDelegator::with_response_and_files(
            &response,
            root_only_files(),
        ));

        let outcome = create_skill_from_request(delegator, store.path(), "req", None, None)
            .await
            .unwrap();
        // Skill still created; the unsafe artifact is silently skipped.
        assert_eq!(outcome.decision, "created");
        assert!(
            !store.path().join("evil.py").exists(),
            "traversal destination must not escape the skill directory"
        );
    }

    #[test]
    fn test_artifact_dest_rel() {
        let mk = |source: Option<&str>, dest: Option<&str>| AgentExtractedTool {
            name: "n".to_string(),
            description: "d".to_string(),
            tool_type: "python".to_string(),
            source_path: source.map(String::from),
            dest_path: dest.map(String::from),
        };

        assert_eq!(
            SkillCreationService::artifact_dest_rel(&mk(None, Some("tools/a.py"))),
            Some("tools/a.py".to_string())
        );
        assert_eq!(
            SkillCreationService::artifact_dest_rel(&mk(Some("/abs/x/foo.py"), None)),
            Some("tools/foo.py".to_string())
        );
        assert_eq!(
            SkillCreationService::artifact_dest_rel(&mk(None, Some("../evil"))),
            None
        );
        assert_eq!(
            SkillCreationService::artifact_dest_rel(&mk(None, Some("/etc/passwd"))),
            None
        );
        assert_eq!(
            SkillCreationService::artifact_dest_rel(&mk(None, None)),
            None
        );
    }
}
