//! Bare-prompt resolution: detect `y-agent "do X"` and forward to `chat`.

/// Known subcommand names (mirrors `commands::Commands` variants, lowercased).
const SUBCOMMANDS: &[&str] = &[
    "chat",
    "init",
    "status",
    "config",
    "session",
    "tool",
    "agent",
    "workflow",
    "diag",
    "skill",
    "kb",
    "mcp",
    "completion",
    "tui",
    "serve",
    "resume",
    "fork",
    "print",
    "rpc",
];

/// Resolve raw argv (excluding `argv[0]`, the program name) into the args that
/// clap should parse.
///
/// Only the **first** token is inspected. This keeps the resolver simple and
/// correct without needing a flag-value table:
/// - If the first token is a flag (`-x` / `--x`) → pass through unchanged
///   (global flags like `--output` require an explicit subcommand, or the user
///   uses `y-agent chat --output json "do X"`).
/// - If the first token is a known subcommand → pass through unchanged.
/// - Otherwise (bare prompt) → prepend `["chat", "--"]` so the rest is
///   captured as the chat prompt. `--` ensures a prompt starting with `-` is
///   treated as a positional.
/// - If there are no tokens → pass through unchanged (clap shows help).
pub fn resolve(raw: &[String]) -> Vec<String> {
    let Some(first) = raw.first() else {
        return raw.to_vec();
    };

    // Flags → let clap handle (global flags need explicit subcommand).
    if first.starts_with('-') {
        return raw.to_vec();
    }

    // Known subcommand → pass through.
    if SUBCOMMANDS.contains(&first.as_str()) {
        return raw.to_vec();
    }

    // Bare prompt: forward to `chat -- <rest>`.
    let mut result = Vec::with_capacity(raw.len() + 2);
    result.push("chat".to_string());
    result.push("--".to_string());
    result.extend_from_slice(raw);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-BARE-01: Known subcommand passes through unchanged.
    #[test]
    fn known_subcommand_passes_through() {
        let raw = vec!["status".to_string()];
        assert_eq!(resolve(&raw), raw);
    }

    // T-CLI-BARE-02: Bare prompt is forwarded to `chat --`.
    #[test]
    fn bare_prompt_forwards_to_chat() {
        let raw = vec!["refactor this code".to_string()];
        let resolved = resolve(&raw);
        assert_eq!(resolved, vec!["chat", "--", "refactor this code"]);
    }

    // T-CLI-BARE-03: No tokens → unchanged (clap shows help).
    #[test]
    fn empty_args_unchanged() {
        let raw: Vec<String> = vec![];
        assert_eq!(resolve(&raw), raw);
    }

    // T-CLI-BARE-04: Flags only (no positional) → unchanged.
    #[test]
    fn flags_only_unchanged() {
        let raw = vec!["--help".to_string()];
        assert_eq!(resolve(&raw), raw);
    }

    // T-CLI-BARE-05: Global flags before a bare prompt pass through unchanged
    // (the resolver only inspects the first token; users combine global flags
    // with an explicit subcommand: `y-agent --output json chat "do X"`).
    #[test]
    fn global_flags_before_bare_prompt_preserved() {
        let raw = vec![
            "--output".to_string(),
            "json".to_string(),
            "do X".to_string(),
        ];
        assert_eq!(resolve(&raw), raw);
    }

    // T-CLI-BARE-06: A first token starting with `-` is a flag, not a prompt.
    // Pass through unchanged (clap will reject unknown flags).
    #[test]
    fn prompt_starting_with_dash_passes_through() {
        let raw = vec!["-summarize this".to_string()];
        assert_eq!(resolve(&raw), raw);
    }

    // T-CLI-BARE-07: Multiple bare tokens are all forwarded.
    #[test]
    fn multiple_bare_tokens_forwarded() {
        let raw = vec!["do".to_string(), "X".to_string(), "now".to_string()];
        let resolved = resolve(&raw);
        assert_eq!(resolved, vec!["chat", "--", "do", "X", "now"]);
    }

    // T-CLI-BARE-08: `chat` itself is a known subcommand (explicit form).
    #[test]
    fn chat_subcommand_passes_through() {
        let raw = vec![
            "chat".to_string(),
            "--session".to_string(),
            "abc".to_string(),
        ];
        assert_eq!(resolve(&raw), raw);
    }

    // T-CLI-BARE-09: New subcommands (print, rpc) are recognized.
    #[test]
    fn new_subcommands_recognized() {
        assert_eq!(
            resolve(&vec!["print".to_string(), "hi".to_string()]),
            vec!["print".to_string(), "hi".to_string()]
        );
        assert_eq!(resolve(&vec!["rpc".to_string()]), vec!["rpc".to_string()]);
    }
}
