use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::tempdir;
use y_core::runtime::{CommandRunner, ExecutionResult, ResourceUsage, RuntimeError};
use y_core::tool::{ToolInput, ToolType};
use y_core::types::{SessionId, ToolName};
use y_service::dynamic_tool_service::{
    DynamicToolCreateRequest, DynamicToolService, DynamicToolUpdateRequest,
};
use y_tools::config::ToolRegistryConfig;
use y_tools::registry::ToolRegistryImpl;

#[derive(Debug)]
struct RecordingRunner {
    commands: Mutex<Vec<String>>,
}

#[async_trait]
impl CommandRunner for RecordingRunner {
    async fn run_command(
        &self,
        command: &str,
        _working_dir: Option<&str>,
        _timeout: Duration,
    ) -> Result<ExecutionResult, RuntimeError> {
        self.commands.lock().unwrap().push(command.to_string());
        Ok(ExecutionResult {
            exit_code: 0,
            stdout: b"dynamic-ok".to_vec(),
            stderr: Vec::new(),
            duration: Duration::from_millis(1),
            resource_usage: ResourceUsage::default(),
        })
    }
}

fn make_registry() -> ToolRegistryImpl {
    ToolRegistryImpl::new(ToolRegistryConfig {
        allow_dynamic_tools: true,
        ..ToolRegistryConfig::default()
    })
}

fn create_request(name: &str) -> DynamicToolCreateRequest {
    DynamicToolCreateRequest {
        name: name.to_string(),
        description: "Echo structured input".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false
        }),
        interpreter: "bash".to_string(),
        source: "read input; printf '%s' \"$input\"".to_string(),
    }
}

#[tokio::test]
async fn lifecycle_persists_rehydrates_executes_updates_and_deletes() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-tools.jsonl");
    let registry = make_registry();
    let service = DynamicToolService::open(&journal, &registry).await.unwrap();

    let created = service
        .create(&registry, create_request("RuntimeEcho"), "root-agent")
        .await
        .unwrap();
    assert_eq!(created.version, 1);
    let definition = registry
        .get_definition(&ToolName::from_string("RuntimeEcho"))
        .await
        .unwrap();
    assert_eq!(definition.tool_type, ToolType::Dynamic);
    assert!(definition.is_dangerous);

    let runner = Arc::new(RecordingRunner {
        commands: Mutex::new(Vec::new()),
    });
    let output = registry
        .get_tool(&ToolName::from_string("RuntimeEcho"))
        .await
        .unwrap()
        .execute(ToolInput {
            call_id: "call-1".to_string(),
            name: ToolName::from_string("RuntimeEcho"),
            arguments: serde_json::json!({"value": "hello"}),
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: Some(runner.clone()),
        })
        .await
        .unwrap();
    assert!(output.success);
    assert_eq!(output.content["stdout"], "dynamic-ok");
    assert_eq!(runner.commands.lock().unwrap().len(), 1);

    drop(service);
    let rehydrated_registry = make_registry();
    let rehydrated = DynamicToolService::open(&journal, &rehydrated_registry)
        .await
        .unwrap();
    assert!(rehydrated_registry
        .get_tool(&ToolName::from_string("RuntimeEcho"))
        .await
        .is_some());

    let updated = rehydrated
        .update(
            &rehydrated_registry,
            DynamicToolUpdateRequest {
                name: "RuntimeEcho".to_string(),
                description: Some("Echo version two".to_string()),
                parameters: None,
                interpreter: None,
                source: Some("printf 'v2'".to_string()),
            },
            "root-agent",
        )
        .await
        .unwrap();
    assert_eq!(updated.version, 2);

    rehydrated
        .delete(
            &rehydrated_registry,
            "RuntimeEcho",
            "root-agent",
            "obsolete",
        )
        .await
        .unwrap();
    assert!(rehydrated_registry
        .get_tool(&ToolName::from_string("RuntimeEcho"))
        .await
        .is_none());

    drop(rehydrated);
    let final_registry = make_registry();
    let final_service = DynamicToolService::open(&journal, &final_registry)
        .await
        .unwrap();
    assert!(final_service.list(None).await.is_empty());
    assert!(final_registry
        .get_tool(&ToolName::from_string("RuntimeEcho"))
        .await
        .is_none());
}

#[tokio::test]
async fn rejects_registry_name_collisions_and_recovers_a_truncated_tail() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-tools.jsonl");
    let registry = make_registry();
    y_tools::builtin::register_builtin_tools(
        &registry,
        y_browser::BrowserConfig::default(),
        None,
        None,
    )
    .await;
    let service = DynamicToolService::open(&journal, &registry).await.unwrap();
    assert!(service
        .create(&registry, create_request("FileRead"), "root-agent")
        .await
        .is_err());
    service
        .create(&registry, create_request("RuntimeEcho"), "root-agent")
        .await
        .unwrap();
    drop(service);

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap();
    file.write_all(b"{\"operation\":").unwrap();
    file.sync_all().unwrap();

    let recovered_registry = make_registry();
    let recovered = DynamicToolService::open(&journal, &recovered_registry)
        .await
        .unwrap();
    assert_eq!(recovered.list(None).await.len(), 1);
    assert!(recovered_registry
        .get_tool(&ToolName::from_string("RuntimeEcho"))
        .await
        .is_some());
}
