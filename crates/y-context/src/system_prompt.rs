//! `BuildSystemPrompt` pipeline stage.
//!
//! Design reference: prompt-design.md, context-session-design.md §Pipeline Stages
//!
//! This stage runs at priority 100 and assembles the system prompt from a
//! `PromptTemplate` + `SectionStore`. Sections are lazily loaded: only those
//! whose condition evaluates to true against the current `PromptContext` are
//! fetched and included. Token budgets are enforced per-section and in total.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use y_core::runtime::RuntimeBackend;
use y_prompt::{
    builtin_section_store_with_overrides, estimate_tokens, truncate_to_budget, PromptContext,
    PromptTemplate, SectionStore,
};

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

// ---------------------------------------------------------------------------
// Virtual environment prompt info
// ---------------------------------------------------------------------------

/// Lightweight snapshot of virtual environment paths for prompt injection.
///
/// Constructed from `y_runtime::PythonVenvConfig` / `y_runtime::BunVenvConfig`
/// at service startup and threaded into the system prompt provider.
#[derive(Debug, Clone, Default)]
pub struct VenvPromptInfo {
    /// Python (uv) environment info.
    pub python: Option<PythonVenvPromptInfo>,
    /// JavaScript (bun) environment info.
    pub bun: Option<BunVenvPromptInfo>,
}

/// Python virtual environment info for prompt injection.
#[derive(Debug, Clone)]
pub struct PythonVenvPromptInfo {
    pub uv_path: String,
    pub python_version: String,
    pub venv_dir: String,
    pub working_dir: String,
}

