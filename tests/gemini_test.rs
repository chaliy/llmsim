//! Gemini native API: generateContent, streaming, tools, JSON, thinking,
//! countTokens, embeddings, models and scenarios.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::scenario::ScenarioSet;
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router() -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some("instant".to_string());
    let scenarios = ScenarioSet::from_json(
        r#"[{"name": "gtools", "steps": [
            {"turn": {"type": "tool_calls", "reasoning_tokens": 5,
                      "calls": [{"name": "lookup", "arguments": {"q": "x"}}]}},
            {"turn": {"type": "assistant", "text": "all done here"}}
        ]}]"#,
    )
    .unwrap();
    build_router(Arc::new(
        AppState::new(config, new_shared_stats()).with_scenarios(Arc::new(scenarios)),
    ))
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 64 * 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let (s, b) = send(app, "POST", uri, Some(body)).await;
    (s, serde_json::from_str(&b).unwrap_or(Value::String(b)))
}

const FLASH: &str = "/gemini/v1beta/models/gemini-2.0-flash:generateContent";
const THINKER: &str = "/gemini/v1beta/models/gemini-2.5-flash:generateContent";

fn user(text: &str) -> Value {
    json!({"contents": [{"role": "user", "parts": [{"text": text}]}]})
}

#[tokio::test]
async fn generate_content_shape_and_usage() {
    let (status, v) = post(&router(), FLASH, user("Hello there")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let cand = &v["candidates"][0];
    assert_eq!(cand["content"]["role"], "model");
    assert!(!cand["content"]["parts"][0]["text"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(cand["finishReason"], "STOP");
    let u = &v["usageMetadata"];
    let total =
        u["promptTokenCount"].as_u64().unwrap() + u["candidatesTokenCount"].as_u64().unwrap();
    assert_eq!(u["totalTokenCount"].as_u64().unwrap(), total);
    assert!(
        u.get("thoughtsTokenCount").is_none(),
        "2.0 Flash does not think"
    );
    assert_eq!(v["modelVersion"], "gemini-2.0-flash");
}

#[tokio::test]
async fn thinking_models_bill_thoughts_and_can_show_them() {
    let app = router();
    let (_, v) = post(&app, THINKER, user("Plan a trip")).await;
    assert!(v["usageMetadata"]["thoughtsTokenCount"].as_u64().unwrap() >= 64);

    let mut body = user("Plan a trip");
    body["generationConfig"] = json!({"thinkingConfig": {"includeThoughts": true}});
    let (_, v) = post(&app, THINKER, body).await;
    let parts = v["candidates"][0]["content"]["parts"].as_array().unwrap();
    assert_eq!(parts[0]["thought"], true);
    assert!(parts[1].get("thought").is_none());

    let mut body = user("Plan a trip");
    body["generation_config"] = json!({"thinking_config": {"thinking_budget": 0}});
    let (_, v) = post(&app, THINKER, body).await;
    assert!(
        v["usageMetadata"].get("thoughtsTokenCount").is_none(),
        "snake_case works too"
    );

    // Pro cannot turn thinking off.
    let mut body = user("x");
    body["generationConfig"] = json!({"thinkingConfig": {"thinkingBudget": 0}});
    let (status, v) = post(
        &app,
        "/gemini/v1beta/models/gemini-2.5-pro:generateContent",
        body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["status"], "INVALID_ARGUMENT");
}

#[tokio::test]
async fn thinking_can_eat_the_whole_output_budget() {
    let mut body = user("Explain everything");
    body["generationConfig"] = json!({"maxOutputTokens": 50});
    let (status, v) = post(&router(), THINKER, body).await;
    assert_eq!(status, StatusCode::OK);
    let cand = &v["candidates"][0];
    assert_eq!(cand["finishReason"], "MAX_TOKENS");
    assert!(cand["content"].get("parts").is_none(), "{v}");
    assert_eq!(v["usageMetadata"]["thoughtsTokenCount"], 50);
}

#[tokio::test]
async fn forced_function_call_and_json_schema() {
    let app = router();
    let mut body = user("weather in Paris?");
    body["tools"] = json!([{"functionDeclarations": [{"name": "get_weather", "parameters": {
        "type": "OBJECT", "properties": {"city": {"type": "STRING"}}, "required": ["city"]}}]}]);
    body["toolConfig"] = json!({"functionCallingConfig": {"mode": "ANY"}});
    let (_, v) = post(&app, THINKER, body).await;
    let part = &v["candidates"][0]["content"]["parts"][0];
    assert_eq!(part["functionCall"]["name"], "get_weather");
    assert!(part["functionCall"]["args"]["city"].is_string());
    assert!(
        part["thoughtSignature"].is_string(),
        "thinking models sign calls"
    );

    let mut body = user("a person");
    body["generationConfig"] = json!({"responseMimeType": "application/json", "responseSchema": {
        "type": "OBJECT", "properties": {"age": {"type": "INTEGER"}}, "required": ["age"]}});
    let (_, v) = post(&app, FLASH, body).await;
    let text = v["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert!(parsed["age"].is_i64());
}

#[tokio::test]
async fn streams_as_sse_and_as_json_array() {
    let app = router();
    let (status, body) = send(
        &app,
        "POST",
        "/gemini/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse",
        Some(user("Tell me a story")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let chunks: Vec<Value> = body
        .split("\r\n\r\n")
        .filter_map(|f| f.strip_prefix("data: "))
        .map(|d| serde_json::from_str(d).unwrap())
        .collect();
    assert!(chunks.len() > 1);
    assert_eq!(
        chunks.last().unwrap()["candidates"][0]["finishReason"],
        "STOP"
    );
    assert!(chunks.last().unwrap()["usageMetadata"].is_object());

    let (_, body) = send(
        &app,
        "POST",
        "/gemini/v1beta/models/gemini-2.0-flash:streamGenerateContent",
        Some(user("Tell me a story")),
    )
    .await;
    let arr: Vec<Value> = serde_json::from_str(&body).unwrap();
    assert!(arr.len() > 1);
}

#[tokio::test]
async fn errors_and_limits() {
    let app = router();
    let (status, v) = post(&app, FLASH, json!({"contents": []})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("contents is not specified"));

    let mut body = user("x");
    body["generationConfig"] = json!({"maxOutputTokens": 10_000_000});
    let (status, v) = post(&app, FLASH, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("maxOutputTokens"));

    let (status, _) = post(
        &app,
        "/gemini/v1beta/models/gemini-2.0-flash:nope",
        user("x"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn implicit_cache_reports_cached_tokens() {
    let app = router();
    let mut body = user("question one");
    body["systemInstruction"] =
        json!({"parts": [{"text": "Follow the house rules carefully. ".repeat(300)}]});
    post(&app, FLASH, body.clone()).await;
    let (_, v) = post(&app, FLASH, body).await;
    assert!(
        v["usageMetadata"]["cachedContentTokenCount"]
            .as_u64()
            .unwrap()
            >= 1024,
        "{v}"
    );
}

#[tokio::test]
async fn count_tokens_embeddings_and_models() {
    let app = router();
    let (_, v) = post(
        &app,
        "/gemini/v1beta/models/gemini-2.5-flash:countTokens",
        user("count these words"),
    )
    .await;
    assert!(v["totalTokens"].as_u64().unwrap() > 0);

    let (_, v) = post(
        &app,
        "/gemini/v1beta/models/gemini-embedding-001:embedContent",
        json!({"content": {"parts": [{"text": "hello"}]}, "outputDimensionality": 768}),
    )
    .await;
    let values = v["embedding"]["values"].as_array().unwrap();
    assert_eq!(values.len(), 768);
    let norm: f64 = values.iter().map(|x| x.as_f64().unwrap().powi(2)).sum();
    assert!(
        norm < 0.9,
        "shortened gemini-embedding-001 vectors are not renormalized"
    );

    let (_, v) = post(
        &app,
        "/gemini/v1beta/models/text-embedding-004:batchEmbedContents",
        json!({"requests": [
            {"model": "models/text-embedding-004", "content": {"parts": [{"text": "a"}]}},
            {"model": "models/text-embedding-004", "content": {"parts": [{"text": "b"}]}}
        ]}),
    )
    .await;
    assert_eq!(v["embeddings"].as_array().unwrap().len(), 2);
    assert_eq!(v["embeddings"][0]["values"].as_array().unwrap().len(), 768);

    let (_, body) = send(&app, "GET", "/gemini/v1beta/models", None).await;
    let v: Value = serde_json::from_str(&body).unwrap();
    let names: Vec<&str> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"models/gemini-2.5-flash"));
    assert!(names.contains(&"models/gemini-embedding-001"));
    let (status, body) = send(&app, "GET", "/gemini/v1beta/models/gemini-2.5-pro", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"inputTokenLimit\""));
}

#[tokio::test]
async fn scenarios_play_on_gemini() {
    let app = router();
    let tools = json!([{"functionDeclarations": [{"name": "lookup"}]}]);
    let mut body = json!({"contents": [{"role": "user", "parts": [{"text": "go [[llmsim:gtools]]"}]}],
                          "tools": tools});
    let (_, v) = post(&app, FLASH, body.clone()).await;
    let parts = v["candidates"][0]["content"]["parts"].as_array().unwrap();
    let call = &parts[0]["functionCall"];
    assert_eq!(call["name"], "lookup");
    assert_eq!(call["args"], json!({"q": "x"}));
    assert_eq!(v["usageMetadata"]["thoughtsTokenCount"], 5);

    let contents = body["contents"].as_array_mut().unwrap();
    contents.push(json!({"role": "model", "parts": parts}));
    contents.push(json!({"role": "user", "parts": [{"functionResponse": {
        "id": call["id"], "name": "lookup", "response": {"ok": true}}}]}));
    let (status, text) = send(
        &app,
        "POST",
        "/gemini/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("\"text\":\"all\""), "{text}");
    assert!(text.contains("\"finishReason\":\"STOP\""));
}
