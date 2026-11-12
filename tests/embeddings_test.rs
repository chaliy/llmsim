//! OpenAI embeddings endpoint.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router_with(strict: bool) -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.validation.strict_models = strict;
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

async fn embed(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/embeddings")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 64 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn vector(v: &Value) -> Vec<f64> {
    v["embedding"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn cos(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[tokio::test]
async fn returns_unit_vectors_in_openai_shape() {
    let app = router_with(false);
    let (status, v) = embed(
        &app,
        json!({"model": "text-embedding-3-small", "input": ["refund policy for damaged items",
               "what is the refund policy", "kubernetes pod scheduling"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["object"], "list");
    assert_eq!(v["model"], "text-embedding-3-small");
    let data = v["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);
    assert_eq!(data[2]["index"], 2);
    assert_eq!(data[0]["object"], "embedding");
    let (a, b, c) = (vector(&data[0]), vector(&data[1]), vector(&data[2]));
    assert_eq!(a.len(), 1536);
    assert!((cos(&a, &a) - 1.0).abs() < 1e-3);
    assert!(cos(&a, &b) > cos(&a, &c) + 0.2, "related texts are closer");
    let usage = &v["usage"];
    assert!(usage["prompt_tokens"].as_u64().unwrap() > 0);
    assert_eq!(usage["prompt_tokens"], usage["total_tokens"]);

    // Deterministic across requests.
    let (_, again) = embed(
        &app,
        json!({"model": "text-embedding-3-small", "input": "refund policy for damaged items"}),
    )
    .await;
    assert_eq!(vector(&again["data"][0]), a);
}

#[tokio::test]
async fn dimensions_base64_and_tokens() {
    let app = router_with(false);
    let (_, v) = embed(
        &app,
        json!({"model": "text-embedding-3-large", "input": "hi", "dimensions": 256}),
    )
    .await;
    assert_eq!(vector(&v["data"][0]).len(), 256);

    let (_, v) = embed(
        &app,
        json!({"model": "text-embedding-3-small", "input": "hi", "dimensions": 4,
               "encoding_format": "base64"}),
    )
    .await;
    // 4 f32 values = 16 bytes = 24 base64 chars.
    assert_eq!(v["data"][0]["embedding"].as_str().unwrap().len(), 24);

    let (status, v) = embed(
        &app,
        json!({"model": "text-embedding-3-small", "input": [[1, 2, 3], [4]]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["usage"]["prompt_tokens"], 4);
}

#[tokio::test]
async fn rejects_what_openai_rejects() {
    let app = router_with(false);
    let cases = [
        (
            json!({"model": "text-embedding-ada-002", "input": "x", "dimensions": 8}),
            "does not support",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": "x", "dimensions": 4096}),
            "maximum embedding dimensions",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": "x", "dimensions": 0}),
            "greater than 0",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": ""}),
            "'$.input' is invalid",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": []}),
            "'$.input' is invalid",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": "x", "encoding_format": "int8"}),
            "Supported values",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": "word ".repeat(9000)}),
            "maximum context length is 8192",
        ),
        (
            json!({"model": "text-embedding-3-small", "input": vec!["a"; 2049]}),
            "maximum is 2048",
        ),
    ];
    for (body, needle) in cases {
        let (status, v) = embed(&app, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(v["error"]["type"], "invalid_request_error");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains(needle), "{msg}");
    }
}

#[tokio::test]
async fn unknown_models_work_unless_strict() {
    let body = json!({"model": "my-embedder", "input": "x"});
    let (status, v) = embed(&router_with(false), body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(vector(&v["data"][0]).len(), 1536);
    let (status, v) = embed(&router_with(true), body).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "model_not_found");
}
