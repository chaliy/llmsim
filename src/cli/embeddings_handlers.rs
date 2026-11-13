// POST /openai/v1/embeddings: deterministic, similarity-preserving vectors
// (see `crate::embeddings`) with OpenAI's limits and error messages.

use super::state::AppState;
use crate::embeddings::{
    embed, embedding_model, to_base64, EmbeddingRequest, DEFAULT_MODEL, MAX_INPUTS,
    MAX_INPUT_TOKENS, MAX_REQUEST_TOKENS,
};
use crate::openai::{ErrorDetail, ErrorResponse};
use crate::{EndpointType, ErrorInjector};
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

fn invalid(
    status: StatusCode,
    message: String,
    param: Option<&str>,
    code: Option<&str>,
) -> Response {
    let body = ErrorResponse {
        error: ErrorDetail {
            message,
            error_type: "invalid_request_error".to_string(),
            param: param.map(str::to_string),
            code: code.map(str::to_string),
        },
    };
    (status, Json(body)).into_response()
}

fn bad(message: String, param: Option<&str>) -> Response {
    invalid(StatusCode::BAD_REQUEST, message, param, None)
}

/// POST /openai/v1/embeddings
pub async fn create_embeddings(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EmbeddingRequest>,
) -> Response {
    let start = Instant::now();
    state
        .stats
        .record_request_start(&request.model, false, EndpointType::Embeddings);

    if let Some(error) = ErrorInjector::new(state.config.error_config()).maybe_inject() {
        state.stats.record_error(error.status_code());
        let status =
            StatusCode::from_u16(error.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(error.to_error_response())).into_response();
        if let Some(retry_after) = error.retry_after() {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, retry_after.into());
        }
        return response;
    }

    let response = match validate(&state, &request) {
        Ok(r) => r,
        Err(response) => {
            state.stats.record_error(response.status().as_u16());
            return response;
        }
    };
    let (items, dims) = response;
    let tokens: usize = items.iter().map(|(_, n)| n).sum();

    let latency = state
        .config
        .latency_for_model(&request.model)
        .with_input_tokens(tokens);
    let delay = latency.sample_ttft();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    let base64 = request.encoding_format.as_deref() == Some("base64");
    let data: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(index, (text, _))| {
            let v = embed(text, dims);
            let embedding = if base64 {
                json!(to_base64(&v))
            } else {
                json!(v)
            };
            json!({"object": "embedding", "index": index, "embedding": embedding})
        })
        .collect();
    state
        .stats
        .record_request_end(start.elapsed(), tokens as u32, 0);
    Json(json!({
        "object": "list",
        "data": data,
        "model": request.model,
        "usage": {"prompt_tokens": tokens, "total_tokens": tokens}
    }))
    .into_response()
}

type Validated = (Vec<(String, usize)>, usize);

// Errors are rendered once and returned whole, as elsewhere in the handlers.
#[allow(clippy::result_large_err)]
fn validate(state: &AppState, request: &EmbeddingRequest) -> Result<Validated, Response> {
    let model = match embedding_model(&request.model) {
        Some(m) => m,
        None if state.config.validation.strict_models => {
            return Err(invalid(
                StatusCode::NOT_FOUND,
                format!(
                    "The model `{}` does not exist or you do not have access to it.",
                    request.model
                ),
                None,
                Some("model_not_found"),
            ))
        }
        None => DEFAULT_MODEL,
    };
    if let Some(f) = request.encoding_format.as_deref() {
        if f != "float" && f != "base64" {
            return Err(bad(
                format!("Invalid value: '{f}'. Supported values are: 'float' and 'base64'."),
                Some("encoding_format"),
            ));
        }
    }
    let dims = match request.dimensions {
        None => model.dimensions,
        Some(_) if !model.shortenable => {
            return Err(bad(
                "This model does not support specifying dimensions.".to_string(),
                Some("dimensions"),
            ))
        }
        Some(d) if d < 1 => {
            return Err(bad(
                format!("Invalid value for 'dimensions' = {d}. Must be greater than 0."),
                Some("dimensions"),
            ))
        }
        Some(d) if d as usize > model.dimensions => {
            return Err(bad(
                format!(
                    "This model's maximum embedding dimensions is {}. However, you requested {d}.",
                    model.dimensions
                ),
                Some("dimensions"),
            ))
        }
        Some(d) => d as usize,
    };

    let items = request.input.items();
    if items.is_empty() || items.iter().any(|(t, n)| t.is_empty() || *n == 0) {
        return Err(bad(
            "'$.input' is invalid. Please check the API reference: \
             https://platform.openai.com/docs/api-reference."
                .to_string(),
            Some("input"),
        ));
    }
    if items.len() > MAX_INPUTS {
        return Err(bad(
            format!(
                "'$.input' is too long: {} items. The maximum is {MAX_INPUTS}.",
                items.len()
            ),
            Some("input"),
        ));
    }
    if let Some((_, n)) = items.iter().find(|(_, n)| *n > MAX_INPUT_TOKENS) {
        return Err(bad(
            format!(
                "This model's maximum context length is {MAX_INPUT_TOKENS} tokens, however you \
                 requested {n} tokens ({n} in your prompt; 0 for the completion). Please reduce \
                 your prompt; or completion length."
            ),
            None,
        ));
    }
    let total: usize = items.iter().map(|(_, n)| n).sum();
    if total > MAX_REQUEST_TOKENS {
        return Err(bad(
            format!("Requested {total} tokens, max {MAX_REQUEST_TOKENS} tokens per request"),
            Some("input"),
        ));
    }
    Ok((items, dims))
}
