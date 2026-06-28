// HTTP Handlers Module
// Implements OpenAI-compatible and OpenResponses-compatible API endpoints.

use super::state::AppState;
use crate::ids::{prefixed_id, unix_timestamp};
use crate::{
    create_generator,
    openai::{
        ChatCompletionRequest, ChatCompletionResponse, ErrorResponse, InputItem, InputRole,
        MessageContent, Model, ModelsResponse, OutputContentPart, OutputItem, OutputRole,
        OutputTokensDetails, ReasoningConfig, ResponseStatus, ResponsesErrorResponse,
        ResponsesInput, ResponsesRequest, ResponsesResponse, ResponsesUsage, Usage,
    },
    openresponses::{
        self, OpenResponsesStreamBuilder, Response as OpenResponsesResponse, ResponseRequest,
        Usage as OpenResponsesUsage,
    },
    script::{ScriptedResponse, SimError, SimTurn},
    script_stream::{build_chat_completion_response, materialize_tool_calls, ScriptedChatStream},
    EndpointType, ErrorInjector, LatencyProfile, ResponsesTokenStreamBuilder, TokenStreamBuilder,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use rand::prelude::IndexedRandom;
use std::sync::Arc;
use std::time::Instant;

/// Result of response generation for the Responses API.
/// Shared between the HTTP and WebSocket handlers.
pub(crate) struct ResponseGenerationResult {
    pub content: String,
    pub usage: ResponsesUsage,
    pub reasoning_tokens: usize,
    pub reasoning_summary: Option<String>,
    pub latency: LatencyProfile,
}

/// Parameters for response generation.
pub(crate) struct ResponseGenerationParams<'a> {
    pub model: &'a str,
    pub input: &'a ResponsesInput,
    pub instructions: &'a Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub reasoning: &'a Option<ReasoningConfig>,
}

/// Generate a response for the Responses API.
/// Extracts the common logic used by both HTTP POST and WebSocket handlers.
pub(crate) fn generate_responses_result(
    state: &AppState,
    params: &ResponseGenerationParams<'_>,
) -> ResponseGenerationResult {
    // Get latency profile
    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(params.model)
        };

    // Extract text from input
    let input_text = extract_input_text(params.input, params.instructions);

    // Scripted mode: take the next turn and reduce it to a text body.
    // (Tool calls in streaming Responses API aren't implemented in v1;
    // see specs/scripted-mode.md.)
    let content = if let Some(script) = state.script.as_ref() {
        match script.next_turn() {
            ScriptedResponse::Turn(SimTurn::Assistant { text }) => text,
            ScriptedResponse::Turn(SimTurn::Mixed { text, .. }) => text,
            ScriptedResponse::Turn(SimTurn::ToolCalls { .. }) => String::new(),
            ScriptedResponse::Turn(SimTurn::Error(err)) => {
                // Surface the error as the response text in this code
                // path; streaming Responses API doesn't have a clean
                // way to abort mid-stream from this helper. Callers
                // who need error semantics on this endpoint should use
                // non-streaming requests.
                format!("[llmsim scripted error: {}]", err.message())
            }
            ScriptedResponse::Exhausted => "[llmsim script exhausted]".to_string(),
        }
    } else {
        // Create a minimal ChatCompletionRequest for the generator
        let chat_request = crate::openai::ChatCompletionRequest {
            model: params.model.to_string(),
            messages: vec![crate::openai::Message::user(&input_text)],
            temperature: params.temperature,
            top_p: params.top_p,
            n: None,
            stream: false,
            stop: None,
            max_tokens: params.max_output_tokens,
            max_completion_tokens: params.max_output_tokens,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
        };

        let generator = create_generator(
            &state.config.response.generator,
            state.config.response.target_tokens,
        );
        generator.generate(&chat_request)
    };

    // Count tokens
    let input_tokens = crate::count_tokens_default(&input_text)
        .unwrap_or(input_text.split_whitespace().count())
        + count_responses_input_image_tokens(params.input);
    let output_tokens =
        crate::count_tokens_default(&content).unwrap_or(content.split_whitespace().count());

    // Reasoning tokens
    let reasoning_tokens =
        calculate_reasoning_tokens(params.model, params.reasoning, output_tokens);

    let usage = ResponsesUsage {
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        total_tokens: (input_tokens + output_tokens + reasoning_tokens) as u32,
        output_tokens_details: Some(OutputTokensDetails {
            reasoning_tokens: reasoning_tokens as u32,
        }),
    };

    let reasoning_summary =
        generate_reasoning_summary(params.model, params.reasoning, reasoning_tokens);

    ResponseGenerationResult {
        content,
        usage,
        reasoning_tokens,
        reasoning_summary,
        latency,
    }
}

/// Health check endpoint
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "llmsim"
    }))
}

