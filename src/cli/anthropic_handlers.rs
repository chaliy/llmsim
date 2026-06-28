// Anthropic Messages API HTTP Handlers
// Implements POST /anthropic/v1/messages, GET /anthropic/v1/models, and
// GET /anthropic/v1/models/:id, mirroring the Anthropic API wire format.

use super::state::AppState;
use crate::anthropic::{
    default_anthropic_model_ids, get_anthropic_model_profile, AnthropicErrorResponse,
    AnthropicModel, AnthropicModelsResponse, ContentBlock, MessagesRequest, MessagesResponse,
    MessagesStreamBuilder, StopReason, Usage,
};
use crate::ids::prefixed_compact_id;
use crate::script::{ScriptedResponse, SimError, SimToolCall, SimTurn};
use crate::{create_generator, EndpointType, ErrorInjector, LatencyProfile};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Instant;

/// Map an HTTP status code to an Axum `StatusCode`, falling back to 500.
fn status_from_code(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Build an Anthropic-shaped error response for a given status + message.
fn anthropic_error(status: u16, message: impl Into<String>) -> Response {
    let body = AnthropicErrorResponse::new(
        AnthropicErrorResponse::type_for_status(status),
        message.into(),
    );
    let mut response = Json(body).into_response();
    *response.status_mut() = status_from_code(status);
    response
}

/// Render a scripted `SimError` as an Anthropic error response.
fn sim_error_to_anthropic_response(err: &SimError) -> Response {
    anthropic_error(err.status_code(), err.message())
}

/// POST /anthropic/v1/messages
pub async fn create_message(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MessagesRequest>,
) -> Response {
    let request_start = Instant::now();

    tracing::info!(
        model = %request.model,
        stream = request.stream,
        messages = request.messages.len(),
        "Anthropic messages request"
    );

    state
        .stats
        .record_request_start(&request.model, request.stream, EndpointType::Messages);

    // Error injection (Anthropic error wire shape).
    let error_injector = ErrorInjector::new(state.config.error_config());
    if let Some(error) = error_injector.maybe_inject() {
        tracing::warn!("Injecting error: {:?}", error);
        let status_code = error.status_code();
        state.stats.record_error(status_code);

        let message = error.to_error_response().error.message;
        let mut response = anthropic_error(status_code, message);
        if let Some(retry_after) = error.retry_after() {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                retry_after.to_string().parse().unwrap(),
            );
        }
        return response;
    }

    // Model-specific latency (unless overridden in config).
    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(&request.model)
        };

    // Scripted non-streaming requests get full tool-call support.
    if let Some(script) = state.script.clone() {
        if !request.stream {
            return handle_scripted_message(state, request, request_start, script).await;
        }
    }

    // Resolve the response content (scripted text turn, or generated).
    let content = if let Some(script) = state.script.as_ref() {
        match script.next_turn() {
            ScriptedResponse::Turn(SimTurn::Assistant { text }) => text,
            ScriptedResponse::Turn(SimTurn::Mixed { text, .. }) => text,
            ScriptedResponse::Turn(SimTurn::ToolCalls { .. }) => String::new(),
            ScriptedResponse::Turn(SimTurn::Error(err)) => {
                state.stats.record_error(err.status_code());
                return sim_error_to_anthropic_response(&err);
            }
            ScriptedResponse::Exhausted => {
                state.stats.record_error(500);
                return sim_error_to_anthropic_response(&SimError::Other {
                    message: "llmsim script exhausted (on_exhausted=error)".to_string(),
                    status_code: Some(500),
                });
            }
        }
    } else {
        generate_content(&state, &request)
    };

    let input_tokens = count_input_tokens(&request);
    let output_tokens =
        crate::count_tokens_default(&content).unwrap_or(content.split_whitespace().count());
    let usage = Usage::new(input_tokens as u32, output_tokens as u32);

    if request.stream {
        let stats = state.stats.clone();
        let input_tok = usage.input_tokens;
        let output_tok = usage.output_tokens;

        let stream = MessagesStreamBuilder::new(&request.model, content)
            .latency(latency)
            .usage(usage)
            .on_complete(move || {
                stats.record_request_end(request_start.elapsed(), input_tok, output_tok);
            })
            .build();

        let body = Body::from_stream(stream.into_stream().map(Ok::<_, std::io::Error>));
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap()
    } else {
        let delay = latency.sample_ttft();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        state.stats.record_request_end(
            request_start.elapsed(),
            usage.input_tokens,
            usage.output_tokens,
        );
        let response = MessagesResponse::text(request.model.clone(), content, usage);
        Json(response).into_response()
    }
}

