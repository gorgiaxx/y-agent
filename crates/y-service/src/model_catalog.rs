//! models.dev catalog: download, cache, and fuzzy model-id resolution.
//!
//! The catalog is the public `models.dev` API payload (`api.json`), stored
//! verbatim in the user config directory so it stays interchangeable with the
//! upstream artifact. Callers get a flattened, y-agent-shaped view
//! ([`CatalogModel`]) plus a fuzzy search that tolerates the model-id spellings
//! aggregator gateways use (`[Kiro] claude-opus-4-6`, `deepseek/v3:free`,
//! `qwen3-235b-cloud`, ...).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use y_core::provider::ProviderCapability;

/// Upstream catalog endpoint. `catalog.json` carries both the per-provider
/// pricing map and the provider-agnostic (lab-published) model metadata, and
/// the latter is what tells us which provider is a model's first-party home.
pub const MODELS_DEV_API_URL: &str = "https://models.dev/catalog.json";

/// Catalog file name inside the user config directory.
pub const CATALOG_FILE_NAME: &str = "models.dev.json";

/// Trailing markers gateways append to an otherwise canonical model id.
const TRAILING_MARKERS: &[&str] = &[
    "-thinking",
    "-nothinking",
    "-reasoning",
    "-cloud",
    "-online",
    "-fp8",
    "-bf16",
    "-preview",
    ":free",
    ":thinking",
    ":online",
    ":nitro",
    ":floor",
];

/// Minimum score a fuzzy match must reach to be treated as a resolution.
/// Sits just below the weakest (subsequence) tier so partial ids still resolve.
const RESOLVE_THRESHOLD: u32 = 450;

/// Cap on within-tier penalties, keeping the scoring tiers 100 points apart.
const TIER_SPAN: u32 = 49;

/// Penalty applied when only the display name matched, never the id.
const NAME_PENALTY: u32 = 150;

/// Bonus for the provider that is the model's first-party home.
const CANONICAL_BONUS: u32 = 15;

/// Bonus for entries matching the configured provider type. Outweighs
/// [`CANONICAL_BONUS`]: an explicit user choice beats the general default.
const PROVIDER_HINT_BONUS: u32 = 30;

// ---------------------------------------------------------------------------
// Public data model
// ---------------------------------------------------------------------------

/// One provider/model pair flattened from the models.dev catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// models.dev provider id, e.g. `anthropic`.
    pub provider_id: String,
    /// Human-readable provider name.
    pub provider_name: String,
    /// Provider base API URL, when models.dev publishes one.
    pub provider_api: Option<String>,
    /// Environment variables the provider authenticates with.
    pub provider_env: Vec<String>,
    /// Provider-scoped model id, e.g. `claude-opus-4-6`.
    pub id: String,
    /// Human-readable model name.
    pub name: String,
    /// Supports native tool/function calling.
    pub tool_call: bool,
    /// Is a reasoning model.
    pub reasoning: bool,
    /// Capabilities in y-agent terms, derived from the published modalities.
    pub capabilities: Vec<ProviderCapability>,
    /// Context window in tokens.
    pub context_window: Option<u64>,
    /// Maximum output tokens.
    pub max_output_tokens: Option<u64>,
    /// USD per 1k input tokens (models.dev publishes per 1M).
    pub cost_per_1k_input: Option<f64>,
    /// USD per 1k output tokens.
    pub cost_per_1k_output: Option<f64>,
    /// Release date, `YYYY-MM` or `YYYY-MM-DD`.
    pub release_date: Option<String>,
    /// Knowledge cutoff.
    pub knowledge: Option<String>,
    /// True when this provider is the model's first-party home, i.e. the
    /// lab-published metadata is keyed `{provider_id}/{id}`.
    pub canonical: bool,
    /// Normalized spellings used for matching. Derived at load time so a
    /// search over thousands of entries allocates nothing.
    #[serde(skip)]
    keys: MatchKeys,
}

