// Capture test traffic only when enabled. Observe bytes as the handler reads
// them so an oversized or rejected request keeps its normal API behavior.
// A reset drops log entries still in flight when the reset started.

use super::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, uri::Authority, HeaderMap, Method, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

const MAX_CALLS: usize = 500;
const MAX_BODY: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct CallLog {
    state: Mutex<Log>,
}

#[derive(Default)]
struct Log {
    calls: VecDeque<Call>,
    generation: u64,
}

#[derive(Clone, Serialize)]
struct Call {
    path: String,
    status: u16,
    scenario: Option<String>,
    step: Option<u32>,
    attempt: Option<u32>,
    body: Option<Value>,
    body_truncated: bool,
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Capture {
    fn push(&mut self, chunk: &[u8]) {
        if self.truncated {
            return;
        }
        if chunk.len() > MAX_BODY - self.bytes.len() {
            self.bytes.clear();
            self.truncated = true;
        } else {
            self.bytes.extend_from_slice(chunk);
        }
    }
}

fn generation_path(path: &str) -> bool {
    matches!(
        path,
        "/openai/v1/chat/completions"
            | "/openai/v1/responses"
            | "/openresponses/v1/responses"
            | "/anthropic/v1/messages"
    ) || path
        .strip_prefix("/gemini/v1beta/models/")
        .and_then(|model_action| model_action.split_once(':'))
        .is_some_and(|(_, action)| matches!(action, "generateContent" | "streamGenerateContent"))
}

pub(super) async fn capture(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(log) = &state.call_log else {
        return next.run(request).await;
    };
    if request.method() != Method::POST || !generation_path(request.uri().path()) {
        return next.run(request).await;
    }
    let generation = log
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .generation;
    let path = request.uri().path().to_string();
    let (parts, body) = request.into_parts();
    let captured = Arc::new(Mutex::new(Capture::default()));
    let observed = captured.clone();
    let body = Body::from_stream(body.into_data_stream().inspect(move |chunk| {
        if let Ok(chunk) = chunk {
            observed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(chunk);
        }
    }));
    let response = next.run(Request::from_parts(parts, body)).await;
    let captured = std::mem::take(&mut *captured.lock().unwrap_or_else(|e| e.into_inner()));
    let read_header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let call = Call {
        path,
        status: response.status().as_u16(),
        scenario: read_header("x-llmsim-scenario").map(str::to_string),
        step: read_header("x-llmsim-step").and_then(|value| value.parse().ok()),
        attempt: read_header("x-llmsim-attempt").and_then(|value| value.parse().ok()),
        body: serde_json::from_slice(&captured.bytes).ok(),
        body_truncated: captured.truncated,
    };
    let mut log = log.state.lock().unwrap_or_else(|e| e.into_inner());
    if log.generation == generation {
        if log.calls.len() == MAX_CALLS {
            log.calls.pop_front();
        }
        log.calls.push_back(call);
    }
    response
}

pub(super) fn is_loopback_ip(host: &str) -> bool {
    host.trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

// A DNS name controlled by another party must not become a local origin by
// resolving to loopback. CLI and SDK requests use a loopback Host and no Origin.
fn allow_control(uri: &Uri, headers: &HeaderMap) -> bool {
    if headers.contains_key(header::ORIGIN) {
        return false;
    }
    let host_header = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Authority>().ok());
    uri.authority()
        .or(host_header.as_ref())
        .is_some_and(|authority| {
            let host = authority.host();
            host.eq_ignore_ascii_case("localhost") || is_loopback_ip(host)
        })
}

pub(super) async fn calls(State(state): State<Arc<AppState>>, request: Request) -> Response {
    if !allow_control(request.uri(), request.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(log) = &state.call_log else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let calls = log
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .calls
        .clone();
    ([(header::CACHE_CONTROL, "no-store")], Json(calls)).into_response()
}

pub(super) async fn reset(State(state): State<Arc<AppState>>, request: Request) -> Response {
    if !allow_control(request.uri(), request.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(log) = &state.call_log else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut log = log.state.lock().unwrap_or_else(|e| e.into_inner());
    log.calls.clear();
    log.generation = log.generation.wrapping_add(1);
    state.attempts.clear();
    StatusCode::NO_CONTENT.into_response()
}
