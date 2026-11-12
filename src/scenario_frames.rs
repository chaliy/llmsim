// Scenario steps in the Responses and Anthropic Messages wire formats.
//
// `scenario_stream::paced_frames` paces a step; the `StepFrames` here render
// it. Non-streaming bodies are built from the same pieces so both modes agree.
//
// Decisions:
// - Reasoning is a Responses `reasoning` item with a summary, and an
//   Anthropic `thinking` block with the text, both ahead of the answer.
// - Tool calls keep the scenario's `call_llmsim_...` ids on both APIs (as
//   `call_id` and as the `tool_use` id), because resolution reads them back
//   from history to count steps.
// - Tool arguments go out in one delta each: scenarios pace text, not
//   arguments, same as on chat completions.

use crate::anthropic::{format_event, ContentBlock, MessagesResponse, Role, StopReason, Usage};
use crate::ids::{prefixed_id, unix_timestamp};
use crate::openai::{
    ItemStatus, OutputContentPart, OutputItem, OutputRole, ReasoningSummary, ResponseStatus,
    ResponsesResponse, ResponsesStreamEvent, ResponsesUsage,
};
use crate::scenario_stream::StepFrames;
use crate::script::SimToolCall;
use serde_json::json;

fn call_id(call: &SimToolCall, i: usize) -> String {
    call.id
        .clone()
        .unwrap_or_else(|| crate::script::auto_tool_call_id(0, i))
}

fn arguments(call: &SimToolCall) -> String {
    serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string())
}

/// Opaque thinking signature for a scenario step.
pub fn thinking_signature(seed: &str) -> String {
    format!(
        "EqQB{:016x}",
        crate::schema_fake::seed_from(seed.as_bytes())
    )
}

// ---------------------------------------------------------------------------
// Responses API
// ---------------------------------------------------------------------------

fn reasoning_item(id: &str, text: &str, status: ItemStatus) -> OutputItem {
    OutputItem::Reasoning {
        id: id.to_string(),
        status,
        summary: Some(vec![ReasoningSummary {
            summary_type: "summary_text".to_string(),
            text: text.to_string(),
        }]),
    }
}

fn message_item(id: &str, text: &str, status: ItemStatus) -> OutputItem {
    OutputItem::Message {
        id: id.to_string(),
        role: OutputRole::Assistant,
        status,
        content: vec![OutputContentPart::OutputText {
            text: text.to_string(),
            annotations: vec![],
        }],
    }
}

fn function_item(id: &str, call: &SimToolCall, i: usize, status: ItemStatus) -> OutputItem {
    OutputItem::FunctionCall {
        id: id.to_string(),
        call_id: call_id(call, i),
        name: call.name.clone(),
        arguments: arguments(call),
        status,
    }
}

/// The complete output items of a step.
pub fn responses_output(
    reasoning: Option<&str>,
    text: Option<&str>,
    calls: &[SimToolCall],
) -> Vec<OutputItem> {
    let mut out = Vec::new();
    if let Some(r) = reasoning {
        out.push(reasoning_item(
            &prefixed_id("rs_"),
            r,
            ItemStatus::Completed,
        ));
    }
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        out.push(message_item(&prefixed_id("msg_"), t, ItemStatus::Completed));
    }
    for (i, c) in calls.iter().enumerate() {
        out.push(function_item(
            &prefixed_id("fc_"),
            c,
            i,
            ItemStatus::Completed,
        ));
    }
    out
}

