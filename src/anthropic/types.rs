// Anthropic Messages API Types
// These types mirror the Anthropic Messages API wire format so that the
// official Anthropic SDKs (Python `anthropic`, `@anthropic-ai/sdk`, Go, etc.)
// work against the simulator when pointed at `{base_url}/anthropic`.
// Reference: https://docs.anthropic.com/en/api/messages

use crate::ids::prefixed_compact_id;
use serde::{Deserialize, Serialize};

/// Role of a message in an Anthropic conversation.
/// The Messages API only allows `user` and `assistant` (system content is a
/// top-level `system` field, not a message role).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A single message in the conversation. `content` is either a plain string or
/// an array of content blocks (text, image, tool_use, tool_result, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: Role,
    pub content: MessageContent,
}

/// Message content: the Messages API accepts either a bare string or an array
/// of typed content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<RequestContentBlock>),
}

impl MessageContent {
    /// Flatten the message content down to plain text for token counting and
    /// prompt construction. Non-text blocks (images, tool results) contribute
    /// any embedded text but are otherwise ignored.
    pub fn extract_text(&self) -> String {
        match self {
            MessageContent::Text(t) => t.clone(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| b.extract_text())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// A content block on an inbound request. We model the common variants and
/// fall back to `Other` for anything we don't need to interpret (images,
/// documents, thinking, etc.) so deserialization never fails on valid input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<serde_json::Value>,
    },
    /// Any other block type (image, document, thinking, ...). Captured so we
    /// can round-trip without erroring; the inner value is the raw JSON.
    #[serde(untagged)]
    Other(serde_json::Value),
}

impl RequestContentBlock {
    fn extract_text(&self) -> Option<String> {
        match self {
            RequestContentBlock::Text { text } => Some(text.clone()),
            RequestContentBlock::ToolResult {
                content: Some(value),
                ..
            } => Some(stringify_tool_result(value)),
            _ => None,
        }
    }
}

/// Reduce a `tool_result` content value (string or array of blocks) to text.
fn stringify_tool_result(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

/// Top-level `system` prompt: a string or an array of text blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

impl SystemPrompt {
    pub fn extract_text(&self) -> String {
        match self {
            SystemPrompt::Text(t) => t.clone(),
            SystemPrompt::Blocks(blocks) => blocks
                .iter()
                .map(|b| b.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// A system content block (only `text` blocks are meaningful for the simulator).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    #[serde(default = "default_text_type", rename = "type")]
    pub block_type: String,
    pub text: String,
}

fn default_text_type() -> String {
    "text".to_string()
}

/// A tool definition (function the model may call).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

/// Request metadata (e.g. `user_id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// Anthropic Messages API request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    /// Required by the real API; the maximum number of tokens to generate.
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemPrompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

impl MessagesRequest {
    /// Build the flattened prompt text used by the response generator and for
    /// input-token accounting: system prompt followed by each message.
    pub fn prompt_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(system) = &self.system {
            parts.push(system.extract_text());
        }
        for msg in &self.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            parts.push(format!("{}: {}", role, msg.content.extract_text()));
        }
        parts.join("\n")
    }
}

/// A content block on the response. The simulator emits `text` blocks for prose
/// and `tool_use` blocks when scripted tool calls are configured.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }
}

/// Why the model stopped generating. Matches the Anthropic `stop_reason` enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    PauseTurn,
    Refusal,
}

/// Token usage for a Messages API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
}

impl Usage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }
    }
}

/// Anthropic Messages API response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: Role,
    pub model: String,
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    pub usage: Usage,
}

impl MessagesResponse {
    /// Build a plain-text response.
    pub fn text(model: impl Into<String>, content: impl Into<String>, usage: Usage) -> Self {
        Self {
            id: prefixed_compact_id("msg_"),
            response_type: "message".to_string(),
            role: Role::Assistant,
            model: model.into(),
            content: vec![ContentBlock::text(content)],
            stop_reason: Some(StopReason::EndTurn),
            stop_sequence: None,
            usage,
        }
    }

