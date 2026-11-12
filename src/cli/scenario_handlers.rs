// Scenarios on the Responses API and Anthropic Messages.
//
// Chat completions plays scenarios in `handlers.rs`; this file maps the other
// two request shapes onto the provider-neutral conversation that
// `scenario::resolve` reads, then plays the step with the same attempt,
// failure, timing and cut rules, rendered by `scenario_frames`.
//
// Decisions:
// - Responses history items that belong to one model turn (`reasoning`,
//   assistant `message`, `function_call`) merge into one assistant message, so
//   a step that made three calls still counts as one step.
// - An Anthropic user message carrying `tool_result` blocks is tool output;
//   only its text blocks (if any) count as a new user message.

use super::handlers::{
    count_responses_input_image_tokens, extract_input_text, sim_error_to_responses_api_response,
};
use super::state::AppState;
use crate::anthropic::{
    MessageContent as AMessageContent, MessagesRequest, RequestContentBlock, Role as ARole,
    SystemPrompt, Usage as AUsage,
};
use crate::openai::{
    ContentPart, InputItem, InputRole, MessageContent, OutputTokensDetails, ResponsesInput,
    ResponsesRequest, ResponsesTool, ResponsesUsage,
};
use crate::scenario::{
    AttemptOutcome, ConversationMessage, MessageRole, Resolution, StepError, ToolSpec,
};
use crate::scenario_frames::{
    anthropic_body, anthropic_content, responses_body, responses_output, thinking_signature,
    AnthropicFrames, ResponsesFrames,
};
use crate::scenario_stream::{paced_frames, word_tokens, Callback, Pacing, StepFrames};
use crate::script::{SimError, SimToolCall, SimTurn};
use crate::LatencyProfile;
use axum::{
    body::Body,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Request shapes -> conversation
// ---------------------------------------------------------------------------

/// Append an assistant-side item, merging into the open assistant turn.
fn push_assistant(out: &mut Vec<ConversationMessage>, text: String, ids: Vec<String>) {
    match out.last_mut() {
        Some(last) if last.role == MessageRole::Assistant => {
            if !text.is_empty() {
                if !last.text.is_empty() {
                    last.text.push(' ');
                }
                last.text.push_str(&text);
            }
            last.tool_call_ids.extend(ids);
        }
        _ => {
            out.push(ConversationMessage::new(MessageRole::Assistant, text).with_tool_call_ids(ids))
        }
    }
}

fn responses_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::InputText { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// The conversation and tools of a Responses request.
pub fn responses_conversation(
    request: &ResponsesRequest,
) -> (Vec<ConversationMessage>, Vec<ToolSpec>) {
    let mut out = Vec::new();
    if let Some(i) = &request.instructions {
        out.push(ConversationMessage::new(MessageRole::System, i.clone()));
    }
    match &request.input {
        ResponsesInput::Text(t) => out.push(ConversationMessage::new(MessageRole::User, t.clone())),
        ResponsesInput::Items(items) => {
            for item in items {
                match item {
                    InputItem::Message { role, content } => {
                        let text = responses_text(content);
                        match role {
                            InputRole::User => {
                                out.push(ConversationMessage::new(MessageRole::User, text))
                            }
                            InputRole::System | InputRole::Developer => {
                                out.push(ConversationMessage::new(MessageRole::System, text))
                            }
                            InputRole::Assistant => push_assistant(&mut out, text, Vec::new()),
                        }
                    }
                    InputItem::FunctionCall { call_id, .. } => {
                        push_assistant(&mut out, String::new(), vec![call_id.clone()])
                    }
                    InputItem::Reasoning { .. } => {
                        push_assistant(&mut out, String::new(), Vec::new())
                    }
                    InputItem::FunctionCallOutput { call_id, output } => out.push(
                        ConversationMessage::new(MessageRole::Tool, output.clone())
                            .with_tool_call_ids([call_id.clone()]),
                    ),
                }
            }
        }
    }
    let tools = request
        .tools
        .iter()
        .flatten()
        .filter_map(|t| match t {
            ResponsesTool::Function {
                name, parameters, ..
            } => Some(ToolSpec {
                name: name.clone(),
                parameters: parameters.clone(),
            }),
            _ => None,
        })
        .collect();
    (out, tools)
}

/// The conversation and tools of a Messages request.
pub fn anthropic_conversation(
    request: &MessagesRequest,
) -> (Vec<ConversationMessage>, Vec<ToolSpec>) {
    let mut out = Vec::new();
    match &request.system {
        Some(SystemPrompt::Text(t)) => {
            out.push(ConversationMessage::new(MessageRole::System, t.clone()))
        }
        Some(s @ SystemPrompt::Blocks(_)) => out.push(ConversationMessage::new(
            MessageRole::System,
            s.extract_text(),
        )),
        None => {}
    }
    for m in &request.messages {
        match (&m.role, &m.content) {
            (ARole::User, AMessageContent::Text(t)) => {
                out.push(ConversationMessage::new(MessageRole::User, t.clone()))
            }
            (ARole::User, AMessageContent::Blocks(blocks)) => {
                let mut texts = Vec::new();
                for b in blocks {
                    match b {
                        RequestContentBlock::ToolResult { tool_use_id, .. } => out.push(
                            ConversationMessage::new(MessageRole::Tool, String::new())
                                .with_tool_call_ids([tool_use_id.clone()]),
                        ),
                        RequestContentBlock::Text { text, .. } => texts.push(text.as_str()),
                        _ => {}
                    }
                }
                let only_tool_results = blocks
                    .iter()
                    .all(|b| matches!(b, RequestContentBlock::ToolResult { .. }));
                if !only_tool_results {
                    out.push(ConversationMessage::new(MessageRole::User, texts.join(" ")));
                }
            }
            (ARole::Assistant, content) => {
                let ids = match content {
                    AMessageContent::Blocks(blocks) => blocks
                        .iter()
                        .filter_map(|b| match b {
                            RequestContentBlock::ToolUse { id, .. } => Some(id.clone()),
                            _ => None,
                        })
                        .collect(),
                    AMessageContent::Text(_) => Vec::new(),
                };
                out.push(
                    ConversationMessage::new(MessageRole::Assistant, content.extract_text())
                        .with_tool_call_ids(ids),
                );
            }
        }
    }
    let tools = request
        .tools
        .iter()
        .flatten()
        .map(|t| ToolSpec {
            name: t.name.clone(),
            parameters: t.input_schema.clone(),
        })
        .collect();
    (out, tools)
}

// ---------------------------------------------------------------------------
// Playing a step
// ---------------------------------------------------------------------------

/// A step ready to render: what to say and how fast.
struct Play {
    text: Option<String>,
    calls: Vec<SimToolCall>,
    reasoning: Option<String>,
    text_tokens: usize,
    reasoning_tokens: usize,
    tool_tokens: usize,
    pacing: Pacing,
    cut: Option<usize>,
}

impl Play {
    fn output_tokens(&self) -> usize {
        self.text_tokens + self.reasoning_tokens + self.tool_tokens
    }
}

/// What a resolved step does on this attempt: an error to render, or a play.
enum Outcome {
    Fail(StepError),
    Play(Play),
}

fn prepare(resolution: &Resolution, attempt: u32, latency: LatencyProfile) -> Outcome {
    let cut = match resolution.outcome(attempt) {
        AttemptOutcome::Fail(err) => return Outcome::Fail(err),
        AttemptOutcome::Respond { cut_after_tokens } => cut_after_tokens,
    };
    let (text, calls) = match resolution.turn.clone() {
        SimTurn::Assistant { text } => (Some(text), Vec::new()),
        SimTurn::ToolCalls { calls } => (None, calls),
        SimTurn::Mixed { text, calls } => (Some(text), calls),
        SimTurn::Error(err) => return Outcome::Fail(StepError::from(err)),
    };
    let tool_tokens = calls
        .iter()
        .map(|c| {
            let args = serde_json::to_string(&c.arguments).unwrap_or_default();
            crate::count_tokens_default(&args).unwrap_or(args.split_whitespace().count()) + 1
        })
        .sum();
    Outcome::Play(Play {
        text_tokens: text.as_deref().map_or(0, |t| word_tokens(t).len()),
        reasoning_tokens: resolution
            .reasoning
            .as_deref()
            .map_or(0, |r| word_tokens(r).len()),
        tool_tokens,
        reasoning: resolution.reasoning.clone(),
        text,
        calls,
        pacing: Pacing::new(resolution.timing.clone(), latency),
        cut,
    })
}

fn trace(mut response: Response, resolution: &Resolution, attempt: u32) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        "x-llmsim-scenario",
        resolution
            .scenario
            .parse()
            .expect("scenario names are ASCII"),
    );
    headers.insert("x-llmsim-step", resolution.step.into());
    headers.insert("x-llmsim-attempt", attempt.into());
    response
}

