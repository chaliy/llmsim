// WebSocket mode integration tests.
// Tests the WebSocket transport for the Responses API.

use futures::{SinkExt, StreamExt};
use llmsim::cli::{build_router, AppState, Config};
use llmsim::stats::new_shared_stats;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Start a test server and return the bound address.
async fn start_server() -> SocketAddr {
    let config = Config::default();
    let stats = new_shared_stats();
    let state = Arc::new(AppState::new(config, stats));
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    addr
}

/// Connect to the WebSocket endpoint and split into sink/stream.
async fn ws_connect(
    addr: SocketAddr,
) -> (
    futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) {
    let url = format!("ws://{}/openai/v1/responses", addr);
    let (ws, _) = connect_async(url).await.expect("Failed to connect");
    ws.split()
}

/// Send a response.create event and collect all server events until response.completed.
async fn send_and_collect(
    sink: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    stream: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    model: &str,
    input: &str,
) -> Vec<Value> {
    let request = serde_json::json!({
        "type": "response.create",
        "response": {
            "model": model,
            "input": input
        }
    });

    sink.send(Message::Text(request.to_string().into()))
        .await
        .unwrap();

    let mut events = Vec::new();
    let timeout = tokio::time::Duration::from_secs(30);

    loop {
        match tokio::time::timeout(timeout, stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let event: Value = serde_json::from_str(&text).unwrap();
                let event_type = event["type"].as_str().unwrap_or("").to_string();
                events.push(event);
                if event_type == "response.completed" {
                    break;
                }
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("WebSocket error: {}", e),
            Ok(None) => panic!("WebSocket closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for response events"),
        }
    }

    events
}

#[tokio::test]
async fn test_ws_basic_response() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;
    let events = send_and_collect(&mut sink, &mut stream, "gpt-5", "Hello!").await;

    // Should have multiple events
    assert!(
        events.len() >= 5,
        "Expected at least 5 events, got {}",
        events.len()
    );

    // First event should be response.created
    assert_eq!(events[0]["type"], "response.created");
    assert!(events[0]["response"]["id"]
        .as_str()
        .unwrap()
        .starts_with("resp_"));

    // Last event should be response.completed
    let last = events.last().unwrap();
    assert_eq!(last["type"], "response.completed");
    assert_eq!(last["response"]["status"], "completed");

    // Should have usage info
    assert!(last["response"]["usage"]["input_tokens"].as_u64().unwrap() > 0);
    assert!(last["response"]["usage"]["output_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_ws_event_sequence() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;
    let events = send_and_collect(&mut sink, &mut stream, "gpt-4o", "Hi").await;

    let types: Vec<String> = events
        .iter()
        .map(|e| e["type"].as_str().unwrap_or("").to_string())
        .collect();

    // Verify expected event ordering
    assert_eq!(types[0], "response.created");
    assert_eq!(types[1], "response.in_progress");

    // Should contain delta events
    assert!(
        types.iter().any(|t| t == "response.output_text.delta"),
        "Should have text delta events"
    );

    // Should end with response.completed
    assert_eq!(types.last().unwrap(), "response.completed");
}

#[tokio::test]
async fn test_ws_multiple_requests_sequential() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;

    // Send first request
    let events1 = send_and_collect(&mut sink, &mut stream, "gpt-5", "First").await;
    assert_eq!(events1.last().unwrap()["type"], "response.completed");

    // Send second request on same connection
    let events2 = send_and_collect(&mut sink, &mut stream, "gpt-5", "Second").await;
    assert_eq!(events2.last().unwrap()["type"], "response.completed");

    // Both should have different response IDs
    let id1 = events1.last().unwrap()["response"]["id"].as_str().unwrap();
    let id2 = events2.last().unwrap()["response"]["id"].as_str().unwrap();
    assert_ne!(id1, id2);
}

#[tokio::test]
async fn test_ws_previous_response_id_cached() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;

    // First request
    let events1 = send_and_collect(&mut sink, &mut stream, "gpt-5", "Hello").await;
    let resp_id = events1.last().unwrap()["response"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Second request referencing the cached response ID
    let request = serde_json::json!({
        "type": "response.create",
        "response": {
            "model": "gpt-5",
            "input": "Follow up",
            "previous_response_id": resp_id
        }
    });

    sink.send(Message::Text(request.to_string().into()))
        .await
        .unwrap();

    let mut events2 = Vec::new();
    let timeout = tokio::time::Duration::from_secs(30);
    loop {
        match tokio::time::timeout(timeout, stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let event: Value = serde_json::from_str(&text).unwrap();
                let event_type = event["type"].as_str().unwrap_or("").to_string();
                events2.push(event);
                if event_type == "response.completed" || event_type == "error" {
                    break;
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => panic!("Unexpected WS state"),
        }
    }

    // Should succeed (response.created, not an error)
    assert_eq!(events2[0]["type"], "response.created");
}

#[tokio::test]
async fn test_ws_previous_response_id_not_found() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;

    // Request with a non-existent previous_response_id
    let request = serde_json::json!({
        "type": "response.create",
        "response": {
            "model": "gpt-5",
            "input": "Hello",
            "previous_response_id": "resp_nonexistent"
        }
    });

    sink.send(Message::Text(request.to_string().into()))
        .await
        .unwrap();

    let timeout = tokio::time::Duration::from_secs(5);
    match tokio::time::timeout(timeout, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let event: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(event["type"], "error");
            assert!(event["error"]["type"]
                .as_str()
                .unwrap()
                .contains("previous_response_not_found"));
        }
        other => panic!("Expected error event, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_ws_invalid_message() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;

    // Send invalid JSON
    sink.send(Message::Text("not valid json".into()))
        .await
        .unwrap();

    let timeout = tokio::time::Duration::from_secs(5);
    match tokio::time::timeout(timeout, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let event: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(event["type"], "error");
            assert!(event["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Failed to parse"));
        }
        other => panic!("Expected error event, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_ws_json_frames_not_sse() {
    let addr = start_server().await;
    let (mut sink, mut stream) = ws_connect(addr).await;

    let request = serde_json::json!({
        "type": "response.create",
        "response": {
            "model": "gpt-5",
            "input": "Hello"
        }
    });

    sink.send(Message::Text(request.to_string().into()))
        .await
        .unwrap();

    let timeout = tokio::time::Duration::from_secs(30);
    match tokio::time::timeout(timeout, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            // Should be valid JSON (not SSE format with "event:" prefix)
            assert!(
                !text.contains("event:"),
                "WS frames should not have SSE envelope"
            );
            let event: Value = serde_json::from_str(&text).unwrap();
            assert!(event["type"].is_string());
        }
        other => panic!("Expected text frame, got: {:?}", other),
    }
}
