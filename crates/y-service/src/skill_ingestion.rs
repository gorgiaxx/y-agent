//! Skill ingestion service — delegates third-party skill transformation to the
//! `skill-ingestion` agent and registers the result in the skill registry.
//!
//! Design reference: `skills-knowledge-design.md` §Skill Ingestion Flow,
//! §Skill Transformation Flow

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_core::skill::{
    SkillClassification, SkillClassificationType, SkillConstraints, SkillManifest, SkillRegistry,
    SkillState, SkillVersion, SubDocumentRef,
};
use y_core::types::SkillId;
use y_skills::{FormatDetector, IngestionFormat, SafetyScreener, SkillRegistryImpl};

// ---------------------------------------------------------------------------
// Import result
// ---------------------------------------------------------------------------

/// Result of a skill import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Whether the import was accepted, rejected, or partially accepted.
    pub decision: ImportDecision,
    /// Classification assigned by the agent.
    pub classification: String,
    /// Skill ID if registered.
    pub skill_id: Option<String>,
    /// Rejection reason if applicable.
    pub rejection_reason: Option<String>,
    /// Redirect suggestion if rejected.
    pub redirect_suggestion: Option<String>,
    /// Safety issues found.
    pub safety_issues: Vec<String>,
}

/// Import decision outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDecision {
    Accepted,
    Rejected,
    PartialAccept,
}

// ---------------------------------------------------------------------------
// Agent output schema
// ---------------------------------------------------------------------------

/// Structured output from the `skill-ingestion` agent.
#[derive(Debug, Deserialize)]
struct AgentIngestionOutput {
    decision: String,
    classification: String,
    #[allow(dead_code)]
    safety_verdict: String,
    #[serde(default)]
    safety_issues: Vec<String>,
    #[serde(default)]
    rejection_reason: Option<String>,
    #[serde(default)]
    redirect_suggestion: Option<String>,
    #[serde(default)]
    manifest: Option<AgentManifestOutput>,
    #[serde(default)]
    root_content: Option<String>,
    #[serde(default)]
    sub_documents: Vec<AgentSubDocOutput>,
    #[serde(default)]
    extracted_tools: Vec<AgentExtractedTool>,
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
    content: String,
    #[serde(default)]
    token_count: u32,
}

