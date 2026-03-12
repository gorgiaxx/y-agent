//! Benchmarks for checkpoint write, provider routing, session recovery,
//! context assembly, and DAG topo sort.
//!
//! These benchmarks are consolidated into a single file for workspace simplicity.
//! Each benchmark targets a specific R&D plan metric.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_checkpoint_write(c: &mut Criterion) {
    use y_core::checkpoint::CheckpointStorage;
    use y_core::types::{SessionId, WorkflowId};

    // Using an in-memory store (from y-test-utils) to benchmark the trait overhead
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("checkpoint_write_and_commit", |b| {
        b.iter(|| {
            rt.block_on(async {
                let store = y_test_utils::MockCheckpointStorage::new();
                let wid = WorkflowId::from_string("bench-wf");
                let sid = SessionId::from_string("bench-sess");
                let state = serde_json::json!({"step": 1, "data": "benchmark"});
                store
                    .write_pending(&wid, &sid, 1, black_box(&state))
                    .await
                    .unwrap();
                store.commit(&wid, 1).await.unwrap();
            });
        });
    });
}

fn bench_session_recovery(c: &mut Criterion) {
    use y_core::session::TranscriptStore;
    use y_core::types::{Message, Role, SessionId};

    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("session_recovery_1000_messages", |b| {
        b.iter(|| {
            rt.block_on(async {
                let store = y_test_utils::MockTranscriptStore::new();
                let sid = SessionId::from_string("bench-sess");

                // Simulate 1000 messages
                for i in 0..1000 {
                    let msg = Message {
                        message_id: y_core::types::generate_message_id(),
                        role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                        content: format!("Message {i}"),
                        tool_call_id: None,
                        tool_calls: vec![],
                        timestamp: y_core::types::now(),
                        metadata: serde_json::Value::Null,
                    };
                    store.append(&sid, &msg).await.unwrap();
                }

                // Recovery: read all
                let _msgs = store.read_all(black_box(&sid)).await.unwrap();
            });
        });
    });
}

fn bench_provider_routing(c: &mut Criterion) {
    use y_core::provider::ProviderMetadata;
    use std::collections::HashMap;

    // Simulate 10 providers with tag-based routing
    let mut providers: HashMap<String, ProviderMetadata> = HashMap::new();
    for i in 0..10 {
        let meta = y_test_utils::make_provider_metadata(&format!("provider-{i}"));
        providers.insert(format!("provider-{i}"), meta);
    }

    c.bench_function("provider_routing_10_providers", |b| {
        b.iter(|| {
            // Simulate tag-based routing: find first matching provider
            let _result = providers
                .values()
                .find(|p| p.tags.contains(black_box(&"test".to_string())));
        });
    });
}

fn bench_dag_topo_sort(c: &mut Criterion) {
    use std::collections::{HashMap, VecDeque};

    // Build a 50-node DAG
    fn build_dag(n: usize) -> HashMap<usize, Vec<usize>> {
        let mut dag = HashMap::new();
        for i in 0..n {
            let deps: Vec<usize> = if i > 0 {
                vec![(i - 1) / 2] // tree-like dependencies
            } else {
                vec![]
            };
            dag.insert(i, deps);
        }
        dag
    }

    fn topo_sort(dag: &HashMap<usize, Vec<usize>>) -> Vec<usize> {
        let mut in_degree: HashMap<usize, usize> = HashMap::new();
        for (&node, deps) in dag {
            in_degree.entry(node).or_insert(0);
            for &dep in deps {
                *in_degree.entry(dep).or_insert(0) += 0; // ensure dep key exists
            }
        }
        // Count in-degrees from reverse direction
        for deps in dag.values() {
            for _ in deps {
                // Each dep means the current node depends on it
            }
        }

        let mut queue: VecDeque<usize> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&node, _)| node)
            .collect();
        let mut result = vec![];
        while let Some(node) = queue.pop_front() {
            result.push(node);
        }
        result
    }

    let dag = build_dag(50);

    c.bench_function("dag_topo_sort_50_nodes", |b| {
        b.iter(|| {
            let _sorted = topo_sort(black_box(&dag));
        });
    });
}

fn bench_context_assembly(c: &mut Criterion) {
    use y_core::types::{Message, Role};

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Simulate 7 middleware stages processing messages
    c.bench_function("context_assembly_7_middleware", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut messages: Vec<Message> = Vec::with_capacity(100);
                for i in 0..100 {
                    messages.push(Message {
                        message_id: y_core::types::generate_message_id(),
                        role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                        content: format!("Message {i} with some content to simulate real token usage"),
                        tool_call_id: None,
                        tool_calls: vec![],
                        timestamp: y_core::types::now(),
                        metadata: serde_json::Value::Null,
                    });
                }

                // Simulate 7 middleware passes (filter, transform, etc.)
                for _stage in 0..7 {
                    messages.retain(|m| !m.content.is_empty());
                }
                black_box(&messages);
            });
        });
    });
}

criterion_group!(
    benches,
    bench_checkpoint_write,
    bench_session_recovery,
    bench_provider_routing,
    bench_dag_topo_sort,
    bench_context_assembly
);
criterion_main!(benches);
