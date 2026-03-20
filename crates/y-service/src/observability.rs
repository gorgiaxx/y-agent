//! Global observability service -- live system state snapshots.
//!
//! Composes data from `ProviderPoolImpl`, `AgentPool`, and
//! `PriorityScheduler` into a single [`SystemSnapshot`] that
//! presentation layers (GUI, CLI, TUI) can poll.

use std::collections::HashMap;

use serde::Serialize;
use y_core::provider::ProviderPool;

use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Snapshot data structures
// ---------------------------------------------------------------------------

/// Top-level point-in-time snapshot of the entire system.
#[derive(Debug, Clone, Serialize)]
pub struct SystemSnapshot {
    /// UTC timestamp when this snapshot was captured.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Per-provider state (status, concurrency, metrics).
    pub providers: Vec<ProviderSnapshot>,
    /// Agent pool state (instances, concurrency).
    pub agents: AgentPoolSnapshot,
    /// Priority scheduler queue state (if wired).
    pub scheduler: Option<SchedulerQueueSnapshot>,
}

/// Combined per-provider state: metadata + freeze status + concurrency + metrics.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderSnapshot {
    // -- identity --
    pub id: String,
    pub model: String,
    pub provider_type: String,
    pub tags: Vec<String>,
    // -- freeze status --
    pub is_frozen: bool,
    pub freeze_reason: Option<String>,
    // -- concurrency --
    pub max_concurrency: usize,
    pub active_requests: usize,
    // -- cumulative metrics --
    pub total_requests: u64,
    pub total_errors: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub error_rate: f64,
}

/// Aggregate agent pool snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct AgentPoolSnapshot {
    /// Total instances tracked (including terminal).
    pub total_instances: usize,
    /// Non-terminal (active) instances.
    pub active_instances: usize,
    /// Remaining concurrency slots.
    pub available_slots: usize,
    /// Per-instance details.
    pub instances: Vec<AgentInstanceSnapshot>,
}

/// Per-instance snapshot of an agent.
#[derive(Debug, Clone, Serialize)]
pub struct AgentInstanceSnapshot {
    pub instance_id: String,
    pub agent_name: String,
    /// Lifecycle state as a string (e.g. "Creating", "Running", "Completed").
    pub state: String,
    pub delegation_id: Option<String>,
    pub iterations: usize,
    pub tool_calls: usize,
    pub tokens_used: u64,
    pub elapsed_ms: u64,
    pub delegation_depth: u32,
}

/// Priority scheduler queue snapshot, grouped by priority tier.
#[derive(Debug, Clone, Serialize)]
pub struct SchedulerQueueSnapshot {
    pub active_critical: u64,
    pub active_normal: u64,
    pub active_idle: u64,
    pub total_capacity: usize,
    pub critical_reserve_pct: u8,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Provides a unified, read-only view of runtime system state.
///
/// All data is gathered from in-memory state -- no database queries.
/// Presentation layers should poll this at a reasonable interval
/// (e.g. 1-2 seconds) for live dashboards.
pub struct ObservabilityService;

impl ObservabilityService {
    /// Capture a point-in-time snapshot of the entire system.
    ///
    /// Gathers:
    /// 1. Provider pool: freeze status, concurrency, cumulative metrics.
    /// 2. Agent pool: all instances with lifecycle state and resource usage.
    /// 3. Priority scheduler: per-tier active counts (when wired).
    pub async fn snapshot(container: &ServiceContainer) -> SystemSnapshot {
        let providers = Self::build_provider_snapshots(container).await;
        let agents = Self::build_agent_pool_snapshot(container).await;

        SystemSnapshot {
            timestamp: chrono::Utc::now(),
            providers,
            agents,
            // PriorityScheduler is not yet wired into ProviderPoolImpl,
            // so this field remains None until that integration lands.
            scheduler: None,
        }
    }

