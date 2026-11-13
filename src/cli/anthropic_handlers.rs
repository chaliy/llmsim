// Anthropic Messages API HTTP Handlers
// Implements POST /anthropic/v1/messages, POST /anthropic/v1/messages/count_tokens,
// GET /anthropic/v1/models, and GET /anthropic/v1/models/:id, mirroring the
// Anthropic API wire format.
//
// Decision: scripted turns never carry a thinking block, so scripts replay
// exactly what they declare; generated responses follow the model's thinking
// defaults (see `crate::anthropic::thinking`).

use super::state::AppState;
use crate::anthropic::{
    default_anthropic_model_ids, get_anthropic_model_profile, thinking, AnthropicErrorResponse,
    AnthropicModel, AnthropicModelsResponse, ContentBlock, CountTokensRequest, CountTokensResponse,
    MessagesRequest, MessagesResponse, MessagesStreamBuilder, StopReason, Usage,
};
use crate::ids::prefixed_compact_id;
use crate::script::{ScriptedResponse, SimError, SimToolCall, SimTurn};
use crate::{create_generator, EndpointType, ErrorInjector};
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
pub(super) fn sim_error_to_anthropic_response(err: &SimError) -> Response {
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

    // Reject thinking/effort combinations the real API rejects.
    if let Err(message) = thinking::validate(
        &request.model,
        Some(request.max_tokens),
        request.thinking.as_ref(),
        request.output_config.as_ref(),
    ) {
        state.stats.record_error(400);
        return anthropic_error(400, message);
    }

    // Reject requests over the model's context window or output cap.
    if let Err(v) = super::validation::check(
        &state.config.validation,
        &request.model,
        super::validation::anthropic_limits(&request.model),
        count_input_tokens(&request),
        Some(request.max_tokens),
    ) {
        let (status, message) = super::validation::anthropic_parts(&v);
        state.stats.record_error(status);
        return anthropic_error(status, message);
    }

    // Model-specific latency (unless overridden in config).
    let latency = state.config.latency_for_model(&request.model);

    // Scenario mode, as on chat completions.
    if let Some(response) =
        super::scenario_handlers::anthropic(&state, &request, request_start, latency.clone()).await
    {
        return response;
    }

    // Scripted non-streaming requests get full tool-call support.
    if let Some(script) = state.script.clone() {
        if !request.stream {
            return handle_scripted_message(state, request, request_start, script).await;
        }
    }

    // Resolve the response: a scripted turn, a forced tool call or JSON
    // (tool_choice / output_config.format), or generated text.
    let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();
    let to_uses = |turn_index: usize, calls: Vec<crate::script::SimToolCall>| {
        calls
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let id = c.id.unwrap_or_else(|| anthropic_tool_use_id(turn_index, i));
                (id, c.name, c.arguments)
            })
            .collect::<Vec<_>>()
    };
    let content = if let Some(script) = state.script.as_ref() {
        let turn_index = script.cursor();
        match script.next_turn() {
            ScriptedResponse::Turn(SimTurn::Assistant { text }) => text,
            ScriptedResponse::Turn(SimTurn::Mixed { text, calls }) => {
                tool_uses = to_uses(turn_index, calls);
                text
            }
            ScriptedResponse::Turn(SimTurn::ToolCalls { calls }) => {
                tool_uses = to_uses(turn_index, calls);
                String::new()
            }
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
    } else if let Some(forced) =
        crate::forced::anthropic(&request, state.config.response.auto_tool_calls)
    {
        match forced {
            crate::forced::Forced::ToolCalls(calls) => {
                tool_uses = to_uses(0, calls)
                    .into_iter()
                    .map(|(id, name, input)| (id.replacen("call_", "toolu_", 1), name, input))
                    .collect();
                String::new()
            }
            crate::forced::Forced::Json(text) => text,
        }
    } else {
        generate_content(&state, &request)
    };

    let input_tokens = count_input_tokens(&request);
    let (cache_read, cache_creation) = anthropic_cache(&state, &request, input_tokens);
    let latency = latency.with_input_tokens(input_tokens - cache_read);
    let text_tokens =
        crate::count_tokens_default(&content).unwrap_or(content.split_whitespace().count());

    // Thinking block for generated (non-scripted) responses; its tokens are
    // billed in output_tokens like the real API.
    let thinking_plan = if state.script.is_none() {
        thinking::plan(
            &request.model,
            request.thinking.as_ref(),
            request.output_config.as_ref(),
            text_tokens,
        )
    } else {
        None
    };
    // `[[llmsim:redacted-thinking]]` in the last message redacts the thinking
    // block, the way the safety system occasionally does.
    let redact = request
        .messages
        .last()
        .is_some_and(|m| m.content.extract_text().contains(REDACTED_THINKING_MARKER));
    let tool_tokens: usize = tool_uses
        .iter()
        .map(|(_, _, input)| {
            let text = input.to_string();
            crate::count_tokens_default(&text).unwrap_or(text.len() / 4) + 1
        })
        .sum();
    let output_tokens = text_tokens + tool_tokens + thinking_plan.as_ref().map_or(0, |p| p.tokens);
    // Anthropic bills cached and newly cached tokens apart from `input_tokens`.
    let mut usage = Usage::new(
        (input_tokens - cache_read - cache_creation) as u32,
        output_tokens as u32,
    );
    if state.config.response.prompt_caching {
        usage.cache_read_input_tokens = Some(cache_read as u32);
        usage.cache_creation_input_tokens = Some(cache_creation as u32);
    }

    if request.stream {
        let stats = state.stats.clone();
        let input_tok = usage.input_tokens;
        let output_tok = usage.output_tokens;

        let thinking_time = std::time::Duration::from_millis(
            thinking_plan.as_ref().map_or(0, |p| p.tokens as u64) * latency.tbt_mean_ms,
        );
        let mut builder = MessagesStreamBuilder::new(&request.model, content)
            .latency(latency)
            .usage(usage)
            .tool_uses(tool_uses)
            .thinking_time(thinking_time);
        if let Some(plan) = thinking_plan {
            if redact {
                builder = builder.redacted_thinking(redacted_data(&plan.signature));
            }
            builder = builder.thinking(plan.text, plan.signature);
        }
        let stream = builder
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
        let mut blocks = Vec::with_capacity(2);
        if let Some(plan) = thinking_plan {
            if redact {
                blocks.push(ContentBlock::RedactedThinking {
                    data: redacted_data(&plan.signature),
                });
            } else {
                blocks.push(ContentBlock::Thinking {
                    thinking: plan.text,
                    signature: plan.signature,
                });
            }
        }
        if !content.is_empty() || tool_uses.is_empty() {
            blocks.push(ContentBlock::text(content));
        }
        let stop_reason = if tool_uses.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        for (id, name, input) in tool_uses {
            blocks.push(ContentBlock::ToolUse { id, name, input });
        }
        let response =
            MessagesResponse::with_content(request.model.clone(), blocks, stop_reason, usage);
        Json(response).into_response()
    }
}