/// POST /openai/v1/chat/completions
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, AppError> {
    let request_start = Instant::now();

    tracing::info!(
        model = %request.model,
        stream = request.stream,
        messages = request.messages.len(),
        "Chat completion request"
    );

    // Record request start in stats
    state.stats.record_request_start(
        &request.model,
        request.stream,
        EndpointType::ChatCompletions,
    );

    // Check for error injection
    let error_injector = ErrorInjector::new(state.config.error_config());
    if let Some(error) = error_injector.maybe_inject() {
        tracing::warn!("Injecting error: {:?}", error);

        let status_code = error.status_code();
        let status = match status_code {
            429 => StatusCode::TOO_MANY_REQUESTS,
            500 => StatusCode::INTERNAL_SERVER_ERROR,
            503 => StatusCode::SERVICE_UNAVAILABLE,
            504 => StatusCode::GATEWAY_TIMEOUT,
            400 => StatusCode::BAD_REQUEST,
            401 => StatusCode::UNAUTHORIZED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Record error in stats
        state.stats.record_error(status_code);

        let mut response = Json(error.to_error_response()).into_response();
        *response.status_mut() = status;

        if let Some(retry_after) = error.retry_after() {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                retry_after.to_string().parse().unwrap(),
            );
        }

        return Ok(response);
    }

    // Reject image inputs to non-vision models before doing any work.
    if let Err(err) = validate_input_modalities(&request) {
        state.stats.record_error(400);
        return Ok(err.into_response());
    }

    // Get latency profile (use model-specific if not configured)
    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(&request.model)
        };

    // Scripted mode short-circuits the generator.
    if let Some(script) = state.script.clone() {
        return handle_scripted_chat_completions(state, request, request_start, latency, script)
            .await;
    }

    // Generate response
    let generator = create_generator(
        &state.config.response.generator,
        state.config.response.target_tokens,
    );
    let content = generator.generate(&request);

    // Count tokens
    let prompt_tokens = count_request_tokens(&request);
    let completion_tokens =
        crate::count_tokens_default(&content).unwrap_or(content.split_whitespace().count());
    let usage = Usage {
        prompt_tokens: prompt_tokens as u32,
        completion_tokens: completion_tokens as u32,
        total_tokens: (prompt_tokens + completion_tokens) as u32,
    };

    if request.stream {
        // Streaming response
        // Clone stats for the streaming completion callback
        let stats = state.stats.clone();
        let prompt_tok = usage.prompt_tokens;
        let completion_tok = usage.completion_tokens;

        let stream = TokenStreamBuilder::new(&request.model, content)
            .latency(latency)
            .usage(usage)
            .on_complete(move || {
                stats.record_request_end(request_start.elapsed(), prompt_tok, completion_tok);
            })
            .build();

        let body = Body::from_stream(stream.into_stream().map(Ok::<_, std::io::Error>));

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap())
    } else {
        // Non-streaming response - simulate time to generate
        let delay = latency.sample_ttft();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        // Record request completion
        state.stats.record_request_end(
            request_start.elapsed(),
            usage.prompt_tokens,
            usage.completion_tokens,
        );

        let response = ChatCompletionResponse::new(request.model.clone(), content, usage);
        Ok(Json(response).into_response())
    }
}

/// Drive the chat completions handler from the configured script.
async fn handle_scripted_chat_completions(
    state: Arc<AppState>,
    request: ChatCompletionRequest,
    request_start: Instant,
    latency: LatencyProfile,
    script: Arc<crate::script::Script>,
) -> Result<Response, AppError> {
    let turn_index = script.cursor();
    let next = script.next_turn();

    let turn = match next {
        ScriptedResponse::Turn(t) => t,
        ScriptedResponse::Exhausted => {
            state.stats.record_error(500);
            return Ok(sim_error_to_response(&SimError::Other {
                message: "llmsim script exhausted (on_exhausted=error)".to_string(),
                status_code: Some(500),
            }));
        }
    };

    let (text, tool_calls) = match turn {
        SimTurn::Assistant { text } => (Some(text), Vec::new()),
        SimTurn::ToolCalls { calls } => (None, calls),
        SimTurn::Mixed { text, calls } => (Some(text), calls),
        SimTurn::Error(err) => {
            state.stats.record_error(err.status_code());
            return Ok(sim_error_to_response(&err));
        }
    };

    let prompt_tokens = count_request_tokens(&request);
    let text_for_usage = text.clone().unwrap_or_default();
    let completion_tokens = crate::count_tokens_default(&text_for_usage)
        .unwrap_or(text_for_usage.split_whitespace().count());
    let usage = Usage {
        prompt_tokens: prompt_tokens as u32,
        completion_tokens: completion_tokens as u32,
        total_tokens: (prompt_tokens + completion_tokens) as u32,
    };

    let wire_calls = materialize_tool_calls(turn_index, &tool_calls);

    if request.stream {
        let stats = state.stats.clone();
        let prompt_tok = usage.prompt_tokens;
        let completion_tok = usage.completion_tokens;

        let stream = ScriptedChatStream::new(
            &request.model,
            text.unwrap_or_default(),
            tool_calls,
            latency,
        )
        .with_usage(usage)
        .with_on_complete(move || {
            stats.record_request_end(request_start.elapsed(), prompt_tok, completion_tok);
        });

        let body = Body::from_stream(stream.into_stream().map(Ok::<_, std::io::Error>));
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap())
    } else {
        let delay = latency.sample_ttft();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        state.stats.record_request_end(
            request_start.elapsed(),
            usage.prompt_tokens,
            usage.completion_tokens,
        );
        let resp = build_chat_completion_response(request.model.clone(), text, wire_calls, usage);
        Ok(Json(resp).into_response())
    }
}

