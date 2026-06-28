// OpenAI API Types
// These types are designed to be compatible with the OpenAI Chat Completions API.
// Reference: https://platform.openai.com/docs/api-reference/chat

use crate::ids::{prefixed_id, unix_timestamp};
use serde::{Deserialize, Serialize};

/// Role of a message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    Function,
}

/// Image reference for a Chat Completions `image_url` content part.
/// Matches the Chat Completions wire format where `image_url` is an object
/// (`{"url": "...", "detail": "..."}`), unlike the Responses API where it is a
/// bare string. Keeping the shapes faithful preserves official-SDK compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A single part of a multimodal Chat Completions message content array.
/// Only the modalities the simulator reasons about are modeled; richer parts
/// (audio, files) can be added alongside a matching capability gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatContentPart {
    /// Plain text segment (`{"type": "text", "text": "..."}`).
    Text { text: String },
    /// Image segment (`{"type": "image_url", "image_url": {"url": "..."}}`).
    ImageUrl { image_url: ImageUrl },
}

/// Message content: either a plain string or an array of content parts.
/// The string form is preserved on the wire (untagged) so non-multimodal
/// requests and all assistant responses serialize as a bare `"content": "..."`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatMessageContent {
    /// Simple text content.
    Text(String),
    /// Array of content parts (multimodal input).
    Parts(Vec<ChatContentPart>),
}

impl ChatMessageContent {
    /// Concatenate all text segments. Non-text parts (images) contribute nothing,
    /// which is what the generator and token counter consume today.
    pub fn text(&self) -> String {
        match self {
            ChatMessageContent::Text(s) => s.clone(),
            ChatMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ChatContentPart::Text { text } => Some(text.as_str()),
                    ChatContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Image references carried by this message, in order.
    pub fn images(&self) -> Vec<&ImageUrl> {
        match self {
            ChatMessageContent::Text(_) => Vec::new(),
            ChatMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ChatContentPart::ImageUrl { image_url } => Some(image_url),
                    ChatContentPart::Text { .. } => None,
                })
                .collect(),
        }
    }

    /// Whether this message contains any image parts.
    pub fn has_images(&self) -> bool {
        matches!(self, ChatMessageContent::Parts(parts)
            if parts.iter().any(|p| matches!(p, ChatContentPart::ImageUrl { .. })))
    }
}

impl From<String> for ChatMessageContent {
    fn from(s: String) -> Self {
        ChatMessageContent::Text(s)
    }
}

/// A message in a chat conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Some(ChatMessageContent::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Some(ChatMessageContent::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Some(ChatMessageContent::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

/// A tool call made by the assistant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// A function call within a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// A function definition for function calling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

/// A tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: Function,
}

/// Tool choice option
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    String(String),
    Object {
        #[serde(rename = "type")]
        choice_type: String,
        function: ToolChoiceFunction,
    },
}

/// Function specification for tool choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

/// Response format specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub format_type: String,
}

/// Chat completion request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<std::collections::HashMap<String, f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

/// Stop condition for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopCondition {
    Single(String),
    Multiple(Vec<String>),
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// A choice in the completion response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<serde_json::Value>,
}

/// Chat completion response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

impl ChatCompletionResponse {
    pub fn new(model: String, content: String, usage: Usage) -> Self {
        Self {
            id: prefixed_id("chatcmpl-"),
            object: "chat.completion".to_string(),
            created: unix_timestamp(),
            model,
            choices: vec![Choice {
                index: 0,
                message: Message::assistant(content),
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: Some(usage),
            system_fingerprint: Some("fp_llmsim".to_string()),
        }
    }
}

/// Delta content in streaming response
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChunkToolCall>>,
}

/// Tool call in streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkToolCall {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<ChunkFunctionCall>,
}

/// Function call in streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkFunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// A choice in streaming response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: ChunkDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<serde_json::Value>,
}

/// Streaming chat completion chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl ChatCompletionChunk {
    pub fn new(id: String, model: String, created: i64) -> Self {
        Self {
            id,
            object: "chat.completion.chunk".to_string(),
            created,
            model,
            choices: vec![],
            system_fingerprint: Some("fp_llmsim".to_string()),
            usage: None,
        }
    }

    pub fn with_role(mut self) -> Self {
        self.choices = vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: Some(Role::Assistant),
                content: None,
                tool_calls: None,
            },
            finish_reason: None,
            logprobs: None,
        }];
        self
    }

    pub fn with_content(mut self, content: String) -> Self {
        self.choices = vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: Some(content),
                tool_calls: None,
            },
            finish_reason: None,
            logprobs: None,
        }];
        self
    }

    pub fn with_finish(mut self, reason: String) -> Self {
        self.choices = vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta::default(),
            finish_reason: Some(reason),
            logprobs: None,
        }];
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
}

/// OpenAI-style error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

