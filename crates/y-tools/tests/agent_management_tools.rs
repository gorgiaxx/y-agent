use y_core::tool::{Tool, ToolCategory, ToolInput};
use y_core::types::{SessionId, ToolName};
use y_tools::builtin::agent_management::{
    AgentCreateTool, AgentDeactivateTool, AgentEvaluateTool, AgentProposalDecideTool,
    AgentProposalListTool, AgentProposalRefineTool, AgentSearchTool, AgentUpdateTool,
};

fn input(name: &str, arguments: serde_json::Value) -> ToolInput {
    ToolInput {
        call_id: "call-1".to_string(),
        name: ToolName::from_string(name),
        arguments,
        session_id: SessionId::new(),
        working_dir: None,
        additional_read_dirs: vec![],
        command_runner: None,
    }
}

#[test]
fn exposes_all_dynamic_agent_lifecycle_definitions() {
    let definitions = [
        AgentCreateTool::tool_definition(),
        AgentUpdateTool::tool_definition(),
        AgentDeactivateTool::tool_definition(),
        AgentSearchTool::tool_definition(),
        AgentEvaluateTool::tool_definition(),
        AgentProposalListTool::tool_definition(),
        AgentProposalRefineTool::tool_definition(),
        AgentProposalDecideTool::tool_definition(),
    ];
    let names: Vec<_> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "AgentCreate",
            "AgentUpdate",
            "AgentDeactivate",
            "AgentSearch",
            "AgentEvaluate",
            "AgentProposalList",
            "AgentProposalRefine",
            "AgentProposalDecide"
        ]
    );
    assert!(definitions
        .iter()
        .all(|definition| definition.category == ToolCategory::Agent));
}

#[tokio::test]
async fn create_proxy_requires_identity_and_purpose() {
    let tool = AgentCreateTool::new();
    let error = tool
        .execute(input("AgentCreate", serde_json::json!({})))
        .await;
    assert!(error.is_err());

    let output = tool
        .execute(input(
            "AgentCreate",
            serde_json::json!({
                "name": "code-scout",
                "description": "Find implementation evidence"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(output.content["action"], "AgentCreate");
}

#[tokio::test]
async fn update_deactivate_and_search_proxies_validate_required_fields() {
    assert!(AgentUpdateTool::new()
        .execute(input("AgentUpdate", serde_json::json!({})))
        .await
        .is_err());
    assert!(AgentDeactivateTool::new()
        .execute(input(
            "AgentDeactivate",
            serde_json::json!({"id": "dyn-code-scout"})
        ))
        .await
        .is_err());
    assert!(
        AgentSearchTool::new()
            .execute(input("AgentSearch", serde_json::json!({})))
            .await
            .unwrap()
            .success
    );
    assert!(
        AgentEvaluateTool::new()
            .execute(input("AgentEvaluate", serde_json::json!({})))
            .await
            .unwrap()
            .success
    );
    assert!(
        AgentProposalListTool::new()
            .execute(input("AgentProposalList", serde_json::json!({})))
            .await
            .unwrap()
            .success
    );
    assert!(AgentProposalDecideTool::new()
        .execute(input("AgentProposalDecide", serde_json::json!({})))
        .await
        .is_err());
    assert!(AgentProposalRefineTool::new()
        .execute(input("AgentProposalRefine", serde_json::json!({})))
        .await
        .is_err());
}