/// Precomputed normalized forms of a catalog entry.
#[derive(Debug, Clone, Default, PartialEq)]
struct MatchKeys {
    id: String,
    qualified: String,
    name: String,
    provider: String,
}

/// A ranked fuzzy-search hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogMatch {
    /// Match score; higher is better. `>= 450` means a confident resolution.
    pub score: u32,
    /// Whether the score clears the resolution threshold.
    pub resolved: bool,
    /// The matched catalog entry.
    pub model: CatalogModel,
}

/// Result of refreshing the local catalog copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogUpdateSummary {
    /// Absolute path the catalog was written to.
    pub path: String,
    /// URL the catalog was downloaded from.
    pub source_url: String,
    /// Number of providers in the downloaded catalog.
    pub provider_count: usize,
    /// Number of provider/model pairs.
    pub model_count: usize,
    /// RFC 3339 download timestamp.
    pub fetched_at: String,
    /// Size of the written file in bytes.
    pub bytes: u64,
}

/// The flattened local catalog plus its freshness metadata.
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    /// RFC 3339 mtime of the cached file, when present.
    pub fetched_at: Option<String>,
    /// All provider/model pairs, sorted by provider then model id.
    pub models: Arc<Vec<CatalogModel>>,
}

// ---------------------------------------------------------------------------
// Upstream payload (subset of the models.dev catalog.json schema)
// ---------------------------------------------------------------------------

/// Provider entries plus the ids of lab-published (first-party) models.
///
/// `catalog.json` nests the provider map under `providers` and publishes the
/// lab metadata under `models`; a bare `api.json` body is the provider map
/// itself. Both are accepted so an older cache keeps working.
struct RawCatalog {
    providers: HashMap<String, RawProvider>,
    canonical_ids: HashSet<String>,
}