/// Bun virtual environment info for prompt injection.
#[derive(Debug, Clone)]
pub struct BunVenvPromptInfo {
    pub bun_path: String,
    pub bun_version: String,
    pub working_dir: String,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the system prompt provider.
pub struct SystemPromptConfig {
    /// Enable template-based prompt assembly.
    /// When false, the provider emits only the `fallback_prompt`.
    pub prompt_templates_enabled: bool,
    /// Fallback prompt used when templates are disabled or all sections are excluded.
    pub fallback_prompt: String,
}

impl Default for SystemPromptConfig {
    fn default() -> Self {
        Self {
            prompt_templates_enabled: true,
            fallback_prompt: "You are a helpful AI assistant.".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Assembles the system prompt from a `PromptTemplate` and `SectionStore`.
///
/// Runs at pipeline priority 100 (`BUILD_SYSTEM_PROMPT`).
///
/// The provider resolves effective sections for the current agent mode,
/// evaluates conditions, lazy-loads content, enforces token budgets,
/// and emits a single `ContextItem` with category `SystemPrompt`.
///
/// Dynamic sections (`core.datetime`, `core.environment`) have their
/// placeholder content replaced with live values at assembly time.
pub struct BuildSystemPromptProvider {
    template: PromptTemplate,
    store: RwLock<SectionStore>,
    prompt_context: Arc<RwLock<PromptContext>>,
    config: SystemPromptConfig,
    /// Virtual environment info for prompt injection (optional).
    venv_info: VenvPromptInfo,
    /// Path to the user prompts override directory (for hot-reload).
    prompts_dir: Option<PathBuf>,
    /// Runtime backend used to select the tool-protocol variant.
    runtime_backend: RuntimeBackend,
    /// Dynamic text listing user-callable agents. Injected from `ServiceContainer`
    /// and replaced into the `{{CALLABLE_AGENTS}}` placeholder in core.orchestration.
    callable_agents_text: Arc<RwLock<String>>,
    /// Cached stable portion of the system prompt (sections that don't change
    /// within a session). Invalidated when agent mode, selected sections,
    /// custom prompt, or config flags change.
    stable_cache: RwLock<Option<StableCache>>,
}

/// Cached stable system prompt content with its cache key.
///
/// The cache key is a hash of all inputs that affect the stable portion:
/// agent mode, selected prompt sections, custom prompt presence, and
/// relevant config flags. When the key changes, the cache is invalidated.
#[derive(Clone)]
struct StableCache {
    key: String,
    content: String,
    token_estimate: u32,
}

impl BuildSystemPromptProvider {
    /// Create a new system prompt provider.
    pub fn new(
        template: PromptTemplate,
        store: SectionStore,
        prompt_context: Arc<RwLock<PromptContext>>,
        config: SystemPromptConfig,
    ) -> Self {
        Self {
            template,
            store: RwLock::new(store),
            prompt_context,
            config,
            venv_info: VenvPromptInfo::default(),
            prompts_dir: None,
            runtime_backend: RuntimeBackend::Native,
            callable_agents_text: Arc::new(RwLock::new(String::new())),
            stable_cache: RwLock::new(None),
        }
    }

    /// Create a new system prompt provider with virtual environment info.
    pub fn with_venv_info(
        template: PromptTemplate,
        store: SectionStore,
        prompt_context: Arc<RwLock<PromptContext>>,
        config: SystemPromptConfig,
        venv_info: VenvPromptInfo,
        runtime_backend: RuntimeBackend,
    ) -> Self {
        Self {
            template,
            store: RwLock::new(store),
            prompt_context,
            config,
            venv_info,
            prompts_dir: None,
            runtime_backend,
            callable_agents_text: Arc::new(RwLock::new(String::new())),
            stable_cache: RwLock::new(None),
        }
    }

    /// Set the prompts directory path (for hot-reload support).
    pub fn set_prompts_dir(&mut self, dir: Option<PathBuf>) {
        self.prompts_dir = dir;
    }

    /// Hot-reload the section store from disk.
    ///
    /// Re-reads all prompt files from `prompts_dir` and rebuilds the
    /// `SectionStore`. This is called when the user saves prompts in the
    /// GUI settings and is a no-op if `prompts_dir` was never set.
    pub async fn reload_store(&self) {
        let new_store = builtin_section_store_with_overrides(
            self.prompts_dir.as_deref(),
            &self.runtime_backend,
        );
        let mut guard = self.store.write().await;
        *guard = new_store;
        // Invalidate stable cache since section content may have changed.
        self.invalidate_stable_cache().await;
        tracing::info!("Prompt section store hot-reloaded");
    }

    /// Invalidate the stable system prompt cache.
    ///
    /// Called when inputs that affect the stable portion change: agent mode,
    /// selected prompt sections, custom prompt, config flags, or prompt
    /// section store hot-reload. The next `provide()` call will rebuild the
    /// stable content from scratch.
    pub async fn invalidate_stable_cache(&self) {
        let mut guard = self.stable_cache.write().await;
        if guard.is_some() {
            tracing::debug!("stable system prompt cache invalidated");
            *guard = None;
        }
    }

    /// Get a reference to the callable agents text handle.
    ///
    /// The `ServiceContainer` uses this to inject the dynamic agent list.
    pub fn callable_agents_handle(&self) -> Arc<RwLock<String>> {
        Arc::clone(&self.callable_agents_text)
    }

    /// Generate dynamic content for `core.datetime`.
    fn generate_datetime() -> String {
        let now = chrono::Utc::now();
        format!(
            "\nCurrent date and time: {} UTC\n",
            now.format("%Y-%m-%d %H:%M:%S")
        )
    }

    /// Generate dynamic content for `core.environment`.
    ///
    /// When `workspace_path` is provided (e.g. from a workspace-bound
    /// session), it is appended as an extra field.
    fn generate_environment(workspace_path: Option<&str>, venv_info: &VenvPromptInfo) -> String {
        use std::fmt::Write;

        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        tracing::debug!(workspace_path = ?workspace_path, "generate_environment");

        let mut env_str = format!("Environment: OS={os}, arch={arch}");
        if let Some(ws) = workspace_path.filter(|s| !s.is_empty()) {
            let _ = write!(&mut env_str, ", workspace_path={ws}");
        }

        // Append Python (uv) venv info.
        if let Some(ref py) = venv_info.python {
            let _ = write!(
                &mut env_str,
                ", python_env=uv(version={}, venv={}, uv_path={}, working_dir={})",
                py.python_version, py.venv_dir, py.uv_path, py.working_dir
            );
        }

        // Append JavaScript (bun) venv info.
        if let Some(ref bun) = venv_info.bun {
            let _ = write!(
                &mut env_str,
                ", js_env=bun(version={}, bun_path={}, working_dir={})",
                bun.bun_version, bun.bun_path, bun.working_dir
            );
        }

        env_str
    }
}

/// Sections whose content changes every turn and must NOT be cached.
///
/// These are rebuilt on every `provide()` call and appended after the
/// cached stable content. Keeping them out of the stable cache ensures
/// the system prompt prefix remains byte-stable for KV cache hits.
fn is_volatile_section(section_id: &str) -> bool {
    matches!(
        section_id,
        "core.datetime" | "core.environment" | "core.mcp_hint"
    )
}

/// Compute a cache key for the stable portion of the system prompt.
///
/// The key incorporates all inputs that affect stable sections:
/// - Agent mode
/// - Selected prompt sections (if any)
/// - Custom system prompt presence (not content — content changes invalidate)
/// - Config flags that affect section conditions
/// - Callable agents text (affects core.orchestration)
fn compute_cache_key(prompt_ctx: &PromptContext, callable_agents: &str) -> String {
    let mut key = String::new();
    key.push_str("mode=");
    key.push_str(&prompt_ctx.agent_mode);
    key.push(';');
    if let Some(ref selected) = prompt_ctx.selected_prompt_sections {
        key.push_str("sections=");
        key.push_str(&selected.join(","));
    }
    key.push(';');
    key.push_str("custom=");
    key.push_str(&prompt_ctx.custom_system_prompt.is_some().to_string());
    key.push(';');
    // Include config flags that affect section conditions (tool_calling,
    // plan_mode, loop_mode, mcp, orchestration).
    let relevant_flags: Vec<(&String, &bool)> = prompt_ctx
        .config_flags
        .iter()
        .filter(|(k, _)| {
            k.starts_with("tool_calling.")
                || k.starts_with("plan_mode.")
                || k.starts_with("loop_mode.")
                || k.starts_with("mcp.")
                || k.starts_with("orchestration.")
        })
        .collect();
    key.push_str("flags=");
    for (k, v) in &relevant_flags {
        key.push_str(k);
        key.push('=');
        key.push_str(&v.to_string());
        key.push(',');
    }
    key.push(';');
    // Callable agents text affects core.orchestration.
    key.push_str("agents=");
    key.push_str(callable_agents);
    key
}

#[async_trait]
impl ContextProvider for BuildSystemPromptProvider {
    fn name(&self) -> &'static str {
        "build_system_prompt"
    }

    fn priority(&self) -> u32 {
        100 // stage_priorities::BUILD_SYSTEM_PROMPT
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        // Feature flag: when templates are disabled, emit fallback only.
        if !self.config.prompt_templates_enabled {
            let tokens = estimate_tokens(&self.config.fallback_prompt);
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.config.fallback_prompt.clone(),
                token_estimate: tokens,
                priority: 100,
            });
            return Ok(());
        }

        let prompt_ctx = self.prompt_context.read().await;
        let has_custom_prompt = prompt_ctx.custom_system_prompt.is_some();
        let mode = &prompt_ctx.agent_mode;
        let selected_sections = prompt_ctx.selected_prompt_sections.as_ref();
        let effective_sections = self
            .template
            .effective_sections(mode)
            .into_iter()
            .filter(|section| {
                selected_sections.is_none_or(|selected| {
                    selected.contains(&section.section_id)
                        || is_runtime_functional_section_active(&section.section_id, &prompt_ctx)
                })
            })
            .collect::<Vec<_>>();
        let total_budget = self.template.effective_budget(mode);

        tracing::debug!(
            mode = %mode,
            config_flags = ?prompt_ctx.config_flags,
            effective_section_count = effective_sections.len(),
            "system prompt assembly: prompt context state"
        );

        // Compute cache key for the stable portion.
        let callable_agents = self.callable_agents_text.read().await;
        let cache_key = compute_cache_key(&prompt_ctx, &callable_agents);
        drop(callable_agents);

        // Resolve sections sorted by their effective priority.
        let store = self.store.read().await;
        let mut section_entries: Vec<_> = effective_sections
            .iter()
            .filter_map(|eff| {
                let section = store.get(&eff.section_id)?;
                let priority = eff.priority_override.unwrap_or(section.priority);
                Some((eff, section, priority))
            })
            .collect();
        section_entries.sort_by_key(|&(_, _, p)| p);

        // Try to use cached stable content.
        let stable_cache_guard = self.stable_cache.read().await;
        let (mut accumulated, mut cumulative_tokens) = if let Some(ref cached) = *stable_cache_guard
        {
            if cached.key == cache_key {
                tracing::debug!(
                    cache_key = %cache_key,
                    tokens = cached.token_estimate,
                    "stable system prompt cache hit"
                );
                (cached.content.clone(), cached.token_estimate)
            } else {
                drop(stable_cache_guard);
                self.rebuild_stable_cache(
                    &prompt_ctx,
                    &store,
                    &effective_sections,
                    has_custom_prompt,
                    total_budget,
                    &cache_key,
                )
                .await
            }
        } else {
            drop(stable_cache_guard);
            self.rebuild_stable_cache(
                &prompt_ctx,
                &store,
                &effective_sections,
                has_custom_prompt,
                total_budget,
                &cache_key,
            )
            .await
        };

        // Append volatile sections (datetime, environment, mcp_hint).
        // These are rebuilt every turn and must NOT be cached.
        for (eff, section, _priority) in &section_entries {
            if !is_volatile_section(&eff.section_id) {
                continue;
            }

            let condition = eff
                .condition_override
                .as_ref()
                .or(section.condition.as_ref());
            if let Some(cond) = condition {
                if !cond.evaluate(&prompt_ctx) {
                    continue;
                }
            }

            let Ok(content) = store.load_content(&eff.section_id) else {
                continue;
            };

            let content = match eff.section_id.as_str() {
                "core.datetime" => Self::generate_datetime(),
                "core.environment" => Self::generate_environment(
                    prompt_ctx.working_directory.as_deref(),
                    &self.venv_info,
                ),
                "core.mcp_hint" => {
                    if let Some(ref instructions) = prompt_ctx.mcp_server_instructions {
                        format!("{content}\n\n{instructions}")
                    } else {
                        content
                    }
                }
                _ => content,
            };

            let (content, _) = truncate_to_budget(&content, section.token_budget);
            let tokens = estimate_tokens(&content);

            if cumulative_tokens + tokens > total_budget {
                break;
            }

            if !accumulated.is_empty() {
                accumulated.push('\n');
            }
            accumulated.push_str(&content);
            cumulative_tokens += tokens;
        }

        // Fallback when all sections are excluded.
        if accumulated.is_empty() {
            accumulated.clone_from(&self.config.fallback_prompt);
            cumulative_tokens = estimate_tokens(&accumulated);
        }

        ctx.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: accumulated,
            token_estimate: cumulative_tokens,
            priority: 100,
        });

