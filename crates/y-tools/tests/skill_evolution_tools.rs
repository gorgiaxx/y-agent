use y_core::tool::{Tool, ToolCategory, ToolInput};
use y_core::types::{SessionId, ToolName};
use y_tools::builtin::skill_evolution::{
    SkillProposalDecideTool, SkillProposalListTool, SkillProposalRefineTool,
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
fn exposes_governed_skill_evolution_definitions() {
    let definitions = [
        SkillProposalListTool::tool_definition(),
        SkillProposalRefineTool::tool_definition(),
        SkillProposalDecideTool::tool_definition(),
    ];
    let names: Vec<_> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "SkillProposalList",
            "SkillProposalRefine",
            "SkillProposalDecide"
        ]
    );
    assert!(definitions
        .iter()
        .all(|definition| definition.category == ToolCategory::Agent));
    assert!(!definitions[0].is_dangerous);
    assert!(!definitions[1].is_dangerous);
    assert!(definitions[2].is_dangerous);
}

#[tokio::test]
async fn proxies_validate_required_arguments() {
    assert!(
        SkillProposalListTool::new()
            .execute(input("SkillProposalList", serde_json::json!({})))
            .await
            .unwrap()
            .success
    );
    assert!(SkillProposalRefineTool::new()
        .execute(input("SkillProposalRefine", serde_json::json!({})))
        .await
        .is_err());
    assert!(SkillProposalDecideTool::new()
        .execute(input("SkillProposalDecide", serde_json::json!({})))
        .await
        .is_err());
}