/// Non-streaming scripted path: emits text and/or `tool_use` content blocks,
/// matching the Anthropic wire shape with `stop_reason: "tool_use"`.
async fn handle_scripted_message(
    state: Arc<AppState>,
    request: MessagesRequest,
    request_start: Instant,
    script: Arc<crate::script::Script>,
) -> Response {
    let turn_index = script.cursor();
    let turn = match script.next_turn() {
        ScriptedResponse::Turn(t) => t,
        ScriptedResponse::Exhausted => {
            state.stats.record_error(500);
            return sim_error_to_anthropic_response(&SimError::Other {
                message: "llmsim script exhausted (on_exhausted=error)".to_string(),
                status_code: Some(500),
            });
        }
    };

    let (text, tool_calls) = match turn {
        SimTurn::Assistant { text } => (Some(text), Vec::new()),
        SimTurn::ToolCalls { calls } => (None, calls),
        SimTurn::Mixed { text, calls } => (Some(text), calls),
        SimTurn::Error(err) => {
            state.stats.record_error(err.status_code());
            return sim_error_to_anthropic_response(&err);
        }
    };

    let input_tokens = count_input_tokens(&request);
    let text_for_usage = text.clone().unwrap_or_default();
    let mut output_tokens = crate::count_tokens_default(&text_for_usage)
        .unwrap_or(text_for_usage.split_whitespace().count());

    let mut content: Vec<ContentBlock> = Vec::new();
    if let Some(t) = text {
        content.push(ContentBlock::text(t));
    }
    let has_tool_calls = !tool_calls.is_empty();
    for (i, call) in tool_calls.into_iter().enumerate() {
        output_tokens += tool_call_token_estimate(&call);
        let id = call
            .id
            .clone()
            .unwrap_or_else(|| anthropic_tool_use_id(turn_index, i));
        content.push(ContentBlock::ToolUse {
            id,
            name: call.name,
            input: call.arguments,
        });
    }

    // An empty response (e.g. a tool-call turn with no text) still needs a
    // valid content array; Anthropic always returns at least one block.
    let stop_reason = if has_tool_calls {
        StopReason::ToolUse
    } else {
        StopReason::EndTurn
    };
    if content.is_empty() {
        content.push(ContentBlock::text(""));
    }

    let usage = Usage::new(input_tokens as u32, output_tokens as u32);
    state.stats.record_request_end(
        request_start.elapsed(),
        usage.input_tokens,
        usage.output_tokens,
    );

    let response = MessagesResponse::with_content(request.model, content, stop_reason, usage);
    Json(response).into_response()
}

/// Generate response content for non-scripted requests via the configured
/// generator, reusing the OpenAI `ChatCompletionRequest` the generators accept.
fn generate_content(state: &AppState, request: &MessagesRequest) -> String {
    let generator = create_generator(
        &state.config.response.generator,
        state.config.response.target_tokens,
    );
    let prompt = request.prompt_text();
    let chat_request = crate::openai::ChatCompletionRequest {
        model: request.model.clone(),
        messages: vec![crate::openai::Message::user(&prompt)],
        temperature: request.temperature,
        top_p: request.top_p,
        n: None,
        stream: request.stream,
        stop: None,
        max_tokens: Some(request.max_tokens),
        max_completion_tokens: Some(request.max_tokens),
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: request.metadata.as_ref().and_then(|m| m.user_id.clone()),
        tools: None,
        tool_choice: None,
        response_format: None,
        seed: None,
    };
    generator.generate(&chat_request)
}

/// Count input tokens for a Messages request (prompt text + small overhead).
fn count_input_tokens(request: &MessagesRequest) -> usize {
    let text = request.prompt_text();
    let base = crate::count_tokens_default(&text).unwrap_or(text.split_whitespace().count());
    // Per-message + request framing overhead, similar to the OpenAI handler.
    base + request.messages.len() * 3 + 5
}

/// Approximate output tokens contributed by a scripted tool call.
fn tool_call_token_estimate(call: &SimToolCall) -> usize {
    let args = serde_json::to_string(&call.arguments).unwrap_or_default();
    crate::count_tokens_default(&args).unwrap_or(args.split_whitespace().count())
        + call.name.split_whitespace().count()
}

/// Generate a deterministic-ish `toolu_`-prefixed tool-use id when the script
/// did not supply one (Anthropic tool_use ids use the `toolu_` prefix).
fn anthropic_tool_use_id(_turn_index: usize, _call_index: usize) -> String {
    prefixed_compact_id("toolu_")
}

/// GET /anthropic/v1/models
pub async fn list_models(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let models: Vec<AnthropicModel> = default_anthropic_model_ids()
        .iter()
        .filter_map(|id| get_anthropic_model_profile(id))
        .map(AnthropicModel::from_profile)
        .collect();
    Json(AnthropicModelsResponse::new(models))
}

/// GET /anthropic/v1/models/:model_id
pub async fn get_model(
    State(_state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> Response {
    match get_anthropic_model_profile(&model_id) {
        Some(profile) => Json(AnthropicModel::from_profile(profile)).into_response(),
        None => anthropic_error(404, format!("model: {}", model_id)),
    }
}
