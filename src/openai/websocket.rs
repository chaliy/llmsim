// WebSocket Mode Types
// Client and server message types for the OpenAI Responses API WebSocket transport.
// Reference: https://platform.openai.com/docs/guides/websocket-mode

use super::{ReasoningConfig, ResponsesError, ResponsesInput, ResponsesTool, ResponsesToolChoice};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Client event sent over WebSocket.
/// Currently only `response.create` is supported.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    /// Request to create a new response.
    /// Maps to `response.create` (the dot is handled by the rename).
    #[serde(rename = "response.create")]
    ResponseCreate {
        /// The response configuration.
        response: ResponseCreateBody,
    },
}

/// Body of a `response.create` client event.
/// Mirrors `ResponsesRequest` but omits `stream` (always streaming over WS)
/// and `background` (not applicable).
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseCreateBody {
    /// Model to use for generation
    pub model: String,
    /// Input text or array of input items
    pub input: ResponsesInput,
    /// System instructions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Sampling temperature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Nucleus sampling parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Custom metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    /// Chain to a previous response (connection-local cache)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Tools available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesTool>>,
    /// Tool choice
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ResponsesToolChoice>,
    /// Reasoning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    /// Include additional data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    /// Set to false for warmup/pre-loading (no model output)
    #[serde(default = "default_generate")]
    pub generate: bool,
}

fn default_generate() -> bool {
    true
}

/// Server event types sent over WebSocket.
/// These are the same events as the SSE streaming format, but sent as JSON
/// WebSocket text frames without the `event:` / `data:` SSE envelope.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Error event
    Error { error: ResponsesError },
}

impl ServerEvent {
    /// Create a `previous_response_not_found` error event.
    pub fn previous_response_not_found(response_id: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "previous_response_not_found",
                "message": format!("Previous response '{}' not found in connection cache", response_id),
                "code": "previous_response_not_found"
            }
        })
    }

    /// Create a `websocket_connection_limit_reached` error event.
    pub fn connection_limit_reached() -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "websocket_connection_limit_reached",
                "message": "WebSocket connection has exceeded the 60-minute limit",
                "code": "websocket_connection_limit_reached"
            }
        })
    }

    /// Create an `invalid_request` error event for malformed messages.
    pub fn invalid_request(message: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": message,
                "code": "invalid_request"
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_event_response_create() {
        let json = r#"{
            "type": "response.create",
            "response": {
                "model": "gpt-5",
                "input": "Hello!"
            }
        }"#;
        let event: ClientEvent = serde_json::from_str(json).unwrap();
        match event {
            ClientEvent::ResponseCreate { response } => {
                assert_eq!(response.model, "gpt-5");
                assert!(response.generate);
            }
        }
    }

    #[test]
    fn test_client_event_with_generate_false() {
        let json = r#"{
            "type": "response.create",
            "response": {
                "model": "gpt-5",
                "input": "Hello!",
                "generate": false
            }
        }"#;
        let event: ClientEvent = serde_json::from_str(json).unwrap();
        match event {
            ClientEvent::ResponseCreate { response } => {
                assert!(!response.generate);
            }
        }
    }

    #[test]
    fn test_client_event_with_items_input() {
        let json = r#"{
            "type": "response.create",
            "response": {
                "model": "gpt-5",
                "input": [
                    {"role": "user", "content": "Hello!"}
                ],
                "temperature": 0.7,
                "previous_response_id": "resp_abc123"
            }
        }"#;
        let event: ClientEvent = serde_json::from_str(json).unwrap();
        match event {
            ClientEvent::ResponseCreate { response } => {
                assert_eq!(response.model, "gpt-5");
                assert_eq!(response.temperature, Some(0.7));
                assert_eq!(
                    response.previous_response_id,
                    Some("resp_abc123".to_string())
                );
            }
        }
    }

    #[test]
    fn test_server_event_previous_response_not_found() {
        let event = ServerEvent::previous_response_not_found("resp_abc123");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("previous_response_not_found"));
        assert!(json.contains("resp_abc123"));
    }

    #[test]
    fn test_server_event_connection_limit() {
        let event = ServerEvent::connection_limit_reached();
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("websocket_connection_limit_reached"));
    }

    #[test]
    fn test_server_event_invalid_request() {
        let event = ServerEvent::invalid_request("bad message");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("invalid_request"));
        assert!(json.contains("bad message"));
    }
}
