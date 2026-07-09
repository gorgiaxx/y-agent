//! Structural code summaries powered by tree-sitter.
//!
//! Parses source code into an AST and folds large bodies (function bodies,
//! class bodies, large literals, multi-line comments) into `…` markers,
//! keeping the structural skeleton (signatures, imports, key control flow).
//!
//! This lets `FileReadTool` return a compact but structurally complete view
//! of a file in a single call, instead of forcing the agent to paginate
//! through a 3000-line file with multiple `line_offset`/`limit` calls.
//!
//! Design reference: omp `crates/pi-ast/src/summary.rs` (Can Bölük's BFS
//! unfold algorithm), simplified for y-agent's needs.

use std::path::Path;

use tree_sitter::{Language, Node, Parser};

// ---------------------------------------------------------------------------
// Language resolution
// ---------------------------------------------------------------------------

/// Resolve a tree-sitter language from a file extension.
fn language_from_path(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "ts" | "tsx" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE.into(),
        "py" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" | "h" => tree_sitter_c::LANGUAGE.into(),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => tree_sitter_cpp::LANGUAGE.into(),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A span of lines (1-based, inclusive) that can be elided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LineSpan {
    start: u32,
    end: u32,
}

impl LineSpan {
    fn lines(self) -> u32 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }
}

/// Result of a structural summary operation.
#[derive(Debug, Clone)]
pub struct SummaryResult {
    /// True when tree-sitter parsed the source without syntax errors.
    pub parsed: bool,
    /// True when at least one elision span was emitted.
    pub elided: bool,
    /// Total source lines.
    pub total_lines: u32,
    /// The summarized text with `…` markers for elided regions.
    pub text: String,
    /// Elided line ranges (1-based, inclusive), for the recovery footer.
    pub elided_ranges: Vec<(u32, u32)>,
    /// Total lines hidden by elision.
    pub elided_lines: u32,
}

// ---------------------------------------------------------------------------
// Elidable node detection
// ---------------------------------------------------------------------------

/// Minimum body lines before a node is eligible for elision.
const MIN_BODY_LINES: u32 = 4;

/// Minimum comment lines before a block comment is eligible for elision.
const MIN_COMMENT_LINES: u32 = 6;

/// Node types that represent elidable bodies, per language family.
fn is_elidable_body(node: &Node) -> Option<LineSpan> {
    let kind = node.kind();

    // Block statements / bodies that span multiple lines.
    let body_kinds = [
        "statement_block",        // JS/TS function body
        "class_body",             // JS/TS class body
        "declaration_list",       // Rust/Go block
        "field_declaration_list", // C/C++ struct/union body
        "compound_statement",     // C/C++/Java block
        "block",                  // Go/Java block
        "module_body",            // Rust module
        "impl_item",              // Rust impl body (elide the whole impl)
        "enum_body",              // Rust enum body
        "struct_expression",      // Rust struct literal
        "array",                  // Large array literals
        "object",                 // JS/TS object literal
    ];

    // Multi-line comment nodes.
    let comment_kinds = ["block_comment", "comment"];

    if body_kinds.contains(&kind) {
        let start = node.start_position().row;
        let end = node.end_position().row;
        let span = LineSpan {
            start: start as u32 + 1,
            end: end as u32 + 1,
        };
        if span.lines() >= MIN_BODY_LINES {
            return Some(span);
        }
    }

    if comment_kinds.contains(&kind) {
        let start = node.start_position().row;
        let end = node.end_position().row;
        let span = LineSpan {
            start: start as u32 + 1,
            end: end as u32 + 1,
        };
        if span.lines() >= MIN_COMMENT_LINES {
            return Some(span);
        }
    }

    // Check for large string/byte literals.
    if matches!(
        kind,
        "string" | "raw_string" | "string_literal" | "template_string"
    ) {
        let start = node.start_position().row;
        let end = node.end_position().row;
        let span = LineSpan {
            start: start as u32 + 1,
            end: end as u32 + 1,
        };
        if span.lines() >= MIN_BODY_LINES {
            return Some(span);
        }
    }

    None
}

fn collect_elidable(node: Node, spans: &mut Vec<LineSpan>) {
    if let Some(span) = is_elidable_body(&node) {
        spans.push(span);
        // Don't recurse into elided bodies — the outer span covers the inner.
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_elidable(child, spans);
    }
}

// ---------------------------------------------------------------------------
// Summary construction
// ---------------------------------------------------------------------------

