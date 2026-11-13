// Asynchronous work: background Responses, OpenAI Files and Batches, and
// Anthropic Message Batches.
//
// Decisions:
// - The work runs at creation, through the internal router (instant timing,
//   no injected errors), so batch lines and background responses get exactly
//   what the synchronous endpoints would return, scenarios and forced tools
//   included. Only the reported status waits: it is a function of the time
//   since creation, so polling sees queued, in_progress, then completed.
// - Background responses finish after the time the model would take (the
//   model's latency profile over the output); batches after
//   `[batch] completion_ms`.
// - Everything is in memory, capped at `MAX_JOBS` per kind (oldest dropped).

use super::state::AppState;
use crate::ids::{prefixed_compact_id, prefixed_id, unix_timestamp};
use axum::{
    body::{to_bytes, Body},
    extract::{Multipart, Path, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

/// Entries kept per kind before the oldest is dropped.
const MAX_JOBS: usize = 10_000;
/// How long a background response reports `queued`.
const QUEUED_FOR: Duration = Duration::from_millis(150);
/// OpenAI batch endpoints that accept lines.
const BATCH_ENDPOINTS: &[&str] = &["/v1/chat/completions", "/v1/responses", "/v1/embeddings"];

struct Timed<T> {
    created: Instant,
    value: T,
}

/// In-memory store for asynchronous work.
#[derive(Default)]
pub struct JobStore {
    responses: Mutex<HashMap<String, Timed<BgResponse>>>,
    files: Mutex<HashMap<String, Timed<StoredFile>>>,
    batches: Mutex<HashMap<String, Timed<Batch>>>,
}

fn insert<T>(map: &Mutex<HashMap<String, Timed<T>>>, id: String, value: T) {
    let mut m = map.lock().unwrap_or_else(|e| e.into_inner());
    if m.len() >= MAX_JOBS {
        if let Some(oldest) = m
            .iter()
            .min_by_key(|(_, t)| t.created)
            .map(|(k, _)| k.clone())
        {
            m.remove(&oldest);
        }
    }
    m.insert(
        id,
        Timed {
            created: Instant::now(),
            value,
        },
    );
}

/// Run one request through the internal router: (status, JSON body).
async fn dispatch(state: &AppState, uri: &str, body: &Value) -> (u16, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("valid request");
    let resp = match state.internal_router().oneshot(req).await {
        Ok(r) => r,
        Err(e) => match e {},
    };
    let status = resp.status().as_u16();
    let bytes = to_bytes(resp.into_body(), 256 * 1024 * 1024)
        .await
        .unwrap_or_default();
    let value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    (status, value)
}

fn openai_error(status: StatusCode, message: String, param: Option<&str>) -> Response {
    (
        status,
        Json(
            json!({"error": {"message": message, "type": "invalid_request_error",
                              "param": param, "code": null}}),
        ),
    )
        .into_response()
}

fn anthropic_error(status: StatusCode, kind: &str, message: String) -> Response {
    (
        status,
        Json(json!({"type": "error", "error": {"type": kind, "message": message}})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Background responses
// ---------------------------------------------------------------------------

struct BgResponse {
    /// The finished response (or an error body), with our id.
    result: Value,
    ok: bool,
    model: String,
    created_at: i64,
    ready: Instant,
    cancelled: bool,
}

fn bg_shell(id: &str, r: &BgResponse, status: &str) -> Value {
    json!({
        "id": id, "object": "response", "created_at": r.created_at, "model": r.model,
        "status": status, "background": true, "output": [], "output_text": null,
        "usage": null, "error": null, "metadata": null
    })
}

fn bg_view(id: &str, entry: &Timed<BgResponse>, now: Instant) -> Value {
    let r = &entry.value;
    if r.cancelled {
        return bg_shell(id, r, "cancelled");
    }
    if now >= r.ready {
        if r.ok {
            return r.result.clone();
        }
        let mut v = bg_shell(id, r, "failed");
        v["error"] = r.result.get("error").cloned().unwrap_or(r.result.clone());
        return v;
    }
    let queued =
        now.saturating_duration_since(entry.created) < QUEUED_FOR.min(r.ready - entry.created);
    bg_shell(id, r, if queued { "queued" } else { "in_progress" })
}

/// Start a background response: compute it now, report it when the model
/// would have finished. Returns the `queued` response.
pub(super) async fn start_background_response(
    state: &AppState,
    request: &crate::openai::ResponsesRequest,
) -> Response {
    if request.store == Some(false) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Background mode requires `store` to be true.".to_string(),
            Some("background"),
        );
    }
    let mut body = serde_json::to_value(request).unwrap_or_default();
    body["background"] = json!(false);
    body["stream"] = json!(false);
    let (status, mut result) = dispatch(state, "/openai/v1/responses", &body).await;
    let id = prefixed_id("resp_");
    let ok = status == 200;
    if ok {
        result["id"] = json!(id);
        result["background"] = json!(true);
    }
    let output_tokens = result["usage"]["output_tokens"].as_u64().unwrap_or(0);
    let input_tokens = result["usage"]["input_tokens"].as_u64().unwrap_or(0) as usize;
    let latency = state
        .config
        .latency_for_model(&request.model)
        .with_input_tokens(input_tokens);
    let duration = latency.sample_ttft() + latency.sample_tbt() * output_tokens as u32;
    let bg = BgResponse {
        result,
        ok,
        model: request.model.clone(),
        created_at: unix_timestamp(),
        ready: Instant::now() + duration,
        cancelled: false,
    };
    let shell = bg_shell(&id, &bg, "queued");
    insert(&state.jobs.responses, id, bg);
    Json(shell).into_response()
}

fn response_not_found(id: &str) -> Response {
    openai_error(
        StatusCode::NOT_FOUND,
        format!("Response with id '{id}' not found."),
        None,
    )
}

/// GET /openai/v1/responses/{id}
pub async fn get_response(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let map = state
        .jobs
        .responses
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match map.get(&id) {
        Some(entry) => Json(bg_view(&id, entry, Instant::now())).into_response(),
        None => response_not_found(&id),
    }
}

/// POST /openai/v1/responses/{id}/cancel
pub async fn cancel_response(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let mut map = state
        .jobs
        .responses
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(entry) = map.get_mut(&id) else {
        return response_not_found(&id);
    };
    if !entry.value.cancelled && Instant::now() >= entry.value.ready {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Cannot cancel a completed response.".to_string(),
            None,
        );
    }
    entry.value.cancelled = true;
    Json(bg_view(&id, entry, Instant::now())).into_response()
}

/// DELETE /openai/v1/responses/{id}
pub async fn delete_response(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let mut map = state
        .jobs
        .responses
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match map.remove(&id) {
        Some(_) => {
            Json(json!({"id": id, "object": "response.deleted", "deleted": true})).into_response()
        }
        None => response_not_found(&id),
    }
}

// ---------------------------------------------------------------------------
// OpenAI Files
// ---------------------------------------------------------------------------

struct StoredFile {
    meta: Value,
    content: Vec<u8>,
}

fn store_file(state: &AppState, filename: &str, purpose: &str, content: Vec<u8>) -> Value {
    let id = prefixed_compact_id("file-");
    let meta = json!({
        "id": id, "object": "file", "bytes": content.len(), "created_at": unix_timestamp(),
        "filename": filename, "purpose": purpose, "status": "processed", "expires_at": null,
        "status_details": null
    });
    insert(
        &state.jobs.files,
        id,
        StoredFile {
            meta: meta.clone(),
            content,
        },
    );
    meta
}

fn file_not_found(id: &str) -> Response {
    openai_error(
        StatusCode::NOT_FOUND,
        format!("No such File object: {id}"),
        Some("id"),
    )
}

/// POST /openai/v1/files (multipart: `file`, `purpose`)
pub async fn upload_file(State(state): State<Arc<AppState>>, mut form: Multipart) -> Response {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut purpose: Option<String> = None;
    while let Ok(Some(field)) = form.next_field().await {
        match field.name() {
            Some("file") => {
                let name = field.file_name().unwrap_or("upload.jsonl").to_string();
                match field.bytes().await {
                    Ok(b) => file = Some((name, b.to_vec())),
                    Err(e) => {
                        return openai_error(StatusCode::BAD_REQUEST, e.to_string(), Some("file"))
                    }
                }
            }
            Some("purpose") => purpose = field.text().await.ok(),
            _ => {}
        }
    }
    let Some((name, content)) = file else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Missing required parameter: 'file'.".to_string(),
            Some("file"),
        );
    };
    let Some(purpose) = purpose else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Missing required parameter: 'purpose'.".to_string(),
            Some("purpose"),
        );
    };
    Json(store_file(&state, &name, &purpose, content)).into_response()
}

