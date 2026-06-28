// Anthropic Messages API Streaming Engine
// Emits the Anthropic streaming event sequence (message_start,
// content_block_start, content_block_delta, content_block_stop, message_delta,
// message_stop) as Server-Sent Events with realistic latency.
//
// The Anthropic SSE wire format differs from OpenAI's: each event carries an
// explicit `event:` line in addition to `data:`, and there is NO terminal
// `[DONE]` sentinel — the stream simply ends after `message_stop`.
// Reference: https://docs.anthropic.com/en/docs/build-with-claude/streaming

use super::types::Usage;
use crate::ids::prefixed_compact_id;
use crate::latency::LatencyProfile;
use async_stream::stream;
use futures_core::Stream;
use serde_json::json;
use std::pin::Pin;
use tokio::time::sleep;

/// Callback type for stream completion.
type OnCompleteCallback = Box<dyn FnOnce() + Send + 'static>;

/// A streaming Anthropic Messages response.
pub struct MessagesTokenStream {
    id: String,
    model: String,
    latency: LatencyProfile,
    content: String,
    input_tokens: u32,
    output_tokens: u32,
    on_complete: Option<OnCompleteCallback>,
}

impl MessagesTokenStream {
    pub fn new(id: String, model: String, content: String, latency: LatencyProfile) -> Self {
        Self {
            id,
            model,
            latency,
            content,
            input_tokens: 0,
            output_tokens: 0,
            on_complete: None,
        }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.input_tokens = usage.input_tokens;
        self.output_tokens = usage.output_tokens;
        self
    }

    pub fn with_on_complete<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.on_complete = Some(Box::new(callback));
        self
    }

    /// Word-level tokenization (keeps whitespace as separate tokens) to
    /// approximate token-by-token streaming.
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

    /// Render the Anthropic streaming event sequence as SSE.
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        let tokens = self.tokenize();
        let id = self.id.clone();
        let model = self.model.clone();
        let latency = self.latency.clone();
        let input_tokens = self.input_tokens;
        let output_tokens = self.output_tokens;
        let on_complete = self.on_complete;

        Box::pin(stream! {
            // Time to first token.
            let ttft = latency.sample_ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }

            // 1. message_start (usage seeded with input tokens, output_tokens=1).
            let message_start = json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": input_tokens, "output_tokens": 1}
                }
            });
            yield format_event("message_start", &message_start);

            // 2. content_block_start (text block at index 0).
            let block_start = json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text", "text": ""}
            });
            yield format_event("content_block_start", &block_start);

            // 3. ping (Anthropic interleaves these to keep the connection warm).
            yield format_event("ping", &json!({"type": "ping"}));

            // 4. content_block_delta for each token.
            for token in tokens {
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }
                let delta = json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": token}
                });
                yield format_event("content_block_delta", &delta);
            }

            // 5. content_block_stop.
            yield format_event(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": 0}),
            );

            // 6. message_delta with final stop_reason + cumulative output usage.
            let message_delta = json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": output_tokens}
            });
            yield format_event("message_delta", &message_delta);

            // 7. message_stop (terminal — no [DONE] sentinel).
            yield format_event("message_stop", &json!({"type": "message_stop"}));

            if let Some(callback) = on_complete {
                callback();
            }
        })
    }
}

/// Format an Anthropic SSE event with both `event:` and `data:` lines.
pub fn format_event(event_type: &str, payload: &serde_json::Value) -> String {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    format!("event: {}\ndata: {}\n\n", event_type, data)
}

/// Builder for [`MessagesTokenStream`].
pub struct MessagesStreamBuilder {
    id: Option<String>,
    model: String,
    content: String,
    latency: LatencyProfile,
    usage: Option<Usage>,
    on_complete: Option<OnCompleteCallback>,
}

impl MessagesStreamBuilder {
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

    pub fn build(self) -> MessagesTokenStream {
        let id = self.id.unwrap_or_else(|| prefixed_compact_id("msg_"));
        let mut stream = MessagesTokenStream::new(id, self.model, self.content, self.latency);
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
    use futures_util::StreamExt;

    #[tokio::test]
    async fn test_stream_event_sequence() {
        let stream = MessagesStreamBuilder::new("claude-opus-4-8", "Hello world")
            .latency(LatencyProfile::instant())
            .usage(Usage::new(7, 2))
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;
        let all = chunks.join("");

        // Required lifecycle events, in order.
        assert!(all.contains("event: message_start"));
        assert!(all.contains("event: content_block_start"));
        assert!(all.contains("event: content_block_delta"));
        assert!(all.contains("event: content_block_stop"));
        assert!(all.contains("event: message_delta"));
        assert!(all.contains("event: message_stop"));

        // Anthropic streams do NOT emit a [DONE] sentinel.
        assert!(!all.contains("[DONE]"));

        // Input usage seeded in message_start; output usage in message_delta.
        assert!(all.contains("\"input_tokens\":7"));
        assert!(all.contains("\"output_tokens\":2"));
    }

    #[tokio::test]
    async fn test_stream_ordering() {
        let stream = MessagesStreamBuilder::new("claude-haiku-4-5", "Hi there")
            .latency(LatencyProfile::instant())
            .build();
        let chunks: Vec<String> = stream.into_stream().collect().await;

        // First event is message_start, last is message_stop.
        assert!(chunks.first().unwrap().contains("message_start"));
        assert!(chunks.last().unwrap().contains("message_stop"));
    }

    #[tokio::test]
    async fn test_text_deltas_reconstruct_content() {
        let stream = MessagesStreamBuilder::new("claude-opus-4-8", "abc def")
            .latency(LatencyProfile::instant())
            .build();
        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Collect all text_delta payloads and reassemble.
        let mut reassembled = String::new();
        for c in &chunks {
            if c.contains("text_delta") {
                if let Some(data_line) = c.lines().find(|l| l.starts_with("data: ")) {
                    let json: serde_json::Value =
                        serde_json::from_str(&data_line["data: ".len()..]).unwrap();
                    reassembled.push_str(json["delta"]["text"].as_str().unwrap());
                }
            }
        }
        assert_eq!(reassembled, "abc def");
    }

    #[tokio::test]
    async fn test_on_complete_callback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        let stream = MessagesStreamBuilder::new("claude-opus-4-8", "x")
            .latency(LatencyProfile::instant())
            .on_complete(move || called_clone.store(true, Ordering::SeqCst))
            .build();
        let _: Vec<String> = stream.into_stream().collect().await;
        assert!(called.load(Ordering::SeqCst));
    }
}