/// A finished Responses API body.
pub fn responses_body(
    id: String,
    model: &str,
    created_at: i64,
    output: Vec<OutputItem>,
    usage: ResponsesUsage,
) -> ResponsesResponse {
    let output_text = output
        .iter()
        .filter_map(|o| match o {
            OutputItem::Message { content, .. } => content.iter().find_map(|p| match p {
                OutputContentPart::OutputText { text, .. } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .reduce(|a, b| a + &b);
    ResponsesResponse {
        id,
        object: "response".to_string(),
        created_at,
        model: model.to_string(),
        status: ResponseStatus::Completed,
        output,
        output_text,
        usage: Some(usage),
        error: None,
        metadata: None,
    }
}

#[derive(PartialEq)]
enum Phase {
    None,
    Reasoning,
    Text,
}

/// Responses API streaming events for a scenario step.
pub struct ResponsesFrames {
    id: String,
    model: String,
    created_at: i64,
    seq: u32,
    phase: Phase,
    item_id: String,
    buf: String,
    output: Vec<OutputItem>,
    calls: Vec<SimToolCall>,
    usage: ResponsesUsage,
}

impl ResponsesFrames {
    pub fn new(model: &str, calls: Vec<SimToolCall>, usage: ResponsesUsage) -> Self {
        Self {
            id: prefixed_id("resp_"),
            model: model.to_string(),
            created_at: unix_timestamp(),
            seq: 0,
            phase: Phase::None,
            item_id: String::new(),
            buf: String::new(),
            output: Vec::new(),
            calls,
            usage,
        }
    }

    fn next(&mut self) -> u32 {
        let s = self.seq;
        self.seq += 1;
        s
    }

    fn shell(&self, status: ResponseStatus) -> ResponsesResponse {
        let mut r = responses_body(
            self.id.clone(),
            &self.model,
            self.created_at,
            Vec::new(),
            self.usage.clone(),
        );
        r.status = status;
        r.usage = None;
        r
    }

    fn index(&self) -> u32 {
        self.output.len() as u32
    }

    /// Close the open reasoning or message item.
    fn end_phase(&mut self, out: &mut Vec<String>) {
        let idx = self.index();
        let text = std::mem::take(&mut self.buf);
        let id = self.item_id.clone();
        match self.phase {
            Phase::None => return,
            Phase::Reasoning => {
                let part = ReasoningSummary {
                    summary_type: "summary_text".to_string(),
                    text: text.clone(),
                };
                let seq = self.next();
                out.push(ResponsesStreamEvent::reasoning_summary_text_done(
                    idx, 0, &id, &text, seq,
                ));
                let seq = self.next();
                out.push(ResponsesStreamEvent::reasoning_summary_part_done(
                    idx, 0, &id, &part, seq,
                ));
                let item = reasoning_item(&id, &text, ItemStatus::Completed);
                let seq = self.next();
                out.push(ResponsesStreamEvent::output_item_done(idx, &item, seq));
                self.output.push(item);
            }
            Phase::Text => {
                let part = OutputContentPart::OutputText {
                    text: text.clone(),
                    annotations: vec![],
                };
                let seq = self.next();
                out.push(ResponsesStreamEvent::output_text_done(
                    idx, 0, &id, &text, seq,
                ));
                let seq = self.next();
                out.push(ResponsesStreamEvent::content_part_done(
                    idx, 0, &id, &part, seq,
                ));
                let item = message_item(&id, &text, ItemStatus::Completed);
                let seq = self.next();
                out.push(ResponsesStreamEvent::output_item_done(idx, &item, seq));
                self.output.push(item);
            }
        }
        self.phase = Phase::None;
    }
}

impl StepFrames for ResponsesFrames {
    fn open(&mut self) -> Vec<String> {
        let created = self.shell(ResponseStatus::InProgress);
        let a = self.next();
        let b = self.next();
        vec![
            ResponsesStreamEvent::response_created(created.clone(), a),
            ResponsesStreamEvent::response_in_progress(created, b),
        ]
    }

    fn token(&mut self, reasoning: bool, token: &str) -> Vec<String> {
        let mut out = Vec::new();
        let want = if reasoning {
            Phase::Reasoning
        } else {
            Phase::Text
        };
        if self.phase != want {
            self.end_phase(&mut out);
            let idx = self.index();
            if reasoning {
                self.item_id = prefixed_id("rs_");
                let item = OutputItem::Reasoning {
                    id: self.item_id.clone(),
                    status: ItemStatus::InProgress,
                    summary: None,
                };
                let seq = self.next();
                out.push(ResponsesStreamEvent::output_item_added(idx, &item, seq));
                let part = ReasoningSummary {
                    summary_type: "summary_text".to_string(),
                    text: String::new(),
                };
                let seq = self.next();
                out.push(ResponsesStreamEvent::reasoning_summary_part_added(
                    idx,
                    0,
                    &self.item_id,
                    &part,
                    seq,
                ));
            } else {
                self.item_id = prefixed_id("msg_");
                let item = OutputItem::Message {
                    id: self.item_id.clone(),
                    role: OutputRole::Assistant,
                    status: ItemStatus::InProgress,
                    content: vec![],
                };
                let seq = self.next();
                out.push(ResponsesStreamEvent::output_item_added(idx, &item, seq));
                let part = OutputContentPart::OutputText {
                    text: String::new(),
                    annotations: vec![],
                };
                let seq = self.next();
                out.push(ResponsesStreamEvent::content_part_added(
                    idx,
                    0,
                    &self.item_id,
                    &part,
                    seq,
                ));
            }
            self.phase = want;
        }
        let idx = self.index();
        let seq = self.next();
        out.push(if reasoning {
            ResponsesStreamEvent::reasoning_summary_text_delta(idx, 0, &self.item_id, token, seq)
        } else {
            ResponsesStreamEvent::output_text_delta(idx, 0, &self.item_id, token, seq)
        });
        self.buf.push_str(token);
        out
    }

    fn close(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.end_phase(&mut out);
        for (i, call) in std::mem::take(&mut self.calls).iter().enumerate() {
            let idx = self.index();
            let id = prefixed_id("fc_");
            let mut added = function_item(&id, call, i, ItemStatus::InProgress);
            if let OutputItem::FunctionCall { arguments, .. } = &mut added {
                arguments.clear();
            }
            let seq = self.next();
            out.push(ResponsesStreamEvent::output_item_added(idx, &added, seq));
            let args = arguments(call);
            for (kind, key) in [
                ("response.function_call_arguments.delta", "delta"),
                ("response.function_call_arguments.done", "arguments"),
            ] {
                let event = json!({
                    "type": kind,
                    "output_index": idx,
                    "item_id": id,
                    key: args,
                    "sequence_number": self.next(),
                });
                out.push(format!("event: {kind}\ndata: {event}\n\n"));
            }
            let item = function_item(&id, call, i, ItemStatus::Completed);
            let seq = self.next();
            out.push(ResponsesStreamEvent::output_item_done(idx, &item, seq));
            self.output.push(item);
        }
        let done = responses_body(
            self.id.clone(),
            &self.model,
            self.created_at,
            std::mem::take(&mut self.output),
            self.usage.clone(),
        );
        let seq = self.next();
        out.push(ResponsesStreamEvent::response_completed(done, seq));
        out
    }
}

// ---------------------------------------------------------------------------
// Anthropic Messages
// ---------------------------------------------------------------------------

/// The content blocks of a step.
pub fn anthropic_content(
    reasoning: Option<&str>,
    text: Option<&str>,
    calls: &[SimToolCall],
    signature: &str,
) -> Vec<ContentBlock> {
    let mut out = Vec::new();
    if let Some(r) = reasoning {
        out.push(ContentBlock::Thinking {
            thinking: r.to_string(),
            signature: signature.to_string(),
        });
    }
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        out.push(ContentBlock::text(t));
    }
    for (i, c) in calls.iter().enumerate() {
        out.push(ContentBlock::ToolUse {
            id: call_id(c, i),
            name: c.name.clone(),
            input: c.arguments.clone(),
        });
    }
    out
}

/// A finished Messages API body.
pub fn anthropic_body(model: &str, content: Vec<ContentBlock>, usage: Usage) -> MessagesResponse {
    let tool = content
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    MessagesResponse {
        id: prefixed_id("msg_"),
        response_type: "message".to_string(),
        role: Role::Assistant,
        model: model.to_string(),
        content,
        stop_reason: Some(if tool {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        }),
        stop_sequence: None,
        usage,
    }
}

/// Anthropic streaming events for a scenario step.
pub struct AnthropicFrames {
    model: String,
    phase: Phase,
    index: usize,
    calls: Vec<SimToolCall>,
    usage: Usage,
    signature: String,
}

impl AnthropicFrames {
    pub fn new(model: &str, calls: Vec<SimToolCall>, usage: Usage, signature: String) -> Self {
        Self {
            model: model.to_string(),
            phase: Phase::None,
            index: 0,
            calls,
            usage,
            signature,
        }
    }

    fn end_phase(&mut self, out: &mut Vec<String>) {
        if self.phase == Phase::None {
            return;
        }
        if self.phase == Phase::Reasoning {
            out.push(format_event(
                "content_block_delta",
                &json!({"type": "content_block_delta", "index": self.index,
                        "delta": {"type": "signature_delta", "signature": self.signature}}),
            ));
        }
        out.push(stop(self.index));
        self.index += 1;
        self.phase = Phase::None;
    }
}

fn stop(index: usize) -> String {
    format_event(
        "content_block_stop",
        &json!({"type": "content_block_stop", "index": index}),
    )
}

impl StepFrames for AnthropicFrames {
    fn open(&mut self) -> Vec<String> {
        let mut usage = json!({"input_tokens": self.usage.input_tokens, "output_tokens": 1});
        if let Some(read) = self.usage.cache_read_input_tokens {
            usage["cache_read_input_tokens"] = json!(read);
        }
        if let Some(created) = self.usage.cache_creation_input_tokens {
            usage["cache_creation_input_tokens"] = json!(created);
        }
        vec![format_event(
            "message_start",
            &json!({"type": "message_start", "message": {
                "id": prefixed_id("msg_"), "type": "message", "role": "assistant",
                "model": self.model, "content": [], "stop_reason": null,
                "stop_sequence": null, "usage": usage
            }}),
        )]
    }

    fn token(&mut self, reasoning: bool, token: &str) -> Vec<String> {
        let mut out = Vec::new();
        let want = if reasoning {
            Phase::Reasoning
        } else {
            Phase::Text
        };
        if self.phase != want {
            self.end_phase(&mut out);
            let block = if reasoning {
                json!({"type": "thinking", "thinking": "", "signature": ""})
            } else {
                json!({"type": "text", "text": ""})
            };
            out.push(format_event(
                "content_block_start",
                &json!({"type": "content_block_start", "index": self.index, "content_block": block}),
            ));
            self.phase = want;
        }
        let delta = if reasoning {
            json!({"type": "thinking_delta", "thinking": token})
        } else {
            json!({"type": "text_delta", "text": token})
        };
        out.push(format_event(
            "content_block_delta",
            &json!({"type": "content_block_delta", "index": self.index, "delta": delta}),
        ));
        out
    }

    fn close(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.end_phase(&mut out);
        let calls = std::mem::take(&mut self.calls);
        for (i, call) in calls.iter().enumerate() {
            out.push(format_event(
                "content_block_start",
                &json!({"type": "content_block_start", "index": self.index, "content_block": {
                    "type": "tool_use", "id": call_id(call, i), "name": call.name, "input": {}
                }}),
            ));
            out.push(format_event(
                "content_block_delta",
                &json!({"type": "content_block_delta", "index": self.index, "delta": {
                    "type": "input_json_delta", "partial_json": arguments(call)
                }}),
            ));
            out.push(stop(self.index));
            self.index += 1;
        }
        let reason = if calls.is_empty() {
            "end_turn"
        } else {
            "tool_use"
        };
        out.push(format_event(
            "message_delta",
            &json!({"type": "message_delta",
                    "delta": {"stop_reason": reason, "stop_sequence": null},
                    "usage": {"output_tokens": self.usage.output_tokens}}),
        ));
        out.push(format_event(
            "message_stop",
            &json!({"type": "message_stop"}),
        ));
        out
    }
}