/// GET /openai/v1/files/{id}
pub async fn get_file(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let map = state.jobs.files.lock().unwrap_or_else(|e| e.into_inner());
    match map.get(&id) {
        Some(f) => Json(f.value.meta.clone()).into_response(),
        None => file_not_found(&id),
    }
}

/// GET /openai/v1/files/{id}/content
pub async fn get_file_content(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let map = state.jobs.files.lock().unwrap_or_else(|e| e.into_inner());
    match map.get(&id) {
        Some(f) => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            f.value.content.clone(),
        )
            .into_response(),
        None => file_not_found(&id),
    }
}

/// DELETE /openai/v1/files/{id}
pub async fn delete_file(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let mut map = state.jobs.files.lock().unwrap_or_else(|e| e.into_inner());
    match map.remove(&id) {
        Some(_) => Json(json!({"id": id, "object": "file", "deleted": true})).into_response(),
        None => file_not_found(&id),
    }
}

// ---------------------------------------------------------------------------
// Batches (shared)
// ---------------------------------------------------------------------------

enum BatchKind {
    OpenAi {
        endpoint: String,
        input_file_id: String,
        output_file_id: Option<String>,
        error_file_id: Option<String>,
        /// Validation errors: the batch failed before running.
        errors: Option<Value>,
        metadata: Value,
    },
    Anthropic {
        /// One JSONL result line per request, in request order.
        results: Vec<String>,
        custom_ids: Vec<String>,
        results_base: String,
    },
}

