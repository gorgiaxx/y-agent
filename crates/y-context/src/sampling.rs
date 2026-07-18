//! Sampling preflight helpers for context-window safety.

use std::ops::Range;

use sha2::{Digest, Sha256};
use y_core::provider::ChatRequest;
use y_core::types::{Message, Role};

const MESSAGE_OVERHEAD_TOKENS: u32 = 4;
const DEFAULT_OUTPUT_RESERVE_TOKENS: u32 = 4096;

/// Conservative token estimate for one provider sampling request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplingTokenEstimate {
    pub message_tokens: u32,
    pub tool_tokens: u32,
    pub reserved_output_tokens: u32,
    pub total_tokens: u32,
}

/// Sampling decision relative to a provider context-window threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingPreflightVerdict {
    UnknownContextWindow,
    Fits {
        estimated_tokens: u32,
        threshold_tokens: u32,
    },
    Compact {
        estimated_tokens: u32,
        threshold_tokens: u32,
    },
}

/// Estimate request occupancy, including tool schemas and output reserve.
pub fn estimate_sampling_tokens(request: &ChatRequest) -> SamplingTokenEstimate {
    let message_tokens = request.messages.iter().fold(0_u32, |total, message| {
        let content_tokens = y_prompt::estimate_tokens(&message.content);
        let tool_call_tokens = message.tool_calls.iter().fold(0_u32, |tool_total, call| {
            tool_total
                .saturating_add(y_prompt::estimate_tokens(&call.name))
                .saturating_add(y_prompt::estimate_tokens(&call.arguments.to_string()))
        });
        let tool_result_id_tokens = message
            .tool_call_id
            .as_deref()
            .map_or(0, y_prompt::estimate_tokens);
        total
            .saturating_add(MESSAGE_OVERHEAD_TOKENS)
            .saturating_add(content_tokens)
            .saturating_add(tool_call_tokens)
            .saturating_add(tool_result_id_tokens)
    });
    let tool_tokens = request.tools.iter().fold(0_u32, |total, tool| {
        total.saturating_add(y_prompt::estimate_tokens(&tool.to_string()))
    });
    let reserved_output_tokens = request.max_tokens.unwrap_or(DEFAULT_OUTPUT_RESERVE_TOKENS);
    let total_tokens = message_tokens
        .saturating_add(tool_tokens)
        .saturating_add(reserved_output_tokens);

    SamplingTokenEstimate {
        message_tokens,
        tool_tokens,
        reserved_output_tokens,
        total_tokens,
    }
}

/// Compare estimated request occupancy with the configured compaction limit.
pub fn sampling_preflight(
    estimate: SamplingTokenEstimate,
    context_window: usize,
    threshold_pct: u32,
) -> SamplingPreflightVerdict {
    if context_window == 0 {
        return SamplingPreflightVerdict::UnknownContextWindow;
    }
    let context_window = u64::try_from(context_window).unwrap_or(u64::MAX);
    let threshold_tokens = context_window.saturating_mul(u64::from(threshold_pct.min(100))) / 100;
    let threshold_tokens = u32::try_from(threshold_tokens).unwrap_or(u32::MAX);
    if estimate.total_tokens >= threshold_tokens {
        SamplingPreflightVerdict::Compact {
            estimated_tokens: estimate.total_tokens,
            threshold_tokens,
        }
    } else {
        SamplingPreflightVerdict::Fits {
            estimated_tokens: estimate.total_tokens,
            threshold_tokens,
        }
    }
}

/// Select the oldest message range that can be compacted while retaining the
/// requested recent window and preserving tool-call/result protocol groups.
pub fn safe_compaction_range(messages: &[Message], retain_window: usize) -> Option<Range<usize>> {
    let start = protected_prefix_len(messages);
    let mut end = messages.len().saturating_sub(retain_window);
    if end <= start {
        return None;
    }

    for (assistant_index, assistant) in messages.iter().enumerate().take(end) {
        if assistant.role != Role::Assistant || assistant.tool_calls.is_empty() {
            continue;
        }
        let crosses_boundary = assistant.tool_calls.iter().any(|tool_call| {
            messages.iter().enumerate().skip(end).any(|(_, message)| {
                message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some(tool_call.id.as_str())
            })
        });
        if crosses_boundary {
            end = assistant_index;
            break;
        }
    }

    (end > start).then_some(start..end)
}

