// Responses API Streaming Engine
// Implements streaming for the OpenAI Responses API format.

use crate::latency::LatencyProfile;
use crate::openai::{
    ItemStatus, OutputContentPart, OutputItem, OutputRole, OutputTokensDetails, ResponseStatus,
    ResponsesResponse, ResponsesStreamEvent, ResponsesUsage,
};
use async_stream::stream;
use futures::Stream;
use std::pin::Pin;
use tokio::time::sleep;

/// Type alias for on-complete callback
type OnCompleteCallback = Box<dyn FnOnce() + Send>;

/// A streaming response for the Responses API
pub struct ResponsesTokenStream {
    /// The response ID
    response_id: String,
    /// The message ID
    message_id: String,
    /// The model name
    model: String,
    /// Unix timestamp of creation
    created_at: i64,
    /// Latency profile for timing simulation
    latency: LatencyProfile,
    /// The full response text to stream
    content: String,
    /// Token usage
    usage: ResponsesUsage,
    /// Callback to invoke when stream completes
    on_complete: Option<OnCompleteCallback>,
}

impl ResponsesTokenStream {
    pub fn new(
        model: String,
        content: String,
        latency: LatencyProfile,
        usage: ResponsesUsage,
    ) -> Self {
        Self {
            response_id: format!("resp_{}", uuid::Uuid::new_v4()),
            message_id: format!("msg_{}", uuid::Uuid::new_v4()),
            model,
            created_at: chrono::Utc::now().timestamp(),
            latency,
            content,
            usage,
            on_complete: None,
        }
    }

    pub fn with_on_complete<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.on_complete = Some(Box::new(callback));
        self
    }

    /// Convert the content into chunks for streaming (word-level)
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

    /// Create a streaming response as Server-Sent Events
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        let tokens = self.tokenize();
        let response_id = self.response_id.clone();
        let message_id = self.message_id.clone();
        let model = self.model.clone();
        let created_at = self.created_at;
        let latency = self.latency.clone();
        let usage = self.usage.clone();
        let content = self.content.clone();
        let on_complete = self.on_complete;

        Box::pin(stream! {
            // Create initial response with in_progress status
            let initial_response = ResponsesResponse {
                id: response_id.clone(),
                object: "response".to_string(),
                created_at,
                model: model.clone(),
                status: ResponseStatus::InProgress,
                output: vec![],
                output_text: None,
                usage: None,
                error: None,
                metadata: None,
            };

            // response.created event
            yield ResponsesStreamEvent::response_created(initial_response.clone());

            // Initial delay (time to first token)
            let ttft = latency.sample_ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }

            // response.in_progress event
            yield ResponsesStreamEvent::response_in_progress(initial_response.clone());

            // Create the output item (message) with in_progress status
            let message_item = OutputItem::Message {
                id: message_id.clone(),
                role: OutputRole::Assistant,
                status: ItemStatus::InProgress,
                content: vec![],
            };

            // response.output_item.added event
            yield ResponsesStreamEvent::output_item_added(0, &message_item);

            // Create the content part
            let content_part = OutputContentPart::OutputText {
                text: String::new(),
            };

            // response.content_part.added event
            yield ResponsesStreamEvent::content_part_added(0, 0, &content_part);

            // Stream content chunks with delta events
            for (sequence_number, token) in tokens.into_iter().enumerate() {
                // Inter-token delay
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                // response.output_text.delta event
                yield ResponsesStreamEvent::output_text_delta(0, 0, &token, sequence_number as u32);
            }

            // response.output_text.done event
            yield ResponsesStreamEvent::output_text_done(0, 0, &content);

            // response.content_part.done event
            let final_content_part = OutputContentPart::OutputText {
                text: content.clone(),
            };
            yield ResponsesStreamEvent::content_part_done(0, 0, &final_content_part);

            // response.output_item.done event
            let final_message_item = OutputItem::Message {
                id: message_id.clone(),
                role: OutputRole::Assistant,
                status: ItemStatus::Completed,
                content: vec![final_content_part],
            };
            yield ResponsesStreamEvent::output_item_done(0, &final_message_item);

            // response.completed event with full response
            let final_response = ResponsesResponse {
                id: response_id.clone(),
                object: "response".to_string(),
                created_at,
                model: model.clone(),
                status: ResponseStatus::Completed,
                output: vec![final_message_item],
                output_text: Some(content.clone()),
                usage: Some(usage),
                error: None,
                metadata: None,
            };
            yield ResponsesStreamEvent::response_completed(final_response);

            // Invoke completion callback
            if let Some(callback) = on_complete {
                callback();
            }
        })
    }
}

