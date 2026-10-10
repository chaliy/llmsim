//! End-to-end tests for scenario mode on the chat completions endpoint.
//!
//! Drives the in-process Axum router with tower::ServiceExt::oneshot, the
//! same way scripted_test.rs does.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::scenario::ScenarioSet;
use llmsim::script::{Script, SimTurn};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

const SCENARIOS: &str = r#"[
  {
    "name": "research",
    "steps": [
      { "turn": { "type": "mixed", "text": "Let me look.",
                  "calls": [{ "tool": "any_read" }] } },
      { "fail_first": [{ "kind": "rate_limit", "retry_after_ms": 1500 }],
        "turn": { "type": "tool_calls",
                  "calls": [{ "name": "bash", "arguments": { "command": "echo 2" } }] } },
      { "turn": { "type": "assistant", "lorem_tokens": 30, "reasoning_tokens": 10 } }
    ],
    "on_exhausted": "error"
  },
  {
    "name": "broken",
    "defaults": { "timing": { "ttft_ms": 0, "tokens_per_sec": 100000 } },
    "steps": [
      { "timing": { "cut_after_tokens": 5 },
        "turn": { "type": "assistant", "lorem_tokens": 20 } }
    ]
  },
  {
    "name": "chat",
    "steps": [{ "turn": { "type": "assistant", "text": "hello from chat" } }]
  }
]"#;

fn state() -> AppState {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    AppState::new(config, new_shared_stats())
        .with_scenarios(Arc::new(ScenarioSet::from_json(SCENARIOS).unwrap()))
}

fn router() -> axum::Router {
    build_router(Arc::new(state()))
}

fn tools() -> Value {
    json!([
        {"type": "function", "function": {"name": "read_file", "parameters": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        }}},
        {"type": "function", "function": {"name": "bash"}}
    ])
}

async fn post(
    router: &axum::Router,
    body: Value,
) -> (StatusCode, HeaderMap, Result<String, String>) {
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .map(|b| String::from_utf8(b.to_vec()).unwrap())
        .map_err(|e| e.to_string());
    (status, headers, body)
}

fn assistant_message(resp: &Value) -> Value {
    resp["choices"][0]["message"].clone()
}

