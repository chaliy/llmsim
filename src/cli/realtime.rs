// OpenAI Realtime API over WebSocket (`GET /openai/v1/realtime?model=...`).
//
// A voice session: the client streams PCM16 audio (24 kHz mono, base64) and
// the server answers with audio plus a transcript, or with text, as events.
//
// Decisions:
// - Event names follow the GA API (`response.output_audio.delta`,
//   `conversation.item.added`); a connection with `OpenAI-Beta: realtime=v1`
//   gets the beta names (`response.audio.delta`, `conversation.item.created`)
//   and the beta session shape. Both `session.update` shapes are accepted.
// - Server VAD runs on the real audio: 20 ms frames whose RMS is over a level
//   derived from `threshold` count as speech; `silence_duration_ms` of quiet
//   after speech ends the turn, commits the buffer and (with
//   `create_response`) starts a response. Speech during a response cancels
//   it when `interrupt_response` is on, like barge-in on the real API.
// - Answer audio is a quiet tone, 400 ms per spoken word, so players and
//   buffers see realistic sizes; it carries no speech.
// - Usage follows OpenAI's audio rates: input audio 10 tokens/s, output
//   audio 20 tokens/s, text through the tokenizer.

use super::state::AppState;
use crate::ids::{prefixed_compact_id, unix_timestamp};
use crate::{create_generator, EndpointType};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Samples per second of PCM16 audio.
const RATE: usize = 24_000;
/// Bytes per millisecond of PCM16 mono audio.
const BYTES_PER_MS: usize = RATE * 2 / 1000;
/// VAD frame length.
const FRAME_MS: usize = 20;
/// Spoken length of one answer word.
const WORD_MS: usize = 400;
/// Shortest buffer a manual commit accepts.
const MIN_COMMIT_MS: usize = 100;
/// Default spoken-answer length in words.
const DEFAULT_WORDS: usize = 40;

fn event_id() -> String {
    prefixed_compact_id("event_")
}

/// Decode standard base64 (padding optional); `None` on bad input.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
        if chunk.len() == 1 {
            return None;
        }
    }
    Some(out)
}

/// RMS of a PCM16 little-endian frame, 0.0-1.0.
fn rms(frame: &[u8]) -> f64 {
    let samples = frame.len() / 2;
    if samples == 0 {
        return 0.0;
    }
    let sum: f64 = frame
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| {
            let s = i16::from_le_bytes(*b) as f64 / 32768.0;
            s * s
        })
        .sum();
    (sum / samples as f64).sqrt()
}

/// A quiet 220 Hz tone, `ms` long, as PCM16.
fn tone(ms: usize, phase: &mut usize) -> Vec<u8> {
    let n = ms * RATE / 1000;
    let mut out = Vec::with_capacity(n * 2);
    for _ in 0..n {
        let t = *phase as f64 / RATE as f64;
        let s = (2.0 * std::f64::consts::PI * 220.0 * t).sin() * 1500.0;
        out.extend_from_slice(&(s as i16).to_le_bytes());
        *phase += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Session settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Vad {
    threshold: f64,
    prefix_padding_ms: u64,
    silence_duration_ms: usize,
    create_response: bool,
    interrupt_response: bool,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            prefix_padding_ms: 300,
            silence_duration_ms: 500,
            create_response: true,
            interrupt_response: true,
        }
    }
}

impl Vad {
    /// RMS level that counts as speech (threshold 0.5 is about -32 dBFS).
    fn level(&self) -> f64 {
        self.threshold * 0.05
    }

    fn json(&self) -> Value {
        json!({"type": "server_vad", "threshold": self.threshold,
               "prefix_padding_ms": self.prefix_padding_ms,
               "silence_duration_ms": self.silence_duration_ms,
               "create_response": self.create_response,
               "interrupt_response": self.interrupt_response})
    }

    fn update(&mut self, v: &Value) {
        if let Some(x) = v["threshold"].as_f64() {
            self.threshold = x.clamp(0.0, 1.0);
        }
        if let Some(x) = v["prefix_padding_ms"].as_u64() {
            self.prefix_padding_ms = x;
        }
        if let Some(x) = v["silence_duration_ms"].as_u64() {
            self.silence_duration_ms = x as usize;
        }
        if let Some(x) = v["create_response"].as_bool() {
            self.create_response = x;
        }
        if let Some(x) = v["interrupt_response"].as_bool() {
            self.interrupt_response = x;
        }
    }
}