/// Builder for creating Responses API token streams
pub struct ResponsesTokenStreamBuilder {
    model: String,
    content: String,
    latency: LatencyProfile,
    usage: ResponsesUsage,
    on_complete: Option<OnCompleteCallback>,
}

impl ResponsesTokenStreamBuilder {
    pub fn new(model: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            content: content.into(),
            latency: LatencyProfile::default(),
            usage: ResponsesUsage {
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                output_tokens_details: Some(OutputTokensDetails {
                    reasoning_tokens: 0,
                }),
            },
            on_complete: None,
        }
    }

    pub fn latency(mut self, latency: LatencyProfile) -> Self {
        self.latency = latency;
        self
    }

    pub fn usage(mut self, usage: ResponsesUsage) -> Self {
        self.usage = usage;
        self
    }

    /// Set a callback to be invoked when the stream completes
    pub fn on_complete<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.on_complete = Some(Box::new(callback));
        self
    }

    pub fn build(self) -> ResponsesTokenStream {
        let mut stream =
            ResponsesTokenStream::new(self.model, self.content, self.latency, self.usage);
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
    async fn test_responses_stream_basic() {
        let usage = ResponsesUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            output_tokens_details: None,
        };

        let stream = ResponsesTokenStreamBuilder::new("gpt-5", "Hello world")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Should have multiple events
        assert!(!events.is_empty());

        // First event should be response.created
        assert!(events[0].contains("response.created"));

        // Last event should be response.completed
        assert!(events.last().unwrap().contains("response.completed"));
    }

    #[tokio::test]
    async fn test_responses_stream_deltas() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            output_tokens_details: None,
        };

        let stream = ResponsesTokenStreamBuilder::new("gpt-5", "Hello world")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Should contain delta events
        let delta_events: Vec<_> = events
            .iter()
            .filter(|e| e.contains("output_text.delta"))
            .collect();
        assert!(!delta_events.is_empty());

        // Delta events should have sequence numbers
        assert!(delta_events[0].contains("sequence_number"));
    }

    #[tokio::test]
    async fn test_responses_stream_event_order() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 2,
            total_tokens: 7,
            output_tokens_details: None,
        };

        let stream = ResponsesTokenStreamBuilder::new("gpt-5", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Verify event order
        let event_types: Vec<&str> = events
            .iter()
            .filter_map(|e| {
                if e.contains("response.created") {
                    Some("created")
                } else if e.contains("response.in_progress") {
                    Some("in_progress")
                } else if e.contains("output_item.added") {
                    Some("item_added")
                } else if e.contains("content_part.added") {
                    Some("part_added")
                } else if e.contains("output_text.delta") {
                    Some("delta")
                } else if e.contains("output_text.done") {
                    Some("text_done")
                } else if e.contains("content_part.done") {
                    Some("part_done")
                } else if e.contains("output_item.done") {
                    Some("item_done")
                } else if e.contains("response.completed") {
                    Some("completed")
                } else {
                    None
                }
            })
            .collect();

        // Should start with created and end with completed
        assert_eq!(event_types.first(), Some(&"created"));
        assert_eq!(event_types.last(), Some(&"completed"));
    }
}
