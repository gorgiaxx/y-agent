#![cfg(feature = "agent_swarm")]

use y_core::tool::{Tool, ToolInput};
use y_core::types::{SessionId, ToolName};
use y_tools::builtin::agent_swarm::{
    prepare_agent_swarm, AgentSwarmTool, MAX_AGENT_SWARM_TASKS, PROMPT_TEMPLATE_PLACEHOLDER,
};

#[test]
fn test_agent_swarm_definition_exposes_bounded_map_contract() {
    let definition = AgentSwarmTool::tool_definition();

    assert_eq!(definition.name.as_str(), "AgentSwarm");
    assert_eq!(
        definition.parameters["properties"]["items"]["maxItems"],
        serde_json::json!(MAX_AGENT_SWARM_TASKS)
    );
    assert_eq!(
        definition.parameters["properties"]["prompt_template"]["description"],
        format!("Prompt template containing {PROMPT_TEMPLATE_PLACEHOLDER}")
    );
    assert_eq!(
        definition.parameters["required"],
        serde_json::json!(["description", "agent_name", "prompt_template", "items"])
    );
}

#[test]
fn test_prepare_agent_swarm_materializes_distinct_prompts_in_input_order() {
    let prepared = prepare_agent_swarm(&serde_json::json!({
        "description": "Review modules",
        "agent_name": "general-purpose",
        "prompt_template": "Review {{item}} and compare {{item}} with its tests",
        "items": ["src/a.rs", "src/b.rs"]
    }))
    .expect("valid swarm input");

    assert_eq!(prepared.description, "Review modules");
    assert_eq!(prepared.agent_name, "general-purpose");
    assert_eq!(prepared.tasks[0].index, 0);
    assert_eq!(prepared.tasks[0].item, "src/a.rs");
    assert_eq!(
        prepared.tasks[0].prompt,
        "Review src/a.rs and compare src/a.rs with its tests"
    );
    assert_eq!(prepared.tasks[1].item, "src/b.rs");
}

#[test]
fn test_prepare_agent_swarm_rejects_duplicate_rendered_prompts_before_dispatch() {
    let error = prepare_agent_swarm(&serde_json::json!({
        "description": "Review modules",
        "agent_name": "general-purpose",
        "prompt_template": "Review {{item}}",
        "items": ["src/a.rs", "src/a.rs"]
    }))
    .expect_err("duplicate prompts must be rejected");

    assert!(error.to_string().contains("duplicate rendered prompt"));
}

#[test]
fn test_prepare_agent_swarm_rejects_batches_above_hard_limit() {
    let items = (0..=MAX_AGENT_SWARM_TASKS)
        .map(|index| format!("src/{index}.rs"))
        .collect::<Vec<_>>();
    let error = prepare_agent_swarm(&serde_json::json!({
        "description": "Review modules",
        "agent_name": "general-purpose",
        "prompt_template": "Review {{item}}",
        "items": items
    }))
    .expect_err("oversized batch must be rejected");

    assert!(error.to_string().contains("at most"));
}

#[test]
fn test_prepare_agent_swarm_rejects_invalid_shared_options_before_dispatch() {
    let error = prepare_agent_swarm(&serde_json::json!({
        "description": "Review modules",
        "agent_name": "general-purpose",
        "prompt_template": "Review {{item}}",
        "items": ["src/a.rs", "src/b.rs"],
        "context_strategy": "everything"
    }))
    .expect_err("invalid shared options must reject the batch");

    assert!(error.to_string().contains("context_strategy"));
}

#[tokio::test]
async fn test_agent_swarm_tool_returns_pending_descriptor_after_validation() {
    let output = AgentSwarmTool::new()
        .execute(ToolInput {
            call_id: "swarm-call".to_string(),
            name: ToolName::from_string("AgentSwarm"),
            arguments: serde_json::json!({
                "description": "Review modules",
                "agent_name": "general-purpose",
                "prompt_template": "Review {{item}}",
                "items": ["src/a.rs", "src/b.rs"]
            }),
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: Vec::new(),
            command_runner: None,
        })
        .await
        .expect("valid swarm descriptor");

    assert!(output.success);
    assert_eq!(output.content["action"], "delegate_swarm");
    assert_eq!(output.content["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(output.content["status"], "pending");
}