/// Non-streaming scripted Responses API. Produces `OutputItem`s that
/// match the OpenAI wire shape: a `message` item for text and one
/// `function_call` item per scripted tool call.
async fn handle_scripted_responses_api(
    state: Arc<AppState>,
    request: ResponsesRequest,
    request_start: Instant,
    script: Arc<crate::script::Script>,
) -> Response {
    let turn_index = script.cursor();
    let next = script.next_turn();

    let turn = match next {
        ScriptedResponse::Turn(t) => t,
        ScriptedResponse::Exhausted => {
            state.stats.record_error(500);
            return sim_error_to_responses_api_response(&SimError::Other {
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
            return sim_error_to_responses_api_response(&err);
        }
    };

    let input_text = extract_input_text(&request.input, &request.instructions);
    let input_tokens = crate::count_tokens_default(&input_text)
        .unwrap_or(input_text.split_whitespace().count())
        + count_responses_input_image_tokens(&request.input);
    let text_for_usage = text.clone().unwrap_or_default();
    let output_text_tokens = crate::count_tokens_default(&text_for_usage)
        .unwrap_or(text_for_usage.split_whitespace().count());
    let tool_call_tokens: usize = tool_calls
        .iter()
        .map(|c| {
            let args = serde_json::to_string(&c.arguments).unwrap_or_default();
            crate::count_tokens_default(&args).unwrap_or(args.split_whitespace().count())
                + c.name.split_whitespace().count()
        })
        .sum();
    let output_tokens = output_text_tokens + tool_call_tokens;

    let usage = ResponsesUsage {
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        total_tokens: (input_tokens + output_tokens) as u32,
        output_tokens_details: Some(OutputTokensDetails {
            reasoning_tokens: 0,
        }),
    };

    let mut output: Vec<OutputItem> = Vec::new();
    let output_text_value: Option<String> = text.clone();
    if let Some(t) = text {
        output.push(OutputItem::Message {
            id: prefixed_id("msg_"),
            role: OutputRole::Assistant,
            status: crate::openai::ItemStatus::Completed,
            content: vec![OutputContentPart::OutputText {
                text: t,
                annotations: vec![],
            }],
        });
    }
    for (i, call) in tool_calls.iter().enumerate() {
        let call_id = call
            .id
            .clone()
            .unwrap_or_else(|| crate::script::auto_tool_call_id(turn_index, i));
        let args = serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());
        output.push(OutputItem::FunctionCall {
            id: prefixed_id("fc_"),
            call_id,
            name: call.name.clone(),
            arguments: args,
            status: crate::openai::ItemStatus::Completed,
        });
    }

    state.stats.record_request_end(
        request_start.elapsed(),
        usage.input_tokens,
        usage.output_tokens,
    );

    let resp = ResponsesResponse {
        id: prefixed_id("resp_"),
        object: "response".to_string(),
        created_at: unix_timestamp(),
        model: request.model,
        status: ResponseStatus::Completed,
        output,
        output_text: output_text_value,
        usage: Some(usage),
        error: None,
        metadata: None,
    };

    Json(resp).into_response()
}

/// Build an OpenAI Responses-API-shaped error response for a SimError.
fn sim_error_to_responses_api_response(err: &SimError) -> Response {
    let status = match err.status_code() {
        429 => StatusCode::TOO_MANY_REQUESTS,
        500 => StatusCode::INTERNAL_SERVER_ERROR,
        503 => StatusCode::SERVICE_UNAVAILABLE,
        504 => StatusCode::GATEWAY_TIMEOUT,
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        502 => StatusCode::BAD_GATEWAY,
        other => StatusCode::from_u16(other).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let body = ResponsesErrorResponse {
        error: crate::openai::ResponsesError::new(err.error_type(), err.message()),
    };
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
}

/// OpenResponses-shaped error response for a scripted SimError.
fn sim_error_to_openresponses_response(err: &SimError) -> Response {
    let status = match err.status_code() {
        429 => StatusCode::TOO_MANY_REQUESTS,
        500 => StatusCode::INTERNAL_SERVER_ERROR,
        503 => StatusCode::SERVICE_UNAVAILABLE,
        504 => StatusCode::GATEWAY_TIMEOUT,
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        502 => StatusCode::BAD_GATEWAY,
        other => StatusCode::from_u16(other).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let body = openresponses::ErrorResponse::new(err.message(), err.error_type());
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
}

/// Render a `SimError` from the script as an HTTP response (matches
/// the existing error-injection wire format).
fn sim_error_to_response(err: &SimError) -> Response {
    let status = match err.status_code() {
        429 => StatusCode::TOO_MANY_REQUESTS,
        500 => StatusCode::INTERNAL_SERVER_ERROR,
        503 => StatusCode::SERVICE_UNAVAILABLE,
        504 => StatusCode::GATEWAY_TIMEOUT,
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        502 => StatusCode::BAD_GATEWAY,
        other => StatusCode::from_u16(other).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let body = ErrorResponse::new(err.message(), err.error_type());
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
}

/// GET /llmsim/stats - Get server statistics
pub async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.stats.snapshot())
}

/// POST /openresponses/v1/responses - OpenResponses API endpoint
pub async fn create_openresponses_response(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResponseRequest>,
) -> Result<Response, AppError> {
    let request_start = Instant::now();

    tracing::info!(
        model = %request.model,
        stream = request.stream,
        "OpenResponses request"
    );

    // Record request start in stats
    state
        .stats
        .record_request_start(&request.model, request.stream, EndpointType::Responses);

    // Check for error injection
    let error_injector = ErrorInjector::new(state.config.error_config());
    if let Some(error) = error_injector.maybe_inject() {
        tracing::warn!("Injecting error: {:?}", error);

        let status_code = error.status_code();
        let status = match status_code {
            429 => StatusCode::TOO_MANY_REQUESTS,
            500 => StatusCode::INTERNAL_SERVER_ERROR,
            503 => StatusCode::SERVICE_UNAVAILABLE,
            504 => StatusCode::GATEWAY_TIMEOUT,
            400 => StatusCode::BAD_REQUEST,
            401 => StatusCode::UNAUTHORIZED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Record error in stats
        state.stats.record_error(status_code);

        let error_response = openresponses::ErrorResponse::new(
            error.to_error_response().error.message,
            error.to_error_response().error.error_type,
        );
        let mut response = Json(error_response).into_response();
        *response.status_mut() = status;

        if let Some(retry_after) = error.retry_after() {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                retry_after.to_string().parse().unwrap(),
            );
        }

        return Ok(response);
    }

    // Get latency profile (use model-specific if not configured)
    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(&request.model)
        };

    // Generate response using the input text
    let input_text = request.input.extract_text();

    // Scripted mode short-circuits to use the next scripted turn's text.
    // Error turns are surfaced as HTTP errors here (works for both
    // streaming and non-streaming); tool-call turns are not yet
    // represented in OpenResponses output items (see specs/scripted-mode.md).
    let content = if let Some(script) = state.script.as_ref() {
        match script.next_turn() {
            ScriptedResponse::Turn(SimTurn::Assistant { text }) => text,
            ScriptedResponse::Turn(SimTurn::Mixed { text, .. }) => text,
            ScriptedResponse::Turn(SimTurn::ToolCalls { .. }) => String::new(),
            ScriptedResponse::Turn(SimTurn::Error(err)) => {
                state.stats.record_error(err.status_code());
                return Ok(sim_error_to_openresponses_response(&err));
            }
            ScriptedResponse::Exhausted => {
                state.stats.record_error(500);
                return Ok(sim_error_to_openresponses_response(&SimError::Other {
                    message: "llmsim script exhausted (on_exhausted=error)".to_string(),
                    status_code: Some(500),
                }));
            }
        }
    } else {
        let generator = create_generator(
            &state.config.response.generator,
            state.config.response.target_tokens,
        );
        let chat_request = ChatCompletionRequest {
            model: request.model.clone(),
            messages: vec![crate::openai::Message::user(&input_text)],
            temperature: request.temperature,
            top_p: request.top_p,
            n: None,
            stream: request.stream,
            stop: None,
            max_tokens: request.max_output_tokens,
            max_completion_tokens: request.max_output_tokens,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: request.user.clone(),
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
        };
        generator.generate(&chat_request)
    };

    // Count tokens
    let input_tokens = count_openresponses_input_tokens(&request);
    let output_tokens =
        crate::count_tokens_default(&content).unwrap_or(content.split_whitespace().count());
    let usage = OpenResponsesUsage {
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        total_tokens: (input_tokens + output_tokens) as u32,
        input_tokens_details: None,
        output_tokens_details: None,
    };

    if request.stream {
        // Streaming response
        let stats = state.stats.clone();
        let input_tok = usage.input_tokens;
        let output_tok = usage.output_tokens;

        let stream = OpenResponsesStreamBuilder::new(&request.model, content)
            .latency(latency)
            .usage(usage)
            .on_complete(move || {
                stats.record_request_end(request_start.elapsed(), input_tok, output_tok);
            })
            .build();

        let body = Body::from_stream(stream.into_stream().map(Ok::<_, std::io::Error>));

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap())
    } else {
        // Non-streaming response - simulate time to generate
        let delay = latency.sample_ttft();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        // Record request completion
        state.stats.record_request_end(
            request_start.elapsed(),
            usage.input_tokens,
            usage.output_tokens,
        );

        let response = OpenResponsesResponse::new(request.model.clone(), content, usage);
        Ok(Json(response).into_response())
    }
}

