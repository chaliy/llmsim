#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "langchain-anthropic>=0.3.0",
#     "langchain-core>=0.3.0",
# ]
# ///
"""
LangChain + Anthropic example for llmsim.

Demonstrates using LangChain's ChatAnthropic client against a running llmsim
server. ChatAnthropic wraps the official Anthropic SDK, so pointing its base URL
at llmsim routes all traffic to the simulator.

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

Usage:
    uv run examples/anthropic_langchain.py

Environment variables:
    LLMSIM_URL: Server base URL (default: http://localhost:8080/anthropic)
"""

import os
import sys

from langchain_anthropic import ChatAnthropic
from langchain_core.messages import HumanMessage, SystemMessage


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/anthropic")

    print("=" * 50)
    print("LangChain (ChatAnthropic) + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}\n")

    # ChatAnthropic forwards base_url + api_key to the underlying Anthropic SDK.
    llm = ChatAnthropic(
        model="claude-opus-4-8",
        max_tokens=128,
        base_url=base_url,
        api_key="not-needed",
        timeout=60,
        stop=None,
    )

    # Example 1: Simple invoke
    print("1. Simple Invoke")
    print("-" * 30)
    try:
        result = llm.invoke(
            [
                SystemMessage(content="You are a helpful assistant."),
                HumanMessage(content="What is the capital of France?"),
            ]
        )
        print(f"Response: {result.content}")
        usage = result.response_metadata.get("usage", {})
        print(f"Usage: {usage}")
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
    for chunk in llm.stream([HumanMessage(content="Tell me a short story.")]):
        print(chunk.content, end="", flush=True)
    print("\n")

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