/// Error detail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ErrorResponse {
    pub fn new(message: impl Into<String>, error_type: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                message: message.into(),
                error_type: error_type.into(),
                param: None,
                code: None,
            },
        }
    }

    pub fn rate_limit() -> Self {
        Self {
            error: ErrorDetail {
                message: "Rate limit exceeded. Please retry after some time.".to_string(),
                error_type: "rate_limit_error".to_string(),
                param: None,
                code: Some("rate_limit_exceeded".to_string()),
            },
        }
    }

    pub fn server_error() -> Self {
        Self {
            error: ErrorDetail {
                message: "The server had an error processing your request.".to_string(),
                error_type: "server_error".to_string(),
                param: None,
                code: Some("server_error".to_string()),
            },
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                message: message.into(),
                error_type: "invalid_request_error".to_string(),
                param: None,
                code: None,
            },
        }
    }
}

/// Model object returned by /openai/v1/models endpoint
/// Extended with context_window and max_output_tokens from models.dev profiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
    /// Maximum context window size in tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    /// Maximum output tokens per request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

impl Model {
    /// Create a basic model (backward compatible)
    pub fn new(id: impl Into<String>, owned_by: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            object: "model".to_string(),
            created: unix_timestamp(),
            owned_by: owned_by.into(),
            context_window: None,
            max_output_tokens: None,
        }
    }

    /// Create a model with full profile information
    pub fn from_profile(profile: &super::models::ModelProfile) -> Self {
        Self {
            id: profile.id.clone(),
            object: "model".to_string(),
            created: profile.created,
            owned_by: profile.owned_by.clone(),
            context_window: Some(profile.context_window),
            max_output_tokens: Some(profile.max_output_tokens),
        }
    }
}

/// Response for /openai/v1/models endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<Model>,
}

impl ModelsResponse {
    pub fn new(models: Vec<Model>) -> Self {
        Self {
            object: "list".to_string(),
            data: models,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("Hello, world!");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello, world!\""));
    }

    #[test]
    fn test_multimodal_content_deserialization() {
        let json = r#"{
            "role": "user",
            "content": [
                {"type": "text", "text": "What's in this image?"},
                {"type": "image_url", "image_url": {"url": "https://example.com/cat.png", "detail": "high"}}
            ]
        }"#;

        let msg: Message = serde_json::from_str(json).unwrap();
        let content = msg.content.expect("content present");

        assert_eq!(content.text(), "What's in this image?");
        assert!(content.has_images());
        let images = content.images();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://example.com/cat.png");
        assert_eq!(images[0].detail.as_deref(), Some("high"));
    }

    #[test]
    fn test_string_content_has_no_images() {
        let json = r#"{"role": "user", "content": "just text"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();
        let content = msg.content.expect("content present");
        assert_eq!(content.text(), "just text");
        assert!(!content.has_images());
        assert!(content.images().is_empty());
    }

    #[test]
    fn test_string_content_roundtrips_as_bare_string() {
        // The untagged string form must serialize back to a bare string so
        // assistant responses and non-multimodal requests stay SDK-compatible.
        let msg = Message::assistant("hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"content\":\"hello\""));
    }

    #[test]
    fn test_chat_request_deserialization() {
        let json = r#"{
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "Hello!"}
            ],
            "temperature": 0.7,
            "stream": true
        }"#;

        let request: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-4");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.temperature, Some(0.7));
        assert!(request.stream);
    }

    #[test]
    fn test_chat_response_serialization() {
        let usage = Usage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
        };
        let response = ChatCompletionResponse::new(
            "gpt-4".to_string(),
            "Hello! How can I help you?".to_string(),
            usage,
        );

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"object\":\"chat.completion\""));
        assert!(json.contains("\"model\":\"gpt-4\""));
        assert!(json.contains("\"finish_reason\":\"stop\""));
    }

    #[test]
    fn test_streaming_chunk() {
        let chunk =
            ChatCompletionChunk::new("chatcmpl-test".to_string(), "gpt-4".to_string(), 1234567890)
                .with_content("Hello".to_string());

        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("\"object\":\"chat.completion.chunk\""));
        assert!(json.contains("\"content\":\"Hello\""));
    }

    #[test]
    fn test_error_response() {
        let error = ErrorResponse::rate_limit();
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"type\":\"rate_limit_error\""));
        assert!(json.contains("\"code\":\"rate_limit_exceeded\""));
    }

    #[test]
    fn test_tool_call() {
        let json = r#"{
            "id": "call_abc123",
            "type": "function",
            "function": {
                "name": "get_weather",
                "arguments": "{\"location\": \"Boston\"}"
            }
        }"#;

        let tool_call: ToolCall = serde_json::from_str(json).unwrap();
        assert_eq!(tool_call.id, "call_abc123");
        assert_eq!(tool_call.function.name, "get_weather");
    }

    #[test]
    fn test_models_response() {
        let models = vec![
            Model::new("gpt-4", "openai"),
            Model::new("gpt-3.5-turbo", "openai"),
        ];
        let response = ModelsResponse::new(models);
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"object\":\"list\""));
        assert!(json.contains("\"id\":\"gpt-4\""));
    }
}
