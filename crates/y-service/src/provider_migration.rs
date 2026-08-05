//! Provider migration — quick-import of LLM provider configs from external
//! agent CLIs (`omp`, `kimi`, `claude`, `codex`, `omo`) into y-agent's
//! `providers.toml`.
//!
//! Each external tool stores its provider credentials (API key, base URL,
//! model) in a well-known location and format. This module detects those
//! configs, extracts provider candidates, and appends them as
//! `[[providers]]` blocks to the user's `providers.toml` without disturbing
//! existing entries, comments, or top-level pool settings.
//!
//! Migration state (which sources have already been migrated) is persisted in
//! `migration_state.toml` alongside the other config files, so the UI can show
//! a locked, check-marked logo per completed source.
//!
//! Design notes:
//! - The full API key never crosses the IPC boundary during detection. The
//!   serializable [`MigrationProviderCandidate`] carries only a masked preview
//!   and a `has_api_key` flag. [`migrate_source`] re-reads the source config
//!   server-side to obtain the real key.
//! - Migration appends `[[providers]]` blocks to the end of `providers.toml`
//!   and validates the merged result before writing. This preserves all
//!   existing content (comments, top-level pool fields, the `[retry]` table,
//!   and previously-migrated providers).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ConfigService;

// ---------------------------------------------------------------------------
// Public types (serialized to the frontend)
// ---------------------------------------------------------------------------

/// One migratable provider extracted from an external agent's config.
///
/// The full API key is intentionally absent — only a masked preview and a
/// `has_api_key` flag travel to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationProviderCandidate {
    pub id: String,
    pub label: String,
    pub provider_type: String,
    pub model: String,
    pub base_url: Option<String>,
    pub has_api_key: bool,
    pub api_key_preview: String,
    pub context_window: Option<usize>,
    pub source_provider_name: String,
}

/// Detection result for one external agent source.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationSourceInfo {
    pub id: String,
    pub label: String,
    pub icon_id: Option<String>,
    pub detected: bool,
    pub migrated: bool,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub providers: Vec<MigrationProviderCandidate>,
}

/// Result of a migration run for one source.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub source_id: String,
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Internal extraction types
// ---------------------------------------------------------------------------

/// A provider extracted from a source config, including the real API key.
/// Never serialized across IPC; used only within this module.
#[derive(Debug, Clone)]
struct ExtractedProvider {
    source_provider_name: String,
    provider_type: String,
    model: String,
    base_url: Option<String>,
    api_key: Option<String>,
    context_window: Option<usize>,
}

impl ExtractedProvider {
    fn candidate_id(&self, source_id: &str) -> String {
        format!("{source_id}-{}", slug(&self.source_provider_name))
    }

    fn label(&self) -> String {
        if self.model.is_empty() {
            self.source_provider_name.clone()
        } else {
            format!("{} / {}", self.source_provider_name, self.model)
        }
    }

    fn to_candidate(&self, source_id: &str) -> MigrationProviderCandidate {
        let has_key = self.api_key.as_deref().filter(|k| !k.is_empty()).is_some();
        MigrationProviderCandidate {
            id: self.candidate_id(source_id),
            label: self.label(),
            provider_type: self.provider_type.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            has_api_key: has_key,
            api_key_preview: self.api_key.as_deref().map(mask_key).unwrap_or_default(),
            context_window: self.context_window,
            source_provider_name: self.source_provider_name.clone(),
        }
    }
}

/// Result of parsing one source: metadata + extracted providers.
struct SourceDetection {
    info: MigrationSourceInfo,
    providers: Vec<ExtractedProvider>,
}

// ---------------------------------------------------------------------------
// Source metadata
// ---------------------------------------------------------------------------

struct SourceMeta {
    id: &'static str,
    label: &'static str,
    icon_id: Option<&'static str>,
}

const SOURCES: &[SourceMeta] = &[
    SourceMeta {
        id: "omp",
        label: "Oh My Pi",
        icon_id: None,
    },
    SourceMeta {
        id: "kimi",
        label: "Kimi Code",
        icon_id: Some("Moonshot"),
    },
    SourceMeta {
        id: "claude",
        label: "Claude Code",
        icon_id: Some("Anthropic"),
    },
    SourceMeta {
        id: "codex",
        label: "Codex",
        icon_id: Some("OpenAI"),
    },
    SourceMeta {
        id: "omo",
        label: "oh-my-openagent",
        icon_id: None,
    },
];

fn source_meta(id: &str) -> Option<&'static SourceMeta> {
    SOURCES.iter().find(|m| m.id == id)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect all migration sources, reporting whether each is present,
