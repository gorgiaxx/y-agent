//! y-scheduler: Scheduled task execution for y-agent.
//!
//! This crate provides time-based and event-driven task scheduling:
//!
//! - [`CronSchedule`] — cron expression-based triggers
//! - [`IntervalSchedule`] — fixed-interval triggers
//! - [`EventSchedule`] — event-driven triggers via `y-hooks`
//! - [`ScheduleStore`] — schedule registry with CRUD operations
//! - [`ScheduleExecutor`] — trigger-to-workflow execution translation
//!
//! Scheduled tasks execute as standard Orchestrator Workflows,
//! supporting parameterized scheduling via `ParameterSchema`.

pub mod config;
pub mod cron;
pub mod event;
pub mod executor;
pub mod interval;
pub mod store;

// Re-export primary types.
pub use config::{ConcurrencyPolicy, MissedPolicy, SchedulerConfig};
pub use cron::CronSchedule;
pub use event::EventSchedule;
pub use executor::{ExecutionStatus, ScheduleContext, ScheduleExecution, ScheduleExecutor};
pub use interval::IntervalSchedule;
pub use store::{Schedule, ScheduleStore, TriggerConfig};
