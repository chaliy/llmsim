// TypeSafe System One HTTP Handlers
// Implements POST /typesafe/v1/systemone and GET /typesafe/v1/models,
// mirroring the TypeSafe API wire format (https://docs.typesafe.ai/api).
//
// Decision: System One returns one JSON verdict per request with no streaming,
// so latency is a single time-to-first-token sample. Scripted mode and
// scenarios produce chat turns and do not apply here; error injection does.

use super::state::AppState;
use crate::typesafe::{
    answer_all, resolve_typesafe_model, typesafe_models, QuestionKind, SystemOneRequest,
    SystemOneResponse, SystemOneUsage, TypeSafeErrorResponse,
};
use crate::{EndpointType, ErrorInjector, LatencyProfile};
use axum::{
    body::Bytes,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

/// Output tokens per answered question (the API reference's example reports
/// 20 for a single noul). Output tokens are free on the real API.
const OUTPUT_TOKENS_PER_QUESTION: usize = 20;
/// Fixed per-request and per-question framing overhead on input tokens.
const REQUEST_OVERHEAD_TOKENS: usize = 8;
const QUESTION_OVERHEAD_TOKENS: usize = 6;

fn typesafe_error(status: u16, message: impl Into<String>) -> Response {
    let body = TypeSafeErrorResponse::new(TypeSafeErrorResponse::type_for_status(status), message);
    let mut response = Json(body).into_response();
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    response
}

/// Count a request rejected before it was parsed. `record_error` closes an
/// in-flight request, so the start is recorded first to keep the active-request
/// gauge balanced.
fn record_rejected(state: &AppState, model: &str) {
    state
        .stats
        .record_request_start(model, false, EndpointType::SystemOne);
    state.stats.record_error(422);
}

fn unprocessable(body: impl serde::Serialize) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

/// POST /typesafe/v1/systemone
pub async fn systemone(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request_start = Instant::now();

    let raw: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            record_rejected(&state, "unknown");
            return unprocessable(serde_json::json!({
                "detail": [{
                    "type": "json_invalid",
                    "loc": ["body", e.column()],
                    "msg": "JSON decode error",
                    "input": {},
                    "ctx": {"error": e.to_string()},
                }]
            }));
        }
    };
    let request = match SystemOneRequest::parse(&raw) {
        Ok(r) => r,
        Err(errors) => {
            record_rejected(&state, raw["model"].as_str().unwrap_or("unknown"));
            return unprocessable(errors);
        }
    };

    tracing::info!(
        model = %request.model,
        questions = request.questions.len(),
        "TypeSafe systemone request"
    );
    state
        .stats
        .record_request_start(&request.model, false, EndpointType::SystemOne);

    let error_injector = ErrorInjector::new(state.config.error_config());
    if let Some(error) = error_injector.maybe_inject() {
        tracing::warn!("Injecting error: {:?}", error);
        let status_code = error.status_code();
        state.stats.record_error(status_code);
        let message = error.to_error_response().error.message;
        let mut response = typesafe_error(status_code, message);
        if let Some(retry_after) = error.retry_after() {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                retry_after.to_string().parse().unwrap(),
            );
        }
        return response;
    }

    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(&request.model)
        };

    let answers = answer_all(&request);
    let usage = SystemOneUsage {
        input_tokens: count_input_tokens(&request) as u32,
        output_tokens: (request.questions.len() * OUTPUT_TOKENS_PER_QUESTION) as u32,
    };

    let delay = latency.sample_ttft();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    state.stats.record_request_end(
        request_start.elapsed(),
        usage.input_tokens,
        usage.output_tokens,
    );

    Json(SystemOneResponse {
        model: resolve_typesafe_model(&request.model).to_string(),
        answers,
        usage,
    })
    .into_response()
}

/// Render free-form content (string, object, array) as the text it bills as.
fn content_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn tokens(text: &str) -> usize {
    crate::count_tokens_default(text).unwrap_or(text.split_whitespace().count())
}

/// Input tokens: the state, every question's instructions and criteria, plus
/// framing overhead. Real billing is per input token.
fn count_input_tokens(request: &SystemOneRequest) -> usize {
    let mut total = tokens(&content_text(&request.state)) + REQUEST_OVERHEAD_TOKENS;
    for question in request.questions.values() {
        total += QUESTION_OVERHEAD_TOKENS;
        if let Some(instructions) = &question.instructions {
            total += tokens(&content_text(instructions));
        }
        total += match &question.kind {
            QuestionKind::Noul { criteria } => criteria
                .iter()
                .flat_map(|c| c.values())
                .filter(|v| !v.is_null())
                .map(|v| tokens(&content_text(v)))
                .sum::<usize>(),
            QuestionKind::Choice { options } => options
                .iter()
                .map(|(name, v)| {
                    tokens(name)
                        + if v.is_null() {
                            0
                        } else {
                            tokens(&content_text(v))
                        }
                })
                .sum(),
            QuestionKind::Score { levels } => levels.iter().map(|v| tokens(&content_text(v))).sum(),
        };
    }
    total
}

/// GET /typesafe/v1/models
pub async fn list_models(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(typesafe_models())
}
