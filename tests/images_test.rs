//! End-to-end tests for the simulated image-generation endpoint.
//!
//! Drives the llmsim Axum router in-process via tower::ServiceExt::oneshot so
//! we exercise the exact HTTP path real OpenAI SDK clients would hit.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

/// Build a router with instant latency so image timing collapses to zero.
fn router() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    let state = AppState::new(config, new_shared_stats());
    build_router(Arc::new(state))
}

async fn post_images(router: &axum::Router, body: Value) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/images/generations")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    // Allow up to 32 MiB: streamed b64 frames can be large.
    let bytes = to_bytes(resp.into_body(), 32 * 1024 * 1024).await.unwrap();
    (status, bytes.to_vec())
}

fn is_valid_png_b64(b64: &str) -> bool {
    // Decode just the first few bytes and check the PNG signature.
    let decoded = base64_decode(b64);
    decoded.len() > 8 && decoded[0..8] == [137, 80, 78, 71, 13, 10, 26, 10]
}

/// Minimal base64 decoder for tests (std has none).
fn base64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        if let Some(v) = val(c) {
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
    }
    out
}

#[tokio::test]
async fn non_streaming_returns_image_and_usage() {
    let router = router();
    let body = json!({
        "model": "gpt-image-1",
        "prompt": "a cat riding a bicycle",
        "size": "1024x1024",
        "quality": "low"
    });
    let (status, bytes) = post_images(&router, body).await;
    assert_eq!(status, StatusCode::OK);

    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["size"], "1024x1024");
    assert_eq!(v["quality"], "low");
    assert_eq!(v["output_format"], "png");
    assert_eq!(v["data"].as_array().unwrap().len(), 1);

    let b64 = v["data"][0]["b64_json"].as_str().unwrap();
    assert!(is_valid_png_b64(b64), "data[0].b64_json should be a PNG");

    // Usage should reflect text input + image output tokens.
    assert!(v["usage"]["input_tokens"].as_u64().unwrap() > 0);
    assert!(v["usage"]["output_tokens"].as_u64().unwrap() > 0);
    assert_eq!(v["usage"]["input_tokens_details"]["image_tokens"], 0);
}

#[tokio::test]
async fn honors_n_for_multiple_images() {
    let router = router();
    let body = json!({
        "model": "gpt-image-1",
        "prompt": "abstract art",
        "n": 3,
        "size": "512x512",
        "quality": "low"
    });
    let (status, bytes) = post_images(&router, body).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["data"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn defaults_when_params_omitted() {
    let router = router();
    let body = json!({ "prompt": "just a prompt" });
    let (status, bytes) = post_images(&router, body).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    // Default model is gpt-image-1, default size 1024x1024.
    assert_eq!(v["size"], "1024x1024");
    assert!(v["data"][0]["b64_json"].is_string());
}

#[tokio::test]
async fn streaming_emits_partials_then_completed() {
    let router = router();
    let body = json!({
        "model": "gpt-image-1",
        "prompt": "sunset over mountains",
        "size": "256x256",
        "quality": "low",
        "stream": true,
        "partial_images": 2
    });
    let (status, bytes) = post_images(&router, body).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(bytes).unwrap();

    let partials = text.matches("image_generation.partial_image").count();
    // Each partial appears in both the `event:` line and the JSON `type` field.
    assert!(partials >= 2, "expected at least 2 partial frames");
    assert!(text.contains("image_generation.completed"));
    assert!(text.contains("\"partial_image_index\":0"));
    assert!(text.contains("\"usage\""));
}

#[tokio::test]
async fn image_model_listed_in_models_endpoint() {
    let router = router();
    let req = Request::builder()
        .method("GET")
        .uri("/openai/v1/models/gpt-image-1")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["id"], "gpt-image-1");
    assert_eq!(v["owned_by"], "openai");
}