/// Count tokens in an OpenResponses request
fn count_openresponses_input_tokens(request: &ResponseRequest) -> usize {
    let text = request.input.extract_text();
    let mut total = crate::count_tokens_default(&text).unwrap_or(text.split_whitespace().count());
    // Account for image inputs (simulator approximation; see estimate_image_tokens).
    total += count_openresponses_input_image_tokens(&request.input);
    // Add overhead for request formatting
    total += 3;
    total
}

/// Approximate the image-input token cost for an OpenResponses request.
/// OpenResponses `input_image` parts carry an optional `detail`, which is
/// passed through to `estimate_image_tokens`.
fn count_openresponses_input_image_tokens(input: &openresponses::Input) -> usize {
    use openresponses::{ContentItem, Input, MessageContent};

    let Input::Messages(messages) = input else {
        return 0;
    };
    messages
        .iter()
        .map(|m| match &m.content {
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentItem::InputImage { detail, .. } => {
                        Some(crate::estimate_image_tokens(detail.as_deref()))
                    }
                    _ => None,
                })
                .sum(),
            MessageContent::Text(_) => 0,
        })
        .sum()
}

/// GET /openai/v1/models
/// Returns models with realistic profiles from models.dev when available
pub async fn list_models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use crate::openai::{get_model_profile, infer_model_owner};

    let models: Vec<Model> = state
        .config
        .models
        .available
        .iter()
        .map(|id| {
            // Use profile from models.dev registry if available
            if let Some(profile) = get_model_profile(id) {
                Model::from_profile(profile)
            } else {
                // Fall back to basic model with inferred owner
                Model::new(id, infer_model_owner(id))
            }
        })
        .collect();

    Json(ModelsResponse::new(models))
}

