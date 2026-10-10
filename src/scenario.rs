// Named scenarios for load testing.
//
// A scenario is a named, ordered list of steps; each step is one model call
// (what the model answers, how fast it streams, whether it fails first). A
// marker in the user message (`[[llmsim:<name> seed=7 speed=0.5]]`) or a
// `llmsim-scenario-<name>` model id picks the scenario. See
// `knowledge/simulation/scenarios.md`.
//
// Decisions:
// - The step to play is derived from the request's messages alone: count the
//   assistant messages after the last user message, cross-checked against the
//   llmsim-issued tool call ids. Any worker, process or restart gives the same
//   answer, and thousands of sessions share a server with no coordination.
//   The legacy `Script` keeps its global cursor for single-agent tests.
// - Every random draw comes from a hand-rolled FNV-1a hash feeding SplitMix64,
//   so draws are stable across processes, platforms and crate upgrades
//   (`rand`'s StdRng and std's hashers make no such guarantee).
// - `fail_first` and `cut_after_tokens` need the attempt number, which the
//   messages cannot carry (a failed call adds no assistant message).
//   `AttemptTracker` keeps that count in process, bounded and with a TTL. A
//   retry that lands on another worker may see the error again; that only adds
//   an extra error, never a wrong answer.
// - Retry-After lives on `StepError`, which flattens `SimError`, instead of on
//   `SimError` itself, so the public `SimError` enum stays source compatible.

use crate::script::{OnExhausted, SimError, SimToolCall, SimTurn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Opening of a scenario marker inside a user message.
pub const MARKER_PREFIX: &str = "[[llmsim:";
/// Model id prefix that selects a scenario when no marker is present.
pub const MODEL_PREFIX: &str = "llmsim-scenario-";
/// Prefix of tool call ids issued by scenarios.
pub const CALL_ID_PREFIX: &str = "call_llmsim_";

/// A number, or an inclusive `[min, max]` range sampled per step.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ValueRange {
    Fixed(u64),
    Between([u64; 2]),
}

impl ValueRange {
    fn sample(&self, rng: &mut SplitMix64) -> u64 {
        match *self {
            ValueRange::Fixed(v) => v,
            ValueRange::Between([min, max]) => min + rng.below((max - min).saturating_add(1)),
        }
    }

    fn validate(&self, field: &str) -> Result<(), String> {
        match *self {
            ValueRange::Between([min, max]) if min > max => Err(format!(
                "{field}: range min {min} is greater than max {max}"
            )),
            _ => Ok(()),
        }
    }
}

/// Pause mid-stream after a number of tokens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stall {
    pub after_tokens: usize,
    pub ms: u64,
}

/// Streaming timing for a step. Unset fields fall back to the scenario
/// defaults, then to the server's latency profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Timing {
    /// Time to first token in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<ValueRange>,
    /// Streaming speed after the first token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_sec: Option<f64>,
    /// Pause mid-stream, to exercise idle timeouts and SSE keep-alives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall: Option<Stall>,
    /// Drop the stream after this many tokens without a finish event, on the
    /// first attempt that is not failed by `fail_first`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_after_tokens: Option<usize>,
}

impl Timing {
    /// Field-wise override: values set on `self` win over `base`.
    fn over(&self, base: &Timing) -> Timing {
        Timing {
            ttft_ms: self.ttft_ms.or(base.ttft_ms),
            tokens_per_sec: self.tokens_per_sec.or(base.tokens_per_sec),
            stall: self.stall.or(base.stall),
            cut_after_tokens: self.cut_after_tokens.or(base.cut_after_tokens),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if let Some(r) = &self.ttft_ms {
            r.validate("timing.ttft_ms")?;
        }
        if let Some(tps) = self.tokens_per_sec {
            if !(tps.is_finite() && tps > 0.0) {
                return Err(format!("timing.tokens_per_sec must be positive, got {tps}"));
            }
        }
        Ok(())
    }
}

/// An error a step returns, with an optional `Retry-After`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepError {
    #[serde(flatten)]
    pub error: SimError,
    /// Sent as `Retry-After` (seconds, rounded up) and `retry-after-ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl From<SimError> for StepError {
    fn from(error: SimError) -> Self {
        Self {
            error,
            retry_after_ms: None,
        }
    }
}

impl StepError {
    /// `Retry-After` header value in whole seconds (rounded up, at least 1).
    pub fn retry_after_secs(&self) -> Option<u64> {
        self.retry_after_ms.map(|ms| ms.div_ceil(1000).max(1))
    }
}

/// Placeholder that picks a tool the agent actually offers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolPlaceholder {
    /// A read-like tool (read, get, list, search, ...); any tool if none.
    AnyRead,
    /// A write-like tool (write, edit, create, ...); any tool if none.
    AnyWrite,
    /// Any offered tool.
    Any,
}

/// A tool call inside a step: an exact `name` or a `tool` placeholder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StepCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolPlaceholder>,
    /// Arguments object. When absent, llmsim fills the tool's required
    /// parameters with placeholder values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

/// What the model returns for a step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepTurn {
    Assistant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lorem_tokens: Option<ValueRange>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<ValueRange>,
    },
    ToolCalls {
        calls: Vec<StepCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<ValueRange>,
    },
    Mixed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lorem_tokens: Option<ValueRange>,
        calls: Vec<StepCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<ValueRange>,
    },
    Error(StepError),
}

/// One model call in a scenario.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub turn: StepTurn,
    #[serde(default)]
    pub timing: Timing,
    /// Errors returned on the first attempts of this step before it succeeds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fail_first: Vec<StepError>,
}

/// Scenario-wide settings each step can override.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default)]
    pub timing: Timing,
    /// Background error rate per error kind (`rate_limit`, `timeout`,
    /// `server_error`), drawn per attempt.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub error_rate: BTreeMap<String, f64>,
}

/// A named list of steps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub defaults: Defaults,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub on_exhausted: OnExhausted,
}

impl Scenario {
    /// Check the scenario is well formed.
    pub fn validate(&self) -> Result<(), ScenarioLoadError> {
        let invalid = |reason: String| ScenarioLoadError::Invalid {
            scenario: self.name.clone(),
            reason,
        };
        if !is_valid_name(&self.name) {
            return Err(invalid(
                "name must be non-empty and use only [a-z0-9-]".to_string(),
            ));
        }
        if self.steps.is_empty() {
            return Err(invalid("scenario must have at least one step".to_string()));
        }
        self.defaults.timing.validate().map_err(invalid)?;
        for (kind, rate) in &self.defaults.error_rate {
            error_for_kind(kind).ok_or_else(|| {
                invalid(format!(
                    "defaults.error_rate: unknown kind '{kind}' (expected rate_limit, timeout or server_error)"
                ))
            })?;
            if !(0.0..=1.0).contains(rate) {
                return Err(invalid(format!(
                    "defaults.error_rate.{kind} must be between 0 and 1, got {rate}"
                )));
            }
        }
        for (i, step) in self.steps.iter().enumerate() {
            step.validate()
                .map_err(|reason| invalid(format!("step {i}: {reason}")))?;
        }
        Ok(())
    }
}