/// supported, and already migrated, along with the extractable provider
/// candidates.
pub fn detect_sources(home_dir: &Path, config_dir: &Path) -> Vec<MigrationSourceInfo> {
    let migrated = migrated_sources(config_dir);
    SOURCES
        .iter()
        .map(|m| {
            let det = detect_source(m.id, home_dir);
            let mut info = det.info;
            info.migrated = migrated.contains(m.id);
            // Hide extracted providers from already-migrated sources so the
            // UI does not offer them again.
            if info.migrated {
                info.providers.clear();
            }
            info
        })
        .collect()
}

/// Migrate selected providers from one source into `providers.toml`.
///
/// Re-reads the source config (so the real API key is obtained server-side),
/// appends new `[[providers]]` blocks for the selected candidates, validates
/// the merged file, and tags each block with `source = <source_id>` so the
/// source stays migrated until all its imported providers are deleted.
/// Candidates whose id already exists in `providers.toml` are skipped.
pub fn migrate_source(
    home_dir: &Path,
    config_dir: &Path,
    source_id: &str,
    selected_ids: &[String],
) -> Result<MigrationReport, String> {
    let meta =
        source_meta(source_id).ok_or_else(|| format!("Unknown migration source: {source_id}"))?;
    let det = detect_source(source_id, home_dir);
    if !det.info.supported {
        return Err(det
            .info
            .unsupported_reason
            .unwrap_or_else(|| format!("{} is not supported for migration", meta.label)));
    }

    let by_id: BTreeMap<String, &ExtractedProvider> = det
        .providers
        .iter()
        .map(|p| (p.candidate_id(source_id), p))
        .collect();

    let existing_text = ConfigService::read_section(config_dir, "providers").unwrap_or_default();
    let existing_ids = existing_provider_ids(&existing_text);

    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();
    let mut blocks = String::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for id in selected_ids {
        if seen.contains(id) {
            continue;
        }
        seen.insert(id.clone());
        let Some(p) = by_id.get(id) else {
            errors.push(format!("Unknown provider id: {id}"));
            continue;
        };
        if existing_ids.iter().any(|e| e == id) {
            skipped.push(id.clone());
            continue;
        }
        let block = build_provider_block(p, source_id, meta.icon_id)?;
        blocks.push_str(&block);
        imported.push(id.clone());
    }

    if !blocks.is_empty() {
        let mut text = existing_text;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&blocks);
        // Validate the merged document before persisting so a malformed merge
        // never corrupts the user's providers.toml.
        let _: toml::Value =
            toml::from_str(&text).map_err(|e| format!("merged providers.toml is invalid: {e}"))?;
        ConfigService::save_section(config_dir, "providers", &text)?;
    }

    Ok(MigrationReport {
        source_id: source_id.to_string(),
        imported,
        skipped,
        errors,
    })
}

// ---------------------------------------------------------------------------
// Source dispatch
// ---------------------------------------------------------------------------

fn detect_source(id: &str, home: &Path) -> SourceDetection {
    match id {
        "omp" => detect_omp(home),
        "kimi" => detect_kimi(home),
        "claude" => detect_claude(home),
        "codex" => detect_codex(home),
        "omo" => detect_omo(home),
        other => SourceDetection {
            info: MigrationSourceInfo {
                id: other.to_string(),
                label: other.to_string(),
                icon_id: None,
                detected: false,
                migrated: false,
                supported: false,
                unsupported_reason: Some("unknown migration source".into()),
                providers: vec![],
            },
            providers: vec![],
        },
    }
}

// ---------------------------------------------------------------------------
// omp — Oh My Pi (`~/.omp/agent/models.yml`, YAML)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OmpModels {
    providers: Option<BTreeMap<String, OmpProvider>>,
}

#[derive(Deserialize)]
struct OmpProvider {
    #[serde(rename = "baseUrl")]
    base_url: Option<String>,
    api: Option<String>,
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    models: Option<Vec<OmpModel>>,
}

#[derive(Deserialize)]
struct OmpModel {
    id: String,
    #[serde(rename = "contextWindow")]
    context_window: Option<usize>,
}

fn detect_omp(home: &Path) -> SourceDetection {
    let label = "Oh My Pi";
    let path = home.join(".omp/agent/models.yml");
    let Some(content) = read_optional(&path) else {
        return not_detected("omp", label, None);
    };
    let models: OmpModels = match serde_yaml::from_str(&content) {
        Ok(m) => m,
        Err(_) => return detected_unsupported("omp", label, None, "无法解析 omp models.yml"),
    };
    let mut providers = Vec::new();
    if let Some(pmap) = models.providers {
        for (name, p) in pmap {
            let model = p
                .models
                .as_ref()
                .and_then(|v| v.first())
                .map(|m| m.id.clone())
                .unwrap_or_default();
            let ctx = p
                .models
                .as_ref()
                .and_then(|v| v.first())
                .and_then(|m| m.context_window);
            providers.push(ExtractedProvider {
                source_provider_name: name,
                provider_type: omp_map_api(p.api.as_deref()),
                model,
                base_url: p.base_url,
                api_key: p.api_key,
                context_window: ctx,
            });
        }
    }
    make_detection("omp", label, None, providers)
}

