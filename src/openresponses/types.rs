// OpenResponses API Types
// These types are designed to be compatible with the Open Responses API specification.
// Reference: https://www.openresponses.org/specification

use serde::{Deserialize, Serialize};

/// Role of a message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Developer,
}

/// Input content item type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    InputText,
    InputImage,
    InputFile,
    OutputText,
}

/// Content item in an input message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentItem {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "input_image")]
    InputImage {
        #[serde(skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    #[serde(rename = "input_file")]
    InputFile {
        file_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

/// Input message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputMessage {
    pub role: Role,
    pub content: MessageContent,
}

/// Message content can be a string or array of content items
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentItem>),
}

impl MessageContent {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(s) => Some(s),
            MessageContent::Parts(parts) => {
                for part in parts {
                    if let ContentItem::InputText { text } = part {
                        return Some(text);
                    }
                }
                None
            }
        }
    }
}

/// Input can be a simple string or an array of messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Input {
    Text(String),
    Messages(Vec<InputMessage>),
}

impl Input {
    /// Extract text content from input for token counting
    pub fn extract_text(&self) -> String {
        match self {
            Input::Text(s) => s.clone(),
            Input::Messages(messages) => messages
                .iter()
                .filter_map(|m| m.content.as_text())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Convert to messages for processing
    pub fn to_messages(&self) -> Vec<InputMessage> {
        match self {
            Input::Text(s) => vec![InputMessage {
                role: Role::User,
                content: MessageContent::Text(s.clone()),
            }],
            Input::Messages(messages) => messages.clone(),
        }
    }
}

/// Function definition for tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Tool {
    #[serde(rename = "function")]
    Function {
        function: FunctionDefinition,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
    #[serde(rename = "web_search")]
    WebSearch {
        #[serde(skip_serializing_if = "Option::is_none")]
        search_context_size: Option<String>,
    },
    #[serde(rename = "file_search")]
    FileSearch {
        #[serde(skip_serializing_if = "Option::is_none")]
        vector_store_ids: Option<Vec<String>>,
    },
    #[serde(rename = "code_interpreter")]
    CodeInterpreter,
}

/// Tool choice option
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String), // "auto", "required", "none"
    Specific {
        #[serde(rename = "type")]
        tool_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        function: Option<ToolChoiceFunction>,
    },
}

/// Function specification for tool choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

/// Reasoning configuration for models that support it
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>, // "none", "low", "medium", "high", "xhigh"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>, // "auto", "concise", "detailed"
}

/// Truncation strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Truncation {
    Mode(String), // "auto", "disabled"
}

/// Response request to the OpenResponses API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseRequest {
    pub model: String,
    pub input: Input,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<Truncation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

/// Response status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Incomplete,
}

/// Output item type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputItemType {
    Message,
    FunctionCall,
    FunctionCallOutput,
    Reasoning,
}

/// Function call in output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Text content in output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<serde_json::Value>>,
}

/// Output message content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputContent {
    #[serde(rename = "output_text")]
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<serde_json::Value>>,
    },
}

/// Output item in response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItem {
    #[serde(rename = "message")]
    Message {
        id: String,
        role: Role,
        content: Vec<OutputContent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<ReasoningSummary>>,
    },
}

/// Reasoning summary in output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSummary {
    #[serde(rename = "type")]
    pub summary_type: String,
    pub text: String,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

/// Input token details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

/// Output token details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

/// Complete response from the OpenResponses API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub model: String,
    pub status: ResponseStatus,
    pub output: Vec<OutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

