use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use llmsim::{
    cli::{build_router, build_state, Config},
    stats::new_shared_stats,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn config(enabled: bool) -> Config {
    Config::from_toml(&format!(
        "[server]\nhost = '127.0.0.1'\ncall_log = {enabled}\n[latency]\nprofile = 'instant'"
    ))
    .unwrap()
}

fn router(enabled: bool) -> Router {
    build_router(Arc::new(
        build_state(config(enabled), new_shared_stats()).unwrap(),
    ))
}

fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1")
}

async fn send(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn responses(router: &Router, body: Value) -> StatusCode {
    send(
        router,
        request("POST", "/openai/v1/responses")
            .header("content-type", "application/json")
            .header("authorization", "Bearer unused-test-key")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .0
}

async fn calls(router: &Router) -> (StatusCode, Value) {
    send(
        router,
        request("GET", "/llmsim/calls").body(Body::empty()).unwrap(),
    )
    .await
}

#[tokio::test]
async fn records_scenario_attempts_and_reset_replays_the_failure() {
    let router = router(true);
    let turn = json!({
        "model": "gpt-5",
        "input": "Read the files [[llmsim:flaky-429 speed=0.001]]",
        "tools": [{"type": "function", "name": "read_file"}],
        "store": false
    });
    assert_eq!(
        responses(&router, turn.clone()).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(responses(&router, turn.clone()).await, StatusCode::OK);
    let (status, log) = calls(&router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(log.as_array().unwrap().len(), 2);
    assert_eq!(
        log[0],
        json!({
            "path": "/openai/v1/responses", "status": 429,
            "scenario": "flaky-429", "step": 0, "attempt": 0,
            "body": turn, "body_truncated": false
        })
    );
    assert_eq!(log[1]["attempt"], 1);
    assert_eq!(log[1]["status"], 200);
    let reset = request("POST", "/llmsim/calls/reset")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, reset).await.0, StatusCode::NO_CONTENT);
    assert_eq!(calls(&router).await.1, json!([]));
    assert_eq!(
        responses(&router, turn).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn drops_large_bodies_and_keeps_a_bounded_log() {
    let router = router(true);
    assert_eq!(
        responses(
            &router,
            json!({"model": "gpt-5", "input": "old", "store": false})
        )
        .await,
        StatusCode::OK
    );
    for _ in 0..499 {
        assert_eq!(
            responses(&router, json!({})).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let large = json!({"model": "gpt-5", "input": "x".repeat(16_384), "store": false});
    assert_eq!(responses(&router, large).await, StatusCode::OK);
    let (_, log) = calls(&router).await;
    assert_eq!(log.as_array().unwrap().len(), 500);
    assert_eq!(log[0]["body"], json!({}));
    assert_eq!(log[499]["body_truncated"], true);
    assert!(log[499]["body"].is_null());
}

#[tokio::test]
async fn control_routes_require_opt_in_and_a_local_request() {
    assert_eq!(calls(&router(false)).await.0, StatusCode::NOT_FOUND);
    let router = router(true);
    let cross_origin = request("POST", "/llmsim/calls/reset")
        .header("origin", "https://example.test")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, cross_origin).await.0, StatusCode::FORBIDDEN);
    let remote_host = Request::builder()
        .uri("/llmsim/calls")
        .header("host", "example.test")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, remote_host).await.0, StatusCode::FORBIDDEN);
    let mut nonlocal = config(true);
    nonlocal.server.host = "0.0.0.0".to_string();
    assert!(build_state(nonlocal, new_shared_stats()).is_err());
}
