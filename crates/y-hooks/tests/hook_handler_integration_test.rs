//! Integration tests for hook handler executor.
//!
//! Tests the complete flow from `HookConfig` → `HookSystem` → handler execution.

use std::collections::HashMap;

use y_core::hook::HookPoint;
use y_hooks::config::{HandlerConfig, HookConfig, HookHandlerGroupConfig};
use y_hooks::hook_handler::{HookDecision, HookInput};
use y_hooks::HookSystem;

#[tokio::test]
async fn test_hook_system_command_handler_e2e() {
    // Configure a command hook that exits 0 (allow).
    let config = HookConfig {
        hook_handlers: {
            let mut m = HashMap::new();
            m.insert(
                "pre_tool_execute".into(),
                vec![HookHandlerGroupConfig {
                    matcher: "*".into(),
                    timeout_ms: Some(5000),
                    handlers: vec![HandlerConfig::Command {
                        command: "/bin/true".into(),
                        r#async: false,
                    }],
                }],
            );
            m
        },
        ..HookConfig::default()
    };

    let system = HookSystem::new(&config);

    // Verify executor was created.
    assert!(system.handler_executor().is_some());

    let input = HookInput {
        session_id: Some("test-session".into()),
        hook_event: "pre_tool_execute".into(),
        timestamp: "2026-03-11T00:00:00Z".into(),
        extra: serde_json::json!({ "tool_name": "Bash", "args": {} }),
    };

    let result = system
        .execute_hook_handlers(HookPoint::PreToolExecute, &input)
        .await;

    assert_eq!(result.decision, HookDecision::Allow);
    assert_eq!(result.handler_count, 1);
    assert_eq!(result.block_count, 0);
}

#[tokio::test]
async fn test_hook_system_no_executor_when_disabled() {
    let config = HookConfig {
        handlers_enabled: false,
        hook_handlers: {
            let mut m = HashMap::new();
            m.insert(
                "pre_tool_execute".into(),
                vec![HookHandlerGroupConfig {
                    matcher: "*".into(),
                    timeout_ms: Some(5000),
                    handlers: vec![HandlerConfig::Command {
                        command: "/bin/true".into(),
                        r#async: false,
                    }],
                }],
            );
            m
        },
        ..HookConfig::default()
    };

    let system = HookSystem::new(&config);
    assert!(system.handler_executor().is_none());
}

#[tokio::test]
async fn test_hook_system_handler_metrics() {
    let config = HookConfig {
        hook_handlers: {
            let mut m = HashMap::new();
            m.insert(
                "pre_tool_execute".into(),
                vec![HookHandlerGroupConfig {
                    matcher: "*".into(),
                    timeout_ms: Some(5000),
                    handlers: vec![HandlerConfig::Command {
                        command: "/bin/true".into(),
                        r#async: false,
                    }],
                }],
            );
            m
        },
        ..HookConfig::default()
    };

    let system = HookSystem::new(&config);
    let executor = system.handler_executor().unwrap();

    // Initially no invocations.
    let snap = executor.metrics().snapshot();
    assert_eq!(snap.invocations, 0);

    // Execute a hook.
    let input = HookInput {
        session_id: None,
        hook_event: "pre_tool_execute".into(),
        timestamp: "2026-03-11T00:00:00Z".into(),
        extra: serde_json::json!({ "tool_name": "Bash" }),
    };
    system
        .execute_hook_handlers(HookPoint::PreToolExecute, &input)
        .await;

    // Verify metrics updated.
    let snap = executor.metrics().snapshot();
    assert_eq!(snap.invocations, 1);
    assert_eq!(snap.blocks, 0);
    assert_eq!(snap.errors, 0);
    assert!(snap.duration_us > 0);
}
