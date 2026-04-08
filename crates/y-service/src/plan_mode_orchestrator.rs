//! Plan mode orchestrator: intercepts `EnterPlanMode` / `ExitPlanMode` tool
//! calls and drives the phased execution lifecycle.
//!
//! Follows the same pattern as `TaskDelegationOrchestrator` and
//! `ToolSearchOrchestrator` -- the `tool_dispatch` layer intercepts specific
//! tool names and routes them here instead of the normal tool registry.

use std::fmt::Write as _;
use std::path::Path;
use std::pin::Pin;

use y_core::tool::{ToolError, ToolOutput};

use crate::agent_service::{AgentExecutionConfig, AgentService};
use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Plan data structures
// ---------------------------------------------------------------------------

/// Status of the overall plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Status of a single phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// A parsed execution phase from the plan markdown.
#[derive(Debug, Clone)]
pub struct Phase {
    pub number: usize,
    pub title: String,
    pub body: String,
    pub status: PhaseStatus,
}

/// Result of a single phase execution.
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub number: usize,
    pub status: PhaseStatus,
    pub summary: String,
}

// ---------------------------------------------------------------------------
// Plan parser
// ---------------------------------------------------------------------------

/// Parse a plan markdown file into a list of phases.
///
/// Looks for `## Phase N: <title>` headers and collects everything between
/// them as the phase body. Also extracts the Overview section if present.
pub fn parse_plan_phases(content: &str) -> (String, Vec<Phase>) {
    let mut overview = String::new();
    let mut phases: Vec<Phase> = Vec::new();
    let mut current_phase: Option<(usize, String, String)> = None;
    let mut in_overview = false;

    for line in content.lines() {
        // Detect Phase headers: `## Phase N:` or `## Phase N -`
        if let Some(phase_info) = parse_phase_header(line) {
            // Flush previous phase.
            if let Some((num, title, body)) = current_phase.take() {
                phases.push(Phase {
                    number: num,
                    title,
                    body: body.trim().to_string(),
                    status: PhaseStatus::Pending,
                });
            }
            in_overview = false;
            current_phase = Some((phase_info.0, phase_info.1, String::new()));
            continue;
        }

        // Detect Overview section.
        if line.starts_with("## Overview") {
            in_overview = true;
            // Flush any current phase (shouldn't happen if overview is first).
            if let Some((num, title, body)) = current_phase.take() {
                phases.push(Phase {
                    number: num,
                    title,
                    body: body.trim().to_string(),
                    status: PhaseStatus::Pending,
                });
            }
            continue;
        }

        // Another `## ` header that isn't a Phase or Overview ends current section.
        if line.starts_with("## ") {
            in_overview = false;
            if let Some((num, title, body)) = current_phase.take() {
                phases.push(Phase {
                    number: num,
                    title,
                    body: body.trim().to_string(),
                    status: PhaseStatus::Pending,
                });
            }
            continue;
        }

        // Skip YAML frontmatter.
        if line.starts_with("---") {
            continue;
        }

        // Accumulate content.
        if let Some((_, _, body)) = current_phase.as_mut() {
            body.push_str(line);
            body.push('\n');
        } else if in_overview {
            overview.push_str(line);
            overview.push('\n');
        }
    }

    // Flush final phase.
    if let Some((num, title, body)) = current_phase {
        phases.push(Phase {
            number: num,
            title,
            body: body.trim().to_string(),
            status: PhaseStatus::Pending,
        });
    }

    (overview.trim().to_string(), phases)
}