impl RawCatalog {
    fn parse(body: &str) -> Result<Self, String> {
        let mut root: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(body).map_err(|e| format!("Invalid models.dev catalog: {e}"))?;

        let (providers, canonical_ids) = match root.remove("providers") {
            Some(providers) => {
                let ids = match root.remove("models") {
                    Some(serde_json::Value::Object(models)) => {
                        models.into_iter().map(|(key, _)| key).collect()
                    }
                    _ => HashSet::new(),
                };
                (providers, ids)
            }
            None => (serde_json::Value::Object(root), HashSet::new()),
        };

        Ok(Self {
            providers: serde_json::from_value(providers)
                .map_err(|e| format!("Invalid models.dev catalog: {e}"))?,
            canonical_ids,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawProvider {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    api: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    models: HashMap<String, RawModel>,
}

#[derive(Debug, Deserialize)]
struct RawModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tool_call: bool,
    #[serde(default)]
    reasoning: bool,
    #[serde(default)]
    modalities: Option<RawModalities>,
    #[serde(default)]
    limit: Option<RawLimit>,
    #[serde(default)]
    cost: Option<RawCost>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    knowledge: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawModalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawLimit {
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    output: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
}

// ---------------------------------------------------------------------------
// Paths, download, load
// ---------------------------------------------------------------------------

/// Path of the cached catalog inside `config_dir`.
pub fn catalog_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CATALOG_FILE_NAME)
}

/// Download the models.dev catalog and store it in `config_dir`.
///
/// `source_url` defaults to [`MODELS_DEV_API_URL`]. The payload is validated by
/// parsing it before anything is written, so a failed download never corrupts
/// an existing catalog.
pub async fn update_catalog(
    config_dir: &Path,
    source_url: Option<&str>,
) -> Result<CatalogUpdateSummary, String> {
    let url = source_url.unwrap_or(MODELS_DEV_API_URL);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Network error reaching {url}: {e}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response from {url}: {e}"))?;
    if !status.is_success() {
        return Err(format!("HTTP {status} from {url}"));
    }

    let models = parse_catalog(&body)?;
    let provider_count = models
        .iter()
        .map(|m| m.provider_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let path = catalog_path(config_dir);
    tokio::fs::create_dir_all(config_dir)
        .await
        .map_err(|e| format!("Failed to create config dir: {e}"))?;
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

    invalidate_cache();

    Ok(CatalogUpdateSummary {
        path: path.display().to_string(),
        source_url: url.to_string(),
        provider_count,
        model_count: models.len(),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        bytes: body.len() as u64,
    })
}

/// Load the cached catalog, returning an empty catalog when none exists.
///
/// Parsed results are memoized per file mtime, so repeated searches during
/// interactive typing do not re-parse the multi-hundred-kilobyte payload.
pub fn load_catalog(config_dir: &Path) -> Result<ModelCatalog, String> {
    let path = catalog_path(config_dir);
    let Ok(meta) = std::fs::metadata(&path) else {
        return Ok(ModelCatalog::default());
    };
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

    if let Some(hit) = cache_get(&path, mtime) {
        return Ok(hit);
    }

    let body = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let catalog = ModelCatalog {
        fetched_at: Some(chrono::DateTime::<chrono::Utc>::from(mtime).to_rfc3339()),
        models: Arc::new(parse_catalog(&body)?),
    };
    cache_put(&path, mtime, &catalog);
    Ok(catalog)
}

/// Flatten a models.dev catalog body into catalog entries.
fn parse_catalog(body: &str) -> Result<Vec<CatalogModel>, String> {
    let RawCatalog {
        providers,
        canonical_ids,
    } = RawCatalog::parse(body)?;

    let mut out = Vec::new();
    for (provider_key, provider) in providers {
        let provider_id = if provider.id.is_empty() {
            provider_key
        } else {
            provider.id
        };
        let provider_name = if provider.name.is_empty() {
            provider_id.clone()
        } else {
            provider.name
        };
        let normalized_provider = normalize(&provider_id);
        for (model_key, model) in provider.models {
            let id = if model.id.is_empty() {
                model_key
            } else {
                model.id
            };
            let name = if model.name.is_empty() {
                id.clone()
            } else {
                model.name
            };
            let modalities = model.modalities.unwrap_or_default();
            let qualified = format!("{provider_id}/{id}");
            out.push(CatalogModel {
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
                provider_api: provider.api.clone(),
                provider_env: provider.env.clone(),
                tool_call: model.tool_call,
                reasoning: model.reasoning,
                capabilities: derive_capabilities(&modalities),
                context_window: model.limit.as_ref().and_then(|l| l.context),
                max_output_tokens: model.limit.as_ref().and_then(|l| l.output),
                cost_per_1k_input: model.cost.as_ref().and_then(|c| c.input).map(per_1k),
                cost_per_1k_output: model.cost.as_ref().and_then(|c| c.output).map(per_1k),
                release_date: model.release_date,
                knowledge: model.knowledge,
                canonical: canonical_ids.contains(&qualified),
                keys: MatchKeys {
                    id: normalize(&id),
                    qualified: normalize(&qualified),
                    name: normalize(&name),
                    provider: normalized_provider.clone(),
                },
                id,
                name,
            });
        }
    }

    out.sort_by(|a, b| {
        a.provider_id
            .cmp(&b.provider_id)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

/// models.dev publishes USD per 1M tokens; y-agent config is per 1k.
fn per_1k(per_million: f64) -> f64 {
    per_million / 1000.0
}

/// Map published modalities onto y-agent provider capabilities.
fn derive_capabilities(modalities: &RawModalities) -> Vec<ProviderCapability> {
    let mut caps = Vec::new();
    if modalities.output.iter().any(|m| m == "text") || modalities.output.is_empty() {
        caps.push(ProviderCapability::Text);
    }
    if modalities.input.iter().any(|m| m == "image") {
        caps.push(ProviderCapability::Vision);
    }
    if modalities.output.iter().any(|m| m == "image") {
        caps.push(ProviderCapability::ImageGeneration);
    }
    caps
}

// ---------------------------------------------------------------------------
// Parsed-catalog cache
// ---------------------------------------------------------------------------

type CacheSlot = Option<(PathBuf, SystemTime, ModelCatalog)>;

static CACHE: LazyLock<RwLock<CacheSlot>> = LazyLock::new(|| RwLock::new(None));

fn cache_get(path: &Path, mtime: SystemTime) -> Option<ModelCatalog> {
    let guard = CACHE.read().ok()?;
    let (cached_path, cached_mtime, catalog) = guard.as_ref()?;
    (cached_path == path && *cached_mtime == mtime).then(|| catalog.clone())
}

fn cache_put(path: &Path, mtime: SystemTime, catalog: &ModelCatalog) {
    if let Ok(mut guard) = CACHE.write() {
        *guard = Some((path.to_path_buf(), mtime, catalog.clone()));
    }
}

fn invalidate_cache() {
    if let Ok(mut guard) = CACHE.write() {
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Rank catalog entries against a free-text model id.
///
/// Results are de-duplicated by model id: many resellers list the same model,
/// and the picker wants one row per model, from the best-fitting provider.
///
/// An empty `query` returns the head of the catalog (score 0) so callers can
/// use the same entry point to browse. `provider_hint` is a y-agent provider
/// type or models.dev provider id; entries from that provider get a small
/// bonus so `gpt-4o` under an `openai` provider outranks a reseller copy.
pub fn search_models(
    models: &[CatalogModel],
    query: &str,
    provider_hint: Option<&str>,
    limit: usize,
) -> Vec<CatalogMatch> {
    // A namespaced query (`deepseek/deepseek-v3`) names its provider outright,
    // which is more specific than the configured provider type.
    let normalized_query = normalize(query);
    let hint = match normalized_query.split_once('/') {
        Some((namespace, _)) if !namespace.is_empty() => namespace.to_string(),
        _ => provider_hint.map(normalize).unwrap_or_default(),
    };
    let candidates = query_candidates(query);

    // Score into (score, index) pairs so only the returned window is cloned.
    let mut scored: Vec<(u32, usize)> = models
        .iter()
        .enumerate()
        .filter_map(|(index, model)| {
            if candidates.is_empty() {
                return Some((0, index));
            }
            let base = candidates
                .iter()
                .filter_map(|candidate| score_model(model, candidate))
                .max()?;
            Some((base + placement_bonus(model, &hint), index))
        })
        .collect();

    scored.sort_by(|&(a_score, a_index), &(b_score, b_index)| {
        let (a, b) = (&models[a_index], &models[b_index]);
        b_score
            .cmp(&a_score)
            .then_with(|| a.id.len().cmp(&b.id.len()))
            .then_with(|| a.provider_id.cmp(&b.provider_id))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut seen = HashSet::with_capacity(limit);
    scored
        .into_iter()
        .filter(|&(_, index)| seen.insert(models[index].id.as_str()))
        .take(limit)
        .map(|(score, index)| CatalogMatch {
            score,
            resolved: score >= RESOLVE_THRESHOLD,
            model: models[index].clone(),
        })
        .collect()
}

/// Best confident match for a model id, if any.
pub fn resolve_model(
    models: &[CatalogModel],
    query: &str,
    provider_hint: Option<&str>,
) -> Option<CatalogModel> {
    search_models(models, query, provider_hint, 1)
        .into_iter()
        .find(|m| m.resolved)
        .map(|m| m.model)
}

/// Score one catalog entry against one normalized query candidate.
///
/// Id matches outrank display-name matches: a reseller whose *name* reads
/// "Claude Opus 4.6" must not beat the provider whose *id* is exactly that.
fn score_model(model: &CatalogModel, candidate: &str) -> Option<u32> {
    let by_id = [&model.keys.id, &model.keys.qualified]
        .into_iter()
        .filter_map(|target| score_pair(target, candidate))
        .max();
    let by_name =
        score_pair(&model.keys.name, candidate).map(|score| score.saturating_sub(NAME_PENALTY));
    by_id.max(by_name)
}

/// Score a normalized target against a normalized candidate.
///
/// Tiers are 100 points apart and each tier's penalty is capped at
/// [`TIER_SPAN`], so a weaker tier can never overtake a stronger one even
/// after the placement bonuses are added.
fn score_pair(target: &str, candidate: &str) -> Option<u32> {
    if target.is_empty() || candidate.is_empty() {
        return None;
    }
    if target == candidate {
        return Some(1000);
    }
    if strip_trailing_markers(target) == candidate {
        return Some(900);
    }
    if target.starts_with(candidate) || candidate.starts_with(target) {
        return Some(800 - span(target.len().abs_diff(candidate.len()) as u32));
    }
    if let Some(offset) = target.find(candidate) {
        return Some(700 - span(offset as u32));
    }
    if let Some(offset) = candidate.find(target) {
        return Some(600 - span(offset as u32));
    }
    subsequence_score(target, candidate).map(|gaps| 500 - span(gaps))
}

/// Clamp a within-tier penalty so tiers never overlap.
fn span(penalty: u32) -> u32 {
    penalty.min(TIER_SPAN)
}

/// Score `candidate` as an in-order subsequence of `target`; `None` if it is
/// not one. The returned value is the total gap size (lower is better).
fn subsequence_score(target: &str, candidate: &str) -> Option<u32> {
    let mut gaps = 0u32;
    let mut target_chars = target.chars();
    let mut matched_any = false;
    for want in candidate.chars() {
        let mut skipped = 0u32;
        loop {
            let next = target_chars.next()?;
            if next == want {
                matched_any = true;
                break;
            }
            skipped += 1;
        }
        gaps += skipped;
    }
    matched_any.then_some(gaps)
}

/// Bonus preferring a model's first-party provider, and the provider type the
/// user already configured. Both are small enough to never cross a score tier.
fn placement_bonus(model: &CatalogModel, hint: &str) -> u32 {
    let canonical = if model.canonical { CANONICAL_BONUS } else { 0 };
    if hint.is_empty() {
        return canonical;
    }
    let provider = &model.keys.provider;
    let matches_hint =
        provider == hint || hint.contains(provider.as_str()) || provider.contains(hint);
    canonical + if matches_hint { PROVIDER_HINT_BONUS } else { 0 }
}

/// Normalized spellings to try for a raw user-supplied model id.
///
/// Gateways wrap ids (`[Kiro] claude-opus-4-6`), namespace them
/// (`anthropic/claude-opus-4-6`), and append markers (`:free`, `-thinking`).
fn query_candidates(query: &str) -> Vec<String> {
    let base = normalize(query);
    if base.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut push = |value: String| {
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    };

    push(base.clone());
    push(strip_trailing_markers(&base).to_string());

    if let Some((_, tail)) = base.rsplit_once('/') {
        let tail = tail.to_string();
        push(strip_trailing_markers(&tail).to_string());
        push(tail);
    }

    out
}

/// Lowercase, drop bracketed wrappers, collapse separators.
fn normalize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut depth = 0usize;
    for ch in value.chars() {
        match ch {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            ':' | '_' | ' ' | '.' => out.push('-'),
            _ => out.extend(ch.to_lowercase()),
        }
    }
    out.trim_matches('-').to_string()
}

/// Drop known gateway suffixes from a normalized id.
fn strip_trailing_markers(value: &str) -> &str {
    let mut current = value;
    loop {
        let Some(next) = TRAILING_MARKERS
            .iter()
            .map(|marker| normalized_marker(marker))
            .find_map(|marker| current.strip_suffix(marker.as_str()))
        else {
            return current;
        };
        if next.is_empty() {
            return current;
        }
        current = next;
    }
}

fn normalized_marker(marker: &str) -> String {
    marker.replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `catalog.json` payload: three providers list overlapping models, and
    /// the lab metadata marks the first-party home of two of them.
    const SAMPLE: &str = r#"{
        "providers": {
            "anthropic": {
                "id": "anthropic",
                "name": "Anthropic",
                "api": "https://api.anthropic.com/v1",
                "env": ["ANTHROPIC_API_KEY"],
                "models": {
                    "claude-opus-4-6": {
                        "id": "claude-opus-4-6",
                        "name": "Claude Opus 4.6",
                        "tool_call": true,
                        "reasoning": true,
                            "modalities": {"input": ["text", "image"], "output": ["text"]},
                        "limit": {"context": 200000, "output": 64000},
                        "cost": {"input": 5, "output": 25},
                        "release_date": "2026-02-05",
                        "knowledge": "2025-11"
                    }
                }
            },
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "env": ["OPENAI_API_KEY"],
                "models": {
                    "gpt-4o": {
                        "id": "gpt-4o",
                        "name": "GPT-4o",
                        "tool_call": true,
                        "reasoning": false,
                            "modalities": {"input": ["text", "image"], "output": ["text"]},
                        "limit": {"context": 128000, "output": 16384},
                        "cost": {"input": 2.5, "output": 10}
                    },
                    "gpt-image-1": {
                        "id": "gpt-image-1",
                        "name": "GPT Image 1",
                        "tool_call": false,
                        "reasoning": false,
                            "modalities": {"input": ["text", "image"], "output": ["image"]},
                        "limit": {"context": 4096, "output": 4096},
                        "cost": {"input": 10, "output": 40}
                    }
                }
            },
            "reseller": {
                "id": "reseller",
                "name": "Reseller",
                "env": [],
                "models": {
                    "gpt-4o": {
                        "id": "gpt-4o",
                        "name": "GPT-4o",
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 128000, "output": 16384}
                    },
                    "claude-opus4-6": {
                        "id": "claude-opus4-6",
                        "name": "Claude Opus 4.6",
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 1000000, "output": 64000}
                    }
                }
            }
        },
        "models": {
            "anthropic/claude-opus-4-6": {"id": "anthropic/claude-opus-4-6"},
            "openai/gpt-4o": {"id": "openai/gpt-4o"}
        }
    }"#;

    fn sample() -> Vec<CatalogModel> {
        parse_catalog(SAMPLE).expect("sample catalog parses")
    }

    #[test]
    fn flattens_providers_and_converts_cost_to_per_1k() {
        let models = sample();
        assert_eq!(models.len(), 5);

        let opus = models.iter().find(|m| m.id == "claude-opus-4-6").unwrap();
        assert_eq!(opus.provider_name, "Anthropic");
        assert_eq!(opus.context_window, Some(200_000));
        assert_eq!(opus.max_output_tokens, Some(64_000));
        // models.dev publishes USD per 1M tokens.
        assert_eq!(opus.cost_per_1k_input, Some(0.005));
        assert_eq!(opus.cost_per_1k_output, Some(0.025));
        assert!(opus.tool_call && opus.reasoning);
    }

    #[test]
    fn derives_capabilities_from_modalities() {
        let models = sample();
        let opus = models.iter().find(|m| m.id == "claude-opus-4-6").unwrap();
        assert_eq!(
            opus.capabilities,
            vec![ProviderCapability::Text, ProviderCapability::Vision]
        );

        let image = models.iter().find(|m| m.id == "gpt-image-1").unwrap();
        assert_eq!(
            image.capabilities,
            vec![
                ProviderCapability::Vision,
                ProviderCapability::ImageGeneration
            ]
        );
    }

    #[test]
    fn resolves_exact_ids() {
        let models = sample();
        let hit = resolve_model(&models, "gpt-4o", None).expect("exact id resolves");
        assert_eq!(hit.id, "gpt-4o");
        assert_eq!(hit.provider_id, "openai");
    }

    #[test]
    fn resolves_gateway_spellings() {
        let models = sample();
        for query in [
            "[Kiro] claude-opus-4-6",
            "anthropic/claude-opus-4-6",
            "claude-opus-4-6:thinking",
            "Claude_Opus_4_6",
            "CLAUDE-OPUS-4-6",
        ] {
            let hit = resolve_model(&models, query, None)
                .unwrap_or_else(|| panic!("{query} should resolve"));
            assert_eq!(hit.id, "claude-opus-4-6", "query: {query}");
        }
    }

    #[test]
    fn ranks_partial_input_by_relevance() {
        let models = sample();
        let hits = search_models(&models, "gpt-4", None, 5);
        assert_eq!(hits[0].model.id, "gpt-4o");
        assert!(hits[0].score >= RESOLVE_THRESHOLD);
    }

    #[test]
    fn prefers_the_first_party_provider_over_a_reseller() {
        let models = sample();
        let hit = resolve_model(&models, "gpt-4o", None).unwrap();
        assert_eq!(hit.provider_id, "openai");
        assert!(hit.canonical);
    }

    #[test]
    fn an_id_match_outranks_a_reseller_whose_display_name_matches() {
        let models = sample();
        // "reseller" lists `claude-opus4-6` under the name "Claude Opus 4.6";
        // the exact id under `anthropic` must still win.
        let hit = resolve_model(&models, "[Kiro] claude-opus-4-6", None).unwrap();
        assert_eq!(hit.provider_id, "anthropic");
        assert_eq!(hit.id, "claude-opus-4-6");
    }

    #[test]
    fn provider_hint_breaks_ties_toward_the_matching_provider() {
        let models = sample();
        let hit = resolve_model(&models, "gpt-4o", Some("reseller")).unwrap();
        assert_eq!(hit.provider_id, "reseller");
    }

    #[test]
    fn results_carry_one_row_per_model_id() {
        let models = sample();
        let hits = search_models(&models, "gpt-4o", None, 10);
        let ids: Vec<&str> = hits.iter().map(|h| h.model.id.as_str()).collect();
        assert_eq!(ids.iter().filter(|id| **id == "gpt-4o").count(), 1);
    }

    #[test]
    fn accepts_a_legacy_api_json_provider_map() {
        let legacy = r#"{"openai":{"id":"openai","name":"OpenAI","models":{
            "gpt-4o":{"id":"gpt-4o","name":"GPT-4o","limit":{"context":128000}}}}}"#;
        let models = parse_catalog(legacy).expect("api.json still parses");
        assert_eq!(models.len(), 1);
        assert!(!models[0].canonical);
    }

    #[test]
    fn unknown_ids_do_not_resolve() {
        let models = sample();
        assert!(resolve_model(&models, "totally-unrelated-zzz", None).is_none());
    }

    #[test]
    fn empty_query_browses_the_catalog() {
        let models = sample();
        let hits = search_models(&models, "  ", None, 2);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| !h.resolved));
    }

    #[test]
    fn load_catalog_returns_empty_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = load_catalog(dir.path()).unwrap();
        assert!(catalog.models.is_empty());
        assert!(catalog.fetched_at.is_none());
    }

    #[test]
    fn load_catalog_reads_the_cached_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(catalog_path(dir.path()), SAMPLE).unwrap();
        let catalog = load_catalog(dir.path()).unwrap();
        assert_eq!(catalog.models.len(), 5);
        assert!(catalog.fetched_at.is_some());
    }

    #[test]
    fn load_catalog_rejects_malformed_payloads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(catalog_path(dir.path()), "not json").unwrap();
        assert!(load_catalog(dir.path()).is_err());
    }
}