fn with_retry(mut response: Response, err: &StepError) -> Response {
    if let (Some(secs), Some(ms)) = (err.retry_after_secs(), err.retry_after_ms) {
        let headers = response.headers_mut();
        headers.insert(header::RETRY_AFTER, secs.into());
        headers.insert("retry-after-ms", ms.into());
    }
    response
}

fn sse(stream: crate::scenario_stream::StreamItemStream) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Headers, then a body that aborts: the client sees a dropped connection.
fn cut_body() -> Response {
    let body = Body::from_stream(futures_util::stream::once(async {
        Err::<String, _>(std::io::Error::other("llmsim scenario: response cut"))
    }));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap()
}

/// Shared tail: stream through `frames`, or wait and return `body`.
#[allow(clippy::too_many_arguments)]
async fn deliver<F: StepFrames>(
    state: &Arc<AppState>,
    stream: bool,
    play: Play,
    frames: F,
    input_tokens: u32,
    request_start: Instant,
    body: impl FnOnce(Play) -> Response,
) -> Response {
    let output_tokens = play.output_tokens() as u32;
    if stream {
        let stats = state.stats.clone();
        let cut_stats = state.stats.clone();
        let on_complete: Callback = Box::new(move || {
            stats.record_request_end(request_start.elapsed(), input_tokens, output_tokens);
        });
        let on_cut: Callback = Box::new(move || cut_stats.record_error(0));
        return sse(paced_frames(
            frames,
            play.reasoning.as_deref(),
            play.text.as_deref().unwrap_or(""),
            play.pacing.clone(),
            play.cut,
            Some(on_complete),
            Some(on_cut),
        ));
    }
    let tokens = play.text_tokens + play.reasoning_tokens;
    if let Some(limit) = play.cut {
        tokio::time::sleep(play.pacing.total(tokens.min(limit))).await;
        state.stats.record_error(0);
        return cut_body();
    }
    tokio::time::sleep(play.pacing.total(tokens)).await;
    state
        .stats
        .record_request_end(request_start.elapsed(), input_tokens, output_tokens);
    body(play)
}

