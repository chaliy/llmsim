// Scripted streaming for OpenAI chat completions.
//
// The plain `TokenStream` only emits text. Scripted mode can also emit
// tool calls and signal `finish_reason=tool_calls` — modelled here as a
// dedicated builder so we can keep the simple path simple.

use crate::ids::{prefixed_id, unix_timestamp};
use crate::latency::LatencyProfile;
use crate::openai::{
    ChatCompletionChunk, ChunkChoice, ChunkDelta, ChunkFunctionCall, ChunkToolCall, Role, Usage,
};
use crate::script::SimToolCall;
use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;
use tokio::time::sleep;

type OnCompleteCallback = Box<dyn FnOnce() + Send + 'static>;

/// Streamed scripted turn: optional text body followed by optional
/// tool calls. At least one of `text` or `tool_calls` must be non-empty.
pub struct ScriptedChatStream {
    id: String,
    model: String,
    created: i64,
    latency: LatencyProfile,
    text: String,
    tool_calls: Vec<SimToolCall>,
    usage: Option<Usage>,
    on_complete: Option<OnCompleteCallback>,
}

impl ScriptedChatStream {
    pub fn new(
        model: impl Into<String>,
        text: String,
        tool_calls: Vec<SimToolCall>,
        latency: LatencyProfile,
    ) -> Self {
        Self {
            id: prefixed_id("chatcmpl-"),
            model: model.into(),
            created: unix_timestamp(),
            latency,
            text,
            tool_calls,
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

    fn tokenize_text(&self) -> Vec<String> {
        // Mirror TokenStream's word-boundary split: keep whitespace as
        // its own delta so downstream re-joins cleanly.
        let mut tokens = Vec::new();
        let mut current = String::new();
        for ch in self.text.chars() {
            if ch.is_whitespace() {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }

    /// Render as SSE chunks for the HTTP body.
    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        let tokens = self.tokenize_text();
        let id = self.id.clone();
        let model = self.model.clone();
        let created = self.created;
        let latency = self.latency.clone();
        let tool_calls = self.tool_calls.clone();
        let usage = self.usage.clone();
        let on_complete = self.on_complete;
        let has_tool_calls = !tool_calls.is_empty();

        Box::pin(stream! {
            // TTFT.
            let ttft = latency.sample_ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }

            // Role chunk first, as real OpenAI does.
            let role_chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                .with_role();
            yield format_sse(&role_chunk);

            // Text deltas (if any).
            for token in tokens {
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }
                let chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                    .with_content(token);
                yield format_sse(&chunk);
            }

            // Tool call deltas. Each call is emitted as two chunks: an
            // "announce" with name+id+empty args, then a single chunk
            // streaming the full arguments JSON. This matches the
            // shape SDKs (OpenAI Python, LangChain) parse, while
            // staying simple — chunked-args streaming is overkill for
            // a test fixture.
            for (index, call) in tool_calls.iter().enumerate() {
                let tbt = latency.sample_tbt();
                if !tbt.is_zero() {
                    sleep(tbt).await;
                }

                // Announce.
                let announce = ChatCompletionChunk {
                    id: id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model.clone(),
                    system_fingerprint: Some("fp_llmsim".to_string()),
                    usage: None,
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: None,
                            content: None,
                            tool_calls: Some(vec![ChunkToolCall {
                                index: index as u32,
                                id: call.id.clone(),
                                call_type: Some("function".to_string()),
                                function: Some(ChunkFunctionCall {
                                    name: Some(call.name.clone()),
                                    arguments: Some(String::new()),
                                }),
                            }]),
                        },
                        finish_reason: None,
                        logprobs: None,
                    }],
                };
                yield format_sse(&announce);

                // Arguments delta (single chunk).
                let args_str = serde_json::to_string(&call.arguments)
                    .unwrap_or_else(|_| "{}".to_string());
                let args_chunk = ChatCompletionChunk {
                    id: id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model.clone(),
                    system_fingerprint: Some("fp_llmsim".to_string()),
                    usage: None,
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: None,
                            content: None,
                            tool_calls: Some(vec![ChunkToolCall {
                                index: index as u32,
                                id: None,
                                call_type: None,
                                function: Some(ChunkFunctionCall {
                                    name: None,
                                    arguments: Some(args_str),
                                }),
                            }]),
                        },
                        finish_reason: None,
                        logprobs: None,
                    }],
                };
                yield format_sse(&args_chunk);
            }

            // Finish chunk.
            let finish_reason = if has_tool_calls { "tool_calls" } else { "stop" };
            let mut finish_chunk = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                .with_finish(finish_reason.to_string());
            if let Some(u) = usage {
                finish_chunk = finish_chunk.with_usage(u);
            }
            yield format_sse(&finish_chunk);

            yield "data: [DONE]\n\n".to_string();

            if let Some(cb) = on_complete {
                cb();
            }
        })
    }
}

fn format_sse(chunk: &ChatCompletionChunk) -> String {
    let json = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".to_string());
    format!("data: {}\n\n", json)
}

