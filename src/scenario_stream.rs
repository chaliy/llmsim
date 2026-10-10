// Streaming engine for scenario steps (OpenAI chat completions shape).
//
// Unlike `ScriptedChatStream`, pacing comes from the step's resolved timing
// (TTFT, tokens per second, a mid-stream stall) and the stream can be cut to
// simulate a broken connection. Text is split into word tokens with the
// leading space attached (" word"), so token counts for stalls and cuts match
// what the scenario author wrote in `lorem_tokens`.
//
// Decision: reasoning is streamed as `delta.reasoning_content`, the field
// OpenAI-compatible providers (DeepSeek, vLLM, OpenRouter) use for reasoning
// deltas, ahead of the answer.

use crate::ids::{prefixed_id, unix_timestamp};
use crate::latency::LatencyProfile;
use crate::openai::{ChatCompletionChunk, Usage};
use crate::scenario::ResolvedTiming;
use crate::script::SimToolCall;
use crate::script_stream::{format_sse, tool_call_chunks};
use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;
use std::time::Duration;
use tokio::time::sleep;

type Callback = Box<dyn FnOnce() + Send + 'static>;

/// Item type of a scenario stream: an SSE frame, or an error that aborts the
/// HTTP body mid-flight (the client sees a dropped connection).
pub type StreamItem = Result<String, std::io::Error>;

/// Per-token pacing for one step: resolved timing, falling back to the
/// latency profile (scaled by `speed`) for anything the scenario leaves unset.
#[derive(Debug, Clone)]
pub struct Pacing {
    pub timing: ResolvedTiming,
    pub latency: LatencyProfile,
}

impl Pacing {
    pub fn new(timing: ResolvedTiming, latency: LatencyProfile) -> Self {
        Self { timing, latency }
    }

    pub fn ttft(&self) -> Duration {
        self.timing
            .ttft
            .unwrap_or_else(|| self.latency.sample_ttft().mul_f64(self.timing.speed))
    }

    pub fn inter_token(&self) -> Duration {
        self.timing
            .inter_token
            .unwrap_or_else(|| self.latency.sample_tbt().mul_f64(self.timing.speed))
    }

    /// Delay before emitting token number `emitted` (0-based, after TTFT):
    /// the inter-token gap plus the stall when it is due.
    pub fn before_token(&self, emitted: usize) -> Duration {
        let mut d = if emitted == 0 {
            Duration::ZERO
        } else {
            self.inter_token()
        };
        if let Some(stall) = self.timing.stall {
            if emitted > 0 && emitted == stall.after_tokens {
                d += stall.duration;
            }
        }
        d
    }

    /// Total time to produce `tokens` tokens (TTFT, gaps and stall), for
    /// non-streaming responses.
    pub fn total(&self, tokens: usize) -> Duration {
        let mut d = self.ttft();
        for i in 0..tokens {
            d += self.before_token(i);
        }
        d
    }
}