impl Response {
    pub fn new(model: String, content: String, usage: Usage) -> Self {
        let created_at = chrono::Utc::now().timestamp();
        let id = format!("resp_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));
        let item_id = format!("msg_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));

        Self {
            id,
            object: "response".to_string(),
            created_at,
            completed_at: Some(created_at),
            model,
            status: ResponseStatus::Completed,
            output: vec![OutputItem::Message {
                id: item_id,
                role: Role::Assistant,
                content: vec![OutputContent::OutputText {
                    text: content,
                    annotations: None,
                }],
                status: Some("completed".to_string()),
            }],
            usage: Some(usage),
            metadata: None,
            error: None,
        }
    }
}

/// Error information in response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
}

/// OpenResponses-style error response
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

// ============================================================================
// Streaming Types
// ============================================================================

/// Event types for streaming responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StreamEventType {
    // Lifecycle events
    #[serde(rename = "response.created")]
    ResponseCreated,
    #[serde(rename = "response.in_progress")]
    ResponseInProgress,
    #[serde(rename = "response.completed")]
    ResponseCompleted,
    #[serde(rename = "response.failed")]
    ResponseFailed,

    // Output item events
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded,
    #[serde(rename = "response.output_item.done")]
    OutputItemDone,

    // Content events
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded,
    #[serde(rename = "response.content_part.done")]
    ContentPartDone,

    // Delta events
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta,
    #[serde(rename = "response.output_text.done")]
    OutputTextDone,

    // Function call events
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta,
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone,

    // Error event
    #[serde(rename = "error")]
    Error,
}

/// Stream event for OpenResponses streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: StreamEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Response>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<OutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<OutputContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

impl StreamEvent {
    /// Create a response.created event
    pub fn response_created(response: Response) -> Self {
        Self {
            event_type: StreamEventType::ResponseCreated,
            response: Some(response),
            output_index: None,
            content_index: None,
            item: None,
            part: None,
            delta: None,
            text: None,
            error: None,
        }
    }

    /// Create a response.in_progress event
    pub fn response_in_progress(response: Response) -> Self {
        Self {
            event_type: StreamEventType::ResponseInProgress,
            response: Some(response),
            output_index: None,
            content_index: None,
            item: None,
            part: None,
            delta: None,
            text: None,
            error: None,
        }
    }

    /// Create a response.output_item.added event
    pub fn output_item_added(output_index: usize, item: OutputItem) -> Self {
        Self {
            event_type: StreamEventType::OutputItemAdded,
            response: None,
            output_index: Some(output_index),
            content_index: None,
            item: Some(item),
            part: None,
            delta: None,
            text: None,
            error: None,
        }
    }

    /// Create a response.content_part.added event
    pub fn content_part_added(
        output_index: usize,
        content_index: usize,
        part: OutputContent,
    ) -> Self {
        Self {
            event_type: StreamEventType::ContentPartAdded,
            response: None,
            output_index: Some(output_index),
            content_index: Some(content_index),
            item: None,
            part: Some(part),
            delta: None,
            text: None,
            error: None,
        }
    }

    /// Create a response.output_text.delta event
    pub fn output_text_delta(output_index: usize, content_index: usize, delta: String) -> Self {
        Self {
            event_type: StreamEventType::OutputTextDelta,
            response: None,
            output_index: Some(output_index),
            content_index: Some(content_index),
            item: None,
            part: None,
            delta: Some(delta),
            text: None,
            error: None,
        }
    }

    /// Create a response.output_text.done event
    pub fn output_text_done(output_index: usize, content_index: usize, text: String) -> Self {
        Self {
            event_type: StreamEventType::OutputTextDone,
            response: None,
            output_index: Some(output_index),
            content_index: Some(content_index),
            item: None,
            part: None,
            delta: None,
            text: Some(text),
            error: None,
        }
    }

    /// Create a response.content_part.done event
    pub fn content_part_done(
        output_index: usize,
        content_index: usize,
        part: OutputContent,
    ) -> Self {
        Self {
            event_type: StreamEventType::ContentPartDone,
            response: None,
            output_index: Some(output_index),
            content_index: Some(content_index),
            item: None,
            part: Some(part),
            delta: None,
            text: None,
            error: None,
        }
    }

    /// Create a response.output_item.done event
    pub fn output_item_done(output_index: usize, item: OutputItem) -> Self {
        Self {
            event_type: StreamEventType::OutputItemDone,
            response: None,
            output_index: Some(output_index),
            content_index: None,
            item: Some(item),
            part: None,
            delta: None,
            text: None,
            error: None,
        }
    }