/// Try to parse a line as a phase header like `## Phase 1: Title` or
/// `## Phase 2 - Title`.
fn parse_phase_header(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("## Phase ") {
        return None;
    }

    let rest = &trimmed["## Phase ".len()..];

    // Extract the number (digits before `:` or `-` or whitespace).
    let num_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if num_end == 0 {
        return None;
    }

    let number: usize = rest[..num_end].parse().ok()?;

    // Extract the title after the separator (`:` or `-`).
    let after_num = rest[num_end..].trim_start();
    let title = if let Some(stripped) = after_num.strip_prefix(':') {
        stripped.trim().to_string()
    } else if let Some(stripped) = after_num.strip_prefix('-') {
        stripped.trim().to_string()
    } else {
        after_num.to_string()
    };

    Some((number, title))
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Orchestrator for plan mode tool interceptions.
pub struct PlanModeOrchestrator;

impl PlanModeOrchestrator {
    /// Handle `EnterPlanMode` tool call.
    ///
    /// Returns a confirmation message. The actual mode transition (tool
    /// restriction) is handled by prompt injection -- the LLM is instructed
    /// to only use read-only tools until it calls `ExitPlanMode`.
    pub fn handle_enter(args: &serde_json::Value) -> Result<ToolOutput, ToolError> {
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("complex task");
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Plan");

        tracing::info!(
            title = %title,
            reason = %reason,
            "plan mode entered"
        );

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "status": "plan_mode_active",
                "title": title,
                "reason": reason,
                "instructions": "You are now in PLAN MODE. Only use read-only tools \
                    (FileRead, Glob, Grep, SearchCode, WebFetch, Browser). \
                    Investigate the codebase, then write your plan with PlanWriter. \
                    When done, call ExitPlanMode with the plan file path."
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle `ExitPlanMode` tool call.
    ///
    /// Parses the plan file, executes each phase as a separate sub-agent run,
    /// and returns a consolidated summary.
    ///
    /// Returns a `Pin<Box<...>>` to break the recursive async cycle:
    /// `handle_exit` -> `execute_phases` -> `AgentService::execute` ->
    /// `execute_tool_call` -> (potentially) `handle_exit`.
    pub fn handle_exit<'a>(
        args: &'a serde_json::Value,
        container: &'a ServiceContainer,
        progress: Option<&'a crate::chat::TurnEventSender>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let plan_file = args
                .get("plan_file")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::ValidationError {
                    message: "'plan_file' is required".into(),
                })?;

            let plan_path = Path::new(plan_file);
            if !plan_path.exists() {
                return Err(ToolError::ValidationError {
                    message: format!("plan file does not exist: {plan_file}"),
                });
            }

            // Read and parse the plan.
            let content =
                tokio::fs::read_to_string(plan_path)
                    .await
                    .map_err(|e| ToolError::Other {
                        message: format!("failed to read plan file: {e}"),
                    })?;

            let (overview, phases) = parse_plan_phases(&content);

            if phases.is_empty() {
                return Err(ToolError::Other {
                    message: "no phases found in plan file".into(),
                });
            }

            tracing::info!(
                plan_file = %plan_file,
                phases = phases.len(),
                "exiting plan mode, beginning phase execution"
            );

            // Execute phases sequentially.
            let results =
                Self::execute_phases(container, &overview, &phases, plan_path, progress).await;

            // Build consolidated summary.
            let summary = Self::build_summary(&overview, &results);

            // Update plan file status.
            let all_ok = results.iter().all(|r| r.status == PhaseStatus::Completed);
            let final_status = if all_ok {
                PlanStatus::Completed
            } else {
                PlanStatus::Failed
            };

            if let Err(e) = Self::update_plan_file(plan_path, &results, final_status).await {
                tracing::warn!(error = %e, "failed to update plan file status");
            }

            Ok(ToolOutput {
                success: all_ok,
                content: serde_json::json!({
                    "status": format!("{final_status:?}").to_lowercase(),
                    "plan_file": plan_file,
                    "phases_total": phases.len(),
                    "phases_completed": results.iter()
                        .filter(|r| r.status == PhaseStatus::Completed)
                        .count(),
                    "summary": summary,
                }),
                warnings: vec![],
                metadata: serde_json::json!({}),
            })
        }) // close Box::pin
    }

    /// Execute all phases sequentially, each as a fresh sub-agent run.
    async fn execute_phases(
        container: &ServiceContainer,
        overview: &str,
        phases: &[Phase],
        plan_path: &Path,
        progress: Option<&crate::chat::TurnEventSender>,
    ) -> Vec<PhaseResult> {
        let mut results: Vec<PhaseResult> = Vec::new();

        for phase in phases {
            tracing::info!(
                phase = phase.number,
                title = %phase.title,
                "starting phase execution"
            );

            let system_prompt = Self::build_phase_prompt(overview, phase, &results);

            // Build tool definitions: full access during phase execution.
            let tool_defs =
                crate::chat::ChatService::build_essential_tool_definitions(container).await;

            let exec_config = AgentExecutionConfig {
                agent_name: format!("plan-phase-{}", phase.number),
                system_prompt: system_prompt.clone(),
                max_iterations: 20,
                tool_definitions: tool_defs,
                tool_calling_mode: y_core::provider::ToolCallingMode::Native,
                messages: vec![y_core::types::Message {
                    message_id: y_core::types::generate_message_id(),
                    role: y_core::types::Role::User,
                    content: format!(
                        "Execute Phase {} of the plan: {}\n\n\
                         Follow the steps exactly. When all acceptance criteria \
                         are met, provide a summary of what was accomplished.",
                        phase.number, phase.title
                    ),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: y_core::types::now(),
                    metadata: serde_json::Value::Null,
                }],
                provider_id: None,
                preferred_models: vec![],
                provider_tags: vec![],
                temperature: Some(0.3),
                max_tokens: None,
                thinking: None,
                session_id: None,
                session_uuid: uuid::Uuid::new_v4(),
                knowledge_collections: vec![],
                use_context_pipeline: false,
                user_query: String::new(),
                external_trace_id: None,
                trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
                agent_allowed_tools: vec![
                    "FileRead".into(),
                    "FileWrite".into(),
                    "FileEdit".into(),
                    "Glob".into(),
                    "Grep".into(),
                    "ShellExec".into(),
                    "WebFetch".into(),
                    "Browser".into(),
                ],
                prune_tool_history: false,
            };

            let phase_result =
                match AgentService::execute(container, &exec_config, progress.cloned(), None).await
                {
                    Ok(result) => {
                        tracing::info!(
                            phase = phase.number,
                            iterations = result.iterations,
                            "phase completed successfully"
                        );
                        PhaseResult {
                            number: phase.number,
                            status: PhaseStatus::Completed,
                            summary: result.content,
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            phase = phase.number,
                            error = %e,
                            "phase execution failed"
                        );
                        PhaseResult {
                            number: phase.number,
                            status: PhaseStatus::Failed,
                            summary: format!("Phase failed: {e}"),
                        }
                    }
                };

            results.push(phase_result);

            // Update plan file with intermediate status.
            if let Err(e) =
                Self::update_plan_file(plan_path, &results, PlanStatus::InProgress).await
            {
                tracing::warn!(error = %e, "failed to update plan file mid-execution");
            }
        }

        results
    }

    /// Build the system prompt for a phase sub-agent.
    fn build_phase_prompt(overview: &str, phase: &Phase, prior_results: &[PhaseResult]) -> String {
        let mut prompt = String::with_capacity(2048);

        prompt.push_str(
            "You are executing one phase of a structured implementation plan.\n\
             Follow the steps exactly as specified. Do not work on tasks from \
             other phases.\n\n",
        );

        // Plan overview.
        if !overview.is_empty() {
            prompt.push_str("## Plan Overview\n\n");
            prompt.push_str(overview);
            prompt.push_str("\n\n");
        }

        // Current phase details.
        let _ = write!(
            prompt,
            "## Current Phase: Phase {} - {}\n\n",
            phase.number, phase.title
        );
        prompt.push_str(&phase.body);
        prompt.push_str("\n\n");

        // Prior phase results (summary only).
        if !prior_results.is_empty() {
            prompt.push_str("## Previous Phase Results\n\n");
            for r in prior_results {
                let status = match r.status {
                    PhaseStatus::Completed => "completed",
                    PhaseStatus::Failed => "failed",
                    PhaseStatus::InProgress => "in_progress",
                    PhaseStatus::Pending => "pending",
                };
                // Truncate summary to avoid context explosion.
                let summary = if r.summary.len() > 1000 {
                    format!("{}...", &r.summary[..1000])
                } else {
                    r.summary.clone()
                };
                let _ = writeln!(prompt, "- Phase {} ({}): {}", r.number, status, summary);
            }
            prompt.push('\n');
        }

        prompt.push_str(
            "## Instructions\n\n\
             Execute the steps for this phase. When all acceptance criteria are met, \
             provide a concise summary of what was accomplished and any issues encountered.\n",
        );

        prompt
    }

    /// Build a consolidated summary from all phase results.
    fn build_summary(overview: &str, results: &[PhaseResult]) -> String {
        let mut summary = String::with_capacity(1024);

        summary.push_str("# Plan Execution Summary\n\n");

        if !overview.is_empty() {
            let brief = if overview.len() > 200 {
                format!("{}...", &overview[..200])
            } else {
                overview.to_string()
            };
            let _ = write!(summary, "**Overview**: {brief}\n\n");
        }

        for r in results {
            let icon = match r.status {
                PhaseStatus::Completed => "[OK]",
                PhaseStatus::Failed => "[FAIL]",
                _ => "[--]",
            };
            let _ = writeln!(
                summary,
                "{icon} Phase {}: {}",
                r.number,
                r.summary.lines().next().unwrap_or("")
            );
        }

        summary
    }

    /// Append execution results to the plan file.
    async fn update_plan_file(
        plan_path: &Path,
        results: &[PhaseResult],
        status: PlanStatus,
    ) -> Result<(), std::io::Error> {
        let content = tokio::fs::read_to_string(plan_path).await?;

        // Replace `status: pending` (or `in_progress`) in frontmatter.
        let status_str = match status {
            PlanStatus::Pending => "pending",
            PlanStatus::InProgress => "in_progress",
            PlanStatus::Completed => "completed",
            PlanStatus::Failed => "failed",
        };
        let updated = content
            .replace("status: pending", &format!("status: {status_str}"))
            .replace("status: in_progress", &format!("status: {status_str}"));

        // Append results section if not already present.
        let results_marker = "\n## Execution Results\n";
        let with_results = if updated.contains(results_marker) {
            // Replace existing results section.
            let parts: Vec<&str> = updated.splitn(2, results_marker).collect();
            let before = parts[0];
            let mut new = before.to_string();
            new.push_str(results_marker);
            new.push('\n');
            for r in results {
                let status_label = match r.status {
                    PhaseStatus::Completed => "completed",
                    PhaseStatus::Failed => "FAILED",
                    _ => "pending",
                };
                let _ = write!(
                    new,
                    "### Phase {} [{}]\n\n{}\n\n",
                    r.number,
                    status_label,
                    r.summary.lines().take(10).collect::<Vec<_>>().join("\n")
                );
            }
            new
        } else {
            let mut new = updated;
            new.push_str(results_marker);
            new.push('\n');
            for r in results {
                let status_label = match r.status {
                    PhaseStatus::Completed => "completed",
                    PhaseStatus::Failed => "FAILED",
                    _ => "pending",
                };
                let _ = write!(
                    new,
                    "### Phase {} [{}]\n\n{}\n\n",
                    r.number,
                    status_label,
                    r.summary.lines().take(10).collect::<Vec<_>>().join("\n")
                );
            }
            new
        };

        tokio::fs::write(plan_path, with_results).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PLAN: &str = r#"---
title: Refactor skill ingestion
created: "2026-04-07T19:00:00+08:00"
status: pending
total_phases: 3
---

## Overview

Refactor the skill ingestion pipeline to support versioned manifests.

## Phase 1: Analyze existing parser

### Objective
Understand the current SkillParser implementation.

### Key Files
- crates/y-skills/src/parser.rs

### Steps
1. Read parser.rs
2. Document extension points

### Acceptance Criteria
- Extension points identified

---

## Phase 2: Add new fields

### Objective
Add version and dependencies fields.

### Steps
1. Update SkillManifest struct
2. Update parser

### Acceptance Criteria
- New fields added with defaults

---

## Phase 3: Tests and docs

### Objective
Ensure everything is tested.

### Steps
1. Add unit tests
2. Update docs

### Acceptance Criteria
- All tests pass
"#;

    #[test]
    fn test_parse_plan_phases_count() {
        let (overview, phases) = parse_plan_phases(SAMPLE_PLAN);
        assert_eq!(phases.len(), 3);
        assert!(!overview.is_empty());
        assert!(overview.contains("versioned manifests"));
    }

    #[test]
    fn test_parse_plan_phase_numbers() {
        let (_, phases) = parse_plan_phases(SAMPLE_PLAN);
        assert_eq!(phases[0].number, 1);
        assert_eq!(phases[1].number, 2);
        assert_eq!(phases[2].number, 3);
    }

    #[test]
    fn test_parse_plan_phase_titles() {
        let (_, phases) = parse_plan_phases(SAMPLE_PLAN);
        assert_eq!(phases[0].title, "Analyze existing parser");
        assert_eq!(phases[1].title, "Add new fields");
        assert_eq!(phases[2].title, "Tests and docs");
    }

    #[test]
    fn test_parse_plan_phase_body() {
        let (_, phases) = parse_plan_phases(SAMPLE_PLAN);
        assert!(phases[0]
            .body
            .contains("Understand the current SkillParser"));
        assert!(phases[0].body.contains("### Acceptance Criteria"));
        assert!(phases[1].body.contains("SkillManifest struct"));
    }

    #[test]
    fn test_parse_phase_header() {
        assert_eq!(
            parse_phase_header("## Phase 1: Analyze code"),
            Some((1, "Analyze code".to_string()))
        );
        assert_eq!(
            parse_phase_header("## Phase 12 - Build feature"),
            Some((12, "Build feature".to_string()))
        );
        assert_eq!(parse_phase_header("## Overview"), None);
        assert_eq!(parse_phase_header("# Phase 1: Not h2"), None);
    }

    #[test]
    fn test_build_phase_prompt_includes_overview() {
        let phase = Phase {
            number: 1,
            title: "Test".to_string(),
            body: "Do something.".to_string(),
            status: PhaseStatus::Pending,
        };
        let prompt = PlanModeOrchestrator::build_phase_prompt("Big picture", &phase, &[]);
        assert!(prompt.contains("Big picture"));
        assert!(prompt.contains("Phase 1 - Test"));
        assert!(prompt.contains("Do something."));
    }

    #[test]
    fn test_build_phase_prompt_includes_prior_results() {
        let phase = Phase {
            number: 2,
            title: "Second".to_string(),
            body: "More work.".to_string(),
            status: PhaseStatus::Pending,
        };
        let prior = vec![PhaseResult {
            number: 1,
            status: PhaseStatus::Completed,
            summary: "Did first thing.".to_string(),
        }];
        let prompt = PlanModeOrchestrator::build_phase_prompt("", &phase, &prior);
        assert!(prompt.contains("Phase 1 (completed)"));
        assert!(prompt.contains("Did first thing."));
    }

    #[test]
    fn test_build_summary() {
        let results = vec![
            PhaseResult {
                number: 1,
                status: PhaseStatus::Completed,
                summary: "Analyzed parser.".to_string(),
            },
            PhaseResult {
                number: 2,
                status: PhaseStatus::Failed,
                summary: "Compilation error.".to_string(),
            },
        ];
        let summary = PlanModeOrchestrator::build_summary("Refactor plan", &results);
        assert!(summary.contains("[OK] Phase 1"));
        assert!(summary.contains("[FAIL] Phase 2"));
    }

    #[test]
    fn test_parse_empty_plan() {
        let (overview, phases) = parse_plan_phases("");
        assert!(overview.is_empty());
        assert!(phases.is_empty());
    }

    #[test]
    fn test_parse_plan_no_phases() {
        let content = "## Overview\n\nJust an overview, no phases.\n";
        let (overview, phases) = parse_plan_phases(content);
        assert!(overview.contains("Just an overview"));
        assert!(phases.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Complexity assessment (auto mode)
// ---------------------------------------------------------------------------

/// Agent ID for the complexity classifier (matches `config/agents/complexity-classifier.toml`).
const CLASSIFIER_AGENT_ID: &str = "complexity-classifier";

/// Fallback system prompt used when the agent definition is not found in the registry.
const CLASSIFIER_FALLBACK_PROMPT: &str = "\
You are a task complexity classifier. Given the user's request, respond with \
exactly one word: \"plan\" if the task requires multi-file changes, architectural \
design, refactoring, or multi-step coordination. Respond \"fast\" if the task is \
a single-file fix, formatting, direct question, or simple tweak. \
Respond with ONLY \"plan\" or \"fast\", nothing else.";

/// Assess whether the user's request is complex enough to warrant plan mode.
///
/// Loads the `complexity-classifier` agent definition from the registry
/// (`config/agents/complexity-classifier.toml`) and executes a single-turn,
/// zero-tool LLM call. If the definition is missing, falls back to built-in
/// defaults.
///
/// Returns `true` if the classifier outputs "plan". On any error (provider
/// unavailable, parse failure), defaults to `false` (no plan) to avoid
/// blocking the user.
pub async fn assess_complexity(
    container: &ServiceContainer,
    user_input: &str,
    provider_id: Option<&str>,
) -> bool {
    use y_core::types::{Message, Role};

    // Load agent definition from registry, falling back to defaults.
    let registry = container.agent_registry.lock().await;
    let agent_def = registry.get(CLASSIFIER_AGENT_ID);

    let system_prompt = agent_def.map_or_else(
        || CLASSIFIER_FALLBACK_PROMPT.to_string(),
        |d| d.system_prompt.clone(),
    );
    let temperature = agent_def.and_then(|d| d.temperature).unwrap_or(0.0);
    let max_iterations = agent_def.map_or(1, |d| d.max_iterations);
    let max_tokens = agent_def.and_then(|d| d.max_completion_tokens).unwrap_or(5) as u32;
    let provider_tags: Vec<String> = agent_def
        .map(|d| d.provider_tags.clone())
        .unwrap_or_default();
    let prune_tool_history = agent_def.is_some_and(|d| d.prune_tool_history);

    // Release the lock before the async LLM call.
    drop(registry);

    let messages = vec![
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: system_prompt.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        },
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: user_input.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        },
    ];

    let exec_config = AgentExecutionConfig {
        agent_name: CLASSIFIER_AGENT_ID.to_string(),
        system_prompt: String::new(), // Included in messages above.
        max_iterations,
        tool_definitions: vec![],
        tool_calling_mode: y_core::provider::ToolCallingMode::Native,
        messages,
        provider_id: provider_id.map(String::from),
        preferred_models: vec![],
        provider_tags,
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
        thinking: None,
        session_id: None,
        session_uuid: uuid::Uuid::new_v4(),
        knowledge_collections: vec![],
        use_context_pipeline: false,
        user_query: user_input.to_string(),
        external_trace_id: None,
        trust_tier: None,
        agent_allowed_tools: vec![],
        prune_tool_history,
    };

    match AgentService::execute(container, &exec_config, None, None).await {
        Ok(result) => {
            let response = result.content.trim().to_lowercase();
            tracing::debug!(
                classifier_response = %response,
                "plan mode complexity assessment"
            );
            response.contains("plan")
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "complexity assessment failed, defaulting to fast mode"
            );
            false
        }
    }
}
