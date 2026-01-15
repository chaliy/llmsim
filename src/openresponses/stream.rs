// OpenResponses Streaming Engine Module
// Implements token-by-token streaming with realistic latency simulation
// following the OpenResponses specification.

use super::types::{
    format_sse, OutputContent, OutputItem, Response, ResponseStatus, Role, StreamEvent, Usage,
};
use crate::latency::LatencyProfile;
use async_stream::stream;
use futures::Stream;
use std::pin::Pin;
use tokio::time::sleep;

/// Callback type for stream completion
type OnCompleteCallback = Box<dyn FnOnce() + Send + 'static>;

/// A streaming response that yields OpenResponses events with simulated delays
pub struct OpenResponsesTokenStream {
    /// The response ID (shared across all events)
    id: String,
    /// The model name
    model: String,
    /// Unix timestamp of creation
    created_at: i64,
    /// Latency profile for timing simulation
    latency: LatencyProfile,
    /// The full response text to stream
    content: String,
    /// Token usage (included in final event)
    usage: Option<Usage>,
    /// Callback to invoke when stream completes
    on_complete: Option<OnCompleteCallback>,
}

impl OpenResponsesTokenStream {
    pub fn new(id: String, model: String, content: String, latency: LatencyProfile) -> Self {
        Self {
            id,
            model,
            created_at: chrono::Utc::now().timestamp(),
            latency,
            content,
            usage: None,
            on_complete: None,
        }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_on_complete<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.on_complete = Some(Box::new(callback));
        self
    }

    /// Convert the content into chunks for streaming
    fn tokenize(&self) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current_word = String::new();

        for ch in self.content.chars() {
            if ch.is_whitespace() {
                if !current_word.is_empty() {
                    tokens.push(current_word.clone());
                    current_word.clear();
                }
                tokens.push(ch.to_string());
            } else {
                current_word.push(ch);
            }
        }

        if !current_word.is_empty() {
            tokens.push(current_word);
        }

        tokens
    }

    /// Create a streaming response as Server-Sent Events following OpenResponses format
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        let tokens = self.tokenize();
        let id = self.id.clone();
        let model = self.model.clone();
        let created_at = self.created_at;
        let latency = self.latency.clone();
        let usage = self.usage.clone();
        let on_complete = self.on_complete;

        Box::pin(stream! {
            // Generate IDs for the output items
            let item_id = format!("msg_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));

            // Initial delay (time to first token)
            let ttft = latency.sample_ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }

            // 1. response.created event
            let created_response = Response {
                id: id.clone(),
                object: "response".to_string(),
                created_at,
                completed_at: None,
                model: model.clone(),
                status: ResponseStatus::InProgress,
                output: vec![],
                usage: None,
                metadata: None,
                error: None,
            };
            yield format_sse(&StreamEvent::response_created(created_response));

            // 2. response.in_progress event
            let in_progress_response = Response {
                id: id.clone(),
                object: "response".to_string(),
                created_at,
                completed_at: None,
                model: model.clone(),
                status: ResponseStatus::InProgress,
                output: vec![],
                usage: None,
                metadata: None,
                error: None,
            };
            yield format_sse(&StreamEvent::response_in_progress(in_progress_response));

            // 3. response.output_item.added event
            let output_item = OutputItem::Message {
                id: item_id.clone(),
                role: Role::Assistant,
                content: vec![],
                status: Some("in_progress".to_string()),
            };
            yield format_sse(&StreamEvent::output_item_added(0, output_item));

            // 4. response.content_part.added event
            let content_part = OutputContent::OutputText {
                text: String::new(),
                annotations: None,
            };
            yield format_sse(&StreamEvent::content_part_added(0, 0, content_part));

            // 5. Stream the tokens as response.output_text.delta events
            let mut full_text = String::new();
            for token in tokens {
                // Inter-token delay
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                full_text.push_str(&token);
                yield format_sse(&StreamEvent::output_text_delta(0, 0, token));
            }

            // 6. response.output_text.done event
            yield format_sse(&StreamEvent::output_text_done(0, 0, full_text.clone()));

            // 7. response.content_part.done event
            let completed_content = OutputContent::OutputText {
                text: full_text.clone(),
                annotations: None,
            };
            yield format_sse(&StreamEvent::content_part_done(0, 0, completed_content));

            // 8. response.output_item.done event
            let completed_item = OutputItem::Message {
                id: item_id.clone(),
                role: Role::Assistant,
                content: vec![OutputContent::OutputText {
                    text: full_text.clone(),
                    annotations: None,
                }],
                status: Some("completed".to_string()),
            };
            yield format_sse(&StreamEvent::output_item_done(0, completed_item));

            // 9. response.completed event
            let completed_at = chrono::Utc::now().timestamp();
            let completed_response = Response {
                id: id.clone(),
                object: "response".to_string(),
                created_at,
                completed_at: Some(completed_at),
                model: model.clone(),
                status: ResponseStatus::Completed,
                output: vec![OutputItem::Message {
                    id: item_id,
                    role: Role::Assistant,
                    content: vec![OutputContent::OutputText {
                        text: full_text,
                        annotations: None,
                    }],
                    status: Some("completed".to_string()),
                }],
                usage,
                metadata: None,
                error: None,
            };
            yield format_sse(&StreamEvent::response_completed(completed_response));

            // Done marker (same as OpenAI)
            yield "data: [DONE]\n\n".to_string();

            // Invoke completion callback
            if let Some(callback) = on_complete {
                callback();
            }
        })
    }
}

