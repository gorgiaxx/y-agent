//! Shared chat types used across y-service and presentation layers.

use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use y_core::permission_types::PermissionMode;
use y_core::provider::{
    GeneratedImage, ImageGenerationOptions, RequestMode, ThinkingConfig, ThinkingEffort,
};
use y_core::trust::TrustTier;
use y_core::types::{Message, SessionId};

use crate::agent_service::AgentExecutionError;

// ---------------------------------------------------------------------------
// TurnMeta -- cached per-session turn metadata for presentation layers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TurnMeta {
    pub provider_id: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub context_window: usize,
    pub context_tokens_used: u64,
}

// ---------------------------------------------------------------------------
// Shared DTO types for chat operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct UndoResult {
    pub messages_removed: usize,
    pub restored_turn_number: u32,
    pub files_restored: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCheckpointInfo {
    pub checkpoint_id: String,
    pub session_id: String,
    pub turn_number: u32,
    pub message_count_before: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageWithStatus {
    pub id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub checkpoint_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub context_window: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreResult {
    pub tombstoned_count: u32,
    pub restored_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactResult {
    pub messages_pruned: usize,
    pub messages_compacted: usize,
    pub tokens_saved: u32,
    pub summary: String,
}

// ---------------------------------------------------------------------------
// Tool call record
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub arguments: String,
    pub success: bool,
    pub duration_ms: u64,
    pub result_content: String,
    pub url_meta: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Turn progress events (for real-time observability)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    LlmResponse {
        iteration: usize,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
        cost_usd: f64,
        tool_calls_requested: Vec<String>,
        prompt_preview: String,
        response_text: String,
        context_window: usize,
        agent_name: String,
    },
    ToolStart {
        name: String,
        input_preview: String,
        agent_name: String,
    },
    ToolResult {
        name: String,
        success: bool,
        duration_ms: u64,
        input_preview: String,
        result_preview: String,
        agent_name: String,
        url_meta: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
    LoopLimitHit {
        iterations: usize,
        max_iterations: usize,
    },
    StreamDelta {
        content: String,
        agent_name: String,
    },
    StreamReasoningDelta {
        content: String,
        agent_name: String,
    },
    StreamImageDelta {
        index: usize,
        mime_type: String,
        partial_data: String,
        agent_name: String,
    },
    StreamImageComplete {
        index: usize,
        mime_type: String,
        data: String,
        agent_name: String,
    },
    LlmError {
        iteration: usize,
        error: String,
        duration_ms: u64,
        model: String,
        prompt_preview: String,
        context_window: usize,
        agent_name: String,
    },
    UserInteractionRequest {
        interaction_id: String,
        questions: serde_json::Value,
    },
    PermissionRequest {
        request_id: String,
        tool_name: String,
        action_description: String,
        reason: String,
        content_preview: Option<String>,
    },
    PlanReviewRequest {
        review_id: String,
        plan_title: String,
        plan_file: String,
        estimated_effort: String,
        overview: String,
        scope_in: Vec<String>,
        scope_out: Vec<String>,
        guardrails: Vec<String>,
        plan_content: String,
        tasks: serde_json::Value,
    },
}

pub type TurnEventSender = mpsc::UnboundedSender<TurnEvent>;

// ---------------------------------------------------------------------------
// Permission prompt
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPromptResponse {
    Approve,
    Deny,
    AllowAllForSession,
}

/// Structured outcome of a plan-review prompt.
///
/// Returned by the GUI / API surface back into the orchestrator over a
/// oneshot channel. The orchestrator -- not the LLM -- consumes this and
/// decides whether to execute the plan or abort with the user's feedback.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PlanReviewDecision {
    Approve,
    Reject {
        #[serde(default)]
        feedback: String,
    },
}

/// Per-turn operation mode selected by the client input area.
///
/// The mode is stored per session before the turn runs so tool dispatch and
/// plan orchestration can resolve policy without presentation-layer logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationMode {
    /// Use the global Guardrails configuration.
    #[default]
    Default,
    /// Skip manual plan review while keeping normal tool permissions.
    AutoReview,
    /// Bypass manual plan review and tool permission prompts for this turn.
    FullAccess,
}

// ---------------------------------------------------------------------------
// Pending interaction / permission channels
// ---------------------------------------------------------------------------

pub type PendingInteractions = std::sync::Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>,
    >,
>;

pub type PendingPermissions = std::sync::Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<PermissionPromptResponse>>,
    >,
>;

pub type PendingPlanReviews = std::sync::Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<PlanReviewDecision>>,
    >,
>;

// ---------------------------------------------------------------------------
// Turn result / error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TurnResult {
    pub content: String,
    pub model: String,
    pub provider_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub last_input_tokens: u64,
    pub context_window: usize,
    pub cost_usd: f64,
    pub tool_calls_executed: Vec<ToolCallRecord>,
    pub iterations: usize,
    pub generated_images: Vec<GeneratedImage>,
    pub new_messages: Vec<Message>,
}

