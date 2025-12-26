#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "openai>=1.60.0",
# ]
# ///
"""
OpenAI client example for llmsim.

This script demonstrates connecting to a running llmsim server using the
official OpenAI Python library. The server simulates LLM responses with
realistic latency without running actual models.

Prerequisites:
    Start the llmsim server first:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/openai_client.py

    Or make executable and run directly:
        chmod +x examples/openai_client.py
        ./examples/openai_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/v1)
"""

import os
import sys

from openai import OpenAI


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/v1")

    print("=" * 50)
    print("OpenAI SDK + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}")
    print()

    # Create OpenAI client pointing to llmsim
    client = OpenAI(
        base_url=base_url,
        api_key="not-needed",  # llmsim doesn't require auth
    )

    # Example 1: Simple completion
    print("1. Simple Completion")
    print("-" * 30)
    try:
        response = client.chat.completions.create(
            model="gpt-5",
            messages=[
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "What is the capital of France?"},
            ],
            max_tokens=100,
        )
        print(f"Response: {response.choices[0].message.content}")
        print(f"Model: {response.model}")
        print(f"Tokens: {response.usage}")
    except Exception as e:
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  cargo run --release -- serve --port 8080")
        sys.exit(1)
    print()

    # Example 2: Streaming
    print("2. Streaming Response")
    print("-" * 30)
    print("Response: ", end="", flush=True)

    stream = client.chat.completions.create(
        model="gpt-5",
        messages=[
            {"role": "user", "content": "Tell me a short story."},
        ],
        max_tokens=100,
        stream=True,
    )

    for chunk in stream:
        if chunk.choices[0].delta.content:
            print(chunk.choices[0].delta.content, end="", flush=True)
    print("\n")

    # Example 3: Different models
    print("3. Different Models")
    print("-" * 30)

    models = ["gpt-5-mini", "claude-opus-4.5", "o3-mini"]
    for model in models:
        response = client.chat.completions.create(
            model=model,
            messages=[{"role": "user", "content": "Hello!"}],
            max_tokens=50,
        )
        content = response.choices[0].message.content or ""
        print(f"{model}: {content[:60]}...")
    print()

    # Example 4: List available models
    print("4. Available Models")
    print("-" * 30)
    models_list = client.models.list()
    for model in list(models_list)[:5]:
        print(f"  - {model.id} (owned by: {model.owned_by})")
    print(f"  ... and {len(list(models_list)) - 5} more")
    print()

    # Example 5: Multiple messages (conversation)
    print("5. Multi-turn Conversation")
    print("-" * 30)
    response = client.chat.completions.create(
        model="gpt-5",
        messages=[
            {"role": "system", "content": "You are a helpful assistant."},
            {"role": "user", "content": "My name is Alice."},
            {"role": "assistant", "content": "Hello Alice! Nice to meet you."},
            {"role": "user", "content": "What's my name?"},
        ],
        max_tokens=50,
    )
    print(f"Response: {response.choices[0].message.content}")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
