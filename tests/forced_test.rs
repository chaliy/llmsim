//! Structured outputs and forced tool calls without a script.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router(auto_tool_calls: bool) -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.response.auto_tool_calls = auto_tool_calls;
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 16 * 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn person_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0, "maximum": 120},
            "email": {"type": "string", "format": "email"}
        },
        "required": ["name", "age", "email"],
        "additionalProperties": false
    })
}

fn weather_tool() -> Value {
    json!({"type": "function", "function": {"name": "get_weather", "parameters": {
        "type": "object",
        "properties": {"city": {"type": "string"}, "unit": {"enum": ["c", "f"]}},
        "required": ["city", "unit"]
    }}})
}

#[tokio::test]
async fn chat_json_schema_returns_schema_valid_json() {
    let app = router(false);
    let (status, body) = post(
        &app,
        "/openai/v1/chat/completions",
        json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "Extract the person"}],
            "response_format": {"type": "json_schema", "json_schema": {
                "name": "person", "strict": true, "schema": person_schema()
            }}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    let person: Value = serde_json::from_str(content).unwrap();
    assert!(person["name"].is_string());
    assert!((0..=120).contains(&person["age"].as_i64().unwrap()));
    assert!(person["email"].as_str().unwrap().contains('@'));
}

#[tokio::test]
async fn chat_json_schema_streams_the_same_json() {
    let app = router(false);
    let (_, body) = post(
        &app,
        "/openai/v1/chat/completions",
        json!({
            "model": "gpt-5", "stream": true,
            "messages": [{"role": "user", "content": "Extract the person"}],
            "response_format": {"type": "json_schema", "json_schema": {"name": "p", "schema": person_schema()}}
        }),
    )
    .await;
    let text: String = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|d| *d != "[DONE]")
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| {
            v["choices"][0]["delta"]["content"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    let person: Value = serde_json::from_str(&text).unwrap();
    assert!(person["age"].is_i64());
}

#[tokio::test]
async fn chat_required_tool_choice_calls_a_tool() {
    let app = router(false);
    let (_, body) = post(
        &app,
        "/openai/v1/chat/completions",
        json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "Weather in Kyiv?"}],
            "tools": [weather_tool()],
            "tool_choice": "required"
        }),
    )
    .await;
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
    let call = &body["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "get_weather");
    let args: Value =
        serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
    assert!(args["city"].is_string());
    assert!(["c", "f"].contains(&args["unit"].as_str().unwrap()));
    assert!(call["id"].as_str().unwrap().starts_with("call_"));
}

#[tokio::test]
async fn chat_auto_tool_calls_once_then_answers() {
    let app = router(true);
    let (_, body) = post(
        &app,
        "/openai/v1/chat/completions",
        json!({"model": "gpt-5", "tools": [weather_tool()],
               "messages": [{"role": "user", "content": "Weather?"}]}),
    )
    .await;
    let body: Value = serde_json::from_str(&body).unwrap();
    let call = body["choices"][0]["message"]["tool_calls"][0].clone();
    assert_eq!(call["function"]["name"], "get_weather");
    let (_, body) = post(
        &app,
        "/openai/v1/chat/completions",
        json!({"model": "gpt-5", "tools": [weather_tool()], "messages": [
            {"role": "user", "content": "Weather?"},
            {"role": "assistant", "content": null, "tool_calls": [call]},
            {"role": "tool", "tool_call_id": "x", "content": "sunny"}
        ]}),
    )
    .await;
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    // Auto is off by default.
    let (_, body) = post(
        &router(false),
        "/openai/v1/chat/completions",
        json!({"model": "gpt-5", "tools": [weather_tool()],
               "messages": [{"role": "user", "content": "Weather?"}]}),
    )
    .await;
    let body: Value = serde_json::from_str(&body).unwrap();
    assert!(body["choices"][0]["message"]["tool_calls"].is_null());
}

fn anthropic_tool() -> Value {
    json!({"name": "get_weather", "input_schema": {
        "type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]
    }})
}

#[tokio::test]
async fn anthropic_forced_tool_use_non_streaming() {
    let app = router(false);
    let (status, body) = post(
        &app,
        "/anthropic/v1/messages",
        json!({"model": "claude-sonnet-4-5", "max_tokens": 200,
               "tools": [anthropic_tool()], "tool_choice": {"type": "tool", "name": "get_weather"},
               "messages": [{"role": "user", "content": "Weather?"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"].as_array().unwrap().len(), 1);
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert!(body["content"][0]["id"]
        .as_str()
        .unwrap()
        .starts_with("toolu_"));
    assert!(body["content"][0]["input"]["city"].is_string());
}

#[tokio::test]
async fn anthropic_forced_tool_use_streams_input_json_delta() {
    let app = router(false);
    let (_, body) = post(
        &app,
        "/anthropic/v1/messages",
        json!({"model": "claude-sonnet-4-5", "max_tokens": 200, "stream": true,
               "tools": [anthropic_tool()], "tool_choice": {"type": "any"},
               "messages": [{"role": "user", "content": "Weather?"}]}),
    )
    .await;
    assert!(body.contains("\"type\":\"tool_use\""));
    assert!(body.contains("input_json_delta"));
    assert!(body.contains("\"stop_reason\":\"tool_use\""));
}

#[tokio::test]
async fn anthropic_output_format_returns_schema_valid_json() {
    let app = router(false);
    let (_, body) = post(
        &app,
        "/anthropic/v1/messages",
        json!({"model": "claude-sonnet-4-5", "max_tokens": 200,
               "output_config": {"format": {"type": "json_schema", "schema": person_schema()}},
               "messages": [{"role": "user", "content": "Extract"}]}),
    )
    .await;
    let body: Value = serde_json::from_str(&body).unwrap();
    let text = body["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "text")
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    let person: Value = serde_json::from_str(&text).unwrap();
    assert!(person["name"].is_string());
}

#[tokio::test]
async fn responses_text_format_returns_schema_valid_json() {
    let app = router(false);
    let (status, body) = post(
        &app,
        "/openai/v1/responses",
        json!({"model": "gpt-5", "input": "Extract",
               "text": {"format": {"type": "json_schema", "name": "p", "schema": person_schema()}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    let text = body["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["type"] == "message")
        .unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let person: Value = serde_json::from_str(&text).unwrap();
    assert!(person["email"].is_string());
}