impl Step {
    fn validate(&self) -> Result<(), String> {
        self.timing.validate()?;
        let check_text = |text: &Option<String>, lorem: &Option<ValueRange>| match (text, lorem) {
            (Some(_), Some(_)) => Err("text and lorem_tokens are mutually exclusive".to_string()),
            (None, None) => Err("one of text or lorem_tokens is required".to_string()),
            (_, Some(r)) => r.validate("turn.lorem_tokens"),
            _ => Ok(()),
        };
        let check_calls = |calls: &[StepCall]| -> Result<(), String> {
            if calls.is_empty() {
                return Err("calls must not be empty".to_string());
            }
            for (n, call) in calls.iter().enumerate() {
                match (&call.name, &call.tool) {
                    (Some(_), Some(_)) => {
                        return Err(format!("calls[{n}]: name and tool are mutually exclusive"))
                    }
                    (None, None) => {
                        return Err(format!("calls[{n}]: one of name or tool is required"))
                    }
                    _ => {}
                }
            }
            Ok(())
        };
        let check_reasoning = |r: &Option<ValueRange>| match r {
            Some(r) => r.validate("turn.reasoning_tokens"),
            None => Ok(()),
        };
        match &self.turn {
            StepTurn::Assistant {
                text,
                lorem_tokens,
                reasoning_tokens,
            } => {
                check_text(text, lorem_tokens)?;
                check_reasoning(reasoning_tokens)
            }
            StepTurn::ToolCalls {
                calls,
                reasoning_tokens,
            } => {
                check_calls(calls)?;
                check_reasoning(reasoning_tokens)
            }
            StepTurn::Mixed {
                text,
                lorem_tokens,
                calls,
                reasoning_tokens,
            } => {
                check_text(text, lorem_tokens)?;
                check_calls(calls)?;
                check_reasoning(reasoning_tokens)
            }
            StepTurn::Error(_) => Ok(()),
        }
    }
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn error_for_kind(kind: &str) -> Option<SimError> {
    match kind {
        "rate_limit" => Some(SimError::RateLimit),
        "timeout" => Some(SimError::Timeout),
        "server_error" => Some(SimError::Other {
            message: "The server had an error while processing your request.".to_string(),
            status_code: Some(500),
        }),
        _ => None,
    }
}

/// Errors loading scenarios.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioLoadError {
    #[error("Failed to read scenarios from {path}: {reason}")]
    Io { path: String, reason: String },
    #[error("Failed to parse scenario JSON{}: {reason}", .path.as_deref().map(|p| format!(" in {p}")).unwrap_or_default())]
    Parse {
        path: Option<String>,
        reason: String,
    },
    #[error("Invalid scenario '{scenario}': {reason}")]
    Invalid { scenario: String, reason: String },
    #[error("Duplicate scenario name '{0}'")]
    Duplicate(String),
}

/// Scenarios looked up by name.
#[derive(Debug, Clone, Default)]
pub struct ScenarioSet {
    scenarios: BTreeMap<String, Scenario>,
}

impl ScenarioSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a set, validating each scenario.
    pub fn from_scenarios(
        scenarios: impl IntoIterator<Item = Scenario>,
    ) -> Result<Self, ScenarioLoadError> {
        let mut set = Self::new();
        for s in scenarios {
            set.insert(s)?;
        }
        Ok(set)
    }

    /// Add a scenario after validating it. Names must be unique.
    pub fn insert(&mut self, scenario: Scenario) -> Result<(), ScenarioLoadError> {
        scenario.validate()?;
        if self.scenarios.contains_key(&scenario.name) {
            return Err(ScenarioLoadError::Duplicate(scenario.name));
        }
        self.scenarios.insert(scenario.name.clone(), scenario);
        Ok(())
    }

    /// Parse JSON holding one scenario object or an array of them.
    pub fn from_json(json: &str) -> Result<Self, ScenarioLoadError> {
        let mut set = Self::new();
        set.add_json(json, None)?;
        Ok(set)
    }

    fn add_json(&mut self, json: &str, path: Option<&Path>) -> Result<(), ScenarioLoadError> {
        let parse_err = |e: serde_json::Error| ScenarioLoadError::Parse {
            path: path.map(|p| p.display().to_string()),
            reason: e.to_string(),
        };
        let value: Value = serde_json::from_str(json).map_err(parse_err)?;
        let scenarios: Vec<Scenario> = if value.is_array() {
            serde_json::from_value(value).map_err(parse_err)?
        } else {
            vec![serde_json::from_value(value).map_err(parse_err)?]
        };
        for s in scenarios {
            self.insert(s)?;
        }
        Ok(())
    }

    /// Load a single JSON file, or every `*.json` file in a directory
    /// (sorted by file name, not recursive).
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ScenarioLoadError> {
        let path = path.as_ref();
        let io_err = |e: std::io::Error| ScenarioLoadError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        };
        let mut files = Vec::new();
        if path.is_dir() {
            for entry in std::fs::read_dir(path).map_err(io_err)? {
                let p = entry.map_err(io_err)?.path();
                if p.is_file() && p.extension().is_some_and(|e| e == "json") {
                    files.push(p);
                }
            }
            files.sort();
        } else {
            files.push(path.to_path_buf());
        }
        let mut set = Self::new();
        for file in files {
            let content = std::fs::read_to_string(&file).map_err(|e| ScenarioLoadError::Io {
                path: file.display().to_string(),
                reason: e.to_string(),
            })?;
            set.add_json(&content, Some(&file))?;
        }
        Ok(set)
    }

    pub fn get(&self, name: &str) -> Option<&Scenario> {
        self.scenarios.get(name)
    }

    /// Scenario names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.scenarios.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.scenarios.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scenarios.is_empty()
    }
}

/// A parsed `[[llmsim:<name> key=value ...]]` marker.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub name: String,
    /// Changes the random draws without editing the scenario.
    pub seed: Option<u64>,
    /// Scales all timing: `0.5` runs twice as fast.
    pub speed: Option<f64>,
}

