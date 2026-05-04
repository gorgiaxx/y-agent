use std::path::{Component, Path, PathBuf};

use y_core::tool::ToolError;

pub(super) fn resolve_workspace_path(
    tool_name: &str,
    path: Option<&str>,
    working_dir: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let workspace = working_dir.filter(|value| !value.is_empty()).map(Path::new);
    let resolved = match (path.filter(|value| !value.is_empty()), workspace) {
        (Some(path), Some(workspace)) => {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            }
        }
        (Some(path), None) => PathBuf::from(path),
        (None, Some(workspace)) => workspace.to_path_buf(),
        (None, None) => PathBuf::from("."),
    };

    let resolved = normalize_lexically(&resolved);
    if let Some(workspace) = workspace {
        let workspace = normalize_lexically(workspace);
        if !resolved.starts_with(&workspace) {
            return Err(ToolError::PermissionDenied {
                name: tool_name.to_string(),
                reason: format!(
                    "path '{}' is outside workspace '{}'",
                    resolved.display(),
                    workspace.display()
                ),
            });
        }
    }

    Ok(resolved)
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}
