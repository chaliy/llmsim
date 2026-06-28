#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "pydantic-ai-slim[openai]>=0.4.0",
# ]
# ///
"""
Pydantic AI client example for llmsim.

This script demonstrates driving a running llmsim server with
[Pydantic AI](https://ai.pydantic.dev). Pydantic AI talks to llmsim through its
OpenAI-compatible Chat Completions endpoint, so we point the OpenAI provider at
the llmsim base URL. The server simulates LLM responses with realistic latency
without running actual models.

Server endpoints:
    POST /openai/v1/chat/completions - Chat completions (streaming supported)
    GET  /openai/v1/models           - List available models

Note:
    A default llmsim server returns simulated (lorem ipsum) text, so it does not
    emit schema-conforming JSON or tool calls. To exercise Pydantic AI structured
    `output_type` results or tool calling deterministically, run the server in
    scripted mode -- see specs/scripted-mode.md and examples/scripted_demo/.

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/pydantic_ai_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import asyncio
import os
import sys

from pydantic_ai import Agent
from pydantic_ai.messages import ModelRequest, ModelResponse, TextPart, UserPromptPart
from pydantic_ai.models.openai import OpenAIChatModel
from pydantic_ai.providers.openai import OpenAIProvider


def build_model(base_url: str, model: str = "gpt-5") -> OpenAIChatModel:
    """Build a Pydantic AI model that talks to llmsim's OpenAI endpoint."""
    # api_key is required by the OpenAI client but llmsim doesn't validate it.
    provider = OpenAIProvider(base_url=base_url, api_key="not-needed")
    return OpenAIChatModel(model, provider=provider)


async def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")

    print("=" * 50)
    print("Pydantic AI + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}")
    print()

    model = build_model(base_url)
    agent = Agent(model, system_prompt="You are a helpful assistant.")

    # Example 1: Simple agent run
    print("1. Simple Agent Run")
    print("-" * 30)
    try:
        result = await agent.run("What is the capital of France?")
        print(f"Response: {result.output}")
        print(f"Usage: {result.usage}")  # token usage for the run
    except Exception as e:
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)
    print()

    # Example 2: Streaming
    print("2. Streaming Response")
    print("-" * 30)
    print("Response: ", end="", flush=True)
    async with agent.run_stream("Tell me a short story.") as stream:
        async for chunk in stream.stream_text(delta=True):
            print(chunk, end="", flush=True)
    print("\n")

    # Example 3: Multi-turn conversation (message history)
    print("3. Multi-turn Conversation")
    print("-" * 30)
    history = [
        ModelRequest(parts=[UserPromptPart(content="My name is Alice.")]),
        ModelResponse(parts=[TextPart(content="Hello Alice! Nice to meet you.")]),
    ]
    result = await agent.run("What's my name?", message_history=history)
    print(f"Response: {result.output}")
    print()

    # Example 4: Different models
    print("4. Different Models")
    print("-" * 30)
    for name in ["gpt-5-mini", "claude-opus-4.5", "o3-mini"]:
        a = Agent(build_model(base_url, name))
        result = await a.run("Hello!")
        print(f"{name}: {result.output[:60]}...")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    asyncio.run(main())