        Ok(())
    }
}

impl BuildSystemPromptProvider {
    /// Rebuild the stable cache from non-volatile sections.
    async fn rebuild_stable_cache(
        &self,
        prompt_ctx: &PromptContext,
        store: &SectionStore,
        effective_sections: &[y_prompt::EffectiveSection],
        has_custom_prompt: bool,
        total_budget: u32,
        cache_key: &str,
    ) -> (String, u32) {
        let mut accumulated = String::new();
        let mut cumulative_tokens: u32 = 0;

        if let Some(ref custom) = prompt_ctx.custom_system_prompt {
            let (custom, _) = truncate_to_budget(custom, total_budget);
            accumulated.push_str(&custom);
            cumulative_tokens = estimate_tokens(&custom);
        }

        let mut section_entries: Vec<_> = effective_sections
            .iter()
            .filter_map(|eff| {
                let section = store.get(&eff.section_id)?;
                let priority = eff.priority_override.unwrap_or(section.priority);
                Some((eff, section, priority))
            })
            .collect();
        section_entries.sort_by_key(|&(_, _, p)| p);

        for (eff, section, _priority) in &section_entries {
            if is_volatile_section(&eff.section_id) {
                continue;
            }
            if has_custom_prompt && is_custom_prompt_replaced(&eff.section_id) {
                continue;
            }
            let condition = eff
                .condition_override
                .as_ref()
                .or(section.condition.as_ref());
            if let Some(cond) = condition {
                if !cond.evaluate(prompt_ctx) {
                    continue;
                }
            }
            let Ok(content) = store.load_content(&eff.section_id) else {
                continue;
            };
            let content = if eff.section_id == "core.orchestration" {
                let agents = self.callable_agents_text.read().await;
                content.replace("{{CALLABLE_AGENTS}}", &agents)
            } else {
                content
            };
            let (content, _) = truncate_to_budget(&content, section.token_budget);
            let tokens = estimate_tokens(&content);
            if cumulative_tokens + tokens > total_budget {
                break;
            }
            if !accumulated.is_empty() {
                accumulated.push('\n');
            }
            accumulated.push_str(&content);
            cumulative_tokens += tokens;
        }

        let mut cache_guard = self.stable_cache.write().await;
        *cache_guard = Some(StableCache {
            key: cache_key.to_string(),
            content: accumulated.clone(),
            token_estimate: cumulative_tokens,
        });
        tracing::debug!(
            cache_key = %cache_key,
            tokens = cumulative_tokens,
            "stable system prompt cache rebuilt"
        );

        (accumulated, cumulative_tokens)
    }
}

