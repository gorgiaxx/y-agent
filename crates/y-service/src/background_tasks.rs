//! Service-layer API for runtime-managed background tasks.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use y_core::runtime::{
    BackgroundProcessInfo, BackgroundProcessSnapshot, CommandRunner, ProcessStatus, RuntimeBackend,
};
use y_core::types::SessionId;

use crate::container::ServiceContainer;

const DEFAULT_YIELD_TIME_MS: u64 = 100;
const MAX_YIELD_TIME_MS: u64 = 2_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Status-bar friendly summary of a runtime-managed background task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackgroundTaskInfo {
    pub process_id: String,
    pub backend: String,
    pub command: String,
    pub working_dir: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Incremental output snapshot for a background task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackgroundTaskSnapshot {
    pub process_id: String,
    pub backend: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Request options for polling a background task.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BackgroundTaskPollRequest {
    pub session_id: String,
    pub process_id: String,
    pub yield_time_ms: Option<u64>,
    pub max_output_bytes: Option<usize>,
}

/// Request options for writing to a background task stdin.
#[derive(Debug, Clone, Deserialize)]
pub struct BackgroundTaskWriteRequest {
    pub session_id: String,
    pub process_id: String,
    pub input: String,
    pub yield_time_ms: Option<u64>,
    pub max_output_bytes: Option<usize>,
}

fn backend_name(backend: &RuntimeBackend) -> &'static str {
    match backend {
        RuntimeBackend::Docker => "docker",
        RuntimeBackend::Native => "native",
        RuntimeBackend::Ssh => "ssh",
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn status_parts(status: &ProcessStatus) -> (&'static str, Option<i32>, Option<String>) {
    match status {
        ProcessStatus::Running => ("running", None, None),
        ProcessStatus::Completed { exit_code } => ("completed", Some(*exit_code), None),
        ProcessStatus::Failed { error } => ("failed", None, Some(error.clone())),
        ProcessStatus::Unknown => ("unknown", None, None),
    }
}

fn bounded_yield_time(value: Option<u64>) -> Duration {
    Duration::from_millis(
        value
            .unwrap_or(DEFAULT_YIELD_TIME_MS)
            .min(MAX_YIELD_TIME_MS),
    )
}

fn bounded_max_output_bytes(value: Option<usize>) -> usize {
    value
        .unwrap_or(DEFAULT_MAX_OUTPUT_BYTES)
        .min(MAX_OUTPUT_BYTES)
}

fn parse_session_id(value: &str) -> anyhow::Result<SessionId> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("session_id is required for background task access");
    }
    Ok(SessionId::from_string(trimmed))
}

impl From<BackgroundProcessInfo> for BackgroundTaskInfo {
    fn from(info: BackgroundProcessInfo) -> Self {
        let (status, exit_code, error) = status_parts(&info.status);
        Self {
            process_id: info.handle.id,
            backend: backend_name(&info.handle.backend).to_string(),
            command: info.command,
            working_dir: info.working_dir,
            status: status.to_string(),
            exit_code,
            error,
            duration_ms: duration_ms(info.duration),
        }
    }
}

impl From<BackgroundProcessSnapshot> for BackgroundTaskSnapshot {
    fn from(snapshot: BackgroundProcessSnapshot) -> Self {
        let (status, exit_code, error) = status_parts(&snapshot.status);
        Self {
            process_id: snapshot.handle.id,
            backend: backend_name(&snapshot.handle.backend).to_string(),
            status: status.to_string(),
            exit_code,
            error,
            stdout: String::from_utf8_lossy(&snapshot.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&snapshot.stderr).into_owned(),
            duration_ms: duration_ms(snapshot.duration),
        }
    }
}

/// Product service facade for background task lifecycle actions.
pub struct BackgroundTaskService;

impl BackgroundTaskService {
    /// List background tasks owned by the given session.
    pub async fn list(
        container: &ServiceContainer,
        session_id: String,
    ) -> anyhow::Result<Vec<BackgroundTaskInfo>> {
        let session_id = parse_session_id(&session_id)?;
        let processes = container
            .runtime_manager
            .list_processes(&session_id)
            .await?;
        Ok(processes
            .into_iter()
            .map(BackgroundTaskInfo::from)
            .collect())
    }

    /// Poll incremental output for a background task.
    pub async fn poll(
        container: &ServiceContainer,
        request: BackgroundTaskPollRequest,
    ) -> anyhow::Result<BackgroundTaskSnapshot> {
        let session_id = parse_session_id(&request.session_id)?;
        let snapshot = container
            .runtime_manager
            .read_process(
                &session_id,
                &request.process_id,
                bounded_yield_time(request.yield_time_ms),
                bounded_max_output_bytes(request.max_output_bytes),
            )
            .await?;
        Ok(snapshot.into())
    }

    /// Write stdin to a running background task, then return the next snapshot.
    pub async fn write(
        container: &ServiceContainer,
        request: BackgroundTaskWriteRequest,
    ) -> anyhow::Result<BackgroundTaskSnapshot> {
        let session_id = parse_session_id(&request.session_id)?;
        let snapshot = container
            .runtime_manager
            .write_process(
                &session_id,
                &request.process_id,
                request.input.as_bytes(),
                bounded_yield_time(request.yield_time_ms),
                bounded_max_output_bytes(request.max_output_bytes),
            )
            .await?;
        Ok(snapshot.into())
    }

    /// Terminate a background task and return the final output snapshot.
    pub async fn kill(
        container: &ServiceContainer,
        request: BackgroundTaskPollRequest,
    ) -> anyhow::Result<BackgroundTaskSnapshot> {
        let session_id = parse_session_id(&request.session_id)?;
        let snapshot = container
            .runtime_manager
            .kill_process(
                &session_id,
                &request.process_id,
                bounded_yield_time(request.yield_time_ms),
                bounded_max_output_bytes(request.max_output_bytes),
            )
            .await?;
        Ok(snapshot.into())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use y_core::runtime::{
        BackgroundProcessInfo, BackgroundProcessSnapshot, ProcessHandle, ProcessStatus,
        RuntimeBackend,
    };

    use super::{BackgroundTaskInfo, BackgroundTaskSnapshot};

    #[test]
    fn maps_background_process_info_to_status_bar_dto() {
        let info = BackgroundProcessInfo {
            handle: ProcessHandle {
                id: "proc-1".into(),
                backend: RuntimeBackend::Native,
            },
            command: "npm run dev".into(),
            working_dir: Some("/repo/app".into()),
            owner_session_id: None,
            status: ProcessStatus::Running,
            duration: Duration::from_millis(1_250),
        };

        let dto = BackgroundTaskInfo::from(info);

        assert_eq!(dto.process_id, "proc-1");
        assert_eq!(dto.backend, "native");
        assert_eq!(dto.command, "npm run dev");
        assert_eq!(dto.working_dir.as_deref(), Some("/repo/app"));
        assert_eq!(dto.status, "running");
        assert_eq!(dto.exit_code, None);
        assert_eq!(dto.error, None);
        assert_eq!(dto.duration_ms, 1_250);
    }

    #[test]
    fn maps_background_snapshot_with_output() {
        let snapshot = BackgroundProcessSnapshot {
            handle: ProcessHandle {
                id: "proc-2".into(),
                backend: RuntimeBackend::Native,
            },
            status: ProcessStatus::Completed { exit_code: 0 },
            owner_session_id: None,
            stdout: b"ready".to_vec(),
            stderr: b"".to_vec(),
            duration: Duration::from_millis(750),
        };

        let dto = BackgroundTaskSnapshot::from(snapshot);

        assert_eq!(dto.process_id, "proc-2");
        assert_eq!(dto.backend, "native");
        assert_eq!(dto.status, "completed");
        assert_eq!(dto.exit_code, Some(0));
        assert_eq!(dto.error, None);
        assert_eq!(dto.stdout, "ready");
        assert_eq!(dto.stderr, "");
        assert_eq!(dto.duration_ms, 750);
    }
}