/// POST /anthropic/v1/messages/count_tokens
///
/// Returns the `input_tokens` a Messages request with the same prompt would
/// report, without generating anything.
pub async fn count_tokens(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<CountTokensRequest>,
) -> Response {
    if let Err(message) = thinking::validate(
        &request.model,
        None,
        request.thinking.as_ref(),
        request.output_config.as_ref(),
    ) {
        return anthropic_error(400, message);
    }
    let input_tokens = count_prompt_tokens(&request.prompt_text(), request.messages.len());
    Json(CountTokensResponse {
        input_tokens: input_tokens as u32,
    })
    .into_response()
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

/// Message marker that turns the thinking block into `redacted_thinking`.
pub const REDACTED_THINKING_MARKER: &str = "[[llmsim:redacted-thinking]]";

/// Opaque `redacted_thinking` data derived from the block's signature.
fn redacted_data(signature: &str) -> String {
    format!("EmwK{}", signature.chars().rev().collect::<String>())
}

/// Prompt caching for a Messages request: (cache_read, cache_creation)
/// tokens, both within `input_tokens`. Breakpoints are blocks carrying
/// `cache_control` (tools, then system, then messages), plus the last block
/// when the request has top-level `cache_control`.
fn anthropic_cache(
    state: &AppState,
    request: &MessagesRequest,
    input_tokens: usize,
) -> (usize, usize) {
    if !state.config.response.prompt_caching {
        return (0, 0);
    }
    let mut prefixes = super::prompt_cache::PrefixBuilder::new(&request.model);
    let mut breakpoints = Vec::new();
    let mut push = |piece: String, marked: bool| {
        let tokens = crate::count_tokens_default(&piece).unwrap_or(piece.len() / 4);
        let p = prefixes.push(&piece, tokens);
        if marked {
            breakpoints.push(p);
        }
    };
    for tool in request.tools.iter().flatten() {
        push(
            serde_json::to_string(tool).unwrap_or_default(),
            tool.cache_control.is_some(),
        );
    }
    match &request.system {
        Some(crate::anthropic::SystemPrompt::Text(t)) => push(t.clone(), false),
        Some(crate::anthropic::SystemPrompt::Blocks(blocks)) => {
            for b in blocks {
                push(b.text.clone(), b.cache_control.is_some());
            }
        }
        None => {}
    }
    for message in &request.messages {
        match &message.content {
            crate::anthropic::MessageContent::Text(t) => push(t.clone(), false),
            crate::anthropic::MessageContent::Blocks(blocks) => {
                for b in blocks {
                    let marked = matches!(
                        b,
                        crate::anthropic::RequestContentBlock::Text {
                            cache_control: Some(_),
                            ..
                        } | crate::anthropic::RequestContentBlock::ToolResult {
                            cache_control: Some(_),
                            ..
                        }
                    ) || matches!(b, crate::anthropic::RequestContentBlock::Other(v) if v.get("cache_control").is_some());
                    push(serde_json::to_string(b).unwrap_or_default(), marked);
                }
            }
        }
    }
    if request.cache_control.is_some() {
        if let Some(last) = prefixes.prefixes.last() {
            breakpoints.push(*last);
        }
    }
    let (read, creation) = state.prompt_cache.anthropic(&breakpoints, Instant::now());
    let read = read.min(input_tokens);
    (read, creation.min(input_tokens - read))
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
        reasoning_effort: None,
    };
    generator.generate(&chat_request)
}

/// Count input tokens for a Messages request (prompt text + small overhead).
pub(super) fn count_input_tokens(request: &MessagesRequest) -> usize {
    count_prompt_tokens(&request.prompt_text(), request.messages.len())
}

/// Token count for a flattened prompt plus per-message framing overhead.
fn count_prompt_tokens(text: &str, message_count: usize) -> usize {
    let base = crate::count_tokens_default(text).unwrap_or(text.split_whitespace().count());
    // Per-message + request framing overhead, similar to the OpenAI handler.
    base + message_count * 3 + 5
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