    /// Build per-provider snapshots by merging metadata, status, and metrics.
    async fn build_provider_snapshots(container: &ServiceContainer) -> Vec<ProviderSnapshot> {
        let pool = container.provider_pool().await;

        // Gather the three data sources.
        let metadata_list = pool.list_metadata();
        let statuses = pool.provider_statuses().await;
        let metrics_list = pool.all_metrics();

        // Index statuses and metrics by provider ID for O(1) lookup.
        let status_map: HashMap<String, _> = statuses
            .into_iter()
            .map(|s| (s.id.to_string(), s))
            .collect();
        let metrics_map: HashMap<String, _> = metrics_list
            .into_iter()
            .map(|(id, snap)| (id.to_string(), snap))
            .collect();

        metadata_list
            .iter()
            .map(|meta| {
                let id_str = meta.id.to_string();
                let status = status_map.get(&id_str);
                let metrics = metrics_map.get(&id_str);

                ProviderSnapshot {
                    id: id_str,
                    model: meta.model.clone(),
                    provider_type: format!("{:?}", meta.provider_type),
                    tags: meta.tags.clone(),
                    is_frozen: status.is_some_and(|s| s.is_frozen),
                    freeze_reason: status.and_then(|s| s.freeze_reason.clone()),
                    max_concurrency: meta.max_concurrency,
                    active_requests: status.map_or(0, |s| s.active_requests),
                    total_requests: metrics.map_or(0, |m| m.total_requests),
                    total_errors: metrics.map_or(0, |m| m.total_errors),
                    total_input_tokens: metrics.map_or(0, |m| m.total_input_tokens),
                    total_output_tokens: metrics.map_or(0, |m| m.total_output_tokens),
                    estimated_cost_usd: metrics
                        .map_or(0.0, y_provider::MetricsSnapshot::estimated_cost_usd),
                    error_rate: metrics.map_or(0.0, y_provider::MetricsSnapshot::error_rate),
                }
            })
            .collect()
    }

    /// Build the agent pool snapshot from the pool's internal state and active delegations.
    async fn build_agent_pool_snapshot(container: &ServiceContainer) -> AgentPoolSnapshot {
        let pool = container.agent_pool.lock().await;

        let all = pool.list_all();
        let active = pool.list_active();
        let available = pool.available_slots();

        let mut instances: Vec<AgentInstanceSnapshot> = all
            .iter()
            .map(|inst| AgentInstanceSnapshot {
                instance_id: inst.instance_id.clone(),
                agent_name: inst.definition.id.clone(),
                state: format!("{:?}", inst.state),
                delegation_id: inst.delegation_id.clone(),
                iterations: inst.iterations,
                tool_calls: inst.tool_calls,
                tokens_used: inst.tokens_used,
                elapsed_ms: inst.elapsed_ms(),
                delegation_depth: inst.delegation_depth,
            })
            .collect();

        // Drop the pool lock before reading the delegation tracker.
        let pool_total = all.len();
        let pool_active = active.len();
        drop(pool);

        // Merge active delegations from the delegator pool's tracker.
        // These are agents (e.g. title-generator) that run via `delegate()`
        // and bypass the pool's instance HashMap.
        let delegations = container.delegation_tracker.active_delegations();
        for d in &delegations {
            instances.push(AgentInstanceSnapshot {
                instance_id: d.id.clone(),
                agent_name: d.agent_name.clone(),
                state: "Running".to_string(),
                delegation_id: Some(d.id.clone()),
                iterations: 0,
                tool_calls: 0,
                tokens_used: 0,
                elapsed_ms: u64::try_from(d.start_time.elapsed().as_millis()).unwrap_or(u64::MAX),
                delegation_depth: 0,
            });
        }

        AgentPoolSnapshot {
            total_instances: pool_total + delegations.len(),
            active_instances: pool_active + delegations.len(),
            available_slots: available,
            instances,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServiceConfig;
    use crate::container::ServiceContainer;

    #[tokio::test]
    async fn test_snapshot_returns_valid_structure() {
        let mut config = ServiceConfig::default();
        config.storage.db_path = ":memory:".to_string();

        let container = ServiceContainer::from_config(&config)
            .await
            .expect("container should build with default config");

        let snap = ObservabilityService::snapshot(&container).await;

        // No providers configured in default config -> empty list.
        assert!(snap.providers.is_empty());

        // Agent pool should exist with zero instances.
        assert_eq!(snap.agents.total_instances, 0);
        assert_eq!(snap.agents.active_instances, 0);
        assert!(
            snap.agents.available_slots > 0,
            "should have concurrency slots"
        );
        assert!(snap.agents.instances.is_empty());

        // Scheduler not yet wired.
        assert!(snap.scheduler.is_none());

        // Timestamp should be recent.
        let now = chrono::Utc::now();
        let diff = (now - snap.timestamp).num_seconds().abs();
        assert!(diff < 5, "timestamp should be within 5 seconds of now");
    }
}
