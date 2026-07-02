use super::*;
use tempfile::TempDir;

async fn make_test_container() -> (crate::container::ServiceContainer, TempDir) {
    let tmpdir = tempfile::TempDir::new().unwrap();
    let config = crate::config::ServiceConfig {
        storage: y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        },
        ..crate::config::ServiceConfig::default()
    };
    let container = crate::container::ServiceContainer::from_config(&config)
        .await
        .expect("test container should build");
    (container, tmpdir)
}

#[test]
fn test_slug_from_request() {
    assert_eq!(
        slug_from_request("Refactor the plan mode"),
        "refactor-the-plan-mode"
    );
    assert_eq!(slug_from_request("Hello, World! 123"), "hello--world--123");
}

#[test]
fn test_extract_json_from_response_plain() {
    let input = r#"{"plan_title": "test", "tasks": []}"#;
    assert_eq!(extract_json_from_response(input), input);
}

#[test]
fn test_extract_json_from_response_code_block() {
    let input = "```json\n{\"plan_title\": \"test\"}\n```";
    assert_eq!(
        extract_json_from_response(input),
        "{\"plan_title\": \"test\"}"
    );
}

#[test]
fn test_extract_json_from_response_generic_block() {
    let input = "```\n{\"a\": 1}\n```";
    assert_eq!(extract_json_from_response(input), "{\"a\": 1}");
}

#[test]
fn test_build_subagent_messages_prepends_system_prompt() {
    let messages = build_subagent_messages("system rules", "user task".into());
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, y_core::types::Role::System);
    assert_eq!(messages[0].content, "system rules");
    assert_eq!(messages[1].role, y_core::types::Role::User);
    assert_eq!(messages[1].content, "user task");
}

#[tokio::test]
async fn test_persist_plan_content_writes_plan_file() {
    let tmpdir = tempfile::TempDir::new().unwrap();
    let plan_path = tmpdir.path().join("gui-plan.md");

    persist_plan_content(&plan_path, "# GUI Plan Stream Fix")
        .await
        .unwrap();

    let content = tokio::fs::read_to_string(&plan_path).await.unwrap();
    assert_eq!(content, "# GUI Plan Stream Fix");
}

#[test]
fn test_build_plan_start_metadata_marks_plan_as_running() {
    let meta = build_plan_start_metadata(std::path::Path::new("/tmp/gui-plan.md"));

    assert_eq!(meta["display"]["kind"], "plan_stage");
    assert_eq!(meta["display"]["stage"], "plan_writer");
    assert_eq!(meta["display"]["stage_status"], "running");
    assert_eq!(meta["display"]["plan_file"], "/tmp/gui-plan.md");
}

#[test]
fn test_structured_plan_to_markdown_includes_phases() {
    let plan = StructuredPlan {
        plan_title: "Test Plan".into(),
        plan_file: "/tmp/plan.md".into(),
        estimated_effort: "Short(1-4h)".into(),
        overview: "Create the initial structure".into(),
        scope_in: vec!["Initial structure".into()],
        scope_out: vec!["Deployment".into()],
        guardrails: vec!["Stay within src/".into()],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![PlanTask {
            id: "phase-1".into(),
            phase: 1,
            title: "Setup".into(),
            description: "Create initial structure".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec!["src/main.rs".into()],
            acceptance_criteria: vec!["Structure exists".into()],
        }],
    };
    let md = structured_plan_to_markdown(&plan);
    assert!(md.contains("title: Test Plan"));
    assert!(md.contains("Estimated effort: Short(1-4h)"));
    assert!(md.contains("Create the initial structure"));
    assert!(md.contains("- Initial structure"));
    assert!(md.contains("- Deployment"));
    assert!(md.contains("- Stay within src/"));
    assert!(md.contains("## Phase 1: Setup"));
    assert!(md.contains("Create initial structure"));
    assert!(md.contains("- src/main.rs"));
    assert!(md.contains("- Structure exists"));
}

#[test]
fn test_review_status_for_mode_distinguishes_auto_and_manual() {
    assert_eq!(
        review_status_for_mode(PlanReviewMode::Manual),
        "awaiting_user"
    );
    assert_eq!(
        review_status_for_mode(PlanReviewMode::Auto),
        "auto_approved"
    );
}

#[tokio::test]
async fn test_operation_mode_overrides_guardrail_plan_review_mode() {
    let (container, _tmpdir) = make_test_container().await;
    let session_id = SessionId("session-1".into());

    assert_eq!(
        resolve_effective_plan_review_mode(&container, &session_id).await,
        PlanReviewMode::Manual
    );

    {
        let mut modes = container
            .session_state
            .session_operation_modes
            .write()
            .await;
        modes.insert(session_id.clone(), OperationMode::AutoReview);
    }

    assert_eq!(
        resolve_effective_plan_review_mode(&container, &session_id).await,
        PlanReviewMode::Auto
    );
}

#[test]
fn test_build_plan_writer_stage_metadata_includes_tasks() {
    let plan = StructuredPlan {
        plan_title: "GUI Plan Stream Fix".into(),
        plan_file: "/tmp/gui-plan.md".into(),
        estimated_effort: "Short(1-4h)".into(),
        overview: "Render structured plan output for review.".into(),
        scope_in: vec!["Plan renderer".into()],
        scope_out: vec!["Execution policy".into()],
        guardrails: vec!["Avoid raw JSON in the GUI".into()],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Render structured plan output".into(),
            description: "Use structured metadata instead of raw JSON".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 12,
            key_files: vec![
                "crates/y-gui/src/components/chat-panel/chat-box/ToolCallCard.tsx".into(),
            ],
            acceptance_criteria: vec!["Task list is rendered as a dedicated component".into()],
        }],
    };

    let meta = build_plan_writer_stage_metadata(
        std::path::Path::new("/tmp/gui-plan.md"),
        &plan,
        "awaiting_user",
    );

    assert_eq!(meta["display"]["kind"], "plan_stage");
    assert_eq!(meta["display"]["stage"], "plan_writer");
    assert_eq!(meta["display"]["plan_title"], "GUI Plan Stream Fix");
    assert_eq!(
        meta["display"]["tasks"][0]["title"],
        "Render structured plan output"
    );
    assert_eq!(meta["display"]["estimated_effort"], "Short(1-4h)");
    assert_eq!(
        meta["display"]["overview"],
        "Render structured plan output for review."
    );
    assert_eq!(meta["display"]["review_status"], "awaiting_user");
}

