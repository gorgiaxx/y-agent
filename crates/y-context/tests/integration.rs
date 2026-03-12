//! Integration tests for the full context preparation flow.
//!
//! Tests the `ContextManager` facade end-to-end, verifying pipeline
//! execution, guard evaluation, and overflow recovery with realistic
//! provider configurations.

use y_context::compaction::CompactionEngine;
use y_context::context_manager::ContextManager;
use y_context::guard::{ContextWindowGuard, GuardVerdict, TokenBudget};
use y_context::inject_bootstrap::{BootstrapEntry, InjectBootstrap};
use y_context::inject_skills::{InjectSkills, SkillSummary};
use y_context::inject_tools::InjectTools;
use y_context::load_history::LoadHistory;
use y_context::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipeline, ContextPipelineError,
    ContextProvider, ContextRequest,
};
use y_context::repair::HistoryMessage;

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn msg(id: &str, role: &str, content: &str) -> HistoryMessage {
    HistoryMessage {
        id: id.into(),
        role: role.into(),
        content: content.into(),
        tool_call_id: None,
    }
}

fn sample_request() -> ContextRequest {
    ContextRequest {
        session_id: Some(y_core::types::SessionId::from_string("test-session")),
        user_query: "How do I fix this bug?".into(),
        agent_mode: "general".into(),
        tools_enabled: vec!["read_file".into(), "write_file".into()],
    }
}

// ---------------------------------------------------------------------------
// T-INT-01: Full pipeline with all providers
// ---------------------------------------------------------------------------

/// The full context preparation flow executes all providers in order
/// and returns a valid `PreparedContext`.
#[tokio::test]
async fn test_full_pipeline_all_providers() {
    let mut pipeline = ContextPipeline::new();

    // Register providers in correct priority order.
    pipeline.register(Box::new(InjectBootstrap::new(vec![BootstrapEntry {
        label: "README.md".into(),
        content: "# Test Project\nA Rust project for testing.".into(),
    }])));

    pipeline.register(Box::new(InjectSkills::new(vec![SkillSummary {
        name: "code_review".into(),
        description: "Reviews code for best practices.".into(),
        triggers: vec!["review".into()],
    }])));

    pipeline.register(Box::new(InjectTools::new(vec![
        "read_file".into(),
        "write_file".into(),
        "run_command".into(),
    ])));

    pipeline.register(Box::new(LoadHistory::new(vec![
        msg("1", "user", "Hello"),
        msg("2", "assistant", "Hi there! How can I help?"),
        msg("3", "user", "How do I fix this bug?"),
    ])));

    let manager = ContextManager::with_components(
        pipeline,
        ContextWindowGuard::new(),
        CompactionEngine::new(),
    );

    let result = manager.prepare_context(sample_request()).await.unwrap();

    // Verify all categories are present.
    let categories: Vec<ContextCategory> =
        result.assembled.items.iter().map(|i| i.category).collect();

    assert!(categories.contains(&ContextCategory::Bootstrap));
    assert!(categories.contains(&ContextCategory::Skills));
    assert!(categories.contains(&ContextCategory::Tools));
    assert!(categories.contains(&ContextCategory::History));

    // Verify token count is reasonable.
    assert!(result.tokens_used > 0);
    assert_eq!(result.verdict, GuardVerdict::Ok);
    assert!(!result.compacted);

    // Verify request context was preserved.
    assert!(result.assembled.request.is_some());
    let req = result.assembled.request.unwrap();
    assert_eq!(req.user_query, "How do I fix this bug?");
}

// ---------------------------------------------------------------------------
// T-INT-02: Empty pipeline produces empty context
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_empty_pipeline() {
    let manager = ContextManager::new();
    let result = manager
        .prepare_context(ContextRequest::default())
        .await
        .unwrap();

    assert!(result.assembled.items.is_empty());
    assert_eq!(result.tokens_used, 0);
    assert_eq!(result.verdict, GuardVerdict::Ok);
}

// ---------------------------------------------------------------------------
// T-INT-03: Guard warning/overflow responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_guard_warning_threshold() {
    let budget = TokenBudget {
        system_prompt: 100,
        tools_schema: 100,
        history: 600,
        bootstrap: 100,
        response_reserve: 100,
    };
    // total_available = 100+100+600+100 = 900
    let guard = ContextWindowGuard {
        budget,
        ..ContextWindowGuard::new()
    };

    let mut pipeline = ContextPipeline::new();
    // Fill ~87% to trigger warning or overflow.
    pipeline.register(Box::new(FixedTokenProvider {
        name: "big_history",
        priority: 600,
        category: ContextCategory::History,
        tokens: 780, // 87% of 900
    }));

    let manager = ContextManager::with_components(pipeline, guard, CompactionEngine::new());

    let result = manager
        .prepare_context(ContextRequest::default())
        .await
        .unwrap();

    match &result.verdict {
        GuardVerdict::Warning { utilization_pct } => {
            assert!(*utilization_pct > 70);
        }
        GuardVerdict::Overflow { .. } => {
            // Also acceptable — overflow was detected and recovery attempted.
        }
        verdict => panic!("expected Warning or Overflow, got {verdict:?}"),
    }
}