/// Build a non-streaming OpenAI ChatCompletion response for a scripted turn.
pub fn build_chat_completion_response(
    model: String,
    text: Option<String>,
    tool_calls: Vec<crate::openai::ToolCall>,
    usage: Usage,
) -> crate::openai::ChatCompletionResponse {
    use crate::openai::{ChatCompletionResponse, Choice, Message};

    let finish_reason = if tool_calls.is_empty() {
        "stop".to_string()
    } else {
        "tool_calls".to_string()
    };

    let message = Message {
        role: Role::Assistant,
        content: text.map(crate::openai::ChatMessageContent::Text),
        name: None,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
    };

    ChatCompletionResponse {
        id: prefixed_id("chatcmpl-"),
        object: "chat.completion".to_string(),
        created: unix_timestamp(),
        model,
        choices: vec![Choice {
            index: 0,
            message,
            finish_reason: Some(finish_reason),
            logprobs: None,
        }],
        usage: Some(usage),
        system_fingerprint: Some("fp_llmsim".to_string()),
    }
}

/// Convert scripted `SimToolCall`s into wire-format `ToolCall`s,
/// generating ids when missing.
pub fn materialize_tool_calls(
    turn_index: usize,
    calls: &[SimToolCall],
) -> Vec<crate::openai::ToolCall> {
    use crate::openai::{FunctionCall, ToolCall};
    calls
        .iter()
        .enumerate()
        .map(|(i, c)| ToolCall {
            id: c
                .id
                .clone()
                .unwrap_or_else(|| crate::script::auto_tool_call_id(turn_index, i)),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: c.name.clone(),
                arguments: serde_json::to_string(&c.arguments).unwrap_or_else(|_| "{}".to_string()),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde_json::json;

    #[tokio::test]
    async fn streams_text_only_with_stop() {
        let stream = ScriptedChatStream::new(
            "gpt-5",
            "hello world".to_string(),
            vec![],
            LatencyProfile::instant(),
        );
        let chunks: Vec<String> = stream.into_stream().collect().await;
        let joined = chunks.join("");
        assert!(joined.contains("\"content\":\"hello\""));
        assert!(joined.contains("\"content\":\"world\""));
        assert!(joined.contains("\"finish_reason\":\"stop\""));
        assert!(joined.contains("[DONE]"));
    }

    #[tokio::test]
    async fn streams_tool_calls_with_tool_calls_finish() {
        let calls = vec![SimToolCall {
            name: "bash".into(),
            arguments: json!({"command": "ls"}),
            id: Some("call_x".into()),
        }];
        let stream =
            ScriptedChatStream::new("gpt-5", String::new(), calls, LatencyProfile::instant());
        let chunks: Vec<String> = stream.into_stream().collect().await;
        let joined = chunks.join("");
        assert!(joined.contains("\"name\":\"bash\""));
        assert!(joined.contains("\"id\":\"call_x\""));
        // arguments are serialized as a JSON string, so the inner
        // quotes are escaped on the wire.
        assert!(joined.contains("\\\"command\\\""));
        assert!(joined.contains("\"finish_reason\":\"tool_calls\""));
    }

    #[tokio::test]
    async fn mixed_text_and_tool_calls() {
        let calls = vec![SimToolCall {
            name: "x".into(),
            arguments: json!({}),
            id: None,
        }];
        let stream = ScriptedChatStream::new(
            "gpt-5",
            "thinking".to_string(),
            calls,
            LatencyProfile::instant(),
        );
        let chunks: Vec<String> = stream.into_stream().collect().await;
        let joined = chunks.join("");
        assert!(joined.contains("\"content\":\"thinking\""));
        assert!(joined.contains("\"name\":\"x\""));
        assert!(joined.contains("\"finish_reason\":\"tool_calls\""));
    }

    #[test]
    fn materializes_tool_call_ids() {
        let calls = vec![
            SimToolCall {
                name: "a".into(),
                arguments: json!({"k": 1}),
                id: None,
            },
            SimToolCall {
                name: "b".into(),
                arguments: json!({}),
                id: Some("provided".into()),
            },
        ];
        let materialized = materialize_tool_calls(2, &calls);
        assert_eq!(materialized[0].id, "call_llmsim_2_0");
        assert_eq!(materialized[1].id, "provided");
        assert_eq!(materialized[0].function.arguments, "{\"k\":1}");
    }

    #[test]
    fn builds_non_streaming_response_with_tool_calls() {
        let calls = materialize_tool_calls(
            0,
            &[SimToolCall {
                name: "bash".into(),
                arguments: json!({"command": "ls"}),
                id: None,
            }],
        );
        let usage = Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        };
        let resp = build_chat_completion_response("gpt-5".to_string(), None, calls, usage);
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("tool_calls"));
        assert!(resp.choices[0].message.tool_calls.is_some());
        assert!(resp.choices[0].message.content.is_none());
    }
}
