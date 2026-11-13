//! End-to-end tests for provider-style rate limits, overload bursts and
//! request-limit validation.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn config() -> Config {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config
}

fn router(config: Config) -> axum::Router {
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

async fn post(
    app: &axum::Router,
    uri: &str,
    key: &str,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {key}"))
        .header("x-api-key", key)
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 16 * 1024 * 1024).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}

fn chat(model: &str) -> Value {
    json!({"model": model, "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]})
}

fn messages(model: &str, max_tokens: u32) -> Value {
    json!({"model": model, "max_tokens": max_tokens, "messages": [{"role": "user", "content": "hi"}]})
}

#[tokio::test]
async fn openai_rate_limit_headers_then_429() {
    let mut c = config();
    c.rate_limits.requests_per_minute = 2;
    c.rate_limits.tokens_per_minute = 100_000;
    let app = router(c);
    let (status, h, _) = post(&app, "/openai/v1/chat/completions", "k1", chat("gpt-5")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h["x-ratelimit-limit-requests"], "2");
    assert_eq!(h["x-ratelimit-remaining-requests"], "1");
    assert_eq!(h["x-ratelimit-limit-tokens"], "100000");
    assert!(h.contains_key("x-ratelimit-reset-tokens"));
    post(&app, "/openai/v1/chat/completions", "k1", chat("gpt-5")).await;
    let (status, h, body) = post(&app, "/openai/v1/chat/completions", "k1", chat("gpt-5")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limit_exceeded");
    assert_eq!(body["error"]["type"], "requests");
    assert_eq!(h["x-ratelimit-remaining-requests"], "0");
    let retry: u64 = h["retry-after"].to_str().unwrap().parse().unwrap();
    assert!((1..=30).contains(&retry), "{retry}");
    // A different key has its own budget.
    let (status, _, _) = post(&app, "/openai/v1/chat/completions", "k2", chat("gpt-5")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn anthropic_gets_its_own_headers_and_envelope() {
    let mut c = config();
    c.rate_limits.tokens_per_minute = 1_100;
    let app = router(c);
    let (status, h, _) = post(
        &app,
        "/anthropic/v1/messages",
        "k",
        messages("claude-sonnet-4-5", 1_000),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h["anthropic-ratelimit-tokens-limit"], "1100");
    assert!(h["anthropic-ratelimit-tokens-reset"]
        .to_str()
        .unwrap()
        .ends_with('Z'));
    assert!(!h.contains_key("x-ratelimit-limit-tokens"));
    let (status, _, body) = post(
        &app,
        "/anthropic/v1/messages",
        "k",
        messages("claude-sonnet-4-5", 1_000),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "rate_limit_error");
}

#[tokio::test]
async fn no_limits_means_no_headers() {
    let app = router(config());
    let (_, h, _) = post(&app, "/openai/v1/chat/completions", "k", chat("gpt-5")).await;
    assert!(!h.contains_key("x-ratelimit-limit-requests"));
}

#[tokio::test]
async fn overload_bursts_fail_every_request_in_the_window() {
    let mut c = config();
    c.errors.overload_burst_rate = 1.0;
    c.errors.overload_burst_ms = 60_000;
    let app = router(c);
    let (status, h, body) = post(
        &app,
        "/anthropic/v1/messages",
        "k",
        messages("claude-sonnet-4-5", 10),
    )
    .await;
    assert_eq!(status.as_u16(), 529);
    assert_eq!(body["error"]["type"], "overloaded_error");
    assert!(h.contains_key("retry-after"));
    let (status, _, _) = post(&app, "/openai/v1/chat/completions", "k", chat("gpt-5")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    // Model listings are never affected.
    let req = Request::builder()
        .uri("/openai/v1/models")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn chat_over_the_context_window_is_rejected() {
    let app = router(config());
    let long = "word ".repeat(9_000);
    let (status, _, body) = post(
        &app,
        "/openai/v1/chat/completions",
        "k",
        json!({"model": "gpt-4", "messages": [{"role": "user", "content": long}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "context_length_exceeded");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("maximum context length is 8192"));
}

#[tokio::test]
async fn output_caps_over_the_model_limit_are_rejected() {
    let app = router(config());
    let (status, _, body) = post(
        &app,
        "/openai/v1/chat/completions",
        "k",
        json!({"model": "gpt-4", "max_completion_tokens": 50_000,
               "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["param"], "max_completion_tokens");
    let (status, _, body) = post(
        &app,
        "/anthropic/v1/messages",
        "k",
        messages("claude-3-haiku", 100_000),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("max_tokens: 100000 >"));
}

#[tokio::test]
async fn unknown_models_pass_unless_strict() {
    let app = router(config());
    let (status, _, _) = post(&app, "/openai/v1/chat/completions", "k", chat("my-model")).await;
    assert_eq!(status, StatusCode::OK);
    let mut c = config();
    c.validation.strict_models = true;
    let app = router(c);
    let (status, _, body) = post(&app, "/openai/v1/chat/completions", "k", chat("my-model")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "model_not_found");
    let (status, _, body) = post(
        &app,
        "/anthropic/v1/messages",
        "k",
        messages("claude-nope", 10),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["type"], "not_found_error");
}

#[tokio::test]
async fn limits_can_be_turned_off() {
    let mut c = config();
    c.validation.enforce_limits = false;
    let app = router(c);
    let (status, _, _) = post(
        &app,
        "/openai/v1/chat/completions",
        "k",
        json!({"model": "gpt-4", "max_completion_tokens": 50_000,
               "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
