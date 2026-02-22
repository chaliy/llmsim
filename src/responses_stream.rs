// Responses API Streaming Engine
// Implements streaming for the OpenAI Responses API format.

use crate::latency::LatencyProfile;
use crate::openai::{
    ItemStatus, OutputContentPart, OutputItem, OutputRole, OutputTokensDetails, ReasoningSummary,
    ResponseStatus, ResponsesResponse, ResponsesStreamEvent, ResponsesUsage,
};
use async_stream::stream;
use futures::Stream;
use std::pin::Pin;
use tokio::time::sleep;

/// Configuration for reasoning output in the stream
#[derive(Debug, Clone)]
pub struct ReasoningStreamConfig {
    /// Summary text to stream (None means no summary)
    pub summary_text: Option<String>,
}

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
    /// Optional reasoning configuration for thinking emulation
    reasoning: Option<ReasoningStreamConfig>,
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
            reasoning: None,
        }
    }

    pub fn with_reasoning(mut self, config: ReasoningStreamConfig) -> Self {
        self.reasoning = Some(config);
        self
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

    /// Tokenize reasoning summary text into chunks for streaming
    fn tokenize_text(text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current_word = String::new();

        for ch in text.chars() {
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
        let reasoning = self.reasoning.clone();

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

            // Track the output index offset (reasoning takes index 0 when present)
            let mut sequence_counter: u32 = 0;
            let message_output_index: u32;
            let mut all_output_items: Vec<OutputItem> = Vec::new();

            // Stream reasoning output item if reasoning is configured
            if let Some(ref reasoning_config) = reasoning {
                let reasoning_id = format!("rs_{}", uuid::Uuid::new_v4());
                message_output_index = 1;

                // Create reasoning item with in_progress status
                let reasoning_item = OutputItem::Reasoning {
                    id: reasoning_id.clone(),
                    status: ItemStatus::InProgress,
                    summary: None,
                };

                // response.output_item.added for reasoning
                yield ResponsesStreamEvent::output_item_added(0, &reasoning_item);

                // Stream reasoning summary if provided
                let final_summary = if let Some(ref summary_text) = reasoning_config.summary_text {
                    let empty_part = ReasoningSummary {
                        summary_type: "summary_text".to_string(),
                        text: String::new(),
                    };

                    // response.reasoning_summary_part.added
                    yield ResponsesStreamEvent::reasoning_summary_part_added(0, 0, &empty_part);

                    // Stream summary text as deltas
                    let summary_tokens = Self::tokenize_text(summary_text);
                    for token in &summary_tokens {
                        let tbt = latency.sample_tbt();
                        if !tbt.is_zero() {
                            sleep(tbt).await;
                        }
                        yield ResponsesStreamEvent::reasoning_summary_text_delta(
                            0, 0, token, sequence_counter,
                        );
                        sequence_counter += 1;
                    }

                    // response.reasoning_summary_text.done
                    yield ResponsesStreamEvent::reasoning_summary_text_done(0, 0, summary_text);

                    let final_part = ReasoningSummary {
                        summary_type: "summary_text".to_string(),
                        text: summary_text.clone(),
                    };

                    // response.reasoning_summary_part.done
                    yield ResponsesStreamEvent::reasoning_summary_part_done(0, 0, &final_part);

                    Some(vec![final_part])
                } else {
                    None
                };

                // response.output_item.done for reasoning
                let completed_reasoning_item = OutputItem::Reasoning {
                    id: reasoning_id,
                    status: ItemStatus::Completed,
                    summary: final_summary,
                };
                yield ResponsesStreamEvent::output_item_done(0, &completed_reasoning_item);
                all_output_items.push(completed_reasoning_item);
            } else {
                message_output_index = 0;
            }

            // Create the output item (message) with in_progress status
            let message_item = OutputItem::Message {
                id: message_id.clone(),
                role: OutputRole::Assistant,
                status: ItemStatus::InProgress,
                content: vec![],
            };

            // response.output_item.added event
            yield ResponsesStreamEvent::output_item_added(message_output_index, &message_item);

            // Create the content part
            let content_part = OutputContentPart::OutputText {
                text: String::new(),
            };

            // response.content_part.added event
            yield ResponsesStreamEvent::content_part_added(message_output_index, 0, &content_part);

            // Stream content chunks with delta events
            for token in tokens {
                // Inter-token delay
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                // response.output_text.delta event
                yield ResponsesStreamEvent::output_text_delta(
                    message_output_index, 0, &token, sequence_counter,
                );
                sequence_counter += 1;
            }

            // response.output_text.done event
            yield ResponsesStreamEvent::output_text_done(message_output_index, 0, &content);

            // response.content_part.done event
            let final_content_part = OutputContentPart::OutputText {
                text: content.clone(),
            };
            yield ResponsesStreamEvent::content_part_done(message_output_index, 0, &final_content_part);

            // response.output_item.done event
            let final_message_item = OutputItem::Message {
                id: message_id.clone(),
                role: OutputRole::Assistant,
                status: ItemStatus::Completed,
                content: vec![final_content_part],
            };
            yield ResponsesStreamEvent::output_item_done(message_output_index, &final_message_item);
            all_output_items.push(final_message_item);

            // response.completed event with full response
            let final_response = ResponsesResponse {
                id: response_id.clone(),
                object: "response".to_string(),
                created_at,
                model: model.clone(),
                status: ResponseStatus::Completed,
                output: all_output_items,
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
    reasoning: Option<ReasoningStreamConfig>,
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
            reasoning: None,
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

    /// Set reasoning configuration for thinking emulation
    pub fn reasoning(mut self, config: ReasoningStreamConfig) -> Self {
        self.reasoning = Some(config);
        self
    }

    pub fn build(self) -> ResponsesTokenStream {
        let mut stream =
            ResponsesTokenStream::new(self.model, self.content, self.latency, self.usage);
        if let Some(on_complete) = self.on_complete {
            stream = stream.with_on_complete(on_complete);
        }
        if let Some(reasoning) = self.reasoning {
            stream = stream.with_reasoning(reasoning);
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

    #[tokio::test]
    async fn test_responses_stream_with_reasoning() {
        let usage = ResponsesUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 30,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 15,
            }),
        };

        let reasoning_config = ReasoningStreamConfig {
            summary_text: Some("Analyzed the request carefully.".to_string()),
        };

        let stream = ResponsesTokenStreamBuilder::new("o3", "Hello world")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .reasoning(reasoning_config)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Should have reasoning events
        let has_reasoning_part_added = events
            .iter()
            .any(|e| e.contains("reasoning_summary_part.added"));
        let has_reasoning_delta = events
            .iter()
            .any(|e| e.contains("reasoning_summary_text.delta"));
        let has_reasoning_text_done = events
            .iter()
            .any(|e| e.contains("reasoning_summary_text.done"));
        let has_reasoning_part_done = events
            .iter()
            .any(|e| e.contains("reasoning_summary_part.done"));

        assert!(
            has_reasoning_part_added,
            "Missing reasoning_summary_part.added"
        );
        assert!(has_reasoning_delta, "Missing reasoning_summary_text.delta");
        assert!(
            has_reasoning_text_done,
            "Missing reasoning_summary_text.done"
        );
        assert!(
            has_reasoning_part_done,
            "Missing reasoning_summary_part.done"
        );

        // Should still have message events
        let has_text_delta = events.iter().any(|e| e.contains("output_text.delta"));
        assert!(has_text_delta, "Missing output_text.delta for message");

        // Reasoning item should appear at output_index 0, message at output_index 1
        let item_added_events: Vec<_> = events
            .iter()
            .filter(|e| e.contains("output_item.added"))
            .collect();
        assert_eq!(
            item_added_events.len(),
            2,
            "Expected 2 output_item.added events"
        );
        // First should be reasoning (output_index 0)
        assert!(item_added_events[0].contains("\"output_index\":0"));
        assert!(item_added_events[0].contains("\"type\":\"reasoning\""));
        // Second should be message (output_index 1)
        assert!(item_added_events[1].contains("\"output_index\":1"));
        assert!(item_added_events[1].contains("\"type\":\"message\""));

        // Completed response should contain both items
        let completed_event = events.last().unwrap();
        assert!(completed_event.contains("response.completed"));
        assert!(completed_event.contains("\"type\":\"reasoning\""));
        assert!(completed_event.contains("\"type\":\"message\""));
    }

    #[tokio::test]
    async fn test_responses_stream_reasoning_event_order() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 2,
            total_tokens: 12,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 5,
            }),
        };

        let reasoning_config = ReasoningStreamConfig {
            summary_text: Some("Thinking.".to_string()),
        };

        let stream = ResponsesTokenStreamBuilder::new("o3", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .reasoning(reasoning_config)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Classify events in order
        let event_types: Vec<&str> = events
            .iter()
            .filter_map(|e| {
                if e.contains("response.created") {
                    Some("created")
                } else if e.contains("response.in_progress") {
                    Some("in_progress")
                } else if e.contains("reasoning_summary_part.added") {
                    Some("reasoning_part_added")
                } else if e.contains("reasoning_summary_text.delta") {
                    Some("reasoning_delta")
                } else if e.contains("reasoning_summary_text.done") {
                    Some("reasoning_text_done")
                } else if e.contains("reasoning_summary_part.done") {
                    Some("reasoning_part_done")
                } else if e.contains("output_item.added") {
                    Some("item_added")
                } else if e.contains("content_part.added") {
                    Some("part_added")
                } else if e.contains("output_text.delta") {
                    Some("text_delta")
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

        // Reasoning events should come before message text deltas
        let first_reasoning_idx = event_types
            .iter()
            .position(|t| *t == "reasoning_part_added")
            .unwrap();
        let first_text_delta_idx = event_types.iter().position(|t| *t == "text_delta").unwrap();
        assert!(
            first_reasoning_idx < first_text_delta_idx,
            "Reasoning events should precede message text deltas"
        );

        // Should start with created and end with completed
        assert_eq!(event_types.first(), Some(&"created"));
        assert_eq!(event_types.last(), Some(&"completed"));
    }

    #[tokio::test]
    async fn test_responses_stream_reasoning_without_summary() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 2,
            total_tokens: 12,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 5,
            }),
        };

        // Reasoning enabled but no summary text
        let reasoning_config = ReasoningStreamConfig { summary_text: None };

        let stream = ResponsesTokenStreamBuilder::new("o3", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .reasoning(reasoning_config)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Should have reasoning output_item.added but no summary events
        let has_reasoning_item = events
            .iter()
            .any(|e| e.contains("output_item.added") && e.contains("\"type\":\"reasoning\""));
        assert!(has_reasoning_item, "Should have reasoning output item");

        let has_reasoning_summary = events
            .iter()
            .any(|e| e.contains("reasoning_summary_part.added"));
        assert!(!has_reasoning_summary, "Should not have summary events");

        // Should still have message events
        let has_text_delta = events.iter().any(|e| e.contains("output_text.delta"));
        assert!(has_text_delta, "Should still have message text deltas");
    }

    #[tokio::test]
    async fn test_responses_stream_no_reasoning() {
        // No reasoning config at all (non-reasoning model)
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 2,
            total_tokens: 7,
            output_tokens_details: None,
        };

        let stream = ResponsesTokenStreamBuilder::new("gpt-4o", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // No reasoning summary events
        let has_reasoning_events = events.iter().any(|e| e.contains("reasoning_summary"));
        assert!(
            !has_reasoning_events,
            "Non-reasoning model should not have reasoning events"
        );

        // Message should be at output_index 0
        let item_added = events
            .iter()
            .find(|e| e.contains("output_item.added"))
            .unwrap();
        assert!(item_added.contains("\"output_index\":0"));
        assert!(item_added.contains("\"type\":\"message\""));
    }
}