struct Batch {
    kind: BatchKind,
    created_at: i64,
    ready: Instant,
    total: usize,
    succeeded: usize,
    failed: usize,
    cancelled_at: Option<i64>,
}

fn ts_after(created_at: i64, created: Instant, at: Instant) -> i64 {
    created_at + at.saturating_duration_since(created).as_secs() as i64
}

// ---------------------------------------------------------------------------
// OpenAI Batches
// ---------------------------------------------------------------------------

fn openai_batch_view(id: &str, entry: &Timed<Batch>, now: Instant) -> Value {
    let b = &entry.value;
    let BatchKind::OpenAi {
        endpoint,
        input_file_id,
        output_file_id,
        error_file_id,
        errors,
        metadata,
    } = &b.kind
    else {
        return Value::Null;
    };
    let validated = entry.created + (b.ready - entry.created).min(Duration::from_millis(200));
    let done = now >= b.ready;
    let status = if errors.is_some() {
        "failed"
    } else if b.cancelled_at.is_some() {
        "cancelled"
    } else if done {
        "completed"
    } else if now < validated {
        "validating"
    } else {
        "in_progress"
    };
    let at = |i: Instant| ts_after(b.created_at, entry.created, i);
    let completed = status == "completed";
    json!({
        "id": id, "object": "batch", "endpoint": endpoint, "errors": errors,
        "input_file_id": input_file_id, "completion_window": "24h", "status": status,
        "output_file_id": if completed { json!(output_file_id) } else { Value::Null },
        "error_file_id": if completed { json!(error_file_id) } else { Value::Null },
        "created_at": b.created_at,
        "in_progress_at": if errors.is_none() && now >= validated { json!(at(validated)) } else { Value::Null },
        "expires_at": b.created_at + 86_400,
        "finalizing_at": if completed { json!(at(b.ready)) } else { Value::Null },
        "completed_at": if completed { json!(at(b.ready)) } else { Value::Null },
        "failed_at": if errors.is_some() { json!(b.created_at) } else { Value::Null },
        "expired_at": null,
        "cancelling_at": b.cancelled_at,
        "cancelled_at": b.cancelled_at,
        "request_counts": {
            "total": b.total,
            "completed": if completed { b.succeeded } else { 0 },
            "failed": if completed { b.failed } else { 0 },
        },
        "metadata": metadata,
    })
}

