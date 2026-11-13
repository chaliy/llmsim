//! OpenAI Realtime API over WebSocket.

use futures_util::{SinkExt, StreamExt};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::Message};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn start(profile: Option<&str>) -> SocketAddr {
    let mut config = Config::default();
    config.latency.profile = profile.map(str::to_string);
    let app = build_router(Arc::new(AppState::new(config, new_shared_stats())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn connect(addr: SocketAddr, beta: bool) -> Ws {
    let mut req = format!("ws://{addr}/openai/v1/realtime?model=gpt-realtime")
        .into_client_request()
        .unwrap();
    if beta {
        req.headers_mut()
            .insert("OpenAI-Beta", "realtime=v1".parse().unwrap());
    }
    connect_async(req).await.unwrap().0
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

async fn next(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("event in time")
            .unwrap()
            .unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

/// Events up to and including the first of type `until`.
async fn until(ws: &mut Ws, until: &str) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        let e = next(ws).await;
        let done = e["type"] == until;
        out.push(e);
        if done {
            return out;
        }
    }
}

fn types(events: &[Value]) -> Vec<&str> {
    events.iter().map(|e| e["type"].as_str().unwrap()).collect()
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// `ms` of PCM16 at 24 kHz: a loud tone or silence.
fn pcm(ms: usize, loud: bool) -> Vec<u8> {
    (0..ms * 24)
        .flat_map(|i| {
            let s = if loud {
                ((i as f64 / 24_000.0 * 2.0 * std::f64::consts::PI * 300.0).sin() * 8000.0) as i16
            } else {
                0
            };
            s.to_le_bytes()
        })
        .collect()
}

#[tokio::test]
async fn text_turn_gets_audio_and_transcript() {
    let addr = start(Some("instant")).await;
    let mut ws = connect(addr, false).await;
    let created = next(&mut ws).await;
    assert_eq!(created["type"], "session.created");
    assert_eq!(created["session"]["type"], "realtime");
    assert_eq!(
        created["session"]["audio"]["input"]["turn_detection"]["type"],
        "server_vad"
    );

    send(
        &mut ws,
        json!({"type": "conversation.item.create", "item": {"type": "message",
        "role": "user", "content": [{"type": "input_text", "text": "Hello!"}]}}),
    )
    .await;
    send(&mut ws, json!({"type": "response.create"})).await;
    let events = until(&mut ws, "response.done").await;
    let t = types(&events);
    assert!(t.contains(&"conversation.item.added"));
    assert!(t.contains(&"response.output_audio.delta"));
    assert!(t.contains(&"response.output_audio_transcript.done"));
    let deltas: String = events
        .iter()
        .filter(|e| e["type"] == "response.output_audio_transcript.delta")
        .map(|e| e["delta"].as_str().unwrap())
        .collect();
    let done = events
        .iter()
        .find(|e| e["type"] == "response.output_audio_transcript.done")
        .unwrap();
    assert_eq!(done["transcript"], deltas);
    let resp = &events.last().unwrap()["response"];
    assert_eq!(resp["status"], "completed");
    assert!(
        resp["usage"]["output_token_details"]["audio_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(events.iter().all(|e| e["event_id"].is_string()));
    assert_eq!(next(&mut ws).await["type"], "rate_limits.updated");
}

#[tokio::test]
async fn server_vad_detects_speech_and_answers() {
    let addr = start(Some("instant")).await;
    let mut ws = connect(addr, false).await;
    next(&mut ws).await;
    send(
        &mut ws,
        json!({"type": "session.update", "session": {"type": "realtime",
        "audio": {"input": {"transcription": {"model": "gpt-4o-transcribe"}}}}}),
    )
    .await;
    assert_eq!(next(&mut ws).await["type"], "session.updated");

    let mut audio = pcm(200, false);
    audio.extend(pcm(600, true));
    audio.extend(pcm(700, false));
    send(
        &mut ws,
        json!({"type": "input_audio_buffer.append", "audio": b64(&audio)}),
    )
    .await;
    let events = until(&mut ws, "response.done").await;
    let t = types(&events);
    let pos = |name: &str| {
        t.iter()
            .position(|x| *x == name)
            .unwrap_or_else(|| panic!("{name} in {t:?}"))
    };
    assert!(pos("input_audio_buffer.speech_started") < pos("input_audio_buffer.speech_stopped"));
    assert!(pos("input_audio_buffer.speech_stopped") < pos("input_audio_buffer.committed"));
    assert!(pos("input_audio_buffer.committed") < pos("response.created"));
    assert!(t.contains(&"conversation.item.input_audio_transcription.completed"));
    let started = &events[pos("input_audio_buffer.speech_started")];
    assert_eq!(
        started["audio_start_ms"], 0,
        "200 ms speech start minus 300 ms padding"
    );
    let resp = &events.last().unwrap()["response"];
    assert!(
        resp["usage"]["input_token_details"]["audio_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[tokio::test]
async fn manual_commit_rules_and_cancel_errors() {
    let addr = start(Some("instant")).await;
    let mut ws = connect(addr, false).await;
    next(&mut ws).await;
    send(&mut ws, json!({"type": "session.update", "session": {"audio": {"input": {"turn_detection": null}}}})).await;
    let updated = next(&mut ws).await;
    assert!(updated["session"]["audio"]["input"]["turn_detection"].is_null());

    send(
        &mut ws,
        json!({"type": "input_audio_buffer.append", "audio": b64(&pcm(50, true))}),
    )
    .await;
    send(
        &mut ws,
        json!({"type": "input_audio_buffer.commit", "event_id": "c1"}),
    )
    .await;
    let e = next(&mut ws).await;
    assert_eq!(e["type"], "error");
    assert_eq!(e["error"]["code"], "input_audio_buffer_commit_empty");
    assert_eq!(e["error"]["event_id"], "c1");

    send(
        &mut ws,
        json!({"type": "input_audio_buffer.append", "audio": b64(&pcm(100, true))}),
    )
    .await;
    send(&mut ws, json!({"type": "input_audio_buffer.commit"})).await;
    assert_eq!(next(&mut ws).await["type"], "input_audio_buffer.committed");

    send(&mut ws, json!({"type": "response.cancel"})).await;
    let e = until(&mut ws, "error").await;
    assert_eq!(
        e.last().unwrap()["error"]["code"],
        "response_cancel_not_active"
    );

    send(&mut ws, json!({"type": "bogus.event"})).await;
    assert_eq!(
        until(&mut ws, "error").await.last().unwrap()["error"]["code"],
        "invalid_value"
    );
}

#[tokio::test]
async fn cancel_stops_an_answer() {
    let addr = start(None).await; // realtime profile: about 0.45 s to first audio
    let mut ws = connect(addr, false).await;
    next(&mut ws).await;
    send(&mut ws, json!({"type": "response.create"})).await;
    assert_eq!(next(&mut ws).await["type"], "response.created");
    send(&mut ws, json!({"type": "response.create"})).await;
    let e = until(&mut ws, "error").await;
    assert_eq!(
        e.last().unwrap()["error"]["code"],
        "conversation_already_has_active_response"
    );
    send(&mut ws, json!({"type": "response.cancel"})).await;
    let events = until(&mut ws, "response.done").await;
    let resp = &events.last().unwrap()["response"];
    assert_eq!(resp["status"], "cancelled");
    assert_eq!(resp["output"][0]["status"], "incomplete");
}

#[tokio::test]
async fn beta_names_text_mode_and_tools() {
    let addr = start(Some("instant")).await;
    let mut ws = connect(addr, true).await;
    let created = next(&mut ws).await;
    assert_eq!(created["session"]["modalities"], json!(["audio", "text"]));
    send(
        &mut ws,
        json!({"type": "conversation.item.create", "item": {"type": "message",
        "role": "user", "content": [{"type": "input_text", "text": "hi"}]}}),
    )
    .await;
    assert_eq!(next(&mut ws).await["type"], "conversation.item.created");
    send(&mut ws, json!({"type": "response.create"})).await;
    let t: Vec<String> = until(&mut ws, "response.done")
        .await
        .iter()
        .map(|e| e["type"].as_str().unwrap().to_string())
        .collect();
    assert!(t.contains(&"response.audio.delta".to_string()));
    assert!(t.contains(&"response.audio_transcript.delta".to_string()));

    send(
        &mut ws,
        json!({"type": "session.update", "session": {"modalities": ["text"],
        "tools": [{"type": "function", "name": "get_time", "parameters": {"type": "object",
            "properties": {"tz": {"type": "string"}}, "required": ["tz"]}}],
        "tool_choice": "required"}}),
    )
    .await;
    until(&mut ws, "session.updated").await;
    send(&mut ws, json!({"type": "response.create"})).await;
    let events = until(&mut ws, "response.done").await;
    let args = events
        .iter()
        .find(|e| e["type"] == "response.function_call_arguments.done")
        .unwrap();
    let parsed: Value = serde_json::from_str(args["arguments"].as_str().unwrap()).unwrap();
    assert!(parsed["tz"].is_string());
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["type"],
        "function_call"
    );
}
