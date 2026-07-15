use std::sync::{Arc, Mutex};

use y_context::KnowledgeContextRetriever;
use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};
use y_knowledge::middleware::InjectKnowledge;
use y_knowledge::retrieval::{HybridRetriever, RetrievalConfig};
use y_knowledge::tokenizer::AutoTokenizer;
use y_service::knowledge_context_retrieval::KnowledgeContextRetrievalAdapter;

#[tokio::test]
async fn adapter_retrieves_only_selected_collections_and_bounds_total_results() {
    let config = RetrievalConfig {
        min_similarity_threshold: 0.0,
        enable_dedup: false,
        ..Default::default()
    };
    let mut retriever = HybridRetriever::with_config(AutoTokenizer::new(), config);
    for collection in ["alpha", "beta", "unselected"] {
        for index in 0..4 {
            let id = format!("{collection}-{index}");
            retriever.index(Chunk {
                id: id.clone(),
                document_id: id.clone(),
                level: ChunkLevel::L0,
                content: "shared ownership guidance".to_string(),
                token_estimate: 5,
                metadata: ChunkMetadata {
                    title: id.clone(),
                    source: format!("file:///{id}.md"),
                    collection: collection.to_string(),
                    ..Default::default()
                },
            });
        }
    }
    let adapter = KnowledgeContextRetrievalAdapter::new(
        Arc::new(Mutex::new(InjectKnowledge::new(retriever))),
        None,
    );

    let snippets = adapter
        .retrieve(
            "shared ownership guidance",
            &["alpha".to_string(), "beta".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(snippets.len(), 5);
    assert!(snippets
        .iter()
        .all(|snippet| matches!(snippet.collection.as_str(), "alpha" | "beta")));
    assert!(snippets
        .iter()
        .all(|snippet| !snippet.source.is_empty() && !snippet.chunk_id.is_empty()));
}
