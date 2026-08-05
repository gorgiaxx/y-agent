//! Provider migration command handlers — detect external agent CLIs and
//! quick-import their provider configs into y-agent's `providers.toml`.
//!
//! Thin wrappers around [`y_service::provider_migration`]; all business logic
//! lives in the service layer per the service-layer ownership rule.

use tauri::State;

use crate::state::AppState;
use y_service::provider_migration::{self, MigrationReport, MigrationSourceInfo};

/// Detect all supported migration sources (omp, kimi, claude, codex, omo).
///
/// Returns one entry per source with its detection status, support flag,
/// migrated state, and the extractable provider candidates.
#[tauri::command]
pub async fn provider_migration_detect(
    state: State<'_, AppState>,
) -> Result<Vec<MigrationSourceInfo>, String> {
    let home = crate::home_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    Ok(provider_migration::detect_sources(&home, &state.config_dir))
}

/// Migrate selected providers from one source into `providers.toml`.
///
/// `source_id` selects the external tool; `selected_ids` are the candidate
/// ids (from `provider_migration_detect`) the user chose to import.
#[tauri::command]
pub async fn provider_migration_run(
    state: State<'_, AppState>,
    source_id: String,
    selected_ids: Vec<String>,
) -> Result<MigrationReport, String> {
    let home = crate::home_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    provider_migration::migrate_source(&home, &state.config_dir, &source_id, &selected_ids)
}
