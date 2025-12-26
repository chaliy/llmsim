// Streaming Engine Module
// Implements token-by-token streaming with realistic latency simulation.

use crate::latency::LatencyProfile;
use crate::openai::{ChatCompletionChunk, Role, Usage};
use async_stream::stream;
use futures::Stream;
use std::pin::Pin;
use tokio::time::sleep;

/// A streaming response that yields chunks with simulated delays
pub struct TokenStream {
    /// The response ID (shared across all chunks)
    id: String,
    /// The model name
    model: String,
    /// Unix timestamp of creation
    created: i64,
    /// Latency profile for timing simulation
    latency: LatencyProfile,
    /// The full response text to stream
    content: String,
    /// Token usage (included in final chunk if stream_options.include_usage is true)
    usage: Option<Usage>,
}

impl TokenStream {
    pub fn new(id: String, model: String, content: String, latency: LatencyProfile) -> Self {
        Self {
            id,
            model,
            created: chrono::Utc::now().timestamp(),
            latency,
            content,
            usage: None,
        }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Convert the content into chunks for streaming
    /// This simulates word-by-word streaming (approximating token streaming)
    fn tokenize(&self) -> Vec<String> {
        // Split by whitespace but keep spaces as separate tokens
        // This approximates token-level streaming
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
        let id = self.id.clone();
        let model = self.model.clone();
        let created = self.created;
        let latency = self.latency.clone();
        let usage = self.usage.clone();

        Box::pin(stream! {
            // Initial delay (time to first token)
            let ttft = latency.sample_ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }

            // First chunk: role announcement
            let role_chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                .with_role();
            yield format_sse(&role_chunk);

            // Content chunks
            for token in tokens {
                // Inter-token delay
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                let content_chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                    .with_content(token);
                yield format_sse(&content_chunk);
            }

            // Final chunk with finish_reason
            let mut finish_chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                .with_finish("stop".to_string());

            // Include usage in final chunk if available
            if let Some(u) = usage {
                finish_chunk = finish_chunk.with_usage(u);
            }
            yield format_sse(&finish_chunk);

            // Done marker
            yield "data: [DONE]\n\n".to_string();
        })
    }

    /// Create a stream that yields ChatCompletionChunk objects directly
    pub fn into_chunk_stream(self) -> Pin<Box<dyn Stream<Item = ChatCompletionChunk> + Send>> {
        let tokens = self.tokenize();
        let id = self.id.clone();
        let model = self.model.clone();
        let created = self.created;
        let latency = self.latency.clone();
        let usage = self.usage.clone();

        Box::pin(stream! {
            // Initial delay (time to first token)
            let ttft = latency.sample_ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }

            // First chunk: role announcement
            yield ChatCompletionChunk::new(id.clone(), model.clone(), created).with_role();

            // Content chunks
            for token in tokens {
                // Inter-token delay
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                yield ChatCompletionChunk::new(id.clone(), model.clone(), created)
                    .with_content(token);
            }

            // Final chunk with finish_reason
            let mut finish_chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                .with_finish("stop".to_string());

            if let Some(u) = usage {
                finish_chunk = finish_chunk.with_usage(u);
            }
            yield finish_chunk;
        })
    }
}

/// Format a chunk as Server-Sent Event
pub fn format_sse(chunk: &ChatCompletionChunk) -> String {
    let json = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".to_string());
    format!("data: {}\n\n", json)
}

/// Builder for creating token streams
pub struct TokenStreamBuilder {
    id: Option<String>,
    model: String,
    content: String,
    latency: LatencyProfile,
    usage: Option<Usage>,
}

impl TokenStreamBuilder {
    pub fn new(model: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: None,
            model: model.into(),
            content: content.into(),
            latency: LatencyProfile::default(),
            usage: None,
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

    pub fn build(self) -> TokenStream {
        let id = self
            .id
            .unwrap_or_else(|| format!("chatcmpl-{}", uuid::Uuid::new_v4()));

        let mut stream = TokenStream::new(id, self.model, self.content, self.latency);
        if let Some(usage) = self.usage {
            stream = stream.with_usage(usage);
        }
        stream
    }
}

/// Simulated role chunk for streaming
pub fn create_role_chunk(id: &str, model: &str, created: i64) -> ChatCompletionChunk {
    let mut chunk = ChatCompletionChunk::new(id.to_string(), model.to_string(), created);
    chunk.choices = vec![crate::openai::ChunkChoice {
        index: 0,
        delta: crate::openai::ChunkDelta {
            role: Some(Role::Assistant),
            content: None,
            tool_calls: None,
        },
        finish_reason: None,
        logprobs: None,
    }];
    chunk
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_token_stream_basic() {
        let stream = TokenStreamBuilder::new("gpt-4", "Hello world")
            .latency(LatencyProfile::instant())
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Should have: role chunk, "Hello", " ", "world", finish chunk, [DONE]
        assert!(chunks.len() >= 4);
        assert!(chunks.last().unwrap().contains("[DONE]"));
    }

    #[tokio::test]
    async fn test_chunk_stream() {
        let stream = TokenStreamBuilder::new("gpt-4", "Test message")
            .latency(LatencyProfile::instant())
            .build();

        let chunks: Vec<ChatCompletionChunk> = stream.into_chunk_stream().collect().await;

        // First chunk should have role
        assert!(chunks[0].choices[0].delta.role.is_some());

        // Last chunk should have finish_reason
        let last = chunks.last().unwrap();
        assert!(last.choices[0].finish_reason.is_some());
    }

    #[tokio::test]
    async fn test_stream_with_usage() {
        let usage = Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        };

        let stream = TokenStreamBuilder::new("gpt-4", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let chunks: Vec<ChatCompletionChunk> = stream.into_chunk_stream().collect().await;

        // Last chunk should include usage
        let last = chunks.last().unwrap();
        assert!(last.usage.is_some());
        assert_eq!(last.usage.as_ref().unwrap().total_tokens, 15);
    }

    #[tokio::test]
    async fn test_sse_format() {
        let chunk =
            ChatCompletionChunk::new("test-id".to_string(), "gpt-4".to_string(), 1234567890)
                .with_content("Hello".to_string());

        let sse = format_sse(&chunk);
        assert!(sse.starts_with("data: "));
        assert!(sse.ends_with("\n\n"));
        assert!(sse.contains("\"content\":\"Hello\""));
    }

    #[tokio::test]
    async fn test_tokenize() {
        let stream = TokenStream::new(
            "id".to_string(),
            "gpt-4".to_string(),
            "Hello, world!".to_string(),
            LatencyProfile::instant(),
        );

        let tokens = stream.tokenize();
        assert_eq!(tokens, vec!["Hello,", " ", "world!"]);
    }

    #[tokio::test]
    async fn test_empty_content() {
        let stream = TokenStreamBuilder::new("gpt-4", "")
            .latency(LatencyProfile::instant())
            .build();

        let chunks: Vec<ChatCompletionChunk> = stream.into_chunk_stream().collect().await;

        // Should still have role and finish chunks
        assert!(chunks.len() >= 2);
    }
}