/// Find the first scenario marker in `text`. Unknown keys and malformed
/// values are ignored.
pub fn parse_marker(text: &str) -> Option<Marker> {
    let start = text.find(MARKER_PREFIX)? + MARKER_PREFIX.len();
    let len = text[start..].find("]]")?;
    let mut parts = text[start..start + len].split_whitespace();
    let name = parts.next().unwrap_or_default().to_string();
    let mut marker = Marker {
        name,
        seed: None,
        speed: None,
    };
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key {
            "seed" => marker.seed = value.parse().ok(),
            "speed" => {
                marker.speed = value
                    .parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0)
            }
            _ => {}
        }
    }
    Some(marker)
}

/// Role of a message in the conversation, provider neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    /// Tool results. They never count as a user message.
    Tool,
}

/// The parts of a request message that scenario resolution reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub text: String,
    /// Tool call ids the message carries: calls an assistant made, or the
    /// call a tool result answers.
    pub tool_call_ids: Vec<String>,
}

impl ConversationMessage {
    pub fn new(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            tool_call_ids: Vec::new(),
        }
    }

    pub fn with_tool_call_ids(mut self, ids: impl IntoIterator<Item = String>) -> Self {
        self.tool_call_ids.extend(ids);
        self
    }
}

impl From<&crate::openai::Message> for ConversationMessage {
    fn from(m: &crate::openai::Message) -> Self {
        use crate::openai::Role;
        let role = match m.role {
            Role::System | Role::Developer => MessageRole::System,
            Role::User => MessageRole::User,
            Role::Assistant => MessageRole::Assistant,
            Role::Tool | Role::Function => MessageRole::Tool,
        };
        let mut ids: Vec<String> = m
            .tool_calls
            .iter()
            .flatten()
            .map(|c| c.id.clone())
            .collect();
        ids.extend(m.tool_call_id.clone());
        Self {
            role,
            text: m.content.as_ref().map(|c| c.text()).unwrap_or_default(),
            tool_call_ids: ids,
        }
    }
}

/// A tool the agent offers.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    /// JSON schema of the arguments, used to fill placeholder arguments.
    pub parameters: Option<Value>,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: None,
        }
    }
}

impl From<&crate::openai::Tool> for ToolSpec {
    fn from(t: &crate::openai::Tool) -> Self {
        Self {
            name: t.function.name.clone(),
            parameters: t.function.parameters.clone(),
        }
    }
}

/// Why a request could not be resolved to a step.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ResolveError {
    #[error("Unknown llmsim scenario '{name}'. Known scenarios: {}", if known.is_empty() { "(none)".to_string() } else { known.join(", ") })]
    UnknownScenario { name: String, known: Vec<String> },
    #[error("llmsim scenario '{scenario}' step {step} calls tool '{tool}', which the request does not offer. Offered tools: {}", if offered.is_empty() { "(none)".to_string() } else { offered.join(", ") })]
    ToolNotOffered {
        scenario: String,
        step: usize,
        tool: String,
        offered: Vec<String>,
    },
    #[error("llmsim scenario '{scenario}' step {step} needs a tool, but the request offers none")]
    NoToolsOffered { scenario: String, step: usize },
    #[error("llmsim scenario '{scenario}' exhausted after {steps} steps (on_exhausted=error)")]
    Exhausted { scenario: String, steps: usize },
}

impl ResolveError {
    pub fn status_code(&self) -> u16 {
        match self {
            ResolveError::Exhausted { .. } => 500,
            _ => 400,
        }
    }

    /// The error as a `SimError`, for rendering in a provider's wire shape.
    pub fn to_sim_error(&self) -> SimError {
        match self {
            ResolveError::Exhausted { .. } => SimError::Other {
                message: self.to_string(),
                status_code: Some(500),
            },
            _ => SimError::InvalidRequest {
                message: self.to_string(),
            },
        }
    }
}

/// Timing resolved for one step (ranges sampled, `speed` applied). `None`
/// means "use the server's latency profile", scaled by `speed`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTiming {
    pub ttft: Option<Duration>,
    /// Delay between tokens, from `tokens_per_sec`.
    pub inter_token: Option<Duration>,
    pub stall: Option<ResolvedStall>,
    pub cut_after_tokens: Option<usize>,
    /// Timing multiplier from the marker (`1.0` when unset).
    pub speed: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedStall {
    pub after_tokens: usize,
    pub duration: Duration,
}

/// What a given attempt of a step does.
#[derive(Debug, Clone, PartialEq)]
pub enum AttemptOutcome {
    /// Return this error.
    Fail(StepError),
    /// Return the turn; drop the stream after `cut_after_tokens` if set.
    Respond { cut_after_tokens: Option<usize> },
}

/// The step a request resolves to.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub scenario: String,
    /// Steps already played in this turn (the step counter).
    pub step: usize,
    /// Index into `steps` after `on_exhausted` is applied.
    pub step_index: usize,
    /// What the model returns. Tool calls carry concrete names, arguments and
    /// ids. For an error step this is `SimTurn::Error`.
    pub turn: SimTurn,
    /// Reasoning text emitted before the answer.
    pub reasoning: Option<String>,
    pub timing: ResolvedTiming,
    pub fail_first: Vec<StepError>,
    /// Background error rates (`defaults.error_rate`).
    pub error_rate: Vec<(StepError, f64)>,
    /// Stable hash of scenario, seed, user message, user message count and
    /// step. Seeds the random draws and keys the attempt tracker.
    pub fingerprint: u64,
    turn_error: Option<StepError>,
}

impl Resolution {
    /// Decide what attempt `attempt` (0-based) of this step does: the
    /// `fail_first` errors in order, then the step's own error (error steps
    /// always fail), then a background `error_rate` draw. The stream is cut
    /// only on the first attempt past `fail_first`, so a retry succeeds.
    pub fn outcome(&self, attempt: u32) -> AttemptOutcome {
        let a = attempt as usize;
        if let Some(err) = self.fail_first.get(a) {
            return AttemptOutcome::Fail(err.clone());
        }
        if let Some(err) = &self.turn_error {
            return AttemptOutcome::Fail(err.clone());
        }
        if !self.error_rate.is_empty() {
            let mut rng = SplitMix64::new(hash_parts(&[
                &self.fingerprint.to_le_bytes(),
                b"error_rate",
                &attempt.to_le_bytes(),
            ]));
            let draw = rng.unit();
            let mut acc = 0.0;
            for (err, rate) in &self.error_rate {
                acc += rate;
                if draw < acc {
                    return AttemptOutcome::Fail(err.clone());
                }
            }
        }
        AttemptOutcome::Respond {
            cut_after_tokens: if a == self.fail_first.len() {
                self.timing.cut_after_tokens
            } else {
                None
            },
        }
    }
}

