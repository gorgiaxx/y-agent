//! ARIA and DOM snapshot formatting.
//!
//! Converts raw CDP accessibility tree data into structured snapshots
//! that the LLM can use to reference interactive elements via `@eN` refs.

use serde::{Deserialize, Serialize};

/// A node in the ARIA (accessibility) snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaSnapshotNode {
    /// Reference ID for this node (e.g., "e1", "e2").
    pub ref_id: String,
    /// ARIA role (button, link, textbox, etc.).
    pub role: String,
    /// Accessible name.
    pub name: String,
    /// Value (for inputs, sliders, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Description text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Tree depth (0 = root).
    pub depth: usize,
    /// CDP backend DOM node ID — used to resolve the element for click/type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_dom_node_id: Option<i64>,
}

/// A node in the DOM snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomSnapshotNode {
    /// Reference ID.
    pub ref_id: String,
    /// Parent reference ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    /// Tree depth.
    pub depth: usize,
    /// HTML tag name.
    pub tag: String,
    /// Element ID attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Class name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    /// ARIA role attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Text content (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Href for links.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

/// Snapshot format selection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormat {
    /// Accessibility tree snapshot (ARIA roles + names).
    #[default]
    Aria,
    /// DOM tree snapshot (HTML elements).
    Dom,
}

/// Raw AX node from CDP `Accessibility.getFullAXTree`.
#[derive(Debug, Deserialize)]
pub struct RawAxNode {
    #[serde(rename = "nodeId")]
    pub node_id: Option<String>,
    pub role: Option<AxValue>,
    pub name: Option<AxValue>,
    pub value: Option<AxValue>,
    pub description: Option<AxValue>,
    #[serde(rename = "childIds", default)]
    pub child_ids: Vec<String>,
    #[serde(rename = "backendDOMNodeId")]
    pub backend_dom_node_id: Option<i64>,
    #[serde(default)]
    pub properties: Vec<AxProperty>,
}

/// CDP AX property (used for checked, disabled, required state etc.).
#[derive(Debug, Deserialize)]
pub struct AxProperty {
    pub name: Option<String>,
    pub value: Option<AxValue>,
}

/// CDP AX value wrapper.
#[derive(Debug, Deserialize)]
pub struct AxValue {
    pub value: Option<serde_json::Value>,
}

impl AxValue {
    /// Extract the string value.
    pub fn as_str(&self) -> String {
        match &self.value {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    }
}

/// ARIA roles that represent interactive elements.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "option",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "switch",
    "slider",
    "spinbutton",
    "tab",
    "treeitem",
];

/// ARIA roles that are structural containers worth keeping as context
/// to show hierarchy, even in interactive-only mode.
#[allow(dead_code)]
const STRUCTURAL_ROLES: &[&str] = &[
    "navigation",
    "main",
    "form",
    "dialog",
    "alertdialog",
    "menu",
    "menubar",
    "tablist",
    "toolbar",
    "group",
    "heading",
    "list",
];

/// Check if a role is interactive.
fn is_interactive_role(role: &str) -> bool {
    let r = role.to_lowercase();
    INTERACTIVE_ROLES.iter().any(|&ir| r == ir)
}

/// Check if a role is structural (useful container).
#[allow(dead_code)]
fn is_structural_role(role: &str) -> bool {
    let r = role.to_lowercase();
    STRUCTURAL_ROLES.iter().any(|&sr| r == sr)
}

