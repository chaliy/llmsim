//! End-to-end tests for scripted-mode responses.
//!
//! Spins the llmsim Axum router up in-process (no real socket) and
//! drives it via tower::ServiceExt::oneshot so we exercise the exact
//! HTTP path real clients would hit.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::script::{OnExhausted, Script, ScriptSpec, SimError, SimToolCall, SimTurn};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router_with_script(script: Script) -> axum::Router {
    let mut config = Config::default();
    // Skip the model-derived latency by setting a fast profile.
    config.latency.profile = Some("instant".to_string());
    let mut state = AppState::new(config, new_shared_stats());
    state = state.with_script(Arc::new(script));
    build_router(Arc::new(state))
}

async fn post_chat_completions(router: &axum::Router, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn post_responses(router: &axum::Router, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn chat_completions_replays_turns_in_order() {
    let script = Script::from_spec(ScriptSpec {
        turns: vec![
            SimTurn::Assistant {
                text: "first".into(),
            },
            SimTurn::Assistant {
                text: "second".into(),
            },
        ],
        on_exhausted: OnExhausted::RepeatLast,
    })
    .unwrap();
    let router = router_with_script(script);

    let body = json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "hi"}]
    });

    let (s1, t1) = post_chat_completions(&router, body.clone()).await;
    let (s2, t2) = post_chat_completions(&router, body.clone()).await;
    let (s3, t3) = post_chat_completions(&router, body).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(s3, StatusCode::OK);

    let v1: Value = serde_json::from_str(&t1).unwrap();
    let v2: Value = serde_json::from_str(&t2).unwrap();
    let v3: Value = serde_json::from_str(&t3).unwrap();
    assert_eq!(v1["choices"][0]["message"]["content"], "first");
    assert_eq!(v2["choices"][0]["message"]["content"], "second");
    // RepeatLast (default).
    assert_eq!(v3["choices"][0]["message"]["content"], "second");
}

#[tokio::test]
async fn chat_completions_returns_tool_calls() {
    let script = Script::new(vec![SimTurn::ToolCalls {
        calls: vec![SimToolCall {
            name: "bash".into(),
            arguments: json!({"command": "ls /tmp"}),
            id: Some("call_test".into()),
        }],
    }]);
    let router = router_with_script(script);

    let (status, body) = post_chat_completions(
        &router,
        json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "list files"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    assert!(v["choices"][0]["message"]["content"].is_null());
    let tool_calls = &v["choices"][0]["message"]["tool_calls"];
    assert_eq!(tool_calls[0]["id"], "call_test");
    assert_eq!(tool_calls[0]["function"]["name"], "bash");
    // arguments is a JSON-encoded string.
    let args: Value =
        serde_json::from_str(tool_calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args["command"], "ls /tmp");
}

#[tokio::test]
async fn chat_completions_mixed_text_and_tool_calls() {
    let script = Script::new(vec![SimTurn::Mixed {
        text: "running ls".into(),
        calls: vec![SimToolCall {
            name: "bash".into(),
            arguments: json!({"command": "ls"}),
            id: None,
        }],
    }]);
    let router = router_with_script(script);

    let (status, body) = post_chat_completions(
        &router,
        json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(v["choices"][0]["message"]["content"], "running ls");
    // Auto-generated id.
    assert_eq!(
        v["choices"][0]["message"]["tool_calls"][0]["id"],
        "call_llmsim_0_0"
    );
}

#[tokio::test]
async fn chat_completions_error_turn() {
    let script = Script::new(vec![SimTurn::Error(SimError::RateLimit)]);
    let router = router_with_script(script);

    let (status, body) = post_chat_completions(
        &router,
        json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], "rate_limit_error");
}

#[tokio::test]
async fn chat_completions_invalid_request_error_turn() {
    let script = Script::new(vec![SimTurn::Error(SimError::InvalidRequest {
        message: "bad tool args".into(),
    })]);
    let router = router_with_script(script);

    let (status, body) = post_chat_completions(
        &router,
        json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], "invalid_request_error");
    assert_eq!(v["error"]["message"], "bad tool args");
}

