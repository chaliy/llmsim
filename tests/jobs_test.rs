//! Background Responses, OpenAI Files and Batches, Anthropic Message Batches.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use tower::ServiceExt;

fn router(batch_ms: u64, profile: &str) -> axum::Router {
    let mut config = Config::default();
    config.latency.profile = Some(profile.to_string());
    config.batch.completion_ms = batch_ms;
    build_router(Arc::new(AppState::new(config, new_shared_stats())))
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "sim.test");
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

async fn json_call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (s, b) = call(app, method, uri, body).await;
    (s, serde_json::from_str(&b).unwrap_or(Value::String(b)))
}

async fn upload(app: &axum::Router, content: &str) -> Value {
    let boundary = "llmsimboundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nbatch\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"in.jsonl\"\r\n\
         Content-Type: application/jsonl\r\n\r\n{content}\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri("/openai/v1/files")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// --- Background responses ---

#[tokio::test]
async fn background_response_is_queued_then_cancellable() {
    // Luna's profile: about 3 s before the response completes.
    let app = router(0, "luna");
    let (status, created) = json_call(
        &app,
        "POST",
        "/openai/v1/responses",
        Some(json!({"model": "gpt-6-luna", "input": "hello", "background": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["status"], "queued");
    assert_eq!(created["background"], true);
    let id = created["id"].as_str().unwrap().to_string();

    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_, v) = json_call(&app, "GET", &format!("/openai/v1/responses/{id}"), None).await;
    assert_eq!(v["status"], "in_progress");

    // Cancel while in progress.
    let (status, v) = json_call(
        &app,
        "POST",
        &format!("/openai/v1/responses/{id}/cancel"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "cancelled");
    let (_, v) = json_call(&app, "GET", &format!("/openai/v1/responses/{id}"), None).await;
    assert_eq!(v["status"], "cancelled");
}

#[tokio::test]
async fn background_response_completes_with_the_full_result() {
    let app = router(0, "instant");
    let (_, created) = json_call(
        &app,
        "POST",
        "/openai/v1/responses",
        Some(json!({"model": "gpt-5", "input": "hello", "background": true})),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let (_, v) = json_call(&app, "GET", &format!("/openai/v1/responses/{id}"), None).await;
    assert_eq!(v["status"], "completed", "{v}");
    assert_eq!(v["id"], id);
    assert!(!v["output_text"].as_str().unwrap().is_empty());
    assert!(v["usage"]["output_tokens"].as_u64().unwrap() > 0);

    // Completed responses cannot be cancelled; they can be deleted.
    let (status, _) = json_call(
        &app,
        "POST",
        &format!("/openai/v1/responses/{id}/cancel"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, v) = json_call(&app, "DELETE", &format!("/openai/v1/responses/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["deleted"], true);
    let (status, _) = json_call(&app, "GET", &format!("/openai/v1/responses/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn background_requires_store() {
    let (status, v) = json_call(
        &router(0, "instant"),
        "POST",
        "/openai/v1/responses",
        Some(json!({"model": "gpt-5", "input": "x", "background": true, "store": false})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"]["message"].as_str().unwrap().contains("store"));
}

// --- OpenAI batches ---

#[tokio::test]
async fn openai_batch_runs_lines_and_writes_output_files() {
    let app = router(0, "instant");
    let lines = [
        json!({"custom_id": "a", "method": "POST", "url": "/v1/chat/completions",
               "body": {"model": "gpt-5", "messages": [{"role": "user", "content": "hi"}]}}),
        json!({"custom_id": "b", "method": "POST", "url": "/v1/chat/completions",
               "body": {"model": "gpt-5", "messages": [{"role": "user", "content": "x"}],
                        "max_tokens": 999999999}}),
    ];
    let content = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let file = upload(&app, &content).await;
    assert_eq!(file["purpose"], "batch");
    let file_id = file["id"].as_str().unwrap();
    let (_, raw) = call(
        &app,
        "GET",
        &format!("/openai/v1/files/{file_id}/content"),
        None,
    )
    .await;
    assert_eq!(raw, content);

    let (status, batch) = json_call(
        &app,
        "POST",
        "/openai/v1/batches",
        Some(
            json!({"input_file_id": file_id, "endpoint": "/v1/chat/completions",
                    "completion_window": "24h", "metadata": {"job": "nightly"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["object"], "batch");
    assert_eq!(batch["status"], "validating");
    let id = batch["id"].as_str().unwrap();

    let (_, batch) = json_call(&app, "GET", &format!("/openai/v1/batches/{id}"), None).await;
    assert_eq!(batch["status"], "completed", "{batch}");
    assert_eq!(
        batch["request_counts"],
        json!({"total": 2, "completed": 1, "failed": 1})
    );
    assert_eq!(batch["metadata"]["job"], "nightly");

    let out_id = batch["output_file_id"].as_str().unwrap();
    let (_, out) = call(
        &app,
        "GET",
        &format!("/openai/v1/files/{out_id}/content"),
        None,
    )
    .await;
    let line: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(line["custom_id"], "a");
    assert_eq!(line["response"]["status_code"], 200);
    assert_eq!(line["response"]["body"]["object"], "chat.completion");

    let err_id = batch["error_file_id"].as_str().unwrap();
    let (_, err) = call(
        &app,
        "GET",
        &format!("/openai/v1/files/{err_id}/content"),
        None,
    )
    .await;
    let line: Value = serde_json::from_str(err.lines().next().unwrap()).unwrap();
    assert_eq!(line["custom_id"], "b");
    assert_eq!(line["response"]["status_code"], 400);

    let (_, list) = json_call(&app, "GET", "/openai/v1/batches", None).await;
    assert_eq!(list["data"][0]["id"], id);
}

#[tokio::test]
async fn openai_batch_validation_and_cancel() {
    let app = router(60_000, "instant");
    let (status, _) = json_call(
        &app,
        "POST",
        "/openai/v1/batches",
        Some(
            json!({"input_file_id": "file-nope", "endpoint": "/v1/chat/completions",
                    "completion_window": "24h"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let bad = upload(
        &app,
        "{not json}\n{\"custom_id\":\"x\",\"url\":\"/v1/embeddings\",\"body\":{}}",
    )
    .await;
    let (_, batch) = json_call(
        &app,
        "POST",
        "/openai/v1/batches",
        Some(
            json!({"input_file_id": bad["id"], "endpoint": "/v1/chat/completions",
                    "completion_window": "24h"}),
        ),
    )
    .await;
    assert_eq!(batch["status"], "failed");
    let codes: Vec<&str> = batch["errors"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["invalid_json_line", "mismatched_endpoint"]);

    let good = upload(
        &app,
        &json!({"custom_id": "e", "method": "POST", "url": "/v1/embeddings",
                "body": {"model": "text-embedding-3-small", "input": "hi"}})
        .to_string(),
    )
    .await;
    let (_, batch) = json_call(
        &app,
        "POST",
        "/openai/v1/batches",
        Some(
            json!({"input_file_id": good["id"], "endpoint": "/v1/embeddings",
                    "completion_window": "24h"}),
        ),
    )
    .await;
    let id = batch["id"].as_str().unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let (_, v) = json_call(&app, "GET", &format!("/openai/v1/batches/{id}"), None).await;
    assert_eq!(v["status"], "in_progress");
    let (_, v) = json_call(
        &app,
        "POST",
        &format!("/openai/v1/batches/{id}/cancel"),
        None,
    )
    .await;
    assert_eq!(v["status"], "cancelled");
    assert!(v["output_file_id"].is_null());
}

// --- Anthropic batches ---

fn anthropic_requests() -> Value {
    json!({"requests": [
        {"custom_id": "ok-1", "params": {"model": "claude-opus-4-8", "max_tokens": 64,
            "messages": [{"role": "user", "content": "hi"}]}},
        {"custom_id": "bad-1", "params": {"model": "claude-opus-4-8", "max_tokens": 99999999,
            "messages": [{"role": "user", "content": "hi"}]}}
    ]})
}

#[tokio::test]
async fn anthropic_batch_ends_with_results() {
    let app = router(0, "instant");
    let (status, batch) = json_call(
        &app,
        "POST",
        "/anthropic/v1/messages/batches",
        Some(anthropic_requests()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["type"], "message_batch");
    assert_eq!(batch["processing_status"], "in_progress");
    assert_eq!(batch["request_counts"]["processing"], 2);
    let id = batch["id"].as_str().unwrap();
    assert!(id.starts_with("msgbatch_"));

    let (_, batch) = json_call(
        &app,
        "GET",
        &format!("/anthropic/v1/messages/batches/{id}"),
        None,
    )
    .await;
    assert_eq!(batch["processing_status"], "ended");
    assert_eq!(batch["request_counts"]["succeeded"], 1);
    assert_eq!(batch["request_counts"]["errored"], 1);
    let url = batch["results_url"].as_str().unwrap();
    assert_eq!(
        url,
        format!("http://sim.test/anthropic/v1/messages/batches/{id}/results")
    );

    let (_, results) = call(
        &app,
        "GET",
        &format!("/anthropic/v1/messages/batches/{id}/results"),
        None,
    )
    .await;
    let lines: Vec<Value> = results
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["custom_id"], "ok-1");
    assert_eq!(lines[0]["result"]["type"], "succeeded");
    assert_eq!(lines[0]["result"]["message"]["type"], "message");
    assert_eq!(lines[1]["result"]["type"], "errored");
    assert_eq!(
        lines[1]["result"]["error"]["error"]["type"],
        "invalid_request_error"
    );

    let (status, _) = json_call(
        &app,
        "DELETE",
        &format!("/anthropic/v1/messages/batches/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn anthropic_batch_validation_cancel_and_early_results() {
    let app = router(60_000, "instant");
    let (status, v) = json_call(
        &app,
        "POST",
        "/anthropic/v1/messages/batches",
        Some(json!({"requests": [{"custom_id": "has space", "params": {}}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["type"], "invalid_request_error");

    let (_, batch) = json_call(
        &app,
        "POST",
        "/anthropic/v1/messages/batches",
        Some(anthropic_requests()),
    )
    .await;
    let id = batch["id"].as_str().unwrap();
    let (status, _) = call(
        &app,
        "GET",
        &format!("/anthropic/v1/messages/batches/{id}/results"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "results wait for the batch to end"
    );
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/anthropic/v1/messages/batches/{id}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "cannot delete a running batch"
    );

    let (_, v) = json_call(
        &app,
        "POST",
        &format!("/anthropic/v1/messages/batches/{id}/cancel"),
        None,
    )
    .await;
    assert_eq!(v["processing_status"], "ended");
    assert_eq!(v["request_counts"]["canceled"], 2);
    let (_, results) = call(
        &app,
        "GET",
        &format!("/anthropic/v1/messages/batches/{id}/results"),
        None,
    )
    .await;
    assert!(results.lines().all(|l| l.contains("\"canceled\"")));

    let (_, list) = json_call(&app, "GET", "/anthropic/v1/messages/batches", None).await;
    assert_eq!(list["data"][0]["id"], id);
}