/// Resolve a scenario for a request. `Err` carries the error to return.
fn resolve(
    state: &AppState,
    messages: &[ConversationMessage],
    tools: &[ToolSpec],
    model: &str,
) -> Result<Option<Resolution>, SimError> {
    let Some(scenarios) = state.scenarios.as_ref() else {
        return Ok(None);
    };
    crate::scenario::resolve(messages, tools, Some(model), scenarios).map_err(|err| {
        tracing::warn!(error = %err, "Scenario resolution failed");
        state.stats.record_error(err.status_code());
        err.to_sim_error()
    })
}

/// Play a scenario on the Responses API, if the request selects one.
pub(super) async fn responses(
    state: &Arc<AppState>,
    request: &ResponsesRequest,
    request_start: Instant,
    latency: LatencyProfile,
) -> Option<Response> {
    let (messages, tools) = responses_conversation(request);
    let resolution = match resolve(state, &messages, &tools, &request.model) {
        Ok(Some(r)) => r,
        Ok(None) => return None,
        Err(e) => return Some(sim_error_to_responses_api_response(&e)),
    };
    let attempt = state.attempts.next_attempt(resolution.fingerprint);
    let input_text = extract_input_text(&request.input, &request.instructions);
    let input_tokens = crate::count_tokens_default(&input_text)
        .unwrap_or(input_text.split_whitespace().count())
        + count_responses_input_image_tokens(&request.input);
    let play = match prepare(
        &resolution,
        attempt,
        latency.with_input_tokens(input_tokens),
    ) {
        Outcome::Fail(err) => {
            state.stats.record_error(err.error.status_code());
            let r = with_retry(sim_error_to_responses_api_response(&err.error), &err);
            return Some(trace(r, &resolution, attempt));
        }
        Outcome::Play(p) => p,
    };
    let output_tokens = play.output_tokens();
    let usage = ResponsesUsage {
        input_tokens_details: None,
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        total_tokens: (input_tokens + output_tokens) as u32,
        output_tokens_details: Some(OutputTokensDetails {
            reasoning_tokens: play.reasoning_tokens as u32,
        }),
    };
    let frames = ResponsesFrames::new(&request.model, play.calls.clone(), usage.clone());
    let model = request.model.clone();
    let response = deliver(
        state,
        request.stream,
        play,
        frames,
        input_tokens as u32,
        request_start,
        move |play| {
            let output =
                responses_output(play.reasoning.as_deref(), play.text.as_deref(), &play.calls);
            let body = responses_body(
                crate::ids::prefixed_id("resp_"),
                &model,
                crate::ids::unix_timestamp(),
                output,
                usage,
            );
            Json(body).into_response()
        },
    )
    .await;
    Some(trace(response, &resolution, attempt))
}

