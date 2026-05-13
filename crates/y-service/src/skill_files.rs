//! Shared path guards for reading and writing files inside skill directories.

use std::path::{Path, PathBuf};

/// Resolve an existing file inside a skill directory for reading.
pub fn resolve_skill_read_path(skill_dir: &Path, relative_path: &Path) -> Result<PathBuf, String> {
    let canonical_dir = canonical_skill_dir(skill_dir)?;
    let target = skill_dir.join(relative_path);
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("File not found: {e}"))?;
    ensure_inside_skill_dir(&canonical_dir, &canonical_target)?;
    Ok(canonical_target)
}

/// Resolve a file inside a skill directory for writing.
///
/// Existing symlink targets are rejected even when they point back inside the
/// skill directory, because following symlinks during writes makes the caller's
/// write target ambiguous.
pub fn resolve_skill_write_path(skill_dir: &Path, relative_path: &Path) -> Result<PathBuf, String> {
    let canonical_dir = canonical_skill_dir(skill_dir)?;
    let target = skill_dir.join(relative_path);

    if target.exists() {
        let metadata = std::fs::symlink_metadata(&target)
            .map_err(|e| format!("Failed to inspect target file: {e}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Access denied: symlink writes are not allowed".to_string());
        }

        let canonical_target = target
            .canonicalize()
            .map_err(|e| format!("File not found: {e}"))?;
        ensure_inside_skill_dir(&canonical_dir, &canonical_target)?;
        return Ok(canonical_target);
    }

    let parent = target.parent().ok_or_else(|| "Invalid path".to_string())?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("Parent dir not found: {e}"))?;
    ensure_inside_skill_dir(&canonical_dir, &canonical_parent)?;
    Ok(target)
}

fn canonical_skill_dir(skill_dir: &Path) -> Result<PathBuf, String> {
    skill_dir
        .canonicalize()
        .map_err(|e| format!("Skill dir not found: {e}"))
}

fn ensure_inside_skill_dir(canonical_dir: &Path, canonical_target: &Path) -> Result<(), String> {
    if canonical_target.starts_with(canonical_dir) {
        Ok(())
    } else {
        Err("Access denied: path traversal detected".to_string())
    }
}
