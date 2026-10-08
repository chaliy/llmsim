//! End-to-end tests for the Anthropic Messages API endpoints.
//!
//! Spins the llmsim Axum router up in-process (no real socket) and drives it
//! via `tower::ServiceExt::oneshot`, exercising the exact HTTP path the
//! official Anthropic SDKs would hit at `{base_url}/anthropic`.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::script::{OnExhausted, Script, ScriptSpec, SimError, SimToolCall, SimTurn};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

/// Router with deterministic, fast behavior and no script.
fn router() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.response.generator = "echo".to_string();
    let state = AppState::new(config, new_shared_stats());
    build_router(Arc::new(state))
}

/// Router driven by a script (instant latency).
fn router_with_script(script: Script) -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    let mut state = AppState::new(config, new_shared_stats());
    state = state.with_script(Arc::new(script));
    build_router(Arc::new(state))
}

/// Router that always injects a given error type (for error-shape tests).
fn router_rate_limited() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.errors.rate_limit_rate = 1.0;
    let state = AppState::new(config, new_shared_stats());
    build_router(Arc::new(state))
}

async fn post_messages(router: &axum::Router, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

// --- Non-streaming messages ---

#[tokio::test]
async fn messages_basic_non_streaming() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 64,
            "messages": [{"role": "user", "content": "Hello, Claude"}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["role"], "assistant");
    assert_eq!(v["model"], "claude-opus-4-8");
    assert_eq!(v["stop_reason"], "end_turn");
    assert_eq!(v["content"][0]["type"], "text");
    assert!(v["content"][0]["text"].is_string());
    assert!(v["usage"]["input_tokens"].as_u64().unwrap() > 0);
    assert!(v["usage"]["output_tokens"].as_u64().unwrap() > 0);
    assert!(v["id"].as_str().unwrap().starts_with("msg_"));
}

#[tokio::test]
async fn messages_accepts_system_and_block_content() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 128,
            "system": "You are a pirate.",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Say hi"},
                    {"type": "text", "text": "to the crew"}
                ]}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "message");
    // Echo generator includes the prompt; system + both text blocks should flow in.
    let text = v["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("pirate"));
    assert!(text.contains("crew"));
}

#[tokio::test]
async fn messages_tolerates_image_blocks() {
    let router = router();
    let (status, _body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 64,
            "messages": [{"role": "user", "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBOR"}},
                {"type": "text", "text": "describe"}
            ]}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn messages_multiturn_history() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 64,
            "messages": [
                {"role": "user", "content": "My name is Ada."},
                {"role": "assistant", "content": "Hello Ada!"},
                {"role": "user", "content": "What is my name?"}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["role"], "assistant");
}

// --- Streaming ---

#[tokio::test]
async fn messages_streaming_event_sequence() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 64,
            "stream": true,
            "messages": [{"role": "user", "content": "stream please"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Anthropic SSE lifecycle events, in order, no [DONE] sentinel.
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: content_block_stop"));
    assert!(body.contains("event: message_delta"));
    assert!(body.contains("event: message_stop"));
    assert!(body.contains("text_delta"));
    assert!(!body.contains("[DONE]"));

    // message_start precedes message_stop.
    let start = body.find("message_start").unwrap();
    let stop = body.find("message_stop").unwrap();
    assert!(start < stop);
}

// --- Models endpoints ---

#[tokio::test]
async fn models_list_returns_claude_models() {
    let router = router();
    let (status, body) = get(&router, "/anthropic/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    let data = v["data"].as_array().unwrap();
    assert!(!data.is_empty());
    assert_eq!(data[0]["type"], "model");
    assert_eq!(v["has_more"], false);

    let ids: Vec<&str> = data.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"claude-opus-4-8"));
    assert!(ids.contains(&"claude-sonnet-4-6"));
    assert!(ids.contains(&"claude-haiku-4-5"));
    assert!(ids.contains(&"claude-fable-5"));
}

