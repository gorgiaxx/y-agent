use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SESSION_PROMPT_CONFIG_PREFIX: &str = "__Y_AGENT_SESSION_PROMPT_CONFIG_V1__\n";
const PROMPT_TEMPLATES_FILE: &str = "prompt_templates.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionPromptConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub prompt_section_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPromptTemplate {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub prompt_section_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PromptTemplateError {
    #[error("template id must not be empty")]
    EmptyId,
    #[error("template id contains invalid path characters")]
    InvalidId,
    #[error("failed to read prompt templates: {0}")]
    Read(#[from] std::io::Error),
    #[error("failed to parse prompt templates: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize prompt templates: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PromptTemplateFile {
    #[serde(default)]
    templates: Vec<UserPromptTemplate>,
}

pub fn decode_session_prompt_config(stored: Option<String>) -> SessionPromptConfig {
    let Some(raw) = stored else {
        return SessionPromptConfig::default();
    };
    if raw.trim().is_empty() {
        return SessionPromptConfig::default();
    }

    if let Some(json) = raw.strip_prefix(SESSION_PROMPT_CONFIG_PREFIX) {
        return serde_json::from_str(json).unwrap_or_default();
    }

    SessionPromptConfig {
        system_prompt: Some(raw),
        prompt_section_ids: Vec::new(),
        template_id: None,
    }
}

pub fn encode_session_prompt_config(config: &SessionPromptConfig) -> Option<String> {
    let normalized = normalized_session_prompt_config(config);
    if !session_prompt_config_has_content(&normalized) {
        return None;
    }

    let json = serde_json::to_string(&normalized).ok()?;
    Some(format!("{SESSION_PROMPT_CONFIG_PREFIX}{json}"))
}

pub fn session_prompt_config_has_content(config: &SessionPromptConfig) -> bool {
    config
        .system_prompt
        .as_deref()
        .is_some_and(|prompt| !prompt.trim().is_empty())
        || !config.prompt_section_ids.is_empty()
        || config
            .template_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
}

pub fn load_user_prompt_templates(
    config_dir: &Path,
) -> Result<Vec<UserPromptTemplate>, PromptTemplateError> {
    let path = prompt_templates_path(config_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(path)?;
    let mut file: PromptTemplateFile = toml::from_str(&content)?;
    normalize_templates(&mut file.templates);
    Ok(file.templates)
}

pub fn save_user_prompt_template(
    config_dir: &Path,
    template: UserPromptTemplate,
) -> Result<(), PromptTemplateError> {
    validate_template_id(&template.id)?;
    let mut templates = load_user_prompt_templates(config_dir)?;
    let normalized = normalized_template(template);

    if let Some(existing) = templates.iter_mut().find(|item| item.id == normalized.id) {
        *existing = normalized;
    } else {
        templates.push(normalized);
    }

    normalize_templates(&mut templates);
    write_prompt_templates(config_dir, templates)
}

pub fn delete_user_prompt_template(config_dir: &Path, id: &str) -> Result<(), PromptTemplateError> {
    validate_template_id(id)?;
    let mut templates = load_user_prompt_templates(config_dir)?;
    templates.retain(|template| template.id != id);
    write_prompt_templates(config_dir, templates)
}

fn prompt_templates_path(config_dir: &Path) -> PathBuf {
    config_dir.join(PROMPT_TEMPLATES_FILE)
}

fn write_prompt_templates(
    config_dir: &Path,
    templates: Vec<UserPromptTemplate>,
) -> Result<(), PromptTemplateError> {
    std::fs::create_dir_all(config_dir)?;
    let content = toml::to_string_pretty(&PromptTemplateFile { templates })?;
    std::fs::write(prompt_templates_path(config_dir), content)?;
    Ok(())
}

fn validate_template_id(id: &str) -> Result<(), PromptTemplateError> {
    if id.trim().is_empty() {
        return Err(PromptTemplateError::EmptyId);
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(PromptTemplateError::InvalidId);
    }
    Ok(())
}

fn normalize_templates(templates: &mut [UserPromptTemplate]) {
    for template in templates.iter_mut() {
        *template = normalized_template(template.clone());
    }
    templates.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.id.cmp(&right.id))
    });
}

fn normalized_template(template: UserPromptTemplate) -> UserPromptTemplate {
    UserPromptTemplate {
        id: template.id.trim().to_string(),
        name: template.name.trim().to_string(),
        description: template
            .description
            .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string())),
        system_prompt: template.system_prompt.trim().to_string(),
        prompt_section_ids: unique_non_empty(template.prompt_section_ids),
    }
}

fn normalized_session_prompt_config(config: &SessionPromptConfig) -> SessionPromptConfig {
    SessionPromptConfig {
        system_prompt: config
            .system_prompt
            .as_deref()
            .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string())),
        prompt_section_ids: unique_non_empty(config.prompt_section_ids.clone()),
        template_id: config
            .template_id
            .as_deref()
            .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string())),
    }
}

fn unique_non_empty(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || out.iter().any(|existing| existing == trimmed) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_prompt_config_roundtrips_sections_and_template_id() {
        let config = SessionPromptConfig {
            system_prompt: Some("Prefer crisp answers.".to_string()),
            prompt_section_ids: vec![
                "core.datetime".to_string(),
                "core.tool_protocol".to_string(),
            ],
            template_id: Some("daily-driver".to_string()),
        };

        let encoded = encode_session_prompt_config(&config).expect("non-empty config encodes");
        assert_ne!(encoded, "Prefer crisp answers.");

        let decoded = decode_session_prompt_config(Some(encoded));
        assert_eq!(decoded, config);
    }

    #[test]
    fn test_session_prompt_config_decodes_legacy_plain_prompt() {
        let decoded = decode_session_prompt_config(Some("Legacy prompt".to_string()));

        assert_eq!(decoded.system_prompt.as_deref(), Some("Legacy prompt"));
        assert!(decoded.prompt_section_ids.is_empty());
        assert_eq!(decoded.template_id, None);
    }

    #[test]
    fn test_empty_session_prompt_config_clears_storage() {
        let config = SessionPromptConfig {
            system_prompt: Some("   ".to_string()),
            prompt_section_ids: Vec::new(),
            template_id: None,
        };

        assert!(encode_session_prompt_config(&config).is_none());
        assert!(!session_prompt_config_has_content(&config));
    }

    #[test]
    fn test_user_prompt_templates_roundtrip_and_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let template = UserPromptTemplate {
            id: "daily-driver".to_string(),
            name: "Daily Driver".to_string(),
            description: Some("Default session setup".to_string()),
            system_prompt: "Stay direct.".to_string(),
            prompt_section_ids: vec!["core.datetime".to_string()],
        };

        save_user_prompt_template(dir.path(), template.clone()).expect("save template");
        assert_eq!(
            load_user_prompt_templates(dir.path()).expect("load templates"),
            vec![template]
        );

        delete_user_prompt_template(dir.path(), "daily-driver").expect("delete template");
        assert!(load_user_prompt_templates(dir.path())
            .expect("reload templates")
            .is_empty());
    }
}
