// OpenAI Responses API Types
// These types are designed to be compatible with the OpenAI Responses API.
// Reference: https://platform.openai.com/docs/api-reference/responses

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Input for a Responses API request - can be a string or array of items
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    /// Simple text input
    Text(String),
    /// Array of input items (messages, etc.)
    Items(Vec<InputItem>),
}

/// An input item in the Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    /// A message input item
    Message {
        role: InputRole,
        content: MessageContent,
    },
    /// A function call result (tool output)
    FunctionCallOutput { call_id: String, output: String },
}

/// Role for input messages
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InputRole {
    User,
    Assistant,
    System,
    Developer,
}

/// Message content - can be a string or array of content parts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content
    Text(String),
    /// Array of content parts
    Parts(Vec<ContentPart>),
}

/// A content part in a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content
    InputText { text: String },
    /// Image content
    InputImage { image_url: String },
    /// File content
    InputFile {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
    },
}

/// Responses API request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesRequest {
    /// Model to use for generation
    pub model: String,
    /// Input text or array of input items
    pub input: ResponsesInput,
    /// System instructions for this request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Sampling temperature (0.0 - 2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Nucleus sampling parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Enable streaming response
    #[serde(default)]
    pub stream: bool,
    /// Custom metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    /// Chain to a previous response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Tools available for the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesTool>>,
    /// Control tool usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ResponsesToolChoice>,
}

/// A tool definition for the Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesTool {
    /// Function tool
    Function {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<serde_json::Value>,
    },
    /// Web search tool
    WebSearch {},
    /// File search tool
    FileSearch {},
    /// Code interpreter tool
    CodeInterpreter {},
}

/// Tool choice option for Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesToolChoice {
    /// String value: "auto", "none", "required"
    String(String),
    /// Specific function
    Function { r#type: String, name: String },
}

/// Response status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Completed,
    Failed,
    InProgress,
    Queued,
    Incomplete,
}

/// Responses API response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesResponse {
    /// Unique response identifier
    pub id: String,
    /// Object type (always "response")
    pub object: String,
    /// Creation timestamp
    pub created_at: i64,
    /// Model used
    pub model: String,
    /// Response status
    pub status: ResponseStatus,
    /// Output items
    pub output: Vec<OutputItem>,
    /// Simplified text output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    /// Token usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponsesUsage>,
    /// Error information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponsesError>,
    /// Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

impl ResponsesResponse {
    pub fn new(model: String, content: String, usage: ResponsesUsage) -> Self {
        let output_item = OutputItem::Message {
            id: format!("msg_{}", uuid::Uuid::new_v4()),
            role: OutputRole::Assistant,
            status: ItemStatus::Completed,
            content: vec![OutputContentPart::OutputText {
                text: content.clone(),
            }],
        };

        Self {
            id: format!("resp_{}", uuid::Uuid::new_v4()),
            object: "response".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            model,
            status: ResponseStatus::Completed,
            output: vec![output_item],
            output_text: Some(content),
            usage: Some(usage),
            error: None,
            metadata: None,
        }
    }
}

/// An output item in the response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    /// A message output
    Message {
        id: String,
        role: OutputRole,
        status: ItemStatus,
        content: Vec<OutputContentPart>,
    },
    /// A function call
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        status: ItemStatus,
    },
    /// Reasoning output (for reasoning models)
    Reasoning {
        id: String,
        status: ItemStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<ReasoningSummary>>,
    },
}

/// Role for output messages
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputRole {
    Assistant,
}

/// Status of an output item
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Completed,
    InProgress,
    Failed,
}

/// An output content part
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContentPart {
    /// Text output
    OutputText { text: String },
    /// Refusal output
    Refusal { refusal: String },
}

/// Reasoning summary for reasoning models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSummary {
    #[serde(rename = "type")]
    pub summary_type: String,
    pub text: String,
}

/// Token usage for Responses API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

/// Details about output token usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    pub reasoning_tokens: u32,
}