#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    #[error("LLM error: {0}")]
    LlmError(String),
    #[error("Context error: {0}")]
    ContextError(String),
    #[error("Tool call loop limit ({max_iterations}) exceeded")]
    ToolLoopLimitExceeded { max_iterations: usize },
    #[error("Tool call limit ({max_tool_calls}) exceeded")]
    ToolCallLimitExceeded { max_tool_calls: usize },
    #[error("Cancelled")]
    Cancelled,
}

impl From<AgentExecutionError> for TurnError {
    fn from(err: AgentExecutionError) -> Self {
        match err {
            AgentExecutionError::LlmError { message, .. } => TurnError::LlmError(message),
            AgentExecutionError::ContextError(msg) => TurnError::ContextError(msg),
            AgentExecutionError::ToolLoopLimitExceeded { max_iterations } => {
                TurnError::ToolLoopLimitExceeded { max_iterations }
            }
            AgentExecutionError::ToolCallLimitExceeded { max_tool_calls } => {
                TurnError::ToolCallLimitExceeded { max_tool_calls }
            }
            AgentExecutionError::Cancelled { .. } => TurnError::Cancelled,
        }
    }
}

// ---------------------------------------------------------------------------
// Turn input
// ---------------------------------------------------------------------------

pub struct TurnInput<'a> {
    pub user_input: &'a str,
    pub session_id: SessionId,
    pub session_uuid: Uuid,
    pub history: &'a [Message],
    pub turn_number: u32,
    pub provider_id: Option<String>,
    pub request_mode: RequestMode,
    pub working_directory: Option<String>,
    pub knowledge_collections: Vec<String>,
    pub thinking: Option<ThinkingConfig>,
    pub plan_mode: Option<String>,
    pub operation_mode: OperationMode,
    pub agent_name: String,
    pub toolcall_enabled: bool,
    pub preferred_models: Vec<String>,
    pub provider_tags: Vec<String>,
    pub temperature: Option<f64>,
    pub max_completion_tokens: Option<u32>,
    pub max_iterations: Option<usize>,
    pub max_tool_calls: Option<usize>,
    pub trust_tier: Option<TrustTier>,
    pub agent_allowed_tools: Vec<String>,
    pub prune_tool_history: bool,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Vec<String>,
    pub image_generation_options: Option<ImageGenerationOptions>,
}

pub type TurnCancellationToken = tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Session agent config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SessionAgentFeatures {
    pub toolcall: bool,
    pub skills: bool,
    pub knowledge: bool,
}

#[derive(Debug, Clone)]
pub struct SessionAgentConfig {
    pub agent_id: String,
    pub agent_name: String,
    pub agent_mode: String,
    pub working_directory: Option<String>,
    pub features: SessionAgentFeatures,
    pub allowed_tools: Vec<String>,
    pub preset_skills: Vec<String>,
    pub knowledge_collections: Vec<String>,
    pub prompt_section_ids: Vec<String>,
    pub system_prompt: Option<String>,
    pub provider_id: Option<String>,
    pub preferred_models: Vec<String>,
    pub provider_tags: Vec<String>,
    pub temperature: Option<f64>,
    pub max_completion_tokens: Option<u32>,
    pub thinking: Option<ThinkingConfig>,
    pub plan_mode: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub trust_tier: TrustTier,
    pub prune_tool_history: bool,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Vec<String>,
}

// ---------------------------------------------------------------------------
// Turn preparation types
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct PrepareTurnRequest {
    pub session_id: Option<SessionId>,
    pub user_input: String,
    pub provider_id: Option<String>,
    pub request_mode: Option<RequestMode>,
    pub skills: Option<Vec<String>>,
    pub knowledge_collections: Option<Vec<String>>,
    pub thinking: Option<ThinkingConfig>,
    pub user_message_metadata: Option<serde_json::Value>,
    pub plan_mode: Option<String>,
    pub operation_mode: Option<OperationMode>,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Option<Vec<String>>,
    pub image_generation_options: Option<ImageGenerationOptions>,
}

