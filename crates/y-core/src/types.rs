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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenUsageSource {
    /// Usage reported by the provider's API response (authoritative).
    #[default]
    ProviderReported,
    /// Usage estimated via heuristic (e.g., chars/4 approximation).
    Estimated,
}

/// Token usage reported by an LLM provider.
///
/// Field semantics are normalized across providers so downstream consumers can
/// treat every provider uniformly:
///
/// - `input_tokens` is the *fresh*, non-cached prompt tokens only. Providers
///   differ on what their native field means (Anthropic's `input_tokens`
///   excludes cache hits, whereas `OpenAI`'s `prompt_tokens` includes them), so
///   each provider backend maps its native counts into this fresh-only field
///   and reports cache hits separately in `cache_read_tokens` /
///   `cache_write_tokens`.
/// - To obtain the real context size processed for this request (the value that
///   should drive context-window occupancy), use [`TokenUsage::total_input_tokens`],
///   which sums the fresh, cache-read, and cache-write counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Fresh (non-cached) prompt tokens. Excludes cache reads and writes.
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Prompt tokens served from the provider's cache (cheaper than fresh
    /// input). Not included in `input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Prompt tokens written to the provider's cache (cache creation). Not
    /// included in `input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
    /// How the token counts were obtained.
    #[serde(default)]
    pub source: TokenUsageSource,
}

impl TokenUsage {
    /// Fresh input plus output. Does not account for cached prompt tokens.
    pub fn total(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// The total prompt tokens processed for this request: fresh input plus
    /// cache reads plus cache writes. This is the authoritative context-window
    /// occupancy figure and is consistent across providers.
    pub fn total_input_tokens(&self) -> u32 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens.unwrap_or(0))
            .saturating_add(self.cache_write_tokens.unwrap_or(0))
    }

    /// Provider-neutral `usage` JSON object for diagnostics payloads.
    ///
    /// Uses the unified field names (`input_tokens` / `output_tokens` /
    /// `cache_read_tokens` / `cache_write_tokens`) rather than any single
    /// provider's wire format, so synthesized diagnostics payloads are
    /// consistent regardless of which backend served the request. Cache fields
    /// are only included when present.
    pub fn to_diagnostics_json(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
        });
        if let Some(read) = self.cache_read_tokens {
            value["cache_read_tokens"] = serde_json::json!(read);
        }
        if let Some(write) = self.cache_write_tokens {
            value["cache_write_tokens"] = serde_json::json!(write);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_usage_total_saturates_on_overflow() {
        let usage = TokenUsage {
            input_tokens: u32::MAX,
            output_tokens: 1,
            ..TokenUsage::default()
        };

        assert_eq!(usage.total(), u32::MAX);
    }

    #[test]
    fn test_total_input_tokens_sums_fresh_and_cache() {
        let usage = TokenUsage {
            input_tokens: 491,
            output_tokens: 145,
            cache_read_tokens: Some(80_384),
            cache_write_tokens: Some(0),
            ..TokenUsage::default()
        };

        // Fresh-only stays small; the real context size includes cache reads.
        assert_eq!(usage.input_tokens, 491);
        assert_eq!(usage.total_input_tokens(), 80_875);
    }

    #[test]
    fn test_total_input_tokens_without_cache_equals_fresh() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            ..TokenUsage::default()
        };

        assert_eq!(usage.total_input_tokens(), 100);
    }

    #[test]
    fn test_total_input_tokens_saturates_on_overflow() {
        let usage = TokenUsage {
            input_tokens: u32::MAX,
            cache_read_tokens: Some(10),
            cache_write_tokens: Some(10),
            ..TokenUsage::default()
        };

        assert_eq!(usage.total_input_tokens(), u32::MAX);
    }
}
