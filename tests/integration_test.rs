//! Integration tests for LLMSim server
//!
//! These tests verify end-to-end functionality including:
//! - Server startup and shutdown
//! - API endpoints
//! - Stats collection
//! - Error injection

use llmsim::cli::Config;
use llmsim::stats::{EndpointType, Stats};
use llmsim::StatsSnapshot;
use std::time::Duration;

mod stats_tests {
    use super::*;

    #[test]
    fn test_stats_snapshot_serialization() {
        let stats = Stats::new();

        // Record some activity
        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_end(Duration::from_millis(150), 50, 100);

        stats.record_request_start("gpt-4", true, EndpointType::ChatCompletions);
        stats.record_request_end(Duration::from_millis(200), 30, 80);

        stats.record_request_start("claude-opus", false, EndpointType::Responses);
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
                    stats.record_request_start(model, is_streaming, EndpointType::ChatCompletions);
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
            stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
            stats.record_request_end(Duration::from_millis(10), 10, 10);
        }

        for _ in 0..50 {
            stats.record_request_start("gpt-5", true, EndpointType::Responses);
            stats.record_request_end(Duration::from_millis(10), 10, 10);
        }

        for _ in 0..25 {
            stats.record_request_start("claude-opus", false, EndpointType::ChatCompletions);
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
        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_end(Duration::from_millis(100), 10, 10);

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_end(Duration::from_millis(200), 10, 10);

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
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
            stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
            stats.record_error(429);
        }

        for _ in 0..5 {
            stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
            stats.record_error(500);
        }

        for _ in 0..3 {
            stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
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
        // Response should end with a period
        assert!(response.ends_with('.'));
        // Response should have reasonable length (at least a few words)
        assert!(response.split_whitespace().count() >= 5);
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