#[derive(Debug, Clone)]
struct Settings {
    id: String,
    model: String,
    beta: bool,
    instructions: String,
    voice: String,
    text_only: bool,
    vad: Option<Vad>,
    transcription: Option<Value>,
    tools: Vec<(String, Option<Value>)>,
    tools_raw: Value,
    tool_choice: Value,
    max_output_tokens: Option<u64>,
}

impl Settings {
    fn new(model: String, beta: bool) -> Self {
        Self {
            id: prefixed_compact_id("sess_"),
            model,
            beta,
            instructions: String::new(),
            voice: "alloy".into(),
            text_only: false,
            vad: Some(Vad::default()),
            transcription: None,
            tools: Vec::new(),
            tools_raw: json!([]),
            tool_choice: json!("auto"),
            max_output_tokens: None,
        }
    }

    fn json(&self) -> Value {
        let vad = self.vad.as_ref().map_or(Value::Null, Vad::json);
        let max = self.max_output_tokens.map_or(json!("inf"), |m| json!(m));
        let expires = unix_timestamp() + 30 * 60;
        if self.beta {
            json!({
                "id": self.id, "object": "realtime.session", "model": self.model,
                "modalities": if self.text_only { json!(["text"]) } else { json!(["audio", "text"]) },
                "instructions": self.instructions, "voice": self.voice,
                "input_audio_format": "pcm16", "output_audio_format": "pcm16",
                "input_audio_transcription": self.transcription, "turn_detection": vad,
                "tools": self.tools_raw, "tool_choice": self.tool_choice,
                "temperature": 0.8, "max_response_output_tokens": max, "expires_at": expires,
            })
        } else {
            json!({
                "type": "realtime", "object": "realtime.session", "id": self.id,
                "model": self.model,
                "output_modalities": if self.text_only { json!(["text"]) } else { json!(["audio"]) },
                "instructions": self.instructions,
                "audio": {
                    "input": {"format": {"type": "audio/pcm", "rate": RATE},
                              "transcription": self.transcription, "turn_detection": vad},
                    "output": {"format": {"type": "audio/pcm", "rate": RATE},
                               "voice": self.voice, "speed": 1.0},
                },
                "tools": self.tools_raw, "tool_choice": self.tool_choice,
                "max_output_tokens": max, "expires_at": expires,
            })
        }
    }

    /// Apply a `session.update` in either shape.
    fn update(&mut self, s: &Value) {
        if let Some(i) = s["instructions"].as_str() {
            self.instructions = i.to_string();
        }
        let modalities = s
            .get("output_modalities")
            .or_else(|| s.get("modalities"))
            .and_then(Value::as_array);
        if let Some(m) = modalities {
            self.text_only = !m.iter().any(|x| x == "audio");
        }
        if let Some(v) = s["audio"]["output"]["voice"]
            .as_str()
            .or(s["voice"].as_str())
        {
            self.voice = v.to_string();
        }
        let td = s["audio"]["input"]
            .get("turn_detection")
            .or_else(|| s.get("turn_detection"));
        match td {
            Some(Value::Null) => self.vad = None,
            Some(v) if v.is_object() => {
                let mut vad = self.vad.take().unwrap_or_default();
                vad.update(v);
                self.vad = Some(vad);
            }
            _ => {}
        }
        let tr = s["audio"]["input"]
            .get("transcription")
            .or_else(|| s.get("input_audio_transcription"));
        if let Some(t) = tr {
            self.transcription = (!t.is_null()).then(|| t.clone());
        }
        if let Some(tools) = s["tools"].as_array() {
            self.tools_raw = json!(tools);
            self.tools = tools
                .iter()
                .filter_map(|t| {
                    let name = t["name"].as_str()?;
                    Some((name.to_string(), t.get("parameters").cloned()))
                })
                .collect();
        }
        if let Some(tc) = s.get("tool_choice") {
            self.tool_choice = tc.clone();
        }
        let max = s
            .get("max_output_tokens")
            .or_else(|| s.get("max_response_output_tokens"));
        if let Some(m) = max {
            self.max_output_tokens = m.as_u64();
        }
    }

