//! Basic usage example for the llmsim library.
//!
//! This example demonstrates:
//! - Token counting with different encodings
//! - Response generators (lorem, echo, fixed)
//! - Latency profiles for simulating different models
//! - Streaming with realistic token-by-token delays
//!
//! Run with: cargo run --example basic_usage

use llmsim::{
    count_tokens, create_generator,
    latency::LatencyProfile,
    openai::{ChatCompletionRequest, Message, Usage},
    stream::TokenStreamBuilder,
    ErrorConfig, LoremGenerator, ResponseGenerator,
};

use futures::StreamExt;

#[tokio::main]
async fn main() {
    println!("=== LLMSim Library Usage Examples ===\n");

    // 1. Token Counting
    println!("1. Token Counting");
    println!("-----------------");
    let text = "Hello, world! This is a test message for token counting.";
    let tokens_gpt5 = count_tokens(text, "gpt-5").unwrap();
    let tokens_gpt4 = count_tokens(text, "gpt-4").unwrap();
    println!("Text: \"{text}\"");
    println!("  GPT-5 (o200k_base): {} tokens", tokens_gpt5);
    println!("  GPT-4 (cl100k_base): {} tokens", tokens_gpt4);
    println!();

    // 2. Response Generators
    println!("2. Response Generators");
    println!("----------------------");

    let request = ChatCompletionRequest {
        model: "gpt-5".to_string(),
        messages: vec![
            Message::system("You are a helpful assistant."),
            Message::user("What is the capital of France?"),
        ],
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
    };

    // Lorem generator - generates lorem ipsum to target token count
    let lorem = LoremGenerator::new(20);
    println!("Lorem (20 tokens): {}", lorem.generate(&request));

    // Echo generator - echoes back the user message
    let echo = create_generator("echo", 100);
    println!("Echo: {}", echo.generate(&request));

    // Fixed generator - returns a fixed response
    let fixed = create_generator("fixed:The capital of France is Paris.", 0);
    println!("Fixed: {}", fixed.generate(&request));
    println!();

    // 3. Latency Profiles
    println!("3. Latency Profiles");
    println!("-------------------");

    let profiles = [
        ("GPT-5", LatencyProfile::gpt5()),
        ("GPT-5-mini", LatencyProfile::gpt5_mini()),
        ("GPT-4", LatencyProfile::gpt4()),
        ("O-series", LatencyProfile::o_series()),
        ("Claude Opus", LatencyProfile::claude_opus()),
        ("Claude Haiku", LatencyProfile::claude_haiku()),
        ("Instant (testing)", LatencyProfile::instant()),
    ];

    for (name, profile) in profiles {
        println!(
            "{:18} TTFT: {:4}ms (±{:3}ms), TBT: {:2}ms (±{:2}ms)",
            name,
            profile.ttft_mean_ms,
            profile.ttft_stddev_ms,
            profile.tbt_mean_ms,
            profile.tbt_stddev_ms
        );
    }
    println!();

    // Auto-select profile from model name
    let auto_profile = LatencyProfile::from_model("gpt-5-mini");
    println!(
        "Auto-selected for 'gpt-5-mini': TTFT={}ms",
        auto_profile.ttft_mean_ms
    );
    println!();

    // 4. Streaming Example
    println!("4. Streaming with Latency");
    println!("-------------------------");

    let usage = Usage {
        prompt_tokens: 15,
        completion_tokens: 8,
        total_tokens: 23,
    };

    // Use fast profile for demo (instant would be too fast to see)
    let stream = TokenStreamBuilder::new("gpt-5", "Hello! I am a simulated LLM response.")
        .latency(LatencyProfile::fast())
        .usage(usage)
        .build();

    print!("Streaming: ");
    let mut chunk_stream = stream.into_chunk_stream();

    while let Some(chunk) = chunk_stream.next().await {
        if let Some(delta) = chunk.choices.first().map(|c| &c.delta) {
            if let Some(content) = &delta.content {
                print!("{content}");
                // Flush to show streaming effect
                use std::io::Write;
                std::io::stdout().flush().unwrap();
            }
        }
    }
    println!("\n");

    // 5. Error Configuration
    println!("5. Error Configuration");
    println!("----------------------");

    let default_errors = ErrorConfig::default();
    println!(
        "Default: rate_limit={:.1}%, server_error={:.2}%",
        default_errors.rate_limit_rate * 100.0,
        default_errors.server_error_rate * 100.0
    );

    let chaos = ErrorConfig::chaos();
    println!(
        "Chaos:   rate_limit={:.0}%, server_error={:.0}%",
        chaos.rate_limit_rate * 100.0,
        chaos.server_error_rate * 100.0
    );

    let no_errors = ErrorConfig::none();
    println!(
        "None:    rate_limit={:.0}%, server_error={:.0}%",
        no_errors.rate_limit_rate * 100.0,
        no_errors.server_error_rate * 100.0
    );
    println!();

    println!("=== Examples Complete ===");
}