/// Summarize source code by folding large bodies into `…` markers.
///
/// Returns `None` if the language is unsupported or the source fails to parse.
pub fn summarize_code(source: &str, path: &Path) -> Option<SummaryResult> {
    let language = language_from_path(path)?;
    let total_lines = source.lines().count() as u32;
    if total_lines == 0 {
        return None;
    }

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    // Collect elidable spans.
    let mut spans = Vec::new();
    collect_elidable(root, &mut spans);

    // Sort by start line and remove nested spans (keep outermost).
    spans.sort_by_key(|s| (s.start, s.end));
    spans.dedup_by(|a, b| a.start >= b.start && a.end <= b.end);

    if spans.is_empty() {
        return Some(SummaryResult {
            parsed: true,
            elided: false,
            total_lines,
            text: source.to_string(),
            elided_ranges: Vec::new(),
            elided_lines: 0,
        });
    }

    // Build the summarized text: keep lines outside elided spans, replace
    // elided regions with `…`.
    let lines: Vec<&str> = source.lines().collect();
    let mut result = String::new();
    let mut elided_ranges = Vec::new();
    let mut elided_lines = 0u32;
    let mut i = 0u32; // 0-based line index

    while i < total_lines {
        // Check if line (i+1) falls inside an elided span.
        let in_span = spans
            .iter()
            .find(|s| (i + 1) >= s.start && (i + 1) <= s.end);

        if let Some(span) = in_span {
            // Emit `…` and skip to end of span.
            if !result.is_empty() {
                result.push('\n');
            }
            result.push('…');
            elided_ranges.push((span.start, span.end));
            elided_lines += span.lines();
            i = span.end; // skip to end (0-based: span.end is 1-based inclusive)
        } else {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(lines[i as usize]);
            i += 1;
        }
    }

    Some(SummaryResult {
        parsed: true,
        elided: true,
        total_lines,
        text: result,
        elided_ranges,
        elided_lines,
    })
}

/// Build a footer telling the agent how to re-read elided ranges.
///
/// Example: `[…45ln elided; re-read needed ranges e.g. src/main.rs:10-55,80-120`
pub fn format_elision_footer(path: &str, ranges: &[(u32, u32)], elided_lines: u32) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    let sample_count = ranges.len().min(3);
    let selector = ranges
        .iter()
        .take(sample_count)
        .map(|(s, e)| format!("{s}-{e}"))
        .collect::<Vec<_>>()
        .join(",");
    let tail = if ranges.len() > sample_count {
        format!(", e.g. {path}:{selector}")
    } else {
        format!(" with {path}:{selector}")
    };
    format!("[…{elided_lines}ln elided; re-read needed ranges{tail}]")
}

// ---------------------------------------------------------------------------
// LRU cache
// ---------------------------------------------------------------------------

/// A simple bounded LRU cache for code summaries.
///
/// Keyed by content hash + path, so a file that hasn't changed reuses the
/// cached summary without re-parsing.
pub struct SummaryCache {
    entries: std::collections::HashMap<String, Option<SummaryResult>>,
    order: std::collections::VecDeque<String>,
    capacity: usize,
}

impl SummaryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
            capacity,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&Option<SummaryResult>> {
        if self.entries.contains_key(key) {
            // Move to front (most recently used).
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
            }
            self.order.push_front(key.to_string());
            self.entries.get(key)
        } else {
            None
        }
    }

    pub fn put(&mut self, key: String, value: Option<SummaryResult>) {
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            // Evict least recently used.
            if let Some(old_key) = self.order.pop_back() {
                self.entries.remove(&old_key);
            }
        }
        self.order.push_front(key.clone());
        self.entries.insert(key, value);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn summarize(code: &str, ext: &str) -> Option<SummaryResult> {
        let path = PathBuf::from(format!("test.{ext}"));
        summarize_code(code, &path)
    }

    #[test]
    fn test_summarize_rust_function_body() {
        let code = r#"fn main() {
    let x = 1;
    let y = 2;
    let z = 3;
    let w = 4;
    let v = 5;
    println!("{}", x + y + z + w + v);
}

fn small() {
    println!("hi");
}
"#;
        let result = summarize(code, "rs").unwrap();
        assert!(result.parsed);
        assert!(result.elided);
        assert!(result.elided_lines > 0);
        // The small function should NOT be elided (only 3 lines).
        assert!(result.text.contains("fn small()"));
        // The main function body should be elided.
        assert!(result.text.contains("…"));
    }

    #[test]
    fn test_summarize_short_file_no_elision() {
        let code = "fn main() {\n    println!(\"hi\");\n}\n";
        let result = summarize(code, "rs").unwrap();
        assert!(result.parsed);
        assert!(!result.elided);
        assert_eq!(result.text, code);
    }

    #[test]
    fn test_unsupported_language() {
        let code = "some unknown language";
        let result = summarize(code, "xyz");
        assert!(result.is_none());
    }

    #[test]
    fn test_elision_footer() {
        let footer = format_elision_footer("src/main.rs", &[(10, 55), (80, 120)], 87);
        assert!(footer.contains("87ln elided"));
        assert!(footer.contains("src/main.rs:10-55,80-120"));
    }

    #[test]
    fn test_cache_hit_miss() {
        let mut cache = SummaryCache::new(2);
        cache.put("key1".into(), None);
        assert!(cache.get("key1").is_some());
        assert!(cache.get("key2").is_none());
    }

    #[test]
    fn test_cache_eviction() {
        let mut cache = SummaryCache::new(2);
        cache.put("k1".into(), None);
        cache.put("k2".into(), None);
        cache.put("k3".into(), None); // should evict k1
        assert!(cache.get("k1").is_none());
        assert!(cache.get("k2").is_some());
        assert!(cache.get("k3").is_some());
    }
}
