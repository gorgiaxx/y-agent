//! y-tools: Tool Registry, lazy loading, JSON Schema validation, LRU activation.
//!
//! This crate provides the tool management system for y-agent:
//!
//! - [`ToolRegistryImpl`] — manages tool registration, lookup, and search
//! - [`ToolIndex`] — compact entries for LLM context (name+description only)
//! - [`ToolActivationSet`] — LRU cache of active tools (ceiling: 20)
//! - [`JsonSchemaValidator`] — parameter validation with compiled schema cache
//! - [`ToolExecutor`] — validates + runs tools through middleware chain
//! - [`builtin::tool_search`] — meta-tool for lazy tool loading
//!
//! # Lazy Loading Design
//!
//! Tools are not loaded into context until the LLM needs them. The compact
//! index is always present, and the LLM calls `tool_search` to activate
//! specific tools' full definitions. This saves 60-90% of token usage.

pub mod activation;
pub mod builtin;
pub mod config;
pub mod error;
pub mod executor;
pub mod index;
pub mod registry;
pub mod validator;

// Re-export primary types.
pub use activation::ToolActivationSet;
pub use config::ToolRegistryConfig;
pub use error::ToolRegistryError;
pub use executor::ToolExecutor;
pub use index::ToolIndex;
pub use registry::ToolRegistryImpl;
pub use validator::JsonSchemaValidator;
