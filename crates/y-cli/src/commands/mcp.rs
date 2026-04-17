//! MCP server management commands — add, remove, list, status.
//!
//! Writes to the `[[mcp_servers]]` section of `<user_config_dir>/tools.toml`.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;

use y_tools::mcp_integration::McpServerConfig;
use y_tools::mcp_toml;

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Arguments for `mcp add`. Boxed in `McpAction` to keep the enum small.
#[derive(Debug, clap::Args)]
pub struct McpAddArgs {
    /// Server name (used as the `mcp_{name}_{tool}` prefix).
    pub name: String,

    /// Optional positional endpoint. For `--transport http` this is shorthand for `--url`.
    pub endpoint: Option<String>,

    /// Transport type: `stdio` or `http`.
    #[arg(long, default_value = "stdio")]
    pub transport: String,

    /// Command to execute (stdio transport).
    #[arg(long)]
    pub command: Option<String>,

    /// Arguments for the command (repeatable).
    #[arg(long = "arg", value_name = "ARG")]
    pub args: Vec<String>,

    /// Environment variable (`KEY=VAL`, repeatable).
    #[arg(long = "env", value_name = "KEY=VAL")]
    pub env: Vec<String>,

    /// Working directory for the subprocess (stdio transport).
    #[arg(long)]
    pub cwd: Option<String>,

    /// Endpoint URL (http transport). Overrides the positional `endpoint`.
    #[arg(long)]
    pub url: Option<String>,

    /// HTTP header (`KEY=VAL`, repeatable).
    #[arg(long = "header", value_name = "KEY=VAL")]
    pub headers: Vec<String>,

    /// Bearer token for HTTP Authorization header.
    #[arg(long)]
    pub bearer_token: Option<String>,

    /// Mark the server as disabled.
    #[arg(long)]
    pub disabled: bool,

    /// Startup / initialize timeout in seconds.
    #[arg(long, default_value_t = 30)]
    pub startup_timeout_secs: u64,

    /// Per-tool-call timeout in seconds.
    #[arg(long, default_value_t = 120)]
    pub tool_timeout_secs: u64,
}

/// MCP subcommands.
#[derive(Debug, Subcommand)]
pub enum McpAction {
    /// List configured MCP servers from `tools.toml`.
    List,

    /// Show live connection status for configured MCP servers.
    Status,

    /// Add or update an MCP server in `tools.toml`.
    Add(Box<McpAddArgs>),

    /// Remove an MCP server from `tools.toml`.
    Remove {
        /// Server name.
        name: String,
    },
}

/// Run an mcp subcommand that does not require a live service container.
///
/// Handles `List`, `Add`, `Remove`. `Status` is handled separately because
/// it needs the `AppServices::mcp_manager`.
pub fn run_offline(
    action: &McpAction,
    tools_toml_path: &std::path::Path,
    mode: OutputMode,
) -> Result<()> {
    match action {
        McpAction::List => list_servers(tools_toml_path, mode),
        McpAction::Add(args) => add_server(
            tools_toml_path,
            &args.name,
            args.endpoint.as_deref(),
            &args.transport,
            args.command.as_deref(),
            &args.args,
            &args.env,
            args.cwd.as_deref(),
            args.url.as_deref(),
            &args.headers,
            args.bearer_token.as_deref(),
            args.disabled,
            args.startup_timeout_secs,
            args.tool_timeout_secs,
        ),
        McpAction::Remove { name } => remove_server(tools_toml_path, name),
        McpAction::Status => Err(anyhow!(
            "mcp status requires the service container; use run_status instead"
        )),
    }
}

/// Run `mcp status` — requires a live `AppServices` with a connection manager.
pub async fn run_status(services: &AppServices, mode: OutputMode) -> Result<()> {
    let statuses = services.mcp_manager.status().await;

    if mode == OutputMode::Json {
        let display: HashMap<String, String> = statuses
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        println!("{}", serde_json::to_string_pretty(&display)?);
    } else {
        let headers = &["Server", "Status"];
        let mut rows: Vec<TableRow> = statuses
            .into_iter()
            .map(|(name, status)| TableRow {
                cells: vec![name, status.to_string()],
            })
            .collect();
        rows.sort_by(|a, b| a.cells[0].cmp(&b.cells[0]));
        if rows.is_empty() {
            output::print_info("No MCP servers connected");
        } else {
            let table = output::format_table(headers, &rows);
            print!("{table}");
        }
    }

    Ok(())
}

fn list_servers(path: &std::path::Path, mode: OutputMode) -> Result<()> {
    let servers = mcp_toml::load_mcp_servers(path)
        .with_context(|| format!("loading mcp_servers from {}", path.display()))?;

    if mode == OutputMode::Json {
        println!("{}", serde_json::to_string_pretty(&servers)?);
    } else if servers.is_empty() {
        output::print_info(&format!("No MCP servers configured in {}", path.display()));
        return Ok(());
    } else {
        let headers = &["Name", "Transport", "Endpoint", "Enabled"];
        let rows: Vec<TableRow> = servers
            .iter()
            .map(|s| TableRow {
                cells: vec![
                    s.name.clone(),
                    s.transport.clone(),
                    endpoint_display(s),
                    if s.enabled { "yes".into() } else { "no".into() },
                ],
            })
            .collect();
        let table = output::format_table(headers, &rows);
        print!("{table}");
    }
    Ok(())
}