/// GET /openai/v1/models/:model_id
/// Returns model with realistic profile from models.dev when available
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> Result<Json<Model>, AppError> {
    use crate::openai::{get_model_profile, infer_model_owner};

    if state.config.models.available.contains(&model_id) {
        // Use profile from models.dev registry if available
        let model = if let Some(profile) = get_model_profile(&model_id) {
            Model::from_profile(profile)
        } else {
            Model::new(&model_id, infer_model_owner(&model_id))
        };
        Ok(Json(model))
    } else {
        Err(AppError::NotFound(format!(
            "Model '{}' not found",
            model_id
        )))
    }
}

/// POST /openai/v1/responses
pub async fn create_response(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResponsesRequest>,
) -> Result<Response, AppError> {
    let request_start = Instant::now();

    tracing::info!(
        model = %request.model,
        stream = request.stream,
        "Responses API request"
    );

    // Record request start in stats
    state
        .stats
        .record_request_start(&request.model, request.stream, EndpointType::Responses);

    // Check for error injection
    let error_injector = ErrorInjector::new(state.config.error_config());
    if let Some(error) = error_injector.maybe_inject() {
        tracing::warn!("Injecting error: {:?}", error);

        let status = match error.status_code() {
            429 => StatusCode::TOO_MANY_REQUESTS,
            500 => StatusCode::INTERNAL_SERVER_ERROR,
            503 => StatusCode::SERVICE_UNAVAILABLE,
            504 => StatusCode::GATEWAY_TIMEOUT,
            400 => StatusCode::BAD_REQUEST,
            401 => StatusCode::UNAUTHORIZED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Record error in stats
        state.stats.record_error(error.status_code());

        let error_response = ResponsesErrorResponse {
            error: crate::openai::ResponsesError::new(
                error.to_error_response().error.error_type,
                error.to_error_response().error.message,
            ),
        };

        let mut response = Json(error_response).into_response();
        *response.status_mut() = status;

        if let Some(retry_after) = error.retry_after() {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                retry_after.to_string().parse().unwrap(),
            );
        }

        return Ok(response);
    }

    // Scripted mode: handle non-streaming with full tool-call support;
    // streaming falls through to the text-based scripted result built
    // by generate_responses_result (text + error turns only).
    if let Some(script) = state.script.clone() {
        if !request.stream {
            return Ok(handle_scripted_responses_api(state, request, request_start, script).await);
        }
    }

    // Generate response using shared logic
    let result = generate_responses_result(
        &state,
        &ResponseGenerationParams {
            model: &request.model,
            input: &request.input,
            instructions: &request.instructions,
            temperature: request.temperature,
            top_p: request.top_p,
            max_output_tokens: request.max_output_tokens,
            reasoning: &request.reasoning,
        },
    );

    if request.stream {
        // Streaming response
        // Clone stats for the streaming completion callback
        let stats = state.stats.clone();
        let input_tok = result.usage.input_tokens;
        let output_tok = result.usage.output_tokens;

        let mut builder = ResponsesTokenStreamBuilder::new(&request.model, result.content)
            .latency(result.latency)
            .usage(result.usage)
            .on_complete(move || {
                stats.record_request_end(request_start.elapsed(), input_tok, output_tok);
            });

        if result.reasoning_tokens > 0 {
            builder = builder.reasoning(result.reasoning_summary);
        }

        let stream = builder.build();

        let body = Body::from_stream(stream.into_stream().map(Ok::<_, std::io::Error>));

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap())
    } else {
        // Non-streaming response - simulate time to generate
        let delay = result.latency.sample_ttft();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        // Record request completion
        state.stats.record_request_end(
            request_start.elapsed(),
            result.usage.input_tokens,
            result.usage.output_tokens,
        );

        let response = if result.reasoning_tokens > 0 {
            ResponsesResponse::with_reasoning(
                request.model.clone(),
                result.content,
                result.reasoning_summary,
                result.usage,
            )
        } else {
            ResponsesResponse::new(request.model.clone(), result.content, result.usage)
        };
        Ok(Json(response).into_response())
    }
}

