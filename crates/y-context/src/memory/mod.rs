//! Memory module: Working Memory, STM/LTM clients, dedup, query, search, recall.

pub mod deduplication;
pub mod ltm_client;
pub mod query;
pub mod recall_middleware;
pub mod search_orchestrator;
pub mod stm_client;
pub mod working_memory;

pub use deduplication::Deduplicator;
pub use ltm_client::LtmClient;
pub use query::{QueryType, TypedQuery};
pub use recall_middleware::RecallMiddleware;
pub use search_orchestrator::SearchOrchestrator;
pub use stm_client::StmClient;
pub use working_memory::{CognitiveCategory, WorkingMemory};
