//! y-core: Core abstractions and traits for y-agent.
//!
//! This crate defines the contracts between all other y-agent crates.
//! Every cross-boundary interaction is mediated by a trait defined here.
//!
//! # Module Overview
//!
//! | Module | Key Traits |
//! |--------|------------|
//! | [`agent`] | `AgentDelegator`, `ContextStrategyHint`, `DelegationOutput` |
//! | [`types`] | Shared IDs, `Message`, `TokenUsage` |
//! | [`error`] | `ClassifiedError`, `Redactable` |
//! | [`provider`] | `LlmProvider`, `ProviderPool` |
//! | [`runtime`] | `RuntimeAdapter` |
//! | [`tool`] | `Tool`, `ToolRegistry` |
//! | [`session`] | `SessionStore`, `TranscriptStore` |
//! | [`memory`] | `MemoryClient`, `ExperienceStore` |
//! | [`checkpoint`] | `CheckpointStorage` |
//! | [`hook`] | `Middleware`, `HookHandler`, `EventSubscriber` |
//! | [`skill`] | `SkillRegistry` |
//! | [`embedding`] | `EmbeddingProvider` |
//! | [`permission_types`] | `PermissionBehavior`, `PermissionRule`, `PermissionMode` |

pub mod agent;
pub mod checkpoint;
pub mod embedding;
pub mod error;
pub mod hook;
pub mod memory;
pub mod permission_types;
pub mod platform;
pub mod provider;
pub mod runtime;
pub mod session;
pub mod skill;
pub mod template;
pub mod tool;
pub mod trust;
pub mod types;