    /// Build a response from explicit content blocks and a stop reason.
    pub fn with_content(
        model: impl Into<String>,
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        usage: Usage,
    ) -> Self {
        Self {
            id: prefixed_compact_id("msg_"),
            response_type: "message".to_string(),
            role: Role::Assistant,
            model: model.into(),
            content,
            stop_reason: Some(stop_reason),
            stop_sequence: None,
            usage,
        }
    }
}

/// Anthropic-style error response: `{"type":"error","error":{...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicErrorResponse {
    #[serde(rename = "type")]
    pub response_type: String,
    pub error: AnthropicErrorDetail,
}

/// The inner `error` object of an Anthropic error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

impl AnthropicErrorResponse {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            response_type: "error".to_string(),
            error: AnthropicErrorDetail {
                error_type: error_type.into(),
                message: message.into(),
            },
        }
    }

    /// Map an HTTP status code to the canonical Anthropic error `type`.
    /// Reference: <https://docs.anthropic.com/en/api/errors>
    pub fn type_for_status(status: u16) -> &'static str {
        match status {
            400 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            413 => "request_too_large",
            429 => "rate_limit_error",
            500 => "api_error",
            529 => "overloaded_error",
            _ => "api_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_deserialize_string_content() {
        let json = r#"{
            "model": "claude-opus-4-8",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        }"#;
        let req: MessagesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "claude-opus-4-8");
        assert_eq!(req.max_tokens, 1024);
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].content.extract_text(), "Hello");
    }

    #[test]
    fn test_request_deserialize_block_content() {
        let json = r#"{
            "model": "claude-sonnet-4-6",
            "max_tokens": 512,
            "system": "You are helpful.",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Part one"},
                    {"type": "text", "text": "Part two"}
                ]}
            ]
        }"#;
        let req: MessagesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.system.as_ref().unwrap().extract_text(),
            "You are helpful."
        );
        assert_eq!(req.messages[0].content.extract_text(), "Part one Part two");
        assert!(req.prompt_text().contains("You are helpful."));
        assert!(req.prompt_text().contains("user: Part one Part two"));
    }

    #[test]
    fn test_request_tolerates_unknown_blocks() {
        // image block should not break deserialization
        let json = r#"{
            "model": "claude-opus-4-8",
            "max_tokens": 64,
            "messages": [
                {"role": "user", "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}},
                    {"type": "text", "text": "What is this?"}
                ]}
            ]
        }"#;
        let req: MessagesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.messages[0].content.extract_text(), "What is this?");
    }

    #[test]
    fn test_response_serialize_shape() {
        let resp = MessagesResponse::text("claude-opus-4-8", "Hi there", Usage::new(5, 3));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"role\":\"assistant\""));
        assert!(json.contains("\"stop_reason\":\"end_turn\""));
        assert!(json.contains("\"input_tokens\":5"));
        assert!(json.contains("\"type\":\"text\""));
        assert!(resp.id.starts_with("msg_"));
    }

    #[test]
    fn test_tool_use_block_serialize() {
        let resp = MessagesResponse::with_content(
            "claude-opus-4-8",
            vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "get_weather".to_string(),
                input: serde_json::json!({"location": "Paris"}),
            }],
            StopReason::ToolUse,
            Usage::new(10, 8),
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"tool_use\""));
        assert!(json.contains("\"name\":\"get_weather\""));
        assert!(json.contains("\"stop_reason\":\"tool_use\""));
    }

    #[test]
    fn test_error_response_shape() {
        let err = AnthropicErrorResponse::new("rate_limit_error", "slow down");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("\"type\":\"rate_limit_error\""));
        assert!(json.contains("\"message\":\"slow down\""));
    }

    #[test]
    fn test_type_for_status() {
        assert_eq!(
            AnthropicErrorResponse::type_for_status(429),
            "rate_limit_error"
        );
        assert_eq!(AnthropicErrorResponse::type_for_status(500), "api_error");
        assert_eq!(
            AnthropicErrorResponse::type_for_status(400),
            "invalid_request_error"
        );
        assert_eq!(
            AnthropicErrorResponse::type_for_status(529),
            "overloaded_error"
        );
    }
}