/// Split text into word tokens with the leading whitespace attached.
pub fn word_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    for ch in text.chars() {
        if ch.is_whitespace() && in_word {
            tokens.push(std::mem::take(&mut cur));
            in_word = false;
        } else if !ch.is_whitespace() {
            in_word = true;
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Streamed scenario step: optional reasoning, optional text, then tool
/// calls, paced by `Pacing`.
pub struct ScenarioChatStream {
    model: String,
    reasoning: Option<String>,
    text: String,
    tool_calls: Vec<SimToolCall>,
    pacing: Pacing,
    cut_after_tokens: Option<usize>,
    usage: Option<Usage>,
    on_complete: Option<Callback>,
    on_cut: Option<Callback>,
}

impl ScenarioChatStream {
    pub fn new(
        model: impl Into<String>,
        text: String,
        tool_calls: Vec<SimToolCall>,
        pacing: Pacing,
    ) -> Self {
        Self {
            model: model.into(),
            reasoning: None,
            text,
            tool_calls,
            pacing,
            cut_after_tokens: None,
            usage: None,
            on_complete: None,
            on_cut: None,
        }
    }

    pub fn with_reasoning(mut self, reasoning: Option<String>) -> Self {
        self.reasoning = reasoning;
        self
    }

    /// Drop the stream after this many tokens, without a finish event.
    pub fn with_cut_after(mut self, tokens: Option<usize>) -> Self {
        self.cut_after_tokens = tokens;
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_on_complete(mut self, f: impl FnOnce() + Send + 'static) -> Self {
        self.on_complete = Some(Box::new(f));
        self
    }

    pub fn with_on_cut(mut self, f: impl FnOnce() + Send + 'static) -> Self {
        self.on_cut = Some(Box::new(f));
        self
    }

    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = StreamItem> + Send>> {
        let id = prefixed_id("chatcmpl-");
        let created = unix_timestamp();
        let model = self.model;
        let reasoning = self
            .reasoning
            .as_deref()
            .map(word_tokens)
            .unwrap_or_default();
        let text = word_tokens(&self.text);
        let tool_calls = self.tool_calls;
        let pacing = self.pacing;
        let cut = self.cut_after_tokens;
        let usage = self.usage;
        let on_complete = self.on_complete;
        let on_cut = self.on_cut;

        Box::pin(stream! {
            let ttft = pacing.ttft();
            if !ttft.is_zero() {
                sleep(ttft).await;
            }
            let role = ChatCompletionChunk::new(id.clone(), model.clone(), created).with_role();
            yield Ok(format_sse(&role));

            let tokens = reasoning
                .into_iter()
                .map(|t| (true, t))
                .chain(text.into_iter().map(|t| (false, t)));
            let mut emitted = 0usize;
            for (is_reasoning, token) in tokens {
                if cut == Some(emitted) {
                    break;
                }
                let delay = pacing.before_token(emitted);
                if !delay.is_zero() {
                    sleep(delay).await;
                }
                let frame = if is_reasoning {
                    reasoning_frame(&id, &model, created, &token)
                } else {
                    format_sse(
                        &ChatCompletionChunk::new(id.clone(), model.clone(), created)
                            .with_content(token),
                    )
                };
                yield Ok(frame);
                emitted += 1;
            }

            if let Some(limit) = cut {
                if emitted <= limit {
                    if let Some(cb) = on_cut {
                        cb();
                    }
                    yield Err(std::io::Error::other(format!(
                        "llmsim scenario: stream cut after {emitted} tokens"
                    )));
                    return;
                }
            }

            for (index, call) in tool_calls.iter().enumerate() {
                let delay = pacing.inter_token();
                if !delay.is_zero() {
                    sleep(delay).await;
                }
                let (announce, args) = tool_call_chunks(&id, &model, created, index, call);
                yield Ok(format_sse(&announce));
                yield Ok(format_sse(&args));
            }

            let finish_reason = if tool_calls.is_empty() { "stop" } else { "tool_calls" };
            let mut finish = ChatCompletionChunk::new(id.clone(), model.clone(), created)
                .with_finish(finish_reason.to_string());
            if let Some(u) = usage {
                finish = finish.with_usage(u);
            }
            yield Ok(format_sse(&finish));
            yield Ok("data: [DONE]\n\n".to_string());

            if let Some(cb) = on_complete {
                cb();
            }
        })
    }
}

fn reasoning_frame(id: &str, model: &str, created: i64, token: &str) -> String {
    let chunk = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "system_fingerprint": "fp_llmsim",
        "choices": [{
            "index": 0,
            "delta": { "reasoning_content": token },
            "finish_reason": null
        }]
    });
    format!("data: {chunk}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ResolvedStall;
    use futures_util::StreamExt;
    use serde_json::json;

    fn instant_timing() -> ResolvedTiming {
        ResolvedTiming {
            ttft: Some(Duration::ZERO),
            inter_token: Some(Duration::ZERO),
            stall: None,
            cut_after_tokens: None,
            speed: 1.0,
        }
    }

    fn pacing(timing: ResolvedTiming) -> Pacing {
        Pacing::new(timing, LatencyProfile::instant())
    }

    async fn collect(s: ScenarioChatStream) -> (String, bool) {
        let items: Vec<StreamItem> = s.into_stream().collect().await;
        let errored = items.iter().any(|i| i.is_err());
        let body = items.into_iter().filter_map(Result::ok).collect();
        (body, errored)
    }

    #[test]
    fn word_tokens_keep_leading_space() {
        assert_eq!(
            word_tokens("Hello big  world"),
            vec!["Hello", " big", "  world"]
        );
        assert_eq!(word_tokens(""), Vec::<String>::new());
        assert_eq!(word_tokens("one").concat(), "one");
    }

    #[tokio::test]
    async fn streams_reasoning_text_and_tools() {
        let calls = vec![SimToolCall {
            name: "bash".into(),
            arguments: json!({"command": "ls"}),
            id: Some("call_llmsim_x_0_0_ab".into()),
        }];
        let s =
            ScenarioChatStream::new("gpt-5", "two words".into(), calls, pacing(instant_timing()))
                .with_reasoning(Some("thinking hard".into()));
        let (body, errored) = collect(s).await;
        assert!(!errored);
        let r = body.find("\"reasoning_content\":\"thinking\"").unwrap();
        let c = body.find("\"content\":\"two\"").unwrap();
        assert!(r < c, "reasoning streams before the answer");
        assert!(body.contains("\"content\":\" words\""));
        assert!(body.contains("\"name\":\"bash\""));
        assert!(body.contains("\"finish_reason\":\"tool_calls\""));
        assert!(body.ends_with("data: [DONE]\n\n"));
    }

    #[tokio::test]
    async fn cut_drops_stream_without_finish() {
        let s = ScenarioChatStream::new(
            "gpt-5",
            "a b c d e f".into(),
            vec![],
            pacing(instant_timing()),
        )
        .with_cut_after(Some(3));
        let (body, errored) = collect(s).await;
        assert!(errored);
        assert_eq!(body.matches("\"content\"").count(), 3);
        assert!(!body.contains("finish_reason\":\"stop"));
        assert!(!body.contains("[DONE]"));
    }

    #[tokio::test]
    async fn cut_beyond_text_still_drops_before_finish() {
        let s = ScenarioChatStream::new("gpt-5", "a b".into(), vec![], pacing(instant_timing()))
            .with_cut_after(Some(50));
        let (body, errored) = collect(s).await;
        assert!(errored);
        assert!(!body.contains("[DONE]"));
    }

    #[tokio::test]
    async fn paces_ttft_tokens_and_stall() {
        let timing = ResolvedTiming {
            ttft: Some(Duration::from_millis(50)),
            inter_token: Some(Duration::from_millis(10)),
            stall: Some(ResolvedStall {
                after_tokens: 2,
                duration: Duration::from_millis(100),
            }),
            cut_after_tokens: None,
            speed: 1.0,
        };
        let p = pacing(timing);
        // 50 TTFT + 3 gaps of 10 + 100 stall.
        assert_eq!(p.total(4), Duration::from_millis(180));

        let start = std::time::Instant::now();
        let s = ScenarioChatStream::new("gpt-5", "a b c d".into(), vec![], p);
        let (_, errored) = collect(s).await;
        assert!(!errored);
        assert!(start.elapsed() >= Duration::from_millis(180));
    }

    #[test]
    fn profile_fallback_is_scaled_by_speed() {
        let timing = ResolvedTiming {
            ttft: None,
            inter_token: None,
            stall: None,
            cut_after_tokens: None,
            speed: 0.0,
        };
        let p = Pacing::new(timing, LatencyProfile::gpt5());
        assert_eq!(p.ttft(), Duration::ZERO);
        assert_eq!(p.inter_token(), Duration::ZERO);
    }
}