    fn name(&self, ga: &'static str, beta: &'static str) -> &'static str {
        if self.beta {
            beta
        } else {
            ga
        }
    }
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// GET /openai/v1/realtime (WebSocket upgrade)
pub async fn ws_realtime(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let max = state.config.server.max_websocket_connections;
    if !state.stats.try_reserve_ws_connection(max) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "WebSocket connection limit reached",
        )
            .into_response();
    }
    let model = query
        .get("model")
        .cloned()
        .unwrap_or_else(|| "gpt-realtime".to_string());
    let beta = headers
        .get("openai-beta")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("realtime=v1"));
    ws.on_failed_upgrade({
        let state = Arc::clone(&state);
        move |_| state.stats.record_ws_disconnect()
    })
    .on_upgrade(move |socket| async move {
        run(socket, Arc::clone(&state), Settings::new(model, beta)).await;
        state.stats.record_ws_disconnect();
    })
}

type Tx = mpsc::UnboundedSender<Value>;

fn send(tx: &Tx, mut event: Value) {
    event["event_id"] = json!(event_id());
    let _ = tx.send(event);
}

fn error(tx: &Tx, kind: &str, code: &str, message: String, client_event: Option<&str>) {
    send(
        tx,
        json!({"type": "error", "error": {"type": kind, "code": code, "message": message,
                                          "param": null, "event_id": client_event}}),
    );
}

struct Conn {
    state: Arc<AppState>,
    settings: Settings,
    tx: Tx,
    /// Uncommitted input audio.
    buffer: Vec<u8>,
    /// Total audio received (ms), for `audio_start_ms` / `audio_end_ms`.
    audio_ms: usize,
    speaking: Option<usize>,
    silence_ms: usize,
    /// Conversation text, for token counts and answers.
    history: Vec<String>,
    input_audio_tokens: usize,
    last_item: Option<String>,
    active: Option<Arc<AtomicBool>>,
    /// Set by the response task when it ends.
    done: Arc<AtomicBool>,
}

async fn run(socket: WebSocket, state: Arc<AppState>, settings: Settings) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if sink
                .send(Message::Text(ev.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let mut conn = Conn {
        state,
        settings,
        tx: tx.clone(),
        buffer: Vec::new(),
        audio_ms: 0,
        speaking: None,
        silence_ms: 0,
        history: Vec::new(),
        input_audio_tokens: 0,
        last_item: None,
        active: None,
        done: Arc::new(AtomicBool::new(true)),
    };
    send(
        &tx,
        json!({"type": "session.created", "session": conn.settings.json()}),
    );
    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(event) = serde_json::from_str::<Value>(&text) else {
            error(
                &tx,
                "invalid_request_error",
                "invalid_json",
                "The server could not parse the event: invalid JSON.".into(),
                None,
            );
            continue;
        };
        conn.handle(&event);
    }
    if let Some(flag) = conn.active.take() {
        flag.store(true, Ordering::SeqCst);
    }
    drop(conn);
    drop(tx);
    let _ = writer.await;
}

impl Conn {
    fn response_active(&self) -> bool {
        !self.done.load(Ordering::SeqCst)
    }

