// Anthropic Messages API Streaming Engine
// Emits the Anthropic streaming event sequence (message_start,
// content_block_start, content_block_delta, content_block_stop, message_delta,
// message_stop) as Server-Sent Events with realistic latency.
//
// When a thinking block is planned, it streams first at index 0
// (`thinking_delta` events, then one `signature_delta`) and the text block
// follows at index 1, matching the real extended-thinking stream.
//
// Tool calls follow the text as `tool_use` blocks whose input streams as
// `input_json_delta` chunks of the JSON text (the SDKs concatenate
// `partial_json`); the message then stops with `stop_reason: "tool_use"`.
// A turn with tool calls and no text has no text block, as on the real API.
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
    cache_usage: Option<(u32, u32)>,
    output_tokens: u32,
    /// Optional thinking block: (visible text, signature).
    thinking: Option<(String, String)>,
    /// Tool calls after the text: (id, name, input).
    tool_uses: Vec<(String, String, serde_json::Value)>,
    /// Stream the thinking block as `redacted_thinking` with this data.
    redacted_thinking: Option<String>,
    /// Total time the thinking block takes, however much of it is visible.
    thinking_time: std::time::Duration,
    on_complete: Option<OnCompleteCallback>,
}

/// Split JSON text into a few `partial_json` chunks (about 16 bytes each, on
/// char boundaries).
fn json_chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if current.len() >= 16 {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

impl MessagesTokenStream {
    pub fn new(id: String, model: String, content: String, latency: LatencyProfile) -> Self {
        Self {
            id,
            model,
            latency,
            content,
            input_tokens: 0,
            cache_usage: None,
            output_tokens: 0,
            thinking: None,
            tool_uses: Vec::new(),
            redacted_thinking: None,
            thinking_time: std::time::Duration::ZERO,
            on_complete: None,
        }
    }

    /// Stream the thinking block as `redacted_thinking` carrying `data`.
    pub fn with_redacted_thinking(mut self, data: impl Into<String>) -> Self {
        self.redacted_thinking = Some(data.into());
        self
    }

    /// How long the thinking block takes in total. Hidden (omitted or
    /// summarized) thinking still takes the time of every thinking token.
    pub fn with_thinking_time(mut self, time: std::time::Duration) -> Self {
        self.thinking_time = time;
        self
    }

    /// Stream `tool_use` blocks after the text: (id, name, input).
    pub fn with_tool_uses(mut self, tool_uses: Vec<(String, String, serde_json::Value)>) -> Self {
        self.tool_uses = tool_uses;
        self
    }

    /// Stream a thinking block (visible text may be empty) before the text.
    pub fn with_thinking(mut self, text: impl Into<String>, signature: impl Into<String>) -> Self {
        self.thinking = Some((text.into(), signature.into()));
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.input_tokens = usage.input_tokens;
        self.output_tokens = usage.output_tokens;
        self.cache_usage = usage
            .cache_read_input_tokens
            .zip(usage.cache_creation_input_tokens);
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
        tokenize_words(&self.content)
    }

    /// Render the Anthropic streaming event sequence as SSE.
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        let tokens = self.tokenize();
        let id = self.id.clone();
        let model = self.model.clone();
        let latency = self.latency.clone();
        let input_tokens = self.input_tokens;
        let mut start_usage = json!({"input_tokens": input_tokens, "output_tokens": 1});
        if let Some((read, creation)) = self.cache_usage {
            start_usage["cache_read_input_tokens"] = json!(read);
            start_usage["cache_creation_input_tokens"] = json!(creation);
        }
        let output_tokens = self.output_tokens;
        let thinking = self.thinking;
        let redacted = self.redacted_thinking;
        let thinking_time = self.thinking_time;
        let tool_uses = self.tool_uses;
        let skip_text = self.content.is_empty() && !tool_uses.is_empty();
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
                    "usage": start_usage
                }
            });
            yield format_event("message_start", &message_start);

            // Optional thinking block at index 0.
            let mut text_index = 0;
            if let Some(data) = redacted {
                let block_start = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "redacted_thinking", "data": data}
                });
                yield format_event("content_block_start", &block_start);
                yield format_event("ping", &json!({"type": "ping"}));
                if !thinking_time.is_zero() {
                    sleep(thinking_time).await;
                }
                yield format_event(
                    "content_block_stop",
                    &json!({"type": "content_block_stop", "index": 0}),
                );
                text_index = 1;
            } else if let Some((thinking_text, signature)) = thinking {
                let block_start = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "thinking", "thinking": "", "signature": ""}
                });
                yield format_event("content_block_start", &block_start);
                yield format_event("ping", &json!({"type": "ping"}));

                let mut spent = std::time::Duration::ZERO;
                for token in tokenize_words(&thinking_text) {
                    let tbt = latency.sample_tbt();
                    if !tbt.is_zero() {
                        sleep(tbt).await;
                        spent += tbt;
                    }
                    let delta = json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "thinking_delta", "thinking": token}
                    });
                    yield format_event("content_block_delta", &delta);
                }
                // The rest of the thinking happens out of sight.
                if thinking_time > spent {
                    sleep(thinking_time - spent).await;
                }

                let delta = json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "signature_delta", "signature": signature}
                });
                yield format_event("content_block_delta", &delta);
                yield format_event(
                    "content_block_stop",
                    &json!({"type": "content_block_stop", "index": 0}),
                );
                text_index = 1;
            }

            let mut next_index = text_index;
            if !skip_text {
                // 2. content_block_start (text block).
                let block_start = json!({
                    "type": "content_block_start",
                    "index": text_index,
                    "content_block": {"type": "text", "text": ""}
                });
                yield format_event("content_block_start", &block_start);

                // 3. ping (Anthropic interleaves these to keep the connection warm).
                if text_index == 0 {
                    yield format_event("ping", &json!({"type": "ping"}));
                }

                // 4. content_block_delta for each token.
                for token in tokens {
                    let tbt = latency.sample_tbt();
                    if !tbt.is_zero() {
                        sleep(tbt).await;
                    }
                    let delta = json!({
                        "type": "content_block_delta",
                        "index": text_index,
                        "delta": {"type": "text_delta", "text": token}
                    });
                    yield format_event("content_block_delta", &delta);
                }

                // 5. content_block_stop.
                yield format_event(
                    "content_block_stop",
                    &json!({"type": "content_block_stop", "index": text_index}),
                );
                next_index = text_index + 1;
            }

            // 5b. tool_use blocks, input streamed as input_json_delta.
            let has_tool_uses = !tool_uses.is_empty();
            for (id, name, input) in tool_uses {
                let index = next_index;
                next_index += 1;
                let block_start = json!({
                    "type": "content_block_start",
                    "index": index,
                    "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                });
                yield format_event("content_block_start", &block_start);
                let text = serde_json::to_string(&input).unwrap_or_else(|_| "{}".into());
                for chunk in json_chunks(&text) {
                    let tbt = latency.sample_tbt();
                    if !tbt.is_zero() {
                        sleep(tbt).await;
                    }
                    let delta = json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "input_json_delta", "partial_json": chunk}
                    });
                    yield format_event("content_block_delta", &delta);
                }
                yield format_event(
                    "content_block_stop",
                    &json!({"type": "content_block_stop", "index": index}),
                );
            }

            // 6. message_delta with final stop_reason + cumulative output usage.
            let message_delta = json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": if has_tool_uses { "tool_use" } else { "end_turn" },
                    "stop_sequence": null
                },
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

