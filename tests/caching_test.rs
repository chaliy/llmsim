//! Prompt-cache usage and thinking realism.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router(caching: bool) -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.response.prompt_caching = caching;
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("x-api-key", "k")
        .header("anthropic-version", "2023-06-01")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 16 * 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

/// A system prompt well over the 1024-token cache minimum.
fn long_system() -> String {
    "You are a careful assistant that follows the house rules. ".repeat(300)
}

fn chat_body(extra_user: Option<&str>) -> Value {
    let mut messages = vec![
        json!({"role": "system", "content": long_system()}),
        json!({"role": "user", "content": "first question"}),
    ];
    if let Some(u) = extra_user {
        messages.push(json!({"role": "assistant", "content": "first answer"}));
        messages.push(json!({"role": "user", "content": u}));
    }
    json!({"model": "gpt-5", "messages": messages})
}

#[tokio::test]
async fn chat_repeated_prefix_reports_cached_tokens() {
    let app = router(true);
    let (status, v) = post(&app, "/openai/v1/chat/completions", chat_body(None)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["usage"]["prompt_tokens_details"]["cached_tokens"], 0);

    let (_, v) = post(&app, "/openai/v1/chat/completions", chat_body(Some("next"))).await;
    let cached = v["usage"]["prompt_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap();
    let prompt = v["usage"]["prompt_tokens"].as_u64().unwrap();
    assert!(cached >= 1024, "cached {cached}");
    assert!(cached < prompt);
    assert_eq!((cached - 1024) % 128, 0);
}

#[tokio::test]
async fn chat_short_prompts_and_disabled_cache_report_nothing() {
    let app = router(true);
    let short = json!({"model": "gpt-5", "messages": [{"role": "user", "content": "hi"}]});
    post(&app, "/openai/v1/chat/completions", short.clone()).await;
    let (_, v) = post(&app, "/openai/v1/chat/completions", short).await;
    let cached = &v["usage"]["prompt_tokens_details"]["cached_tokens"];
    assert!(cached.is_null() || *cached == 0, "{v}");

    let off = router(false);
    post(&off, "/openai/v1/chat/completions", chat_body(None)).await;
    let (_, v) = post(&off, "/openai/v1/chat/completions", chat_body(None)).await;
    let cached = &v["usage"]["prompt_tokens_details"]["cached_tokens"];
    assert!(cached.is_null() || *cached == 0, "{v}");
}

#[tokio::test]
async fn responses_repeated_prompt_reports_cached_tokens() {
    let app = router(true);
    let body = json!({"model": "gpt-5", "instructions": long_system(), "input": "hello"});
    post(&app, "/openai/v1/responses", body.clone()).await;
    let (status, v) = post(&app, "/openai/v1/responses", body).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let cached = v["usage"]["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap();
    assert!(cached >= 1024, "{v}");
}

fn anthropic_body() -> Value {
    json!({
        "model": "claude-opus-4-8",
        "max_tokens": 256,
        "system": [{"type": "text", "text": long_system(), "cache_control": {"type": "ephemeral"}}],
        "messages": [{"role": "user", "content": "hello"}]
    })
}

#[tokio::test]
async fn anthropic_cache_control_creates_then_reads() {
    let app = router(true);
    let (status, first) = post(&app, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let created = first["usage"]["cache_creation_input_tokens"]
        .as_u64()
        .unwrap();
    assert!(created >= 1024, "{first}");
    assert_eq!(first["usage"]["cache_read_input_tokens"], 0);

    let (_, second) = post(&app, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(
        second["usage"]["cache_read_input_tokens"].as_u64(),
        Some(created)
    );
    assert_eq!(second["usage"]["cache_creation_input_tokens"], 0);
    // Cached tokens are not billed again as input.
    assert!(second["usage"]["input_tokens"].as_u64().unwrap() < created);
}

#[tokio::test]
async fn anthropic_without_cache_control_caches_nothing() {
    let app = router(true);
    let body = json!({
        "model": "claude-opus-4-8",
        "max_tokens": 256,
        "system": long_system(),
        "messages": [{"role": "user", "content": "hello"}]
    });
    post(&app, "/anthropic/v1/messages", body.clone()).await;
    let (_, v) = post(&app, "/anthropic/v1/messages", body).await;
    let read = &v["usage"]["cache_read_input_tokens"];
    assert!(read.is_null() || *read == 0, "{v}");
}

#[tokio::test]
async fn redacted_thinking_marker_redacts_the_block() {
    let app = router(true);
    let (status, v) = post(
        &app,
        "/anthropic/v1/messages",
        json!({
            "model": "claude-opus-5-5",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "plan it [[llmsim:redacted-thinking]]"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let content = v["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "redacted_thinking");
    assert!(!content[0]["data"].as_str().unwrap().is_empty());
    assert!(content[0].get("thinking").is_none());
    assert_eq!(content.last().unwrap()["type"], "text");
}

#[tokio::test]
async fn redacted_thinking_streams_as_one_block() {
    let app = router(true);
    let req = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "claude-opus-5-5",
                "max_tokens": 1024,
                "stream": true,
                "messages": [{"role": "user", "content": "go [[llmsim:redacted-thinking]]"}]
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), 16 * 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("\"type\":\"redacted_thinking\""), "{text}");
    assert!(!text.contains("thinking_delta"), "{text}");
}