fn omp_map_api(api: Option<&str>) -> String {
    match api {
        Some("openai-responses") => "openai",
        Some("anthropic-messages") => "anthropic",
        // openai-completions and any unknown value map to the broadest compat
        // backend so the imported provider is immediately usable.
        _ => "openai-compat",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// kimi — Kimi Code (`~/.kimi-code/config.toml`, TOML)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct KimiConfig {
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    providers: BTreeMap<String, KimiProvider>,
    #[serde(default)]
    models: BTreeMap<String, KimiModel>,
}

#[derive(Deserialize)]
struct KimiProvider {
    #[serde(rename = "type")]
    ptype: Option<String>,
    api_key: Option<String>,
    base_url: Option<String>,
}

#[derive(Deserialize)]
struct KimiModel {
    provider: Option<String>,
    model: Option<String>,
    #[serde(rename = "max_context_size")]
    max_context_size: Option<usize>,
}

fn detect_kimi(home: &Path) -> SourceDetection {
    let label = "Kimi Code";
    let path = home.join(".kimi-code/config.toml");
    let Some(content) = read_optional(&path) else {
        return not_detected("kimi", label, Some("Moonshot"));
    };
    let cfg: KimiConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(_) => {
            return detected_unsupported(
                "kimi",
                label,
                Some("Moonshot"),
                "无法解析 kimi config.toml",
            )
        }
    };
    let mut providers = Vec::new();
    for (name, p) in &cfg.providers {
        // Prefer the model that the source designates as default; fall back to
        // the first model entry that references this provider.
        let model_entry = cfg
            .models
            .iter()
            .find(|(k, m)| {
                m.provider.as_deref() == Some(name.as_str())
                    && Some(k.as_str()) == cfg.default_model.as_deref()
            })
            .or_else(|| {
                cfg.models
                    .iter()
                    .find(|(_, m)| m.provider.as_deref() == Some(name.as_str()))
            });
        let model = model_entry
            .and_then(|(_, m)| m.model.clone())
            .unwrap_or_default();
        let ctx = model_entry.and_then(|(_, m)| m.max_context_size);
        providers.push(ExtractedProvider {
            source_provider_name: name.clone(),
            provider_type: kimi_map_type(p.ptype.as_deref()),
            model,
            base_url: p.base_url.clone(),
            api_key: p.api_key.clone(),
            context_window: ctx,
        });
    }
    make_detection("kimi", label, Some("Moonshot"), providers)
}

fn kimi_map_type(ptype: Option<&str>) -> String {
    match ptype {
        Some("anthropic") => "anthropic",
        // kimi and unknown values use the OpenAI-compatible wire shape.
        _ => "openai-compat",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// claude — Claude Code (`~/.claude/settings.json`, JSON)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ClaudeSettings {
    env: Option<BTreeMap<String, String>>,
}

fn detect_claude(home: &Path) -> SourceDetection {
    let label = "Claude Code";
    let path = home.join(".claude/settings.json");
    let Some(content) = read_optional(&path) else {
        return not_detected("claude", label, Some("Anthropic"));
    };
    let s: ClaudeSettings = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(_) => {
            return detected_unsupported(
                "claude",
                label,
                Some("Anthropic"),
                "无法解析 claude settings.json",
            )
        }
    };
    let env = s.env.unwrap_or_default();
    let key = env
        .get("ANTHROPIC_API_KEY")
        .filter(|v| !v.is_empty())
        .cloned()
        .or_else(|| {
            env.get("ANTHROPIC_AUTH_TOKEN")
                .filter(|v| !v.is_empty())
                .cloned()
        });
    let base_url = env.get("ANTHROPIC_BASE_URL").cloned();
    let model = env.get("ANTHROPIC_MODEL").map(|m| strip_model_suffix(m));
    let provider = ExtractedProvider {
        source_provider_name: "claude".into(),
        provider_type: "anthropic".into(),
        model: model.unwrap_or_default(),
        base_url,
        api_key: key,
        context_window: Some(200_000),
    };
    make_detection("claude", label, Some("Anthropic"), vec![provider])
}