/// Play a scenario on Anthropic Messages, if the request selects one.
pub(super) async fn anthropic(
    state: &Arc<AppState>,
    request: &MessagesRequest,
    request_start: Instant,
    latency: LatencyProfile,
) -> Option<Response> {
    let (messages, tools) = anthropic_conversation(request);
    let resolution = match resolve(state, &messages, &tools, &request.model) {
        Ok(Some(r)) => r,
        Ok(None) => return None,
        Err(e) => {
            return Some(super::anthropic_handlers::sim_error_to_anthropic_response(
                &e,
            ))
        }
    };
    let attempt = state.attempts.next_attempt(resolution.fingerprint);
    let input_tokens = super::anthropic_handlers::count_input_tokens(request);
    let play = match prepare(
        &resolution,
        attempt,
        latency.with_input_tokens(input_tokens),
    ) {
        Outcome::Fail(err) => {
            state.stats.record_error(err.error.status_code());
            let r = with_retry(
                super::anthropic_handlers::sim_error_to_anthropic_response(&err.error),
                &err,
            );
            return Some(trace(r, &resolution, attempt));
        }
        Outcome::Play(p) => p,
    };
    let usage = AUsage::new(input_tokens as u32, play.output_tokens() as u32);
    let signature = thinking_signature(&resolution.fingerprint.to_string());
    let frames = AnthropicFrames::new(
        &request.model,
        play.calls.clone(),
        usage.clone(),
        signature.clone(),
    );
    let model = request.model.clone();
    let response = deliver(
        state,
        request.stream,
        play,
        frames,
        input_tokens as u32,
        request_start,
        move |play| {
            let content = anthropic_content(
                play.reasoning.as_deref(),
                play.text.as_deref(),
                &play.calls,
                &signature,
            );
            Json(anthropic_body(&model, content, usage)).into_response()
        },
    )
    .await;
    Some(trace(response, &resolution, attempt))
}

// ---------------------------------------------------------------------------
// Gemini
// ---------------------------------------------------------------------------

/// The conversation and tools of a Gemini generateContent request.
pub(super) fn gemini_conversation(
    req: &super::gemini_handlers::GeminiRequest,
) -> (Vec<ConversationMessage>, Vec<ToolSpec>) {
    let mut out = Vec::new();
    if !req.system.is_empty() {
        out.push(ConversationMessage::new(
            MessageRole::System,
            req.system.clone(),
        ));
    }
    for c in &req.contents {
        let parts = c["parts"].as_array().cloned().unwrap_or_default();
        let text = parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let call_id = |p: &serde_json::Value, key: &str| {
            p.get(key)
                .and_then(|f| f["id"].as_str().or(f["name"].as_str()))
                .map(str::to_string)
        };
        if c["role"].as_str() == Some("model") {
            let ids = parts
                .iter()
                .filter_map(|p| call_id(p, "functionCall"))
                .collect::<Vec<_>>();
            push_assistant(&mut out, text, ids);
            continue;
        }
        let mut results = false;
        for p in &parts {
            if let Some(id) = call_id(p, "functionResponse") {
                results = true;
                out.push(
                    ConversationMessage::new(MessageRole::Tool, String::new())
                        .with_tool_call_ids([id]),
                );
            }
        }
        if !results || !text.is_empty() {
            out.push(ConversationMessage::new(MessageRole::User, text));
        }
    }
    let tools = req
        .tools
        .iter()
        .map(|(name, schema)| ToolSpec {
            name: name.clone(),
            parameters: schema.clone(),
        })
        .collect();
    (out, tools)
}

