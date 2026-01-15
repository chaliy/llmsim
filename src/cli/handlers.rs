// HTTP Handlers Module
// Implements OpenAI-compatible and OpenResponses-compatible API endpoints.

use super::state::AppState;
use crate::{
    create_generator,
    openai::{
        ChatCompletionRequest, ChatCompletionResponse, ErrorResponse, Model, ModelsResponse, Usage,
    },
    openresponses::{
        self, OpenResponsesStreamBuilder, Response as OpenResponsesResponse, ResponseRequest,
        Usage as OpenResponsesUsage,
    },
    ErrorInjector, LatencyProfile, TokenStreamBuilder,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Instant;

/// Health check endpoint
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "llmsim"
    }))
}

/// POST /v1/chat/completions
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
    state
        .stats
        .record_request_start(&request.model, request.stream);

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

/// GET /v1/stats - Get server statistics
pub async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.stats.snapshot())
}

/// POST /v1/responses - OpenResponses API endpoint
pub async fn create_response(
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
        .record_request_start(&request.model, request.stream);

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

/// GET /v1/models
pub async fn list_models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let models: Vec<Model> = state
        .config
        .models
        .available
        .iter()
        .map(|id| {
            let owned_by = if id.contains("gpt") {
                "openai"
            } else if id.contains("claude") {
                "anthropic"
            } else if id.contains("gemini") {
                "google"
            } else {
                "llmsim"
            };
            Model::new(id, owned_by)
        })
        .collect();

    Json(ModelsResponse::new(models))
}

/// GET /v1/models/:model_id
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> Result<Json<Model>, AppError> {
    if state.config.models.available.contains(&model_id) {
        let owned_by = if model_id.contains("gpt") {
            "openai"
        } else if model_id.contains("claude") {
            "anthropic"
        } else if model_id.contains("gemini") {
            "google"
        } else {
            "llmsim"
        };
        Ok(Json(Model::new(&model_id, owned_by)))
    } else {
        Err(AppError::NotFound(format!(
            "Model '{}' not found",
            model_id
        )))
    }
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
}
