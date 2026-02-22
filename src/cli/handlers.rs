// HTTP Handlers Module
// Implements OpenAI-compatible and OpenResponses-compatible API endpoints.

use super::state::AppState;
use crate::{
    create_generator,
    openai::{
        ChatCompletionRequest, ChatCompletionResponse, ErrorResponse, InputItem, InputRole,
        MessageContent, Model, ModelsResponse, OutputTokensDetails, ReasoningConfig,
        ResponsesErrorResponse, ResponsesInput, ResponsesRequest, ResponsesResponse,
        ResponsesUsage, Usage,
    },
    openresponses::{
        self, OpenResponsesStreamBuilder, Response as OpenResponsesResponse, ResponseRequest,
        Usage as OpenResponsesUsage,
    },
    EndpointType, ErrorInjector, LatencyProfile, ResponsesTokenStreamBuilder, TokenStreamBuilder,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use rand::prelude::IndexedRandom;
use std::sync::Arc;
use std::time::Instant;

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

    // Get latency profile (use model-specific if not configured)
    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(&request.model)
        };

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
    let generator = create_generator(
        &state.config.response.generator,
        state.config.response.target_tokens,
    );

    // Create a minimal ChatCompletionRequest for the generator
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
    let content = generator.generate(&chat_request);

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
    // Add overhead for request formatting
    total += 3;
    total
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

    // Get latency profile (use model-specific if not configured)
    let latency =
        if state.config.latency.profile.is_some() || state.config.latency.ttft_mean_ms.is_some() {
            state.config.latency_profile()
        } else {
            LatencyProfile::from_model(&request.model)
        };

    // Extract text from input for response generation
    let input_text = extract_input_text(&request.input, &request.instructions);

    // Generate response using the configured generator
    // Create a minimal ChatCompletionRequest for the generator
    let chat_request = crate::openai::ChatCompletionRequest {
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
    let content = generator.generate(&chat_request);

    // Count tokens
    let input_tokens =
        crate::count_tokens_default(&input_text).unwrap_or(input_text.split_whitespace().count());
    let output_tokens =
        crate::count_tokens_default(&content).unwrap_or(content.split_whitespace().count());

    // Simulate reasoning tokens for o-series and reasoning models
    let reasoning_tokens =
        calculate_reasoning_tokens(&request.model, &request.reasoning, output_tokens);

    let usage = ResponsesUsage {
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        total_tokens: (input_tokens + output_tokens + reasoning_tokens) as u32,
        output_tokens_details: Some(OutputTokensDetails {
            reasoning_tokens: reasoning_tokens as u32,
        }),
    };

    // Generate reasoning summary text if applicable
    let reasoning_summary =
        generate_reasoning_summary(&request.model, &request.reasoning, reasoning_tokens);

    if request.stream {
        // Streaming response
        // Clone stats for the streaming completion callback
        let stats = state.stats.clone();
        let input_tok = usage.input_tokens;
        let output_tok = usage.output_tokens;

        let mut builder = ResponsesTokenStreamBuilder::new(&request.model, content)
            .latency(latency)
            .usage(usage)
            .on_complete(move || {
                stats.record_request_end(request_start.elapsed(), input_tok, output_tok);
            });

        if reasoning_tokens > 0 {
            builder = builder.reasoning(reasoning_summary);
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

        let response = if reasoning_tokens > 0 {
            ResponsesResponse::with_reasoning(
                request.model.clone(),
                content,
                reasoning_summary,
                usage,
            )
        } else {
            ResponsesResponse::new(request.model.clone(), content, usage)
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

/// Count tokens in a chat request
fn count_request_tokens(request: &ChatCompletionRequest) -> usize {
    let mut total = 0;
    for message in &request.messages {
        if let Some(content) = &message.content {
            total +=
                crate::count_tokens_default(content).unwrap_or(content.split_whitespace().count());
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