/// Extract text content from ResponsesInput for processing
fn extract_input_text(input: &ResponsesInput, instructions: &Option<String>) -> String {
    let mut parts = Vec::new();

    // Add instructions if present
    if let Some(instr) = instructions {
        parts.push(instr.clone());
    }

    match input {
        ResponsesInput::Text(text) => {
            parts.push(text.clone());
        }
        ResponsesInput::Items(items) => {
            for item in items {
                if let InputItem::Message { role, content } = item {
                    let role_str = match role {
                        InputRole::User => "user",
                        InputRole::Assistant => "assistant",
                        InputRole::System => "system",
                        InputRole::Developer => "developer",
                    };

                    let content_str = match content {
                        MessageContent::Text(text) => text.clone(),
                        MessageContent::Parts(content_parts) => content_parts
                            .iter()
                            .filter_map(|p| {
                                if let crate::openai::ContentPart::InputText { text } = p {
                                    Some(text.clone())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" "),
                    };

                    parts.push(format!("{}: {}", role_str, content_str));
                }
            }
        }
    }

    parts.join("\n")
}

/// Approximate the image-input token cost for a Responses API request.
/// The Responses `input_image` part carries no `detail`, so each image is
/// charged at the high-detail default (see `estimate_image_tokens`).
fn count_responses_input_image_tokens(input: &ResponsesInput) -> usize {
    let ResponsesInput::Items(items) = input else {
        return 0;
    };
    items
        .iter()
        .filter_map(|item| match item {
            InputItem::Message { content, .. } => Some(content),
            _ => None,
        })
        .map(|content| match content {
            MessageContent::Parts(parts) => parts
                .iter()
                .filter(|p| matches!(p, crate::openai::ContentPart::InputImage { .. }))
                .map(|_| crate::estimate_image_tokens(None))
                .sum(),
            MessageContent::Text(_) => 0,
        })
        .sum()
}

/// Generate simulated reasoning summary text.
/// Returns `Some(text)` when the model is a reasoning model and summary is requested.
fn generate_reasoning_summary(
    model: &str,
    reasoning: &Option<ReasoningConfig>,
    reasoning_tokens: usize,
) -> Option<String> {
    if reasoning_tokens == 0 {
        return None;
    }

    // Check if summary is requested
    let summary_mode = reasoning.as_ref().and_then(|r| r.summary.as_deref());
    match summary_mode {
        Some("auto") | Some("concise") | Some("detailed") => {}
        _ => return None,
    }

    // Scale summary length based on mode and reasoning token count
    let word_count = match summary_mode {
        Some("concise") => (reasoning_tokens as f64 * 0.05).max(8.0) as usize,
        Some("detailed") => (reasoning_tokens as f64 * 0.15).max(15.0) as usize,
        _ => (reasoning_tokens as f64 * 0.1).max(10.0) as usize, // "auto"
    };

    Some(generate_reasoning_text(model, word_count))
}

/// Generate plausible reasoning summary text of the given word count.
fn generate_reasoning_text(_model: &str, word_count: usize) -> String {
    const REASONING_PHRASES: &[&str] = &[
        "the model considered",
        "analyzing the input",
        "evaluating possible approaches",
        "breaking down the problem",
        "considering multiple perspectives",
        "reviewing relevant context",
        "weighing the alternatives",
        "synthesizing information",
        "formulating a response",
        "assessing the requirements",
        "identifying key factors",
        "examining the constraints",
        "reasoning through the steps",
        "determining the best approach",
        "processing the query",
    ];

    const FILLER_WORDS: &[&str] = &[
        "and",
        "then",
        "next",
        "also",
        "before",
        "after",
        "while",
        "during",
        "through",
        "carefully",
        "thoroughly",
        "systematically",
        "logically",
        "methodically",
    ];

    let mut rng = rand::rng();
    let mut words = Vec::with_capacity(word_count);

    // Start with a reasoning phrase
    let phrase = REASONING_PHRASES.choose(&mut rng).unwrap();
    words.extend(phrase.split_whitespace());

    while words.len() < word_count {
        // Alternate between filler words and reasoning phrases
        if words.len() % 5 == 0 && words.len() + 3 < word_count {
            let filler = FILLER_WORDS.choose(&mut rng).unwrap();
            words.push(filler);
        }
        let phrase = REASONING_PHRASES.choose(&mut rng).unwrap();
        for w in phrase.split_whitespace() {
            if words.len() >= word_count {
                break;
            }
            words.push(w);
        }
    }

    words.truncate(word_count);

    // Capitalize first word and join
    let mut result = String::new();
    for (i, word) in words.iter().enumerate() {
        if i == 0 {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                result.push(first.to_ascii_uppercase());
                result.extend(chars);
            }
        } else {
            result.push(' ');
            result.push_str(word);
        }
    }
    result.push('.');
    result
}

/// Check if a model is a reasoning model (o-series or GPT-5 family)
fn is_reasoning_model(model: &str) -> bool {
    let is_o_series = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("-o1")
        || model.contains("-o3");

    let is_gpt5 = model.starts_with("gpt-5");

    is_o_series || is_gpt5
}

/// Calculate simulated reasoning tokens for reasoning models (o-series and GPT-5)
fn calculate_reasoning_tokens(
    model: &str,
    reasoning: &Option<ReasoningConfig>,
    output_tokens: usize,
) -> usize {
    if !is_reasoning_model(model) {
        return 0;
    }

    // Determine effort level
    // GPT-5 supports: minimal, low, medium, high, xhigh
    // o-series supports: low, medium, high
    let effort = reasoning
        .as_ref()
        .and_then(|r| r.effort.as_deref())
        .unwrap_or("medium");

    // Simulate reasoning tokens based on effort level
    // Reasoning models typically generate 2-10x the output tokens in reasoning
    let multiplier = match effort {
        "none" => 0.0,
        "minimal" => 0.5, // GPT-5 only: fastest, minimal reasoning
        "low" => 1.5,
        "medium" => 3.0,
        "high" => 6.0,
        "xhigh" => 10.0, // GPT-5.2 only: most thorough reasoning
        _ => 3.0,        // default to medium
    };

    (output_tokens as f64 * multiplier) as usize
}

/// Reject image inputs sent to a model that does not advertise vision support,
/// mirroring the real provider behavior. Decision: only enforce when the model
/// resolves to a known profile — unknown/custom model ids are let through since
/// the simulator cannot assert their capabilities. The content-array syntax
/// itself is always accepted (valid for every chat model); only image parts gate.
fn validate_input_modalities(request: &ChatCompletionRequest) -> Result<(), AppError> {
    let has_images = request
        .messages
        .iter()
        .filter_map(|m| m.content.as_ref())
        .any(|c| c.has_images());

    if has_images {
        if let Some(profile) = crate::openai::get_model_profile(&request.model) {
            if !profile.capabilities.vision {
                return Err(AppError::BadRequest(format!(
                    "The model `{}` does not support image inputs. \
                     Use a vision-capable model.",
                    request.model
                )));
            }
        }
    }

    Ok(())
}

/// Count tokens in a chat request
fn count_request_tokens(request: &ChatCompletionRequest) -> usize {
    let mut total = 0;
    for message in &request.messages {
        if let Some(content) = &message.content {
            let text = content.text();
            total += crate::count_tokens_default(&text).unwrap_or(text.split_whitespace().count());
            // Account for image inputs (simulator approximation; see estimate_image_tokens).
            for image in content.images() {
                total += crate::estimate_image_tokens(image.detail.as_deref());
            }
        }
        // Add overhead for message formatting
        total += 4;
    }
    // Add overhead for request formatting
    total += 3;
    total
}

/// Application error type
#[derive(Debug)]
#[allow(dead_code)]
pub enum AppError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_response) = match self {
            AppError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                ErrorResponse::new(msg, "not_found_error"),
            ),
            AppError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, ErrorResponse::invalid_request(msg))
            }
            AppError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorResponse::new(msg, "internal_error"),
            ),
        };

        let mut response = Json(error_response).into_response();
        *response.status_mut() = status;
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::Message;

    #[test]
    fn test_count_request_tokens() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                Message::system("You are a helpful assistant."),
                Message::user("Hello!"),
            ],
            temperature: None,
            top_p: None,
            n: None,
            stream: false,
            stop: None,
            max_tokens: None,
            max_completion_tokens: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
        };

        let tokens = count_request_tokens(&request);
        assert!(tokens > 0);
    }

    fn request_with_image(model: &str) -> ChatCompletionRequest {
        let json = format!(
            r#"{{
                "model": "{model}",
                "messages": [
                    {{"role": "user", "content": [
                        {{"type": "text", "text": "Describe this"}},
                        {{"type": "image_url", "image_url": {{"url": "https://example.com/x.png"}}}}
                    ]}}
                ]
            }}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn test_count_request_tokens_includes_image_tokens() {
        // A high-detail image adds IMAGE_TOKENS_HIGH over the text-only count.
        let with_image = request_with_image("gpt-4o");

        let text_only: ChatCompletionRequest = serde_json::from_str(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":[{"type":"text","text":"Describe this"}]}]}"#,
        )
        .unwrap();

        let delta = count_request_tokens(&with_image) - count_request_tokens(&text_only);
        assert_eq!(delta, crate::tokens::IMAGE_TOKENS_HIGH);
    }

    #[test]
    fn test_count_request_tokens_low_detail_image() {
        let low: ChatCompletionRequest = serde_json::from_str(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"u","detail":"low"}}]}]}"#,
        )
        .unwrap();
        let none: ChatCompletionRequest =
            serde_json::from_str(r#"{"model":"gpt-4o","messages":[{"role":"user","content":[]}]}"#)
                .unwrap();
        let delta = count_request_tokens(&low) - count_request_tokens(&none);
        assert_eq!(delta, crate::tokens::IMAGE_TOKENS_LOW);
    }

    #[test]
    fn test_responses_input_image_tokens() {
        let input: ResponsesInput = serde_json::from_str(
            r#"[{"type":"message","role":"user","content":[
                {"type":"input_text","text":"hi"},
                {"type":"input_image","image_url":"https://example.com/a.png"}
            ]}]"#,
        )
        .unwrap();
        assert_eq!(
            count_responses_input_image_tokens(&input),
            crate::tokens::IMAGE_TOKENS_HIGH
        );

        let text_only: ResponsesInput = serde_json::from_str(r#""just text""#).unwrap();
        assert_eq!(count_responses_input_image_tokens(&text_only), 0);
    }

    #[test]
    fn test_openresponses_input_image_tokens() {
        let request: ResponseRequest = serde_json::from_str(
            r#"{"model":"gpt-4o","input":[{"role":"user","content":[
                {"type":"input_text","text":"hi"},
                {"type":"input_image","image_url":"u","detail":"low"}
            ]}]}"#,
        )
        .unwrap();
        assert_eq!(
            count_openresponses_input_image_tokens(&request.input),
            crate::tokens::IMAGE_TOKENS_LOW
        );
    }

    #[test]
    fn test_vision_model_accepts_image_input() {
        // gpt-4o advertises vision.
        let request = request_with_image("gpt-4o");
        assert!(validate_input_modalities(&request).is_ok());
    }

    #[test]
    fn test_non_vision_model_rejects_image_input() {
        // gpt-4 has vision: false in its profile.
        let request = request_with_image("gpt-4");
        let err = validate_input_modalities(&request).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn test_unknown_model_allows_image_input() {
        // Custom/unknown ids have no profile, so we cannot assert non-vision.
        let request = request_with_image("my-custom-model");
        assert!(validate_input_modalities(&request).is_ok());
    }

    #[test]
    fn test_text_only_request_passes_modality_check() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message::user("Hello!")],
            temperature: None,
            top_p: None,
            n: None,
            stream: false,
            stop: None,
            max_tokens: None,
            max_completion_tokens: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
        };
        assert!(validate_input_modalities(&request).is_ok());
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let response = health().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_is_reasoning_model() {
        // o-series models
        assert!(is_reasoning_model("o1"));
        assert!(is_reasoning_model("o1-mini"));
        assert!(is_reasoning_model("o3"));
        assert!(is_reasoning_model("o3-mini"));
        assert!(is_reasoning_model("o4-mini"));

        // GPT-5 models
        assert!(is_reasoning_model("gpt-5"));
        assert!(is_reasoning_model("gpt-5-mini"));
        assert!(is_reasoning_model("gpt-5.1"));
        assert!(is_reasoning_model("gpt-5.2"));

        // Non-reasoning models
        assert!(!is_reasoning_model("gpt-4o"));
        assert!(!is_reasoning_model("gpt-4o-mini"));
        assert!(!is_reasoning_model("gpt-4"));
        assert!(!is_reasoning_model("claude-sonnet-4"));
    }

    #[test]
    fn test_calculate_reasoning_tokens_reasoning_model() {
        // o3 with default (medium) effort
        let tokens = calculate_reasoning_tokens("o3", &None, 100);
        assert_eq!(tokens, 300); // 3.0x multiplier

        // o3 with high effort
        let config = Some(ReasoningConfig {
            effort: Some("high".to_string()),
            summary: None,
        });
        let tokens = calculate_reasoning_tokens("o3", &config, 100);
        assert_eq!(tokens, 600); // 6.0x multiplier

        // gpt-5 with none effort
        let config = Some(ReasoningConfig {
            effort: Some("none".to_string()),
            summary: None,
        });
        let tokens = calculate_reasoning_tokens("gpt-5", &config, 100);
        assert_eq!(tokens, 0); // 0.0x multiplier
    }

    #[test]
    fn test_calculate_reasoning_tokens_non_reasoning_model() {
        let tokens = calculate_reasoning_tokens("gpt-4o", &None, 100);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_generate_reasoning_summary_with_summary() {
        let config = Some(ReasoningConfig {
            effort: Some("medium".to_string()),
            summary: Some("auto".to_string()),
        });
        let summary = generate_reasoning_summary("o3", &config, 300);
        assert!(summary.is_some());
        let text = summary.unwrap();
        assert!(!text.is_empty());
        assert!(text.ends_with('.'));
    }

    #[test]
    fn test_generate_reasoning_summary_without_summary() {
        // No reasoning config
        let summary = generate_reasoning_summary("o3", &None, 300);
        assert!(summary.is_none());

        // Reasoning config without summary
        let config = Some(ReasoningConfig {
            effort: Some("medium".to_string()),
            summary: None,
        });
        let summary = generate_reasoning_summary("o3", &config, 300);
        assert!(summary.is_none());
    }

    #[test]
    fn test_generate_reasoning_summary_zero_tokens() {
        let config = Some(ReasoningConfig {
            effort: Some("none".to_string()),
            summary: Some("auto".to_string()),
        });
        let summary = generate_reasoning_summary("o3", &config, 0);
        assert!(summary.is_none());
    }

    #[test]
    fn test_generate_reasoning_summary_modes() {
        let reasoning_tokens = 200;

        let concise_config = Some(ReasoningConfig {
            effort: Some("medium".to_string()),
            summary: Some("concise".to_string()),
        });
        let concise = generate_reasoning_summary("o3", &concise_config, reasoning_tokens).unwrap();

        let detailed_config = Some(ReasoningConfig {
            effort: Some("medium".to_string()),
            summary: Some("detailed".to_string()),
        });
        let detailed =
            generate_reasoning_summary("o3", &detailed_config, reasoning_tokens).unwrap();

        // Detailed should generally be longer than concise
        assert!(
            detailed.len() > concise.len(),
            "Detailed summary ({}) should be longer than concise ({})",
            detailed.len(),
            concise.len()
        );
    }

    #[test]
    fn test_generate_reasoning_text() {
        let text = generate_reasoning_text("o3", 20);
        assert!(!text.is_empty());
        assert!(text.ends_with('.'));
        // First character should be uppercase
        assert!(text.chars().next().unwrap().is_uppercase());
    }
}