/// POST /openai/v1/batches
pub async fn create_openai_batch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let Some(input_file_id) = body["input_file_id"].as_str().map(str::to_string) else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Missing required parameter: 'input_file_id'.".to_string(),
            Some("input_file_id"),
        );
    };
    let endpoint = body["endpoint"].as_str().unwrap_or_default().to_string();
    if !BATCH_ENDPOINTS.contains(&endpoint.as_str()) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid value: '{endpoint}'. Supported values are: {}.",
                BATCH_ENDPOINTS.join(", ")
            ),
            Some("endpoint"),
        );
    }
    if body["completion_window"].as_str() != Some("24h") {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Invalid value for 'completion_window': only '24h' is supported.".to_string(),
            Some("completion_window"),
        );
    }
    let content = {
        let map = state.jobs.files.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(&input_file_id) {
            Some(f) => f.value.content.clone(),
            None => return file_not_found(&input_file_id),
        }
    };

    // Validate every line before running any.
    let text = String::from_utf8_lossy(&content);
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let line_no = n + 1;
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v["url"].as_str() != Some(endpoint.as_str()) => errors.push(json!({
                "code": "mismatched_endpoint", "line": line_no, "param": "url",
                "message": format!("The url for line {line_no} does not match the batch endpoint ({endpoint}).")
            })),
            Ok(v) if v["custom_id"].as_str().is_none() => errors.push(json!({
                "code": "missing_required_parameter", "line": line_no, "param": "custom_id",
                "message": format!("Line {line_no} is missing 'custom_id'.")
            })),
            Ok(v) => lines.push(v),
            Err(_) => errors.push(json!({
                "code": "invalid_json_line", "line": line_no, "param": null,
                "message": format!("Line {line_no} is not valid JSON.")
            })),
        }
    }
    if lines.is_empty() && errors.is_empty() {
        errors.push(json!({"code": "empty_file", "line": null, "param": null,
                           "message": "The input file is empty."}));
    }

    let id = prefixed_compact_id("batch_");
    let created_at = unix_timestamp();
    let metadata = body.get("metadata").cloned().unwrap_or(Value::Null);
    let mut batch = Batch {
        kind: BatchKind::OpenAi {
            endpoint: endpoint.clone(),
            input_file_id,
            output_file_id: None,
            error_file_id: None,
            errors: None,
            metadata,
        },
        created_at,
        ready: Instant::now() + Duration::from_millis(state.config.batch.completion_ms),
        total: lines.len(),
        succeeded: 0,
        failed: 0,
        cancelled_at: None,
    };
    if !errors.is_empty() {
        if let BatchKind::OpenAi { errors: e, .. } = &mut batch.kind {
            *e = Some(json!({"object": "list", "data": errors}));
        }
    } else {
        let uri = format!("/openai{endpoint}");
        let mut out = String::new();
        let mut err = String::new();
        for line in &lines {
            let mut req = line["body"].clone();
            if req.is_object() {
                req["stream"] = json!(false);
            }
            let (status, resp) = dispatch(&state, &uri, &req).await;
            let entry = json!({
                "id": prefixed_compact_id("batch_req_"),
                "custom_id": line["custom_id"],
                "response": {"status_code": status, "request_id": prefixed_compact_id("req_"), "body": resp},
                "error": null
            });
            let target = if status == 200 {
                batch.succeeded += 1;
                &mut out
            } else {
                batch.failed += 1;
                &mut err
            };
            target.push_str(&entry.to_string());
            target.push('\n');
        }
        let stem = format!("{id}_output.jsonl");
        let output_id = (!out.is_empty())
            .then(|| store_file(&state, &stem, "batch_output", out.into_bytes())["id"].clone());
        let error_id = (!err.is_empty()).then(|| {
            store_file(
                &state,
                &format!("{id}_error.jsonl"),
                "batch_output",
                err.into_bytes(),
            )["id"]
                .clone()
        });
        if let BatchKind::OpenAi {
            output_file_id,
            error_file_id,
            ..
        } = &mut batch.kind
        {
            *output_file_id = output_id.and_then(|v| v.as_str().map(str::to_string));
            *error_file_id = error_id.and_then(|v| v.as_str().map(str::to_string));
        }
    }
    let entry = Timed {
        created: Instant::now(),
        value: batch,
    };
    // The create response always shows the first state.
    let mut view = openai_batch_view(&id, &entry, entry.created);
    if view["status"] != "failed" {
        view["status"] = json!("validating");
        for k in [
            "output_file_id",
            "error_file_id",
            "in_progress_at",
            "finalizing_at",
            "completed_at",
        ] {
            view[k] = Value::Null;
        }
        view["request_counts"] = json!({"total": entry.value.total, "completed": 0, "failed": 0});
    }
    insert(&state.jobs.batches, id, entry.value);
    Json(view).into_response()
}