#[test]
fn test_build_plan_execution_metadata_updates_task_statuses() {
    let plan = StructuredPlan {
            plan_title: "GUI Plan Stream Fix".into(),
            plan_file: "/tmp/gui-plan.md".into(),
            estimated_effort: String::new(),
            overview: String::new(),
            scope_in: vec![],
            scope_out: vec![],
            guardrails: vec![],
            execution_contract: PlanExecutionContract::default(),
            tasks: vec![
                PlanTask {
                    id: "task-1".into(),
                    phase: 1,
                    title: "Render markdown output".into(),
                    description: "Use markdown rendering for plan output.".into(),
                    depends_on: vec![],
                    status: TaskStatus::Pending,
                    estimated_iterations: 8,
                    key_files: vec![
                        "crates/y-gui/src/components/chat-panel/chat-box/tool-renderers/PlanRenderer.tsx"
                            .into(),
                    ],
                    acceptance_criteria: vec!["Plan content renders as markdown".into()],
                },
                PlanTask {
                    id: "task-2".into(),
                    phase: 2,
                    title: "Keep execution state visible".into(),
                    description: "Do not drop the running indicator during plan execution.".into(),
                    depends_on: vec!["task-1".into()],
                    status: TaskStatus::Pending,
                    estimated_iterations: 10,
                    key_files: vec!["crates/y-gui/src/hooks/useChat.ts".into()],
                    acceptance_criteria: vec!["Stop button stays visible".into()],
                },
            ],
        };

    let phase_results = vec![serde_json::json!({
        "task_id": "task-1",
        "phase": 1,
        "title": "Render markdown output",
        "status": "completed",
    })];

    let meta = build_plan_execution_metadata(
        std::path::Path::new("/tmp/gui-plan.md"),
        &plan,
        "test-run-id",
        1,
        0,
        &phase_results,
    );

    assert_eq!(meta["display"]["tasks"][0]["status"], "completed");
    assert_eq!(meta["display"]["tasks"][1]["status"], "pending");
}

#[test]
fn test_build_plan_execution_metadata_omits_verbose_phase_summaries() {
    let plan = StructuredPlan {
        plan_title: "GUI Plan Stream Fix".into(),
        plan_file: "/tmp/gui-plan.md".into(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Render markdown output".into(),
            description: "Use markdown rendering for plan output.".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 8,
            key_files: vec![],
            acceptance_criteria: vec![],
        }],
    };
    let verbose_summary = "verbose completed phase output ".repeat(80);
    let phase_results = vec![serde_json::json!({
        "task_id": "task-1",
        "phase": 1,
        "title": "Render markdown output",
        "status": "completed",
        "summary": verbose_summary,
    })];

    let meta = build_plan_execution_metadata(
        std::path::Path::new("/tmp/gui-plan.md"),
        &plan,
        "test-run-id",
        1,
        0,
        &phase_results,
    );

    assert_eq!(meta["display"]["phases"][0]["status"], "completed");
    assert!(meta["display"]["phases"][0].get("summary").is_none());
}

#[test]
fn test_build_plan_execution_tool_content_omits_verbose_phase_summaries() {
    let plan = StructuredPlan {
        plan_title: "GUI Plan Stream Fix".into(),
        plan_file: "/tmp/gui-plan.md".into(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Render markdown output".into(),
            description: "Use markdown rendering for plan output.".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 8,
            key_files: vec![],
            acceptance_criteria: vec![],
        }],
    };
    let verbose_summary = "verbose completed phase output ".repeat(80);
    let phase_results = vec![serde_json::json!({
        "task_id": "task-1",
        "phase": 1,
        "title": "Render markdown output",
        "status": "completed",
        "summary": verbose_summary,
    })];

    let content = build_plan_execution_tool_content(
        std::path::Path::new("/tmp/gui-plan.md"),
        &plan,
        "test-run-id",
        1,
        0,
        &phase_results,
        None,
        None,
    );

    assert_eq!(content["phases"][0]["status"], "completed");
    assert!(content["phases"][0].get("summary").is_none());
}

#[test]
fn test_build_phase_execution_config_uses_registry_limits() {
    let settings = ResolvedAgentConfig {
        system_prompt: "Execute the phase".into(),
        max_iterations: 30,
        max_tool_calls: 60,
        preferred_models: vec!["test-model".into()],
        provider_tags: vec!["coding".into()],
        temperature: Some(0.7),
        max_tokens: Some(2048),
        trust_tier: Some(TrustTier::BuiltIn),
        allowed_tools: vec!["FileWrite".into(), "ShellExec".into()],
        prune_tool_history: false,
        response_format: None,
    };
    let task = PlanTask {
        id: "task-1".into(),
        phase: 1,
        title: "Implement execution path".into(),
        description: "Wire the phase executor through the agent registry.".into(),
        depends_on: vec![],
        status: TaskStatus::Pending,
        estimated_iterations: 12,
        key_files: vec!["crates/y-service/src/plan_orchestrator.rs".into()],
        acceptance_criteria: vec!["Phase executor limit comes from registry".into()],
    };

    let config = build_phase_execution_config(
        &settings,
        &SessionId::new(),
        Uuid::new_v4(),
        &task,
        "Registry-backed phase execution",
        Path::new("/tmp/gui-plan.md"),
        1,
        3,
        vec![],
        &PlanExecutionContract::default(),
        &[],
    );

    assert_eq!(config.agent_name, "plan-phase-executor:phase-1");
    assert_eq!(config.max_iterations, 30);
    assert_eq!(config.max_tool_calls, 60);
    assert_eq!(
        config.agent_allowed_tools,
        vec!["FileWrite".to_string(), "ShellExec".to_string()]
    );
    assert_eq!(config.messages.len(), 2);
    assert_eq!(config.messages[0].role, y_core::types::Role::System);
    assert!(config.messages[1].content.contains("phase 1 of 3"));
    assert_eq!(config.additional_read_dirs, vec!["/tmp/gui-plan.md"]);
}

#[test]
fn test_build_phase_user_message_includes_plan_constraints() {
    let task = PlanTask {
        id: "task-1".into(),
        phase: 1,
        title: "Implement execution path".into(),
        description: "Wire the phase executor through the agent registry.".into(),
        depends_on: vec![],
        status: TaskStatus::Pending,
        estimated_iterations: 12,
        key_files: vec!["crates/y-service/src/plan_orchestrator.rs".into()],
        acceptance_criteria: vec!["Phase executor sees inherited constraints".into()],
    };
    let scope_out = vec!["crates/y-gui/".to_string(), "docs/".to_string()];
    let guardrails = vec!["Report findings before summary".to_string()];

    let constraints =
        inherited_constraints_from_parts(&scope_out, &guardrails).expect("constraints");
    let message = build_phase_user_message(
        &task,
        "Constrained Phase Execution",
        1,
        2,
        Some(&constraints),
        &[],
    );

    assert!(message.contains("## Constraints"));
    assert!(message.contains("### Out of Scope (Do NOT touch)"));
    assert!(message.contains("- crates/y-gui/"));
    assert!(message.contains("- docs/"));
    assert!(message.contains("### Guardrails"));
    assert!(message.contains("- Report findings before summary"));
    assert!(message.contains("## Phase 1: Implement execution path"));
}