/// Resolve which scenario step a request plays. A pure function of its
/// inputs: any worker or process returns the same answer.
///
/// Returns `Ok(None)` when neither the last user message carries a marker
/// nor `model` is `llmsim-scenario-<name>`; callers then apply their
/// default behaviour.
pub fn resolve(
    messages: &[ConversationMessage],
    tools: &[ToolSpec],
    model: Option<&str>,
    set: &ScenarioSet,
) -> Result<Option<Resolution>, ResolveError> {
    let user_idx = messages.iter().rposition(|m| m.role == MessageRole::User);
    let user_text = user_idx.map(|i| messages[i].text.as_str()).unwrap_or("");

    let marker = parse_marker(user_text).or_else(|| {
        model
            .and_then(|m| m.strip_prefix(MODEL_PREFIX))
            .map(|name| Marker {
                name: name.to_string(),
                seed: None,
                speed: None,
            })
    });
    let Some(marker) = marker else {
        return Ok(None);
    };
    let scenario = set
        .get(&marker.name)
        .ok_or_else(|| ResolveError::UnknownScenario {
            name: marker.name.clone(),
            known: set.names(),
        })?;

    // Steps already played: assistant messages after the triggering user
    // message, or more if llmsim-issued call ids show a later step (history
    // compaction can drop an assistant message mid-turn).
    let after = &messages[user_idx.map_or(0, |i| i + 1)..];
    let assistant_count = after
        .iter()
        .filter(|m| m.role == MessageRole::Assistant)
        .count();
    let id_steps = after
        .iter()
        .flat_map(|m| m.tool_call_ids.iter())
        .filter_map(|id| parse_call_id(id))
        .filter(|(name, _)| name == &scenario.name)
        .map(|(_, step)| step + 1)
        .max()
        .unwrap_or(0);
    let step = assistant_count.max(id_steps);

    let n = scenario.steps.len();
    let step_index = if step < n {
        step
    } else {
        match scenario.on_exhausted {
            OnExhausted::RepeatLast => n - 1,
            OnExhausted::Loop => step % n,
            OnExhausted::Error => {
                return Err(ResolveError::Exhausted {
                    scenario: scenario.name.clone(),
                    steps: n,
                })
            }
        }
    };
    let def = &scenario.steps[step_index];

    let user_count = messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .count() as u64;
    let fingerprint = hash_parts(&[
        scenario.name.as_bytes(),
        &marker.seed.unwrap_or(0).to_le_bytes(),
        user_text.as_bytes(),
        &user_count.to_le_bytes(),
        &(step as u64).to_le_bytes(),
    ]);
    let mut rng = SplitMix64::new(fingerprint);

    let speed = marker.speed.unwrap_or(1.0);
    let timing = def.timing.over(&scenario.defaults.timing);
    let scale = |ms: f64| Duration::from_secs_f64((ms * speed).max(0.0) / 1000.0);
    let resolved_timing = ResolvedTiming {
        ttft: timing.ttft_ms.map(|r| scale(r.sample(&mut rng) as f64)),
        inter_token: timing.tokens_per_sec.map(|tps| scale(1000.0 / tps)),
        stall: timing.stall.map(|s| ResolvedStall {
            after_tokens: s.after_tokens,
            duration: scale(s.ms as f64),
        }),
        cut_after_tokens: timing.cut_after_tokens,
        speed,
    };

    let nonce = format!("{:08x}", (fingerprint >> 32) as u32);
    let make_calls = |calls: &[StepCall], rng: &mut SplitMix64| {
        calls
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let tool = pick_tool(c, tools, rng).map_err(|e| match e {
                    PickError::NotOffered(tool) => ResolveError::ToolNotOffered {
                        scenario: scenario.name.clone(),
                        step,
                        tool,
                        offered: tools.iter().map(|t| t.name.clone()).collect(),
                    },
                    PickError::NoTools => ResolveError::NoToolsOffered {
                        scenario: scenario.name.clone(),
                        step,
                    },
                })?;
                let arguments = c.arguments.clone().unwrap_or_else(|| {
                    tool.and_then(|t| t.parameters.as_ref())
                        .map(placeholder_arguments)
                        .unwrap_or_else(|| Value::Object(Default::default()))
                });
                let name = c
                    .name
                    .clone()
                    .or_else(|| tool.map(|t| t.name.clone()))
                    .unwrap_or_default();
                Ok(SimToolCall {
                    name,
                    arguments,
                    id: Some(format!(
                        "{CALL_ID_PREFIX}{}_{step}_{i}_{nonce}",
                        scenario.name
                    )),
                })
            })
            .collect::<Result<Vec<_>, ResolveError>>()
    };
    let text_of = |text: &Option<String>, lorem: &Option<ValueRange>, rng: &mut SplitMix64| {
        text.clone()
            .unwrap_or_else(|| lorem_text(lorem.map_or(0, |r| r.sample(rng)) as usize, rng))
    };
    let reasoning_of = |r: &Option<ValueRange>, rng: &mut SplitMix64| {
        r.map(|r| lorem_text(r.sample(rng) as usize, rng))
            .filter(|t| !t.is_empty())
    };

    let mut turn_error = None;
    let (turn, reasoning) = match &def.turn {
        StepTurn::Assistant {
            text,
            lorem_tokens,
            reasoning_tokens,
        } => {
            let reasoning = reasoning_of(reasoning_tokens, &mut rng);
            let text = text_of(text, lorem_tokens, &mut rng);
            (SimTurn::Assistant { text }, reasoning)
        }
        StepTurn::ToolCalls {
            calls,
            reasoning_tokens,
        } => {
            let reasoning = reasoning_of(reasoning_tokens, &mut rng);
            let calls = make_calls(calls, &mut rng)?;
            (SimTurn::ToolCalls { calls }, reasoning)
        }
        StepTurn::Mixed {
            text,
            lorem_tokens,
            calls,
            reasoning_tokens,
        } => {
            let reasoning = reasoning_of(reasoning_tokens, &mut rng);
            let text = text_of(text, lorem_tokens, &mut rng);
            let calls = make_calls(calls, &mut rng)?;
            (SimTurn::Mixed { text, calls }, reasoning)
        }
        StepTurn::Error(err) => {
            turn_error = Some(err.clone());
            (SimTurn::Error(err.error.clone()), None)
        }
    };

    let error_rate = scenario
        .defaults
        .error_rate
        .iter()
        .filter(|(_, rate)| **rate > 0.0)
        .filter_map(|(kind, rate)| error_for_kind(kind).map(|e| (StepError::from(e), *rate)))
        .collect();

    Ok(Some(Resolution {
        scenario: scenario.name.clone(),
        step,
        step_index,
        turn,
        reasoning,
        timing: resolved_timing,
        fail_first: def.fail_first.clone(),
        error_rate,
        fingerprint,
        turn_error,
    }))
}

