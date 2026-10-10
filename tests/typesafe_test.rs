//! End-to-end tests for the TypeSafe System One endpoints.
//!
//! Spins the llmsim Axum router up in-process and drives it the way the
//! official `typesafe-sdk` clients (and everruns' System One driver) do at
//! `{base_url}/typesafe`.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::{new_shared_stats, SharedStats};
use serde_json::{json, Value};
use tower::ServiceExt;

fn router_with(config: Config) -> (axum::Router, SharedStats) {
    let stats = new_shared_stats();
    let state = AppState::new(config, stats.clone());
    (build_router(Arc::new(state)), stats)
}

fn router() -> (axum::Router, SharedStats) {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    router_with(config)
}

async fn post_raw(router: &axum::Router, body: String) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/typesafe/v1/systemone")
        .header("content-type", "application/json")
        .header("authorization", "Bearer test-key")
        .body(Body::from(body))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn post(router: &axum::Router, body: Value) -> (StatusCode, Value) {
    post_raw(router, body.to_string()).await
}

fn ticket_request(model: &str) -> Value {
    json!({
        "state": {"document": "I was charged twice. Please fix this ASAP."},
        "model": model,
        "questions": {
            "billing": {"type": "noul", "instructions": "Is this about billing?",
                        "criteria": {"true": "Payments or invoices", "false": "Anything else"}},
            "tone": {"type": "choice", "instructions": "What is the tone?",
                     "criteria": {"angry": "Upset or hostile", "calm": null, "excited": "Eager"}},
            "urgency": {"type": "score", "instructions": "How urgent is this?",
                        "criteria": ["Can wait", "This week", "Today"]}
        }
    })
}

fn assert_distribution(probabilities: &Value, keys: &[&str]) {
    let map = probabilities.as_object().unwrap();
    assert_eq!(map.len(), keys.len());
    let mut sum = 0.0;
    for key in keys {
        let p = map[*key].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&p), "{key}: {p}");
        sum += p;
    }
    assert!((sum - 1.0).abs() <= 0.001, "sum {sum}");
}

#[tokio::test]
async fn answers_every_question_with_the_wire_shape() {
    let (router, _) = router();
    let (status, body) = post(&router, ticket_request("jev-latest")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["model"], "jev-1.13.0");
    let answers = body["answers"].as_object().unwrap();
    assert_eq!(answers.len(), 3);

    let billing = &answers["billing"];
    assert_eq!(billing["type"], "noul");
    assert!((0.0..=1.0).contains(&billing["noul"].as_f64().unwrap()));

    let tone = &answers["tone"];
    assert_eq!(tone["type"], "choice");
    assert_distribution(&tone["probabilities"], &["angry", "calm", "excited"]);
    let choice = tone["choice"].as_str().unwrap();
    assert!(["angry", "calm", "excited"].contains(&choice));
    assert!((0.0..=1.0).contains(&tone["confidence"].as_f64().unwrap()));

    let urgency = &answers["urgency"];
    assert_eq!(urgency["type"], "score");
    assert_distribution(&urgency["probabilities"], &["0", "1", "2"]);
    assert_eq!(
        urgency["legend"],
        json!({"0": "Can wait", "1": "This week", "2": "Today"})
    );
    let score = urgency["score"].as_f64().unwrap();
    assert!((0.0..=2.0).contains(&score));
    assert!((0.0..=1.0).contains(&urgency["confidence"].as_f64().unwrap()));

    assert!(body["usage"]["input_tokens"].as_u64().unwrap() > 0);
    assert_eq!(body["usage"]["output_tokens"], 60);
}

#[tokio::test]
async fn identical_requests_get_identical_answers() {
    let (router, _) = router();
    let (_, a) = post(&router, ticket_request("jev-latest")).await;
    let (_, b) = post(&router, ticket_request("jev-latest")).await;
    assert_eq!(a["answers"], b["answers"]);
}

#[tokio::test]
async fn versioned_and_unknown_models_are_echoed() {
    let (router, _) = router();
    let (_, body) = post(&router, ticket_request("jev-1.13.0")).await;
    assert_eq!(body["model"], "jev-1.13.0");
    let (_, body) = post(&router, ticket_request("jev-preview")).await;
    assert_eq!(body["model"], "jev-1.13.0");
    let (status, body) = post(&router, ticket_request("jev-2.0.0")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["model"], "jev-2.0.0");
}

#[tokio::test]
async fn string_and_array_state_are_accepted() {
    let (router, _) = router();
    for state in [json!("Help!"), json!(["line one", "line two"])] {
        let (status, body) = post(
            &router,
            json!({"state": state, "model": "jev-latest",
                   "questions": {"q": {"type": "noul", "instructions": "Urgent?"}}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

#[tokio::test]
async fn validation_errors_use_the_fastapi_shape() {
    let (router, stats) = router();
    let (status, body) = post(
        &router,
        json!({"state": "s", "model": "jev-latest",
               "questions": {"u": {"type": "score", "criteria": []}}}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let detail = body["detail"].as_array().unwrap();
    assert_eq!(detail.len(), 1);
    assert_eq!(detail[0]["type"], "too_short");
    assert_eq!(
        detail[0]["loc"],
        json!(["body", "questions", "u", "score", "criteria"])
    );
    assert!(detail[0]["msg"].is_string());

    let (status, body) = post(&router, json!({"model": "jev-latest"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let locs: Vec<_> = body["detail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["loc"].clone())
        .collect();
    assert_eq!(
        locs,
        [json!(["body", "state"]), json!(["body", "questions"])]
    );

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.total_errors, 2);
    assert_eq!(snapshot.systemone_requests, 2);
    assert_eq!(snapshot.active_requests, 0);
}

#[tokio::test]
async fn invalid_json_is_a_422() {
    let (router, _) = router();
    let (status, body) = post_raw(&router, "{not json".to_string()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["detail"][0]["type"], "json_invalid");
}

#[tokio::test]
async fn injected_rate_limit_uses_the_typesafe_error_shape() {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.errors.rate_limit_rate = 1.0;
    let (router, _) = router_with(config);

    let req = Request::builder()
        .method("POST")
        .uri("/typesafe/v1/systemone")
        .header("content-type", "application/json")
        .body(Body::from(ticket_request("jev-latest").to_string()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().contains_key("retry-after"));
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["detail"]["error_type"], "rate_limit_error");
    assert!(body["detail"]["message"].is_string());
}

#[tokio::test]
async fn stats_count_systemone_requests_and_tokens() {
    let (router, stats) = router();
    let (_, body) = post(&router, ticket_request("jev-latest")).await;
    let snapshot = stats.snapshot();
    assert_eq!(snapshot.systemone_requests, 1);
    assert_eq!(snapshot.total_requests, 1);
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(
        snapshot.prompt_tokens,
        body["usage"]["input_tokens"].as_u64().unwrap()
    );
    assert_eq!(snapshot.model_requests.get("jev-latest"), Some(&1));
}

#[tokio::test]
async fn models_lists_the_jev_aliases() {
    let (router, _) = router();
    let req = Request::builder()
        .uri("/typesafe/v1/models")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let models = body["models"].as_array().unwrap();
    let names: Vec<_> = models.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["jev-latest", "jev-preview"]);
    for model in models {
        assert!(model["description"].is_string());
        assert_eq!(model["release_date"].as_str().unwrap().len(), 10);
    }
}
