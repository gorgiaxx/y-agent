//! y-core: Core abstractions and traits for y-agent.
//!
//! This crate defines the contracts between all other y-agent crates.
//! Every cross-boundary interaction is mediated by a trait defined here.
//!
//! # Module Overview
//!
//! | Module | Key Traits | Design Reference |
//! |--------|-----------|-----------------|
//! | [`types`] | Shared IDs, `Message`, `TokenUsage` | - |
//! | [`error`] | `ClassifiedError`, `Redactable` | - |
//! | [`provider`] | `LlmProvider`, `ProviderPool` | providers-design.md |
//! | [`runtime`] | `RuntimeAdapter` | runtime-design.md |
//! | [`tool`] | `Tool`, `ToolRegistry` | tools-design.md |
//! | [`session`] | `SessionStore`, `TranscriptStore` | context-session-design.md |
//! | [`memory`] | `MemoryClient`, `ExperienceStore` | memory-architecture-design.md |
//! | [`checkpoint`] | `CheckpointStorage` | orchestrator-design.md |
//! | [`hook`] | `Middleware`, `HookHandler`, `EventSubscriber` | hooks-plugin-design.md |
//! | [`skill`] | `SkillRegistry` | skills-knowledge-design.md |

pub mod checkpoint;
pub mod error;
pub mod hook;
pub mod memory;
pub mod provider;
pub mod runtime;
pub mod session;
pub mod skill;
pub mod tool;
pub mod types;