/// Strip a trailing `[...]` qualifier (e.g. `"mco-6[1m]"` → `"mco-6"`) that
/// Claude Code uses to request a 1M-token context tier.
fn strip_model_suffix(model: &str) -> String {
    model.split('[').next().unwrap_or(model).to_string()
}

// ---------------------------------------------------------------------------
// codex — OpenAI Codex CLI (`~/.codex/auth.json` + `~/.codex/config.toml`)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CodexAuth {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    auth_mode: Option<String>,
}

#[derive(Deserialize)]
struct CodexConfig {
    model: Option<String>,
    #[serde(default)]
    model_providers: BTreeMap<String, CodexModelProvider>,
}

#[derive(Deserialize)]
struct CodexModelProvider {
    base_url: Option<String>,
}

fn detect_codex(home: &Path) -> SourceDetection {
    let label = "Codex";
    let auth_path = home.join(".codex/auth.json");
    let Some(content) = read_optional(&auth_path) else {
        return not_detected("codex", label, Some("OpenAI"));
    };
    let auth: CodexAuth = match serde_json::from_str(&content) {
        Ok(a) => a,
        Err(_) => {
            return detected_unsupported("codex", label, Some("OpenAI"), "无法解析 codex auth.json")
        }
    };
    let mode = auth.auth_mode.as_deref().unwrap_or("");
    let has_static_key = auth
        .openai_api_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .is_some();
    if mode == "chatgpt" || (!has_static_key && mode != "apikey") {
        // ChatGPT OAuth has no extractable static key — not supported yet.
        return detected_unsupported(
            "codex",
            label,
            Some("OpenAI"),
            "OAuth 模式暂不支持，请在 codex 中切换为 API key 模式",
        );
    }

    let model = std::fs::read_to_string(home.join(".codex/config.toml"))
        .ok()
        .and_then(|c| toml::from_str::<CodexConfig>(&c).ok())
        .and_then(|cfg| cfg.model)
        .unwrap_or_default();
    let base_url = std::fs::read_to_string(home.join(".codex/config.toml"))
        .ok()
        .and_then(|c| toml::from_str::<CodexConfig>(&c).ok())
        .and_then(|cfg| cfg.model_providers.into_values().find_map(|mp| mp.base_url));
    let provider_type = if base_url.is_some() {
        "openai-compat"
    } else {
        "openai"
    };
    let provider = ExtractedProvider {
        source_provider_name: "codex".into(),
        provider_type: provider_type.into(),
        model,
        base_url,
        api_key: auth.openai_api_key,
        context_window: Some(200_000),
    };
    make_detection("codex", label, Some("OpenAI"), vec![provider])
}

// ---------------------------------------------------------------------------
// omo — oh-my-openagent / opencode (`~/.config/opencode/opencode.json`)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OmoConfig {
    provider: Option<BTreeMap<String, OmoProvider>>,
}

#[derive(Deserialize)]
struct OmoProvider {
    npm: Option<String>,
    options: Option<OmoOptions>,
    models: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Deserialize, Default)]
struct OmoOptions {
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    #[serde(rename = "baseURL")]
    base_url: Option<String>,
}

fn detect_omo(home: &Path) -> SourceDetection {
    let label = "oh-my-openagent";
    let path = home.join(".config/opencode/opencode.json");
    let Some(content) = read_optional(&path) else {
        return not_detected("omo", label, None);
    };
    let cfg: OmoConfig = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(_) => return detected_unsupported("omo", label, None, "无法解析 opencode.json"),
    };
    let mut providers = Vec::new();
    if let Some(pmap) = cfg.provider {
        for (name, p) in pmap {
            let model = p
                .models
                .as_ref()
                .and_then(|m| m.keys().next().cloned())
                .unwrap_or_default();
            let opts = p.options.unwrap_or_default();
            providers.push(ExtractedProvider {
                source_provider_name: name,
                provider_type: omo_map_npm(p.npm.as_deref()),
                model,
                base_url: opts.base_url,
                api_key: opts.api_key,
                context_window: Some(1_000_000),
            });
        }
    }
    make_detection("omo", label, None, providers)
}