#[tokio::test]
async fn chat_completions_on_exhausted_error() {
    let script = Script::new(vec![SimTurn::Assistant {
        text: "only".into(),
    }])
    .with_on_exhausted(OnExhausted::Error);
    let router = router_with_script(script);
    let body = json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]});

    let (s1, _) = post_chat_completions(&router, body.clone()).await;
    assert_eq!(s1, StatusCode::OK);
    let (s2, _) = post_chat_completions(&router, body).await;
    assert_eq!(s2, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn chat_completions_on_exhausted_loop() {
    let script = Script::new(vec![
        SimTurn::Assistant { text: "a".into() },
        SimTurn::Assistant { text: "b".into() },
    ])
    .with_on_exhausted(OnExhausted::Loop);
    let router = router_with_script(script);
    let body = json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]});

    let r = async |b| {
        let (_, t) = post_chat_completions(&router, b).await;
        let v: Value = serde_json::from_str(&t).unwrap();
        v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(r(body.clone()).await, "a");
    assert_eq!(r(body.clone()).await, "b");
    assert_eq!(r(body.clone()).await, "a");
    assert_eq!(r(body).await, "b");
}

#[tokio::test]
async fn chat_completions_streaming_emits_tool_call_deltas() {
    let script = Script::new(vec![SimTurn::ToolCalls {
        calls: vec![SimToolCall {
            name: "write_file".into(),
            arguments: json!({"path": "x.txt", "content": "hi"}),
            id: Some("call_w".into()),
        }],
    }]);
    let router = router_with_script(script);

    let (status, body) = post_chat_completions(
        &router,
        json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x"}], "stream": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // SSE body: contains tool_calls deltas, finish_reason, and [DONE].
    assert!(body.contains("\"finish_reason\":\"tool_calls\""));
    assert!(body.contains("\"name\":\"write_file\""));
    assert!(body.contains("\"id\":\"call_w\""));
    assert!(body.contains("\\\"path\\\""));
    assert!(body.contains("[DONE]"));
}

#[tokio::test]
async fn responses_api_returns_function_call_output_items() {
    let script = Script::new(vec![SimTurn::Mixed {
        text: "calling bash".into(),
        calls: vec![SimToolCall {
            name: "bash".into(),
            arguments: json!({"cmd": "echo hi"}),
            id: None,
        }],
    }]);
    let router = router_with_script(script);

    let (status, body) =
        post_responses(&router, json!({"model": "gpt-5", "input": "do thing"})).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    let output = v["output"].as_array().unwrap();
    assert_eq!(output.len(), 2);
    assert_eq!(output[0]["type"], "message");
    assert_eq!(output[1]["type"], "function_call");
    assert_eq!(output[1]["name"], "bash");
    assert_eq!(output[1]["call_id"], "call_llmsim_0_0");
}

#[tokio::test]
async fn responses_api_error_turn() {
    let script = Script::new(vec![SimTurn::Error(SimError::RateLimit)]);
    let router = router_with_script(script);
    let (status, body) = post_responses(&router, json!({"model": "gpt-5", "input": "hi"})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], "rate_limit_error");
}

#[tokio::test]
async fn script_loads_from_file_via_config() {
    let dir = tempfile_dir();
    let path = dir.join("script.json");
    std::fs::write(
        &path,
        r#"{
            "turns": [
                {"type": "assistant", "text": "from disk"}
            ]
        }"#,
    )
    .unwrap();

    let script = Script::from_file(&path).unwrap();
    assert_eq!(script.len(), 1);

    // Also check the round-trip via the public API.
    let mut config = Config::default();
    config.response.script_path = Some(path.to_string_lossy().into_owned());
    assert_eq!(
        config.response.script_path.as_deref(),
        Some(path.to_string_lossy().as_ref())
    );
}

/// Create a unique tempdir under the system temp root. We don't pull
/// in the `tempfile` crate just for one path — a nanosecond-suffixed
/// directory is plenty for tests.
fn tempfile_dir() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("llmsim-scripted-{}", ns));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
