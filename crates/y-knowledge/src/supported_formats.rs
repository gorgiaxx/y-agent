//! Supported file formats for knowledge ingestion.
//!
//! Centralises the list of file extensions that the knowledge pipeline
//! can ingest, plus helpers for checking paths and expanding directories.

use std::path::{Path, PathBuf};

/// File extensions accepted by the knowledge ingestion pipeline.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Markdown
    "md", "markdown", "mdx",
    // Plain text & docs
    "txt", "text", "rst", "adoc", "org", "rtf",
    // Data / config
    "json", "jsonl", "yaml", "yml", "toml", "csv", "tsv",
    "xml", "html", "htm", "svg",
    "ini", "cfg", "conf", "env", "properties",
    // Source code
    "rs", "py", "js", "ts", "jsx", "tsx", "go", "java",
    "c", "h", "cpp", "hpp", "cc", "cs", "rb", "php",
    "swift", "kt", "kts", "scala", "lua", "r", "pl",
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd",
    "sql", "graphql", "gql",
    // Misc text
    "log", "diff", "patch", "tex", "bib",
    "css", "scss", "less", "sass",
    "vue", "svelte", "astro",
    "dockerfile", "makefile", "cmake",
];

/// Well-known extensionless filenames accepted for ingestion.
const KNOWN_EXTENSIONLESS: &[&str] = &[
    "dockerfile",
    "makefile",
    "cmakelists.txt",
    "readme",
    "license",
    "changelog",
];

/// Check if a path has a supported extension (or a known extensionless name).
pub fn is_supported(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext.is_empty() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            return KNOWN_EXTENSIONLESS.contains(&lower.as_str());
        }
        return false;
    }

    SUPPORTED_EXTENSIONS.contains(&ext.as_str())
}

/// Recursively collect files with supported extensions from a directory.
///
/// Hidden files/directories (names starting with `.`) are skipped.
pub fn expand_directory(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_recursive(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip hidden files/directories.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        if path.is_dir() {
            collect_recursive(&path, out)?;
        } else if path.is_file() && is_supported(&path) {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_supported_extensions() {
        assert!(is_supported(Path::new("file.rs")));
        assert!(is_supported(Path::new("file.md")));
        assert!(is_supported(Path::new("file.py")));
        assert!(is_supported(Path::new("file.json")));
        assert!(is_supported(Path::new("file.tsx")));
        assert!(!is_supported(Path::new("file.exe")));
        assert!(!is_supported(Path::new("file.png")));
        assert!(!is_supported(Path::new("file.pdf")));
    }

    #[test]
    fn test_known_extensionless() {
        assert!(is_supported(Path::new("Dockerfile")));
        assert!(is_supported(Path::new("Makefile")));
        assert!(is_supported(Path::new("README")));
        assert!(is_supported(Path::new("LICENSE")));
        assert!(!is_supported(Path::new("randomfile")));
    }

    #[test]
    fn test_case_insensitive_extension() {
        assert!(is_supported(Path::new("file.RS")));
        assert!(is_supported(Path::new("file.Md")));
        assert!(is_supported(Path::new("file.JSON")));
    }
}