/// Builder for creating OpenResponses token streams
pub struct OpenResponsesStreamBuilder {
    id: Option<String>,
    model: String,
    content: String,
    latency: LatencyProfile,
    usage: Option<Usage>,
    on_complete: Option<OnCompleteCallback>,
}

impl OpenResponsesStreamBuilder {
    pub fn new(model: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: None,
            model: model.into(),
            content: content.into(),
            latency: LatencyProfile::default(),
            usage: None,
            on_complete: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn latency(mut self, latency: LatencyProfile) -> Self {
        self.latency = latency;
        self
    }

    pub fn usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn on_complete<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.on_complete = Some(Box::new(callback));
        self
    }

    pub fn build(self) -> OpenResponsesTokenStream {
        let id = self.id.unwrap_or_else(|| {
            format!("resp_{}", uuid::Uuid::new_v4().to_string().replace("-", ""))
        });

        let mut stream = OpenResponsesTokenStream::new(id, self.model, self.content, self.latency);
        if let Some(usage) = self.usage {
            stream = stream.with_usage(usage);
        }
        if let Some(on_complete) = self.on_complete {
            stream = stream.with_on_complete(on_complete);
        }
        stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_openresponses_stream_basic() {
        let stream = OpenResponsesStreamBuilder::new("gpt-5", "Hello world")
            .latency(LatencyProfile::instant())
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Should have lifecycle events plus content deltas
        assert!(chunks.len() >= 6);
        assert!(chunks.last().unwrap().contains("[DONE]"));

        // Check for key events
        let all_text = chunks.join("");
        assert!(all_text.contains("response.created"));
        assert!(all_text.contains("response.in_progress"));
        assert!(all_text.contains("response.output_text.delta"));
        assert!(all_text.contains("response.completed"));
    }

    #[tokio::test]
    async fn test_openresponses_stream_with_usage() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            input_tokens_details: None,
            output_tokens_details: None,
        };

        let stream = OpenResponsesStreamBuilder::new("gpt-5", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Should include usage in completed event
        let has_usage = chunks.iter().any(|c| c.contains("\"total_tokens\":15"));
        assert!(has_usage, "Stream should include usage in final event");
    }

    #[tokio::test]
    async fn test_openresponses_stream_callback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let callback_called = Arc::new(AtomicBool::new(false));
        let callback_clone = callback_called.clone();

        let stream = OpenResponsesStreamBuilder::new("gpt-5", "Test")
            .latency(LatencyProfile::instant())
            .on_complete(move || {
                callback_clone.store(true, Ordering::SeqCst);
            })
            .build();

        let _chunks: Vec<String> = stream.into_stream().collect().await;
        assert!(callback_called.load(Ordering::SeqCst));
    }
}