/// Sections replaced by a per-session custom system prompt.
///
/// These are the identity/behavioral sections that define "who the agent is"
/// and "how it should behave". Dynamic/functional sections (datetime,
/// environment, `tool_protocol`, planning, exploration, orchestration,
/// `plan_mode_active`) are NOT in this list and will remain active even when
/// a custom prompt is set.
const CUSTOM_PROMPT_REPLACED_SECTIONS: &[&str] = &[
    "core.identity",
    "core.guidelines",
    "core.security",
    "core.persona",
];

/// Check whether a section is replaced when a custom system prompt is active.
fn is_custom_prompt_replaced(section_id: &str) -> bool {
    CUSTOM_PROMPT_REPLACED_SECTIONS.contains(&section_id)
}

/// Runtime sections controlled by config flags must remain available even
/// when a session/agent selected an explicit prompt-section subset.
fn is_runtime_functional_section_active(section_id: &str, ctx: &PromptContext) -> bool {
    let required_flag = match section_id {
        "core.plan_mode_active" => "plan_mode.active",
        "core.loop_mode_active" => "loop_mode.active",
        "core.mcp_hint" => "mcp.enabled",
        "core.orchestration" => "orchestration.enabled",
        _ => return false,
    };
    ctx.config_flags
        .get(required_flag)
        .copied()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_prompt::{builtin_section_store, default_template};

    fn make_provider(ctx: PromptContext, config: SystemPromptConfig) -> BuildSystemPromptProvider {
        BuildSystemPromptProvider::new(
            default_template(),
            builtin_section_store(),
            Arc::new(RwLock::new(ctx)),
            config,
        )
    }

    fn general_ctx() -> PromptContext {
        PromptContext {
            agent_mode: "general".into(),
            active_skills: vec![],
            available_tools: vec!["FileRead".into()],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
            custom_system_prompt: None,
            selected_prompt_sections: None,
            mcp_server_instructions: None,
        }
    }

    #[test]
    fn test_provider_name_and_priority() {
        let provider = make_provider(general_ctx(), SystemPromptConfig::default());
        assert_eq!(provider.name(), "build_system_prompt");
        assert_eq!(provider.priority(), 100);
    }

    #[tokio::test]
    async fn test_provide_emits_system_prompt() {
        let provider = make_provider(general_ctx(), SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        let item = &ctx.items[0];
        assert_eq!(item.category, ContextCategory::SystemPrompt);
        // Should contain identity section content.
        assert!(item.content.contains("y-agent"));
        // Should contain guidelines.
        assert!(item.content.contains("Guidelines"));
        // Should contain security.
        assert!(item.content.contains("Security rules"));
        // core.tool_protocol is now always included
        assert!(item.content.contains("Tool Usage Protocol"));
        // Token estimate should be reasonable.
        assert!(item.token_estimate > 0);
    }

    #[test]
    fn test_environment_prompt_omits_shell_cwd() {
        let content = BuildSystemPromptProvider::generate_environment(
            Some("/tmp/session-workspace"),
            &VenvPromptInfo::default(),
        );

        assert!(content.contains("workspace_path=/tmp/session-workspace"));
        assert!(!content.contains("shell_cwd"));
    }

    #[tokio::test]
    async fn test_conditions_exclude_sections() {
        // Plan mode excludes core.tool_behavior via overlay.
        let plan_ctx = PromptContext {
            agent_mode: "plan".into(),
            active_skills: vec![],
            available_tools: vec!["FileRead".into()],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
            custom_system_prompt: None,
            selected_prompt_sections: None,
            mcp_server_instructions: None,
        };
        let provider = make_provider(plan_ctx, SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        // Plan mode: no sections excluded; planning included.
        assert!(content.contains("planning mode"));
    }

    #[tokio::test]
    async fn test_per_section_budget_truncates() {
        // Create a custom store with an oversized section.
        let mut store = SectionStore::new();
        store.register(y_prompt::PromptSection {
            id: "core.identity".into(),
            content_source: y_prompt::ContentSource::Inline("x".repeat(5000)),
            token_budget: 10, // Very small budget.
            priority: 100,
            condition: Some(y_prompt::SectionCondition::Always),
            category: y_prompt::SectionCategory::Identity,
        });

        let template = y_prompt::PromptTemplate {
            id: "test".into(),
            parent: None,
            sections: vec![y_prompt::SectionRef {
                section_id: "core.identity".into(),
                priority_override: None,
                condition_override: None,
                enabled: true,
            }],
            mode_overlays: std::collections::HashMap::new(),
            total_token_budget: 4000,
        };

        let provider = BuildSystemPromptProvider::new(
            template,
            store,
            Arc::new(RwLock::new(general_ctx())),
            SystemPromptConfig::default(),
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert!(ctx.items[0].content.contains("[truncated]"));
    }

    #[tokio::test]
    async fn test_total_budget_drops_low_priority() {
        // Template with very small total budget — only first section should fit.
        let mut store = SectionStore::new();
        store.register(y_prompt::PromptSection {
            id: "first".into(),
            content_source: y_prompt::ContentSource::Inline("A".repeat(100)),
            token_budget: 500,
            priority: 100,
            condition: Some(y_prompt::SectionCondition::Always),
            category: y_prompt::SectionCategory::Identity,
        });
        store.register(y_prompt::PromptSection {
            id: "second".into(),
            content_source: y_prompt::ContentSource::Inline("B".repeat(100)),
            token_budget: 500,
            priority: 200,
            condition: Some(y_prompt::SectionCondition::Always),
            category: y_prompt::SectionCategory::Behavioral,
        });

        let template = y_prompt::PromptTemplate {
            id: "test".into(),
            parent: None,
            sections: vec![
                y_prompt::SectionRef {
                    section_id: "first".into(),
                    priority_override: None,
                    condition_override: None,
                    enabled: true,
                },
                y_prompt::SectionRef {
                    section_id: "second".into(),
                    priority_override: None,
                    condition_override: None,
                    enabled: true,
                },
            ],
            mode_overlays: std::collections::HashMap::new(),
            total_token_budget: 30, // Only ~30 tokens — "A".repeat(100) = 25 tokens
        };

        let provider = BuildSystemPromptProvider::new(
            template,
            store,
            Arc::new(RwLock::new(general_ctx())),
            SystemPromptConfig::default(),
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains('A'));
        assert!(!content.contains('B')); // Second section dropped.
    }

    #[tokio::test]
    async fn test_all_excluded_uses_fallback() {
        // Context with no tools, no skills, no config flags, and mode that excludes everything.
        let empty_ctx = PromptContext {
            agent_mode: "nonexistent_mode".into(),
            active_skills: vec![],
            available_tools: vec![],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
            custom_system_prompt: None,
            selected_prompt_sections: None,
            mcp_server_instructions: None,
        };

        // Template where every section has a condition that won't match.
        let mut store = SectionStore::new();
        store.register(y_prompt::PromptSection {
            id: "only".into(),
            content_source: y_prompt::ContentSource::Inline("content".into()),
            token_budget: 200,
            priority: 100,
            condition: Some(y_prompt::SectionCondition::HasTool("specific_tool".into())),
            category: y_prompt::SectionCategory::Identity,
        });

        let template = y_prompt::PromptTemplate {
            id: "test".into(),
            parent: None,
            sections: vec![y_prompt::SectionRef {
                section_id: "only".into(),
                priority_override: None,
                condition_override: None,
                enabled: true,
            }],
            mode_overlays: std::collections::HashMap::new(),
            total_token_budget: 4000,
        };

        let provider = BuildSystemPromptProvider::new(
            template,
            store,
            Arc::new(RwLock::new(empty_ctx)),
            SystemPromptConfig {
                fallback_prompt: "I am the fallback.".into(),
                ..Default::default()
            },
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items[0].content, "I am the fallback.");
    }

    #[tokio::test]
    async fn test_feature_flag_disabled_uses_fallback() {
        let provider = make_provider(
            general_ctx(),
            SystemPromptConfig {
                prompt_templates_enabled: false,
                fallback_prompt: "Fallback prompt only.".into(),
            },
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        assert_eq!(ctx.items[0].content, "Fallback prompt only.");
    }

    #[tokio::test]
    async fn test_custom_prompt_replaces_behavioral_sections_but_keeps_functional_sections() {
        let mut prompt_ctx = general_ctx();
        prompt_ctx.custom_system_prompt = Some("Custom session rules.".into());

        let provider = make_provider(prompt_ctx, SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("Custom session rules."));
        assert!(!content.contains("Guidelines"));
        assert!(!content.contains("Security rules"));
        assert!(content.contains("Tool Usage Protocol"));
    }

    #[tokio::test]
    async fn test_custom_prompt_respects_total_budget() {
        let mut store = SectionStore::new();
        store.register(y_prompt::PromptSection {
            id: "tool_protocol".into(),
            content_source: y_prompt::ContentSource::Inline("protocol".into()),
            token_budget: 50,
            priority: 200,
            condition: Some(y_prompt::SectionCondition::Always),
            category: y_prompt::SectionCategory::Behavioral,
        });

        let template = y_prompt::PromptTemplate {
            id: "test".into(),
            parent: None,
            sections: vec![y_prompt::SectionRef {
                section_id: "tool_protocol".into(),
                priority_override: None,
                condition_override: None,
                enabled: true,
            }],
            mode_overlays: std::collections::HashMap::new(),
            total_token_budget: 20,
        };

        let prompt_ctx = PromptContext {
            custom_system_prompt: Some("X".repeat(500)),
            ..general_ctx()
        };

        let provider = BuildSystemPromptProvider::new(
            template,
            store,
            Arc::new(RwLock::new(prompt_ctx)),
            SystemPromptConfig::default(),
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert!(ctx.items[0].content.contains("[truncated]"));
        assert!(ctx.items[0].token_estimate <= 20);
    }

    #[tokio::test]
    async fn test_mode_overlay_applied() {
        // Explore mode excludes security, includes exploration.
        let explore_ctx = PromptContext {
            agent_mode: "explore".into(),
            active_skills: vec![],
            available_tools: vec!["FileRead".into()],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
            custom_system_prompt: None,
            selected_prompt_sections: None,
            mcp_server_instructions: None,
        };
        let provider = make_provider(explore_ctx, SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(!content.contains("Security rules"));
        assert!(content.contains("exploration mode"));
    }

    #[tokio::test]
    async fn test_selected_prompt_sections_filter_builtins() {
        let mut prompt_ctx = general_ctx();
        prompt_ctx.selected_prompt_sections =
            Some(vec!["core.identity".into(), "core.tool_protocol".into()]);

        let provider = make_provider(prompt_ctx, SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("You are y-agent"));
        assert!(content.contains("Tool Usage Protocol"));
        assert!(!content.contains("Guidelines"));
        assert!(!content.contains("Security rules"));
    }

    #[tokio::test]
    async fn test_selected_prompt_sections_keep_active_plan_mode_hint() {
        let mut prompt_ctx = general_ctx();
        prompt_ctx
            .config_flags
            .insert("plan_mode.active".into(), true);
        prompt_ctx.selected_prompt_sections = Some(vec!["core.identity".into()]);

        let provider = make_provider(prompt_ctx, SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("## Plan Mode"));
        assert!(content.contains("request (required)"));
    }

    #[tokio::test]
    async fn test_dynamic_datetime_replaced() {
        let provider = make_provider(general_ctx(), SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        // Should NOT contain the placeholder.
        assert!(!content.contains("{{datetime}}"));
        // Should contain actual date info.
        assert!(content.contains("Current date and time:"));
    }

    #[tokio::test]
    async fn test_missing_section_skipped() {
        // Template references a section that doesn't exist in the store.
        let store = SectionStore::new(); // Empty store.
        let template = y_prompt::PromptTemplate {
            id: "test".into(),
            parent: None,
            sections: vec![y_prompt::SectionRef {
                section_id: "nonexistent".into(),
                priority_override: None,
                condition_override: None,
                enabled: true,
            }],
            mode_overlays: std::collections::HashMap::new(),
            total_token_budget: 4000,
        };

        let provider = BuildSystemPromptProvider::new(
            template,
            store,
            Arc::new(RwLock::new(general_ctx())),
            SystemPromptConfig {
                fallback_prompt: "fallback".into(),
                ..Default::default()
            },
        );

        let mut ctx = AssembledContext::default();
        // Should not error — just skip missing sections and use fallback.
        provider.provide(&mut ctx).await.unwrap();
        assert_eq!(ctx.items[0].content, "fallback");
    }

    // --- Stable cache tests ---

    #[tokio::test]
    async fn test_stable_cache_hit_on_second_call() {
        let provider = make_provider(general_ctx(), SystemPromptConfig::default());
        let mut ctx1 = AssembledContext::default();
        provider.provide(&mut ctx1).await.unwrap();
        let content1 = ctx1.items[0].content.clone();

        let mut ctx2 = AssembledContext::default();
        provider.provide(&mut ctx2).await.unwrap();
        let content2 = ctx2.items[0].content.clone();

        // Second call should produce the same stable content (cache hit).
        // The volatile sections (datetime) may differ, but the stable prefix
        // should be identical.
        assert!(
            content2.starts_with(&content1[..content1.len().min(100)]),
            "stable prefix should be identical on cache hit"
        );
    }

    #[tokio::test]
    async fn test_stable_cache_invalidation() {
        let provider = make_provider(general_ctx(), SystemPromptConfig::default());
        let mut ctx1 = AssembledContext::default();
        provider.provide(&mut ctx1).await.unwrap();

        // Invalidate cache.
        provider.invalidate_stable_cache().await;

        let mut ctx2 = AssembledContext::default();
        provider.provide(&mut ctx2).await.unwrap();

        // Should still produce valid output after invalidation.
        assert!(!ctx2.items[0].content.is_empty());
    }

    #[tokio::test]
    async fn test_volatile_sections_rebuilt_each_call() {
        let provider = make_provider(general_ctx(), SystemPromptConfig::default());
        let mut ctx1 = AssembledContext::default();
        provider.provide(&mut ctx1).await.unwrap();

        // Wait a moment so datetime might change (at least the second call
        // runs at a different time).
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

        let mut ctx2 = AssembledContext::default();
        provider.provide(&mut ctx2).await.unwrap();

        // Both should contain datetime (volatile section is present).
        assert!(ctx1.items[0].content.contains("Current date"));
        assert!(ctx2.items[0].content.contains("Current date"));
    }

    #[tokio::test]
    async fn test_is_volatile_section() {
        assert!(is_volatile_section("core.datetime"));
        assert!(is_volatile_section("core.environment"));
        assert!(is_volatile_section("core.mcp_hint"));
        assert!(!is_volatile_section("core.identity"));
        assert!(!is_volatile_section("core.guidelines"));
        assert!(!is_volatile_section("core.orchestration"));
    }

    #[tokio::test]
    async fn test_compute_cache_key_changes_with_mode() {
        use y_prompt::PromptContext;
        let mut ctx = PromptContext::default();
        ctx.agent_mode = "general".into();
        let key1 = compute_cache_key(&ctx, "");

        ctx.agent_mode = "plan".into();
        let key2 = compute_cache_key(&ctx, "");

        assert_ne!(key1, key2, "cache key should differ for different modes");
    }

    #[tokio::test]
    async fn test_compute_cache_key_changes_with_flags() {
        use y_prompt::PromptContext;
        let ctx1 = PromptContext::default();
        let key1 = compute_cache_key(&ctx1, "");

        let mut ctx2 = PromptContext::default();
        ctx2.config_flags.insert("plan_mode.active".into(), true);
        let key2 = compute_cache_key(&ctx2, "");

        assert_ne!(key1, key2, "cache key should differ for different flags");
    }

    #[tokio::test]
    async fn test_compute_cache_key_same_inputs_same_key() {
        use y_prompt::PromptContext;
        let ctx = PromptContext::default();
        let key1 = compute_cache_key(&ctx, "agents text");
        let key2 = compute_cache_key(&ctx, "agents text");
        assert_eq!(key1, key2, "same inputs should produce same key");
    }
}