/// Parse `call_llmsim_<scenario>_<step>_<n>_<nonce>` into scenario and step.
pub fn parse_call_id(id: &str) -> Option<(String, usize)> {
    let rest = id.strip_prefix(CALL_ID_PREFIX)?;
    let mut parts = rest.rsplitn(4, '_');
    let _nonce = parts.next()?;
    let _n: usize = parts.next()?.parse().ok()?;
    let step: usize = parts.next()?.parse().ok()?;
    let name = parts.next()?;
    is_valid_name(name).then(|| (name.to_string(), step))
}

enum PickError {
    NotOffered(String),
    NoTools,
}

/// Resolve a step call to an offered tool. Exact names must be offered;
/// placeholders pick among offered tools deterministically.
fn pick_tool<'a>(
    call: &StepCall,
    tools: &'a [ToolSpec],
    rng: &mut SplitMix64,
) -> Result<Option<&'a ToolSpec>, PickError> {
    if let Some(name) = &call.name {
        return tools
            .iter()
            .find(|t| &t.name == name)
            .map(Some)
            .ok_or_else(|| PickError::NotOffered(name.clone()));
    }
    let placeholder = call.tool.unwrap_or(ToolPlaceholder::Any);
    if tools.is_empty() {
        return Err(PickError::NoTools);
    }
    let class = |kws: &[&str]| -> Vec<&'a ToolSpec> {
        tools
            .iter()
            .filter(|t| tool_matches(&t.name, kws))
            .collect()
    };
    let mut candidates = match placeholder {
        ToolPlaceholder::AnyRead => class(READ_WORDS),
        ToolPlaceholder::AnyWrite => class(WRITE_WORDS),
        ToolPlaceholder::Any => Vec::new(),
    };
    if candidates.is_empty() {
        candidates = tools.iter().collect();
    }
    let i = rng.below(candidates.len() as u64) as usize;
    Ok(Some(candidates[i]))
}

const READ_WORDS: &[&str] = &[
    "read", "get", "list", "ls", "search", "find", "grep", "glob", "view", "cat", "fetch", "query",
    "show", "lookup", "stat", "head", "tail", "describe", "inspect",
];
const WRITE_WORDS: &[&str] = &[
    "write", "edit", "create", "update", "delete", "remove", "patch", "put", "save", "apply",
    "insert", "append", "set", "mkdir", "move", "rename", "replace",
];

/// Whether any word of a tool name (split on `_`, `-`, `.` and case
/// changes) starts with one of the keywords.
fn tool_matches(name: &str, keywords: &[&str]) -> bool {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            words.push(std::mem::take(&mut cur));
            prev_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_lower {
            words.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_ascii_lowercase();
        cur.push(ch.to_ascii_lowercase());
    }
    words.push(cur);
    words
        .iter()
        .filter(|w| !w.is_empty())
        .any(|w| keywords.iter().any(|k| w.starts_with(k)))
}

/// Fill a JSON schema's required properties with placeholder values.
fn placeholder_arguments(schema: &Value) -> Value {
    let mut out = serde_json::Map::new();
    let props = schema.get("properties").and_then(Value::as_object);
    let required = schema.get("required").and_then(Value::as_array);
    if let (Some(props), Some(required)) = (props, required) {
        for key in required.iter().filter_map(Value::as_str) {
            if let Some(prop) = props.get(key) {
                out.insert(key.to_string(), placeholder_value(key, prop));
            }
        }
    }
    Value::Object(out)
}

fn placeholder_value(key: &str, schema: &Value) -> Value {
    if let Some(first) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return first.clone();
    }
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("string"),
        _ => "string",
    };
    match ty {
        "integer" | "number" => schema
            .get("minimum")
            .cloned()
            .unwrap_or_else(|| Value::from(1)),
        "boolean" => Value::Bool(false),
        "array" => Value::Array(Vec::new()),
        "object" => placeholder_arguments(schema),
        _ => {
            let k = key.to_ascii_lowercase();
            if k.contains("path") || k.contains("file") || k.contains("dir") {
                Value::from(".")
            } else {
                Value::from("llmsim")
            }
        }
    }
}

/// Lorem text of `tokens` words, as sentences.
fn lorem_text(tokens: usize, rng: &mut SplitMix64) -> String {
    let words = crate::generator::LoremGenerator::LOREM_WORDS;
    let mut out = String::with_capacity(tokens * 7);
    let mut sentence_len = 0;
    for i in 0..tokens {
        let w = words[rng.below(words.len() as u64) as usize];
        if i > 0 {
            out.push(' ');
        }
        if sentence_len == 0 {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                out.extend(f.to_uppercase());
                out.push_str(c.as_str());
            }
        } else {
            out.push_str(w);
        }
        sentence_len += 1;
        if sentence_len >= 8 + (i % 7) || i + 1 == tokens {
            out.push('.');
            sentence_len = 0;
        }
    }
    out
}

/// 64-bit FNV-1a over length-prefixed parts.
fn hash_parts(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for p in parts {
        feed(&(p.len() as u64).to_le_bytes());
        feed(p);
    }
    h
}

/// SplitMix64: tiny, fast and stable across versions.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    fn below(&mut self, n: u64) -> u64 {
        if n <= 1 {
            return 0;
        }
        self.next_u64() % n
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// In-process attempt counter per step fingerprint, so `fail_first` knows
/// which attempt a retry is. Bounded; entries expire after `ttl`.
#[derive(Debug)]
pub struct AttemptTracker {
    entries: Mutex<HashMap<u64, (u32, Instant)>>,
    capacity: usize,
    ttl: Duration,
}

impl Default for AttemptTracker {
    fn default() -> Self {
        Self::new(50_000, Duration::from_secs(600))
    }
}