    fn handle(&mut self, event: &Value) {
        let client_id = event["event_id"].as_str();
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "session.update" => {
                self.settings.update(&event["session"]);
                send(
                    &self.tx,
                    json!({"type": "session.updated", "session": self.settings.json()}),
                );
            }
            "input_audio_buffer.append" => {
                let Some(audio) = event["audio"].as_str().and_then(base64_decode) else {
                    error(
                        &self.tx,
                        "invalid_request_error",
                        "invalid_value",
                        "Invalid 'audio'. Expected base64-encoded audio bytes.".into(),
                        client_id,
                    );
                    return;
                };
                self.append(audio);
            }
            "input_audio_buffer.commit" => {
                let ms = self.buffer.len() / BYTES_PER_MS;
                if ms < MIN_COMMIT_MS {
                    error(
                        &self.tx,
                        "invalid_request_error",
                        "input_audio_buffer_commit_empty",
                        format!(
                            "Error committing input audio buffer: buffer too small. Expected at \
                             least {MIN_COMMIT_MS}ms of audio, but buffer only has {ms}.00ms of \
                             audio."
                        ),
                        client_id,
                    );
                    return;
                }
                self.commit(prefixed_compact_id("item_"));
            }
            "input_audio_buffer.clear" => {
                self.buffer.clear();
                self.speaking = None;
                send(&self.tx, json!({"type": "input_audio_buffer.cleared"}));
            }
            "conversation.item.create" => self.create_item(&event["item"]),
            "response.create" => {
                if self.response_active() {
                    error(
                        &self.tx,
                        "invalid_request_error",
                        "conversation_already_has_active_response",
                        "Conversation already has an active response in progress.".into(),
                        client_id,
                    );
                    return;
                }
                self.start_response(&event["response"]);
            }
            "response.cancel" => match (&self.active, self.response_active()) {
                (Some(flag), true) => flag.store(true, Ordering::SeqCst),
                _ => error(
                    &self.tx,
                    "invalid_request_error",
                    "response_cancel_not_active",
                    "Cancellation failed: no active response found.".into(),
                    client_id,
                ),
            },
            other => error(
                &self.tx,
                "invalid_request_error",
                "invalid_value",
                format!("Invalid value: '{other}'. Supported values are: 'session.update', 'input_audio_buffer.append', 'input_audio_buffer.commit', 'input_audio_buffer.clear', 'conversation.item.create', 'response.create', and 'response.cancel'."),
                client_id,
            ),
        }
    }

    /// Append audio and run server VAD over its frames.
    fn append(&mut self, audio: Vec<u8>) {
        let Some(vad) = self.settings.vad.clone() else {
            self.audio_ms += audio.len() / BYTES_PER_MS;
            self.buffer.extend(audio);
            return;
        };
        for frame in audio.chunks(FRAME_MS * BYTES_PER_MS) {
            self.buffer.extend_from_slice(frame);
            let ms = frame.len() / BYTES_PER_MS;
            let loud = rms(frame) >= vad.level();
            match (self.speaking, loud) {
                (None, true) => {
                    let item = prefixed_compact_id("item_");
                    let start = self.audio_ms.saturating_sub(vad.prefix_padding_ms as usize);
                    self.speaking = Some(start);
                    self.silence_ms = 0;
                    self.last_item = Some(item.clone());
                    send(
                        &self.tx,
                        json!({"type": "input_audio_buffer.speech_started",
                               "audio_start_ms": start, "item_id": item}),
                    );
                    if vad.interrupt_response && self.response_active() {
                        if let Some(flag) = &self.active {
                            flag.store(true, Ordering::SeqCst);
                        }
                    }
                }
                (Some(_), true) => self.silence_ms = 0,
                (Some(_), false) => {
                    self.silence_ms += ms;
                    if self.silence_ms >= vad.silence_duration_ms {
                        let item = self.last_item.clone().unwrap_or_default();
                        send(
                            &self.tx,
                            json!({"type": "input_audio_buffer.speech_stopped",
                                   "audio_end_ms": self.audio_ms + ms, "item_id": item}),
                        );
                        self.speaking = None;
                        self.commit(item);
                        if vad.create_response && !self.response_active() {
                            self.start_response(&Value::Null);
                        }
                    }
                }
                (None, false) => {}
            }
            self.audio_ms += ms;
        }
        // Without speech, keep only the padding window, as the server does.
        if self.speaking.is_none() {
            let keep = vad.prefix_padding_ms as usize * BYTES_PER_MS;
            if self.buffer.len() > keep {
                self.buffer.drain(..self.buffer.len() - keep);
            }
        }
    }

    fn item_added(&self, item: &Value) {
        let previous = self.last_item.clone();
        if self.settings.beta {
            send(
                &self.tx,
                json!({"type": "conversation.item.created", "previous_item_id": previous,
                       "item": item}),
            );
        } else {
            send(
                &self.tx,
                json!({"type": "conversation.item.added", "previous_item_id": previous,
                       "item": item}),
            );
            send(
                &self.tx,
                json!({"type": "conversation.item.done", "item": item}),
            );
        }
    }

    /// Commit the buffer as a user audio item.
    fn commit(&mut self, item_id: String) {
        let ms = self.buffer.len() / BYTES_PER_MS;
        self.buffer.clear();
        self.input_audio_tokens += ms / 100;
        let previous = self.last_item.clone().filter(|p| *p != item_id);
        send(
            &self.tx,
            json!({"type": "input_audio_buffer.committed", "previous_item_id": previous,
                   "item_id": item_id}),
        );
        let transcript = format!(
            "(simulated transcript of {:.1} s of audio)",
            ms as f64 / 1000.0
        );
        let audio_type = self.settings.name("input_audio", "input_audio");
        let item = json!({"id": item_id, "object": "realtime.item", "type": "message",
                          "status": "completed", "role": "user",
                          "content": [{"type": audio_type, "transcript": null}]});
        self.item_added(&item);
        self.last_item = Some(item_id.clone());
        self.history.push(transcript.clone());
        if self.settings.transcription.is_some() {
            send(
                &self.tx,
                json!({"type": "conversation.item.input_audio_transcription.completed",
                       "item_id": item_id, "content_index": 0, "transcript": transcript,
                       "usage": {"type": "duration", "seconds": ms as f64 / 1000.0}}),
            );
        }
    }

    fn create_item(&mut self, item: &Value) {
        let id = item["id"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| prefixed_compact_id("item_"));
        let mut stored = item.clone();
        stored["id"] = json!(id);
        stored["object"] = json!("realtime.item");
        stored["status"] = json!("completed");
        let text: String = item["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            self.history.push(text);
        }
        if let Some(out) = item["output"].as_str() {
            self.history.push(out.to_string());
        }
        self.item_added(&stored);
        self.last_item = Some(id);
    }

    fn start_response(&mut self, overrides: &Value) {
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        self.active = Some(Arc::clone(&cancel));
        self.done = Arc::clone(&done);
        let mut settings = self.settings.clone();
        if overrides.is_object() {
            settings.update(overrides);
        }
        let plan = Plan::new(
            &self.state,
            &settings,
            &self.history,
            self.input_audio_tokens,
        );
        if let Some(t) = &plan.text {
            self.history.push(t.clone());
        }
        let state = Arc::clone(&self.state);
        let tx = self.tx.clone();
        let previous = self.last_item.clone();
        self.last_item = Some(plan.item_id.clone());
        tokio::spawn(async move {
            play(&state, &settings, plan, previous, &tx, &cancel).await;
            done.store(true, Ordering::SeqCst);
        });
    }
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

