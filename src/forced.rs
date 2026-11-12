// What a request forces the model to produce: a tool call (tool_choice) or
// JSON matching a schema (structured outputs). Shared by the OpenAI and
// Anthropic handlers when no script or scenario is playing.
//
// Decisions:
// - `tool_choice` that requires a tool (`required`/`any`, or a named tool)
//   always produces one call with schema-valid arguments, the way a real
//   model must. `auto` produces a call only when `[response]
//   auto_tool_calls` is on and the last message is the user's (not a tool
//   result), so an agent loop calls a tool once and then gets an answer.
// - The tool and the arguments are picked deterministically from the last
//   message, so reruns are reproducible.
// - Structured output wins over plain text but loses to a forced tool call,
//   matching provider precedence.

use crate::openai::{ChatCompletionRequest, Role, ToolChoice};
use crate::schema_fake::{fake_from_schema, seed_from};
use crate::script::SimToolCall;
use serde_json::{json, Value};

/// A forced output.
#[derive(Debug, Clone, PartialEq)]
pub enum Forced {
    /// Call these tools (no text).
    ToolCalls(Vec<SimToolCall>),
    /// Answer with this JSON text.
    Json(String),
}

/// A tool the request offers: name and parameter schema.
struct Offered<'a> {
    name: &'a str,
    schema: Option<&'a Value>,
}

/// How the request constrains tool use.
enum Choice<'a> {
    None,
    Auto,
    Any,
    Named(&'a str),
}

fn call_for(
    tools: &[Offered<'_>],
    choice: Choice<'_>,
    may_auto: bool,
    seed: u64,
) -> Option<SimToolCall> {
    if tools.is_empty() {
        return None;
    }
    let tool = match choice {
        Choice::None => return None,
        Choice::Named(name) => tools.iter().find(|t| t.name == name)?,
        Choice::Any => &tools[(seed % tools.len() as u64) as usize],
        Choice::Auto if may_auto => &tools[(seed % tools.len() as u64) as usize],
        Choice::Auto => return None,
    };
    let arguments = tool
        .schema
        .map(|s| fake_from_schema(s, seed))
        .unwrap_or_else(|| json!({}));
    Some(SimToolCall {
        name: tool.name.to_string(),
        arguments,
        id: Some(format!("call_{:016x}", seed_from(&seed.to_le_bytes()))),
    })
}

/// JSON text for a structured-output schema.
pub fn json_for_schema(schema: &Value, seed: u64) -> String {
    serde_json::to_string(&fake_from_schema(schema, seed)).unwrap_or_else(|_| "{}".into())
}

/// A forced function call for a Gemini request. `mode` is
/// `toolConfig.functionCallingConfig.mode` (`AUTO`, `ANY`, `NONE`,
/// `VALIDATED`); `allowed` narrows the tools under `ANY`.
pub fn gemini(
    tools: &[(String, Option<Value>)],
    mode: &str,
    allowed: &[String],
    may_auto: bool,
    seed: u64,
) -> Option<SimToolCall> {
    let offered: Vec<Offered<'_>> = tools
        .iter()
        .filter(|(name, _)| mode != "ANY" || allowed.is_empty() || allowed.contains(name))
        .map(|(name, schema)| Offered {
            name,
            schema: schema.as_ref(),
        })
        .collect();
    let choice = match mode {
        "NONE" => Choice::None,
        "ANY" => Choice::Any,
        _ => Choice::Auto,
    };
    call_for(&offered, choice, may_auto, seed)
}

/// Forced output for a chat completions request.
pub fn openai_chat(request: &ChatCompletionRequest, auto_tool_calls: bool) -> Option<Forced> {
    let last = request.messages.last();
    let seed_text = last
        .and_then(|m| m.content.as_ref())
        .map(|c| c.text())
        .unwrap_or_default();
    let seed = seed_from(seed_text.as_bytes()) ^ request.messages.len() as u64;
    let tools: Vec<Offered<'_>> = request
        .tools
        .iter()
        .flatten()
        .map(|t| Offered {
            name: &t.function.name,
            schema: t.function.parameters.as_ref(),
        })
        .collect();
    let choice = match &request.tool_choice {
        None => Choice::Auto,
        Some(ToolChoice::String(s)) => match s.as_str() {
            "none" => Choice::None,
            "required" => Choice::Any,
            _ => Choice::Auto,
        },
        Some(ToolChoice::Object { function, .. }) => Choice::Named(&function.name),
    };
    let user_last = last.is_some_and(|m| m.role == Role::User);
    if let Some(call) = call_for(&tools, choice, auto_tool_calls && user_last, seed) {
        return Some(Forced::ToolCalls(vec![call]));
    }
    let format = request.response_format.as_ref()?;
    match format.format_type.as_str() {
        "json_schema" => {
            let schema = format
                .json_schema
                .as_ref()
                .and_then(|j| j.get("schema"))
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"}));
            Some(Forced::Json(json_for_schema(&schema, seed)))
        }
        "json_object" => Some(Forced::Json(json_for_schema(
            &json!({"type": "object", "properties": {"result": {"type": "string"}}}),
            seed,
        ))),
        _ => None,
    }
}

/// Forced output for an Anthropic Messages request.
pub fn anthropic(
    request: &crate::anthropic::MessagesRequest,
    auto_tool_calls: bool,
) -> Option<Forced> {
    let last = request.messages.last();
    let last_json = last
        .map(|m| serde_json::to_string(&m.content).unwrap_or_default())
        .unwrap_or_default();
    let seed = seed_from(last_json.as_bytes()) ^ request.messages.len() as u64;
    let tools: Vec<Offered<'_>> = request
        .tools
        .iter()
        .flatten()
        .map(|t| Offered {
            name: &t.name,
            schema: t.input_schema.as_ref(),
        })
        .collect();
    let choice = match request
        .tool_choice
        .as_ref()
        .and_then(|c| c.get("type"))
        .and_then(Value::as_str)
    {
        Some("none") => Choice::None,
        Some("any") => Choice::Any,
        Some("tool") => match request
            .tool_choice
            .as_ref()
            .and_then(|c| c["name"].as_str())
        {
            Some(name) => Choice::Named(name),
            None => Choice::Auto,
        },
        _ => Choice::Auto,
    };
    let user_last = last.is_some_and(|m| {
        matches!(m.role, crate::anthropic::Role::User) && !last_json.contains("\"tool_result\"")
    });
    if let Some(call) = call_for(&tools, choice, auto_tool_calls && user_last, seed) {
        return Some(Forced::ToolCalls(vec![call]));
    }
    let format = request.output_config.as_ref()?.format.as_ref()?;
    if format.get("type").and_then(Value::as_str) == Some("json_schema") {
        let schema = format
            .get("schema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"}));
        return Some(Forced::Json(json_for_schema(&schema, seed)));
    }
    None
}