/// A tool that the agent extracted from a hybrid skill.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentExtractedTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub content: String,
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

    #[error("safety post-check failed: {issues:?}")]
    SafetyPostCheckFailed { issues: Vec<String> },

    #[error("skill registration failed: {0}")]
    RegistrationFailed(String),
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
/// 3. Delegate to `skill-ingestion` agent (LLM-assisted)
/// 4. Parse structured output
/// 5. Safety post-check (deterministic — `SafetyScreener`, defense in depth)
/// 6. Register in `SkillRegistry`
pub struct SkillIngestionService {
    delegator: Arc<dyn AgentDelegator>,
    registry: Arc<tokio::sync::RwLock<SkillRegistryImpl>>,
    safety_screener: SafetyScreener,
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
            safety_screener: SafetyScreener::new(),
        }
    }

    /// Import a skill from a file path.
    pub async fn import(&self, path: &Path) -> Result<ImportResult, ImportError> {
        // 1. Read source file
        if !path.exists() {
            return Err(ImportError::FileNotFound {
                path: path.display().to_string(),
            });
        }
        let source_content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ImportError::IoError(e.to_string()))?;

        // 2. Format detection (deterministic)
        let format = FormatDetector::from_path(path);
        let format_str = format_to_str(&format);

        // 3. Gather existing skills for dedup context
        let existing_skills: Vec<String> = self.registry.read().await.list_names().await;

        // 4. Delegate to skill-ingestion agent
        let input = serde_json::json!({
            "source_content": source_content,
            "source_format": format_str,
            "existing_skills": existing_skills,
            "existing_tools": [],
        });

        info!(
            path = %path.display(),
            format = format_str,
            "Delegating skill ingestion to agent"
        );

        let delegation_output = self
            .delegator
            .delegate("skill-ingestion", input, ContextStrategyHint::None)
            .await
            .map_err(|e| ImportError::DelegationFailed(e.to_string()))?;

        // 5. Parse agent output
        let agent_output: AgentIngestionOutput =
            serde_json::from_str(&delegation_output.text).map_err(|e| {
                ImportError::InvalidAgentOutput(format!(
                    "failed to parse agent response: {e}\nraw: {}",
                    &delegation_output.text[..delegation_output.text.len().min(500)]
                ))
            })?;

        // 6. Handle rejection
        let decision = match agent_output.decision.as_str() {
            "accepted" => ImportDecision::Accepted,
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
                safety_issues: agent_output.safety_issues,
            });
        }

        // 7. Safety post-check (defense in depth)
        if let Some(ref root_content) = agent_output.root_content {
            let verdict = self.safety_screener.screen(root_content);
            if !verdict.is_pass() {
                warn!("Safety post-check caught issues missed by agent");
                return Err(ImportError::SafetyPostCheckFailed {
                    issues: vec![format!("post-check verdict: {verdict:?}")],
                });
            }
        }

        // 8. Build SkillManifest from agent output
        let manifest_data = agent_output.manifest.ok_or_else(|| {
            ImportError::InvalidAgentOutput("accepted skill missing manifest".to_string())
        })?;

        let root_content = agent_output.root_content.ok_or_else(|| {
            ImportError::InvalidAgentOutput("accepted skill missing root_content".to_string())
        })?;

        let token_estimate = (root_content.len() / 4) as u32;
        let now = chrono::Utc::now();
        let skill_name = manifest_data.name.clone();

        let sub_doc_refs: Vec<SubDocumentRef> = agent_output
            .sub_documents
            .iter()
            .map(|sd| SubDocumentRef {
                id: sd.path.clone(),
                path: sd.path.clone(),
                title: sd.title.clone(),
                load_condition: "on_demand".to_string(),
                token_estimate: sd.token_count,
            })
            .collect();

        let tags = manifest_data
            .classification
            .as_ref()
            .map(|c| c.domain.clone())
            .unwrap_or_default();

        let manifest = SkillManifest {
            id: SkillId::from_string(&manifest_data.name),
            name: manifest_data.name,
            description: manifest_data.description,
            version: SkillVersion(manifest_data.version),
            tags,
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content,
            sub_documents: sub_doc_refs,
            token_estimate,
            created_at: now,
            updated_at: now,
            classification: manifest_data.classification.map(|c| {
                let skill_type = match c.skill_type.as_str() {
                    "llm_reasoning" => SkillClassificationType::LlmReasoning,
                    "api_call" => SkillClassificationType::ApiCall,
                    "tool_wrapper" => SkillClassificationType::ToolWrapper,
                    "agent_behavior" => SkillClassificationType::AgentBehavior,
                    "hybrid" => SkillClassificationType::Hybrid,
                    _ => SkillClassificationType::LlmReasoning,
                };
                SkillClassification {
                    skill_type,
                    domain: c.domain,
                    atomic: c.atomic,
                }
            }),
            constraints: manifest_data.constraints.map(|c| SkillConstraints {
                max_input_tokens: c.max_input_tokens,
                max_output_tokens: c.max_output_tokens,
                requires_language: c.requires_language,
            }),
            safety: None,
            references: None,
            author: Some("skill-ingestion-agent".to_string()),
            source_format: Some(format_str.to_string()),
            source_hash: None,
            state: Some(SkillState::Registered),
            root_path: Some("root.md".to_string()),
        };

        // 9. Register
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

        // 10. Store sub-document content
        for sub_doc in &agent_output.sub_documents {
            if let Err(e) = reg
                .store_sub_document(&skill_id_str, &sub_doc.path, &sub_doc.content)
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

        // 11. Log extracted tools (future: register in ToolRegistry)
        if !agent_output.extracted_tools.is_empty() {
            info!(
                count = agent_output.extracted_tools.len(),
                "Agent extracted tools (not yet auto-registered)"
            );
        }

        Ok(ImportResult {
            decision,
            classification: agent_output.classification,
            skill_id: Some(skill_id_str),
            rejection_reason: None,
            redirect_suggestion: None,
            safety_issues: agent_output.safety_issues,
        })
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

fn format_to_str(format: &IngestionFormat) -> &'static str {
    match format {
        IngestionFormat::Toml => "toml",
        IngestionFormat::Markdown => "markdown",
        IngestionFormat::Yaml => "yaml",
        IngestionFormat::Json => "json",
        IngestionFormat::PlainText => "plaintext",
        IngestionFormat::Directory => "directory",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use y_core::agent::{AgentDelegator, DelegationError, DelegationOutput};

    // Mock delegator that returns configurable JSON output
    #[derive(Debug)]
    struct MockDelegator {
        response: String,
    }

    impl MockDelegator {
        fn with_response(json: &str) -> Self {
            Self {
                response: json.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl AgentDelegator for MockDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
        ) -> Result<DelegationOutput, DelegationError> {
            Ok(DelegationOutput {
                text: self.response.clone(),
                tokens_used: 100,
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
            "safety_verdict": "safe",
            "safety_issues": [],
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
            "root_content": "# Humanizer\n\nRemove AI artifacts from text.\n\n## Rules\n\n1. Detect exaggerated language.\n2. Replace vague statements.",
            "sub_documents": [],
            "extracted_tools": []
        })
        .to_string()
    }

    fn rejected_response() -> String {
        serde_json::json!({
            "decision": "rejected",
            "classification": "api_call",
            "safety_verdict": "safe",
            "safety_issues": [],
            "rejection_reason": "This skill describes API interactions",
            "redirect_suggestion": "Register as a Tool via y-agent tool register"
        })
        .to_string()
    }

    /// T-SK-A2-01: Accepted skill is registered.
    #[tokio::test]
    async fn test_import_accepted_skill() {
        let delegator = Arc::new(MockDelegator::with_response(&accepted_response()));
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
        let delegator = Arc::new(MockDelegator::with_response(&accepted_response()));
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
}
