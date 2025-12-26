#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "langchain-openai>=0.3.0",
#     "httpx>=0.28.0",
# ]
# ///
"""
LangChain client example for llmsim.

This script demonstrates connecting to a running llmsim server using LangChain's
OpenAI-compatible interface. The server simulates LLM responses with realistic
latency without running actual models.

Prerequisites:
    Start the llmsim server first:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/langchain_client.py

    Or make executable and run directly:
        chmod +x examples/langchain_client.py
        ./examples/langchain_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/v1)
"""

import os
import sys

from langchain_openai import ChatOpenAI
from langchain_core.messages import HumanMessage, SystemMessage


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/v1")

    print("=" * 50)
    print("LangChain + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}")
    print()

    # Create ChatOpenAI client pointing to llmsim
    # Note: api_key is required by LangChain but llmsim doesn't validate it
    llm = ChatOpenAI(
        base_url=base_url,
        api_key="not-needed",  # llmsim doesn't require auth
        model="gpt-5",
        temperature=0.7,
        max_tokens=100,
    )

    # Example 1: Simple completion
    print("1. Simple Completion")
    print("-" * 30)
    messages = [
        SystemMessage(content="You are a helpful assistant."),
        HumanMessage(content="What is the capital of France?"),
    ]

    try:
        response = llm.invoke(messages)
        print(f"Response: {response.content}")
        print(f"Model: {response.response_metadata.get('model_name', 'unknown')}")
        if usage := response.response_metadata.get("token_usage"):
            print(f"Tokens: {usage}")
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

    for chunk in llm.stream(messages):
        print(chunk.content, end="", flush=True)
    print("\n")

    # Example 3: Different models
    print("3. Different Models")
    print("-" * 30)

    models = ["gpt-5-mini", "claude-opus-4.5", "o3-mini"]
    for model in models:
        llm_model = ChatOpenAI(
            base_url=base_url,
            api_key="not-needed",
            model=model,
            max_tokens=50,
        )
        response = llm_model.invoke([HumanMessage(content="Hello!")])
        print(f"{model}: {response.content[:60]}...")
    print()

    # Example 4: Batch processing
    print("4. Batch Processing")
    print("-" * 30)
    batch_messages = [
        [HumanMessage(content="Say 'one'")],
        [HumanMessage(content="Say 'two'")],
        [HumanMessage(content="Say 'three'")],
    ]

    responses = llm.batch(batch_messages)
    for i, resp in enumerate(responses, 1):
        print(f"  Batch {i}: {resp.content[:40]}...")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