fn batch_not_found_openai(id: &str) -> Response {
    openai_error(
        StatusCode::NOT_FOUND,
        format!("No batch found with id '{id}'."),
        None,
    )
}

/// GET /openai/v1/batches/{id}
pub async fn get_openai_batch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    match map.get(&id) {
        Some(e) if matches!(e.value.kind, BatchKind::OpenAi { .. }) => {
            Json(openai_batch_view(&id, e, Instant::now())).into_response()
        }
        _ => batch_not_found_openai(&id),
    }
}

/// POST /openai/v1/batches/{id}/cancel
pub async fn cancel_openai_batch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let mut map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    let Some(e) = map.get_mut(&id) else {
        return batch_not_found_openai(&id);
    };
    if !matches!(e.value.kind, BatchKind::OpenAi { .. }) {
        return batch_not_found_openai(&id);
    }
    let now = Instant::now();
    let status = openai_batch_view(&id, e, now)["status"].clone();
    if status == "validating" || status == "in_progress" {
        e.value.cancelled_at = Some(unix_timestamp());
    } else if status != "cancelled" {
        return openai_error(
            StatusCode::CONFLICT,
            format!(
                "Cannot cancel a batch with status '{}'.",
                status.as_str().unwrap_or("")
            ),
            None,
        );
    }
    Json(openai_batch_view(&id, e, now)).into_response()
}

