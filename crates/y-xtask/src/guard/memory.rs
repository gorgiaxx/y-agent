//! Memory-budget guard.
//!
//! `docs/standards/MEMORY_BUDGET.md` records the explicit ceilings that bound
//! y-agent's in-memory growth. A budget document that is not machine-checked
//! rots the first time someone changes a constant, so this guard re-reads every
//! documented constant from its source file and fails when the two disagree.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Path of the budget document, relative to the repository root.
pub const BUDGET_DOC: &str = "docs/standards/MEMORY_BUDGET.md";

/// One documented ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetRow {
    /// Constant name, for example `MAX_TOASTS`.
    pub name: String,
    /// Source file declaring the constant, relative to the repository root.
    pub path: String,
    /// Documented value expression, normalized.
    pub value: String,
}

/// Strip formatting noise so `20 * 1024 * 1024` and `20*1024*1024` compare equal.
fn normalize_value(raw: &str) -> String {
    raw.chars()
        .filter(|character| !character.is_whitespace() && *character != '_')
        .collect()
}

/// Extract the cell contents of one Markdown table row.
fn row_cells(line: &str) -> Option<Vec<&str>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return None;
    }
    let cells: Vec<&str> = trimmed
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect();
    (cells.len() >= 3).then_some(cells)
}

/// Unwrap a `` `code` `` cell, returning `None` when it is not code-formatted.
fn code_cell(cell: &str) -> Option<&str> {
    cell.strip_prefix('`')?.strip_suffix('`')
}

/// Parse the budget rows out of the document text.
///
/// Rows are recognized structurally: any table row whose first three cells are
/// all code-formatted. Prose, headings, and the separator row are ignored, so
/// the document stays readable.
pub fn parse_rows(document: &str) -> Vec<BudgetRow> {
    document
        .lines()
        .filter_map(row_cells)
        .filter_map(|cells| {
            let name = code_cell(cells[0])?;
            let path = code_cell(cells[1])?;
            let value = code_cell(cells[2])?;
            (!name.is_empty() && path.contains('/')).then(|| BudgetRow {
                name: name.to_string(),
                path: path.to_string(),
                value: normalize_value(value),
            })
        })
        .collect()
}

/// Find the value expression of `const NAME` in `source`.
pub fn constant_value(source: &str, name: &str) -> Option<String> {
    let needle = format!("const {name}:");
    let start = source.find(&needle)?;
    let rest = &source[start + needle.len()..];
    let equals = rest.find('=')?;
    let end = rest[equals + 1..].find(';')?;
    Some(normalize_value(&rest[equals + 1..equals + 1 + end]))
}

/// Verify every documented ceiling against its source declaration.
pub fn check(root: &Path) -> Result<()> {
    let doc_path = root.join(BUDGET_DOC);
    let document =
        fs::read_to_string(&doc_path).with_context(|| format!("read {}", doc_path.display()))?;
    let rows = parse_rows(&document);
    if rows.is_empty() {
        bail!("{BUDGET_DOC} declares no budget rows; the guard would be vacuous");
    }

    let mut failures = Vec::new();
    for row in &rows {
        let source_path = root.join(&row.path);
        let Ok(source) = fs::read_to_string(&source_path) else {
            failures.push(format!(
                "{}: documented source {} does not exist",
                row.name, row.path
            ));
            continue;
        };
        match constant_value(&source, &row.name) {
            None => failures.push(format!(
                "{}: no `const {}` declared in {}",
                row.name, row.name, row.path
            )),
            Some(actual) if actual != row.value => failures.push(format!(
                "{}: {} declares {} but {BUDGET_DOC} records {}",
                row.name, row.path, actual, row.value
            )),
            Some(_) => {}
        }
    }

    if failures.is_empty() {
        println!("memory: ok ({} documented ceiling(s))", rows.len());
        return Ok(());
    }
    for failure in &failures {
        println!("  FAIL {failure}");
    }
    bail!(
        "memory budget guard failed: {} ceiling(s) disagree with {BUDGET_DOC}. \
         Changing a ceiling requires updating the document and justifying the change.",
        failures.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_formatted_rows_only() {
        let document = "\
| Constant | Source | Ceiling | Why |
| --- | --- | --- | --- |
| `MAX_TOASTS` | `crates/y-cli/src/tui/state.rs` | `5` | screen space |
| plain | text | row | ignored |
";
        let rows = parse_rows(document);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "MAX_TOASTS");
        assert_eq!(rows[0].value, "5");
    }

    #[test]
    fn parse_ignores_separator_and_prose() {
        assert!(parse_rows("Some prose.\n\n| --- | --- | --- |\n").is_empty());
    }

    #[test]
    fn value_normalization_ignores_underscores_and_spacing() {
        let document =
            "| `A` | `crates/x/src/a.rs` | `20 * 1024 * 1024` |\n| `B` | `crates/x/src/b.rs` | `100_000` |\n";
        let rows = parse_rows(document);
        assert_eq!(rows[0].value, "20*1024*1024");
        assert_eq!(rows[1].value, "100000");
    }

    #[test]
    fn reads_constant_value_from_source() {
        let source = "/// doc\npub const MAX_TOASTS: usize = 5;\n";
        assert_eq!(constant_value(source, "MAX_TOASTS"), Some("5".to_string()));
    }

    #[test]
    fn reads_expression_valued_constant() {
        let source = "const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;\n";
        assert_eq!(
            constant_value(source, "MAX_IMAGE_BYTES"),
            Some("20*1024*1024".to_string())
        );
    }

    #[test]
    fn missing_constant_is_reported_as_absent() {
        assert_eq!(constant_value("fn main() {}", "MAX_TOASTS"), None);
    }

    #[test]
    fn does_not_match_a_similarly_named_constant() {
        let source = "const OTHER_MAX_TOASTS: usize = 9;\nconst MAX_TOASTS: usize = 5;\n";
        assert_eq!(constant_value(source, "MAX_TOASTS"), Some("5".to_string()));
    }
}
