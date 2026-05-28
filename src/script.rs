// Scripted response mode.
//
// Drives a deterministic, multi-turn sequence of responses for agent
// scenario tests. The script is loaded once at startup and shared
// across requests via an atomic cursor; downstream callers point their
// agent at llmsim and assert on the sequence of HTTP responses.
//
// The Rust types here are also the wire format for the JSON script
// file: callers (e.g. a yolop scenario test) generate JSON, hand the
// path to llmsim via `[response] script_path = "..."`, and read back
// the scripted responses.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A single scripted assistant turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SimTurn {
    /// Plain assistant text response.
    Assistant { text: String },

    /// One or more tool calls in a single assistant turn.
    /// The assistant message has no text content; finish_reason is
    /// `tool_calls`.
    ToolCalls { calls: Vec<SimToolCall> },

    /// Mixed: assistant text + tool calls in the same turn.
    /// finish_reason is `tool_calls`.
    Mixed {
        text: String,
        calls: Vec<SimToolCall>,
    },

    /// Simulate an API/transport error on this turn. Returns the
    /// appropriate HTTP status; the cursor still advances so the next
    /// request gets the next turn (caller can retry against llmsim and
    /// see different behaviour).
    Error(SimError),
}

/// A single tool call inside a scripted turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimToolCall {
    pub name: String,
    /// Arguments object. Sent to the wire as a JSON-encoded string so
    /// it matches OpenAI's `function.arguments` shape.
    pub arguments: serde_json::Value,
    /// Optional explicit id. Auto-generated if `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Error to inject for a scripted turn.
///
/// Modeled after `SimulatedError` but smaller — the script only needs
/// to cover the failure modes agents commonly need to test against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SimError {
    /// HTTP 429.
    RateLimit,
    /// HTTP 504, returned immediately (no actual sleep — tests don't
    /// want to wait).
    Timeout,
    /// HTTP 400 with the provided message.
    InvalidRequest { message: String },
    /// Catch-all. Defaults to HTTP 500; override with `status_code`.
    Other {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status_code: Option<u16>,
    },
}

impl SimError {
    pub fn status_code(&self) -> u16 {
        match self {
            SimError::RateLimit => 429,
            SimError::Timeout => 504,
            SimError::InvalidRequest { .. } => 400,
            SimError::Other { status_code, .. } => status_code.unwrap_or(500),
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            SimError::RateLimit => "rate_limit_error",
            SimError::Timeout => "timeout_error",
            SimError::InvalidRequest { .. } => "invalid_request_error",
            SimError::Other { .. } => "server_error",
        }
    }

    pub fn message(&self) -> String {
        match self {
            SimError::RateLimit => "Rate limit exceeded. Please retry after some time.".to_string(),
            SimError::Timeout => "Request timed out".to_string(),
            SimError::InvalidRequest { message } => message.clone(),
            SimError::Other { message, .. } => message.clone(),
        }
    }
}

/// Behaviour when the script's cursor has run past the last turn.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnExhausted {
    /// Repeat the last turn forever. Matches the existing `fixed`
    /// generator behaviour and is the default.
    #[default]
    RepeatLast,
    /// Return an HTTP 500 error indicating the script is exhausted.
    /// Useful for asserting the agent stopped on its own.
    Error,
    /// Cycle back to the start of the script.
    Loop,
}

/// On-disk / over-the-wire script representation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScriptSpec {
    pub turns: Vec<SimTurn>,
    #[serde(default)]
    pub on_exhausted: OnExhausted,
}

/// Errors that can occur while loading a script from disk.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("Failed to read script file: {0}")]
    Io(String),
    #[error("Failed to parse script JSON: {0}")]
    Parse(String),
    #[error("Script must contain at least one turn")]
    Empty,
}

/// Live, thread-safe scripted response source.
///
/// The cursor advances on every `next_turn()` call; concurrent requests
/// are serialised via an atomic counter so a 2-turn script driven by a
/// 2-request agent yields turn[0] then turn[1] regardless of which
/// task lands first.
#[derive(Debug)]
pub struct Script {
    turns: Vec<SimTurn>,
    on_exhausted: OnExhausted,
    cursor: AtomicUsize,
}

