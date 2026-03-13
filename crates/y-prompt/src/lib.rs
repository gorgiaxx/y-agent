//! y-prompt: Prompt assembly and template system.
//!
//! This crate provides the prompt composition infrastructure for y-agent:
//!
//! - [`PromptSection`] — typed, prioritized, token-budgeted prompt fragments
//! - [`PromptTemplate`] — declarative composition of sections with mode overlays
//! - [`SectionStore`] — section persistence and content loading
//! - [`budget`] — token estimation and budget-based truncation
//!
//! # Design
//!
//! Prompts are composed from reusable [`PromptSection`] units. Each section has
//! a semantic category, assembly priority, token budget, and an optional
//! activation condition. A [`PromptTemplate`] declares which sections to
//! include and supports mode overlays (build/plan/explore/general) that
//! toggle sections without content duplication.
//!
//! Sections are lazily loaded: the template declares references, but content
//! is fetched from the [`SectionStore`] only when the condition evaluates
//! to true. This saves 60-90% of token usage compared to monolithic prompts.

pub mod budget;
pub mod builtins;
pub mod section;
pub mod store;
pub mod template;

// Re-export primary types.
pub use budget::{estimate_tokens, truncate_to_budget};
pub use builtins::{
    builtin_section_store, builtin_section_store_with_overrides, default_template,
    BUILTIN_PROMPT_FILES,
};
pub use section::{
    ContentSource, PromptContext, PromptSection, SectionCategory, SectionCondition, SectionId,
    TemplateId,
};
pub use store::{SectionStore, StoreError};
pub use template::{EffectiveSection, ModeOverlay, PromptTemplate, SectionRef};