fn gemini_call_parts(calls: &[SimToolCall]) -> Vec<serde_json::Value> {
    calls
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let id =
                c.id.clone()
                    .unwrap_or_else(|| crate::script::auto_tool_call_id(0, i));
            serde_json::json!({"functionCall": {"id": id, "name": c.name, "args": c.arguments}})
        })
        .collect()
}

/// Gemini stream chunks for a scenario step, as SSE or one JSON array.
struct GeminiFrames {
    model: String,
    id: String,
    sse: bool,
    started: bool,
    include_thoughts: bool,
    calls: Vec<SimToolCall>,
    usage: serde_json::Value,
}

impl GeminiFrames {
    fn frame(&mut self, chunk: serde_json::Value, last: bool) -> String {
        if self.sse {
            return format!("data: {chunk}\r\n\r\n");
        }
        let open = if self.started { ",\r\n" } else { "[" };
        self.started = true;
        format!("{open}{chunk}{}", if last { "]" } else { "" })
    }
}

impl StepFrames for GeminiFrames {
    fn open(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn token(&mut self, reasoning: bool, token: &str) -> Vec<String> {
        if reasoning && !self.include_thoughts {
            return Vec::new();
        }
        let mut part = serde_json::json!({"text": token});
        if reasoning {
            part["thought"] = serde_json::json!(true);
        }
        let c = super::gemini_handlers::chunk(&self.model, &self.id, vec![part], None, None);
        vec![self.frame(c, false)]
    }

    fn close(&mut self) -> Vec<String> {
        let parts = gemini_call_parts(&self.calls);
        let c = super::gemini_handlers::chunk(
            &self.model,
            &self.id,
            parts,
            Some("STOP"),
            Some(self.usage.clone()),
        );
        vec![self.frame(c, true)]
    }
}

/// Play a scenario on Gemini generateContent, if the request selects one.
/// `stream` is `None` for generateContent, `Some(sse)` for streaming.
pub(super) async fn gemini(
    state: &Arc<AppState>,
    model: &str,
    req: &super::gemini_handlers::GeminiRequest,
    stream: Option<bool>,
    request_start: Instant,
    latency: LatencyProfile,
) -> Option<Response> {
    use super::decisions_handlers::gemini_error;
    let (messages, tools) = gemini_conversation(req);
    let resolution = match resolve(state, &messages, &tools, model) {
        Ok(Some(r)) => r,
        Ok(None) => return None,
        Err(e) => return Some(gemini_error(e.status_code(), e.message())),
    };
    let attempt = state.attempts.next_attempt(resolution.fingerprint);
    let play = match prepare(
        &resolution,
        attempt,
        latency.with_input_tokens(req.input_tokens),
    ) {
        Outcome::Fail(err) => {
            state.stats.record_error(err.error.status_code());
            let r = with_retry(
                gemini_error(err.error.status_code(), err.error.message()),
                &err,
            );
            return Some(trace(r, &resolution, attempt));
        }
        Outcome::Play(p) => p,
    };
    let usage = super::gemini_handlers::usage_metadata(
        req.input_tokens,
        play.text_tokens + play.tool_tokens,
        play.reasoning_tokens,
        0,
    );
    let id = crate::ids::prefixed_compact_id("")
        .chars()
        .take(22)
        .collect::<String>();
    let frames = GeminiFrames {
        model: model.to_string(),
        id: id.clone(),
        sse: stream.unwrap_or(true),
        started: false,
        include_thoughts: req.include_thoughts,
        calls: play.calls.clone(),
        usage: usage.clone(),
    };
    let model = model.to_string();
    let include_thoughts = req.include_thoughts;
    let response = deliver(
        state,
        stream.is_some(),
        play,
        frames,
        req.input_tokens as u32,
        request_start,
        move |play| {
            let mut parts = Vec::new();
            if let (Some(r), true) = (&play.reasoning, include_thoughts) {
                parts.push(serde_json::json!({"text": r, "thought": true}));
            }
            if let Some(t) = play.text.as_deref().filter(|t| !t.is_empty()) {
                parts.push(serde_json::json!({"text": t}));
            }
            parts.extend(gemini_call_parts(&play.calls));
            Json(super::gemini_handlers::chunk(
                &model,
                &id,
                parts,
                Some("STOP"),
                Some(usage),
            ))
            .into_response()
        },
    )
    .await;
    // Array streams are JSON, not SSE.
    let mut response = response;
    if stream == Some(false) && response.status() == StatusCode::OK {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    }
    Some(trace(response, &resolution, attempt))
}
