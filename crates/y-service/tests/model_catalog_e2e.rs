//! End-to-end coverage for the models.dev catalog service: download over HTTP,
//! cache in the config dir, then fuzzy-resolve a model id from the cache.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const FIXTURE: &str = r#"{
    "providers": {
        "anthropic": {
            "id": "anthropic",
            "name": "Anthropic",
            "api": "https://api.anthropic.com/v1",
            "env": ["ANTHROPIC_API_KEY"],
            "models": {
                "claude-opus-4-6": {
                    "id": "claude-opus-4-6",
                    "name": "Claude Opus 4.6",
                    "reasoning": true,
                    "tool_call": true,
                    "release_date": "2026-02-04",
                    "modalities": {"input": ["text", "image"], "output": ["text"]},
                    "limit": {"context": 200000, "output": 64000},
                    "cost": {"input": 5, "output": 25}
                }
            }
        },
        "openai": {
            "id": "openai",
            "name": "OpenAI",
            "env": ["OPENAI_API_KEY"],
            "models": {
                "gpt-4o": {
                    "id": "gpt-4o",
                    "name": "GPT-4o",
                    "reasoning": false,
                    "tool_call": true,
                    "release_date": "2024-05-13",
                    "modalities": {"input": ["text", "image"], "output": ["text"]},
                    "limit": {"context": 128000, "output": 16384},
                    "cost": {"input": 2.5, "output": 10}
                }
            }
        }
    },
    "models": {
        "anthropic/claude-opus-4-6": {"id": "anthropic/claude-opus-4-6"},
        "openai/gpt-4o": {"id": "openai/gpt-4o"}
    }
}"#;

/// Serve `FIXTURE` once over HTTP and return the bound address.
async fn spawn_catalog_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut scratch = [0u8; 2048];
        let _ = stream.read(&mut scratch).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{FIXTURE}",
            FIXTURE.len()
        );
        stream.write_all(response.as_bytes()).await.expect("write");
        stream.flush().await.expect("flush");
    });
    addr
}

#[tokio::test]
async fn downloads_caches_and_resolves_models() {
    let dir = tempfile::tempdir().expect("tempdir");
    let addr = spawn_catalog_server().await;
    let url = format!("http://{addr}/api.json");

    let summary = y_service::update_catalog(dir.path(), Some(&url))
        .await
        .expect("catalog downloads");

    assert_eq!(summary.provider_count, 2);
    assert_eq!(summary.model_count, 2);
    assert_eq!(summary.source_url, url);

    // The catalog is stored verbatim in the config dir, so it stays usable by
    // anything else that understands the models.dev artifact.
    let catalog_file = y_service::catalog_path(dir.path());
    assert_eq!(catalog_file, dir.path().join("models.dev.json"));
    let stored = std::fs::read_to_string(&catalog_file).expect("catalog written");
    assert_eq!(stored, FIXTURE);

    let catalog = y_service::load_catalog(dir.path()).expect("catalog loads");
    assert!(catalog.fetched_at.is_some());
    assert_eq!(catalog.models.len(), 2);

    // A gateway-style spelling still resolves to the canonical entry.
    let hit = y_service::resolve_model(&catalog.models, "[Kiro] Claude_Opus_4_6:free", None)
        .expect("gateway spelling resolves");
    assert_eq!(hit.provider_id, "anthropic");
    assert_eq!(hit.id, "claude-opus-4-6");
    assert_eq!(hit.context_window, Some(200_000));
    assert_eq!(hit.cost_per_1k_input, Some(0.005));

    // Partial input ranks the intended model first.
    let hits = y_service::search_models(&catalog.models, "gpt4", Some("openai"), 5);
    assert_eq!(hits[0].model.id, "gpt-4o");
    assert!(hits[0].resolved);
}

#[tokio::test]
async fn failed_download_leaves_an_existing_catalog_intact() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(y_service::catalog_path(dir.path()), FIXTURE).expect("seed catalog");

    // Port 0 is never connectable, so the fetch fails before any write.
    let error = y_service::update_catalog(dir.path(), Some("http://127.0.0.1:0/api.json"))
        .await
        .expect_err("unreachable host fails");
    assert!(error.contains("Network error"), "unexpected error: {error}");

    let stored = std::fs::read_to_string(y_service::catalog_path(dir.path())).expect("read");
    assert_eq!(stored, FIXTURE);
}