/// Hash the exact source prefix and compaction configuration for cache reuse.
pub fn compaction_prefix_fingerprint(
    messages: &[Message],
    range: Range<usize>,
    config: &crate::compaction::CompactionConfig,
) -> String {
    let mut hasher = Sha256::new();
    update_hash(&mut hasher, &range.start.to_le_bytes());
    update_hash(&mut hasher, &range.end.to_le_bytes());
    if let Ok(config_bytes) = serde_json::to_vec(config) {
        update_hash(&mut hasher, &config_bytes);
    }
    if let Some(prefix) = messages.get(range) {
        for message in prefix {
            if let Ok(message_bytes) = serde_json::to_vec(message) {
                update_hash(&mut hasher, &message_bytes);
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

fn update_hash(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(bytes);
}

fn protected_prefix_len(messages: &[Message]) -> usize {
    messages.first().map_or(0, |message| {
        let is_generated_summary = message
            .metadata
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| matches!(kind, "compaction_summary" | "handoff"));
        usize::from(message.role == Role::System && !is_generated_summary)
    })
}

#[cfg(test)]
mod tests {
    use y_core::provider::{ChatRequest, RequestMode, ToolCallingMode, ToolDialect};
    use y_core::types::{Message, Role, ToolCallRequest};

    use crate::compaction::CompactionConfig;

    use super::{
        compaction_prefix_fingerprint, estimate_sampling_tokens, safe_compaction_range,
        sampling_preflight, SamplingPreflightVerdict, SamplingTokenEstimate,
    };

    fn message(role: Role, content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn safe_range_never_splits_assistant_tool_call_from_result() {
        let mut assistant = message(Role::Assistant, "checking");
        assistant.tool_calls.push(ToolCallRequest {
            id: "call-1".to_string(),
            name: "FileRead".to_string(),
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        });
        let mut tool_result = message(Role::Tool, "file contents");
        tool_result.tool_call_id = Some("call-1".to_string());
        let messages = vec![
            message(Role::User, "old request"),
            assistant,
            tool_result,
            message(Role::Assistant, "old answer"),
            message(Role::User, "recent request"),
        ];

        let range = safe_compaction_range(&messages, 3).expect("compactable prefix");

        assert_eq!(range, 0..1);
    }

    #[test]
    fn sampling_estimate_includes_tools_and_output_reserve() {
        let request = ChatRequest {
            messages: vec![message(Role::User, &"x".repeat(400))],
            model: None,
            request_mode: RequestMode::TextChat,
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            tools: vec![serde_json::json!({
                "type": "function",
                "function": {
                    "name": "FileRead",
                    "parameters": {"type": "object", "properties": {}}
                }
            })],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::default(),
            stop: Vec::new(),
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let estimate = estimate_sampling_tokens(&request);

        assert!(estimate.message_tokens >= 100);
        assert!(estimate.tool_tokens > 0);
        assert_eq!(estimate.reserved_output_tokens, 512);
        assert_eq!(
            estimate.total_tokens,
            estimate
                .message_tokens
                .saturating_add(estimate.tool_tokens)
                .saturating_add(estimate.reserved_output_tokens)
        );
    }

    #[test]
    fn prefix_fingerprint_allows_appended_recent_messages_but_rejects_source_changes() {
        let mut messages = vec![
            message(Role::User, "old request"),
            message(Role::Assistant, "old response"),
            message(Role::User, "recent request"),
        ];
        let config = CompactionConfig::default();
        let original = compaction_prefix_fingerprint(&messages, 0..2, &config);

        messages.push(message(Role::Assistant, "newly appended"));
        let appended = compaction_prefix_fingerprint(&messages, 0..2, &config);
        messages[1].content = "changed old response".to_string();
        let changed = compaction_prefix_fingerprint(&messages, 0..2, &config);

        assert_eq!(appended, original);
        assert_ne!(changed, original);
    }

    #[test]
    fn preflight_compacts_at_configured_request_occupancy() {
        let estimate = SamplingTokenEstimate {
            message_tokens: 700,
            tool_tokens: 50,
            reserved_output_tokens: 100,
            total_tokens: 850,
        };

        assert_eq!(
            sampling_preflight(estimate, 1000, 85),
            SamplingPreflightVerdict::Compact {
                estimated_tokens: 850,
                threshold_tokens: 850,
            }
        );
    }
}
