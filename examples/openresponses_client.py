#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "httpx>=0.28.0",
# ]
# ///
"""
OpenResponses client example for llmsim.

This script demonstrates connecting to a running llmsim server using the
OpenResponses API specification (https://www.openresponses.org).

The OpenResponses API is an open-source specification for building multi-provider,
interoperable LLM interfaces. It provides a standardized way to interact with
different LLM providers.

Server endpoints:
    POST /openresponses/v1/responses - Create response (streaming supported)

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/openresponses_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080)
"""

import os
import sys
import json

import httpx


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080")
    endpoint = f"{base_url}/openresponses/v1/responses"

    print("=" * 50)
    print("OpenResponses API + LLMSim Example")
    print("=" * 50)
    print(f"\nEndpoint: {endpoint}")
    print("Spec: https://www.openresponses.org")
    print()

    # Example 1: Simple text input (non-streaming)
    print("1. Simple Text Input")
    print("-" * 30)
    try:
        response = httpx.post(
            endpoint,
            json={
                "model": "gpt-5",
                "input": "What is the capital of France?",
                "stream": False,
            },
            timeout=30.0,
        )
        response.raise_for_status()
        data = response.json()
        print(f"Response ID: {data['id']}")
        print(f"Status: {data['status']}")
        print(f"Model: {data['model']}")
        if data.get("output"):
            for item in data["output"]:
                if item["type"] == "message":
                    for content in item.get("content", []):
                        if content["type"] == "output_text":
                            print(f"Content: {content['text'][:100]}...")
        if data.get("usage"):
            print(f"Usage: {data['usage']}")
    except httpx.ConnectError:
        print("Error: Could not connect to server")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)
    except Exception as e:
        print(f"Error: {e}")
        sys.exit(1)
    print()

    # Example 2: Message-based input
    print("2. Message-based Input")
    print("-" * 30)
    response = httpx.post(
        endpoint,
        json={
            "model": "gpt-5",
            "input": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "Hello! What can you help me with?"},
            ],
            "stream": False,
        },
        timeout=30.0,
    )
    data = response.json()
    print(f"Response ID: {data['id']}")
    if data.get("output"):
        for item in data["output"]:
            if item["type"] == "message":
                for content in item.get("content", []):
                    if content["type"] == "output_text":
                        print(f"Content: {content['text'][:100]}...")
    print()

    # Example 3: Streaming response
    print("3. Streaming Response")
    print("-" * 30)
    print("Content: ", end="", flush=True)

    with httpx.stream(
        "POST",
        endpoint,
        json={
            "model": "gpt-5",
            "input": "Tell me a short story about a robot.",
            "stream": True,
        },
        timeout=60.0,
    ) as response:
        for line in response.iter_lines():
            if line.startswith("data: "):
                data_str = line[6:]  # Remove "data: " prefix
                if data_str == "[DONE]":
                    break
                try:
                    event = json.loads(data_str)
                    # Print delta content as it streams
                    if event.get("type") == "response.output_text.delta":
                        if delta := event.get("delta"):
                            print(delta, end="", flush=True)
                except json.JSONDecodeError:
                    pass
    print("\n")

    # Example 4: Different models
    print("4. Different Models")
    print("-" * 30)

    models = ["gpt-5-mini", "claude-opus-4.5", "o3-mini"]
    for model in models:
        response = httpx.post(
            endpoint,
            json={
                "model": model,
                "input": "Hello!",
                "stream": False,
            },
            timeout=30.0,
        )
        data = response.json()
        content = ""
        if data.get("output"):
            for item in data["output"]:
                if item["type"] == "message":
                    for c in item.get("content", []):
                        if c["type"] == "output_text":
                            content = c["text"]
                            break
        print(f"{model}: {content[:60]}...")
    print()

    # Example 5: With metadata
    print("5. With Metadata")
    print("-" * 30)
    response = httpx.post(
        endpoint,
        json={
            "model": "gpt-5",
            "input": "What time is it?",
            "stream": False,
            "metadata": {
                "user_id": "test-user-123",
                "session_id": "session-456",
            },
        },
        timeout=30.0,
    )
    data = response.json()
    print(f"Response ID: {data['id']}")
    print(f"Object: {data['object']}")
    print(f"Created at: {data['created_at']}")
    print()

    # Example 6: With reasoning config (for o-series models)
    print("6. Reasoning Config (o-series)")
    print("-" * 30)
    response = httpx.post(
        endpoint,
        json={
            "model": "o3",
            "input": "What is 2 + 2?",
            "stream": False,
            "reasoning": {
                "effort": "low",
            },
        },
        timeout=30.0,
    )
    data = response.json()
    print(f"Model: {data['model']}")
    if data.get("output"):
        for item in data["output"]:
            if item["type"] == "message":
                for content in item.get("content", []):
                    if content["type"] == "output_text":
                        print(f"Content: {content['text'][:100]}...")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
