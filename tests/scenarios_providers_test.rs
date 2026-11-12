//! Scenario mode on the Responses API and Anthropic Messages.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::scenario::ScenarioSet;
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

const SCENARIOS: &str = r#"[
  {
    "name": "research",
    "steps": [
      { "turn": { "type": "mixed", "text": "Let me look.",
                  "calls": [{ "tool": "any_read" }, { "name": "bash", "arguments": { "command": "ls" } }] } },
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
  }
]"#;

fn router() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    let state = AppState::new(config, new_shared_stats())
        .with_scenarios(Arc::new(ScenarioSet::from_json(SCENARIOS).unwrap()));
    build_router(Arc::new(state))
}

async fn post(
    router: &axum::Router,
    uri: &str,
    body: &Value,
) -> (StatusCode, HeaderMap, Result<String, String>) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
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

fn json_body(body: Result<String, String>) -> Value {
    serde_json::from_str(&body.unwrap()).unwrap()
}

// --- Responses API ---

fn responses_tools() -> Value {
    json!([
        {"type": "function", "name": "read_file", "parameters": {
            "type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]
        }},
        {"type": "function", "name": "bash"}
    ])
}

#[tokio::test]
async fn responses_plays_a_scenario_across_a_conversation() {
    let r = router();
    let uri = "/openai/v1/responses";
    let mut input = vec![json!({"role": "user", "content": "Look around [[llmsim:research]]"})];

    // Step 0: text plus two calls.
    let req = json!({"model": "gpt-5", "input": input, "tools": responses_tools()});
    let (status, headers, body) = post(&r, uri, &req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-llmsim-scenario"], "research");
    assert_eq!(headers["x-llmsim-step"], "0");
    let v = json_body(body);
    let output = v["output"].as_array().unwrap();
    assert_eq!(output[0]["type"], "message");
    assert_eq!(v["output_text"], "Let me look.");
    assert_eq!(output[1]["type"], "function_call");
    assert_eq!(output[1]["name"], "read_file");
    assert_eq!(output[1]["arguments"], "{\"path\":\".\"}");
    assert_eq!(output[2]["name"], "bash");
    // History goes back as message + function_call + function_call_output items.
    input.push(json!({"role": "assistant", "content": "Let me look."}));
    for item in &output[1..] {
        let call_id = item["call_id"].as_str().unwrap();
        assert!(call_id.starts_with("call_llmsim_research_0_"), "{call_id}");
        input.push(json!({"type": "function_call", "call_id": call_id,
                          "name": item["name"], "arguments": item["arguments"]}));
    }
    for item in &output[1..] {
        input.push(
            json!({"type": "function_call_output", "call_id": item["call_id"], "output": "ok"}),
        );
    }

    // Step 1: 429 first (Responses error shape), then the call.
    let req = json!({"model": "gpt-5", "input": input, "tools": responses_tools()});
    let (status, headers, body) = post(&r, uri, &req).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers["retry-after-ms"], "1500");
    assert!(body.unwrap().contains("rate_limit"));
    let (status, headers, body) = post(&r, uri, &req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-llmsim-step"], "1");
    let v = json_body(body);
    let call = &v["output"][0];
    assert_eq!(call["arguments"], "{\"command\":\"echo 2\"}");
    input.push(json!({"type": "reasoning", "id": "rs_1", "summary": []}));
    input.push(json!({"type": "function_call", "call_id": call["call_id"],
                      "name": "bash", "arguments": call["arguments"]}));
    input.push(json!({"type": "function_call_output", "call_id": call["call_id"], "output": "2"}));

    // Step 2: reasoning item, then 30 lorem tokens.
    let req = json!({"model": "gpt-5", "input": input, "tools": responses_tools()});
    let (_, headers, body) = post(&r, uri, &req).await;
    assert_eq!(headers["x-llmsim-step"], "2");
    let v = json_body(body);
    assert_eq!(v["output"][0]["type"], "reasoning");
    assert_eq!(v["output"][1]["type"], "message");
    assert_eq!(
        v["output_text"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .count(),
        30
    );
    assert_eq!(v["usage"]["output_tokens_details"]["reasoning_tokens"], 10);
}

#[tokio::test]
async fn responses_streams_text_and_function_calls() {
    let r = router();
    let req = json!({"model": "gpt-5", "stream": true, "tools": responses_tools(),
                     "input": "go [[llmsim:research]]"});
    let (status, _, body) = post(&r, "/openai/v1/responses", &req).await;
    assert_eq!(status, StatusCode::OK);
    let body = body.unwrap();
    let created = body.find("response.created").unwrap();
    let text = body.find("response.output_text.delta").unwrap();
    let args = body.find("response.function_call_arguments.done").unwrap();
    let done = body.find("response.completed").unwrap();
    assert!(created < text && text < args && args < done, "{body}");
    assert!(body.contains("\"name\":\"bash\""));
    // Sequence numbers count up from 0 without gaps.
    let seqs: Vec<u64> = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|d| {
            serde_json::from_str::<Value>(d).unwrap()["sequence_number"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert_eq!(seqs, (0..seqs.len() as u64).collect::<Vec<_>>());
}

#[tokio::test]
async fn responses_stream_cut_drops_the_connection() {
    let r = router();
    let req = json!({"model": "gpt-5", "stream": true, "input": "x [[llmsim:broken]]"});
    let (_, _, body) = post(&r, "/openai/v1/responses", &req).await;
    assert!(body.is_err(), "the body should abort");
    // The retry completes.
    let (_, _, body) = post(&r, "/openai/v1/responses", &req).await;
    assert!(body.unwrap().contains("response.completed"));
}

#[tokio::test]
async fn responses_without_marker_is_unchanged() {
    let (status, headers, _) = post(
        &router(),
        "/openai/v1/responses",
        &json!({"model": "gpt-5", "input": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("x-llmsim-scenario").is_none());
}

// --- Anthropic Messages ---

fn anthropic_tools() -> Value {
    json!([
        {"name": "read_file", "input_schema": {
            "type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]
        }},
        {"name": "bash", "input_schema": {"type": "object"}}
    ])
}

#[tokio::test]
async fn anthropic_plays_a_scenario_across_a_conversation() {
    let r = router();
    let uri = "/anthropic/v1/messages";
    let mut messages = vec![json!({"role": "user", "content": "Look [[llmsim:research]]"})];
    let req = |m: &Vec<Value>| json!({"model": "claude-opus-4-8", "max_tokens": 1024, "tools": anthropic_tools(), "messages": m});

    let (status, headers, body) = post(&r, uri, &req(&messages)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-llmsim-step"], "0");
    let v = json_body(body);
    assert_eq!(v["stop_reason"], "tool_use");
    let content = v["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "Let me look.");
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["input"], json!({"path": "."}));
    assert_eq!(content[2]["name"], "bash");
    let results: Vec<Value> = content[1..]
        .iter()
        .map(|b| json!({"type": "tool_result", "tool_use_id": b["id"], "content": "ok"}))
        .collect();
    messages.push(json!({"role": "assistant", "content": content}));
    messages.push(json!({"role": "user", "content": results}));

    // Step 1: Anthropic-shaped 429, then the call.
    let (status, headers, body) = post(&r, uri, &req(&messages)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers["retry-after"], "2");
    let e = json_body(body);
    assert_eq!(e["type"], "error");
    assert_eq!(e["error"]["type"], "rate_limit_error");
    let (_, headers, body) = post(&r, uri, &req(&messages)).await;
    assert_eq!(headers["x-llmsim-step"], "1");
    let v = json_body(body);
    assert_eq!(v["content"][0]["input"], json!({"command": "echo 2"}));
    let id = v["content"][0]["id"].clone();
    messages.push(json!({"role": "assistant", "content": v["content"]}));
    messages.push(json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": id, "content": "2"}
    ]}));

    // Step 2: thinking then text; thinking billed as output.
    let (_, headers, body) = post(&r, uri, &req(&messages)).await;
    assert_eq!(headers["x-llmsim-step"], "2");
    let v = json_body(body);
    assert_eq!(v["content"][0]["type"], "thinking");
    assert!(!v["content"][0]["signature"].as_str().unwrap().is_empty());
    assert_eq!(v["content"][1]["type"], "text");
    assert_eq!(v["stop_reason"], "end_turn");
    assert!(v["usage"]["output_tokens"].as_u64().unwrap() >= 40);

    // A new user message starts the scenario over.
    messages.push(json!({"role": "assistant", "content": v["content"]}));
    messages.push(json!({"role": "user", "content": "again [[llmsim:research]]"}));
    let (_, headers, _) = post(&r, uri, &req(&messages)).await;
    assert_eq!(headers["x-llmsim-step"], "0");
}

#[tokio::test]
async fn anthropic_streams_tool_use_with_input_json() {
    let r = router();
    let req = json!({"model": "claude-opus-4-8", "max_tokens": 1024, "stream": true,
                     "tools": anthropic_tools(),
                     "messages": [{"role": "user", "content": "go [[llmsim:research]]"}]});
    let (status, _, body) = post(&r, "/anthropic/v1/messages", &req).await;
    assert_eq!(status, StatusCode::OK);
    let body = body.unwrap();
    let order = [
        "event: message_start",
        "\"text_delta\"",
        "\"type\":\"tool_use\"",
        "input_json_delta",
        "\"stop_reason\":\"tool_use\"",
        "event: message_stop",
    ];
    let mut at = 0;
    for needle in order {
        let pos = body[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} in {body}"));
        at += pos;
    }
    // Three blocks: text, then two tool_use.
    assert_eq!(body.matches("event: content_block_start").count(), 3);
}

#[tokio::test]
async fn anthropic_model_id_selects_a_scenario() {
    let req = json!({"model": "llmsim-scenario-broken", "max_tokens": 64,
                     "messages": [{"role": "user", "content": "hi"}]});
    let r = router();
    let (_, headers, body) = post(&r, "/anthropic/v1/messages", &req).await;
    assert_eq!(headers["x-llmsim-scenario"], "broken");
    assert!(body.is_err(), "first attempt is cut");
    let (_, _, body) = post(&r, "/anthropic/v1/messages", &req).await;
    assert_eq!(json_body(body)["content"][0]["type"], "text");
}

#[tokio::test]
async fn anthropic_unknown_scenario_is_an_error() {
    let req = json!({"model": "claude-opus-4-8", "max_tokens": 64,
                     "messages": [{"role": "user", "content": "[[llmsim:nope]]"}]});
    let (status, _, body) = post(&router(), "/anthropic/v1/messages", &req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json_body(body)["type"], "error");
}
