//! Integration tests for LLMSim server
//!
//! These tests verify end-to-end functionality including:
//! - Server startup and shutdown
//! - API endpoints
//! - Stats collection
//! - Error injection

use llmsim::cli::Config;
use llmsim::stats::Stats;
use llmsim::StatsSnapshot;
use std::time::Duration;

mod stats_tests {
    use super::*;

    #[test]
    fn test_stats_snapshot_serialization() {
        let stats = Stats::new();

        // Record some activity
        stats.record_request_start("gpt-4", false);
        stats.record_request_end(Duration::from_millis(150), 50, 100);

        stats.record_request_start("gpt-4", true);
        stats.record_request_end(Duration::from_millis(200), 30, 80);

        stats.record_request_start("claude-opus", false);
        stats.record_error(429);

        let snapshot = stats.snapshot();

        // Verify serialization works
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"total_requests\":3"));
        assert!(json.contains("\"streaming_requests\":1"));
        assert!(json.contains("\"non_streaming_requests\":2"));
        assert!(json.contains("\"total_errors\":1"));
        assert!(json.contains("\"rate_limit_errors\":1"));

        // Verify deserialization works
        let deserialized: StatsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_requests, 3);
        assert_eq!(deserialized.streaming_requests, 1);
        assert_eq!(deserialized.total_errors, 1);
    }

    #[test]
    fn test_stats_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let stats = Arc::new(Stats::new());
        let mut handles = vec![];

        // Spawn multiple threads recording stats
        for i in 0..10 {
            let stats = stats.clone();
            let model = if i % 2 == 0 { "gpt-4" } else { "claude-opus" };
            let is_streaming = i % 3 == 0;

            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    stats.record_request_start(model, is_streaming);
                    stats.record_request_end(Duration::from_millis(10), 10, 20);
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.total_requests, 1000);
        assert_eq!(snapshot.prompt_tokens, 10000);
        assert_eq!(snapshot.completion_tokens, 20000);
    }

    #[test]
    fn test_stats_model_distribution() {
        let stats = Stats::new();

        // Record requests for different models
        for _ in 0..100 {
            stats.record_request_start("gpt-4", false);
            stats.record_request_end(Duration::from_millis(10), 10, 10);
        }

        for _ in 0..50 {
            stats.record_request_start("gpt-5", true);
            stats.record_request_end(Duration::from_millis(10), 10, 10);
        }

        for _ in 0..25 {
            stats.record_request_start("claude-opus", false);
            stats.record_request_end(Duration::from_millis(10), 10, 10);
        }

        let model_requests = stats.model_requests();
        assert_eq!(model_requests.get("gpt-4"), Some(&100));
        assert_eq!(model_requests.get("gpt-5"), Some(&50));
        assert_eq!(model_requests.get("claude-opus"), Some(&25));
    }

    #[test]
    fn test_stats_latency_tracking() {
        let stats = Stats::new();

        // Record requests with varying latencies
        stats.record_request_start("gpt-4", false);
        stats.record_request_end(Duration::from_millis(100), 10, 10);

        stats.record_request_start("gpt-4", false);
        stats.record_request_end(Duration::from_millis(200), 10, 10);

        stats.record_request_start("gpt-4", false);
        stats.record_request_end(Duration::from_millis(300), 10, 10);

        // Average should be 200ms
        assert!((stats.avg_latency_ms() - 200.0).abs() < 0.1);
        assert_eq!(stats.min_latency_ms(), Some(100.0));
        assert_eq!(stats.max_latency_ms(), Some(300.0));
    }

    #[test]
    fn test_stats_error_breakdown() {
        let stats = Stats::new();

        // Record different types of errors
        for _ in 0..10 {
            stats.record_request_start("gpt-4", false);
            stats.record_error(429);
        }

        for _ in 0..5 {
            stats.record_request_start("gpt-4", false);
            stats.record_error(500);
        }

        for _ in 0..3 {
            stats.record_request_start("gpt-4", false);
            stats.record_error(504);
        }

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.total_errors, 18);
        assert_eq!(snapshot.rate_limit_errors, 10);
        assert_eq!(snapshot.server_errors, 5);
        assert_eq!(snapshot.timeout_errors, 3);
    }
}

