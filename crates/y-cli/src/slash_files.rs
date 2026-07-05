//! File-based slash commands: drop a `.md` file in a commands directory, get a
//! `/command` that expands into a prompt template.
//!
//! Discovery roots (project wins over user on name collision):
//! - Project: `./.y-agent/commands/*.md`
//! - User: `<user_config_dir>/commands/*.md` (respects `--profile`)
//!
//! Template placeholders (simple substitution, no Handlebars in v1):
//! - `$ARGUMENTS` / `$@` — all arguments joined by space.
//! - `$1`, `$2`, ... — positional arguments (1-indexed). Missing → empty string.
//! - `$$` — literal `$`.

use std::path::{Path, PathBuf};

/// Frontmatter metadata for a slash command file (manually parsed; no YAML dep).
#[derive(Debug, Clone, Default)]
struct Frontmatter {
    description: Option<String>,
    aliases: Vec<String>,
    agent: Option<String>,
}

/// A discovered file-based slash command.
#[derive(Debug, Clone)]
pub struct FileSlashCommand {
    pub name: String,
    #[allow(dead_code)]
    pub description: Option<String>,
    pub aliases: Vec<String>,
    #[allow(dead_code)]
    pub agent: Option<String>,
    pub body: String,
}

/// Parse a Markdown file into frontmatter + body.
///
/// Frontmatter is a simple YAML-like block delimited by `---` lines at the
/// very start. Only the subset used by slash commands is supported:
/// `key: value` pairs and `aliases: [a, b]` inline lists. If no frontmatter is
/// present, the entire content is the body and metadata is default.
fn parse_frontmatter(content: &str) -> (Frontmatter, String) {
    let trimmed = content.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return (Frontmatter::default(), content.to_string());
    }
    let after_first = &trimmed[3..]; // skip leading `---`
                                     // Skip the newline after the first `---`.
    let after_first = after_first
        .strip_prefix('\n')
        .or_else(|| after_first.strip_prefix("\r\n"))
        .unwrap_or(after_first);

    // Find the closing `---`.
    let close = after_first
        .find("\n---")
        .or_else(|| after_first.find("\r\n---"));
    let Some(close_idx) = close else {
        return (Frontmatter::default(), content.to_string());
    };

    let yaml_part = &after_first[..close_idx];
    // Skip past the closing delimiter and its newline.
    let mut body_start = close_idx + 4; // `\n---` or `\r\n---` (4 bytes for `\n---`)
                                        // Handle `\r\n---`: the `\r` was before `\n`, already counted.
                                        // Now skip the trailing newline after `---`.
    if body_start < after_first.len() && after_first[body_start..].starts_with('\n') {
        body_start += 1;
    } else if after_first[body_start..].starts_with("\r\n") {
        body_start += 2;
    }

    let body = after_first[body_start..]
        .trim_start_matches('\n')
        .to_string();
    let fm = parse_yaml_frontmatter(yaml_part);
    (fm, body)
}

/// Parse a minimal YAML subset: `key: value` and `aliases: [a, b, c]`.
///
/// Unknown keys are ignored. Malformed lines are skipped (no error). This
/// avoids pulling in a YAML dependency for what is a tiny structured header.
fn parse_yaml_frontmatter(yaml: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key {
            "description" => {
                fm.description = Some(unquote(value).to_string());
            }
            "agent" => {
                fm.agent = Some(unquote(value).to_string());
            }
            "aliases" => {
                fm.aliases = parse_inline_list(value);
            }
            _ => {}
        }
    }
    fm
}

/// Strip surrounding quotes from a YAML scalar value.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Parse an inline YAML list: `[a, b, c]`.
fn parse_inline_list(s: &str) -> Vec<String> {
    let s = s.trim();
    let Some(inner) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    inner
        .split(',')
        .map(|item| unquote(item.trim()).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Expand template placeholders in the body.
///
/// - `$ARGUMENTS` / `$@` → all args joined by space.
/// - `$1`..`$9` → positional arg (1-indexed). Missing → empty.
/// - `$$` → literal `$`.
pub fn expand(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    let all_args = args.join(" ");

    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            // `$$` → literal `$`.
            if next == b'$' {
                out.push('$');
                i += 2;
                continue;
            }
            // `$@` or `$ARGUMENTS` → all args.
            if next == b'@' {
                out.push_str(&all_args);
                i += 2;
                continue;
            }
            if template[i + 1..].starts_with("ARGUMENTS") {
                out.push_str(&all_args);
                i += 1 + "ARGUMENTS".len();
                continue;
            }
            // `$1`..`$9` → positional.
            if next.is_ascii_digit() {
                let idx = (next - b'1') as usize;
                if let Some(arg) = args.get(idx) {
                    out.push_str(arg);
                }
                i += 2;
                continue;
            }
            // Unknown `$X` — leave literal.
            out.push('$');
            i += 1;
            continue;
        }
        // SAFETY: we advance one byte at a time; UTF-8 boundaries are respected
        // because we only special-case ASCII `$` and push the rest verbatim.
        // Find the next `$` to bulk-copy.
        let remaining = &template[i..];
        let next_dollar = remaining.find('$');
        if let Some(rel) = next_dollar {
            out.push_str(&remaining[..rel]);
            i += rel;
        } else {
            out.push_str(remaining);
            break;
        }
    }

    out
}