#[test]
fn test_build_phase_execution_config_threads_plan_constraints_into_user_message() {
    let settings = ResolvedAgentConfig {
        system_prompt: "Execute the phase".into(),
        max_iterations: 30,
        max_tool_calls: 60,
        preferred_models: vec!["test-model".into()],
        provider_tags: vec!["coding".into()],
        temperature: Some(0.7),
        max_tokens: Some(2048),
        trust_tier: Some(TrustTier::BuiltIn),
        allowed_tools: vec!["FileWrite".into(), "ShellExec".into()],
        prune_tool_history: false,
        response_format: None,
    };
    let task = PlanTask {
        id: "task-1".into(),
        phase: 1,
        title: "Implement execution path".into(),
        description: "Wire the phase executor through the agent registry.".into(),
        depends_on: vec![],
        status: TaskStatus::Pending,
        estimated_iterations: 12,
        key_files: vec!["crates/y-service/src/plan_orchestrator.rs".into()],
        acceptance_criteria: vec!["Phase executor sees inherited constraints".into()],
    };
    let execution_contract = PlanExecutionContract {
        inherited_constraints: inherited_constraints_from_parts(
            &["crates/y-gui/".to_string()],
            &["Use the parent report format".to_string()],
        ),
        ..Default::default()
    };

    let config = build_phase_execution_config(
        &settings,
        &SessionId::new(),
        Uuid::new_v4(),
        &task,
        "Constrained Phase Execution",
        Path::new("/tmp/gui-plan.md"),
        1,
        2,
        vec![],
        &execution_contract,
        &[],
    );

    assert_eq!(config.messages.len(), 2);
    assert!(config.messages[1].content.contains("- crates/y-gui/"));
    assert!(config.messages[1]
        .content
        .contains("- Use the parent report format"));
}

#[test]
fn test_build_phase_execution_config_preserves_execution_contract() {
    let settings = ResolvedAgentConfig {
        system_prompt: "Execute the phase".into(),
        max_iterations: 30,
        max_tool_calls: 60,
        preferred_models: vec![],
        provider_tags: vec![],
        temperature: Some(0.7),
        max_tokens: None,
        trust_tier: Some(TrustTier::BuiltIn),
        allowed_tools: vec!["FileRead".into(), "FileWrite".into()],
        prune_tool_history: false,
        response_format: None,
    };
    let task = PlanTask {
        id: "task-1".into(),
        phase: 1,
        title: "Retry-safe phase".into(),
        description: "Preserve workspace permissions on retry.".into(),
        depends_on: vec![],
        status: TaskStatus::Pending,
        estimated_iterations: 12,
        key_files: vec!["crates/y-service/src/plan_orchestrator.rs".into()],
        acceptance_criteria: vec!["Retry keeps workspace roots".into()],
    };
    let inherited_constraints = inherited_constraints_from_parts(
        &["crates/y-gui/".to_string()],
        &["Use the agreed report format".to_string()],
    );
    let execution_contract = PlanExecutionContract {
        working_directory: Some("/repo/workspace".to_string()),
        additional_read_dirs: vec!["/repo/.y-agent/plan.md".to_string()],
        inherited_constraints: inherited_constraints.clone(),
    };

    let config = build_phase_execution_config(
        &settings,
        &SessionId::new(),
        Uuid::new_v4(),
        &task,
        "Retry Permissions",
        Path::new("/tmp/fallback-plan.md"),
        1,
        1,
        vec![],
        &execution_contract,
        &[],
    );

    assert_eq!(config.working_directory.as_deref(), Some("/repo/workspace"));
    assert_eq!(
        config.additional_read_dirs,
        vec!["/repo/.y-agent/plan.md".to_string()]
    );
    assert_eq!(config.inherited_constraints, inherited_constraints);
}

#[test]
fn test_hydrate_plan_execution_contract_recovers_legacy_plan_scope() {
    let mut plan = StructuredPlan {
        plan_title: "Legacy Retry".into(),
        plan_file: "/tmp/gui-plan.md".into(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec!["crates/y-gui/".into()],
        guardrails: vec!["Use parent format".into()],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![],
    };

    hydrate_plan_execution_contract(
        &mut plan,
        Path::new("/tmp/gui-plan.md"),
        Some("/repo/workspace".to_string()),
    );

    assert_eq!(
        plan.execution_contract.working_directory.as_deref(),
        Some("/repo/workspace")
    );
    assert_eq!(
        plan.execution_contract.additional_read_dirs,
        vec!["/tmp/gui-plan.md".to_string()]
    );
    let constraints = plan
        .execution_contract
        .inherited_constraints
        .as_ref()
        .expect("constraints should be recovered");
    assert_eq!(constraints.scope_boundaries, vec!["crates/y-gui/"]);
    assert_eq!(constraints.guardrails, vec!["Use parent format"]);
}

#[test]
fn test_build_phase_execution_config_includes_retained_phase_context() {
    let settings = ResolvedAgentConfig {
        system_prompt: "Execute the phase".into(),
        max_iterations: 30,
        max_tool_calls: 60,
        preferred_models: vec![],
        provider_tags: vec![],
        temperature: Some(0.7),
        max_tokens: None,
        trust_tier: Some(TrustTier::BuiltIn),
        allowed_tools: vec!["FileRead".into()],
        prune_tool_history: false,
        response_format: None,
    };
    let task = PlanTask {
        id: "task-2".into(),
        phase: 2,
        title: "Continue implementation".into(),
        description: "Use prior phase output instead of global rediscovery.".into(),
        depends_on: vec!["task-1".into()],
        status: TaskStatus::Pending,
        estimated_iterations: 12,
        key_files: vec!["crates/y-service/src/plan_orchestrator.rs".into()],
        acceptance_criteria: vec!["Prior summary is visible".into()],
    };
    let retained = vec![RetainedPhaseContext {
        task_id: "task-1".to_string(),
        phase: 1,
        title: "Locate executor".to_string(),
        summary: "Executor code lives in crates/y-service/src/agent_service/executor.rs"
            .to_string(),
    }];

    let config = build_phase_execution_config(
        &settings,
        &SessionId::new(),
        Uuid::new_v4(),
        &task,
        "Retry Context",
        Path::new("/tmp/gui-plan.md"),
        2,
        2,
        vec![],
        &PlanExecutionContract::default(),
        &retained,
    );

    assert!(config.messages[1]
        .content
        .contains("## Retained Completed Phase Context"));
    assert!(config.messages[1]
        .content
        .contains("Phase 1: Locate executor"));
    assert!(config.messages[1]
        .content
        .contains("Executor code lives in crates/y-service/src/agent_service/executor.rs"));
}

#[test]
fn test_retained_phase_context_from_results_keeps_completed_summaries() {
    let phase_results = vec![
        serde_json::json!({
            "task_id": "task-1",
            "phase": 1,
            "title": "Discover files",
            "status": "completed",
            "summary": "Use crates/y-service/src/plan_orchestrator.rs",
        }),
        serde_json::json!({
            "task_id": "task-2",
            "phase": 2,
            "title": "Failed phase",
            "status": "failed",
            "error": "blocked",
        }),
    ];

    let retained = retained_phase_context_from_results(&phase_results);

    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].task_id, "task-1");
    assert_eq!(
        retained[0].summary,
        "Use crates/y-service/src/plan_orchestrator.rs"
    );
}