/// GET /openai/v1/batches
pub async fn list_openai_batches(State(state): State<Arc<AppState>>) -> Response {
    let map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let mut items: Vec<(&String, &Timed<Batch>)> = map
        .iter()
        .filter(|(_, e)| matches!(e.value.kind, BatchKind::OpenAi { .. }))
        .collect();
    items.sort_by_key(|(_, e)| std::cmp::Reverse(e.created));
    let data: Vec<Value> = items
        .iter()
        .map(|(id, e)| openai_batch_view(id, e, now))
        .collect();
    Json(json!({
        "object": "list",
        "first_id": data.first().map(|d| d["id"].clone()),
        "last_id": data.last().map(|d| d["id"].clone()),
        "has_more": false,
        "data": data,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Anthropic Message Batches
// ---------------------------------------------------------------------------

fn anthropic_batch_view(id: &str, entry: &Timed<Batch>, now: Instant) -> Value {
    let b = &entry.value;
    let BatchKind::Anthropic { results_base, .. } = &b.kind else {
        return Value::Null;
    };
    let ended = b.cancelled_at.is_some() || now >= b.ready;
    let iso = crate::anthropic::iso8601_utc;
    let ended_at = match b.cancelled_at {
        Some(t) => Some(t),
        None if ended => Some(ts_after(b.created_at, entry.created, b.ready)),
        None => None,
    };
    let (processing, succeeded, errored, canceled) = match (b.cancelled_at, ended) {
        (Some(_), _) => (0, 0, 0, b.total),
        (None, true) => (0, b.succeeded, b.failed, 0),
        (None, false) => (b.total, 0, 0, 0),
    };
    json!({
        "id": id, "type": "message_batch",
        "processing_status": if ended { "ended" } else { "in_progress" },
        "request_counts": {"processing": processing, "succeeded": succeeded,
                           "errored": errored, "canceled": canceled, "expired": 0},
        "ended_at": ended_at.map(iso),
        "created_at": iso(b.created_at),
        "expires_at": iso(b.created_at + 86_400),
        "archived_at": null,
        "cancel_initiated_at": b.cancelled_at.map(iso),
        "results_url": ended.then(|| format!("{results_base}/{id}/results")),
    })
}

fn valid_custom_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// POST /anthropic/v1/messages/batches
pub async fn create_anthropic_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(requests) = body["requests"].as_array().filter(|r| !r.is_empty()) else {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "requests: List should have at least 1 item".to_string(),
        );
    };
    if requests.len() > 100_000 {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "requests: List should have at most 100000 items".to_string(),
        );
    }
    let mut seen = std::collections::HashSet::new();
    for (i, r) in requests.iter().enumerate() {
        let cid = r["custom_id"].as_str().unwrap_or_default();
        if !valid_custom_id(cid) {
            return anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                format!(
                    "requests.{i}.custom_id: String should match pattern '^[a-zA-Z0-9_-]{{1,64}}$'"
                ),
            );
        }
        if !seen.insert(cid) {
            return anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                format!("Duplicate custom_id: {cid}"),
            );
        }
        if !r["params"].is_object() {
            return anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                format!("requests.{i}.params: Field required"),
            );
        }
    }

    let mut results = Vec::with_capacity(requests.len());
    let mut custom_ids = Vec::with_capacity(requests.len());
    let (mut succeeded, mut failed) = (0, 0);
    for r in requests {
        let mut params = r["params"].clone();
        params["stream"] = json!(false);
        let (status, resp) = dispatch(&state, "/anthropic/v1/messages", &params).await;
        let result = if status == 200 {
            succeeded += 1;
            json!({"type": "succeeded", "message": resp})
        } else {
            failed += 1;
            json!({"type": "errored", "error": resp})
        };
        custom_ids.push(r["custom_id"].as_str().unwrap_or_default().to_string());
        results.push(json!({"custom_id": r["custom_id"], "result": result}).to_string());
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:8080");
    let id = prefixed_compact_id("msgbatch_");
    let entry = Timed {
        created: Instant::now(),
        value: Batch {
            kind: BatchKind::Anthropic {
                results,
                custom_ids,
                results_base: format!("http://{host}/anthropic/v1/messages/batches"),
            },
            created_at: unix_timestamp(),
            ready: Instant::now() + Duration::from_millis(state.config.batch.completion_ms),
            total: requests.len(),
            succeeded,
            failed,
            cancelled_at: None,
        },
    };
    // The create response always shows the batch in progress.
    let mut view = anthropic_batch_view(&id, &entry, entry.created);
    view["processing_status"] = json!("in_progress");
    view["ended_at"] = Value::Null;
    view["results_url"] = Value::Null;
    view["request_counts"] = json!({"processing": entry.value.total, "succeeded": 0,
                                    "errored": 0, "canceled": 0, "expired": 0});
    insert(&state.jobs.batches, id, entry.value);
    Json(view).into_response()
}