fn endpoint_display(s: &McpServerConfig) -> String {
    match s.transport.as_str() {
        "http" => s.url.clone().unwrap_or_default(),
        "stdio" => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(cmd) = &s.command {
                parts.push(cmd.clone());
            }
            parts.extend(s.args.iter().cloned());
            parts.join(" ")
        }
        _ => String::new(),
    }
}

fn add_server(
    path: &std::path::Path,
    name: &str,
    endpoint: Option<&str>,
    transport: &str,
    command: Option<&str>,
    args: &[String],
    env_pairs: &[String],
    cwd: Option<&str>,
    url: Option<&str>,
    header_pairs: &[String],
    bearer_token: Option<&str>,
    disabled: bool,
    startup_timeout_secs: u64,
    tool_timeout_secs: u64,
) -> Result<()> {
    let transport_norm = transport.to_ascii_lowercase();
    if transport_norm != "stdio" && transport_norm != "http" {
        return Err(anyhow!(
            "invalid --transport '{transport}' (expected 'stdio' or 'http')"
        ));
    }

    let env = parse_kv_pairs(env_pairs, "--env")?;
    let headers = parse_kv_pairs(header_pairs, "--header")?;

    // Resolve URL: --url wins, else positional endpoint (http only).
    let resolved_url = match (url, endpoint) {
        (Some(u), _) => Some(u.to_string()),
        (None, Some(p)) if transport_norm == "http" => Some(p.to_string()),
        _ => None,
    };

    match transport_norm.as_str() {
        "stdio" => {
            if command.is_none() {
                return Err(anyhow!("stdio transport requires --command"));
            }
        }
        "http" => {
            if resolved_url.is_none() {
                return Err(anyhow!("http transport requires --url or a positional URL"));
            }
        }
        _ => unreachable!(),
    }

    let server = McpServerConfig {
        name: name.to_string(),
        transport: transport_norm,
        command: command.map(str::to_string),
        args: args.to_vec(),
        url: resolved_url,
        env,
        enabled: !disabled,
        headers,
        startup_timeout_secs,
        tool_timeout_secs,
        cwd: cwd.map(str::to_string),
        bearer_token: bearer_token.map(str::to_string),
        enabled_tools: None,
        disabled_tools: None,
        auto_reconnect: true,
        max_reconnect_attempts: 5,
    };

    let replaced = mcp_toml::upsert_mcp_server(path, &server)
        .with_context(|| format!("writing mcp_servers to {}", path.display()))?;

    if replaced {
        output::print_success(&format!(
            "Updated MCP server '{}' in {}",
            server.name,
            path.display()
        ));
    } else {
        output::print_success(&format!(
            "Added MCP server '{}' to {}",
            server.name,
            path.display()
        ));
    }
    Ok(())
}

fn remove_server(path: &std::path::Path, name: &str) -> Result<()> {
    let removed = mcp_toml::remove_mcp_server(path, name)
        .with_context(|| format!("updating {}", path.display()))?;
    if removed {
        output::print_success(&format!("Removed MCP server '{name}'"));
    } else {
        output::print_info(&format!("No MCP server named '{name}' was found"));
    }
    Ok(())
}

fn parse_kv_pairs(pairs: &[String], flag: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in pairs {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid {flag} value '{pair}': expected KEY=VALUE"))?;
        if k.is_empty() {
            return Err(anyhow!("invalid {flag} value '{pair}': empty key"));
        }
        map.insert(k.to_string(), v.to_string());
    }
    Ok(map)
}

/// Default path to `tools.toml` inside the user config directory.
pub fn default_tools_toml_path() -> Result<PathBuf> {
    let dir = crate::config::dirs_user_config()
        .ok_or_else(|| anyhow!("unable to resolve user config directory"))?;
    Ok(dir.join("tools.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kv_pairs_ok() {
        let pairs = vec!["A=1".into(), "B=hello world".into()];
        let map = parse_kv_pairs(&pairs, "--env").unwrap();
        assert_eq!(map.get("A").map(String::as_str), Some("1"));
        assert_eq!(map.get("B").map(String::as_str), Some("hello world"));
    }

    #[test]
    fn parse_kv_pairs_rejects_missing_eq() {
        let pairs = vec!["NOEQ".into()];
        assert!(parse_kv_pairs(&pairs, "--env").is_err());
    }

    #[test]
    fn add_then_list_then_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tools.toml");

        add_server(
            &path,
            "grep",
            Some("https://mcp.grep.app"),
            "http",
            None,
            &[],
            &[],
            None,
            None,
            &[],
            None,
            false,
            30,
            120,
        )
        .unwrap();

        let servers = mcp_toml::load_mcp_servers(&path).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "grep");
        assert_eq!(servers[0].transport, "http");
        assert_eq!(servers[0].url.as_deref(), Some("https://mcp.grep.app"));

        remove_server(&path, "grep").unwrap();
        let servers = mcp_toml::load_mcp_servers(&path).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn add_http_requires_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tools.toml");

        let err = add_server(
            &path,
            "x",
            None,
            "http",
            None,
            &[],
            &[],
            None,
            None,
            &[],
            None,
            false,
            30,
            120,
        );
        assert!(err.is_err());
    }

    #[test]
    fn add_stdio_requires_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tools.toml");
        let err = add_server(
            &path,
            "x",
            None,
            "stdio",
            None,
            &[],
            &[],
            None,
            None,
            &[],
            None,
            false,
            30,
            120,
        );
        assert!(err.is_err());
    }
}