struct Plan {
    response_id: String,
    item_id: String,
    text: Option<String>,
    call: Option<crate::script::SimToolCall>,
    input_text_tokens: usize,
    input_audio_tokens: usize,
}

impl Plan {
    fn new(state: &AppState, s: &Settings, history: &[String], audio_tokens: usize) -> Plan {
        let prompt = format!("{}\n{}", s.instructions, history.join("\n"));
        let input_text_tokens = crate::count_tokens_default(&prompt).unwrap_or(prompt.len() / 4);
        let seed = crate::schema_fake::seed_from(prompt.as_bytes());
        let mode = match &s.tool_choice {
            Value::String(c) if c == "required" => "ANY".to_string(),
            Value::String(c) if c == "none" => "NONE".to_string(),
            _ => "AUTO".to_string(),
        };
        let allowed: Vec<String> = s.tool_choice["name"]
            .as_str()
            .map(|n| vec![n.to_string()])
            .unwrap_or_default();
        let mode = if allowed.is_empty() {
            mode
        } else {
            "ANY".into()
        };
        let call = crate::forced::gemini(
            &s.tools,
            &mode,
            &allowed,
            state.config.response.auto_tool_calls,
            seed,
        );
        let text = call.is_none().then(|| {
            let words = s
                .max_output_tokens
                .map_or(DEFAULT_WORDS, |m| (m as usize).min(DEFAULT_WORDS));
            let generator = create_generator(&state.config.response.generator, words);
            let req = crate::openai::ChatCompletionRequest {
                model: s.model.clone(),
                messages: vec![crate::openai::Message::user(&prompt)],
                temperature: None,
                top_p: None,
                n: None,
                stream: false,
                stop: None,
                max_tokens: None,
                max_completion_tokens: None,
                presence_penalty: None,
                frequency_penalty: None,
                logit_bias: None,
                user: None,
                tools: None,
                tool_choice: None,
                response_format: None,
                seed: None,
                reasoning_effort: None,
            };
            generator
                .generate(&req)
                .split_whitespace()
                .take(words)
                .collect::<Vec<_>>()
                .join(" ")
        });
        Plan {
            response_id: prefixed_compact_id("resp_"),
            item_id: prefixed_compact_id("item_"),
            text,
            call,
            input_text_tokens,
            input_audio_tokens: audio_tokens,
        }
    }
}

