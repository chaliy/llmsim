#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "httpx>=0.27.0",
# ]
# ///
"""
OpenAI Responses API client example for llmsim.

This script demonstrates connecting to a running llmsim server using the
OpenAI Responses API format. The Responses API is OpenAI's newer stateful API
that unifies Chat Completions and Assistants capabilities.

Server endpoints:
    POST /openai/v1/responses - Create a response (streaming and non-streaming)

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/responses_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080)
"""

import json
import os
import sys
from typing import Any, Iterator

import httpx


def create_response(
    client: httpx.Client,
    model: str,
    input_data: str | list[dict[str, Any]],
    stream: bool = False,
    **kwargs: Any,
) -> dict[str, Any] | Iterator[dict[str, Any]]:
    """Create a response using the Responses API."""
    payload = {
        "model": model,
        "input": input_data,
        "stream": stream,
        **kwargs,
    }

    if stream:
        return _stream_response(client, payload)
    else:
        response = client.post("/openai/v1/responses", json=payload)
        response.raise_for_status()
        return response.json()


def _stream_response(
    client: httpx.Client, payload: dict[str, Any]
) -> Iterator[dict[str, Any]]:
    """Stream a response and yield parsed events."""
    with client.stream("POST", "/openai/v1/responses", json=payload) as response:
        response.raise_for_status()

        buffer = ""
        for chunk in response.iter_text():
            buffer += chunk

            while "\n\n" in buffer:
                event_str, buffer = buffer.split("\n\n", 1)
                if not event_str.strip():
                    continue

                event = {}
                for line in event_str.strip().split("\n"):
                    if line.startswith("event: "):
                        event["event"] = line[7:]
                    elif line.startswith("data: "):
                        try:
                            event["data"] = json.loads(line[6:])
                        except json.JSONDecodeError:
                            event["data"] = line[6:]

                if event:
                    yield event


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080")

    print("=" * 60)
    print("OpenAI Responses API + LLMSim Example")
    print("=" * 60)
    print(f"\nConnecting to: {base_url}")
    print()

    client = httpx.Client(base_url=base_url, timeout=60.0)

    # Example 1: Simple text input
    print("1. Simple Text Input")
    print("-" * 40)
    try:
        response = create_response(
            client,
            model="gpt-5",
            input_data="What is the capital of France?",
        )
        print(f"Response ID: {response['id']}")
        print(f"Status: {response['status']}")
        print(f"Output: {response.get('output_text', 'N/A')}")
        if response.get("usage"):
            print(f"Tokens: {response['usage']}")
    except httpx.HTTPStatusError as e:
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)
    print()

    # Example 2: Message array input
    print("2. Message Array Input")
    print("-" * 40)
    response = create_response(
        client,
        model="gpt-5",
        input_data=[
            {"type": "message", "role": "system", "content": "You are a helpful assistant."},
            {"type": "message", "role": "user", "content": "Tell me a joke."},
        ],
    )
    print(f"Response ID: {response['id']}")
    print(f"Output: {response.get('output_text', 'N/A')}")
    print()

    # Example 3: With instructions
    print("3. With Instructions")
    print("-" * 40)
    response = create_response(
        client,
        model="gpt-5",
        input_data="Write something creative.",
        instructions="You are a creative writing assistant. Be poetic and imaginative.",
    )
    print(f"Output: {response.get('output_text', 'N/A')}")
    print()

    # Example 4: Different models
    print("4. Different Models")
    print("-" * 40)
    models = ["gpt-5-mini", "claude-opus-4.5", "o3-mini"]
    for model in models:
        response = create_response(
            client,
            model=model,
            input_data="Hello!",
        )
        output = response.get("output_text", "")
        print(f"{model}: {output[:60]}...")
    print()

    # Example 5: Streaming response
    print("5. Streaming Response")
    print("-" * 40)
    print("Response: ", end="", flush=True)

    for event in create_response(
        client,
        model="gpt-5",
        input_data="Tell me a short story.",
        stream=True,
    ):
        event_type = event.get("event", "")
        data = event.get("data", {})

        if event_type == "response.output_text.delta":
            print(data.get("delta", ""), end="", flush=True)
        elif event_type == "response.completed":
            usage = data.get("response", {}).get("usage", {})
            print(f"\n\nTokens: {usage}")
    print()

    # Example 6: Examine full response structure
    print("6. Full Response Structure")
    print("-" * 40)
    response = create_response(
        client,
        model="gpt-5",
        input_data="Hello, world!",
    )
    print(f"ID: {response['id']}")
    print(f"Object: {response['object']}")
    print(f"Model: {response['model']}")
    print(f"Status: {response['status']}")
    print(f"Output items: {len(response.get('output', []))}")
    if response.get("output"):
        item = response["output"][0]
        print(f"  - Type: {item.get('type')}")
        print(f"  - Role: {item.get('role')}")
        print(f"  - Status: {item.get('status')}")
    print()

    print("=" * 60)
    print("Examples complete!")
    print("=" * 60)


if __name__ == "__main__":
    main()
