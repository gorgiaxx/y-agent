//! Deterministic retrieval-quality evaluation.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// One benchmark query represented by its relevant and retrieved chunk IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationCase {
    /// Chunk IDs considered relevant for the query.
    pub relevant_chunk_ids: Vec<String>,
    /// Retrieved chunk IDs ordered from most to least relevant.
    pub ranked_chunk_ids: Vec<String>,
}

impl EvaluationCase {
    /// Create a benchmark case from iterable chunk-ID collections.
    pub fn new<R, I, RS, IS>(relevant_chunk_ids: R, ranked_chunk_ids: I) -> Self
    where
        R: IntoIterator<Item = RS>,
        I: IntoIterator<Item = IS>,
        RS: Into<String>,
        IS: Into<String>,
    {
        Self {
            relevant_chunk_ids: relevant_chunk_ids.into_iter().map(Into::into).collect(),
            ranked_chunk_ids: ranked_chunk_ids.into_iter().map(Into::into).collect(),
        }
    }
}

/// Macro-averaged information-retrieval metrics for a benchmark corpus.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RetrievalEvaluationMetrics {
    /// Number of benchmark queries with at least one relevant chunk.
    pub query_count: usize,
    /// Mean recall among the first five results.
    pub recall_at_5: f64,
    /// Mean recall among the first ten results.
    pub recall_at_10: f64,
    /// Mean reciprocal rank of the first relevant result within ten results.
    pub mrr_at_10: f64,
    /// Mean normalized discounted cumulative gain within ten results.
    pub ndcg_at_10: f64,
}

/// Evaluate ranked retrieval outputs with binary relevance judgments.
///
/// Cases without relevant chunks are excluded because recall and nDCG are
/// undefined for them. Duplicate retrieved IDs are counted at most once.
pub fn evaluate_rankings(cases: &[EvaluationCase]) -> RetrievalEvaluationMetrics {
    let mut metrics = RetrievalEvaluationMetrics::default();

    for case in cases {
        let relevant: HashSet<&str> = case.relevant_chunk_ids.iter().map(String::as_str).collect();
        if relevant.is_empty() {
            continue;
        }

        metrics.query_count += 1;
        metrics.recall_at_5 += recall_at_k(case, &relevant, 5);
        metrics.recall_at_10 += recall_at_k(case, &relevant, 10);
        metrics.mrr_at_10 += reciprocal_rank_at_k(case, &relevant, 10);
        metrics.ndcg_at_10 += ndcg_at_k(case, &relevant, 10);
    }

    if metrics.query_count > 0 {
        let count = metrics.query_count as f64;
        metrics.recall_at_5 /= count;
        metrics.recall_at_10 /= count;
        metrics.mrr_at_10 /= count;
        metrics.ndcg_at_10 /= count;
    }

    metrics
}

fn recall_at_k(case: &EvaluationCase, relevant: &HashSet<&str>, k: usize) -> f64 {
    let retrieved_relevant = unique_relevant_ranks(case, relevant, k).count();
    retrieved_relevant as f64 / relevant.len() as f64
}

fn reciprocal_rank_at_k(case: &EvaluationCase, relevant: &HashSet<&str>, k: usize) -> f64 {
    unique_relevant_ranks(case, relevant, k)
        .next()
        .map_or(0.0, |rank| 1.0 / rank as f64)
}

fn ndcg_at_k(case: &EvaluationCase, relevant: &HashSet<&str>, k: usize) -> f64 {
    let dcg: f64 = unique_relevant_ranks(case, relevant, k)
        .map(discount_for_rank)
        .sum();
    let ideal_count = relevant.len().min(k);
    let ideal_dcg: f64 = (1..=ideal_count).map(discount_for_rank).sum();
    if ideal_dcg == 0.0 {
        0.0
    } else {
        dcg / ideal_dcg
    }
}

fn unique_relevant_ranks<'a>(
    case: &'a EvaluationCase,
    relevant: &'a HashSet<&'a str>,
    k: usize,
) -> impl Iterator<Item = usize> + 'a {
    let mut seen = HashSet::new();
    case.ranked_chunk_ids
        .iter()
        .take(k)
        .enumerate()
        .filter_map(move |(index, chunk_id)| {
            let id = chunk_id.as_str();
            (relevant.contains(id) && seen.insert(id)).then_some(index + 1)
        })
}