/// Discover all file-based slash commands from both roots.
///
/// Project commands shadow user commands on name collision. Aliases are
/// registered alongside the primary name.
pub fn discover(project_root: Option<&Path>, user_root: Option<&Path>) -> Vec<FileSlashCommand> {
    let mut commands: Vec<FileSlashCommand> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Project first (higher priority).
    if let Some(root) = project_root {
        for cmd in scan_dir(root) {
            seen.insert(cmd.name.clone());
            commands.push(cmd);
        }
    }
    // User second (shadowed names skipped).
    if let Some(root) = user_root {
        for cmd in scan_dir(root) {
            if !seen.contains(&cmd.name) {
                commands.push(cmd);
            }
        }
    }

    commands
}

/// Scan a single directory for `*.md` command files.
fn scan_dir(dir: &Path) -> Vec<FileSlashCommand> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut commands = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip files starting with `_` (convention for drafts/includes).
        if stem.starts_with('_') {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (fm, body) = parse_frontmatter(&content);
        commands.push(FileSlashCommand {
            name: stem.to_string(),
            description: fm.description,
            aliases: fm.aliases,
            agent: fm.agent,
            body,
        });
    }

    commands
}

/// Try to expand a slash command input (`/name args...`) into a prompt.
///
/// Returns `None` if no matching file command is found. Searches both the
/// primary name and aliases.
pub fn try_expand(
    input: &str,
    project_root: Option<&Path>,
    user_root: Option<&Path>,
) -> Option<(String, Option<String>)> {
    let input = input.strip_prefix('/')?;
    let (name, rest) = split_name_and_args(input);
    if name.is_empty() {
        return None;
    }

    let commands = discover(project_root, user_root);
    let cmd = commands
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))?;

    let args: Vec<&str> = parse_args(&rest);
    let expanded = expand(&cmd.body, &args);
    Some((expanded, cmd.agent.clone()))
}

/// Split `/name rest of args` into `(name, "rest of args")`.
fn split_name_and_args(input: &str) -> (String, String) {
    let input = input.trim();
    match input.find(char::is_whitespace) {
        Some(idx) => (input[..idx].to_string(), input[idx..].trim().to_string()),
        None => (input.to_string(), String::new()),
    }
}

/// Simple quote-aware argument parser.
///
/// Supports `'single'` and `"double"` quoting. Strips quote delimiters. Does
/// not implement backslash escaping. Unmatched quote consumes to end.
fn parse_args(input: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1; // skip opening quote
            let arg_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            args.push(&input[arg_start..i]);
            if i < bytes.len() {
                i += 1; // skip closing quote
            }
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            args.push(&input[start..i]);
        }
    }
    args
}

/// Resolve the user commands directory from the active config.
///
/// Returns `<user_config_dir>/commands/`.
pub fn user_commands_dir(user_config_dir: Option<&Path>) -> Option<PathBuf> {
    user_config_dir.map(|d| d.join("commands"))
}