fn omo_map_npm(npm: Option<&str>) -> String {
    match npm {
        Some(s) if s.ends_with("anthropic") => "anthropic",
        Some(s) if s.ends_with("google") => "gemini",
        // @ai-sdk/openai, @ai-sdk/groq, and others all speak the OpenAI
        // chat-compatible shape.
        _ => "openai-compat",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Provider block serialization
// ---------------------------------------------------------------------------

/// Minimal serializable shape for a `[[providers]]` block. Only the fields
/// relevant to a migrated provider are emitted; everything else uses
/// `ProviderConfig`'s serde defaults when y-agent loads the file.
#[derive(Serialize)]
struct ProviderBlock<'a> {
    id: &'a str,
    provider_type: &'a str,
    model: &'a str,
    tags: Vec<&'a str>,
    enabled: bool,
    max_concurrency: usize,
    context_window: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<&'a str>,
}

fn build_provider_block(
    p: &ExtractedProvider,
    source_id: &str,
    icon_id: Option<&str>,
) -> Result<String, String> {
    let id = p.candidate_id(source_id);
    let body = toml::to_string(&ProviderBlock {
        id: &id,
        provider_type: &p.provider_type,
        model: &p.model,
        tags: vec!["general"],
        enabled: true,
        max_concurrency: 10,
        context_window: p.context_window.unwrap_or(200_000),
        api_key: p.api_key.as_deref().filter(|k| !k.is_empty()),
        base_url: p.base_url.as_deref().filter(|u| !u.is_empty()),
        icon: icon_id,
    })
    .map_err(|e| format!("serialize provider block: {e}"))?;
    Ok(format!("[[providers]]\n{body}\n"))
}

// ---------------------------------------------------------------------------
// Migration state -- derived from providers.toml, not a sidecar file.
//
// A source is "migrated" while at least one provider id in providers.toml
// starts with `<source_id>-` (the id format migration always produces).
// Deleting every imported provider re-enables the source's logo, matching
// user expectations. No separate state file is used, so providers.toml is
// the single source of truth.
// ---------------------------------------------------------------------------

/// Collect the set of source ids that currently have at least one provider
/// whose id starts with `<source_id>-` in `providers.toml`.
fn migrated_sources(config_dir: &Path) -> BTreeSet<String> {
    let text = ConfigService::read_section(config_dir, "providers").unwrap_or_default();
    let mut set = BTreeSet::new();
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return set;
    };
    let Some(arr) = value.get("providers").and_then(|p| p.as_array()) else {
        return set;
    };
    for entry in arr {
        let Some(id) = entry.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        for meta in SOURCES {
            let prefix = format!("{}-", meta.id);
            if id.starts_with(&prefix) {
                set.insert(meta.id.to_string());
                break;
            }
        }
    }
    set
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a file to string, returning `None` if it does not exist (so callers
/// can treat absence as "not detected" without a separate stat).
fn read_optional(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Build a supported [`SourceDetection`] from extracted providers.
fn make_detection(
    id: &str,
    label: &str,
    icon_id: Option<&str>,
    providers: Vec<ExtractedProvider>,
) -> SourceDetection {
    let info = MigrationSourceInfo {
        id: id.into(),
        label: label.into(),
        icon_id: icon_id.map(str::to_string),
        detected: true,
        migrated: false,
        supported: true,
        unsupported_reason: None,
        providers: providers.iter().map(|p| p.to_candidate(id)).collect(),
    };
    SourceDetection { info, providers }
}

fn not_detected(id: &str, label: &str, icon_id: Option<&str>) -> SourceDetection {
    SourceDetection {
        info: MigrationSourceInfo {
            id: id.into(),
            label: label.into(),
            icon_id: icon_id.map(str::to_string),
            detected: false,
            migrated: false,
            supported: true,
            unsupported_reason: None,
            providers: vec![],
        },
        providers: vec![],
    }
}

fn detected_unsupported(
    id: &str,
    label: &str,
    icon_id: Option<&str>,
    reason: &str,
) -> SourceDetection {
    SourceDetection {
        info: MigrationSourceInfo {
            id: id.into(),
            label: label.into(),
            icon_id: icon_id.map(str::to_string),
            detected: true,
            migrated: false,
            supported: false,
            unsupported_reason: Some(reason.into()),
            providers: vec![],
        },
        providers: vec![],
    }
}

/// Collect existing provider ids from a `providers.toml` document so migration
/// can skip duplicates.
fn existing_provider_ids(text: &str) -> Vec<String> {
    toml::from_str::<toml::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("providers").and_then(|p| p.as_array()).map(|a| {
                a.iter()
                    .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// URL/file-id slug: lowercase alphanumerics, non-alphanumeric → `-`,
/// collapsed and trimmed.
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Mask an API key for display: first/last 4 chars with an ellipsis.
fn mask_key(k: &str) -> String {
    let len = k.chars().count();
    if len <= 8 {
        return "••••".into();
    }
    let prefix: String = k.chars().take(4).collect();
    let suffix: String = k
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ---- omp ---------------------------------------------------------------

    #[test]
    fn test_detect_omp_extracts_providers() {
        let home = tempdir().unwrap();
        let dir = home.path().join(".omp/agent");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("models.yml"),
            r"
providers:
  DeepSeek-OpenAI:
    baseUrl: https://api.example.com/v1
    api: openai-completions
    apiKey: sk-1234567890abcdef
    models:
      - id: deepseek-v4-pro
        contextWindow: 1000000
      - id: deepseek-v4-flash
        contextWindow: 1000000
  Anthropic-Relay:
    baseUrl: https://relay.example.com/v1
    api: anthropic-messages
    apiKey: sk-relaykey123456
    models:
      - id: glm-5.2
",
        )
        .unwrap();

        let det = detect_source("omp", home.path());
        assert!(det.info.detected);
        assert!(det.info.supported);
        assert_eq!(det.providers.len(), 2);
        let first = det
            .providers
            .iter()
            .find(|p| p.source_provider_name == "DeepSeek-OpenAI")
            .unwrap();
        assert_eq!(first.provider_type, "openai-compat");
        assert_eq!(first.model, "deepseek-v4-pro");
        assert_eq!(
            first.base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(first.api_key.as_deref(), Some("sk-1234567890abcdef"));
        let relay = det
            .providers
            .iter()
            .find(|p| p.source_provider_name == "Anthropic-Relay")
            .unwrap();
        assert_eq!(relay.provider_type, "anthropic");
        assert_eq!(relay.model, "glm-5.2");

        let candidate = first.to_candidate("omp");
        assert_eq!(candidate.id, "omp-deepseek-openai");
        assert!(candidate.has_api_key);
        assert!(candidate.api_key_preview.starts_with("sk-1"));
    }

    #[test]
    fn test_detect_omp_missing_is_not_detected() {
        let home = tempdir().unwrap();
        let det = detect_source("omp", home.path());
        assert!(!det.info.detected);
        assert!(det.info.supported);
        assert!(det.providers.is_empty());
    }

    // ---- kimi --------------------------------------------------------------

    #[test]
    fn test_detect_kimi_extracts_providers() {
        let home = tempdir().unwrap();
        let dir = home.path().join(".kimi-code");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.toml"),
            r#"
default_model = "ModelScope/glm-5.2"

[providers.ModelScope]
type = "anthropic"
api_key = "sk-4054c819ae2e400ea5b4ceafacbf7cb7"
base_url = "http://relay.example.com/anthropic"

[providers."managed:kimi-code"]
type = "kimi"
api_key = "sk-kimi-abcdef123456"
base_url = "https://api.kimi.com/coding/v1"

[models."ModelScope/glm-5.2"]
provider = "ModelScope"
model = "glm-5.2"
max_context_size = 1048576

[models."managed:kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
"#,
        )
        .unwrap();

        let det = detect_source("kimi", home.path());
        assert!(det.info.detected);
        assert_eq!(det.providers.len(), 2);
        let ms = det
            .providers
            .iter()
            .find(|p| p.source_provider_name == "ModelScope")
            .unwrap();
        // Default model wins over k3.
        assert_eq!(ms.model, "glm-5.2");
        assert_eq!(ms.provider_type, "anthropic");
        assert_eq!(ms.context_window, Some(1_048_576));
        let kimi = det
            .providers
            .iter()
            .find(|p| p.source_provider_name == "managed:kimi-code")
            .unwrap();
        assert_eq!(kimi.provider_type, "openai-compat");
        assert_eq!(kimi.model, "k3");
    }

    // ---- claude ------------------------------------------------------------

    #[test]
    fn test_detect_claude_extracts_env_provider() {
        let home = tempdir().unwrap();
        let dir = home.path().join(".claude");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("settings.json"),
            r#"{
  "env": {
    "ANTHROPIC_AUTH_TOKEN": "26a58994-7644-4aeb-8cb0-27c421829a67",
    "ANTHROPIC_BASE_URL": "https://kspmas.ksyun.com",
    "ANTHROPIC_MODEL": "mco-6[1m]"
  }
}"#,
        )
        .unwrap();

        let det = detect_source("claude", home.path());
        assert!(det.info.detected);
        assert_eq!(det.providers.len(), 1);
        let p = &det.providers[0];
        assert_eq!(p.provider_type, "anthropic");
        assert_eq!(p.model, "mco-6");
        assert_eq!(p.base_url.as_deref(), Some("https://kspmas.ksyun.com"));
        assert_eq!(
            p.api_key.as_deref(),
            Some("26a58994-7644-4aeb-8cb0-27c421829a67")
        );
    }

    #[test]
    fn test_strip_model_suffix_handles_brackets() {
        assert_eq!(strip_model_suffix("mco-6[1m]"), "mco-6");
        assert_eq!(strip_model_suffix("claude-sonnet-4-6"), "claude-sonnet-4-6");
    }

    // ---- codex -------------------------------------------------------------

    #[test]
    fn test_detect_codex_oauth_is_unsupported() {
        let home = tempdir().unwrap();
        let dir = home.path().join(".codex");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("auth.json"),
            r#"{"OPENAI_API_KEY": null, "auth_mode": "chatgpt"}"#,
        )
        .unwrap();
        fs::write(dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").unwrap();

        let det = detect_source("codex", home.path());
        assert!(det.info.detected);
        assert!(!det.info.supported);
        assert!(det
            .info
            .unsupported_reason
            .as_deref()
            .unwrap()
            .contains("OAuth"));
        assert!(det.providers.is_empty());
    }

    #[test]
    fn test_detect_codex_apikey_is_supported() {
        let home = tempdir().unwrap();
        let dir = home.path().join(".codex");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("auth.json"),
            r#"{"OPENAI_API_KEY": "sk-codexkey123456", "auth_mode": "apikey"}"#,
        )
        .unwrap();
        fs::write(dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").unwrap();

        let det = detect_source("codex", home.path());
        assert!(det.info.detected);
        assert!(det.info.supported);
        assert_eq!(det.providers.len(), 1);
        let p = &det.providers[0];
        assert_eq!(p.provider_type, "openai");
        assert_eq!(p.model, "gpt-5.6-sol");
        assert!(p.base_url.is_none());
        assert_eq!(p.api_key.as_deref(), Some("sk-codexkey123456"));
    }

    // ---- omo ---------------------------------------------------------------

    #[test]
    fn test_detect_omo_extracts_providers() {
        let home = tempdir().unwrap();
        let dir = home.path().join(".config/opencode");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("opencode.json"),
            r#"{
  "provider": {
    "hmind-glm5": {
      "npm": "@ai-sdk/anthropic",
      "options": {
        "apiKey": "sk-4054c819ae2e400ea5b4ceafacbf7cb7",
        "baseURL": "http://relay.example.com/anthropic"
      },
      "models": {
        "glm-5.2": { "name": "glm-5.2" }
      }
    },
    "hmind-openai": {
      "npm": "@ai-sdk/openai",
      "options": {
        "apiKey": "sk-openaikey123",
        "baseURL": "https://api.openai.com/v1"
      },
      "models": {
        "gpt-5.6": { "name": "gpt-5.6" }
      }
    }
  }
}"#,
        )
        .unwrap();

        let det = detect_source("omo", home.path());
        assert!(det.info.detected);
        assert_eq!(det.providers.len(), 2);
        let glm = det
            .providers
            .iter()
            .find(|p| p.source_provider_name == "hmind-glm5")
            .unwrap();
        assert_eq!(glm.provider_type, "anthropic");
        assert_eq!(glm.model, "glm-5.2");
        let oai = det
            .providers
            .iter()
            .find(|p| p.source_provider_name == "hmind-openai")
            .unwrap();
        assert_eq!(oai.provider_type, "openai-compat");
    }

    // ---- migration ---------------------------------------------------------

    #[test]
    fn test_migrate_source_appends_and_marks() {
        let home = tempdir().unwrap();
        let config_dir = tempdir().unwrap();
        // Seed an existing providers.toml with comments and one provider.
        fs::write(
            config_dir.path().join("providers.toml"),
            r#"# header comment
default_freeze_duration_secs = 60

[retry]
enabled = true

[[providers]]
id = "existing"
provider_type = "openai"
model = "gpt-4o"
tags = ["general"]
"#,
        )
        .unwrap();
        // omp source config.
        let omp_dir = home.path().join(".omp/agent");
        fs::create_dir_all(&omp_dir).unwrap();
        fs::write(
            omp_dir.join("models.yml"),
            "providers:\n  DeepSeek-OpenAI:\n    baseUrl: https://api.example.com/v1\n    api: openai-completions\n    apiKey: sk-1234567890abcdef\n    models:\n      - id: deepseek-v4-pro\n        contextWindow: 1000000\n",
        )
        .unwrap();

        let id = "omp-deepseek-openai";
        let report = migrate_source(home.path(), config_dir.path(), "omp", &[id.into()])
            .expect("migration should succeed");
        assert_eq!(report.imported, vec![id.to_string()]);
        assert!(report.skipped.is_empty());

        let written = fs::read_to_string(config_dir.path().join("providers.toml")).unwrap();
        assert!(
            written.contains("# header comment"),
            "comments must be preserved"
        );
        assert!(
            written.contains("id = \"existing\""),
            "existing provider preserved"
        );
        assert!(written.contains("[[providers]]\nid = \"omp-deepseek-openai\""));
        assert!(written.contains("api_key = \"sk-1234567890abcdef\""));
        assert!(written.contains("tags = [\"general\"]"));
        // The imported provider id carries the source prefix; deleting it re-enables.
        assert!(migrated_sources(config_dir.path()).contains("omp"));

        // Re-detect: source is now migrated, candidates hidden.
        let infos = detect_sources(home.path(), config_dir.path());
        let omp_info = infos.iter().find(|i| i.id == "omp").unwrap();
        assert!(omp_info.migrated);
        assert!(
            omp_info.providers.is_empty(),
            "migrated sources hide candidates"
        );
    }

    #[test]
    fn test_migrate_delete_re_enables_source() {
        let home = tempdir().unwrap();
        let config_dir = tempdir().unwrap();
        let omp_dir = home.path().join(".omp/agent");
        fs::create_dir_all(&omp_dir).unwrap();
        fs::write(
            omp_dir.join("models.yml"),
            "providers:\n  DeepSeek-OpenAI:\n    baseUrl: https://api.example.com/v1\n    api: openai-completions\n    apiKey: sk-1234567890abcdef\n    models:\n      - id: deepseek-v4-pro\n",
        )
        .unwrap();

        // Migrate omp -> providers.toml now carries source = "omp".
        migrate_source(
            home.path(),
            config_dir.path(),
            "omp",
            &["omp-deepseek-openai".into()],
        )
        .expect("migration should succeed");
        let after_migrate = detect_sources(home.path(), config_dir.path())
            .into_iter()
            .find(|i| i.id == "omp")
            .unwrap();
        assert!(
            after_migrate.migrated,
            "source should be migrated after import"
        );
        assert!(after_migrate.providers.is_empty());

        // Delete every imported provider -> source is no longer migrated and
        // candidates are offered again.
        fs::write(config_dir.path().join("providers.toml"), "").unwrap();
        let after_delete = detect_sources(home.path(), config_dir.path())
            .into_iter()
            .find(|i| i.id == "omp")
            .unwrap();
        assert!(
            !after_delete.migrated,
            "deleting all imported providers re-enables the source"
        );
        assert!(
            !after_delete.providers.is_empty(),
            "candidates reappear after deletion"
        );
    }

    #[test]
    fn test_migrate_source_skips_existing_ids() {
        let home = tempdir().unwrap();
        let config_dir = tempdir().unwrap();
        // Pre-existing provider with the same id the migration would produce.
        fs::write(
            config_dir.path().join("providers.toml"),
            "[[providers]]\nid = \"omp-deepseek-openai\"\nprovider_type = \"openai-compat\"\nmodel = \"already\"\ntags = [\"general\"]\n",
        )
        .unwrap();
        let omp_dir = home.path().join(".omp/agent");
        fs::create_dir_all(&omp_dir).unwrap();
        fs::write(
            omp_dir.join("models.yml"),
            "providers:\n  DeepSeek-OpenAI:\n    baseUrl: https://api.example.com/v1\n    api: openai-completions\n    apiKey: sk-1234567890abcdef\n    models:\n      - id: deepseek-v4-pro\n",
        )
        .unwrap();

        let report = migrate_source(
            home.path(),
            config_dir.path(),
            "omp",
            &["omp-deepseek-openai".into()],
        )
        .expect("migration should succeed");
        assert!(report.imported.is_empty());
        assert_eq!(report.skipped, vec!["omp-deepseek-openai".to_string()]);
        // No duplicate block appended.
        let written = fs::read_to_string(config_dir.path().join("providers.toml")).unwrap();
        assert_eq!(written.matches("omp-deepseek-openai").count(), 1);
    }

    #[test]
    fn test_migrate_unsupported_source_errors() {
        let home = tempdir().unwrap();
        let config_dir = tempdir().unwrap();
        let codex_dir = home.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(
            codex_dir.join("auth.json"),
            r#"{"OPENAI_API_KEY": null, "auth_mode": "chatgpt"}"#,
        )
        .unwrap();
        let err = migrate_source(
            home.path(),
            config_dir.path(),
            "codex",
            &["codex-openai".into()],
        )
        .unwrap_err();
        assert!(err.contains("OAuth"));
    }

    // ---- misc --------------------------------------------------------------

    #[test]
    fn test_slug_and_mask() {
        assert_eq!(slug("DeepSeek-OpenAI"), "deepseek-openai");
        assert_eq!(slug("managed:kimi-code"), "managed-kimi-code");
        assert_eq!(mask_key("sk-1234567890abcdef"), "sk-1...cdef");
        assert_eq!(mask_key("short"), "••••");
    }
}
