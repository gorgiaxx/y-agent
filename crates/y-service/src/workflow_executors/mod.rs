//! Workflow task executor implementations.
//!
//! Each executor handles a specific [`TaskType`] variant from the orchestrator
//! DAG. All executors hold an `Arc<ServiceContainer>` to access shared
//! infrastructure (LLM providers, tool registry, agent delegator).
//!
//! [`TaskType`]: y_agent::orchestrator::dag::TaskType

pub mod fallback_llm;
pub mod llm_call;
pub mod sub_agent;
pub mod tool_exec;

pub use fallback_llm::FallbackLlmExecutor;
pub use llm_call::LlmCallExecutor;
pub use sub_agent::SubAgentExecutor;
pub use tool_exec::ToolExecutionExecutor;
