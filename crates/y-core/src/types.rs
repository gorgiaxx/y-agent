//! Shared primitive types used across all y-agent crates.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Strongly-typed session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Strongly-typed workflow identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub String);

/// Strongly-typed task identifier within a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

/// Strongly-typed provider identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub String);

/// Strongly-typed agent identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

/// Strongly-typed tool name (tools are identified by name, not UUID).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolName(pub String);

/// Strongly-typed skill identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SkillId(pub String);

/// Strongly-typed memory identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryId(pub String);

// Implement Display for all ID types via a macro.
macro_rules! impl_id_display {
    ($($t:ty),*) => {
        $(
            impl fmt::Display for $t {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str(&self.0)
                }
            }

            impl $t {
                /// Create a new random identifier.
                pub fn new() -> Self {
                    Self(Uuid::new_v4().to_string())
                }

                /// Create from an existing string.
                pub fn from_string(s: impl Into<String>) -> Self {
                    Self(s.into())
                }

                /// Get the inner string reference.
                pub fn as_str(&self) -> &str {
                    &self.0
                }
            }

            impl Default for $t {
                fn default() -> Self {
                    Self::new()
                }
            }
        )*
    };
}

impl_id_display!(SessionId, WorkflowId, TaskId, ProviderId, AgentId, ToolName, SkillId, MemoryId);

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

/// Standard timestamp type used across the system.
pub type Timestamp = DateTime<Utc>;

/// Return the current UTC timestamp.
pub fn now() -> Timestamp {
    Utc::now()
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Role in a conversation message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Generate a new unique message ID.
pub fn generate_message_id() -> String {
    Uuid::new_v4().to_string()
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message identifier for checkpoint addressing.
    #[serde(default = "generate_message_id")]
    pub message_id: String,
    pub role: Role,
    pub content: String,
    /// Tool call ID (when role = Tool, this links to the originating call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls requested by the assistant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRequest>,
    pub timestamp: Timestamp,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Token usage
// ---------------------------------------------------------------------------

/// Source of token usage data.
///
/// Providers report token counts through different mechanisms. This enum
/// tracks the origin so downstream consumers can assess accuracy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenUsageSource {
    /// Usage reported by the provider's API response (authoritative).
    #[default]
    ProviderReported,
    /// Usage estimated via heuristic (e.g., chars/4 approximation).
    Estimated,
}

/// Token usage reported by an LLM provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
    /// How the token counts were obtained.
    #[serde(default)]
    pub source: TokenUsageSource,
}

impl TokenUsage {
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}