mod config_tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.response.generator, "lorem");
        assert_eq!(config.response.target_tokens, 100);
    }

    #[test]
    fn test_config_yaml_parsing() {
        let yaml = r#"
server:
  port: 9000
  host: "127.0.0.1"

response:
  generator: "echo"
  target_tokens: 200

errors:
  rate_limit_rate: 0.1
  server_error_rate: 0.05
"#;
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.server.port, 9000);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.response.generator, "echo");
        assert_eq!(config.response.target_tokens, 200);
    }
}

mod generator_tests {
    use llmsim::generator::{LoremGenerator, ResponseGenerator};
    use llmsim::openai::{ChatCompletionRequest, Message};

    fn create_test_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message::user("Hello!")],
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
        }
    }

    #[test]
    fn test_lorem_generator_produces_output() {
        let generator = LoremGenerator::new(50);
        let request = create_test_request();
        let response = generator.generate(&request);

        assert!(!response.is_empty());
        // Should contain lorem ipsum words
        assert!(
            response.contains("Lorem")
                || response.contains("ipsum")
                || response.contains("dolor")
                || response.to_lowercase().contains("lorem")
        );
    }
}

mod stream_tests {
    use futures::StreamExt;
    use llmsim::latency::LatencyProfile;
    use llmsim::openai::Usage;
    use llmsim::TokenStreamBuilder;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_stream_on_complete_callback() {
        let callback_called = Arc::new(AtomicBool::new(false));
        let callback_clone = callback_called.clone();

        let stream = TokenStreamBuilder::new("gpt-4", "Hello world")
            .latency(LatencyProfile::instant())
            .on_complete(move || {
                callback_clone.store(true, Ordering::SeqCst);
            })
            .build();

        // Consume the stream
        let _chunks: Vec<String> = stream.into_stream().collect().await;

        // Callback should have been called
        assert!(callback_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_stream_includes_usage() {
        let usage = Usage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
        };

        let stream = TokenStreamBuilder::new("gpt-4", "Test")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Find the chunk with usage info
        let has_usage = chunks.iter().any(|c| c.contains("\"total_tokens\":30"));
        assert!(has_usage, "Stream should include usage in final chunk");
    }
}

mod openresponses_tests {
    use futures::StreamExt;
    use llmsim::latency::LatencyProfile;
    use llmsim::openresponses::{
        Input, InputMessage, MessageContent, OpenResponsesStreamBuilder, Response, ResponseRequest,
        ResponseStatus, Role, StreamEvent, StreamEventType, Usage,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_response_request_text_input() {
        let json = r#"{
            "model": "gpt-5",
            "input": "Hello, world!",
            "stream": false
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-5");
        assert!(!request.stream);
        match request.input {
            Input::Text(s) => assert_eq!(s, "Hello, world!"),
            _ => panic!("Expected text input"),
        }
    }

    #[test]
    fn test_response_request_messages_input() {
        let json = r#"{
            "model": "gpt-5",
            "input": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello!"}
            ],
            "stream": true
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "gpt-5");
        assert!(request.stream);
        match &request.input {
            Input::Messages(msgs) => {
                assert_eq!(msgs.len(), 2);
                assert_eq!(msgs[0].role, Role::System);
                assert_eq!(msgs[1].role, Role::User);
            }
            _ => panic!("Expected messages input"),
        }
    }