#[derive(Debug)]
pub struct PreparedTurn {
    pub session_id: SessionId,
    pub session_uuid: Uuid,
    pub history: Vec<Message>,
    pub turn_number: u32,
    pub user_input: String,
    pub provider_id: Option<String>,
    pub request_mode: RequestMode,
    pub session_created: bool,
    pub working_directory: Option<String>,
    pub knowledge_collections: Vec<String>,
    pub thinking: Option<ThinkingConfig>,
    pub plan_mode: Option<String>,
    pub operation_mode: OperationMode,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Vec<String>,
    pub skills: Vec<String>,
    pub agent_config: Option<SessionAgentConfig>,
    pub image_generation_options: Option<ImageGenerationOptions>,
}

impl PreparedTurn {
    pub fn as_turn_input(&self) -> TurnInput<'_> {
        let agent_name = self.agent_config.as_ref().map_or_else(
            || "chat-turn".to_string(),
            |config| config.agent_name.clone(),
        );
        TurnInput {
            user_input: &self.user_input,
            session_id: self.session_id.clone(),
            session_uuid: self.session_uuid,
            history: &self.history,
            turn_number: self.turn_number,
            provider_id: self.provider_id.clone(),
            request_mode: self.request_mode,
            working_directory: self.working_directory.clone(),
            knowledge_collections: self.knowledge_collections.clone(),
            thinking: self.thinking.clone(),
            plan_mode: self.plan_mode.clone(),
            operation_mode: self.operation_mode,
            agent_name,
            toolcall_enabled: self
                .agent_config
                .as_ref()
                .is_none_or(|config| config.features.toolcall),
            preferred_models: self
                .agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.preferred_models.clone()),
            provider_tags: self
                .agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.provider_tags.clone()),
            temperature: self
                .agent_config
                .as_ref()
                .and_then(|config| config.temperature),
            max_completion_tokens: self
                .agent_config
                .as_ref()
                .and_then(|config| config.max_completion_tokens),
            max_iterations: self
                .agent_config
                .as_ref()
                .map(|config| config.max_iterations),
            max_tool_calls: self
                .agent_config
                .as_ref()
                .map(|config| config.max_tool_calls),
            trust_tier: self.agent_config.as_ref().map(|config| config.trust_tier),
            agent_allowed_tools: self
                .agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.allowed_tools.clone()),
            prune_tool_history: self
                .agent_config
                .as_ref()
                .is_some_and(|config| config.prune_tool_history),
            mcp_mode: self.mcp_mode.clone(),
            mcp_servers: self.mcp_servers.clone(),
            image_generation_options: self.image_generation_options.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareTurnError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("failed to create session: {0}")]
    SessionCreationFailed(String),
    #[error("failed to persist user message: {0}")]
    PersistFailed(String),
    #[error("failed to read transcript: {0}")]
    TranscriptReadFailed(String),
    #[error("session agent not found: {0}")]
    SessionAgentNotFound(String),
    #[error("session turn limit reached for agent '{agent_id}' ({max_turns} turns)")]
    SessionTurnLimitReached { agent_id: String, max_turns: usize },
}

// ---------------------------------------------------------------------------
// Resend-turn types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ResendTurnRequest {
    pub session_id: SessionId,
    pub checkpoint_id: String,
    pub provider_id: Option<String>,
    pub request_mode: Option<RequestMode>,
    pub knowledge_collections: Option<Vec<String>>,
    pub thinking: Option<ThinkingConfig>,
    pub plan_mode: Option<String>,
    pub operation_mode: Option<OperationMode>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResendTurnError {
    #[error("checkpoint not found: {0}")]
    CheckpointNotFound(String),
    #[error("truncation failed: {0}")]
    TruncateFailed(String),
    #[error("transcript empty after truncation")]
    TranscriptEmpty,
    #[error("failed to read transcript: {0}")]
    TranscriptReadFailed(String),
    #[error("session agent not found: {0}")]
    SessionAgentNotFound(String),
}

// ---------------------------------------------------------------------------
// Turn metadata summary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnMetaSummary {
    pub provider_id: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub context_window: usize,
    pub context_tokens_used: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn parse_thinking(effort: Option<String>) -> Option<ThinkingConfig> {
    effort.and_then(|e| {
        let effort = match e.as_str() {
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "max" => ThinkingEffort::Max,
            _ => return None,
        };
        Some(ThinkingConfig { effort })
    })
}
