//! End-to-end tests for the Decisions endpoints: OpenAI `/v1/decisions`,
//! Gemini `/v1beta/decisions`, and Microsoft Foundry's System One route.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router_with(config: Config) -> axum::Router {
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

fn router() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    router_with(config)
}

async fn post(router: &axum::Router, uri: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}

fn openai_ticket(input: &str) -> Value {
    json!({
        "model": "gpt-6-luna",
        "input": input,
        "questions": [
            {"type": "predicate", "name": "q0", "instructions": "Is it urgent?"},
            {"type": "choice", "name": "q1", "instructions": "Which team?",
             "choices": [{"value": "billing", "description": "Payments"}, {"value": "technical"}]},
            {"type": "score", "name": "q2", "instructions": "Priority?",
             "levels": [{"label": "Routine"}, {"label": "Important"}, {"label": "Urgent"}]}
        ]
    })
}

#[tokio::test]
async fn openai_answers_in_question_order_with_the_wire_shape() {
    let app = router();
    let (status, _, body) = post(
        &app,
        "/openai/v1/decisions",
        openai_ticket("My payment failed twice today."),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model"], "gpt-6-luna");
    let answers = body["answers"].as_array().unwrap();
    assert_eq!(answers.len(), 3);

    assert_eq!(answers[0]["type"], "predicate");
    assert_eq!(answers[0]["name"], "q0");
    let p = answers[0]["probability"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&p));

    assert_eq!(answers[1]["type"], "choice");
    let probs = answers[1]["probabilities"].as_array().unwrap();
    assert_eq!(probs.len(), 2);
    let sum: f64 = probs
        .iter()
        .map(|p| p["probability"].as_f64().unwrap())
        .sum();
    assert!((sum - 1.0).abs() < 1e-6);
    let picked = answers[1]["choice"].as_str().unwrap();
    assert!(picked == "billing" || picked == "technical");

    assert_eq!(answers[2]["type"], "score");
    let score = answers[2]["score"].as_f64().unwrap();
    assert!((0.0..=2.0).contains(&score));
    assert_eq!(answers[2]["probabilities"][0]["value"], 0);
    assert!(body["usage"]["input_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn openai_answers_are_deterministic_and_default_the_model() {
    let app = router();
    let mut req = openai_ticket("same input");
    req.as_object_mut().unwrap().remove("model");
    let (_, _, a) = post(&app, "/openai/v1/decisions", req.clone()).await;
    let (_, _, b) = post(&app, "/openai/v1/decisions", req).await;
    assert_eq!(a["answers"], b["answers"]);
    assert_eq!(a["model"], "gpt-6-luna");
}

#[tokio::test]
async fn openai_refusal_marker_refuses_every_question() {
    let app = router();
    let (status, _, body) = post(
        &app,
        "/openai/v1/decisions",
        openai_ticket("help [[llmsim:refuse]]"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for answer in body["answers"].as_array().unwrap() {
        assert_eq!(answer["type"], "refusal");
        assert!(answer["name"].is_string());
    }
}

#[tokio::test]
async fn openai_invalid_requests_use_the_openai_error_envelope() {
    let app = router();
    let (status, _, body) = post(
        &app,
        "/openai/v1/decisions",
        json!({"input": "x", "questions": [{"type": "maybe", "name": "q"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(body["error"]["message"].as_str().unwrap().contains("type"));
}

#[tokio::test]
async fn injected_rate_limit_carries_retry_after_in_each_envelope() {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.errors.rate_limit_rate = 1.0;
    let app = router_with(config);
    let (status, headers, body) = post(&app, "/openai/v1/decisions", openai_ticket("x")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(headers.contains_key("retry-after"));
    let (status, _, body) = post(
        &app,
        "/gemini/v1beta/decisions",
        json!({"input": "x", "questions": {"q0": {"type": "noul"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["status"], "RESOURCE_EXHAUSTED");
}

#[tokio::test]
async fn gemini_answers_under_the_request_keys() {
    let app = router();
    let (status, _, body) = post(
        &app,
        "/gemini/v1beta/decisions",
        json!({
            "model": "gemini-decision-1",
            "input": "My payment failed twice today.",
            "questions": {
                "q0": {"type": "noul", "instructions": "Is it urgent?",
                       "criteria": {"true_description": "needs action today"}},
                "q1": {"type": "choice", "instructions": "Which team?",
                       "criteria": {"billing": "Payments", "technical": null}},
                "q2": {"type": "score", "instructions": "Priority?",
                       "criteria": ["Routine", "Important", "Urgent"]}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("model").is_none());
    let answers = body["answers"].as_object().unwrap();
    assert_eq!(answers.len(), 3);
    assert_eq!(answers["q0"]["type"], "noul");
    assert!(answers["q0"]["confidence"].is_number());
    assert_eq!(answers["q1"]["type"], "choice");
    assert!(answers["q1"]["probabilities"]["billing"].is_number());
    assert_eq!(answers["q2"]["legend"]["2"], "Urgent");
    assert_eq!(body["usage"]["total_output_tokens"], 0);
    assert!(body["usage"]["total_input_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn gemini_rejects_bad_keys_with_a_google_error() {
    let app = router();
    let (status, _, body) = post(
        &app,
        "/gemini/v1beta/decisions",
        json!({"input": "x", "questions": {"is urgent?": {"type": "noul"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], 400);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
}

#[tokio::test]
async fn foundry_serves_system_one_at_the_resource_root() {
    let app = router();
    let (status, _, body) = post(
        &app,
        "/foundry/providers/microsoft/v1/systemone",
        json!({
            "model": "Microsoft-Decision-1",
            "state": "My payment failed twice today.",
            "questions": {"urgent": {"type": "noul", "instructions": "Is it urgent?"}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["model"], "Microsoft-Decision-1");
    assert_eq!(body["answers"]["urgent"]["type"], "noul");
}