impl AttemptTracker {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            capacity: capacity.max(1),
            ttl,
        }
    }

    /// Return this call's 0-based attempt number for `fingerprint` and
    /// count it.
    pub fn next_attempt(&self, fingerprint: u64) -> u32 {
        let now = Instant::now();
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((count, seen)) = map.get_mut(&fingerprint) {
            if now.duration_since(*seen) < self.ttl {
                let attempt = *count;
                *count = count.saturating_add(1);
                *seen = now;
                return attempt;
            }
        }
        if map.len() >= self.capacity && !map.contains_key(&fingerprint) {
            let ttl = self.ttl;
            map.retain(|_, (_, seen)| now.duration_since(*seen) < ttl);
            if map.len() >= self.capacity {
                // Still full of live entries: start over rather than scan
                // for the oldest on every insert. Rare, and only costs a
                // repeated `fail_first` error.
                map.clear();
            }
        }
        map.insert(fingerprint, (1, now));
        0
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RESEARCH: &str = r#"{
      "name": "research-3-tools",
      "description": "Three tool rounds, then a long answer. 429 on the second call.",
      "defaults": { "timing": { "ttft_ms": [800, 1300], "tokens_per_sec": 65 } },
      "steps": [
        { "turn": { "type": "mixed", "text": "Let me check the repo.",
                    "calls": [{ "tool": "any_read", "arguments": {} }] } },
        { "fail_first": [{ "kind": "rate_limit", "retry_after_ms": 1000 }],
          "turn": { "type": "tool_calls",
                    "calls": [{ "name": "bash", "arguments": { "command": "echo step 2" } }] } },
        { "timing": { "ttft_ms": 2500 },
          "turn": { "type": "tool_calls", "calls": [{ "tool": "any_read" }, { "tool": "any_read" }] } },
        { "timing": { "tokens_per_sec": 40, "stall": { "after_tokens": 120, "ms": 3000 } },
          "turn": { "type": "assistant", "lorem_tokens": 600 } }
      ],
      "on_exhausted": "repeat_last"
    }"#;

    fn set() -> ScenarioSet {
        ScenarioSet::from_json(RESEARCH).unwrap()
    }

    fn tools() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "read_file".into(),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}, "limit": {"type": "integer"}},
                    "required": ["path"]
                })),
            },
            ToolSpec::new("bash"),
            ToolSpec::new("write_file"),
        ]
    }

    fn user(text: &str) -> ConversationMessage {
        ConversationMessage::new(MessageRole::User, text)
    }

    fn assistant_with_calls(res: &Resolution) -> ConversationMessage {
        let ids = match &res.turn {
            SimTurn::ToolCalls { calls } | SimTurn::Mixed { calls, .. } => {
                calls.iter().filter_map(|c| c.id.clone()).collect()
            }
            _ => Vec::new(),
        };
        ConversationMessage::new(MessageRole::Assistant, "").with_tool_call_ids(ids)
    }

    fn tool_result(id: &str) -> ConversationMessage {
        ConversationMessage::new(MessageRole::Tool, "ok").with_tool_call_ids([id.to_string()])
    }

    fn resolve_ok(messages: &[ConversationMessage]) -> Resolution {
        resolve(messages, &tools(), Some("gpt-5"), &set())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn parses_the_spec_example() {
        let s = set();
        let scn = s.get("research-3-tools").unwrap();
        assert_eq!(scn.steps.len(), 4);
        assert_eq!(scn.steps[1].fail_first[0].retry_after_ms, Some(1000));
        assert_eq!(scn.steps[1].fail_first[0].error, SimError::RateLimit);
    }

    #[test]
    fn rejects_invalid_scenarios() {
        let bad = [
            (
                r#"{"name":"Bad Name","steps":[{"turn":{"type":"assistant","text":"x"}}]}"#,
                "name",
            ),
            (r#"{"name":"x","steps":[]}"#, "at least one step"),
            (
                r#"{"name":"x","steps":[{"turn":{"type":"assistant","text":"a","lorem_tokens":3}}]}"#,
                "mutually exclusive",
            ),
            (
                r#"{"name":"x","steps":[{"turn":{"type":"assistant"}}]}"#,
                "required",
            ),
            (
                r#"{"name":"x","steps":[{"turn":{"type":"tool_calls","calls":[{}]}}]}"#,
                "name or tool",
            ),
            (
                r#"{"name":"x","steps":[{"turn":{"type":"assistant","lorem_tokens":[9,3]}}]}"#,
                "greater than max",
            ),
            (
                r#"{"name":"x","defaults":{"error_rate":{"bogus":0.1}},"steps":[{"turn":{"type":"assistant","text":"a"}}]}"#,
                "unknown kind",
            ),
            (
                r#"{"name":"x","steps":[{"turn":{"type":"assistant","text":"a"},"timing":{"tokens_per_sec":0}}]}"#,
                "positive",
            ),
        ];
        for (json, needle) in bad {
            let err = ScenarioSet::from_json(json).unwrap_err().to_string();
            assert!(err.contains(needle), "{json}: {err}");
        }
        // Typos in field names fail loudly.
        assert!(ScenarioSet::from_json(
            r#"{"name":"x","steps":[{"turn":{"type":"assistant","text":"a"},"timming":{}}]}"#
        )
        .is_err());
    }

    #[test]
    fn duplicate_names_rejected() {
        let json = r#"[{"name":"a","steps":[{"turn":{"type":"assistant","text":"x"}}]},
                       {"name":"a","steps":[{"turn":{"type":"assistant","text":"y"}}]}]"#;
        assert!(matches!(
            ScenarioSet::from_json(json),
            Err(ScenarioLoadError::Duplicate(_))
        ));
    }

    #[test]
    fn parses_markers() {
        assert_eq!(
            parse_marker("Summarise. [[llmsim:research-3-tools]]"),
            Some(Marker {
                name: "research-3-tools".into(),
                seed: None,
                speed: None
            })
        );
        let m = parse_marker("x [[llmsim:a-1 seed=7 speed=0.5 colour=red]] y").unwrap();
        assert_eq!(m.name, "a-1");
        assert_eq!(m.seed, Some(7));
        assert_eq!(m.speed, Some(0.5));
        let m = parse_marker("[[llmsim:a seed=x speed=-1]]").unwrap();
        assert_eq!((m.seed, m.speed), (None, None));
        assert_eq!(parse_marker("no marker here"), None);
        assert_eq!(parse_marker("[[llmsim:unterminated"), None);
    }

    #[test]
    fn no_marker_and_plain_model_resolves_to_none() {
        let r = resolve(&[user("hello")], &tools(), Some("gpt-5"), &set()).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn model_name_selects_scenario_and_marker_wins() {
        let other = ScenarioSet::from_json(
            r#"[{"name":"chat","steps":[{"turn":{"type":"assistant","text":"from-model"}}]},
                {"name":"marked","steps":[{"turn":{"type":"assistant","text":"from-marker"}}]}]"#,
        )
        .unwrap();
        let r = resolve(&[user("hi")], &[], Some("llmsim-scenario-chat"), &other)
            .unwrap()
            .unwrap();
        assert_eq!(
            r.turn,
            SimTurn::Assistant {
                text: "from-model".into()
            }
        );
        let r = resolve(
            &[user("hi [[llmsim:marked]]")],
            &[],
            Some("llmsim-scenario-chat"),
            &other,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            r.turn,
            SimTurn::Assistant {
                text: "from-marker".into()
            }
        );
    }

    #[test]
    fn unknown_scenario_is_a_400_listing_known_ones() {
        let err = resolve(&[user("[[llmsim:resarch]]")], &tools(), None, &set()).unwrap_err();
        assert_eq!(err.status_code(), 400);
        let msg = err.to_string();
        assert!(
            msg.contains("resarch") && msg.contains("research-3-tools"),
            "{msg}"
        );
    }

    #[test]
    fn walks_steps_through_a_conversation() {
        let mut msgs = vec![
            ConversationMessage::new(MessageRole::System, "sys"),
            user("Summarise. [[llmsim:research-3-tools]]"),
        ];
        let r0 = resolve_ok(&msgs);
        assert_eq!(r0.step, 0);
        match &r0.turn {
            SimTurn::Mixed { text, calls } => {
                assert_eq!(text, "Let me check the repo.");
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].arguments, json!({}));
            }
            t => panic!("unexpected {t:?}"),
        }
        msgs.push(assistant_with_calls(&r0));
        // Tool results do not count as a user message.
        msgs.push(tool_result(r0.turn_call_id(0)));

        let r1 = resolve_ok(&msgs);
        assert_eq!(r1.step, 1);
        assert_eq!(r1.fail_first.len(), 1);
        match &r1.turn {
            SimTurn::ToolCalls { calls } => assert_eq!(calls[0].name, "bash"),
            t => panic!("unexpected {t:?}"),
        }
        msgs.push(assistant_with_calls(&r1));
        msgs.push(tool_result(r1.turn_call_id(0)));

        let r2 = resolve_ok(&msgs);
        assert_eq!(r2.step, 2);
        assert_eq!(r2.timing.ttft, Some(Duration::from_millis(2500)));
        match &r2.turn {
            SimTurn::ToolCalls { calls } => {
                assert_eq!(calls.len(), 2);
                // Placeholder with no arguments fills required params.
                assert_eq!(calls[0].arguments, json!({"path": "."}));
            }
            t => panic!("unexpected {t:?}"),
        }
        msgs.push(assistant_with_calls(&r2));

        let r3 = resolve_ok(&msgs);
        assert_eq!(r3.step, 3);
        let SimTurn::Assistant { text } = &r3.turn else {
            panic!("expected assistant")
        };
        assert_eq!(text.split_whitespace().count(), 600);
        assert_eq!(
            r3.timing.inter_token,
            Some(Duration::from_secs_f64(1.0 / 40.0))
        );
        assert_eq!(
            r3.timing.stall,
            Some(ResolvedStall {
                after_tokens: 120,
                duration: Duration::from_millis(3000)
            })
        );
        msgs.push(ConversationMessage::new(
            MessageRole::Assistant,
            text.clone(),
        ));

        // Past the end: repeat_last.
        let r4 = resolve_ok(&msgs);
        assert_eq!((r4.step, r4.step_index), (4, 3));

        // A new user message without a marker leaves the scenario.
        msgs.push(user("thanks"));
        assert!(resolve(&msgs, &tools(), Some("gpt-5"), &set())
            .unwrap()
            .is_none());
        // And a new marker restarts at step 0.
        msgs.push(ConversationMessage::new(MessageRole::Assistant, "ok"));
        msgs.push(user("again [[llmsim:research-3-tools]]"));
        assert_eq!(resolve_ok(&msgs).step, 0);
    }

    #[test]
    fn call_ids_recover_compacted_history() {
        let msgs = vec![
            user("go [[llmsim:research-3-tools]]"),
            // The assistant messages for steps 0 and 1 were compacted away;
            // only a tool result for step 1 survives.
            tool_result("call_llmsim_research-3-tools_1_0_deadbeef"),
        ];
        assert_eq!(resolve_ok(&msgs).step, 2);
        // Ids from another scenario or the legacy script format are ignored.
        let msgs = vec![
            user("go [[llmsim:research-3-tools]]"),
            tool_result("call_llmsim_other_5_0_deadbeef"),
            tool_result("call_llmsim_3_0"),
        ];
        assert_eq!(resolve_ok(&msgs).step, 0);
    }

    #[test]
    fn parses_call_ids() {
        assert_eq!(
            parse_call_id("call_llmsim_research-3-tools_12_1_0a0b0c0d"),
            Some(("research-3-tools".into(), 12))
        );
        assert_eq!(parse_call_id("call_llmsim_3_0"), None);
        assert_eq!(parse_call_id("call_abc"), None);
    }

    #[test]
    fn exact_tool_name_must_be_offered() {
        let msgs = vec![
            user("go [[llmsim:research-3-tools]]"),
            tool_result("call_llmsim_research-3-tools_0_0_deadbeef"),
        ];
        let err = resolve(&msgs, &[ToolSpec::new("read_file")], None, &set()).unwrap_err();
        assert_eq!(err.status_code(), 400);
        assert!(err.to_string().contains("'bash'"), "{err}");
        let err = resolve(&[user("[[llmsim:research-3-tools]]")], &[], None, &set()).unwrap_err();
        assert!(matches!(err, ResolveError::NoToolsOffered { .. }));
    }

    #[test]
    fn placeholders_pick_by_class() {
        let mut rng = SplitMix64::new(1);
        let tools = vec![
            ToolSpec::new("bash"),
            ToolSpec::new("writeFile"),
            ToolSpec::new("search_code"),
        ];
        for _ in 0..20 {
            let read = StepCall {
                tool: Some(ToolPlaceholder::AnyRead),
                ..Default::default()
            };
            let write = StepCall {
                tool: Some(ToolPlaceholder::AnyWrite),
                ..Default::default()
            };
            let r = pick_tool(&read, &tools, &mut rng).ok().flatten().unwrap();
            let w = pick_tool(&write, &tools, &mut rng).ok().flatten().unwrap();
            assert_eq!(r.name, "search_code");
            assert_eq!(w.name, "writeFile");
        }
        // No read-like tool: falls back to any offered tool.
        let only = vec![ToolSpec::new("bash")];
        let read = StepCall {
            tool: Some(ToolPlaceholder::AnyRead),
            ..Default::default()
        };
        assert_eq!(
            pick_tool(&read, &only, &mut rng)
                .ok()
                .flatten()
                .unwrap()
                .name,
            "bash"
        );
    }

    #[test]
    fn same_inputs_give_same_draws_and_seed_changes_them() {
        let msgs = vec![user("Summarise #17. [[llmsim:research-3-tools]]")];
        let a = resolve_ok(&msgs);
        let b = resolve_ok(&msgs);
        assert_eq!(a, b);
        let ttft = a.timing.ttft.unwrap();
        assert!(ttft >= Duration::from_millis(800) && ttft <= Duration::from_millis(1300));

        // Different sessions (different text) and seeds draw differently.
        let ttfts: std::collections::HashSet<_> = (0..20)
            .map(|i| {
                resolve_ok(&[user(&format!(
                    "Summarise #17. [[llmsim:research-3-tools seed={i}]]"
                ))])
                .timing
                .ttft
            })
            .collect();
        assert!(ttfts.len() > 1);
    }

    #[test]
    fn speed_scales_timing() {
        let set = ScenarioSet::from_json(
            r#"{"name":"timed","defaults":{"timing":{"ttft_ms":1000,"tokens_per_sec":50,
                "stall":{"after_tokens":3,"ms":400}}},
                "steps":[{"turn":{"type":"assistant","text":"x"}}]}"#,
        )
        .unwrap();
        let r = resolve(&[user("x [[llmsim:timed speed=0.5]]")], &[], None, &set)
            .unwrap()
            .unwrap();
        assert_eq!(r.timing.speed, 0.5);
        assert_eq!(r.timing.ttft, Some(Duration::from_millis(500)));
        assert_eq!(r.timing.inter_token, Some(Duration::from_millis(10)));
        assert_eq!(r.timing.stall.unwrap().duration, Duration::from_millis(200));
    }

    #[test]
    fn fail_first_then_cut_then_success() {
        let set = ScenarioSet::from_json(
            r#"{"name":"flaky","steps":[{
                "fail_first":[{"kind":"rate_limit","retry_after_ms":1500},{"kind":"timeout"}],
                "timing":{"cut_after_tokens":5},
                "turn":{"type":"assistant","lorem_tokens":20}}]}"#,
        )
        .unwrap();
        let r = resolve(&[user("[[llmsim:flaky]]")], &[], None, &set)
            .unwrap()
            .unwrap();
        match r.outcome(0) {
            AttemptOutcome::Fail(e) => {
                assert_eq!(e.error, SimError::RateLimit);
                assert_eq!(e.retry_after_secs(), Some(2));
            }
            o => panic!("{o:?}"),
        }
        assert!(matches!(
            r.outcome(1),
            AttemptOutcome::Fail(StepError {
                error: SimError::Timeout,
                ..
            })
        ));
        assert_eq!(
            r.outcome(2),
            AttemptOutcome::Respond {
                cut_after_tokens: Some(5)
            }
        );
        assert_eq!(
            r.outcome(3),
            AttemptOutcome::Respond {
                cut_after_tokens: None
            }
        );
    }

    #[test]
    fn error_steps_always_fail() {
        let set = ScenarioSet::from_json(
            r#"{"name":"down","steps":[{"turn":{"type":"error","kind":"other","message":"boom","status_code":503,"retry_after_ms":200}}]}"#,
        )
        .unwrap();
        let r = resolve(&[user("[[llmsim:down]]")], &[], None, &set)
            .unwrap()
            .unwrap();
        for a in 0..3 {
            let AttemptOutcome::Fail(e) = r.outcome(a) else {
                panic!("expected failure")
            };
            assert_eq!(e.error.status_code(), 503);
            assert_eq!(e.retry_after_secs(), Some(1));
        }
    }

    #[test]
    fn error_rate_is_reproducible() {
        let set = ScenarioSet::from_json(
            r#"{"name":"bg","defaults":{"error_rate":{"rate_limit":0.3}},
                "steps":[{"turn":{"type":"assistant","text":"ok"}}]}"#,
        )
        .unwrap();
        let mut failures = 0;
        for i in 0..400 {
            let r = resolve(&[user(&format!("s{i} [[llmsim:bg]]"))], &[], None, &set)
                .unwrap()
                .unwrap();
            let first = r.outcome(0);
            assert_eq!(first, r.outcome(0));
            if matches!(first, AttemptOutcome::Fail(_)) {
                failures += 1;
            }
        }
        assert!((80..=160).contains(&failures), "failures={failures}");
    }

    #[test]
    fn on_exhausted_modes() {
        let json = |mode: &str| {
            format!(
                r#"{{"name":"two","on_exhausted":"{mode}","steps":[
                    {{"turn":{{"type":"assistant","text":"a"}}}},
                    {{"turn":{{"type":"assistant","text":"b"}}}}]}}"#
            )
        };
        let msgs = |played: usize| {
            let mut m = vec![user("[[llmsim:two]]")];
            m.extend((0..played).map(|_| ConversationMessage::new(MessageRole::Assistant, "")));
            m
        };
        let loop_set = ScenarioSet::from_json(&json("loop")).unwrap();
        let r = resolve(&msgs(2), &[], None, &loop_set).unwrap().unwrap();
        assert_eq!(r.turn, SimTurn::Assistant { text: "a".into() });
        let err_set = ScenarioSet::from_json(&json("error")).unwrap();
        let err = resolve(&msgs(2), &[], None, &err_set).unwrap_err();
        assert_eq!(err.status_code(), 500);
    }

    #[test]
    fn reasoning_tokens_are_generated() {
        let set = ScenarioSet::from_json(
            r#"{"name":"thinking","steps":[{"turn":{"type":"assistant","lorem_tokens":30,"reasoning_tokens":40}}]}"#,
        )
        .unwrap();
        let r = resolve(&[user("[[llmsim:thinking]]")], &[], None, &set)
            .unwrap()
            .unwrap();
        assert_eq!(r.reasoning.unwrap().split_whitespace().count(), 40);
    }

    #[test]
    fn attempt_tracker_counts_and_bounds() {
        let t = AttemptTracker::new(3, Duration::from_secs(60));
        assert_eq!(t.next_attempt(1), 0);
        assert_eq!(t.next_attempt(1), 1);
        assert_eq!(t.next_attempt(2), 0);
        assert_eq!(t.next_attempt(3), 0);
        assert_eq!(t.len(), 3);
        // Full of live entries: inserting a new key never exceeds capacity.
        assert_eq!(t.next_attempt(4), 0);
        assert!(t.len() <= 3);

        let expiring = AttemptTracker::new(10, Duration::from_millis(0));
        assert_eq!(expiring.next_attempt(9), 0);
        assert_eq!(expiring.next_attempt(9), 0);
    }

    #[test]
    fn interleaved_sessions_get_their_own_steps() {
        // 500 sessions advance through the scenario in an interleaved order;
        // each always resolves the step its own history implies.
        let tools = tools();
        let set = set();
        let mut sessions: Vec<Vec<ConversationMessage>> = (0..500)
            .map(|i| vec![user(&format!("session {i} [[llmsim:research-3-tools]]"))])
            .collect();
        for round in 0..4 {
            for idx in (0..500).rev().step_by(1) {
                let msgs = &mut sessions[(idx * 7919) % 500];
                let r = resolve(msgs, &tools, None, &set).unwrap().unwrap();
                assert_eq!(r.step, round);
                msgs.push(assistant_with_calls(&r));
            }
        }
    }

    impl Resolution {
        fn turn_call_id(&self, n: usize) -> &str {
            match &self.turn {
                SimTurn::ToolCalls { calls } | SimTurn::Mixed { calls, .. } => {
                    calls[n].id.as_deref().unwrap()
                }
                _ => panic!("no calls"),
            }
        }
    }
}
