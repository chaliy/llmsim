//! Responses API usage example for the llmsim library.
//!
//! This example demonstrates:
//! - Responses API types and structures
//! - Creating ResponsesRequest objects
//! - Generating simulated responses
//! - Streaming with the Responses API format
//!
//! Run with: cargo run --example responses_usage

use llmsim::{
    count_tokens, create_generator,
    latency::LatencyProfile,
    openai::{
        InputItem, InputRole, ItemStatus, MessageContent, OutputContentPart, OutputItem,
        OutputRole, OutputTokensDetails, ReasoningConfig, ResponsesInput, ResponsesRequest,
        ResponsesResponse, ResponsesUsage,
    },
    responses_stream::ResponsesTokenStreamBuilder,
};

use futures::StreamExt;

#[tokio::main]
async fn main() {
    println!("=== LLMSim Responses API Examples ===\n");

    // 1. Responses API Types
    println!("1. Responses API Request Types");
    println!("-------------------------------");

    // Simple text input
    let simple_request = ResponsesRequest {
        model: "gpt-5".to_string(),
        input: ResponsesInput::Text("What is the capital of France?".to_string()),
        instructions: None,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stream: false,
        metadata: None,
        previous_response_id: None,
        tools: None,
        tool_choice: None,
        reasoning: None,
    };
    println!("Simple request model: {}", simple_request.model);
    println!(
        "Simple request input: {:?}",
        match &simple_request.input {
            ResponsesInput::Text(t) => t.clone(),
            ResponsesInput::Items(_) => "Array of items".to_string(),
        }
    );

    // Message array input
    let message_request = ResponsesRequest {
        model: "gpt-5".to_string(),
        input: ResponsesInput::Items(vec![
            InputItem::Message {
                role: InputRole::System,
                content: MessageContent::Text("You are a helpful assistant.".to_string()),
            },
            InputItem::Message {
                role: InputRole::User,
                content: MessageContent::Text("Hello!".to_string()),
            },
        ]),
        instructions: Some("Be concise.".to_string()),
        temperature: Some(0.7),
        top_p: None,
        max_output_tokens: Some(100),
        stream: false,
        metadata: None,
        previous_response_id: None,
        tools: None,
        tool_choice: None,
        reasoning: None,
    };

    // Reasoning model request (o-series)
    let reasoning_request = ResponsesRequest {
        model: "o3-mini".to_string(),
        input: ResponsesInput::Text("Solve this step by step: 2 + 2 * 3".to_string()),
        instructions: None,
        temperature: None,
        top_p: None,
        max_output_tokens: Some(200),
        stream: false,
        metadata: None,
        previous_response_id: None,
        tools: None,
        tool_choice: None,
        reasoning: Some(ReasoningConfig {
            effort: Some("high".to_string()),
            summary: None,
        }),
    };
    println!(
        "Reasoning request model: {} (effort: {:?})",
        reasoning_request.model,
        reasoning_request
            .reasoning
            .as_ref()
            .and_then(|r| r.effort.as_deref())
    );
    println!(
        "Message request has instructions: {}",
        message_request.instructions.is_some()
    );
    println!(
        "Message request temperature: {:?}",
        message_request.temperature
    );
    println!();

    // 2. Response Generation
    println!("2. Response Generation");
    println!("----------------------");

    // Create a generator
    let generator = create_generator("lorem", 50);

    // Create a minimal chat request for the generator
    let chat_request = llmsim::openai::ChatCompletionRequest {
        model: "gpt-5".to_string(),
        messages: vec![llmsim::openai::Message::user("What is AI?")],
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

    let content = generator.generate(&chat_request);
    println!("Generated content: {}", content);

    // Count tokens
    let input_tokens = count_tokens("What is AI?", "gpt-5").unwrap_or(3);
    let output_tokens =
        count_tokens(&content, "gpt-5").unwrap_or(content.split_whitespace().count());
    println!("Input tokens: {}", input_tokens);
    println!("Output tokens: {}", output_tokens);
    println!();

    // 3. Responses API Response Structure
    println!("3. Responses API Response Structure");
    println!("-----------------------------------");

    let usage = ResponsesUsage {
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        total_tokens: (input_tokens + output_tokens) as u32,
        output_tokens_details: Some(OutputTokensDetails {
            reasoning_tokens: 0,
        }),
    };

    let response = ResponsesResponse::new("gpt-5".to_string(), content.clone(), usage);

    println!("Response ID: {}", response.id);
    println!("Response object: {}", response.object);
    println!("Response status: {:?}", response.status);
    println!("Response model: {}", response.model);
    println!("Output items: {}", response.output.len());
    println!("Output text: {:?}", response.output_text);

    // Examine output item
    if let Some(OutputItem::Message {
        id,
        role,
        status,
        content,
    }) = response.output.first()
    {
        println!("  Message ID: {}", id);
        println!("  Message role: {:?}", role);
        println!("  Message status: {:?}", status);
        println!("  Content parts: {}", content.len());
    }
    println!();

    // 4. Latency Profiles for Responses API
    println!("4. Latency Profiles");
    println!("-------------------");

    let profiles = [
        ("gpt-5", LatencyProfile::gpt5()),
        ("gpt-5-mini", LatencyProfile::gpt5_mini()),
        ("o3", LatencyProfile::o_series()),
        ("claude-opus-4.5", LatencyProfile::claude_opus()),
    ];

    for (name, profile) in profiles {
        println!(
            "{:18} TTFT: {:4}ms, TBT: {:2}ms",
            name, profile.ttft_mean_ms, profile.tbt_mean_ms
        );
    }

    // Auto-select from model
    let auto_profile = LatencyProfile::from_model("gpt-5");
    println!(
        "\nAuto-selected for 'gpt-5': TTFT={}ms",
        auto_profile.ttft_mean_ms
    );
    println!();

    // 5. Streaming Example
    println!("5. Responses API Streaming");
    println!("--------------------------");

    let stream_usage = ResponsesUsage {
        input_tokens: 10,
        output_tokens: 15,
        total_tokens: 25,
        output_tokens_details: Some(OutputTokensDetails {
            reasoning_tokens: 0,
        }),
    };

    let stream = ResponsesTokenStreamBuilder::new("gpt-5", "Hello! I am a simulated response.")
        .latency(LatencyProfile::fast())
        .usage(stream_usage)
        .build();

    print!("Streaming: ");
    let mut event_stream = stream.into_stream();

    let mut text_deltas = Vec::new();
    while let Some(event) = event_stream.next().await {
        // Parse the event to extract deltas
        if event.contains("output_text.delta") {
            // Extract delta text from the event
            if let Some(start) = event.find("\"delta\":\"") {
                let rest = &event[start + 9..];
                if let Some(end) = rest.find('"') {
                    let delta = &rest[..end];
                    print!("{}", delta);
                    text_deltas.push(delta.to_string());
                }
            }
        }
        // Flush to show streaming effect
        use std::io::Write;
        std::io::stdout().flush().unwrap();
    }
    println!("\n");

    // 6. Output Item Types
    println!("6. Output Item Types");
    println!("--------------------");

    // Message output
    let message_output = OutputItem::Message {
        id: "msg_123".to_string(),
        role: OutputRole::Assistant,
        status: ItemStatus::Completed,
        content: vec![OutputContentPart::OutputText {
            text: "Hello, world!".to_string(),
        }],
    };
    println!(
        "Message output: {:?}",
        serde_json::to_string(&message_output).unwrap()
    );

    // Function call output
    let function_call_output = OutputItem::FunctionCall {
        id: "fc_456".to_string(),
        call_id: "call_789".to_string(),
        name: "get_weather".to_string(),
        arguments: r#"{"location": "Paris"}"#.to_string(),
        status: ItemStatus::Completed,
    };
    println!(
        "Function call output: {:?}",
        serde_json::to_string(&function_call_output).unwrap()
    );
    println!();

    // 7. Serialization Examples
    println!("7. Serialization Examples");
    println!("-------------------------");

    // Serialize a full response
    let example_response = ResponsesResponse::new(
        "gpt-5".to_string(),
        "This is a test response.".to_string(),
        ResponsesUsage {
            input_tokens: 5,
            output_tokens: 6,
            total_tokens: 11,
            output_tokens_details: None,
        },
    );

    let json = serde_json::to_string_pretty(&example_response).unwrap();
    println!("Response JSON (truncated):");
    for line in json.lines().take(15) {
        println!("  {}", line);
    }
    println!("  ...");
    println!();

    println!("=== Examples Complete ===");
}