#[tokio::test]
async fn model_get_by_alias_and_snapshot() {
    let router = router();

    let (status, body) = get(&router, "/anthropic/v1/models/claude-opus-4-8").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "model");
    assert_eq!(v["id"], "claude-opus-4-8");
    assert_eq!(v["display_name"], "Claude Opus 4.8");
    assert!(v["created_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(v["max_input_tokens"], 1_000_000);
    assert_eq!(v["max_tokens"], 128_000);

    // Dated snapshot alias resolves too.
    let (status, _) = get(&router, "/anthropic/v1/models/claude-haiku-4-5-20251001").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn model_get_unknown_returns_anthropic_404() {
    let router = router();
    let (status, body) = get(&router, "/anthropic/v1/models/not-a-model").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "not_found_error");
}

// --- Scripted mode ---

#[tokio::test]
async fn messages_scripted_text_replay_in_order() {
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
        "model": "claude-opus-4-8",
        "max_tokens": 32,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let texts: Vec<String> = {
        let mut v = Vec::new();
        for _ in 0..3 {
            let (s, t) = post_messages(&router, body.clone()).await;
            assert_eq!(s, StatusCode::OK);
            let parsed: Value = serde_json::from_str(&t).unwrap();
            v.push(parsed["content"][0]["text"].as_str().unwrap().to_string());
        }
        v
    };
    assert_eq!(texts, vec!["first", "second", "second"]); // RepeatLast
}

#[tokio::test]
async fn messages_scripted_tool_use_blocks() {
    let script = Script::new(vec![SimTurn::ToolCalls {
        calls: vec![SimToolCall {
            name: "get_weather".into(),
            arguments: json!({"location": "Paris"}),
            id: Some("toolu_fixed".into()),
        }],
    }]);
    let router = router_with_script(script);
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 256,
            "messages": [{"role": "user", "content": "weather in Paris?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get the weather",
                "input_schema": {"type": "object", "properties": {"location": {"type": "string"}}}
            }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["stop_reason"], "tool_use");
    let block = &v["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "toolu_fixed");
    assert_eq!(block["name"], "get_weather");
    assert_eq!(block["input"]["location"], "Paris");
}

#[tokio::test]
async fn messages_scripted_mixed_text_and_tool_use() {
    let script = Script::new(vec![SimTurn::Mixed {
        text: "Let me check.".into(),
        calls: vec![SimToolCall {
            name: "lookup".into(),
            arguments: json!({"q": "x"}),
            id: None,
        }],
    }]);
    let router = router_with_script(script);
    let (status, body) = post_messages(
        &router,
        json!({"model": "claude-opus-4-8", "max_tokens": 64, "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["stop_reason"], "tool_use");
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["content"][0]["text"], "Let me check.");
    assert_eq!(v["content"][1]["type"], "tool_use");
    // Auto-generated id uses the toolu_ prefix.
    assert!(v["content"][1]["id"]
        .as_str()
        .unwrap()
        .starts_with("toolu_"));
}

#[tokio::test]
async fn messages_scripted_error_turn_uses_anthropic_shape() {
    let script = Script::new(vec![SimTurn::Error(SimError::RateLimit)]);
    let router = router_with_script(script);
    let (status, body) = post_messages(
        &router,
        json!({"model": "claude-opus-4-8", "max_tokens": 32, "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "rate_limit_error");
    assert!(v["error"]["message"].is_string());
}

#[tokio::test]
async fn messages_scripted_invalid_request_error() {
    let script = Script::new(vec![SimTurn::Error(SimError::InvalidRequest {
        message: "bad input".into(),
    })]);
    let router = router_with_script(script);
    let (status, body) = post_messages(
        &router,
        json!({"model": "claude-opus-4-8", "max_tokens": 32, "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], "invalid_request_error");
    assert_eq!(v["error"]["message"], "bad input");
}

#[tokio::test]
async fn messages_scripted_streaming_text() {
    let script = Script::new(vec![SimTurn::Assistant {
        text: "streamed reply".into(),
    }]);
    let router = router_with_script(script);
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 64,
            "stream": true,
            "messages": [{"role": "user", "content": "x"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: message_stop"));
    // The scripted text should appear across text_delta events.
    assert!(body.contains("streamed") || body.contains("reply"));
}

// --- Error injection ---

#[tokio::test]
async fn messages_error_injection_rate_limit() {
    let router = router_rate_limited();
    let (status, body) = post_messages(
        &router,
        json!({"model": "claude-opus-4-8", "max_tokens": 32, "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "rate_limit_error");
}

// --- Stats integration ---

#[tokio::test]
async fn messages_increment_stats() {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    config.response.generator = "echo".to_string();
    let stats = new_shared_stats();
    let state = AppState::new(config, stats.clone());
    let router = build_router(Arc::new(state));

    for _ in 0..3 {
        let (s, _) = post_messages(
            &router,
            json!({"model": "claude-opus-4-8", "max_tokens": 16, "messages": [{"role": "user", "content": "x"}]}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.messages_requests, 3);
    assert_eq!(snapshot.total_requests, 3);
    assert_eq!(*snapshot.model_requests.get("claude-opus-4-8").unwrap(), 3);
}

#[tokio::test]
async fn messages_missing_max_tokens_is_rejected() {
    // max_tokens is required by the Anthropic API; omitting it should 4xx
    // (JSON deserialization rejects the body before the handler runs).
    let router = router();
    let req = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"model": "claude-opus-4-8", "messages": [{"role": "user", "content": "x"}]})
                .to_string(),
        ))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_client_error());
}

// --- Extended thinking + effort ---

#[tokio::test]
async fn messages_5x_model_thinks_by_default_with_omitted_text() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-5-5",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "think about this"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    let content = v["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["thinking"], "");
    assert!(!content[0]["signature"].as_str().unwrap().is_empty());
    assert_eq!(content[1]["type"], "text");
    assert_eq!(v["stop_reason"], "end_turn");
}

#[tokio::test]
async fn messages_4x_model_has_no_thinking_unless_requested() {
    let router = router();
    let (_, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["content"].as_array().unwrap().len(), 1);
    assert_eq!(v["content"][0]["type"], "text");

    let (_, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-4-8",
            "max_tokens": 1024,
            "thinking": {"type": "adaptive", "display": "summarized"},
            "output_config": {"effort": "xhigh"},
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["content"][0]["type"], "thinking");
    assert!(!v["content"][0]["thinking"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn messages_thinking_tokens_billed_in_output_tokens() {
    let router = router();
    let body_for = |effort: &str| {
        json!({
            "model": "claude-sonnet-5-5",
            "max_tokens": 1024,
            "output_config": {"effort": effort},
            "messages": [{"role": "user", "content": "same prompt every time"}]
        })
    };
    let (_, low) = post_messages(&router, body_for("low")).await;
    let (_, max) = post_messages(&router, body_for("max")).await;
    let low: Value = serde_json::from_str(&low).unwrap();
    let max: Value = serde_json::from_str(&max).unwrap();
    assert!(max["usage"]["output_tokens"].as_u64() > low["usage"]["output_tokens"].as_u64());
}

#[tokio::test]
async fn messages_disabled_thinking_turns_it_off_where_allowed() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-5",
            "max_tokens": 1024,
            "thinking": {"type": "disabled"},
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["content"][0]["type"], "text");
}

#[tokio::test]
async fn messages_rejects_unsupported_thinking_configs() {
    let router = router();
    for (model, thinking) in [
        // budget_tokens was removed on 4.7+ / 5.x models
        (
            "claude-fable-5-1",
            json!({"type": "enabled", "budget_tokens": 2048}),
        ),
        // thinking cannot be disabled on Opus 5.5
        ("claude-opus-5-5", json!({"type": "disabled"})),
        // budget must be below max_tokens
        (
            "claude-sonnet-4-5",
            json!({"type": "enabled", "budget_tokens": 4096}),
        ),
    ] {
        let (status, body) = post_messages(
            &router,
            json!({
                "model": model,
                "max_tokens": 4096,
                "thinking": thinking,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{model}: {body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "invalid_request_error");
    }

    let (status, _) = post_messages(
        &router,
        json!({
            "model": "claude-opus-5-5",
            "max_tokens": 1024,
            "output_config": {"effort": "ludicrous"},
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn messages_streaming_thinking_block_precedes_text() {
    let router = router();
    let (status, body) = post_messages(
        &router,
        json!({
            "model": "claude-fable-5-1",
            "max_tokens": 1024,
            "stream": true,
            "thinking": {"type": "adaptive", "display": "summarized"},
            "messages": [{"role": "user", "content": "stream with thinking"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let thinking_start = body.find(r#""type":"thinking""#).expect("thinking block");
    let thinking_delta = body.find("thinking_delta").expect("thinking_delta");
    let signature = body.find("signature_delta").expect("signature_delta");
    let text_start = body.find(r#""type":"text""#).expect("text block");
    let text_delta = body.find("text_delta").expect("text_delta");
    assert!(thinking_start < thinking_delta);
    assert!(thinking_delta < signature);
    assert!(signature < text_start);
    assert!(text_start < text_delta);
    // The text block streams at index 1 after the thinking block.
    let starts: Vec<Value> = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|d| serde_json::from_str::<Value>(d).unwrap())
        .filter(|v| v["type"] == "content_block_start")
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0]["index"], 0);
    assert_eq!(starts[0]["content_block"]["type"], "thinking");
    assert_eq!(starts[1]["index"], 1);
    assert_eq!(starts[1]["content_block"]["type"], "text");
}

#[tokio::test]
async fn messages_scripted_turns_have_no_thinking() {
    let spec = ScriptSpec {
        turns: vec![SimTurn::Assistant {
            text: "scripted".to_string(),
        }],
        on_exhausted: OnExhausted::Error,
    };
    let router = router_with_script(Script::from_spec(spec).unwrap());
    let (_, body) = post_messages(
        &router,
        json!({
            "model": "claude-opus-5-5",
            "max_tokens": 64,
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["content"].as_array().unwrap().len(), 1);
    assert_eq!(v["content"][0]["text"], "scripted");
}

// --- Token counting ---

async fn post_count_tokens(router: &axum::Router, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages/count_tokens")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn count_tokens_matches_messages_input_tokens() {
    let router = router();
    let prompt = json!({
        "model": "claude-opus-5-5",
        "system": "You are terse.",
        "messages": [
            {"role": "user", "content": "How many tokens is this?"},
            {"role": "assistant", "content": "Let me count."},
            {"role": "user", "content": [{"type": "text", "text": "Please do."}]}
        ]
    });
    let (status, body) = post_count_tokens(&router, prompt.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let counted: Value = serde_json::from_str(&body).unwrap();
    let counted = counted["input_tokens"].as_u64().unwrap();
    assert!(counted > 0);

    let mut msg = prompt;
    msg["max_tokens"] = json!(64);
    let (_, body) = post_messages(&router, msg).await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["usage"]["input_tokens"].as_u64().unwrap(), counted);
}

#[tokio::test]
async fn count_tokens_does_not_need_max_tokens_and_validates_thinking() {
    let router = router();
    let (status, _) = post_count_tokens(
        &router,
        json!({"model": "claude-haiku-5-5", "messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_count_tokens(
        &router,
        json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "x"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], "invalid_request_error");
}
