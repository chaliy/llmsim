// WebSocket Mode Types
// Client and server message types for the OpenAI Responses API WebSocket transport.
//
// The official wire format uses a flat structure where model, input, etc.
// are top-level fields alongside "type". For backward compatibility we also
// accept the nested {"type": "response.create", "response": {...}} form.
//
// Reference: https://platform.openai.com/docs/guides/websocket-mode

use super::{ReasoningConfig, ResponsesInput, ResponsesTool, ResponsesToolChoice};
use serde::Deserialize;
use std::collections::HashMap;

/// Client event sent over WebSocket.
/// Currently only `response.create` is supported.
///
/// Accepts both the flat format used by the OpenAI SDK:
///   {"type": "response.create", "model": "gpt-5", "input": "Hello"}
///
/// And the nested format:
///   {"type": "response.create", "response": {"model": "gpt-5", "input": "Hello"}}
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Request to create a new response.
    ResponseCreate { response: ResponseCreateBody },
}

impl<'de> Deserialize<'de> for ClientEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;

        let event_type = value
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;

        match event_type {
            "response.create" => {
                // Try nested format first: {"response": {...}}
                if let Some(response_obj) = value.get("response") {
                    let body: ResponseCreateBody = serde_json::from_value(response_obj.clone())
                        .map_err(|e| {
                            serde::de::Error::custom(format!("invalid response.create body: {}", e))
                        })?;
                    Ok(ClientEvent::ResponseCreate { response: body })
                } else {
                    // Flat format: all fields at top level alongside "type"
                    let body: ResponseCreateBody = serde_json::from_value(value).map_err(|e| {
                        serde::de::Error::custom(format!("invalid response.create body: {}", e))
                    })?;
                    Ok(ClientEvent::ResponseCreate { response: body })
                }
            }
            other => Err(serde::de::Error::custom(format!(
                "unknown event type: {}",
                other
            ))),
        }
    }
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

/// Helper for building WebSocket error events in the flat format expected
/// by the OpenAI SDK: `{"type": "error", "code": "...", "message": "...", "param": null, "sequence_number": 0}`
pub struct ServerEvent;

impl ServerEvent {
    /// Create a `previous_response_not_found` error event.
    pub fn previous_response_not_found(response_id: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "code": "previous_response_not_found",
            "message": format!("Previous response '{}' not found in connection cache", response_id),
            "param": null,
            "sequence_number": 0
        })
    }

    /// Create a `websocket_connection_limit_reached` error event.
    pub fn connection_limit_reached() -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "code": "websocket_connection_limit_reached",
            "message": "WebSocket connection has exceeded the 60-minute limit",
            "param": null,
            "sequence_number": 0
        })
    }

    /// Create an `invalid_request` error event for malformed messages.
    pub fn invalid_request(message: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "code": "invalid_request",
            "message": message,
            "param": null,
            "sequence_number": 0
        })
    }

    /// Create a generic error event from an injected error.
    pub fn from_error(code: &str, message: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "error",
            "code": code,
            "message": message,
            "param": null,
            "sequence_number": 0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_event_flat_format() {
        // Flat format used by the official OpenAI SDK
        let json = r#"{
            "type": "response.create",
            "model": "gpt-5",
            "input": "Hello!"
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
    fn test_client_event_nested_format() {
        // Nested format for backward compatibility with raw WebSocket clients
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
    fn test_client_event_flat_with_generate_false() {
        let json = r#"{
            "type": "response.create",
            "model": "gpt-5",
            "input": "Hello!",
            "generate": false
        }"#;
        let event: ClientEvent = serde_json::from_str(json).unwrap();
        match event {
            ClientEvent::ResponseCreate { response } => {
                assert!(!response.generate);
            }
        }
    }

    #[test]
    fn test_client_event_nested_with_generate_false() {
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
    fn test_client_event_flat_with_previous_response_id() {
        let json = r#"{
            "type": "response.create",
            "model": "gpt-5",
            "input": [
                {"role": "user", "content": "Hello!"}
            ],
            "temperature": 0.7,
            "previous_response_id": "resp_abc123"
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
    fn test_client_event_nested_with_items_input() {
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

    #[test]
    fn test_server_event_flat_error_format() {
        // Verify error events use the flat format expected by the OpenAI SDK
        let event = ServerEvent::invalid_request("test error");
        assert_eq!(event["type"], "error");
        assert_eq!(event["code"], "invalid_request");
        assert_eq!(event["message"], "test error");
        assert!(event["param"].is_null());
        assert_eq!(event["sequence_number"], 0);
    }
}