/// Format raw AX nodes from CDP into a flat ARIA snapshot.
///
/// When `interactive_only` is true, only interactive elements and their
/// structural ancestors are included — dramatically reducing token usage.
///
/// Walks the tree depth-first up to `limit` nodes.
pub fn format_aria_snapshot(
    nodes: &[RawAxNode],
    limit: usize,
    interactive_only: bool,
) -> Vec<AriaSnapshotNode> {
    use std::collections::{HashMap, HashSet};

    let by_id: HashMap<&str, &RawAxNode> = nodes
        .iter()
        .filter_map(|n| n.node_id.as_deref().map(|id| (id, n)))
        .collect();

    // Find root: a node not referenced as a child.
    let referenced: HashSet<&str> = nodes
        .iter()
        .flat_map(|n| n.child_ids.iter().map(String::as_str))
        .collect();

    let root = nodes
        .iter()
        .find(|n| {
            n.node_id
                .as_deref()
                .is_some_and(|id| !referenced.contains(id))
        })
        .or(nodes.first());

    let Some(root) = root else {
        return vec![];
    };
    let Some(root_id) = root.node_id.as_deref() else {
        return vec![];
    };

    // If interactive_only, first pass: collect IDs of interactive nodes
    // and all their ancestors so we can show structural context.
    let keep_ids: Option<HashSet<&str>> = if interactive_only {
        // Build parent map
        let mut parent_map: HashMap<&str, &str> = HashMap::new();
        for node in nodes {
            let Some(parent_id) = node.node_id.as_deref() else {
                continue;
            };
            for child_id in &node.child_ids {
                parent_map.insert(child_id.as_str(), parent_id);
            }
        }

        let mut keep = HashSet::new();
        for node in nodes {
            let Some(node_id) = node.node_id.as_deref() else {
                continue;
            };
            let role = node.role.as_ref().map_or_else(String::new, AxValue::as_str);
            if is_interactive_role(&role) {
                // Mark this node and all ancestors
                let mut current = Some(node_id);
                while let Some(id) = current {
                    if !keep.insert(id) {
                        break; // already in set, ancestors already added
                    }
                    current = parent_map.get(id).copied();
                }
            }
        }
        Some(keep)
    } else {
        None
    };

    let mut out = Vec::new();
    let mut stack: Vec<(&str, usize)> = vec![(root_id, 0)];

    while let Some((id, depth)) = stack.pop() {
        if out.len() >= limit {
            break;
        }
        let Some(node) = by_id.get(id) else {
            continue;
        };

        let role = node.role.as_ref().map_or_else(String::new, AxValue::as_str);
        let name = node.name.as_ref().map_or_else(String::new, AxValue::as_str);

        // Apply interactive filter
        if let Some(ref keep) = keep_ids {
            if !keep.contains(id) {
                // Still push children — they might be in the keep set
                let children: Vec<&str> = node
                    .child_ids
                    .iter()
                    .filter(|c| by_id.contains_key(c.as_str()))
                    .map(String::as_str)
                    .collect();
                for child_id in children.into_iter().rev() {
                    stack.push((child_id, depth));
                }
                continue;
            }
        }

        // Skip generic/noise roles with empty names
        if matches!(
            role.as_str(),
            "none" | "generic" | "GenericContainer" | "InlineTextBox" | "LineBreak"
        ) {
            let children: Vec<&str> = node
                .child_ids
                .iter()
                .filter(|c| by_id.contains_key(c.as_str()))
                .map(String::as_str)
                .collect();
            for child_id in children.into_iter().rev() {
                stack.push((child_id, depth));
            }
            continue;
        }

        let value = node
            .value
            .as_ref()
            .map(AxValue::as_str)
            .filter(|s| !s.is_empty());
        let description = node
            .description
            .as_ref()
            .map(AxValue::as_str)
            .filter(|s| !s.is_empty());

        let ref_id = format!("e{}", out.len() + 1);

        out.push(AriaSnapshotNode {
            ref_id,
            role: if role.is_empty() {
                "unknown".into()
            } else {
                role
            },
            name,
            value,
            description,
            depth,
            backend_dom_node_id: node.backend_dom_node_id,
        });

        // Push children in reverse order for correct DFS order.
        let children: Vec<&str> = node
            .child_ids
            .iter()
            .filter(|c| by_id.contains_key(c.as_str()))
            .map(String::as_str)
            .collect();
        for child_id in children.into_iter().rev() {
            stack.push((child_id, depth + 1));
        }
    }

    out
}

/// Format an ARIA snapshot into text suitable for LLM consumption.
///
/// Output format:
/// ```text
/// [@e1] button "Submit"
///   [@e2] textbox "Email" value="user@example.com"
/// ```
///
/// The `@eN` refs can be passed directly to click/type actions.
pub fn aria_snapshot_to_text(nodes: &[AriaSnapshotNode]) -> String {
    let mut lines = Vec::with_capacity(nodes.len());
    for node in nodes {
        let indent = "  ".repeat(node.depth);
        let mut line = format!("{indent}[@{}] {}", node.ref_id, node.role);
        if !node.name.is_empty() {
            line.push_str(&format!(" \"{}\"", node.name));
        }
        if let Some(value) = &node.value {
            line.push_str(&format!(" value=\"{value}\""));
        }
        if let Some(desc) = &node.description {
            line.push_str(&format!(" desc=\"{desc}\""));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Truncate text output to a maximum character count.
///
/// When truncated, appends a `[truncated: showing N of M chars]` note.
pub fn truncate_output(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    // Find char boundary
    match content.char_indices().nth(max_chars).map(|(i, _)| i) {
        Some(byte_offset) => {
            let total_chars = content.chars().count();
            format!(
                "{}\n[truncated: showing {} of {} chars]",
                &content[..byte_offset],
                max_chars,
                total_chars
            )
        }
        None => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aria_snapshot_to_text() {
        let nodes = vec![
            AriaSnapshotNode {
                ref_id: "e1".into(),
                role: "document".into(),
                name: "Example Page".into(),
                value: None,
                description: None,
                depth: 0,
                backend_dom_node_id: Some(1),
            },
            AriaSnapshotNode {
                ref_id: "e2".into(),
                role: "button".into(),
                name: "Submit".into(),
                value: None,
                description: None,
                depth: 1,
                backend_dom_node_id: Some(5),
            },
        ];
        let text = aria_snapshot_to_text(&nodes);
        assert!(text.contains("[@e1] document \"Example Page\""));
        assert!(text.contains("  [@e2] button \"Submit\""));
    }

    #[test]
    fn test_truncate_output() {
        let content = "Hello, World!"; // 13 chars
        assert_eq!(truncate_output(content, 100), content);

        let truncated = truncate_output(content, 5);
        assert!(truncated.starts_with("Hello"));
        assert!(truncated.contains("[truncated:"));
    }

    #[test]
    fn test_interactive_roles() {
        assert!(is_interactive_role("button"));
        assert!(is_interactive_role("textbox"));
        assert!(is_interactive_role("link"));
        assert!(!is_interactive_role("document"));
        assert!(!is_interactive_role("generic"));
    }
}
