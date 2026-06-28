#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "anthropic>=0.40.0",
# ]
# ///
"""
Anthropic client example for llmsim.

Demonstrates connecting to a running llmsim server using the official Anthropic
Python SDK. The server simulates Claude responses with realistic latency
without running actual models.

Server endpoints:
    POST /anthropic/v1/messages    - Messages API (streaming supported)
    GET  /anthropic/v1/models      - List available Claude models
    GET  /anthropic/v1/models/:id  - Get model details

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/anthropic_client.py

Environment variables:
    LLMSIM_URL: Server base URL (default: http://localhost:8080/anthropic)
"""

import os
import sys

from anthropic import Anthropic


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/anthropic")

    print("=" * 50)
    print("Anthropic SDK + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}\n")

    # Point the official Anthropic client at llmsim. The simulator ignores the
    # API key, but the SDK requires one to be set.
    client = Anthropic(base_url=base_url, api_key="not-needed")

    # Example 1: Simple message
    print("1. Simple Message")
    print("-" * 30)
    try:
        msg = client.messages.create(
            model="claude-opus-4-8",
            max_tokens=128,
            system="You are a helpful assistant.",
            messages=[{"role": "user", "content": "What is the capital of France?"}],
        )
        print(f"Response: {msg.content[0].text}")
        print(f"Model: {msg.model} | stop_reason: {msg.stop_reason}")
        print(f"Tokens: in={msg.usage.input_tokens} out={msg.usage.output_tokens}")
    except Exception as e:  # noqa: BLE001
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)
    print()

    # Example 2: Streaming
    print("2. Streaming Response")
    print("-" * 30)
    print("Response: ", end="", flush=True)
    with client.messages.stream(
        model="claude-haiku-4-5",
        max_tokens=128,
        messages=[{"role": "user", "content": "Tell me a short story."}],
    ) as stream:
        for text in stream.text_stream:
            print(text, end="", flush=True)
        final = stream.get_final_message()
    print(f"\n(streamed, output tokens: {final.usage.output_tokens})\n")

    # Example 3: Multi-turn conversation (the API is stateless; resend history)
    print("3. Multi-turn Conversation")
    print("-" * 30)
    msg = client.messages.create(
        model="claude-sonnet-4-6",
        max_tokens=64,
        messages=[
            {"role": "user", "content": "My name is Ada."},
            {"role": "assistant", "content": "Hello Ada! Nice to meet you."},
            {"role": "user", "content": "What is my name?"},
        ],
    )
    print(f"Response: {msg.content[0].text[:80]}\n")

    # Example 4: Multiple models
    print("4. Different Claude Models")
    print("-" * 30)
    for model in ["claude-fable-5", "claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"]:
        m = client.messages.create(
            model=model,
            max_tokens=48,
            messages=[{"role": "user", "content": "Hello!"}],
        )
        print(f"{model}: {m.content[0].text[:50]}...")
    print()

    # Example 5: Tool definitions (the model only emits tool_use blocks in
    # scripted mode; here we show that tools are accepted by the API).
    print("5. Tool Use (request shape)")
    print("-" * 30)
    msg = client.messages.create(
        model="claude-opus-4-8",
        max_tokens=128,
        tools=[
            {
                "name": "get_weather",
                "description": "Get the current weather in a location",
                "input_schema": {
                    "type": "object",
                    "properties": {"location": {"type": "string"}},
                    "required": ["location"],
                },
            }
        ],
        messages=[{"role": "user", "content": "What is the weather in Paris?"}],
    )
    print(f"stop_reason: {msg.stop_reason}")
    for block in msg.content:
        print(f"  block type: {block.type}")
    print("  (run the server with a script to get deterministic tool_use blocks)")
    print()

    # Example 6: List and retrieve models
    print("6. Available Models")
    print("-" * 30)
    models = client.models.list()
    for m in models.data[:6]:
        print(f"  - {m.id} ({m.display_name})")
    print(f"  ... and {max(0, len(models.data) - 6)} more")
    one = client.models.retrieve("claude-opus-4-8")
    print(f"  retrieve: {one.id} created_at={one.created_at}")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
