//! Git status for the status bar, polled off the UI loop.
//!
//! Mirrors the pi-powerline-footer segment contract: branch name (or
//! `detached`), `+N` staged, `*N` unstaged, `?N` untracked. The bar renders
//! from the cached value (serve-stale) while a refresh runs in the
//! background, so the segment never blanks out.

use std::time::Duration;

/// Parsed working-tree status for one repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// Current branch name, or `detached` for a detached HEAD.
    pub branch: String,
    /// Files staged for the next commit (`+N`).
    pub staged: usize,
    /// Tracked files with unstaged modifications (`*N`).
    pub unstaged: usize,
    /// Untracked files (`?N`).
    pub untracked: usize,
}

impl GitStatus {
    /// Whether the working tree has any pending change.
    pub fn is_dirty(&self) -> bool {
        self.staged + self.unstaged + self.untracked > 0
    }
}

/// Query the repository at `workdir`, returning `None` when git is missing,
/// the directory is not a repository, or the command exceeds the timeout.
/// Runs with a short budget: the status bar tolerates a stale frame far
/// better than a blocked one.
pub async fn query(workdir: &str) -> Option<GitStatus> {
    let output = tokio::time::timeout(
        Duration::from_millis(800),
        tokio::process::Command::new("git")
            .args(["status", "--porcelain=v1", "--branch"])
            .current_dir(workdir)
            // Never let git prompt (credentials, etc.) from the TUI loop.
            .env("GIT_TERMINAL_PROMPT", "0")
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_porcelain(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse `git status --porcelain=v1 --branch` output.
///
/// The header line `## main...origin/main [ahead 1]` carries the branch;
/// each following line is an XY status code where X is the staged state and
/// Y the unstaged state. Untracked entries use `??`.
pub fn parse_porcelain(output: &str) -> GitStatus {
    let mut status = GitStatus::default();
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            status.branch = parse_branch(header);
            continue;
        }
        let mut chars = line.chars();
        let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
            continue;
        };
        if x == '?' && y == '?' {
            status.untracked += 1;
            continue;
        }
        if matches!(x, 'M' | 'A' | 'D' | 'R' | 'C') {
            status.staged += 1;
        }
        if matches!(y, 'M' | 'D') {
            status.unstaged += 1;
        }
    }
    status
}

/// Extract the branch name from the `##` header payload:
/// `main...origin/main [ahead 1]` -> `main`,
/// `HEAD (no branch)` -> `detached`, `No commits yet on main` -> `main`.
fn parse_branch(header: &str) -> String {
    if header.starts_with("HEAD ") {
        return "detached".to_string();
    }
    let name = header
        .strip_prefix("No commits yet on ")
        .unwrap_or(header)
        .split("...")
        .next()
        .unwrap_or(header)
        .trim();
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_change_counts() {
        let output = "## main...origin/main [ahead 1]\nM  src/lib.rs\n M src/main.rs\n?? new.txt\nA  added.rs\n";
        let status = parse_porcelain(output);
        assert_eq!(status.branch, "main");
        assert_eq!(status.staged, 2, "M_ and A_ are staged");
        assert_eq!(status.unstaged, 1, "_M is unstaged");
        assert_eq!(status.untracked, 1);
        assert!(status.is_dirty());
    }

    #[test]
    fn clean_tree_reports_no_changes() {
        let status = parse_porcelain("## feature/x\n");
        assert_eq!(status.branch, "feature/x");
        assert!(!status.is_dirty());
    }

    #[test]
    fn detached_head_maps_to_label() {
        let status = parse_porcelain("## HEAD (no branch)\n M a.rs\n");
        assert_eq!(status.branch, "detached");
        assert_eq!(status.unstaged, 1);
    }

    #[test]
    fn unborn_branch_strips_prefix() {
        let status = parse_porcelain("## No commits yet on main\n?? a.txt\n");
        assert_eq!(status.branch, "main");
        assert_eq!(status.untracked, 1);
    }

    #[test]
    fn rename_counts_staged_only() {
        let status = parse_porcelain("## main\nR  old.rs -> new.rs\n");
        assert_eq!(status.staged, 1);
        assert_eq!(status.unstaged, 0);
    }
}
