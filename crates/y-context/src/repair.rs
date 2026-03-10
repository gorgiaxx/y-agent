//! Session repair: fixes history inconsistencies.

use serde::{Deserialize, Serialize};

/// A message in the session history (simplified).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    /// Unique message ID.
    pub id: String,
    /// Role (system, user, assistant, tool).
    pub role: String,
    /// Content (may be empty for tool calls).
    pub content: String,
    /// Tool call ID (for tool results linking).
    pub tool_call_id: Option<String>,
}

/// Report of repairs applied to a session.
#[derive(Debug, Clone, Default)]
pub struct RepairReport {
    /// Empty messages removed.
    pub empty_removed: usize,
    /// Orphan tool results removed.
    pub orphans_removed: usize,
    /// Duplicate system messages removed.
    pub duplicates_removed: usize,
    /// Consecutive same-role messages merged.
    pub merged: usize,
}

impl RepairReport {
    /// Total fixes applied.
    pub fn total_fixes(&self) -> usize {
        self.empty_removed + self.orphans_removed + self.duplicates_removed + self.merged
    }
}

/// Repair session history by fixing known inconsistencies.
pub fn repair_history(messages: &[HistoryMessage]) -> (Vec<HistoryMessage>, RepairReport) {
    let mut report = RepairReport::default();
    let mut result: Vec<HistoryMessage> = Vec::with_capacity(messages.len());

    // Pass 1: Collect tool call IDs.
    let tool_call_ids: std::collections::HashSet<String> = messages
        .iter()
        .filter(|m| m.role == "assistant" && m.tool_call_id.is_some())
        .filter_map(|m| m.tool_call_id.clone())
        .collect();

    let mut seen_system = false;

    for msg in messages {
        // Skip empty messages.
        if msg.content.is_empty() && msg.tool_call_id.is_none() {
            report.empty_removed += 1;
            continue;
        }

        // Skip duplicate system messages.
        if msg.role == "system" {
            if seen_system {
                report.duplicates_removed += 1;
                continue;
            }
            seen_system = true;
        }

        // Skip orphan tool results.
        if msg.role == "tool" {
            if let Some(ref tc_id) = msg.tool_call_id {
                if !tool_call_ids.contains(tc_id) {
                    report.orphans_removed += 1;
                    continue;
                }
            }
        }

        // Merge consecutive same-role messages.
        if let Some(last) = result.last_mut() {
            if last.role == msg.role && msg.role != "system" && msg.tool_call_id.is_none() {
                last.content = format!("{}\n{}", last.content, msg.content);
                report.merged += 1;
                continue;
            }
        }

        result.push(msg.clone());
    }

    (result, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: &str, content: &str) -> HistoryMessage {
        HistoryMessage {
            id: id.into(),
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
        }
    }

    #[test]
    fn test_repair_removes_empty_messages() {
        let messages = vec![
            msg("1", "user", "hello"),
            msg("2", "assistant", ""),
            msg("3", "assistant", "hi"),
        ];
        let (result, report) = repair_history(&messages);
        assert_eq!(result.len(), 2);
        assert_eq!(report.empty_removed, 1);
    }

    #[test]
    fn test_repair_removes_duplicate_system() {
        let messages = vec![
            msg("1", "system", "You are an AI."),
            msg("2", "system", "You are helpful."),
            msg("3", "user", "hi"),
        ];
        let (result, report) = repair_history(&messages);
        assert_eq!(result.len(), 2); // 1 system + 1 user
        assert_eq!(report.duplicates_removed, 1);
    }

    #[test]
    fn test_repair_merges_consecutive_same_role() {
        let messages = vec![
            msg("1", "user", "part 1"),
            msg("2", "user", "part 2"),
            msg("3", "assistant", "response"),
        ];
        let (result, report) = repair_history(&messages);
        assert_eq!(result.len(), 2);
        assert!(result[0].content.contains("part 1"));
        assert!(result[0].content.contains("part 2"));
        assert_eq!(report.merged, 1);
    }

    #[test]
    fn test_repair_clean_history_unchanged() {
        let messages = vec![
            msg("1", "system", "sys"),
            msg("2", "user", "hello"),
            msg("3", "assistant", "hi"),
        ];
        let (result, report) = repair_history(&messages);
        assert_eq!(result.len(), 3);
        assert_eq!(report.total_fixes(), 0);
    }
}
