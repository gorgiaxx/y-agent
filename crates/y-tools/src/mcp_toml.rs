//! Read/write helpers for the `[[mcp_servers]]` section of `tools.toml`.
//!
//! Used by the CLI (`y-agent mcp add/remove/list`) and the Tauri GUI config
//! commands so both operate on the same file with the same semantics.
//!
//! Writes preserve the rest of the file via `toml_edit`.

use std::fs;
use std::io::Write;
use std::path::Path;

use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use crate::mcp_integration::McpServerConfig;

/// Errors that can occur while manipulating the `tools.toml` MCP section.
#[derive(Debug, thiserror::Error)]
pub enum McpTomlError {
    #[error("failed to read '{path}': {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write '{path}': {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse '{path}' as TOML: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml_edit::TomlError,
    },
    #[error("failed to deserialize mcp_servers: {0}")]
    Deserialize(#[from] toml::de::Error),
    #[error("failed to serialize mcp_servers: {0}")]
    Serialize(#[from] toml::ser::Error),
}

fn read_document(path: &Path) -> Result<DocumentMut, McpTomlError> {
    let text = if path.exists() {
        fs::read_to_string(path).map_err(|e| McpTomlError::Read {
            path: path.display().to_string(),
            source: e,
        })?
    } else {
        String::new()
    };
    text.parse::<DocumentMut>()
        .map_err(|e| McpTomlError::Parse {
            path: path.display().to_string(),
            source: e,
        })
}

fn write_document_atomic(path: &Path, doc: &DocumentMut) -> Result<(), McpTomlError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| McpTomlError::Write {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| McpTomlError::Write {
            path: tmp.display().to_string(),
            source: e,
        })?;
        f.write_all(doc.to_string().as_bytes())
            .map_err(|e| McpTomlError::Write {
                path: tmp.display().to_string(),
                source: e,
            })?;
        f.sync_all().map_err(|e| McpTomlError::Write {
            path: tmp.display().to_string(),
            source: e,
        })?;
    }
    fs::rename(&tmp, path).map_err(|e| McpTomlError::Write {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load all `[[mcp_servers]]` entries from `tools.toml`.
///
/// Returns an empty vec if the file or section is missing.
pub fn load_mcp_servers(path: &Path) -> Result<Vec<McpServerConfig>, McpTomlError> {
    #[derive(serde::Deserialize, Default)]
    struct Partial {
        #[serde(default)]
        mcp_servers: Vec<McpServerConfig>,
    }

    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|e| McpTomlError::Read {
        path: path.display().to_string(),
        source: e,
    })?;

    let partial: Partial = toml::from_str(&text)?;
    Ok(partial.mcp_servers)
}

/// Insert or update an MCP server entry (matched by `name`).
///
/// Returns `true` if an existing entry was replaced, `false` if appended.
pub fn upsert_mcp_server(path: &Path, server: &McpServerConfig) -> Result<bool, McpTomlError> {
    let mut doc = read_document(path)?;
    let servers = ensure_array_of_tables(&mut doc, "mcp_servers");

    let table = server_to_table(server);
    let mut replaced = false;
    for existing in servers.iter_mut() {
        if existing.get("name").and_then(Item::as_str) == Some(server.name.as_str()) {
            *existing = table.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        servers.push(table);
    }

    write_document_atomic(path, &doc)?;
    Ok(replaced)
}

/// Remove an MCP server entry by name. Returns `true` if removed.
pub fn remove_mcp_server(path: &Path, name: &str) -> Result<bool, McpTomlError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut doc = read_document(path)?;
    let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return Ok(false);
    };

    let mut removed_index: Option<usize> = None;
    for (i, t) in servers.iter().enumerate() {
        if t.get("name").and_then(Item::as_str) == Some(name) {
            removed_index = Some(i);
            break;
        }
    }
    let Some(idx) = removed_index else {
        return Ok(false);
    };
    servers.remove(idx);

    write_document_atomic(path, &doc)?;
    Ok(true)
}

/// Replace the entire `[[mcp_servers]]` section with `servers`.
///
/// Used by the GUI save path where the frontend supplies the full list.
pub fn replace_mcp_servers(path: &Path, servers: &[McpServerConfig]) -> Result<(), McpTomlError> {
    let mut doc = read_document(path)?;
    let aot = ensure_array_of_tables(&mut doc, "mcp_servers");
    aot.clear();
    for s in servers {
        aot.push(server_to_table(s));
    }
    write_document_atomic(path, &doc)
}

