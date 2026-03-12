//! Skill diff: compares two versions of a skill.
//!
//! Produces line-level diffs between the root documents and sub-documents
//! of two skill version snapshots.

/// A diff between two versions of a skill.
#[derive(Debug, Clone)]
pub struct SkillDiff {
    /// Skill name.
    pub skill_name: String,
    /// Hash of the "before" version.
    pub hash_a: String,
    /// Hash of the "after" version.
    pub hash_b: String,
    /// Per-file diffs.
    pub file_diffs: Vec<FileDiff>,
}

/// A diff for a single file within a skill.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Relative file path (e.g., `root.md`, `details/tone.md`).
    pub path: String,
    /// Type of change.
    pub change_type: ChangeType,
    /// Unified diff hunks (line-level).
    pub hunks: Vec<DiffHunk>,
}

/// Type of file change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeType {
    /// File was added in version B.
    Added,
    /// File was removed from version A.
    Removed,
    /// File was modified between versions.
    Modified,
    /// File is identical in both versions.
    Unchanged,
}

/// A contiguous block of changed lines.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// Starting line in version A (1-indexed).
    pub old_start: usize,
    /// Number of lines from version A in this hunk.
    pub old_count: usize,
    /// Starting line in version B (1-indexed).
    pub new_start: usize,
    /// Number of lines from version B in this hunk.
    pub new_count: usize,
    /// The diff lines (prefixed with ` `, `+`, or `-`).
    pub lines: Vec<DiffLine>,
}

/// A single line in a diff hunk.
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// The type of this line.
    pub kind: DiffLineKind,
    /// The line content (without prefix).
    pub content: String,
}

/// Whether a diff line is context, added, or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Unchanged context line.
    Context,
    /// Line added in version B.
    Added,
    /// Line removed from version A.
    Removed,
}

/// Compute a line-level diff between two text contents.
pub fn diff_texts(old: &str, new: &str) -> Vec<DiffHunk> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Simple Myers-like diff using longest common subsequence
    let mut hunks = Vec::new();
    let mut lines = Vec::new();
    let mut old_idx = 0;
    let mut new_idx = 0;

    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        if old_idx < old_lines.len()
            && new_idx < new_lines.len()
            && old_lines[old_idx] == new_lines[new_idx]
        {
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                content: old_lines[old_idx].to_string(),
            });
            old_idx += 1;
            new_idx += 1;
        } else if new_idx < new_lines.len()
            && (old_idx >= old_lines.len() || !old_lines[old_idx..].contains(&new_lines[new_idx]))
        {
            lines.push(DiffLine {
                kind: DiffLineKind::Added,
                content: new_lines[new_idx].to_string(),
            });
            new_idx += 1;
        } else if old_idx < old_lines.len() {
            lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                content: old_lines[old_idx].to_string(),
            });
            old_idx += 1;
        }
    }

    // Only create a hunk if there are actual changes
    let has_changes = lines.iter().any(|l| l.kind != DiffLineKind::Context);

    if has_changes {
        hunks.push(DiffHunk {
            old_start: 1,
            old_count: old_lines.len(),
            new_start: 1,
            new_count: new_lines.len(),
            lines,
        });
    }

    hunks
}

/// Create a `FileDiff` for two versions of the same file.
pub fn diff_file(path: &str, old_content: &str, new_content: &str) -> FileDiff {
    if old_content == new_content {
        return FileDiff {
            path: path.to_string(),
            change_type: ChangeType::Unchanged,
            hunks: vec![],
        };
    }

    let hunks = diff_texts(old_content, new_content);
    FileDiff {
        path: path.to_string(),
        change_type: ChangeType::Modified,
        hunks,
    }
}

impl std::fmt::Display for FileDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "--- a/{}", self.path)?;
        writeln!(f, "+++ b/{}", self.path)?;
        for hunk in &self.hunks {
            writeln!(
                f,
                "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
            )?;
            for line in &hunk.lines {
                let prefix = match line.kind {
                    DiffLineKind::Context => ' ',
                    DiffLineKind::Added => '+',
                    DiffLineKind::Removed => '-',
                };
                writeln!(f, "{prefix}{}", line.content)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S3-06: Diff detects additions/deletions in `root.md`.
    #[test]
    fn test_diff_detects_changes() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nModified Line 2\nLine 3\nLine 4\n";

        let hunks = diff_texts(old, new);
        assert!(!hunks.is_empty(), "expected at least one hunk");

        let hunk = &hunks[0];
        let added: Vec<_> = hunk
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .collect();
        let removed: Vec<_> = hunk
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Removed)
            .collect();

        assert!(!added.is_empty(), "expected added lines");
        assert!(!removed.is_empty(), "expected removed lines");
    }

    /// Identical content produces no hunks.
    #[test]
    fn test_diff_identical() {
        let content = "Line 1\nLine 2\n";
        let hunks = diff_texts(content, content);
        assert!(hunks.is_empty());
    }

    /// `FileDiff` for identical content reports `Unchanged`.
    #[test]
    fn test_file_diff_unchanged() {
        let diff = diff_file("root.md", "same content", "same content");
        assert_eq!(diff.change_type, ChangeType::Unchanged);
        assert!(diff.hunks.is_empty());
    }

    /// `FileDiff` display produces unified diff format.
    #[test]
    fn test_file_diff_display() {
        let diff = diff_file("root.md", "old line\n", "new line\n");
        let output = diff.to_string();
        assert!(output.contains("--- a/root.md"));
        assert!(output.contains("+++ b/root.md"));
        assert!(output.contains("-old line") || output.contains("+new line"));
    }

    /// Empty to non-empty is all additions.
    #[test]
    fn test_diff_empty_to_content() {
        let hunks = diff_texts("", "Line 1\nLine 2\n");
        assert!(!hunks.is_empty());
        let added_count = hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .count();
        assert_eq!(added_count, 2);
    }
}