/// The project commands directory: `./.y-agent/commands/`.
pub fn project_commands_dir() -> PathBuf {
    PathBuf::from(".y-agent/commands")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // T-CLI-SLASH-01: frontmatter + body parsing.
    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\ndescription: Refactor code\naliases: [rf]\nagent: default\n---\nYou are refactoring. $ARGUMENTS\n";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.description.as_deref(), Some("Refactor code"));
        assert_eq!(fm.aliases, vec!["rf"]);
        assert_eq!(fm.agent.as_deref(), Some("default"));
        assert!(body.contains("You are refactoring."));
    }

    // T-CLI-SLASH-02: no frontmatter → entire content is body.
    #[test]
    fn test_parse_no_frontmatter() {
        let content = "Just a prompt with $1";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.description.is_none());
        assert_eq!(body, content);
    }

    // T-CLI-SLASH-03: expand `$ARGUMENTS` and `$@`.
    #[test]
    fn test_expand_arguments() {
        let args = vec!["foo", "bar"];
        assert_eq!(expand("do: $ARGUMENTS", &args), "do: foo bar");
        assert_eq!(expand("do: $@", &args), "do: foo bar");
    }

    // T-CLI-SLASH-04: expand positional `$1`, `$2`.
    #[test]
    fn test_expand_positional() {
        let args = vec!["alpha", "beta"];
        assert_eq!(
            expand("first=$1 second=$2", &args),
            "first=alpha second=beta"
        );
    }

    // T-CLI-SLASH-05: missing positional → empty.
    #[test]
    fn test_expand_missing_positional() {
        let args = vec!["only"];
        assert_eq!(expand("$1 $2 $3", &args), "only  ");
    }

    // T-CLI-SLASH-06: `$$` → literal `$`.
    #[test]
    fn test_expand_literal_dollar() {
        assert_eq!(expand("cost: $$5", &[]), "cost: $5");
    }

    // T-CLI-SLASH-07: unknown `$X` left literal.
    #[test]
    fn test_expand_unknown_left_literal() {
        assert_eq!(expand("value $x", &[]), "value $x");
    }

    // T-CLI-SLASH-08: discovery from project + user, project wins.
    #[test]
    fn test_discover_project_shadows_user() {
        let proj = tempdir().unwrap();
        let user = tempdir().unwrap();
        let proj_cmds = proj.path().join("commands");
        let user_cmds = user.path().join("commands");
        fs::create_dir_all(&proj_cmds).unwrap();
        fs::create_dir_all(&user_cmds).unwrap();
        fs::write(
            proj_cmds.join("review.md"),
            "---\ndescription: project review\n---\nProject review $ARGUMENTS",
        )
        .unwrap();
        fs::write(
            user_cmds.join("review.md"),
            "---\ndescription: user review\n---\nUser review $ARGUMENTS",
        )
        .unwrap();
        fs::write(
            user_cmds.join("commit.md"),
            "---\ndescription: user commit\n---\nCommit: $ARGUMENTS",
        )
        .unwrap();

        let commands = discover(Some(&proj_cmds), Some(&user_cmds));
        assert_eq!(commands.len(), 2);
        let review = commands.iter().find(|c| c.name == "review").unwrap();
        assert_eq!(review.description.as_deref(), Some("project review"));
    }

    // T-CLI-SLASH-09: try_expand via primary name.
    #[test]
    fn test_try_expand_by_name() {
        let dir = tempdir().unwrap();
        let cmds = dir.path().join("commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(
            cmds.join("refactor.md"),
            "---\naliases: [rf]\n---\nRefactor: $1",
        )
        .unwrap();

        let (expanded, _agent) = try_expand("/refactor src/main.rs", Some(&cmds), None).unwrap();
        assert_eq!(expanded, "Refactor: src/main.rs");
    }

    // T-CLI-SLASH-10: try_expand via alias.
    #[test]
    fn test_try_expand_by_alias() {
        let dir = tempdir().unwrap();
        let cmds = dir.path().join("commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(
            cmds.join("refactor.md"),
            "---\naliases: [rf]\n---\nRefactor: $1",
        )
        .unwrap();

        let (expanded, _) = try_expand("/rf src/lib.rs", Some(&cmds), None).unwrap();
        assert_eq!(expanded, "Refactor: src/lib.rs");
    }

    // T-CLI-SLASH-11: try_expand returns None for unknown command.
    #[test]
    fn test_try_expand_unknown_returns_none() {
        let dir = tempdir().unwrap();
        let cmds = dir.path().join("commands");
        fs::create_dir_all(&cmds).unwrap();
        assert!(try_expand("/nonexistent", Some(&cmds), None).is_none());
    }

    // T-CLI-SLASH-12: try_expand returns None for non-slash input.
    #[test]
    fn test_try_expand_non_slash_returns_none() {
        assert!(try_expand("just text", None, None).is_none());
    }

    // T-CLI-SLASH-13: quote-aware arg parsing.
    #[test]
    fn test_parse_args_quoted() {
        let args = parse_args(r#"review "src/main.rs" 'other file'"#);
        assert_eq!(args, vec!["review", "src/main.rs", "other file"]);
    }

    // T-CLI-SLASH-14: files starting with `_` are skipped.
    #[test]
    fn test_skip_underscore_files() {
        let dir = tempdir().unwrap();
        let cmds = dir.path().join("commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(cmds.join("_partial.md"), "should be skipped").unwrap();
        fs::write(cmds.join("real.md"), "real command").unwrap();
        let commands = discover(Some(&cmds), None);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "real");
    }

    // T-CLI-SLASH-15: empty body is valid.
    #[test]
    fn test_empty_body() {
        let dir = tempdir().unwrap();
        let cmds = dir.path().join("commands");
        fs::create_dir_all(&cmds).unwrap();
        fs::write(cmds.join("empty.md"), "---\ndescription: empty\n---\n").unwrap();
        let commands = discover(Some(&cmds), None);
        assert_eq!(commands.len(), 1);
        assert!(commands[0].body.trim().is_empty());
    }
}