fn ensure_array_of_tables<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut ArrayOfTables {
    if !doc.contains_key(key) {
        doc.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let item = doc.get_mut(key).expect("just inserted");
    if item.as_array_of_tables().is_none() {
        *item = Item::ArrayOfTables(ArrayOfTables::new());
    }
    item.as_array_of_tables_mut().expect("ensured above")
}

fn server_to_table(server: &McpServerConfig) -> Table {
    let mut t = Table::new();
    t.insert("name", value(server.name.as_str()));
    t.insert("transport", value(server.transport.as_str()));
    if let Some(cmd) = &server.command {
        t.insert("command", value(cmd.as_str()));
    }
    if !server.args.is_empty() {
        let mut arr = toml_edit::Array::new();
        for a in &server.args {
            arr.push(a.as_str());
        }
        t.insert("args", value(arr));
    }
    if let Some(url) = &server.url {
        t.insert("url", value(url.as_str()));
    }
    t.insert("enabled", value(server.enabled));
    t.insert(
        "startup_timeout_secs",
        value(i64::try_from(server.startup_timeout_secs).unwrap_or(i64::MAX)),
    );
    t.insert(
        "tool_timeout_secs",
        value(i64::try_from(server.tool_timeout_secs).unwrap_or(i64::MAX)),
    );
    if let Some(cwd) = &server.cwd {
        t.insert("cwd", value(cwd.as_str()));
    }
    if let Some(tok) = &server.bearer_token {
        t.insert("bearer_token", value(tok.as_str()));
    }
    if !server.env.is_empty() {
        let mut inline = toml_edit::InlineTable::new();
        let mut keys: Vec<_> = server.env.keys().collect();
        keys.sort();
        for k in keys {
            inline.insert(k, server.env[k].as_str().into());
        }
        t.insert("env", value(inline));
    }
    if !server.headers.is_empty() {
        let mut inline = toml_edit::InlineTable::new();
        let mut keys: Vec<_> = server.headers.keys().collect();
        keys.sort();
        for k in keys {
            inline.insert(k, server.headers[k].as_str().into());
        }
        t.insert("headers", value(inline));
    }
    if let Some(list) = &server.enabled_tools {
        let mut arr = toml_edit::Array::new();
        for s in list {
            arr.push(s.as_str());
        }
        t.insert("enabled_tools", value(arr));
    }
    if let Some(list) = &server.disabled_tools {
        let mut arr = toml_edit::Array::new();
        for s in list {
            arr.push(s.as_str());
        }
        t.insert("disabled_tools", value(arr));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn sample_http(name: &str, url: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some(url.into()),
            env: HashMap::new(),
            enabled: true,
            headers: HashMap::new(),
            startup_timeout_secs: 30,
            tool_timeout_secs: 120,
            cwd: None,
            bearer_token: None,
            enabled_tools: None,
            disabled_tools: None,
            auto_reconnect: true,
            max_reconnect_attempts: 5,
        }
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools.toml");
        let servers = load_mcp_servers(&path).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn upsert_appends_then_replaces() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools.toml");

        let a = sample_http("grep", "https://mcp.grep.app");
        assert!(!upsert_mcp_server(&path, &a).unwrap());

        let mut b = a.clone();
        b.url = Some("https://mcp.grep.app/v2".into());
        assert!(upsert_mcp_server(&path, &b).unwrap());

        let loaded = load_mcp_servers(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].url.as_deref(), Some("https://mcp.grep.app/v2"));
    }

    #[test]
    fn remove_by_name() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools.toml");
        upsert_mcp_server(&path, &sample_http("one", "https://a")).unwrap();
        upsert_mcp_server(&path, &sample_http("two", "https://b")).unwrap();

        assert!(remove_mcp_server(&path, "one").unwrap());
        assert!(!remove_mcp_server(&path, "missing").unwrap());

        let loaded = load_mcp_servers(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "two");
    }

    #[test]
    fn preserves_other_sections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools.toml");
        fs::write(
            &path,
            "# top comment\nmax_active = 42\n\n[other]\nkey = \"value\"\n",
        )
        .unwrap();

        upsert_mcp_server(&path, &sample_http("grep", "https://mcp.grep.app")).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# top comment"), "comment preserved");
        assert!(text.contains("max_active = 42"), "scalar preserved");
        assert!(text.contains("[other]"), "table preserved");
        assert!(text.contains("[[mcp_servers]]"), "mcp section added");
        assert!(text.contains("grep"), "server name present");
    }

    #[test]
    fn replace_overwrites_all() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools.toml");
        upsert_mcp_server(&path, &sample_http("a", "https://a")).unwrap();
        upsert_mcp_server(&path, &sample_http("b", "https://b")).unwrap();

        replace_mcp_servers(&path, &[sample_http("c", "https://c")]).unwrap();

        let loaded = load_mcp_servers(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "c");
    }
}
