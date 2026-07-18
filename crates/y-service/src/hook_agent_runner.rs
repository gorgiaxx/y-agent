use std::sync::Arc;
use std::time::Duration;

use y_core::agent::{AgentRunConfig, AgentRunner, WorkspaceIsolationPreference};
use y_core::hook::HookAgentRunner;
use y_core::trust::TrustTier;

use crate::agent_service::ServiceAgentRunner;
use crate::container::ServiceContainer;

pub(crate) struct ServiceHookAgentRunner {
    container: Arc<ServiceContainer>,
}

impl ServiceHookAgentRunner {
    pub(crate) fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait::async_trait]
impl HookAgentRunner for ServiceHookAgentRunner {
    async fn run_agent(
        &self,
        task_prompt: &str,
        model: Option<&str>,
        max_turns: u32,
        timeout: Duration,
    ) -> Result<String, String> {
        let preferred_models = model.into_iter().map(str::to_string).collect();
        let config = AgentRunConfig {
            agent_name: "hook-verifier".to_string(),
            system_prompt: "Evaluate the hook event using read-only evidence. Return only JSON with boolean 'ok' and string 'reason'.".to_string(),
            input: serde_json::Value::String(task_prompt.to_string()),
            preferred_models,
            fallback_models: Vec::new(),
            provider_tags: Vec::new(),
            fallback_provider_tags: Vec::new(),
            temperature: None,
            max_tokens: None,
            timeout_secs: timeout.as_secs().max(1),
            allowed_tools: vec!["FileRead".to_string(), "Glob".to_string(), "Grep".to_string()],
            max_iterations: usize::try_from(max_turns.clamp(1, 50)).unwrap_or(50),
            trust_tier: Some(TrustTier::Dynamic),
            trace_id: None,
            prune_tool_history: false,
            response_format: None,
            workspace_isolation: WorkspaceIsolationPreference::Shared,
            workspace_snapshot_id: None,
        };
        let runner = ServiceAgentRunner::new(Arc::clone(&self.container));
        tokio::time::timeout(timeout, runner.run(config))
            .await
            .map_err(|_| format!("hook agent timed out after {} ms", timeout.as_millis()))?
            .map(|output| output.text)
            .map_err(|error| error.to_string())
    }
}
