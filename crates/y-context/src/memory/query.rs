//! `TypedQuery`: intent-aware query decomposition.
//!
//! Decomposes complex queries into typed sub-queries for targeted recall.

use y_core::memory::MemoryType;

/// Type of query intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryType {
    /// Searching for factual/domain knowledge.
    Factual,
    /// Searching for personal preferences/patterns.
    Personal,
    /// Searching for tool usage tips.
    ToolUsage,
    /// Searching for past task experiences.
    Experience,
}

/// A typed sub-query with intent classification.
#[derive(Debug, Clone)]
pub struct TypedQuery {
    pub query: String,
    pub query_type: QueryType,
    pub memory_type_filter: Option<MemoryType>,
}

/// Decompose a complex query into typed sub-queries.
///
/// In production, this would use an LLM for intent classification.
/// This implementation uses keyword-based heuristics.
pub fn decompose(query: &str) -> Vec<TypedQuery> {
    let lower = query.to_lowercase();

    // Check for personal keywords
    if lower.contains("my ") || lower.contains("prefer") || lower.contains("style") {
        return vec![TypedQuery {
            query: query.to_string(),
            query_type: QueryType::Personal,
            memory_type_filter: Some(MemoryType::Personal),
        }];
    }

    // Check for tool keywords
    if lower.contains("tool") || lower.contains("command") || lower.contains("how to use") {
        return vec![TypedQuery {
            query: query.to_string(),
            query_type: QueryType::ToolUsage,
            memory_type_filter: Some(MemoryType::Tool),
        }];
    }

    // Check for experience keywords
    if lower.contains("last time") || lower.contains("previously") || lower.contains("experience") {
        return vec![TypedQuery {
            query: query.to_string(),
            query_type: QueryType::Experience,
            memory_type_filter: Some(MemoryType::Experience),
        }];
    }

    // Default: factual query, no type filter
    vec![TypedQuery {
        query: query.to_string(),
        query_type: QueryType::Factual,
        memory_type_filter: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-MEM-006-01: Complex query decomposes correctly.
    #[test]
    fn test_typed_query_decomposition() {
        let queries = decompose("how to use cargo test command");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query_type, QueryType::ToolUsage);
        assert_eq!(queries[0].memory_type_filter, Some(MemoryType::Tool));
    }

    /// T-MEM-006-02: Simple query gets default type.
    #[test]
    fn test_typed_query_single_type() {
        let queries = decompose("what is Rust ownership");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query_type, QueryType::Factual);
        assert_eq!(queries[0].memory_type_filter, None);
    }

    /// T-MEM-006-03: Personal filter applied.
    #[test]
    fn test_typed_query_personal_filter() {
        let queries = decompose("my preferred coding style");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query_type, QueryType::Personal);
        assert_eq!(queries[0].memory_type_filter, Some(MemoryType::Personal));
    }
}
