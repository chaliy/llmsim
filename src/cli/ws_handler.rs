// WebSocket Handler Module
// Implements WebSocket transport for the OpenAI Responses API.
// Reference: https://platform.openai.com/docs/guides/websocket-mode

use super::handlers::{generate_responses_result, ResponseGenerationParams};
use super::state::AppState;
use crate::openai::websocket::{ClientEvent, ServerEvent};
use crate::openai::ResponsesResponse;
use crate::{EndpointType, ErrorInjector, ResponsesTokenStreamBuilder};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum connection duration (60 minutes)
const MAX_CONNECTION_DURATION: Duration = Duration::from_secs(60 * 60);

/// GET /openai/v1/responses (WebSocket upgrade)
pub async fn ws_responses(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    tracing::info!("WebSocket connection upgrade request");
    ws.on_upgrade(move |socket| handle_ws_connection(socket, state))
}

/// Handle a single WebSocket connection lifecycle.
async fn handle_ws_connection(mut socket: WebSocket, state: Arc<AppState>) {
    let connection_start = Instant::now();

    state.stats.record_ws_connect();
    tracing::info!("WebSocket connection established");

    // Connection-local cache: the most recent completed response
    let mut cached_response: Option<ResponsesResponse> = None;

    loop {
        // Check connection timeout
        if connection_start.elapsed() >= MAX_CONNECTION_DURATION {
            let event = ServerEvent::connection_limit_reached();
            let _ = socket
                .send(Message::Text(serde_json::to_string(&event).unwrap().into()))
                .await;
            break;
        }

        // Wait for next message with a timeout aligned to the remaining connection time
        let remaining = MAX_CONNECTION_DURATION.saturating_sub(connection_start.elapsed());
        let msg = tokio::time::timeout(remaining, socket.recv()).await;

        let msg = match msg {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => {
                tracing::warn!("WebSocket receive error: {}", e);
                break;
            }
            Ok(None) => {
                // Client disconnected
                tracing::info!("WebSocket client disconnected");
                break;
            }
            Err(_) => {
                // Connection timeout
                let event = ServerEvent::connection_limit_reached();
                let _ = socket
                    .send(Message::Text(serde_json::to_string(&event).unwrap().into()))
                    .await;
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let event = match serde_json::from_str::<ClientEvent>(&text) {
                    Ok(event) => event,
                    Err(e) => {
                        let error = ServerEvent::invalid_request(&format!(
                            "Failed to parse message: {}",
                            e
                        ));
                        let _ = socket
                            .send(Message::Text(serde_json::to_string(&error).unwrap().into()))
                            .await;
                        continue;
                    }
                };

                match event {
                    ClientEvent::ResponseCreate { response: body } => {
                        // Validate previous_response_id if provided
                        if let Some(ref prev_id) = body.previous_response_id {
                            let cached_id = cached_response.as_ref().map(|r| r.id.as_str());
                            if cached_id != Some(prev_id.as_str()) {
                                let error = ServerEvent::previous_response_not_found(prev_id);
                                let _ = socket
                                    .send(Message::Text(
                                        serde_json::to_string(&error).unwrap().into(),
                                    ))
                                    .await;
                                // Failed turn evicts cached response
                                cached_response = None;
                                continue;
                            }
                        }

                        // Handle generate=false (warmup/pre-loading)
                        if !body.generate {
                            tracing::info!(
                                model = %body.model,
                                "WebSocket warmup request (generate=false)"
                            );
                            // Acknowledge with a minimal response.created event
                            let warmup_response = ResponsesResponse::warmup(body.model.clone());
                            let event = serde_json::json!({
                                "type": "response.created",
                                "response": warmup_response
                            });
                            let _ = socket
                                .send(Message::Text(serde_json::to_string(&event).unwrap().into()))
                                .await;
                            continue;
                        }

                        let request_start = Instant::now();

                        tracing::info!(
                            model = %body.model,
                            "WebSocket response.create"
                        );

                        // Record request start
                        state.stats.record_request_start(
                            &body.model,
                            true, // WS is always streaming
                            EndpointType::WebSocketResponses,
                        );

                        // Check for error injection
                        let error_injector = ErrorInjector::new(state.config.error_config());
                        if let Some(error) = error_injector.maybe_inject() {
                            tracing::warn!("Injecting error on WebSocket: {:?}", error);
                            state.stats.record_error(error.status_code());

                            let err_resp = error.to_error_response();
                            let error_event = ServerEvent::from_error(
                                &err_resp.error.error_type,
                                &err_resp.error.message,
                            );
                            let _ = socket
                                .send(Message::Text(
                                    serde_json::to_string(&error_event).unwrap().into(),
                                ))
                                .await;
                            cached_response = None;
                            continue;
                        }

                        // Generate response using shared logic
                        let result = generate_responses_result(
                            &state,
                            &ResponseGenerationParams {
                                model: &body.model,
                                input: &body.input,
                                instructions: &body.instructions,
                                temperature: body.temperature,
                                top_p: body.top_p,
                                max_output_tokens: body.max_output_tokens,
                                reasoning: &body.reasoning,
                            },
                        );

                        // Build the streaming response
                        let stats = state.stats.clone();
                        let input_tok = result.usage.input_tokens;
                        let output_tok = result.usage.output_tokens;

                        let mut builder =
                            ResponsesTokenStreamBuilder::new(&body.model, result.content)
                                .latency(result.latency)
                                .usage(result.usage)
                                .on_complete(move || {
                                    stats.record_request_end(
                                        request_start.elapsed(),
                                        input_tok,
                                        output_tok,
                                    );
                                });

                        if result.reasoning_tokens > 0 {
                            builder = builder.reasoning(result.reasoning_summary);
                        }

                        let stream = builder.build();

                        // Stream events over WebSocket as JSON text frames.
                        // The existing stream produces SSE-formatted strings;
                        // we extract the JSON payload from each SSE chunk.
                        let mut sse_stream = stream.into_stream();
                        let mut last_completed_response: Option<ResponsesResponse> = None;

                        while let Some(sse_chunk) = sse_stream.next().await {
                            if let Some(json_str) = extract_json_from_sse(&sse_chunk) {
                                // Capture the completed response for caching
                                if sse_chunk.contains("response.completed") {
                                    if let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(json_str)
                                    {
                                        if let Some(resp) = parsed.get("response") {
                                            if let Ok(response) =
                                                serde_json::from_value::<ResponsesResponse>(
                                                    resp.clone(),
                                                )
                                            {
                                                last_completed_response = Some(response);
                                            }
                                        }
                                    }
                                }

                                if socket
                                    .send(Message::Text(json_str.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    tracing::warn!("Failed to send WS frame, client disconnected");
                                    break;
                                }
                            }
                        }

                        // Update connection-local cache
                        cached_response = last_completed_response;
                    }
                }
            }
            Message::Close(_) => {
                tracing::info!("WebSocket close frame received");
                break;
            }
            Message::Ping(data) => {
                let _ = socket.send(Message::Pong(data)).await;
            }
            _ => {
                // Ignore binary frames, pong frames
            }
        }
    }

    state.stats.record_ws_disconnect();
    tracing::info!(
        duration_secs = connection_start.elapsed().as_secs(),
        "WebSocket connection closed"
    );
}

/// Extract the JSON payload from an SSE-formatted string.
/// SSE format: `event: <type>\ndata: <json>\n\n`
fn extract_json_from_sse(sse: &str) -> Option<&str> {
    for line in sse.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            return Some(data.trim());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_sse() {
        let sse = "event: response.created\ndata: {\"type\":\"response.created\"}\n\n";
        let json = extract_json_from_sse(sse).unwrap();
        assert_eq!(json, "{\"type\":\"response.created\"}");
    }

    #[test]
    fn test_extract_json_from_sse_no_data() {
        let sse = "event: response.created\n\n";
        assert!(extract_json_from_sse(sse).is_none());
    }

    #[test]
    fn test_extract_json_from_sse_empty() {
        assert!(extract_json_from_sse("").is_none());
    }
}