    /// Create a response.completed event
    pub fn response_completed(response: Response) -> Self {
        Self {
            event_type: StreamEventType::ResponseCompleted,
            response: Some(response),
            output_index: None,
            content_index: None,
            item: None,
            part: None,
            delta: None,
            text: None,
            error: None,
        }
    }
}

/// Format a stream event as Server-Sent Event
pub fn format_sse(event: &StreamEvent) -> String {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    format!("data: {}\n\n", json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_text_parsing() {
        let json = r#""Hello, world!""#;
        let input: Input = serde_json::from_str(json).unwrap();
        match input {
            Input::Text(s) => assert_eq!(s, "Hello, world!"),
            _ => panic!("Expected text input"),
        }
    }

    #[test]
    fn test_input_messages_parsing() {
        let json = r#"[
            {"role": "user", "content": "Hello!"},
            {"role": "assistant", "content": "Hi there!"}
        ]"#;
        let input: Input = serde_json::from_str(json).unwrap();
        match input {
            Input::Messages(msgs) => {
                assert_eq!(msgs.len(), 2);
                assert_eq!(msgs[0].role, Role::User);
            }
            _ => panic!("Expected messages input"),
        }
    }

    #[test]
    fn test_response_request_deserialization() {
        let json = r#"{
            "model": "gpt-5",
            "input": "What is the capital of France?",
            "temperature": 0.7,
            "stream": false
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-5");
        assert_eq!(request.temperature, Some(0.7));
        assert!(!request.stream);
    }

    #[test]
    fn test_response_request_with_messages() {
        let json = r#"{
            "model": "gpt-5",
            "input": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "Hello!"}
            ],
            "stream": true
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-5");
        assert!(request.stream);
        match request.input {
            Input::Messages(msgs) => assert_eq!(msgs.len(), 2),
            _ => panic!("Expected messages input"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            input_tokens_details: None,
            output_tokens_details: None,
        };
        let response = Response::new(
            "gpt-5".to_string(),
            "Hello! How can I help?".to_string(),
            usage,
        );

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"object\":\"response\""));
        assert!(json.contains("\"model\":\"gpt-5\""));
        assert!(json.contains("\"status\":\"completed\""));
    }

    #[test]
    fn test_error_response() {
        let error = ErrorResponse::rate_limit();
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"type\":\"rate_limit_error\""));
        assert!(json.contains("\"code\":\"rate_limit_exceeded\""));
    }

    #[test]
    fn test_tool_function_parsing() {
        let json = r#"{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the current weather",
                "parameters": {"type": "object"}
            }
        }"#;

        let tool: Tool = serde_json::from_str(json).unwrap();
        match tool {
            Tool::Function { function, .. } => {
                assert_eq!(function.name, "get_weather");
            }
            _ => panic!("Expected function tool"),
        }
    }

    #[test]
    fn test_stream_event_serialization() {
        let event = StreamEvent::output_text_delta(0, 0, "Hello".to_string());
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"response.output_text.delta\""));
        assert!(json.contains("\"delta\":\"Hello\""));
    }

    #[test]
    fn test_reasoning_config() {
        let json = r#"{
            "model": "o3",
            "input": "Solve this math problem",
            "reasoning": {
                "effort": "high",
                "summary": "detailed"
            }
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert!(request.reasoning.is_some());
        let reasoning = request.reasoning.unwrap();
        assert_eq!(reasoning.effort, Some("high".to_string()));
    }

    #[test]
    fn test_content_item_parsing() {
        let json = r#"[
            {"role": "user", "content": [
                {"type": "input_text", "text": "What's in this image?"},
                {"type": "input_image", "image_url": "https://example.com/image.jpg"}
            ]}
        ]"#;

        let input: Input = serde_json::from_str(json).unwrap();
        match input {
            Input::Messages(msgs) => {
                assert_eq!(msgs.len(), 1);
                match &msgs[0].content {
                    MessageContent::Parts(parts) => assert_eq!(parts.len(), 2),
                    _ => panic!("Expected parts content"),
                }
            }
            _ => panic!("Expected messages input"),
        }
    }
}