fn batch_not_found_anthropic(id: &str) -> Response {
    anthropic_error(
        StatusCode::NOT_FOUND,
        "not_found_error",
        format!("Message batch {id} not found"),
    )
}

/// GET /anthropic/v1/messages/batches/{id}
pub async fn get_anthropic_batch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    match map.get(&id) {
        Some(e) if matches!(e.value.kind, BatchKind::Anthropic { .. }) => {
            Json(anthropic_batch_view(&id, e, Instant::now())).into_response()
        }
        _ => batch_not_found_anthropic(&id),
    }
}

/// POST /anthropic/v1/messages/batches/{id}/cancel
pub async fn cancel_anthropic_batch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let mut map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    match map.get_mut(&id) {
        Some(e) if matches!(e.value.kind, BatchKind::Anthropic { .. }) => {
            let now = Instant::now();
            if e.value.cancelled_at.is_none() && now < e.value.ready {
                e.value.cancelled_at = Some(unix_timestamp());
            }
            Json(anthropic_batch_view(&id, e, now)).into_response()
        }
        _ => batch_not_found_anthropic(&id),
    }
}

/// GET /anthropic/v1/messages/batches/{id}/results (JSONL)
pub async fn anthropic_batch_results(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    let Some(e) = map.get(&id) else {
        return batch_not_found_anthropic(&id);
    };
    let BatchKind::Anthropic {
        results,
        custom_ids,
        ..
    } = &e.value.kind
    else {
        return batch_not_found_anthropic(&id);
    };
    if e.value.cancelled_at.is_none() && Instant::now() < e.value.ready {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("Message batch {id} is still processing; results are not available yet"),
        );
    }
    let body: String = if e.value.cancelled_at.is_some() {
        custom_ids
            .iter()
            .map(|c| json!({"custom_id": c, "result": {"type": "canceled"}}).to_string() + "\n")
            .collect()
    } else {
        results.iter().map(|l| format!("{l}\n")).collect()
    };
    ([(header::CONTENT_TYPE, "application/x-jsonl")], body).into_response()
}

/// GET /anthropic/v1/messages/batches
pub async fn list_anthropic_batches(State(state): State<Arc<AppState>>) -> Response {
    let map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let mut items: Vec<(&String, &Timed<Batch>)> = map
        .iter()
        .filter(|(_, e)| matches!(e.value.kind, BatchKind::Anthropic { .. }))
        .collect();
    items.sort_by_key(|(_, e)| std::cmp::Reverse(e.created));
    let data: Vec<Value> = items
        .iter()
        .map(|(id, e)| anthropic_batch_view(id, e, now))
        .collect();
    Json(json!({
        "data": data,
        "has_more": false,
        "first_id": items.first().map(|(id, _)| id.as_str()),
        "last_id": items.last().map(|(id, _)| id.as_str()),
    }))
    .into_response()
}

/// DELETE /anthropic/v1/messages/batches/{id}
pub async fn delete_anthropic_batch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let mut map = state.jobs.batches.lock().unwrap_or_else(|e| e.into_inner());
    let ended = match map.get(&id) {
        Some(e) if matches!(e.value.kind, BatchKind::Anthropic { .. }) => {
            e.value.cancelled_at.is_some() || Instant::now() >= e.value.ready
        }
        _ => return batch_not_found_anthropic(&id),
    };
    if !ended {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "Message batch must be ended before it can be deleted; cancel it first".to_string(),
        );
    }
    map.remove(&id);
    Json(json!({"id": id, "type": "message_batch_deleted"})).into_response()
}
