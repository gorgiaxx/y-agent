use y_core::tool::{Tool, ToolCategory, ToolInput};
use y_core::types::{SessionId, ToolName};
use y_tools::builtin::dynamic_tool_management::{
    ToolCreateTool, ToolDeleteTool, ToolGetTool, ToolListTool, ToolUpdateTool,
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
fn exposes_dynamic_tool_lifecycle_definitions_with_safe_risk_tiers() {
    let definitions = [
        ToolCreateTool::tool_definition(),
        ToolUpdateTool::tool_definition(),
        ToolDeleteTool::tool_definition(),
        ToolGetTool::tool_definition(),
        ToolListTool::tool_definition(),
    ];
    assert_eq!(
        definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        [
            "ToolCreate",
            "ToolUpdate",
            "ToolDelete",
            "ToolGet",
            "ToolList"
        ]
    );
    assert!(definitions
        .iter()
        .all(|definition| definition.category == ToolCategory::Custom));
    assert!(definitions[..3]
        .iter()
        .all(|definition| definition.is_dangerous));
    assert!(definitions[3..]
        .iter()
        .all(|definition| !definition.is_dangerous));
}

#[tokio::test]
async fn mutation_proxies_validate_required_arguments() {
    assert!(ToolCreateTool::new()
        .execute(input("ToolCreate", serde_json::json!({})))
        .await
        .is_err());
    assert!(ToolUpdateTool::new()
        .execute(input("ToolUpdate", serde_json::json!({})))
        .await
        .is_err());
    assert!(ToolDeleteTool::new()
        .execute(input("ToolDelete", serde_json::json!({})))
        .await
        .is_err());
    assert!(ToolGetTool::new()
        .execute(input("ToolGet", serde_json::json!({})))
        .await
        .is_err());
    assert!(
        ToolListTool::new()
            .execute(input("ToolList", serde_json::json!({})))
            .await
            .unwrap()
            .success
    );
}