// ---------------------------------------------------------------------------
// T-INT-04: Overflow triggers recovery cascade
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_overflow_recovery_cascade() {
    let budget = TokenBudget {
        system_prompt: 100,
        tools_schema: 100,
        history: 500,
        bootstrap: 100,
        response_reserve: 100,
    };
    // total_available = 100+100+500+100 = 800
    let guard = ContextWindowGuard {
        budget,
        ..ContextWindowGuard::new()
    };

    let mut pipeline = ContextPipeline::new();

    // Add components that collectively overflow the budget.
    pipeline.register(Box::new(FixedTokenProvider {
        name: "bootstrap",
        priority: 200,
        category: ContextCategory::Bootstrap,
        tokens: 200,
    }));
    pipeline.register(Box::new(FixedTokenProvider {
        name: "tools",
        priority: 500,
        category: ContextCategory::Tools,
        tokens: 200,
    }));
    // Many history items to trigger compaction.
    for i in 0..15 {
        pipeline.register(Box::new(FixedTokenProvider {
            name: Box::leak(format!("history_{i}").into_boxed_str()),
            priority: 600,
            category: ContextCategory::History,
            tokens: 100,
        }));
    }
    // Total: 200 + 200 + 15*100 = 1900 tokens (190% of budget)

    let manager = ContextManager::with_components(pipeline, guard, CompactionEngine::new());

    let result = manager
        .prepare_context(ContextRequest::default())
        .await
        .unwrap();

    // Recovery should have been attempted.
    assert!(result.compacted);
}

// ---------------------------------------------------------------------------
// T-INT-05: History repair in full pipeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_history_repair_in_pipeline() {
    let mut pipeline = ContextPipeline::new();

    pipeline.register(Box::new(LoadHistory::new(vec![
        msg("1", "system", "You are an AI assistant."),
        msg("2", "system", "Duplicate system message."), // Will be removed.
        msg("3", "user", "Hello"),
        msg("4", "assistant", ""), // Empty, will be removed.
        msg("5", "assistant", "Hi there!"),
    ])));

    let manager = ContextManager::with_components(
        pipeline,
        ContextWindowGuard::new(),
        CompactionEngine::new(),
    );

    let result = manager
        .prepare_context(ContextRequest::default())
        .await
        .unwrap();

    // After repair: 1 system + 1 user + 1 assistant = 3 messages.
    let history_items: Vec<&ContextItem> = result
        .assembled
        .items
        .iter()
        .filter(|i| i.category == ContextCategory::History)
        .collect();

    assert_eq!(history_items.len(), 3);
}

// ---------------------------------------------------------------------------
// T-INT-06: Failed provider doesn't break pipeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_failed_provider_resilience() {
    struct BrokenProvider;

    #[async_trait]
    impl ContextProvider for BrokenProvider {
        fn name(&self) -> &'static str {
            "broken"
        }
        fn priority(&self) -> u32 {
            300
        }
        async fn provide(&self, _ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
            Err(ContextPipelineError::ProviderFailed {
                name: "broken".into(),
                message: "something went wrong".into(),
            })
        }
    }

    let mut pipeline = ContextPipeline::new();
    pipeline.register(Box::new(InjectBootstrap::new(vec![BootstrapEntry {
        label: "README.md".into(),
        content: "test".into(),
    }])));
    pipeline.register(Box::new(BrokenProvider));
    pipeline.register(Box::new(InjectTools::new(vec!["read_file".into()])));

    let manager = ContextManager::with_components(
        pipeline,
        ContextWindowGuard::new(),
        CompactionEngine::new(),
    );

    let result = manager
        .prepare_context(ContextRequest::default())
        .await
        .unwrap();

    // Should have bootstrap + tools, but not the broken provider's items.
    assert!(result
        .assembled
        .items
        .iter()
        .any(|i| i.category == ContextCategory::Bootstrap));
    assert!(result
        .assembled
        .items
        .iter()
        .any(|i| i.category == ContextCategory::Tools));
}

// ---------------------------------------------------------------------------
// T-INT-07: Provider priority order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_provider_priority_order() {
    let mut pipeline = ContextPipeline::new();

    // Register in wrong order — pipeline should sort by priority.
    pipeline.register(Box::new(InjectTools::new(vec!["tool1".into()])));
    pipeline.register(Box::new(InjectBootstrap::new(vec![BootstrapEntry {
        label: "readme".into(),
        content: "test".into(),
    }])));

    let manager = ContextManager::with_components(
        pipeline,
        ContextWindowGuard::new(),
        CompactionEngine::new(),
    );

    let result = manager
        .prepare_context(ContextRequest::default())
        .await
        .unwrap();

    // Bootstrap (200) should come before Tools (500).
    let categories: Vec<ContextCategory> =
        result.assembled.items.iter().map(|i| i.category).collect();

    let bootstrap_pos = categories
        .iter()
        .position(|c| *c == ContextCategory::Bootstrap)
        .unwrap();
    let tools_pos = categories
        .iter()
        .position(|c| *c == ContextCategory::Tools)
        .unwrap();

    assert!(
        bootstrap_pos < tools_pos,
        "bootstrap should come before tools"
    );
}

// ---------------------------------------------------------------------------
// Helper: Fixed token provider for integration tests
// ---------------------------------------------------------------------------

struct FixedTokenProvider {
    name: &'static str,
    priority: u32,
    category: ContextCategory,
    tokens: u32,
}

#[async_trait]
impl ContextProvider for FixedTokenProvider {
    fn name(&self) -> &'static str {
        self.name
    }
    fn priority(&self) -> u32 {
        self.priority
    }
    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        ctx.add(ContextItem {
            category: self.category,
            content: format!("[{} content - {} tokens]", self.name, self.tokens),
            token_estimate: self.tokens,
            priority: self.priority,
        });
        Ok(())
    }
}
