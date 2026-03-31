//! y-tools: Tool Registry, lazy loading, JSON Schema validation, LRU activation.
//!
//! This crate provides the tool management system for y-agent:
//!
//! - [`ToolRegistryImpl`] — manages tool registration, lookup, and search
//! - [`ToolIndex`] — compact entries for LLM context (name+description only)
//! - [`ToolActivationSet`] — LRU cache of active tools (ceiling: 20)
//! - [`JsonSchemaValidator`] — parameter validation with compiled schema cache
//! - [`ToolExecutor`] — validates + runs tools through middleware chain
//! - [`DynamicToolManager`] — CRUD lifecycle for agent-created tools
//! - [`RateLimiter`] — per-tool token-bucket rate limiting
//! - [`ResultFormatter`] — formats tool output for LLM consumption
//! - [`builtin::tool_search`] — meta-tool for lazy tool loading
//! - [`builtin::register_builtin_tools`] — registers all built-in tools
//!
//! # Lazy Loading Design
//!
//! Tools are not loaded into context until the LLM needs them. The compact
//! index is always present, and the LLM calls `ToolSearch` to activate
//! specific tools' full definitions. This saves 60-90% of token usage.

pub mod activation;
pub mod builtin;
pub mod config;
pub mod dynamic;
pub mod error;
pub mod executor;
pub mod formatter;
pub mod index;
pub mod mcp_integration;
pub mod parser;
pub mod rate_limiter;
pub mod registry;
pub mod taxonomy;
pub mod validator;

// Re-export primary types.
pub use activation::ToolActivationSet;
pub use config::ToolRegistryConfig;
pub use dynamic::{DynamicToolDef, DynamicToolKind, DynamicToolManager};
pub use error::ToolRegistryError;
pub use executor::ToolExecutor;
pub use formatter::{FormattedResult, FormatterConfig, ResultFormat, ResultFormatter};
pub use index::ToolIndex;
pub use mcp_integration::{McpDiscoveryResult, McpServerConfig};
pub use parser::{
    format_tool_result, parse_tool_calls, strip_tool_call_blocks, ParseResult, ParsedToolCall,
};
pub use rate_limiter::{RateLimitConfig, RateLimitResult, RateLimiter};
pub use registry::ToolRegistryImpl;
pub use taxonomy::ToolTaxonomy;
pub use validator::JsonSchemaValidator;
