//! Resource monitor for tracking runtime resource usage.
//!
//! Monitors CPU, memory, disk, and network usage with configurable
//! thresholds and alerting. Used for preventing resource exhaustion
//! and providing observability into runtime operations.
//!
//! Design reference: runtime-design.md §Resource Management

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Resource usage thresholds for alerting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceThresholds {
    /// Maximum memory usage in bytes before warning.
    pub max_memory_bytes: u64,
    /// Maximum number of open file descriptors.
    pub max_open_files: u32,
    /// Maximum number of active processes/tasks.
    pub max_active_tasks: u32,
    /// Maximum disk usage in bytes.
    pub max_disk_bytes: u64,
}

impl Default for ResourceThresholds {
    fn default() -> Self {
        Self {
            max_memory_bytes: 512 * 1024 * 1024, // 512 MB
            max_open_files: 1024,
            max_active_tasks: 100,
            max_disk_bytes: 1024 * 1024 * 1024, // 1 GB
        }
    }
}

/// Snapshot of current resource usage.
#[derive(Debug, Clone)]
pub struct ResourceSnapshot {
    /// Current memory usage in bytes.
    pub memory_bytes: u64,
    /// Current number of open file descriptors.
    pub open_files: u32,
    /// Current number of active tasks.
    pub active_tasks: u32,
    /// Current disk usage in bytes.
    pub disk_bytes: u64,
    /// When this snapshot was taken.
    pub timestamp: Instant,
}

/// A resource violation detected by the monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceViolation {
    /// Type of resource that exceeded its threshold.
    pub resource: ResourceKind,
    /// Current value.
    pub current_value: u64,
    /// Threshold value.
    pub threshold: u64,
    /// Human-readable description.
    pub message: String,
}

/// Types of monitored resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Memory,
    OpenFiles,
    ActiveTasks,
    DiskUsage,
}

/// Resource monitor tracking usage and detecting threshold violations.
#[derive(Debug, Clone)]
pub struct ResourceMonitor {
    inner: Arc<Mutex<ResourceMonitorInner>>,
}

#[derive(Debug)]
struct ResourceMonitorInner {
    thresholds: ResourceThresholds,
    current: ResourceState,
}

#[derive(Debug, Default)]
struct ResourceState {
    memory_bytes: u64,
    open_files: u32,
    active_tasks: u32,
    disk_bytes: u64,
}