/// Error in Responses API format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ResponsesError {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error_type: error_type.into(),
            message: message.into(),
            code: None,
        }
    }

    pub fn rate_limit() -> Self {
        Self {
            error_type: "rate_limit_error".to_string(),
            message: "Rate limit exceeded. Please retry after some time.".to_string(),
            code: Some("rate_limit_exceeded".to_string()),
        }
    }

    pub fn server_error() -> Self {
        Self {
            error_type: "server_error".to_string(),
            message: "The server had an error processing your request.".to_string(),
            code: Some("server_error".to_string()),
        }
    }
}

/// Responses API error response (for HTTP error responses)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesErrorResponse {
    pub error: ResponsesError,
}

// ============================================================================
// Streaming Types
// ============================================================================

/// Base streaming event structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub data: StreamEventData,
}

/// Data payload for different event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StreamEventData {
    /// Response lifecycle events
    Response(ResponseEventData),
    /// Output item events
    OutputItem(OutputItemEventData),
    /// Content part events
    ContentPart(ContentPartEventData),
    /// Text delta events
    TextDelta(TextDeltaEventData),
    /// Error events
    Error(ErrorEventData),
}

/// Data for response lifecycle events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseEventData {
    pub response: ResponsesResponse,
}

/// Data for output item events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputItemEventData {
    pub output_index: u32,
    pub item: OutputItem,
}

/// Data for content part events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPartEventData {
    pub output_index: u32,
    pub content_index: u32,
    pub part: OutputContentPart,
}

/// Data for text delta events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDeltaEventData {
    pub output_index: u32,
    pub content_index: u32,
    pub delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<u32>,
}

/// Data for error events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEventData {
    pub error: ResponsesError,
}

/// Helper to create streaming events
pub struct ResponsesStreamEvent;

impl ResponsesStreamEvent {
    pub fn response_created(response: ResponsesResponse) -> String {
        let event = serde_json::json!({
            "type": "response.created",
            "response": response
        });
        format!("event: response.created\ndata: {}\n\n", event)
    }

    pub fn response_in_progress(response: ResponsesResponse) -> String {
        let event = serde_json::json!({
            "type": "response.in_progress",
            "response": response
        });
        format!("event: response.in_progress\ndata: {}\n\n", event)
    }

