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
    /// Whether to include a reasoning output item
    include_reasoning: bool,
    /// Optional reasoning summary text to stream
    reasoning_summary: Option<String>,
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
            include_reasoning: false,
            reasoning_summary: None,
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

    /// Convert text into chunks for streaming (word-level)
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
        let content_tokens = Self::tokenize_text(&self.content);
        let response_id = self.response_id.clone();
        let message_id = self.message_id.clone();
        let model = self.model.clone();
        let created_at = self.created_at;
        let latency = self.latency.clone();
        let usage = self.usage.clone();
        let content = self.content.clone();
        let include_reasoning = self.include_reasoning;
        let reasoning_summary = self.reasoning_summary.clone();
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

            // Track output_index: reasoning item takes index 0 when present,
            // message takes the next index
            let mut sequence_number: u32 = 0;
            let mut final_output_items: Vec<OutputItem> = Vec::new();

            // --- Reasoning output item (if enabled) ---
            if include_reasoning {
                let reasoning_id = format!("rs_{}", uuid::Uuid::new_v4());
                let reasoning_output_index: u32 = 0;

                // Emit reasoning item added (in_progress, no summary yet)
                let reasoning_item = OutputItem::Reasoning {
                    id: reasoning_id.clone(),
                    status: ItemStatus::InProgress,
                    summary: None,
                };
                yield ResponsesStreamEvent::output_item_added(reasoning_output_index, &reasoning_item);

                // If we have summary text, stream it
                if let Some(ref summary_text) = reasoning_summary {
                    let empty_summary = ReasoningSummary {
                        summary_type: "summary_text".to_string(),
                        text: String::new(),
                    };

                    // reasoning_summary_part.added
                    yield ResponsesStreamEvent::reasoning_summary_part_added(
                        reasoning_output_index, 0, &empty_summary,
                    );

                    // Stream summary text deltas
                    let summary_tokens = Self::tokenize_text(summary_text);
                    for token in summary_tokens.into_iter() {
                        let tbt = latency.sample_tbt();
                        if !tbt.is_zero() {
                            sleep(tbt).await;
                        }

                        yield ResponsesStreamEvent::reasoning_summary_text_delta(
                            reasoning_output_index, 0, &token, sequence_number,
                        );
                        sequence_number += 1;
                    }

                    // reasoning_summary_text.done
                    yield ResponsesStreamEvent::reasoning_summary_text_done(
                        reasoning_output_index, 0, summary_text,
                    );

                    // reasoning_summary_part.done
                    let final_summary = ReasoningSummary {
                        summary_type: "summary_text".to_string(),
                        text: summary_text.clone(),
                    };
                    yield ResponsesStreamEvent::reasoning_summary_part_done(
                        reasoning_output_index, 0, &final_summary,
                    );
                }

                // Emit reasoning item done
                let final_reasoning_item = OutputItem::Reasoning {
                    id: reasoning_id,
                    status: ItemStatus::Completed,
                    summary: reasoning_summary.as_ref().map(|text| {
                        vec![ReasoningSummary {
                            summary_type: "summary_text".to_string(),
                            text: text.clone(),
                        }]
                    }),
                };
                yield ResponsesStreamEvent::output_item_done(reasoning_output_index, &final_reasoning_item);
                final_output_items.push(final_reasoning_item);
            }

            // --- Message output item ---
            let message_output_index = if include_reasoning { 1 } else { 0 };

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
            for token in content_tokens.into_iter() {
                // Inter-token delay
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                // response.output_text.delta event
                yield ResponsesStreamEvent::output_text_delta(
                    message_output_index, 0, &token, sequence_number,
                );
                sequence_number += 1;
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
            final_output_items.push(final_message_item);

            // response.completed event with full response
            let final_response = ResponsesResponse {
                id: response_id.clone(),
                object: "response".to_string(),
                created_at,
                model: model.clone(),
                status: ResponseStatus::Completed,
                output: final_output_items,
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
    include_reasoning: bool,
    reasoning_summary: Option<String>,
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
            include_reasoning: false,
            reasoning_summary: None,
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

    /// Enable reasoning output item with optional summary text.
    /// When `summary_text` is `Some`, the reasoning item will include a streamed summary.
    /// When `summary_text` is `None`, the reasoning item appears without summary content.
    pub fn reasoning(mut self, summary_text: Option<String>) -> Self {
        self.include_reasoning = true;
        self.reasoning_summary = summary_text;
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
        stream.include_reasoning = self.include_reasoning;
        stream.reasoning_summary = self.reasoning_summary;
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

    #[tokio::test]
    async fn test_responses_stream_with_reasoning_and_summary() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 17,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 9,
            }),
        };

        let stream = ResponsesTokenStreamBuilder::new("o3", "Answer here")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .reasoning(Some("Thinking about it.".to_string()))
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Should contain reasoning events
        let reasoning_added: Vec<_> = events
            .iter()
            .filter(|e| e.contains("output_item.added") && e.contains("\"reasoning\""))
            .collect();
        assert_eq!(
            reasoning_added.len(),
            1,
            "Should have one reasoning item added"
        );

        let summary_part_added: Vec<_> = events
            .iter()
            .filter(|e| e.contains("reasoning_summary_part.added"))
            .collect();
        assert_eq!(summary_part_added.len(), 1);

        let summary_deltas: Vec<_> = events
            .iter()
            .filter(|e| e.contains("reasoning_summary_text.delta"))
            .collect();
        assert!(
            !summary_deltas.is_empty(),
            "Should have summary text deltas"
        );

        let summary_done: Vec<_> = events
            .iter()
            .filter(|e| e.contains("reasoning_summary_text.done"))
            .collect();
        assert_eq!(summary_done.len(), 1);

        let summary_part_done: Vec<_> = events
            .iter()
            .filter(|e| e.contains("reasoning_summary_part.done"))
            .collect();
        assert_eq!(summary_part_done.len(), 1);

        // Reasoning item done should come before message item added
        let reasoning_done_idx = events
            .iter()
            .position(|e| e.contains("output_item.done") && e.contains("\"reasoning\""))
            .expect("Should have reasoning item done");
        let message_added_idx = events
            .iter()
            .position(|e| e.contains("output_item.added") && e.contains("\"message\""))
            .expect("Should have message item added");
        assert!(
            reasoning_done_idx < message_added_idx,
            "Reasoning item should complete before message starts"
        );

        // Message output_index should be 1
        let message_event = &events[message_added_idx];
        assert!(message_event.contains("\"output_index\":1"));

        // response.completed should have both items in output
        let completed = events.last().unwrap();
        assert!(completed.contains("response.completed"));
        assert!(completed.contains("\"reasoning\""));
        assert!(completed.contains("\"message\""));
    }

    #[tokio::test]
    async fn test_responses_stream_with_reasoning_no_summary() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 17,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 9,
            }),
        };

        let stream = ResponsesTokenStreamBuilder::new("o3", "Answer")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .reasoning(None)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Should have reasoning output item but no summary events
        let reasoning_added: Vec<_> = events
            .iter()
            .filter(|e| e.contains("output_item.added") && e.contains("\"reasoning\""))
            .collect();
        assert_eq!(reasoning_added.len(), 1);

        let summary_events: Vec<_> = events
            .iter()
            .filter(|e| e.contains("reasoning_summary"))
            .collect();
        assert!(
            summary_events.is_empty(),
            "No summary events when summary not requested"
        );

        // Message should still be at output_index 1
        let message_added = events
            .iter()
            .find(|e| e.contains("output_item.added") && e.contains("\"message\""))
            .expect("Should have message item");
        assert!(message_added.contains("\"output_index\":1"));
    }

    #[tokio::test]
    async fn test_responses_stream_no_reasoning() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            output_tokens_details: None,
        };

        let stream = ResponsesTokenStreamBuilder::new("gpt-4o", "Hello")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // No reasoning events
        let reasoning_events: Vec<_> = events
            .iter()
            .filter(|e| e.contains("\"reasoning\""))
            .collect();
        assert!(
            reasoning_events.is_empty(),
            "Non-reasoning model should not have reasoning items"
        );

        // Message at output_index 0
        let message_added = events
            .iter()
            .find(|e| e.contains("output_item.added") && e.contains("\"message\""))
            .expect("Should have message item");
        assert!(message_added.contains("\"output_index\":0"));
    }

    #[tokio::test]
    async fn test_responses_stream_reasoning_sequence_numbers_continuous() {
        let usage = ResponsesUsage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 20,
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: 12,
            }),
        };

        let stream = ResponsesTokenStreamBuilder::new("o3", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .reasoning(Some("Thinking carefully.".to_string()))
            .build();

        let events: Vec<String> = stream.into_stream().collect().await;

        // Collect all sequence numbers from both summary deltas and text deltas
        let mut seq_numbers: Vec<u32> = Vec::new();
        for event in &events {
            if event.contains("reasoning_summary_text.delta") || event.contains("output_text.delta")
            {
                // Extract sequence_number from JSON
                if let Some(data_start) = event.find("data: ") {
                    let data = &event[data_start + 6..];
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data.trim()) {
                        if let Some(seq) = json.get("sequence_number").and_then(|v| v.as_u64()) {
                            seq_numbers.push(seq as u32);
                        }
                    }
                }
            }
        }

        // Sequence numbers should be continuous starting from 0
        assert!(!seq_numbers.is_empty());
        for (i, seq) in seq_numbers.iter().enumerate() {
            assert_eq!(
                *seq, i as u32,
                "Sequence numbers should be continuous: expected {}, got {}",
                i, seq
            );
        }
    }
}
