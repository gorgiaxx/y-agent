//! Shared helpers for building skill manifests and storing sub-documents.
//!
//! Both [`SkillIngestionService`](crate::skill_ingestion::SkillIngestionService)
//! and [`SkillCreationService`](crate::skill_creation::SkillCreationService)
//! transform agent metadata into a registered `SkillManifest` and then persist
//! sub-document content read from the agent's temp output directory. The
//! manifest-building and sub-document-storing logic is identical across the two
//! services except for five manifest fields (`tags`, `references`,
//! `knowledge_bases`, `author`, `source_format`); those differences are exposed
//! as parameters so a single implementation serves both call sites.

use std::path::Path;

use tracing::warn;

use y_core::skill::{
    SkillClassification, SkillClassificationType, SkillConstraints, SkillManifest, SkillReferences,
    SkillState, SkillVersion, SubDocumentRef,
};
use y_core::types::SkillId;
use y_skills::SkillRegistryImpl;

use crate::skill_ingestion::{
    AgentClassificationOutput, AgentConstraintsOutput, AgentSubDocOutput,
};

/// Inputs shared by both ingestion and creation when building a manifest.
///
/// The five fields that differ between the two services (`tags`, `references`,
/// `knowledge_bases`, `author`, `source_format`) are passed in directly; every
/// other manifest field is derived from the common agent-output data.
pub struct SkillManifestInput<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub description: &'a str,
    pub classification: Option<&'a AgentClassificationOutput>,
    pub constraints: Option<&'a AgentConstraintsOutput>,
    pub sub_documents: &'a [AgentSubDocOutput],
    pub root_content: &'a str,
    pub tags: Vec<String>,
    pub references: Option<SkillReferences>,
    pub knowledge_bases: Vec<String>,
    pub author: Option<String>,
    pub source_format: Option<String>,
}

/// Build a `SkillManifest` from the common agent-output metadata plus the five
/// service-specific fields.
///
/// This is the single implementation of the manifest-construction logic shared
/// by skill ingestion and skill creation. Behavioral differences between the two
/// callers are confined to the `tags`, `references`, `knowledge_bases`,
/// `author`, and `source_format` values they supply via [`SkillManifestInput`].
pub fn build_skill_manifest_from_agent_output(input: SkillManifestInput<'_>) -> SkillManifest {
    let token_estimate = u32::try_from(input.root_content.chars().count() / 4).unwrap_or(0);
    let now = chrono::Utc::now();

    let sub_doc_refs: Vec<SubDocumentRef> = input
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

    SkillManifest {
        id: SkillId::from_string(input.name),
        name: input.name.to_string(),
        description: input.description.to_string(),
        version: SkillVersion(input.version.to_string()),
        tags: input.tags,
        trigger_patterns: vec![],
        knowledge_bases: input.knowledge_bases,
        root_content: input.root_content.to_string(),
        sub_documents: sub_doc_refs,
        token_estimate,
        created_at: now,
        updated_at: now,
        classification: input.classification.map(|c| {
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
        constraints: input.constraints.map(|c| SkillConstraints {
            max_input_tokens: c.max_input_tokens,
            max_output_tokens: c.max_output_tokens,
            requires_language: c.requires_language.clone(),
        }),
        security: None,
        references: input.references,
        author: input.author,
        source_format: input.source_format,
        source_hash: None,
        state: Some(SkillState::Registered),
        root_path: Some("root.md".to_string()),
    }
}

/// Read each declared sub-document from the agent's output directory and store
/// it via the registry.
///
/// Failures (missing files or storage errors) are logged and never fatal: the
/// skill is already registered at this point and a missing sub-document must
/// not abort the operation. This is the single implementation of the loop
/// shared by skill ingestion and skill creation.
pub async fn store_sub_documents(
    registry: &SkillRegistryImpl,
    skill_id: &str,
    sub_docs: &[AgentSubDocOutput],
    output_dir: &Path,
) {
    for sub_doc in sub_docs {
        let sub_doc_path = output_dir.join(&sub_doc.path);
        match tokio::fs::read_to_string(&sub_doc_path).await {
            Ok(content) => {
                if let Err(e) = registry
                    .store_sub_document(skill_id, &sub_doc.path, &content)
                    .await
                {
                    warn!(
                        skill_id = %skill_id,
                        path = %sub_doc.path,
                        error = %e,
                        "Failed to store sub-document"
                    );
                }
            }
            Err(e) => {
                warn!(
                    skill_id = %skill_id,
                    path = %sub_doc.path,
                    error = %e,
                    "Agent declared sub-document but file not found in output dir"
                );
            }
        }
    }
}