fn discount_for_rank(rank: usize) -> f64 {
    1.0 / (rank as f64 + 1.0).log2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::{Chunk, ChunkLevel, ChunkMetadata};
    use crate::retrieval::{HybridRetriever, RetrievalConfig, RetrievalFilter};
    use crate::tokenizer::AutoTokenizer;

    #[test]
    fn test_evaluate_rankings_computes_macro_ir_metrics() {
        let cases = vec![
            EvaluationCase::new(["a", "c"], ["x", "a", "c"]),
            EvaluationCase::new(["d"], ["d"]),
        ];

        let metrics = evaluate_rankings(&cases);

        assert_eq!(metrics.query_count, 2);
        assert!((metrics.recall_at_5 - 1.0).abs() < 1e-9);
        assert!((metrics.recall_at_10 - 1.0).abs() < 1e-9);
        assert!((metrics.mrr_at_10 - 0.75).abs() < 1e-9);
        assert!((metrics.ndcg_at_10 - 0.846_713_201_808_635_4).abs() < 1e-9);
    }

    #[test]
    fn test_controlled_hybrid_retrieval_corpus_exceeds_mrr_target() {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(AutoTokenizer::new(), config);
        retriever.index(chunk(
            "iso-part5-table6",
            "ISO 26262 Part 5 Table 6 defines hardware architectural metrics for random hardware failures.",
            "standards",
        ));
        retriever.index(chunk(
            "iso-part4-plan",
            "ISO 26262 Part 4 describes safety plans and system development activities.",
            "standards",
        ));
        retriever.index(chunk(
            "tokio-select",
            "The Rust tokio::select! macro waits on branches and cancels the remaining branches after one completes.",
            "api",
        ));
        retriever.index(chunk(
            "http-429",
            "HTTP status code 429 signals rate limiting; clients should honor Retry-After before retrying.",
            "api",
        ));
        retriever.index(chunk(
            "knowledge-zh",
            "知识库使用语义检索与关键词检索的混合排序来提高检索质量。",
            "knowledge",
        ));

        let cases = vec![
            benchmark_case(
                &retriever,
                "Which hardware architectural metrics are required by ISO 26262 Part 5 Table 6?",
                None,
                "iso-part5-table6",
            ),
            benchmark_case(
                &retriever,
                "tokio::select cancellation behavior",
                None,
                "tokio-select",
            ),
            benchmark_case(
                &retriever,
                "How should a client back off after HTTP error 429 with Retry-After?",
                Some("api"),
                "http-429",
            ),
            benchmark_case(
                &retriever,
                "如何提高知识库的混合检索质量",
                Some("knowledge"),
                "knowledge-zh",
            ),
        ];

        let metrics = evaluate_rankings(&cases);

        assert_eq!(metrics.query_count, 4);
        assert!(metrics.recall_at_5 >= 0.95, "metrics: {metrics:?}");
        assert!(metrics.mrr_at_10 > 0.7, "metrics: {metrics:?}");
    }

    fn chunk(id: &str, content: &str, collection: &str) -> Chunk {
        Chunk {
            id: id.to_string(),
            document_id: id.to_string(),
            level: ChunkLevel::L2,
            content: content.to_string(),
            token_estimate: 20,
            metadata: ChunkMetadata {
                domain: collection.to_string(),
                collection: collection.to_string(),
                title: id.to_string(),
                ..Default::default()
            },
        }
    }

    fn benchmark_case(
        retriever: &HybridRetriever<AutoTokenizer>,
        query: &str,
        collection: Option<&str>,
        relevant_id: &str,
    ) -> EvaluationCase {
        let filter = RetrievalFilter {
            collection: collection.map(String::from),
            limit: 10,
            ..Default::default()
        };
        let ranked = retriever
            .search(query, &filter)
            .into_iter()
            .map(|result| result.chunk.id)
            .collect::<Vec<_>>();
        EvaluationCase::new([relevant_id], ranked)
    }
}