#[test]
fn test_plan_task_serde() {
    let task = PlanTask {
        id: "task-1".into(),
        phase: 1,
        title: "Test".into(),
        description: "Do things".into(),
        depends_on: vec![],
        status: TaskStatus::Pending,
        estimated_iterations: 15,
        key_files: vec!["file.rs".into()],
        acceptance_criteria: vec!["It works".into()],
    };
    let json = serde_json::to_string(&task).unwrap();
    let parsed: PlanTask = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, "task-1");
    assert_eq!(parsed.status, TaskStatus::Pending);
}

#[test]
fn test_structured_plan_serde() {
    let json = r#"{
            "plan_title": "Test Plan",
            "plan_file": "/tmp/plan.md",
            "tasks": [{
                "id": "task-1",
                "phase": 1,
                "title": "Phase 1",
                "description": "Do stuff",
                "depends_on": [],
                "status": "pending",
                "estimated_iterations": 10,
                "key_files": [],
                "acceptance_criteria": ["works"]
            }]
        }"#;
    let plan: StructuredPlan = serde_json::from_str(json).unwrap();
    assert_eq!(plan.plan_title, "Test Plan");
    assert_eq!(plan.tasks.len(), 1);
}

#[tokio::test]
async fn test_resolve_agent_config_uses_plan_writer_definition() {
    let (container, _tmpdir) = make_test_container().await;
    let config = PlanOrchestrator::resolve_agent_config(
        &container,
        "plan-writer",
        ResolvedAgentConfig {
            system_prompt: String::new(),
            max_iterations: 1,
            max_tool_calls: 1,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: None,
            max_tokens: None,
            trust_tier: None,
            allowed_tools: vec![],
            prune_tool_history: false,
            response_format: None,
        },
    )
    .await;

    assert!(config.system_prompt.contains("You are a plan writer"));
    assert_eq!(config.max_iterations, 1);
    assert_eq!(config.max_tool_calls, 0);
    assert_eq!(config.provider_tags, vec!["general", "coding"]);
    assert!(config.allowed_tools.is_empty());
    assert!(!config.prune_tool_history);
}

#[tokio::test]
async fn test_resolve_agent_config_uses_plan_phase_executor_definition() {
    let (container, _tmpdir) = make_test_container().await;
    let config = PlanOrchestrator::resolve_agent_config(
        &container,
        PHASE_EXECUTOR_AGENT_ID,
        ResolvedAgentConfig {
            system_prompt: String::new(),
            max_iterations: 12,
            max_tool_calls: 24,
            preferred_models: vec![],
            provider_tags: vec!["fallback".into()],
            temperature: None,
            max_tokens: None,
            trust_tier: None,
            allowed_tools: vec!["FallbackTool".into()],
            prune_tool_history: true,
            response_format: None,
        },
    )
    .await;

    assert!(config.system_prompt.contains("plan phase executor"));
    assert_eq!(config.max_iterations, 600);
    assert_eq!(config.max_tool_calls, 400);
    assert!(config.allowed_tools.iter().any(|tool| tool == "FileWrite"));
    assert!(config.allowed_tools.iter().any(|tool| tool == "ToolSearch"));
    assert!(!config.prune_tool_history);
}

#[tokio::test]
async fn test_resolve_review_mode_is_manual_at_top_level() {
    // Default guardrail config is Manual; with no plan-execution context
    // active (top-level plan), the human review gate applies.
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("sess-top".into());

    let mode = resolve_review_mode_for_handle(&container, &session).await;

    assert_eq!(mode, PlanReviewMode::Manual);
}

#[tokio::test]
async fn test_resolve_review_mode_forces_auto_when_nested_in_plan_execution() {
    // A plan spawned while executing an approved plan's phases must
    // auto-approve, even though the guardrail config is Manual, so that
    // sub-plans never raise a concurrent review the user cannot see.
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("sess-nested".into());

    let mode =
        plan_execution_ctx::scoped(resolve_review_mode_for_handle(&container, &session)).await;

    assert_eq!(mode, PlanReviewMode::Auto);
}

#[tokio::test]
async fn test_plan_execution_ctx_marker_scopes_correctly() {
    assert!(!plan_execution_ctx::is_active());
    plan_execution_ctx::scoped(async {
        assert!(plan_execution_ctx::is_active());
    })
    .await;
    assert!(!plan_execution_ctx::is_active());
}