    #[test]
    fn test_response_creation() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            input_tokens_details: None,
            output_tokens_details: None,
        };

        let response = Response::new("gpt-5".to_string(), "Hello!".to_string(), usage);

        assert_eq!(response.model, "gpt-5");
        assert_eq!(response.status, ResponseStatus::Completed);
        assert_eq!(response.object, "response");
        assert!(response.id.starts_with("resp_"));
        assert_eq!(response.output.len(), 1);
    }

    #[test]
    fn test_response_serialization() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            input_tokens_details: None,
            output_tokens_details: None,
        };

        let response = Response::new("gpt-5".to_string(), "Test response".to_string(), usage);
        let json = serde_json::to_string(&response).unwrap();

        assert!(json.contains("\"object\":\"response\""));
        assert!(json.contains("\"model\":\"gpt-5\""));
        assert!(json.contains("\"status\":\"completed\""));
        assert!(json.contains("\"total_tokens\":30"));
    }

    #[test]
    fn test_input_extract_text() {
        // Test text input
        let text_input = Input::Text("Hello, world!".to_string());
        assert_eq!(text_input.extract_text(), "Hello, world!");

        // Test messages input
        let messages_input = Input::Messages(vec![
            InputMessage {
                role: Role::System,
                content: MessageContent::Text("You are helpful.".to_string()),
            },
            InputMessage {
                role: Role::User,
                content: MessageContent::Text("Hello!".to_string()),
            },
        ]);
        assert_eq!(messages_input.extract_text(), "You are helpful. Hello!");
    }

    #[test]
    fn test_stream_event_types() {
        let event = StreamEvent::output_text_delta(0, 0, "Hello".to_string());
        assert_eq!(event.event_type, StreamEventType::OutputTextDelta);
        assert_eq!(event.delta, Some("Hello".to_string()));
        assert_eq!(event.output_index, Some(0));
        assert_eq!(event.content_index, Some(0));
    }

    #[tokio::test]
    async fn test_openresponses_stream_basic() {
        let stream = OpenResponsesStreamBuilder::new("gpt-5", "Hello world")
            .latency(LatencyProfile::instant())
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Should have multiple events
        assert!(chunks.len() >= 6);
        assert!(chunks.last().unwrap().contains("[DONE]"));

        // Verify key events are present
        let all_text = chunks.join("");
        assert!(all_text.contains("response.created"));
        assert!(all_text.contains("response.in_progress"));
        assert!(all_text.contains("response.output_text.delta"));
        assert!(all_text.contains("response.completed"));
    }

    #[tokio::test]
    async fn test_openresponses_stream_with_usage() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            input_tokens_details: None,
            output_tokens_details: None,
        };

        let stream = OpenResponsesStreamBuilder::new("gpt-5", "Hi")
            .latency(LatencyProfile::instant())
            .usage(usage)
            .build();

        let chunks: Vec<String> = stream.into_stream().collect().await;

        // Should include usage in completed event
        let has_usage = chunks.iter().any(|c| c.contains("\"total_tokens\":15"));
        assert!(has_usage, "Stream should include usage in completed event");
    }

    #[tokio::test]
    async fn test_openresponses_stream_callback() {
        let callback_called = Arc::new(AtomicBool::new(false));
        let callback_clone = callback_called.clone();

        let stream = OpenResponsesStreamBuilder::new("gpt-5", "Test")
            .latency(LatencyProfile::instant())
            .on_complete(move || {
                callback_clone.store(true, Ordering::SeqCst);
            })
            .build();

        let _chunks: Vec<String> = stream.into_stream().collect().await;
        assert!(callback_called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_error_response() {
        let error = llmsim::openresponses::ErrorResponse::rate_limit();
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"type\":\"rate_limit_error\""));
        assert!(json.contains("\"code\":\"rate_limit_exceeded\""));
    }

    #[test]
    fn test_tool_parsing() {
        let json = r#"{
            "model": "gpt-5",
            "input": "What's the weather?",
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get weather for a location"
                    }
                }
            ]
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert!(request.tools.is_some());
        assert_eq!(request.tools.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_reasoning_config() {
        let json = r#"{
            "model": "o3",
            "input": "Solve this",
            "reasoning": {
                "effort": "high",
                "summary": "detailed"
            }
        }"#;

        let request: ResponseRequest = serde_json::from_str(json).unwrap();
        assert!(request.reasoning.is_some());
        let reasoning = request.reasoning.unwrap();
        assert_eq!(reasoning.effort, Some("high".to_string()));
        assert_eq!(reasoning.summary, Some("detailed".to_string()));
    }
}
