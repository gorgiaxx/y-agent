//! ARIA and DOM snapshot formatting.
//!
//! Converts raw CDP accessibility tree data into structured snapshots
//! that the LLM can use to reference interactive elements.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormat {
    /// Accessibility tree snapshot (ARIA roles + names).
    Aria,
    /// DOM tree snapshot (HTML elements).
    Dom,
}

impl Default for SnapshotFormat {
    fn default() -> Self {
        Self::Aria
    }
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

/// Format raw AX nodes from CDP into a flat ARIA snapshot.
///
/// Walks the tree depth-first up to `limit` nodes.
pub fn format_aria_snapshot(nodes: &[RawAxNode], limit: usize) -> Vec<AriaSnapshotNode> {
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
        .find(|n| n.node_id.as_deref().is_some_and(|id| !referenced.contains(id)))
        .or(nodes.first());

    let Some(root) = root else {
        return vec![];
    };
    let Some(root_id) = root.node_id.as_deref() else {
        return vec![];
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

        let role = node.role.as_ref().map_or_else(String::new, |v| v.as_str());
        let name = node.name.as_ref().map_or_else(String::new, |v| v.as_str());
        let value = node.value.as_ref().map(|v| v.as_str()).filter(|s| !s.is_empty());
        let description = node
            .description
            .as_ref()
            .map(|v| v.as_str())
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
        lines.push(line);
    }
    lines.join("\n")
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
            },
            AriaSnapshotNode {
                ref_id: "e2".into(),
                role: "button".into(),
                name: "Submit".into(),
                value: None,
                description: None,
                depth: 1,
            },
        ];
        let text = aria_snapshot_to_text(&nodes);
        assert!(text.contains("[@e1] document \"Example Page\""));
        assert!(text.contains("  [@e2] button \"Submit\""));
    }
}