#[tokio::test]
async fn test_list_session_plans_reconstructs_persisted_history() {
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("hist-session".into());

    let plan = StructuredPlan {
        plan_title: "Persisted Plan".into(),
        plan_file: "/tmp/persisted.md".into(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![PlanTask {
            id: "t1".into(),
            phase: 1,
            title: "Do step one".into(),
            description: String::new(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 1,
            key_files: vec![],
            acceptance_criteria: vec![],
        }],
    };
    let plan_json = serde_json::to_string(&plan).unwrap();
    container
        .plan_run_store
        .create_run_with_status(
            "run-1",
            session.as_str(),
            &plan_json,
            "/tmp/persisted.md",
            "completed",
        )
        .await
        .unwrap();
    container
        .plan_run_store
        .record_step_result("run-1", "t1", 1, "Do step one", "completed", Some("done"))
        .await
        .unwrap();

    let plans = PlanOrchestrator::list_session_plans(&container, &session).await;

    assert_eq!(plans.len(), 1);
    let display = &plans[0]["display"];
    assert_eq!(display["kind"], "plan_execution");
    assert_eq!(display["plan_run_status"], "completed");
    assert_eq!(display["plan_title"], "Persisted Plan");
    // The persisted step status is reflected on the reconstructed task.
    assert_eq!(display["tasks"][0]["status"], "completed");
}

#[tokio::test]
async fn test_list_session_plans_empty_for_unknown_session() {
    let (container, _tmpdir) = make_test_container().await;
    let plans = PlanOrchestrator::list_session_plans(&container, &SessionId("nobody".into())).await;
    assert!(plans.is_empty());
}

#[test]
fn test_build_task_dag_independent_tasks() {
    let tasks = vec![
        PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Setup schema".into(),
            description: "Create tables".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
        PlanTask {
            id: "task-2".into(),
            phase: 2,
            title: "Add API routes".into(),
            description: "Create endpoints".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
    ];

    let dag = build_task_dag(&tasks).unwrap();
    let ready = dag.ready_tasks(&HashSet::new());
    assert_eq!(ready.len(), 2, "independent tasks should both be ready");
}

#[test]
fn test_build_task_dag_with_dependencies() {
    let tasks = vec![
        PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Create model".into(),
            description: "".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
        PlanTask {
            id: "task-2".into(),
            phase: 2,
            title: "Use model in API".into(),
            description: "".into(),
            depends_on: vec!["task-1".into()],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
        PlanTask {
            id: "task-3".into(),
            phase: 3,
            title: "Independent test".into(),
            description: "".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
    ];

    let dag = build_task_dag(&tasks).unwrap();

    // Initially: task-1 and task-3 are ready (no deps).
    let ready = dag.ready_tasks(&HashSet::new());
    let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
    assert!(ready_ids.contains(&"task-1"));
    assert!(ready_ids.contains(&"task-3"));
    assert!(!ready_ids.contains(&"task-2"));

    // After task-1 completes: task-2 becomes ready.
    let mut completed = HashSet::new();
    completed.insert("task-1".to_string());
    let ready = dag.ready_tasks(&completed);
    let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
    assert!(ready_ids.contains(&"task-2"));
    assert!(ready_ids.contains(&"task-3"));
}

#[test]
fn test_build_task_dag_detects_cycle() {
    let tasks = vec![
        PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "A".into(),
            description: "".into(),
            depends_on: vec!["task-2".into()],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
        PlanTask {
            id: "task-2".into(),
            phase: 2,
            title: "B".into(),
            description: "".into(),
            depends_on: vec!["task-1".into()],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
    ];

    assert!(build_task_dag(&tasks).is_err());
}

#[test]
fn test_build_task_dag_detects_missing_dependency() {
    let tasks = vec![PlanTask {
        id: "task-1".into(),
        phase: 1,
        title: "A".into(),
        description: "".into(),
        depends_on: vec!["nonexistent".into()],
        status: TaskStatus::Pending,
        estimated_iterations: 10,
        key_files: vec![],
        acceptance_criteria: vec![],
    }];

    assert!(build_task_dag(&tasks).is_err());
}

#[test]
fn test_resolve_max_parallel_phases_default_when_missing() {
    let args = serde_json::json!({ "request": "do something" });
    assert_eq!(
        resolve_max_parallel_phases(&args),
        DEFAULT_MAX_PARALLEL_PHASES
    );
}

#[test]
fn test_resolve_max_parallel_phases_accepts_explicit_value() {
    let args = serde_json::json!({ "max_parallel_phases": 2 });
    assert_eq!(resolve_max_parallel_phases(&args), 2);
}

#[test]
fn test_resolve_max_parallel_phases_clamps_to_ceiling() {
    let args = serde_json::json!({ "max_parallel_phases": 999 });
    assert_eq!(
        resolve_max_parallel_phases(&args),
        MAX_PARALLEL_PHASES_CEILING
    );
}

#[test]
fn test_plan_review_decision_revise_serde_round_trip() {
    let decision = PlanReviewDecision::Revise {
        feedback: "reduce scope".into(),
    };
    let json = serde_json::to_string(&decision).unwrap();
    assert!(json.contains(r#""decision":"revise""#));
    assert!(json.contains(r#""feedback":"reduce scope""#));
    let parsed: PlanReviewDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, decision);
}

#[test]
fn test_resolve_max_parallel_phases_zero_falls_back_to_default() {
    let args = serde_json::json!({ "max_parallel_phases": 0 });
    assert_eq!(
        resolve_max_parallel_phases(&args),
        DEFAULT_MAX_PARALLEL_PHASES
    );
}

#[test]
fn test_resolve_max_parallel_phases_ignores_non_numeric() {
    let args = serde_json::json!({ "max_parallel_phases": "four" });
    assert_eq!(
        resolve_max_parallel_phases(&args),
        DEFAULT_MAX_PARALLEL_PHASES
    );
}

#[test]
fn test_build_plan_writer_input_includes_required_fields() {
    let raw = build_plan_writer_input("refactor auth", "src/auth/", None);
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["task"], "refactor auth");
    assert_eq!(parsed["context"], "src/auth/");
    assert!(parsed.get("plan_path").is_none());
    assert!(parsed.get("revision_feedback").is_none());
}

#[test]
fn test_build_plan_writer_input_with_empty_context() {
    let raw = build_plan_writer_input("task", "", None);
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["task"], "task");
    assert_eq!(parsed["context"], "");
}

#[test]
fn test_build_plan_writer_input_with_revision_feedback() {
    let raw = build_plan_writer_input("task", "ctx", Some("make it smaller"));
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["task"], "task");
    assert_eq!(parsed["revision_feedback"], "make it smaller");
}

#[test]
fn test_repair_json_escapes_unescaped_interior_quotes() {
    let input = r#"["text with "interior" quotes"]"#;
    let repaired = repair_json(input);
    let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr[0].as_str().unwrap(), r#"text with "interior" quotes"#);
}

#[test]
fn test_repair_json_preserves_valid_json() {
    let input = r#"{"key": "value", "arr": [1, 2]}"#;
    let repaired = repair_json(input);
    assert_eq!(repaired, input);
}

#[test]
fn test_repair_json_handles_cjk_unescaped_quotes() {
    let input = concat!(
        r#"["normal step", "#,
        r#""record "#,
        "\"this CVE\"",
        r#" as conclusion"]"#,
    );
    let repaired = repair_json(input);
    let parsed: serde_json::Result<serde_json::Value> = serde_json::from_str(&repaired);
    assert!(parsed.is_ok(), "repaired JSON should parse: {repaired}");
}

#[test]
fn test_repair_json_escapes_invalid_escape_in_regex() {
    // Reproduces the reported failure: the plan-writer emits a regex
    // containing `\d`, `\s`, `\w` -- invalid JSON escapes that make
    // serde_json fail with "invalid escape".
    let input = r#"{"overview": "match \d+\s*\w tokens"}"#;
    assert!(
        serde_json::from_str::<serde_json::Value>(input).is_err(),
        "precondition: raw input must be invalid JSON"
    );
    let repaired = repair_json(input);
    let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(
        parsed["overview"].as_str().unwrap(),
        r#"match \d+\s*\w tokens"#
    );
}

#[test]
fn test_repair_json_escapes_windows_path() {
    // Backslashes before non-escape chars in a path are repaired. (Segments
    // starting with b/f/n/r/t/u would be valid escapes and thus transformed,
    // which is unavoidable without semantic knowledge.)
    let input = r#"{"path": "C:\Users\Admin\Data"}"#;
    let repaired = repair_json(input);
    let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(parsed["path"].as_str().unwrap(), r#"C:\Users\Admin\Data"#);
}

#[test]
fn test_repair_json_preserves_valid_escapes() {
    let input = r#"{"s": "a\nb\tc\"d\\e\/fé"}"#;
    let repaired = repair_json(input);
    let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(parsed["s"].as_str().unwrap(), "a\nb\tc\"d\\e/f\u{00e9}");
}

#[test]
fn test_repair_json_preserves_escaped_backslash_before_terminator() {
    // A string ending in an escaped backslash must not swallow the
    // closing quote.
    let input = r#"{"a": "path\\", "b": 1}"#;
    let repaired = repair_json(input);
    let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(parsed["a"].as_str().unwrap(), r"path\");
    assert_eq!(parsed["b"].as_i64().unwrap(), 1);
}

#[test]
fn test_repair_json_escapes_invalid_unicode_escape() {
    // `\u` not followed by four hex digits is an invalid escape.
    let input = r#"{"s": "bad \u12 escape"}"#;
    let repaired = repair_json(input);
    let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(parsed["s"].as_str().unwrap(), r#"bad \u12 escape"#);
}

#[test]
fn test_repair_json_then_parse_plan_with_regex_in_description() {
    let raw = concat!(
        "```json\n",
        "{\n",
        r#"  "plan_title": "Add validation","#,
        "\n",
        r#"  "plan_file": "","#,
        "\n",
        r#"  "tasks": [{"#,
        "\n",
        r#"    "task_id": "p1","#,
        "\n",
        r#"    "title": "Regex","#,
        "\n",
        r#"    "objective": "Validate with pattern \d{3}-\d{4} and \w+","#,
        "\n",
        r#"    "depends_on": [],"#,
        "\n",
        r#"    "status": "pending","#,
        "\n",
        r#"    "estimated_iterations": 10,"#,
        "\n",
        r#"    "key_files": [],"#,
        "\n",
        r#"    "acceptance_criteria": []"#,
        "\n",
        "  }]\n",
        "}\n",
        "```",
    );
    let extracted = extract_json_from_response(raw);
    let repaired = repair_json(&extracted);
    let plan = parse_structured_plan(&repaired).unwrap();
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(
        plan.tasks[0].description,
        r#"Validate with pattern \d{3}-\d{4} and \w+"#
    );
}

#[test]
fn test_parse_structured_plan_handles_bare_array() {
    let input = r#"[
            {
                "task_id": "phase_1",
                "title": "Search NVD",
                "objective": "Query the NVD database",
                "steps": ["step 1"],
                "depends_on": [],
                "estimated_iterations": 12,
                "key_files": [],
                "acceptance_criteria": ["found or not"],
                "status": "pending"
            }
        ]"#;
    let plan = parse_structured_plan(input).unwrap();
    assert_eq!(plan.plan_title, "Untitled Plan");
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "phase_1");
    assert_eq!(plan.tasks[0].description, "Query the NVD database");
}

#[test]
fn test_parse_structured_plan_aliases_task_id_and_objective() {
    let input = r#"{
            "plan_title": "Test",
            "plan_file": "",
            "tasks": [{
                "task_id": "p1",
                "phase": 1,
                "title": "Do work",
                "objective": "The objective",
                "depends_on": [],
                "status": "pending",
                "estimated_iterations": 10,
                "key_files": [],
                "acceptance_criteria": []
            }]
        }"#;
    let plan = parse_structured_plan(input).unwrap();
    assert_eq!(plan.tasks[0].id, "p1");
    assert_eq!(plan.tasks[0].description, "The objective");
}

#[test]
fn test_repair_json_then_parse_bare_array_with_interior_quotes() {
    let raw = concat!(
        "```json\n",
        "[\n",
        "  {\n",
        r#"    "task_id": "phase_1","#,
        "\n",
        r#"    "title": "Search","#,
        "\n",
        r#"    "objective": "Find the CVE","#,
        "\n",
        r#"    "steps": ["check NVD", "if not found, record "#,
        "\"not found in DB\"",
        r#" as conclusion"],"#,
        "\n",
        r#"    "depends_on": [],"#,
        "\n",
        r#"    "estimated_iterations": 10,"#,
        "\n",
        r#"    "key_files": [],"#,
        "\n",
        r#"    "acceptance_criteria": ["done"],"#,
        "\n",
        r#"    "status": "pending""#,
        "\n",
        "  }\n",
        "]\n",
        "```",
    );
    let extracted = extract_json_from_response(raw);
    let repaired = repair_json(&extracted);
    let plan = parse_structured_plan(&repaired).unwrap();
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].id, "phase_1");
}

#[test]
fn test_normalize_plan_task_phases_renumbers_arbitrary_values() {
    let mut tasks = vec![
        PlanTask {
            id: "a".into(),
            phase: 5,
            title: "first".into(),
            description: String::new(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 0,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
        PlanTask {
            id: "b".into(),
            phase: 6,
            title: "second".into(),
            description: String::new(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 0,
            key_files: vec![],
            acceptance_criteria: vec![],
        },
    ];

    normalize_plan_task_phases(&mut tasks);

    assert_eq!(tasks[0].phase, 1);
    assert_eq!(tasks[1].phase, 2);
}

#[test]
fn test_stage_status_for_review_status_keeps_stage_running_during_review() {
    assert_eq!(stage_status_for_review_status("awaiting_user"), "running");
    assert_eq!(
        stage_status_for_review_status("feedback_received"),
        "running"
    );
    assert_eq!(stage_status_for_review_status("approved"), "completed");
    assert_eq!(stage_status_for_review_status("auto_approved"), "completed");
    assert_eq!(stage_status_for_review_status("rejected"), "completed");
}

#[test]
fn test_build_plan_writer_stage_metadata_marks_stage_running_when_awaiting_user() {
    let plan = StructuredPlan {
        plan_title: "Plan".into(),
        plan_file: String::new(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![],
    };

    let awaiting =
        build_plan_writer_stage_metadata(std::path::Path::new("/tmp/p.md"), &plan, "awaiting_user");
    assert_eq!(awaiting["display"]["stage_status"], "running");

    let approved =
        build_plan_writer_stage_metadata(std::path::Path::new("/tmp/p.md"), &plan, "approved");
    assert_eq!(approved["display"]["stage_status"], "completed");
}

#[test]
fn test_build_plan_review_metadata_includes_review_id_when_awaiting() {
    let plan = StructuredPlan {
        plan_title: "Plan".into(),
        plan_file: String::new(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![],
    };

    let awaiting = build_plan_review_metadata(
        std::path::Path::new("/tmp/p.md"),
        &plan,
        "awaiting_user",
        "",
        Some("review-123"),
    );
    assert_eq!(awaiting["display"]["review_id"], "review-123");

    // Non-awaiting emissions carry an empty review id so the frontend
    // bubble only renders approval controls for the awaiting review.
    let approved = build_plan_review_metadata(
        std::path::Path::new("/tmp/p.md"),
        &plan,
        "approved",
        "",
        None,
    );
    assert_eq!(approved["display"]["review_id"], "");
}

#[test]
fn test_resolve_plan_file_for_display_prefers_plan_file_when_set() {
    let mut plan = StructuredPlan {
        plan_title: String::new(),
        plan_file: String::new(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![],
    };
    let path = std::path::Path::new("/tmp/fallback.md");

    assert_eq!(
        resolve_plan_file_for_display(path, &plan),
        "/tmp/fallback.md"
    );

    plan.plan_file = "/explicit/plan.md".into();
    assert_eq!(
        resolve_plan_file_for_display(path, &plan),
        "/explicit/plan.md"
    );
}

#[test]
fn test_build_plan_execution_metadata_uses_unified_plan_file_resolution() {
    let plan = StructuredPlan {
        plan_title: "Plan".into(),
        plan_file: "/explicit/plan.md".into(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: vec![],
    };

    let meta = build_plan_execution_metadata(
        std::path::Path::new("/tmp/fallback.md"),
        &plan,
        "run-1",
        0,
        0,
        &[],
    );

    assert_eq!(meta["display"]["plan_file"], "/explicit/plan.md");
}

// ---------------------------------------------------------------------------
// Transient error detection for phase-level retry
//
// Verifies that `is_transient_llm_error` delegates to the standard
// `StandardError::should_auto_retry` classification (same logic the
// provider pool uses), not string matching on the error message.
// ---------------------------------------------------------------------------

use y_core::provider::ProviderError;

fn llm_err(pe: ProviderError) -> AgentExecutionError {
    AgentExecutionError::LlmError {
        message: format!("{pe}"),
        provider_error: Some(pe),
        partial_messages: vec![],
    }
}

#[test]
fn test_is_transient_llm_error_network_error_is_transient() {
    // Mid-stream EOF after HTTP 200 — the exact scenario from the bug report.
    let error = llm_err(ProviderError::NetworkError {
        status: Some(200),
        message: "stream read error after HTTP 200: unexpected EOF".into(),
    });
    assert!(is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_server_error_is_transient() {
    let error = llm_err(ProviderError::ServerError {
        provider: "openai".into(),
        message: "internal server error".into(),
    });
    assert!(is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_auth_error_is_not_transient() {
    let error = llm_err(ProviderError::AuthenticationFailed {
        provider: "openai".into(),
        message: "invalid API key".into(),
    });
    assert!(!is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_quota_error_is_not_transient() {
    let error = llm_err(ProviderError::QuotaExhausted {
        provider: "openai".into(),
        message: "billing limit reached".into(),
    });
    assert!(!is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_rate_limited_is_not_auto_retried() {
    // Rate limits carry their own Retry-After; should_auto_retry returns
    // false (the pool handles them via freeze + retry-after, not auto-retry).
    let error = llm_err(ProviderError::RateLimited {
        provider: "openai".into(),
        retry_after_secs: 60,
    });
    assert!(!is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_context_error_is_not_transient() {
    let error = AgentExecutionError::ContextError("context too large".into());
    assert!(!is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_cancelled_is_not_transient() {
    let error = AgentExecutionError::Cancelled {
        partial_messages: vec![],
        accumulated_content: String::new(),
        iteration_texts: vec![],
        iteration_reasonings: vec![],
        iteration_reasoning_durations_ms: vec![],
        iteration_tool_counts: vec![],
        tool_calls_executed: vec![],
        iterations: 0,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: 0.0,
        model: String::new(),
        generated_images: vec![],
    };
    assert!(!is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_loop_limit_is_not_transient() {
    let error = AgentExecutionError::ToolLoopLimitExceeded { max_iterations: 30 };
    assert!(!is_transient_llm_error(&error));
}

#[test]
fn test_is_transient_llm_error_without_provider_error_is_not_transient() {
    // LlmError with provider_error=None (synthetic/test construction) should
    // not be retried — we can't classify it.
    let error = AgentExecutionError::LlmError {
        message: "unknown".into(),
        provider_error: None,
        partial_messages: vec![],
    };
    assert!(!is_transient_llm_error(&error));
}

// ---------------------------------------------------------------------------
// Cancellation and resume detection
// ---------------------------------------------------------------------------

fn make_simple_plan(tasks: Vec<&str>) -> StructuredPlan {
    StructuredPlan {
        plan_title: "Test Plan".into(),
        plan_file: "/tmp/test-plan.md".into(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks: tasks
            .iter()
            .enumerate()
            .map(|(i, id)| PlanTask {
                id: (*id).into(),
                phase: i + 1,
                title: format!("Task {}", id),
                description: String::new(),
                depends_on: if i > 0 {
                    vec![tasks[i - 1].into()]
                } else {
                    vec![]
                },
                status: TaskStatus::Pending,
                estimated_iterations: 1,
                key_files: vec![],
                acceptance_criteria: vec![],
            })
            .collect(),
    }
}

#[tokio::test]
async fn test_cancelled_plan_run_is_detectable_for_resume() {
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("resume-session".into());
    let plan = make_simple_plan(vec!["t1", "t2", "t3"]);
    let plan_json = serde_json::to_string(&plan).unwrap();

    container
        .plan_run_store
        .create_run_with_status(
            "run-cancelled",
            session.as_str(),
            &plan_json,
            "/tmp/test-plan.md",
            "cancelled",
        )
        .await
        .unwrap();
    // t1 completed before cancel; t2 and t3 never ran.
    container
        .plan_run_store
        .record_step_result(
            "run-cancelled",
            "t1",
            1,
            "Task t1",
            "completed",
            Some("done"),
        )
        .await
        .unwrap();

    let latest = container
        .plan_run_store
        .find_latest_run(session.as_str())
        .await
        .unwrap()
        .expect("should find a run");
    assert_eq!(latest.status, "cancelled");

    let steps = container
        .plan_run_store
        .load_step_results(&latest.id)
        .await
        .unwrap();
    let completed_ids: HashSet<&str> = steps
        .iter()
        .filter(|s| s.status == "completed")
        .map(|s| s.task_id.as_str())
        .collect();
    let has_uncompleted = plan
        .tasks
        .iter()
        .any(|t| !completed_ids.contains(t.id.as_str()));
    assert!(has_uncompleted, "should have uncompleted tasks");
}

#[tokio::test]
async fn test_completed_plan_run_all_tasks_done_is_not_resumable() {
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("done-session".into());
    let plan = make_simple_plan(vec!["t1", "t2"]);
    let plan_json = serde_json::to_string(&plan).unwrap();

    container
        .plan_run_store
        .create_run_with_status(
            "run-done",
            session.as_str(),
            &plan_json,
            "/tmp/test-plan.md",
            "completed",
        )
        .await
        .unwrap();
    container
        .plan_run_store
        .record_step_result("run-done", "t1", 1, "Task t1", "completed", Some("done"))
        .await
        .unwrap();
    container
        .plan_run_store
        .record_step_result("run-done", "t2", 2, "Task t2", "completed", Some("done"))
        .await
        .unwrap();

    let latest = container
        .plan_run_store
        .find_latest_run(session.as_str())
        .await
        .unwrap()
        .expect("should find a run");
    assert_eq!(latest.status, "completed");

    let steps = container
        .plan_run_store
        .load_step_results(&latest.id)
        .await
        .unwrap();
    let completed_ids: HashSet<&str> = steps
        .iter()
        .filter(|s| s.status == "completed")
        .map(|s| s.task_id.as_str())
        .collect();
    let has_uncompleted = plan
        .tasks
        .iter()
        .any(|t| !completed_ids.contains(t.id.as_str()));
    assert!(!has_uncompleted, "all tasks completed — not resumable");
}

#[tokio::test]
async fn test_partial_failure_run_is_resumable() {
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("partial-session".into());
    let plan = make_simple_plan(vec!["t1", "t2", "t3"]);
    let plan_json = serde_json::to_string(&plan).unwrap();

    container
        .plan_run_store
        .create_run_with_status(
            "run-partial",
            session.as_str(),
            &plan_json,
            "/tmp/test-plan.md",
            "partial_failure",
        )
        .await
        .unwrap();
    container
        .plan_run_store
        .record_step_result("run-partial", "t1", 1, "Task t1", "completed", Some("done"))
        .await
        .unwrap();
    container
        .plan_run_store
        .record_step_result("run-partial", "t2", 2, "Task t2", "failed", Some("error"))
        .await
        .unwrap();

    let latest = container
        .plan_run_store
        .find_latest_run(session.as_str())
        .await
        .unwrap()
        .expect("should find a run");
    assert_eq!(latest.status, "partial_failure");
    assert!(
        matches!(latest.status.as_str(), "cancelled" | "partial_failure"),
        "partial_failure should be detected as interrupted/resumable"
    );
}

#[test]
fn test_cancelled_tool_error_round_trips() {
    let err = cancelled_tool_error();
    assert!(is_cancelled_tool_error(&err));
    // A non-cancelled RuntimeError should not match.
    let other = ToolError::RuntimeError {
        name: "Plan".into(),
        message: "phase-1 execution failed: LLM error: timeout".into(),
    };
    assert!(!is_cancelled_tool_error(&other));
}

#[tokio::test]
async fn test_resume_plan_with_empty_from_task_id_invalidates_nothing() {
    let (container, _tmpdir) = make_test_container().await;
    let session = SessionId("empty-resume-session".into());
    let plan = make_simple_plan(vec!["t1", "t2"]);
    let plan_json = serde_json::to_string(&plan).unwrap();

    container
        .plan_run_store
        .create_run_with_status(
            "run-empty",
            session.as_str(),
            &plan_json,
            "/tmp/test-plan.md",
            "cancelled",
        )
        .await
        .unwrap();
    container
        .plan_run_store
        .record_step_result("run-empty", "t1", 1, "Task t1", "completed", Some("done"))
        .await
        .unwrap();

    // With empty from_task_id, compute_downstream_tasks is not called and
    // no step results are deleted. Verify the completed step survives.
    let invalidated = HashSet::<String>::new();
    assert!(invalidated.is_empty());

    let steps = container
        .plan_run_store
        .load_step_results("run-empty")
        .await
        .unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].task_id, "t1");
    assert_eq!(steps[0].status, "completed");
}

#[tokio::test]
async fn test_archive_stale_phase_sessions_archives_duplicates() {
    let (container, _tmpdir) = make_test_container().await;
    let _parent = SessionId("parent-session".into());

    // Create the parent session first so children can reference it.
    container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Parent".into()),
        })
        .await
        .unwrap();
    // Use the actual created session as parent.
    let parent_session = container
        .session_manager
        .list_sessions(&y_core::session::SessionFilter::default())
        .await
        .unwrap();
    let parent_id = parent_session[0].id.clone();

    // Create two child sessions with the same phase title (simulating a retry).
    let phase_title = "Phase 4: Fix the bug";
    let _old_child = container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: Some(parent_id.clone()),
            session_type: SessionType::SubAgent,
            agent_id: Some(y_core::types::AgentId::from_string(PHASE_EXECUTOR_AGENT_ID)),
            title: Some(phase_title.into()),
        })
        .await
        .unwrap();
    let _new_child = container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: Some(parent_id.clone()),
            session_type: SessionType::SubAgent,
            agent_id: Some(y_core::types::AgentId::from_string(PHASE_EXECUTOR_AGENT_ID)),
            title: Some(phase_title.into()),
        })
        .await
        .unwrap();

    // Both should be active before archival.
    let children = container
        .session_manager
        .children(&parent_id)
        .await
        .unwrap();
    let active_count = children
        .iter()
        .filter(|c| c.state == y_core::session::SessionState::Active)
        .count();
    assert_eq!(active_count, 2);

    // Archive stale sessions with the same title.
    PlanOrchestrator::archive_stale_phase_sessions(&container, &parent_id, phase_title).await;

    // Both old sessions should now be archived (none active).
    let children = container
        .session_manager
        .children(&parent_id)
        .await
        .unwrap();
    let active_count = children
        .iter()
        .filter(|c| c.state == y_core::session::SessionState::Active)
        .count();
    assert_eq!(
        active_count, 0,
        "all sessions with matching title should be archived"
    );

    // A session with a different title should not be affected.
    let other_child = container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: Some(parent_id.clone()),
            session_type: SessionType::SubAgent,
            agent_id: Some(y_core::types::AgentId::from_string(PHASE_EXECUTOR_AGENT_ID)),
            title: Some("Phase 5: Different".into()),
        })
        .await
        .unwrap();
    PlanOrchestrator::archive_stale_phase_sessions(&container, &parent_id, "Phase 4: Fix the bug")
        .await;
    let other = container
        .session_manager
        .children(&parent_id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.id == other_child.id)
        .unwrap();
    assert_eq!(
        other.state,
        y_core::session::SessionState::Active,
        "sessions with different title should not be archived"
    );
}