fn response_obj(s: &Settings, id: &str, status: &str, output: Value, usage: Value) -> Value {
    let mut r = json!({"object": "realtime.response", "id": id, "status": status,
                       "status_details": null, "output": output, "usage": usage,
                       "conversation_id": s.id});
    if s.beta {
        r["modalities"] = if s.text_only {
            json!(["text"])
        } else {
            json!(["audio", "text"])
        };
        r["voice"] = json!(s.voice);
    } else {
        r["output_modalities"] = if s.text_only {
            json!(["text"])
        } else {
            json!(["audio"])
        };
    }
    r
}

async fn play(
    state: &AppState,
    s: &Settings,
    plan: Plan,
    previous: Option<String>,
    tx: &Tx,
    cancel: &AtomicBool,
) {
    let start = Instant::now();
    state
        .stats
        .record_request_start(&s.model, true, EndpointType::WebSocketResponses);
    let latency = state
        .config
        .latency_for_model(&s.model)
        .with_input_tokens(plan.input_text_tokens + plan.input_audio_tokens);
    let rid = plan.response_id.clone();
    let iid = plan.item_id.clone();
    send(
        tx,
        json!({"type": "response.created",
               "response": response_obj(s, &rid, "in_progress", json!([]), Value::Null)}),
    );
    let ttft = latency.sample_ttft();
    if !ttft.is_zero() {
        tokio::time::sleep(ttft).await;
    }

    let output_text_tokens;
    let mut output_audio_ms = 0usize;
    let mut cancelled = false;
    let item = if let Some(call) = &plan.call {
        let args = call.arguments.to_string();
        let call_id = call
            .id
            .clone()
            .unwrap_or_else(|| prefixed_compact_id("call_"));
        let mut item = json!({"id": iid, "object": "realtime.item", "type": "function_call",
                              "status": "in_progress", "name": call.name, "call_id": call_id,
                              "arguments": ""});
        send(
            tx,
            json!({"type": "response.output_item.added", "response_id": rid,
                   "output_index": 0, "item": item}),
        );
        send(
            tx,
            json!({"type": "response.function_call_arguments.delta", "response_id": rid,
                   "item_id": iid, "output_index": 0, "call_id": call_id, "delta": args}),
        );
        send(
            tx,
            json!({"type": "response.function_call_arguments.done", "response_id": rid,
                   "item_id": iid, "output_index": 0, "call_id": call_id, "arguments": args}),
        );
        output_text_tokens = crate::count_tokens_default(&args).unwrap_or(1);
        item["status"] = json!("completed");
        item["arguments"] = json!(args);
        item
    } else {
        let text = plan.text.clone().unwrap_or_default();
        let audio = !s.text_only;
        let part_type = match (audio, s.beta) {
            (true, true) => "audio",
            (true, false) => "output_audio",
            (false, true) => "text",
            (false, false) => "output_text",
        };
        let empty_part = if audio {
            json!({"type": part_type, "transcript": ""})
        } else {
            json!({"type": part_type, "text": ""})
        };
        let item = json!({"id": iid, "object": "realtime.item", "type": "message",
                          "status": "in_progress", "role": "assistant", "content": []});
        send(
            tx,
            json!({"type": "response.output_item.added", "response_id": rid,
                   "output_index": 0, "item": item}),
        );
        if s.beta {
            send(
                tx,
                json!({"type": "conversation.item.created", "previous_item_id": previous,
                       "item": item}),
            );
        } else {
            send(
                tx,
                json!({"type": "conversation.item.added", "previous_item_id": previous,
                       "item": item}),
            );
        }
        send(
            tx,
            json!({"type": "response.content_part.added", "response_id": rid, "item_id": iid,
                   "output_index": 0, "content_index": 0, "part": empty_part}),
        );
        let (delta_ev, done_ev) = if audio {
            (
                s.name(
                    "response.output_audio_transcript.delta",
                    "response.audio_transcript.delta",
                ),
                s.name(
                    "response.output_audio_transcript.done",
                    "response.audio_transcript.done",
                ),
            )
        } else {
            (
                s.name("response.output_text.delta", "response.text.delta"),
                s.name("response.output_text.done", "response.text.done"),
            )
        };
        let audio_ev = s.name("response.output_audio.delta", "response.audio.delta");
        let mut said = String::new();
        let mut phase = 0usize;
        let tbt = Duration::from_millis(latency.tbt_mean_ms);
        for word in crate::scenario_stream::word_tokens(&text) {
            if cancel.load(Ordering::SeqCst) {
                cancelled = true;
                break;
            }
            if !tbt.is_zero() {
                tokio::time::sleep(tbt).await;
            }
            send(
                tx,
                json!({"type": delta_ev, "response_id": rid, "item_id": iid,
                       "output_index": 0, "content_index": 0, "delta": word}),
            );
            if audio {
                let pcm = tone(WORD_MS, &mut phase);
                output_audio_ms += WORD_MS;
                send(
                    tx,
                    json!({"type": audio_ev, "response_id": rid, "item_id": iid,
                           "output_index": 0, "content_index": 0,
                           "delta": crate::imagegen::base64_encode(&pcm)}),
                );
            }
            said.push_str(&word);
        }
        output_text_tokens = crate::count_tokens_default(&said).unwrap_or(0);
        if audio {
            send(
                tx,
                json!({"type": s.name("response.output_audio.done", "response.audio.done"),
                       "response_id": rid, "item_id": iid, "output_index": 0,
                       "content_index": 0}),
            );
        }
        let key = if audio { "transcript" } else { "text" };
        let mut done = json!({"type": done_ev, "response_id": rid, "item_id": iid,
                              "output_index": 0, "content_index": 0});
        done[key] = json!(said);
        send(tx, done);
        let mut part = empty_part.clone();
        part[key] = json!(said);
        send(
            tx,
            json!({"type": "response.content_part.done", "response_id": rid, "item_id": iid,
                   "output_index": 0, "content_index": 0, "part": part}),
        );
        json!({"id": iid, "object": "realtime.item", "type": "message",
               "status": if cancelled { "incomplete" } else { "completed" },
               "role": "assistant", "content": [part]})
    };
    send(
        tx,
        json!({"type": "response.output_item.done", "response_id": rid, "output_index": 0,
               "item": item}),
    );
    if !s.beta {
        send(tx, json!({"type": "conversation.item.done", "item": item}));
    }

    let output_audio_tokens = output_audio_ms / 50;
    let input = plan.input_text_tokens + plan.input_audio_tokens;
    let output = output_text_tokens + output_audio_tokens;
    let usage = json!({
        "total_tokens": input + output, "input_tokens": input, "output_tokens": output,
        "input_token_details": {"text_tokens": plan.input_text_tokens,
                                "audio_tokens": plan.input_audio_tokens, "cached_tokens": 0},
        "output_token_details": {"text_tokens": output_text_tokens,
                                 "audio_tokens": output_audio_tokens},
    });
    let mut response = response_obj(
        s,
        &rid,
        if cancelled { "cancelled" } else { "completed" },
        json!([item]),
        usage,
    );
    if cancelled {
        response["status_details"] = json!({"type": "cancelled", "reason": "client_cancelled"});
    }
    send(tx, json!({"type": "response.done", "response": response}));
    send(
        tx,
        json!({"type": "rate_limits.updated", "rate_limits": [
            {"name": "requests", "limit": 5000, "remaining": 4999, "reset_seconds": 0.012},
            {"name": "tokens", "limit": 800000, "remaining": 800000 - (input + output).min(800000),
             "reset_seconds": 0.3}
        ]}),
    );
    state
        .stats
        .record_request_end(start.elapsed(), input as u32, output as u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"hello world"] {
            let enc = crate::imagegen::base64_encode(data);
            assert_eq!(base64_decode(&enc).unwrap(), data);
        }
        assert!(base64_decode("!!!!").is_none());
    }

    #[test]
    fn rms_separates_tone_from_silence() {
        let mut phase = 0;
        let loud = tone(20, &mut phase);
        let quiet = vec![0u8; loud.len()];
        let vad = Vad::default();
        assert!(rms(&loud) >= vad.level());
        assert!(rms(&quiet) < vad.level());
    }

    #[test]
    fn session_update_accepts_both_shapes() {
        let mut s = Settings::new("gpt-realtime".into(), false);
        s.update(&json!({"output_modalities": ["text"],
                         "audio": {"input": {"turn_detection": null}}}));
        assert!(s.text_only);
        assert!(s.vad.is_none());
        s.update(&json!({"modalities": ["audio", "text"],
                         "turn_detection": {"type": "server_vad", "silence_duration_ms": 200}}));
        assert!(!s.text_only);
        assert_eq!(s.vad.unwrap().silence_duration_ms, 200);
    }
}