/// A turn yielded by `Script::next_turn`. Includes whether the script
/// has been exhausted (so callers can pick error semantics).
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptedResponse {
    Turn(SimTurn),
    /// Script exhausted and `on_exhausted = Error`. Callers should map
    /// this to a 500 with a descriptive message.
    Exhausted,
}

impl Script {
    pub fn new(turns: Vec<SimTurn>) -> Self {
        Self {
            turns,
            on_exhausted: OnExhausted::RepeatLast,
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn from_spec(spec: ScriptSpec) -> Result<Self, ScriptError> {
        if spec.turns.is_empty() {
            return Err(ScriptError::Empty);
        }
        Ok(Self {
            turns: spec.turns,
            on_exhausted: spec.on_exhausted,
            cursor: AtomicUsize::new(0),
        })
    }

    pub fn with_on_exhausted(mut self, mode: OnExhausted) -> Self {
        self.on_exhausted = mode;
        self
    }

    pub fn from_json(json: &str) -> Result<Self, ScriptError> {
        let spec: ScriptSpec =
            serde_json::from_str(json).map_err(|e| ScriptError::Parse(e.to_string()))?;
        Self::from_spec(spec)
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ScriptError> {
        let content =
            std::fs::read_to_string(path.as_ref()).map_err(|e| ScriptError::Io(e.to_string()))?;
        Self::from_json(&content)
    }

    pub fn len(&self) -> usize {
        self.turns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    pub fn on_exhausted(&self) -> OnExhausted {
        self.on_exhausted
    }

    /// Atomically advance the cursor and return the next scripted
    /// response.
    pub fn next_turn(&self) -> ScriptedResponse {
        let n = self.turns.len();
        debug_assert!(n > 0, "Script must have at least one turn");

        let idx = self.cursor.fetch_add(1, Ordering::SeqCst);
        if idx < n {
            return ScriptedResponse::Turn(self.turns[idx].clone());
        }

        match self.on_exhausted {
            OnExhausted::RepeatLast => ScriptedResponse::Turn(self.turns[n - 1].clone()),
            OnExhausted::Loop => ScriptedResponse::Turn(self.turns[idx % n].clone()),
            OnExhausted::Error => ScriptedResponse::Exhausted,
        }
    }

    /// Number of turns consumed so far (for tests / debugging).
    pub fn cursor(&self) -> usize {
        self.cursor.load(Ordering::SeqCst)
    }
}

/// Generate a stable tool-call id when the script doesn't provide one.
/// Format mirrors OpenAI's `call_<random>` shape and includes the turn
/// index so calls are unique across the whole script.
pub fn auto_tool_call_id(turn_index: usize, call_index: usize) -> String {
    format!("call_llmsim_{}_{}", turn_index, call_index)
}

/// Ensure every tool call in a turn has an id. Used by handlers so
/// downstream callers can correlate function_call_output items back.
pub fn resolve_tool_call_ids(turn_index: usize, calls: &mut [SimToolCall]) {
    for (i, call) in calls.iter_mut().enumerate() {
        if call.id.is_none() {
            call.id = Some(auto_tool_call_id(turn_index, i));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_simple_assistant_script() {
        let spec = r#"{
            "turns": [
                {"type": "assistant", "text": "hello"}
            ]
        }"#;
        let script = Script::from_json(spec).unwrap();
        assert_eq!(script.len(), 1);
        assert_eq!(script.on_exhausted(), OnExhausted::RepeatLast);
    }

    #[test]
    fn parses_tool_call_script() {
        let spec = r#"{
            "on_exhausted": "error",
            "turns": [
                {"type": "tool_calls", "calls": [
                    {"name": "bash", "arguments": {"command": "ls"}}
                ]},
                {"type": "assistant", "text": "done"}
            ]
        }"#;
        let script = Script::from_json(spec).unwrap();
        assert_eq!(script.len(), 2);
        assert_eq!(script.on_exhausted(), OnExhausted::Error);
    }

    #[test]
    fn parses_mixed_and_error_turns() {
        let spec = r#"{
            "turns": [
                {"type": "mixed", "text": "thinking", "calls": [
                    {"name": "x", "arguments": {}, "id": "call_a"}
                ]},
                {"type": "error", "kind": "rate_limit"},
                {"type": "error", "kind": "other", "message": "boom", "status_code": 502}
            ]
        }"#;
        let script = Script::from_json(spec).unwrap();
        assert_eq!(script.len(), 3);
    }

    #[test]
    fn empty_script_rejected() {
        let spec = r#"{"turns": []}"#;
        assert!(matches!(Script::from_json(spec), Err(ScriptError::Empty)));
    }

    #[test]
    fn cursor_advances_in_order() {
        let script = Script::new(vec![
            SimTurn::Assistant { text: "one".into() },
            SimTurn::Assistant { text: "two".into() },
        ]);
        assert_eq!(
            script.next_turn(),
            ScriptedResponse::Turn(SimTurn::Assistant { text: "one".into() })
        );
        assert_eq!(
            script.next_turn(),
            ScriptedResponse::Turn(SimTurn::Assistant { text: "two".into() })
        );
    }

    #[test]
    fn on_exhausted_repeat_last() {
        let script = Script::new(vec![
            SimTurn::Assistant { text: "one".into() },
            SimTurn::Assistant { text: "two".into() },
        ]);
        script.next_turn();
        script.next_turn();
        // Exhausted: repeat the last turn.
        assert_eq!(
            script.next_turn(),
            ScriptedResponse::Turn(SimTurn::Assistant { text: "two".into() })
        );
        assert_eq!(
            script.next_turn(),
            ScriptedResponse::Turn(SimTurn::Assistant { text: "two".into() })
        );
    }

    #[test]
    fn on_exhausted_error() {
        let script = Script::new(vec![SimTurn::Assistant {
            text: "only".into(),
        }])
        .with_on_exhausted(OnExhausted::Error);
        script.next_turn();
        assert_eq!(script.next_turn(), ScriptedResponse::Exhausted);
    }

    #[test]
    fn on_exhausted_loop() {
        let script = Script::new(vec![
            SimTurn::Assistant { text: "a".into() },
            SimTurn::Assistant { text: "b".into() },
        ])
        .with_on_exhausted(OnExhausted::Loop);
        script.next_turn(); // a
        script.next_turn(); // b
        assert_eq!(
            script.next_turn(),
            ScriptedResponse::Turn(SimTurn::Assistant { text: "a".into() })
        );
        assert_eq!(
            script.next_turn(),
            ScriptedResponse::Turn(SimTurn::Assistant { text: "b".into() })
        );
    }

    #[test]
    fn cursor_is_thread_safe() {
        use std::sync::Arc;
        use std::thread;

        let script = Arc::new(
            Script::new(
                (0..100)
                    .map(|i| SimTurn::Assistant {
                        text: format!("t{}", i),
                    })
                    .collect(),
            )
            .with_on_exhausted(OnExhausted::Error),
        );

        let mut handles = vec![];
        for _ in 0..10 {
            let s = script.clone();
            handles.push(thread::spawn(move || {
                let mut taken = 0;
                for _ in 0..10 {
                    if let ScriptedResponse::Turn(_) = s.next_turn() {
                        taken += 1;
                    }
                }
                taken
            }));
        }

        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total, 100, "exactly 100 turns should be consumed in total");
    }

    #[test]
    fn resolves_tool_call_ids() {
        let mut calls = vec![
            SimToolCall {
                name: "a".into(),
                arguments: json!({}),
                id: None,
            },
            SimToolCall {
                name: "b".into(),
                arguments: json!({}),
                id: Some("provided".into()),
            },
        ];
        resolve_tool_call_ids(3, &mut calls);
        assert_eq!(calls[0].id.as_deref(), Some("call_llmsim_3_0"));
        assert_eq!(calls[1].id.as_deref(), Some("provided"));
    }

    #[test]
    fn sim_error_codes() {
        assert_eq!(SimError::RateLimit.status_code(), 429);
        assert_eq!(SimError::Timeout.status_code(), 504);
        assert_eq!(
            SimError::InvalidRequest {
                message: "bad".into()
            }
            .status_code(),
            400
        );
        assert_eq!(
            SimError::Other {
                message: "boom".into(),
                status_code: None
            }
            .status_code(),
            500
        );
        assert_eq!(
            SimError::Other {
                message: "boom".into(),
                status_code: Some(502)
            }
            .status_code(),
            502
        );
    }
}