impl ResourceMonitor {
    /// Create a new resource monitor with the given thresholds.
    pub fn new(thresholds: ResourceThresholds) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ResourceMonitorInner {
                thresholds,
                current: ResourceState::default(),
            })),
        }
    }

    /// Create with default thresholds.
    pub fn with_defaults() -> Self {
        Self::new(ResourceThresholds::default())
    }

    /// Record memory usage.
    pub async fn record_memory(&self, bytes: u64) {
        let mut inner = self.inner.lock().await;
        inner.current.memory_bytes = bytes;
    }

    /// Increment active task count.
    pub async fn task_started(&self) {
        let mut inner = self.inner.lock().await;
        inner.current.active_tasks += 1;
    }

    /// Decrement active task count.
    pub async fn task_completed(&self) {
        let mut inner = self.inner.lock().await;
        inner.current.active_tasks = inner.current.active_tasks.saturating_sub(1);
    }

    /// Record disk usage.
    pub async fn record_disk(&self, bytes: u64) {
        let mut inner = self.inner.lock().await;
        inner.current.disk_bytes = bytes;
    }

    /// Record open file count.
    pub async fn record_open_files(&self, count: u32) {
        let mut inner = self.inner.lock().await;
        inner.current.open_files = count;
    }

    /// Get a snapshot of current resource usage.
    pub async fn snapshot(&self) -> ResourceSnapshot {
        let inner = self.inner.lock().await;
        ResourceSnapshot {
            memory_bytes: inner.current.memory_bytes,
            open_files: inner.current.open_files,
            active_tasks: inner.current.active_tasks,
            disk_bytes: inner.current.disk_bytes,
            timestamp: Instant::now(),
        }
    }

    /// Check all thresholds and return any violations.
    pub async fn check_violations(&self) -> Vec<ResourceViolation> {
        let inner = self.inner.lock().await;
        let mut violations = Vec::new();

        if inner.current.memory_bytes > inner.thresholds.max_memory_bytes {
            violations.push(ResourceViolation {
                resource: ResourceKind::Memory,
                current_value: inner.current.memory_bytes,
                threshold: inner.thresholds.max_memory_bytes,
                message: format!(
                    "Memory usage {}MB exceeds threshold {}MB",
                    inner.current.memory_bytes / (1024 * 1024),
                    inner.thresholds.max_memory_bytes / (1024 * 1024)
                ),
            });
        }

        if inner.current.open_files > inner.thresholds.max_open_files {
            violations.push(ResourceViolation {
                resource: ResourceKind::OpenFiles,
                current_value: u64::from(inner.current.open_files),
                threshold: u64::from(inner.thresholds.max_open_files),
                message: format!(
                    "Open files {} exceeds threshold {}",
                    inner.current.open_files, inner.thresholds.max_open_files
                ),
            });
        }

        if inner.current.active_tasks > inner.thresholds.max_active_tasks {
            violations.push(ResourceViolation {
                resource: ResourceKind::ActiveTasks,
                current_value: u64::from(inner.current.active_tasks),
                threshold: u64::from(inner.thresholds.max_active_tasks),
                message: format!(
                    "Active tasks {} exceeds threshold {}",
                    inner.current.active_tasks, inner.thresholds.max_active_tasks
                ),
            });
        }

        if inner.current.disk_bytes > inner.thresholds.max_disk_bytes {
            violations.push(ResourceViolation {
                resource: ResourceKind::DiskUsage,
                current_value: inner.current.disk_bytes,
                threshold: inner.thresholds.max_disk_bytes,
                message: format!(
                    "Disk usage {}MB exceeds threshold {}MB",
                    inner.current.disk_bytes / (1024 * 1024),
                    inner.thresholds.max_disk_bytes / (1024 * 1024)
                ),
            });
        }

        violations
    }

    /// Returns true if any threshold is exceeded.
    pub async fn has_violations(&self) -> bool {
        !self.check_violations().await.is_empty()
    }

    /// Update thresholds at runtime.
    pub async fn update_thresholds(&self, thresholds: ResourceThresholds) {
        let mut inner = self.inner.lock().await;
        inner.thresholds = thresholds;
    }

    /// Get the utilization ratio (0.0 to 1.0+) for each resource, indicating
    /// how close it is to exceeding the threshold.
    pub async fn utilization(&self) -> ResourceUtilization {
        let inner = self.inner.lock().await;
        ResourceUtilization {
            memory: inner.current.memory_bytes as f64 / inner.thresholds.max_memory_bytes as f64,
            open_files: f64::from(inner.current.open_files) / f64::from(inner.thresholds.max_open_files),
            active_tasks: f64::from(inner.current.active_tasks)
                / f64::from(inner.thresholds.max_active_tasks),
            disk: inner.current.disk_bytes as f64 / inner.thresholds.max_disk_bytes as f64,
        }
    }
}

/// Utilization ratios for each monitored resource.
///
/// Values are in the range [0.0, ∞), where 1.0 means the threshold is reached.
#[derive(Debug, Clone)]
pub struct ResourceUtilization {
    pub memory: f64,
    pub open_files: f64,
    pub active_tasks: f64,
    pub disk: f64,
}

