//! End-to-end tests for reasoning token accounting on the OpenAI
//! Chat Completions endpoint (`reasoning_effort`, `completion_tokens_details`).

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.response.generator = "echo".to_string();
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

async fn chat(router: &axum::Router, body: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn request(model: &str, effort: Option<&str>) -> Value {
    let mut body = json!({
        "model": model,
        "messages": [{"role": "user", "content": "the same prompt every time"}]
    });
    if let Some(effort) = effort {
        body["reasoning_effort"] = json!(effort);
    }
    body
}

fn reasoning_tokens(v: &Value) -> u64 {
    v["usage"]["completion_tokens_details"]["reasoning_tokens"]
        .as_u64()
        .unwrap()
}

#[tokio::test]
async fn reasoning_model_reports_reasoning_tokens_inside_completion_tokens() {
    let router = router();
    let v = chat(&router, request("gpt-6-sol", None)).await;
    let usage = &v["usage"];
    let reasoning = reasoning_tokens(&v);
    assert!(reasoning > 0);
    assert!(usage["completion_tokens"].as_u64().unwrap() > reasoning);
    assert_eq!(
        usage["total_tokens"].as_u64().unwrap(),
        usage["prompt_tokens"].as_u64().unwrap() + usage["completion_tokens"].as_u64().unwrap()
    );
}

#[tokio::test]
async fn reasoning_effort_scales_reasoning_tokens() {
    let router = router();
    let none = chat(&router, request("gpt-5.6", Some("none"))).await;
    let low = chat(&router, request("gpt-5.6", Some("low"))).await;
    let max = chat(&router, request("gpt-5.6", Some("max"))).await;
    assert_eq!(reasoning_tokens(&none), 0);
    assert!(reasoning_tokens(&max) > reasoning_tokens(&low));
}

#[tokio::test]
async fn non_reasoning_model_omits_completion_tokens_details() {
    let router = router();
    let v = chat(&router, request("gpt-4o", Some("high"))).await;
    assert!(v["usage"].get("completion_tokens_details").is_none());
}
