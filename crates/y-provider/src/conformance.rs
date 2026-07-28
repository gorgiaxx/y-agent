//! Reusable fake-HTTP-provider harness for adapter contract tests.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use y_core::provider::{
    ChatRequest, LlmProvider, ProviderError, RequestMode, ToolCallingMode, ToolDialect,
};
use y_core::types::{Message, Role};

struct FakeHttpProvider;

impl FakeHttpProvider {
    async fn serve_once(
        status: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake provider");
        let address = listener.local_addr().expect("fake provider address");
        let status = status.to_string();
        let headers = headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<Vec<_>>();
        let body = body.to_string();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept fake request");
            let mut request = vec![0; 32 * 1024];
            let read = socket.read(&mut request).await.expect("read fake request");
            request.truncate(read);
            let mut response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(&body);
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write fake response");
            request
        });
        (format!("http://{address}"), task)
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: "hello".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        }],
        model: None,
        request_mode: RequestMode::TextChat,
        max_tokens: Some(16),
        temperature: None,
        top_p: None,
        tools: Vec::new(),
        tool_calling_mode: ToolCallingMode::Native,
        tool_dialect: ToolDialect::OpenAi,
        stop: Vec::new(),
        extra: serde_json::Value::Null,
        thinking: None,
        response_format: None,
        image_generation_options: None,
    }
}

async fn assert_rate_limited(provider: &dyn LlmProvider) {
    let error = provider
        .chat_completion(&request())
        .await
        .expect_err("429 fixture must fail");
    assert!(
        matches!(
            error,
            ProviderError::RateLimited {
                retry_after_secs: 7,
                ..
            }
        ),
        "adapter returned the wrong normalized error: {error}"
    );
}

#[tokio::test]
async fn ollama_adapter_honors_retry_after_in_shared_http_fixture() {
    let (base_url, server) =
        FakeHttpProvider::serve_once("429 Too Many Requests", &[("Retry-After", "7")], "busy")
            .await;
    let provider = crate::providers::ollama::OllamaProvider::new(
        "ollama",
        "qwen3",
        String::new(),
        Some(base_url),
        None,
        Vec::new(),
        Vec::new(),
        1,
        32_768,
        ToolCallingMode::Native,
        ToolDialect::OpenAi,
    );

    assert_rate_limited(&provider).await;
    let captured = server.await.expect("fake provider task");
    assert!(String::from_utf8_lossy(&captured).starts_with("POST /api/chat"));
}

#[tokio::test]
async fn hosted_adapters_share_rate_limit_and_retry_after_contract() {
    let (base_url, openai_server) =
        FakeHttpProvider::serve_once("429 Too Many Requests", &[("Retry-After", "7")], "busy")
            .await;
    let openai = crate::providers::openai::OpenAiProvider::new(
        "openai",
        "gpt-4o",
        "test".into(),
        Some(base_url),
        None,
        Vec::new(),
        Vec::new(),
        1,
        128_000,
        ToolCallingMode::Native,
        ToolDialect::OpenAi,
    )
    .with_use_responses_api(false);
    assert_rate_limited(&openai).await;
    assert!(!openai_server.await.expect("OpenAI fixture").is_empty());

    let (base_url, anthropic_server) =
        FakeHttpProvider::serve_once("429 Too Many Requests", &[("Retry-After", "7")], "busy")
            .await;
    let anthropic = crate::providers::anthropic::AnthropicProvider::new(
        "anthropic",
        "claude-sonnet-4",
        "test".into(),
        Some(base_url),
        None,
        Vec::new(),
        Vec::new(),
        1,
        200_000,
        ToolCallingMode::Native,
        ToolDialect::Anthropic,
    );
    assert_rate_limited(&anthropic).await;
    assert!(!anthropic_server
        .await
        .expect("Anthropic fixture")
        .is_empty());

    let (base_url, gemini_server) =
        FakeHttpProvider::serve_once("429 Too Many Requests", &[("Retry-After", "7")], "busy")
            .await;
    let gemini = crate::providers::gemini::GeminiProvider::new(
        "gemini",
        "gemini-2.5-pro",
        "test".into(),
        Some(base_url),
        None,
        Vec::new(),
        Vec::new(),
        1,
        1_000_000,
        ToolCallingMode::Native,
        ToolDialect::Gemini,
    );
    assert_rate_limited(&gemini).await;
    assert!(!gemini_server.await.expect("Gemini fixture").is_empty());

    let (base_url, azure_server) =
        FakeHttpProvider::serve_once("429 Too Many Requests", &[("Retry-After", "7")], "busy")
            .await;
    let azure = crate::providers::azure::AzureOpenAiProvider::new(
        "azure",
        "gpt-4o",
        "test".into(),
        Some(base_url),
        None,
        Vec::new(),
        Vec::new(),
        1,
        128_000,
        ToolCallingMode::Native,
        ToolDialect::OpenAi,
    );
    assert_rate_limited(&azure).await;
    assert!(!azure_server.await.expect("Azure fixture").is_empty());
}