#[tokio::test]
async fn plays_a_scenario_across_a_conversation() {
    let router = router();
    let mut messages = vec![
        json!({"role": "system", "content": "You are an agent."}),
        json!({"role": "user", "content": "Summarise the issues. [[llmsim:research]]"}),
    ];

    // Step 0: text plus a placeholder read call with schema-filled args.
    let (status, headers, body) = post(
        &router,
        json!({"model": "gpt-5", "messages": messages, "tools": tools()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-llmsim-scenario"], "research");
    assert_eq!(headers["x-llmsim-step"], "0");
    let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    let msg = assistant_message(&v);
    assert_eq!(msg["content"], "Let me look.");
    let call = &msg["tool_calls"][0];
    assert_eq!(call["function"]["name"], "read_file");
    assert_eq!(call["function"]["arguments"], "{\"path\":\".\"}");
    let call_id = call["id"].as_str().unwrap().to_string();
    assert!(
        call_id.starts_with("call_llmsim_research_0_0_"),
        "{call_id}"
    );
    messages.push(msg);
    messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": "README"}));

    // Step 1: 429 with Retry-After first, then the bash call on retry.
    let req = json!({"model": "gpt-5", "messages": messages, "tools": tools()});
    let (status, headers, body) = post(&router, req.clone()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers["retry-after"], "2");
    assert_eq!(headers["retry-after-ms"], "1500");
    assert_eq!(headers["x-llmsim-attempt"], "0");
    assert!(body.unwrap().contains("rate_limit_error"));

    let (status, headers, body) = post(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-llmsim-step"], "1");
    assert_eq!(headers["x-llmsim-attempt"], "1");
    let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
    let msg = assistant_message(&v);
    assert_eq!(msg["tool_calls"][0]["function"]["name"], "bash");
    assert_eq!(
        msg["tool_calls"][0]["function"]["arguments"],
        "{\"command\":\"echo 2\"}"
    );
    let call_id = msg["tool_calls"][0]["id"].as_str().unwrap().to_string();
    messages.push(msg);
    messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": "2"}));

    // Step 2: a lorem answer with reasoning billed in usage.
    let (status, _, body) = post(
        &router,
        json!({"model": "gpt-5", "messages": messages, "tools": tools()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
    let text = v["choices"][0]["message"]["content"].as_str().unwrap();
    assert_eq!(text.split_whitespace().count(), 30);
    assert_eq!(
        v["usage"]["completion_tokens_details"]["reasoning_tokens"],
        10
    );
    messages.push(assistant_message(&v));

    // Past the end with on_exhausted=error.
    let (status, _, body) = post(
        &router,
        json!({"model": "gpt-5", "messages": messages, "tools": tools()}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.unwrap().contains("exhausted"));
}

#[tokio::test]
async fn streams_reasoning_and_tool_calls() {
    let router = router();
    let messages = json!([
        {"role": "user", "content": "go [[llmsim:research]]"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_llmsim_research_0_0_aa", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}]},
        {"role": "tool", "tool_call_id": "call_llmsim_research_0_0_aa", "content": "x"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_llmsim_research_1_0_aa", "type": "function", "function": {"name": "bash", "arguments": "{}"}}]},
        {"role": "tool", "tool_call_id": "call_llmsim_research_1_0_aa", "content": "x"}
    ]);
    let (status, headers, body) = post(
        &router,
        json!({"model": "gpt-5", "stream": true, "messages": messages, "tools": tools()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/event-stream");
    let body = body.unwrap();
    let reasoning = body.find("reasoning_content").unwrap();
    let content = body.find("\"content\":").unwrap();
    assert!(reasoning < content);
    assert!(body.contains("\"finish_reason\":\"stop\""));
    assert!(body.ends_with("data: [DONE]\n\n"));
}

#[tokio::test]
async fn unknown_scenario_fails_loudly() {
    let (status, _, body) = post(
        &router(),
        json!({"model": "gpt-5", "messages": [{"role": "user", "content": "x [[llmsim:reserch]]"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let body = body.unwrap();
    assert!(body.contains("reserch"), "{body}");
    assert!(body.contains("broken, chat, research"), "{body}");
}

#[tokio::test]
async fn exact_tool_name_not_offered_is_a_400() {
    let messages = json!([
        {"role": "user", "content": "go [[llmsim:research]]"},
        {"role": "assistant", "content": "Let me look."}
    ]);
    let only_read = json!([{"type": "function", "function": {"name": "read_file"}}]);
    let (status, _, body) = post(
        &router(),
        json!({"model": "gpt-5", "messages": messages, "tools": only_read}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.unwrap().contains("'bash'"));
}

#[tokio::test]
async fn broken_stream_is_cut_then_retry_completes() {
    let router = router();
    let req = json!({
        "model": "gpt-5",
        "stream": true,
        "messages": [{"role": "user", "content": "talk [[llmsim:broken]]"}]
    });
    let (status, _, body) = post(&router, req.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_err(), "first attempt must drop the stream");

    let (status, _, body) = post(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    let body = body.unwrap();
    assert!(body.contains("\"finish_reason\":\"stop\""));
    assert!(body.ends_with("data: [DONE]\n\n"));
}

#[tokio::test]
async fn non_streaming_cut_drops_the_body() {
    let router = router();
    let req = json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "talk [[llmsim:broken seed=4]]"}]
    });
    let (_, _, body) = post(&router, req.clone()).await;
    assert!(body.is_err());
    let (status, _, body) = post(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_ok());
}

#[tokio::test]
async fn model_name_selects_scenario() {
    let (status, _, body) = post(
        &router(),
        json!({"model": "llmsim-scenario-chat", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "hello from chat");
}

#[tokio::test]
async fn requests_without_marker_keep_script_behaviour() {
    let state = state().with_script(Arc::new(Script::new(vec![SimTurn::Assistant {
        text: "scripted".into(),
    }])));
    let router = build_router(Arc::new(state));
    let (status, headers, body) = post(
        &router,
        json!({"model": "gpt-5", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("x-llmsim-scenario").is_none());
    let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "scripted");
}

#[tokio::test]
async fn concurrent_interleaved_sessions_get_their_own_steps() {
    // 500 sessions run the research scenario concurrently against one
    // server; each must see step 0, then (after its 429) step 1, then step 2.
    let router = router();
    let mut handles = Vec::new();
    for i in 0..500 {
        let router = router.clone();
        handles.push(tokio::spawn(async move {
            let mut messages = vec![json!({
                "role": "user",
                "content": format!("session {i} [[llmsim:research]]")
            })];
            let mut steps = Vec::new();
            while steps.len() < 3 {
                let (status, headers, body) = post(
                    &router,
                    json!({"model": "gpt-5", "messages": messages, "tools": tools()}),
                )
                .await;
                if status == StatusCode::TOO_MANY_REQUESTS {
                    tokio::task::yield_now().await;
                    continue;
                }
                assert_eq!(status, StatusCode::OK);
                steps.push(headers["x-llmsim-step"].to_str().unwrap().to_string());
                let v: Value = serde_json::from_str(&body.unwrap()).unwrap();
                let msg = assistant_message(&v);
                let ids: Vec<String> = msg["tool_calls"]
                    .as_array()
                    .map(|calls| {
                        calls
                            .iter()
                            .map(|c| c["id"].as_str().unwrap().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                messages.push(msg);
                for id in ids {
                    messages.push(json!({"role": "tool", "tool_call_id": id, "content": "ok"}));
                }
            }
            steps
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap(), vec!["0", "1", "2"]);
    }
}

#[test]
fn starter_library_loads() {
    let set =
        ScenarioSet::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/scenarios")).unwrap();
    assert_eq!(
        set.names(),
        vec![
            "broken-stream",
            "chat-short",
            "flaky-429",
            "long-answer",
            "parallel-tools",
            "research-3-tools",
            "slow-first-token",
            "stream-stall",
            "thinking",
        ]
    );
}
