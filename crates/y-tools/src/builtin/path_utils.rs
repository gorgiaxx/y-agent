use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use y_core::tool::ToolError;

/// Sets the inner `AtomicBool` to `true` when dropped, signalling
/// a blocking worker thread to stop early.
pub(super) struct DropGuard(pub Option<Arc<AtomicBool>>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        if let Some(flag) = self.0.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

pub(super) fn resolve_workspace_path(
    tool_name: &str,
    path: Option<&str>,
    working_dir: Option<&str>,
) -> Result<PathBuf, ToolError> {
    resolve_path_with_read_dirs(tool_name, path, working_dir, &[])
}

pub(super) fn resolve_read_path(
    tool_name: &str,
    path: Option<&str>,
    working_dir: Option<&str>,
    additional_read_dirs: &[String],
) -> Result<PathBuf, ToolError> {
    resolve_path_with_read_dirs(tool_name, path, working_dir, additional_read_dirs)
}

fn resolve_path_with_read_dirs(
    tool_name: &str,
    path: Option<&str>,
    working_dir: Option<&str>,
    additional_read_dirs: &[String],
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
    let workspace_root = workspace.map(normalize_lexically);
    let additional_roots = additional_read_dirs
        .iter()
        .filter(|value| !value.is_empty())
        .map(|value| normalize_lexically(Path::new(value)))
        .collect::<Vec<_>>();
    let has_additional_roots = !additional_roots.is_empty();

    let mut allowed_roots =
        Vec::with_capacity(workspace_root.as_ref().map_or(0, |_| 1) + additional_roots.len());
    if let Some(workspace) = workspace_root {
        allowed_roots.push(workspace);
    }
    allowed_roots.extend(additional_roots);

    if !allowed_roots.is_empty() {
        let is_allowed = allowed_roots
            .iter()
            .any(|root| path_is_within_root(&resolved, root));
        if !is_allowed {
            if allowed_roots.len() == 1 && !has_additional_roots {
                return Err(ToolError::PermissionDenied {
                    name: tool_name.to_string(),
                    reason: format!(
                        "path '{}' is outside workspace '{}'",
                        resolved.display(),
                        allowed_roots[0].display()
                    ),
                });
            }

            let allowed = allowed_roots
                .iter()
                .map(|root| format!("'{}'", root.display()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ToolError::PermissionDenied {
                name: tool_name.to_string(),
                reason: format!(
                    "path '{}' is outside allowed roots {allowed}",
                    resolved.display()
                ),
            });
        }
    }

    Ok(resolved)
}

fn path_is_within_root(path: &Path, root: &Path) -> bool {
    if path == root {
        return true;
    }

    match std::fs::metadata(root) {
        Ok(metadata) if metadata.is_file() => false,
        _ => path.starts_with(root),
    }
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