impl ResourceUtilization {
    /// Returns the highest utilization ratio across all resources.
    pub fn max_utilization(&self) -> f64 {
        self.memory
            .max(self.open_files)
            .max(self.active_tasks)
            .max(self.disk)
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resource_monitor_no_violations() {
        let monitor = ResourceMonitor::with_defaults();
        monitor.record_memory(100 * 1024 * 1024).await; // 100MB
        monitor.record_open_files(50).await;

        let violations = monitor.check_violations().await;
        assert!(violations.is_empty());
    }

    #[tokio::test]
    async fn test_resource_monitor_memory_violation() {
        let monitor = ResourceMonitor::new(ResourceThresholds {
            max_memory_bytes: 100 * 1024 * 1024, // 100MB
            ..Default::default()
        });

        monitor.record_memory(200 * 1024 * 1024).await; // 200MB

        let violations = monitor.check_violations().await;
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].resource, ResourceKind::Memory);
    }

    #[tokio::test]
    async fn test_resource_monitor_task_tracking() {
        let monitor = ResourceMonitor::new(ResourceThresholds {
            max_active_tasks: 2,
            ..Default::default()
        });

        monitor.task_started().await;
        monitor.task_started().await;
        assert!(!monitor.has_violations().await);

        monitor.task_started().await; // Exceeds threshold.
        assert!(monitor.has_violations().await);

        monitor.task_completed().await;
        assert!(!monitor.has_violations().await);
    }

    #[tokio::test]
    async fn test_resource_monitor_snapshot() {
        let monitor = ResourceMonitor::with_defaults();
        monitor.record_memory(100).await;
        monitor.record_disk(200).await;
        monitor.record_open_files(5).await;
        monitor.task_started().await;
        monitor.task_started().await;

        let snap = monitor.snapshot().await;
        assert_eq!(snap.memory_bytes, 100);
        assert_eq!(snap.disk_bytes, 200);
        assert_eq!(snap.open_files, 5);
        assert_eq!(snap.active_tasks, 2);
    }

    #[tokio::test]
    async fn test_resource_monitor_utilization() {
        let monitor = ResourceMonitor::new(ResourceThresholds {
            max_memory_bytes: 100,
            max_open_files: 10,
            max_active_tasks: 4,
            max_disk_bytes: 1000,
        });

        monitor.record_memory(50).await;
        monitor.record_open_files(5).await;
        monitor.task_started().await;
        monitor.record_disk(250).await;

        let util = monitor.utilization().await;
        assert!((util.memory - 0.5).abs() < 0.001);
        assert!((util.open_files - 0.5).abs() < 0.001);
        assert!((util.active_tasks - 0.25).abs() < 0.001);
        assert!((util.disk - 0.25).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_resource_monitor_multiple_violations() {
        let monitor = ResourceMonitor::new(ResourceThresholds {
            max_memory_bytes: 100,
            max_open_files: 10,
            max_active_tasks: 5,
            max_disk_bytes: 1000,
        });

        monitor.record_memory(200).await;
        monitor.record_open_files(20).await;

        let violations = monitor.check_violations().await;
        assert_eq!(violations.len(), 2);
    }

    #[tokio::test]
    async fn test_resource_monitor_update_thresholds() {
        let monitor = ResourceMonitor::new(ResourceThresholds {
            max_memory_bytes: 100,
            ..Default::default()
        });

        monitor.record_memory(150).await;
        assert!(monitor.has_violations().await);

        // Raise the threshold.
        monitor
            .update_thresholds(ResourceThresholds {
                max_memory_bytes: 200,
                ..Default::default()
            })
            .await;
        assert!(!monitor.has_violations().await);
    }

    #[tokio::test]
    async fn test_resource_monitor_task_saturating_sub() {
        let monitor = ResourceMonitor::with_defaults();
        // Should not underflow.
        monitor.task_completed().await;
        let snap = monitor.snapshot().await;
        assert_eq!(snap.active_tasks, 0);
    }

    #[tokio::test]
    async fn test_resource_utilization_max() {
        let util = ResourceUtilization {
            memory: 0.5,
            open_files: 0.8,
            active_tasks: 0.3,
            disk: 0.1,
        };
        assert!((util.max_utilization() - 0.8).abs() < 0.001);
    }
}