/// Word-level tokenization (keeps whitespace as separate tokens).
fn tokenize_words(content: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current_word = String::new();
    for ch in content.chars() {
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
    thinking: Option<(String, String)>,
    tool_uses: Vec<(String, String, serde_json::Value)>,
    redacted_thinking: Option<String>,
    thinking_time: std::time::Duration,
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
            thinking: None,
            tool_uses: Vec::new(),
            redacted_thinking: None,
            thinking_time: std::time::Duration::ZERO,
            on_complete: None,
        }
    }

    /// Emit the thinking block as `redacted_thinking` carrying `data`.
    pub fn redacted_thinking(mut self, data: impl Into<String>) -> Self {
        self.redacted_thinking = Some(data.into());
        self
    }

    /// Total thinking time (hidden thinking still takes time).
    pub fn thinking_time(mut self, time: std::time::Duration) -> Self {
        self.thinking_time = time;
        self
    }

    /// Emit `tool_use` blocks after the text: (id, name, input).
    pub fn tool_uses(mut self, tool_uses: Vec<(String, String, serde_json::Value)>) -> Self {
        self.tool_uses = tool_uses;
        self
    }

    /// Emit a thinking block (index 0) before the text block.
    pub fn thinking(mut self, text: impl Into<String>, signature: impl Into<String>) -> Self {
        self.thinking = Some((text.into(), signature.into()));
        self
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
        if let Some((text, signature)) = self.thinking {
            stream = stream.with_thinking(text, signature);
        }
        if !self.tool_uses.is_empty() {
            stream = stream.with_tool_uses(self.tool_uses);
        }
        if let Some(data) = self.redacted_thinking {
            stream = stream.with_redacted_thinking(data);
        }
        stream = stream.with_thinking_time(self.thinking_time);
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

    #[tokio::test]
    async fn test_tool_use_streams_input_json_deltas() {
        use futures_util::StreamExt;
        let input = json!({"path": "/tmp/some/long/file/name.txt", "limit": 10});
        let stream = MessagesStreamBuilder::new("claude-sonnet-4-5", "")
            .latency(LatencyProfile::instant())
            .tool_uses(vec![("toolu_1".into(), "read_file".into(), input.clone())])
            .build();
        let events: Vec<String> = stream.into_stream().collect().await;
        let all = events.join("");
        assert!(!all.contains("\"type\":\"text\""), "no empty text block");
        assert!(all.contains("\"type\":\"tool_use\""));
        assert!(all.contains("\"stop_reason\":\"tool_use\""));
        let partial: String = events
            .iter()
            .filter(|e| e.contains("input_json_delta"))
            .map(|e| {
                let data = e.lines().find(|l| l.starts_with("data: ")).unwrap();
                let v: serde_json::Value = serde_json::from_str(&data[6..]).unwrap();
                v["delta"]["partial_json"].as_str().unwrap().to_string()
            })
            .collect();
        assert!(
            events
                .iter()
                .filter(|e| e.contains("input_json_delta"))
                .count()
                > 1
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&partial).unwrap(),
            input
        );
    }
}
