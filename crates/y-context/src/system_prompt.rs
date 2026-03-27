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
        }
    }

    /// Create a new system prompt provider with virtual environment info.
    pub fn with_venv_info(
        template: PromptTemplate,
        store: SectionStore,
        prompt_context: Arc<RwLock<PromptContext>>,
        config: SystemPromptConfig,
        venv_info: VenvPromptInfo,
    ) -> Self {
        Self {
            template,
            store: RwLock::new(store),
            prompt_context,
            config,
            venv_info,
            prompts_dir: None,
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
        let new_store = builtin_section_store_with_overrides(self.prompts_dir.as_deref());
        let mut guard = self.store.write().await;
        *guard = new_store;
        tracing::info!("Prompt section store hot-reloaded");
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
    /// `cwd` always reflects the process working directory.
    /// When `workspace_path` is provided (e.g. from a workspace-bound
    /// session), it is appended as an extra field.
    fn generate_environment(workspace_path: Option<&str>, venv_info: &VenvPromptInfo) -> String {
        use std::fmt::Write;

        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let cwd = std::env::current_dir()
            .map_or_else(|_| "(unknown)".into(), |p| p.display().to_string());
        tracing::debug!(workspace_path = ?workspace_path, cwd = %cwd, "generate_environment");

        let mut env_str = match workspace_path.filter(|s| !s.is_empty()) {
            Some(ws) => {
                format!("Environment: OS={os}, arch={arch}, workspace_path={ws}, shell_cwd(optional)={cwd}")
            }
            None => format!("Environment: OS={os}, arch={arch}, shell_cwd={cwd}"),
        };

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
        let mode = &prompt_ctx.agent_mode;
        let effective_sections = self.template.effective_sections(mode);
        let total_budget = self.template.effective_budget(mode);

        let mut accumulated = String::new();
        let mut cumulative_tokens: u32 = 0;

        // Resolve sections sorted by their effective priority.
        // PromptSection.priority is the canonical order; overlay priority_override
        // takes precedence when present.
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

        for (eff, section, _priority) in &section_entries {
            // Evaluate condition.
            let condition = eff
                .condition_override
                .as_ref()
                .or(section.condition.as_ref());
            if let Some(cond) = condition {
                if !cond.evaluate(&prompt_ctx) {
                    continue;
                }
            }

            // Load content (lazy).
            let content = match store.load_content(&eff.section_id) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        section = %eff.section_id,
                        error = %e,
                        "failed to load section content; skipping"
                    );
                    continue;
                }
            };

            // Dynamic section replacement.
            let content = match eff.section_id.as_str() {
                "core.datetime" => Self::generate_datetime(),
                "core.environment" => Self::generate_environment(
                    prompt_ctx.working_directory.as_deref(),
                    &self.venv_info,
                ),
                _ => content,
            };

            // Per-section token budget.
            let (content, truncated) = truncate_to_budget(&content, section.token_budget);
            if truncated {
                tracing::debug!(
                    section = %eff.section_id,
                    budget = section.token_budget,
                    "section content truncated to fit budget"
                );
            }

            let tokens = estimate_tokens(&content);

            // Total budget check: stop adding if we'd exceed.
            if cumulative_tokens + tokens > total_budget {
                tracing::debug!(
                    section = %eff.section_id,
                    cumulative = cumulative_tokens,
                    section_tokens = tokens,
                    total_budget = total_budget,
                    "dropping section: total token budget exceeded"
                );
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
            available_tools: vec!["file_read".into()],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
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
        // core.tool_protocol requires ConfigFlag("tool_calling.prompt_based")
        // which is not set in general_ctx(), so it should be excluded.
        assert!(!item.content.contains("Tool Behavior"));
        // Token estimate should be reasonable.
        assert!(item.token_estimate > 0);
    }

    #[tokio::test]
    async fn test_conditions_exclude_sections() {
        // Plan mode excludes core.tool_behavior via overlay.
        let plan_ctx = PromptContext {
            agent_mode: "plan".into(),
            active_skills: vec![],
            available_tools: vec!["file_read".into()],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
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
    async fn test_mode_overlay_applied() {
        // Explore mode excludes security, includes exploration.
        let explore_ctx = PromptContext {
            agent_mode: "explore".into(),
            active_skills: vec![],
            available_tools: vec!["file_read".into()],
            config_flags: std::collections::HashMap::new(),
            working_directory: None,
        };
        let provider = make_provider(explore_ctx, SystemPromptConfig::default());
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(!content.contains("Security rules"));
        assert!(content.contains("exploration mode"));
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
}
