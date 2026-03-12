//! Output formatting: JSON, table, and plain text modes.

use serde::Serialize;

/// Output rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// JSON output (suitable for piping to jq).
    Json,
    /// Tabular output with aligned columns.
    Table,
    /// Plain text (human-friendly default).
    Plain,
}

impl OutputMode {
    /// Parse from string, defaulting to Plain.
    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "json" => Self::Json,
            "table" => Self::Table,
            _ => Self::Plain,
        }
    }
}

/// A row in a table display.
#[derive(Debug)]
pub struct TableRow {
    pub cells: Vec<String>,
}

/// Format a serializable value according to the output mode.
pub fn format_value<T: Serialize>(value: &T, mode: OutputMode) -> String {
    match mode {
        OutputMode::Json => serde_json::to_string_pretty(value)
            .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}")),
        OutputMode::Table | OutputMode::Plain => {
            // For simple values, use the JSON representation as a fallback.
            serde_json::to_string_pretty(value).unwrap_or_default()
        }
    }
}

/// Format a table with headers and rows.
pub fn format_table(headers: &[&str], rows: &[TableRow]) -> String {
    if rows.is_empty() {
        return "(no data)".to_string();
    }

    // Determine column widths.
    let num_cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    for row in rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i < num_cols {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let mut output = String::new();

    // Header row.
    for (i, header) in headers.iter().enumerate() {
        if i > 0 {
            output.push_str("  ");
        }
        output.push_str(&format!("{:<width$}", header, width = widths[i]));
    }
    output.push('\n');

    // Separator.
    for (i, width) in widths.iter().enumerate() {
        if i > 0 {
            output.push_str("  ");
        }
        output.push_str(&"-".repeat(*width));
    }
    output.push('\n');

    // Data rows.
    for row in rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i >= num_cols {
                break;
            }
            if i > 0 {
                output.push_str("  ");
            }
            output.push_str(&format!("{:<width$}", cell, width = widths[i]));
        }
        output.push('\n');
    }

    output
}

/// Print a status line with a label and value.
pub fn print_status(label: &str, value: &str) {
    println!("{label}: {value}");
}

/// Print an error message to stderr.
pub fn print_error(msg: &str) {
    eprintln!("error: {msg}");
}

/// Print a success message.
pub fn print_success(msg: &str) {
    println!("✓ {msg}");
}

/// Print a warning message to stderr.
pub fn print_warning(msg: &str) {
    eprintln!("warning: {msg}");
}

/// Print an info message.
pub fn print_info(msg: &str) {
    println!("ℹ {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_mode_parse() {
        assert_eq!(OutputMode::from_str_or_default("json"), OutputMode::Json);
        assert_eq!(OutputMode::from_str_or_default("table"), OutputMode::Table);
        assert_eq!(OutputMode::from_str_or_default("plain"), OutputMode::Plain);
        assert_eq!(
            OutputMode::from_str_or_default("unknown"),
            OutputMode::Plain
        );
    }

    #[test]
    fn test_format_table() {
        let headers = &["ID", "Name", "Status"];
        let rows = vec![
            TableRow {
                cells: vec!["1".into(), "test".into(), "active".into()],
            },
            TableRow {
                cells: vec!["2".into(), "demo".into(), "archived".into()],
            },
        ];

        let result = format_table(headers, &rows);
        assert!(result.contains("ID"));
        assert!(result.contains("test"));
        assert!(result.contains("archived"));
        assert!(result.contains("--"));
    }

    #[test]
    fn test_format_table_empty() {
        let result = format_table(&["A"], &[]);
        assert_eq!(result, "(no data)");
    }

    #[test]
    fn test_format_value_json() {
        let val = serde_json::json!({"key": "value"});
        let result = format_value(&val, OutputMode::Json);
        assert!(result.contains("key"));
        assert!(result.contains("value"));
    }
}