    pub fn output_item_added(output_index: u32, item: &OutputItem) -> String {
        let event = serde_json::json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": item
        });
        format!("event: response.output_item.added\ndata: {}\n\n", event)
    }

    pub fn content_part_added(
        output_index: u32,
        content_index: u32,
        part: &OutputContentPart,
    ) -> String {
        let event = serde_json::json!({
            "type": "response.content_part.added",
            "output_index": output_index,
            "content_index": content_index,
            "part": part
        });
        format!("event: response.content_part.added\ndata: {}\n\n", event)
    }

    pub fn output_text_delta(
        output_index: u32,
        content_index: u32,
        delta: &str,
        sequence_number: u32,
    ) -> String {
        let event = serde_json::json!({
            "type": "response.output_text.delta",
            "output_index": output_index,
            "content_index": content_index,
            "delta": delta,
            "sequence_number": sequence_number
        });
        format!("event: response.output_text.delta\ndata: {}\n\n", event)
    }

    pub fn output_text_done(output_index: u32, content_index: u32, text: &str) -> String {
        let event = serde_json::json!({
            "type": "response.output_text.done",
            "output_index": output_index,
            "content_index": content_index,
            "text": text
        });
        format!("event: response.output_text.done\ndata: {}\n\n", event)
    }

    pub fn content_part_done(
        output_index: u32,
        content_index: u32,
        part: &OutputContentPart,
    ) -> String {
        let event = serde_json::json!({
            "type": "response.content_part.done",
            "output_index": output_index,
            "content_index": content_index,
            "part": part
        });
        format!("event: response.content_part.done\ndata: {}\n\n", event)
    }

    pub fn output_item_done(output_index: u32, item: &OutputItem) -> String {
        let event = serde_json::json!({
            "type": "response.output_item.done",
            "output_index": output_index,
            "item": item
        });
        format!("event: response.output_item.done\ndata: {}\n\n", event)
    }

    pub fn response_completed(response: ResponsesResponse) -> String {
        let event = serde_json::json!({
            "type": "response.completed",
            "response": response
        });
        format!("event: response.completed\ndata: {}\n\n", event)
    }

    pub fn error(error: ResponsesError) -> String {
        let event = serde_json::json!({
            "type": "error",
            "error": error
        });
        format!("event: error\ndata: {}\n\n", event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_responses_input_text() {
        let json = r#""What is the capital of France?""#;
        let input: ResponsesInput = serde_json::from_str(json).unwrap();
        match input {
            ResponsesInput::Text(s) => assert_eq!(s, "What is the capital of France?"),
            _ => panic!("Expected Text variant"),
        }
    }

    #[test]
    fn test_responses_input_items() {
        let json = r#"[
            {"type": "message", "role": "user", "content": "Hello!"}
        ]"#;
        let input: ResponsesInput = serde_json::from_str(json).unwrap();
        match input {
            ResponsesInput::Items(items) => {
                assert_eq!(items.len(), 1);
            }
            _ => panic!("Expected Items variant"),
        }
    }

    #[test]
    fn test_responses_request_simple() {
        let json = r#"{
            "model": "gpt-5",
            "input": "Tell me a story"
        }"#;
        let request: ResponsesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-5");
        assert!(!request.stream);
    }

    #[test]
    fn test_responses_request_with_messages() {
        let json = r#"{
            "model": "gpt-5",
            "input": [
                {"type": "message", "role": "user", "content": "Hello!"},
                {"type": "message", "role": "assistant", "content": "Hi there!"}
            ],
            "temperature": 0.7,
            "stream": true
        }"#;
        let request: ResponsesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-5");
        assert_eq!(request.temperature, Some(0.7));
        assert!(request.stream);
    }

    #[test]
    fn test_responses_response_new() {
        let usage = ResponsesUsage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            output_tokens_details: None,
        };
        let response = ResponsesResponse::new("gpt-5".to_string(), "Hello!".to_string(), usage);

        assert_eq!(response.object, "response");
        assert_eq!(response.model, "gpt-5");
        assert_eq!(response.status, ResponseStatus::Completed);
        assert_eq!(response.output.len(), 1);
        assert_eq!(response.output_text, Some("Hello!".to_string()));
    }

    #[test]
    fn test_responses_response_serialization() {
        let usage = ResponsesUsage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 0,
            }),
        };
        let response =
            ResponsesResponse::new("gpt-5".to_string(), "Test response".to_string(), usage);

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"object\":\"response\""));
        assert!(json.contains("\"status\":\"completed\""));
        assert!(json.contains("\"output_text\":\"Test response\""));
    }

    #[test]
    fn test_content_part_types() {
        let json = r#"{"type": "input_text", "text": "Hello"}"#;
        let part: ContentPart = serde_json::from_str(json).unwrap();
        match part {
            ContentPart::InputText { text } => assert_eq!(text, "Hello"),
            _ => panic!("Expected InputText variant"),
        }
    }

    #[test]
    fn test_stream_event_creation() {
        let delta = ResponsesStreamEvent::output_text_delta(0, 0, "Hello", 1);
        assert!(delta.contains("event: response.output_text.delta"));
        assert!(delta.contains("\"delta\":\"Hello\""));
        assert!(delta.contains("\"sequence_number\":1"));
    }

    #[test]
    fn test_error_response() {
        let error = ResponsesError::rate_limit();
        let error_response = ResponsesErrorResponse { error };
        let json = serde_json::to_string(&error_response).unwrap();
        assert!(json.contains("\"type\":\"rate_limit_error\""));
    }
}
