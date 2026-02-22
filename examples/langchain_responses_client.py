#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "langchain-openai>=0.3.28",
# ]
# ///
"""
LangChain Responses API client example for llmsim.

This script demonstrates using LangChain's ChatOpenAI with the OpenAI Responses
API, including reasoning/thinking support. LangChain >= 0.3.9 supports
`use_responses_api=True` and >= 0.3.28 supports `reasoning_effort`.

Server endpoints:
    POST /openai/v1/responses - Responses API (streaming supported)

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/langchain_responses_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import os
import sys

from langchain_openai import ChatOpenAI
from langchain_core.messages import HumanMessage


def extract_text(response) -> str:
    """Extract plain text from a Responses API response."""
    for block in getattr(response, "content_blocks", []):
        if isinstance(block, dict) and block.get("type") == "text":
            return block["text"]
    # Fallback for string content
    if isinstance(response.content, str):
        return response.content
    return str(response.content)


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")

    print("=" * 60)
    print("LangChain Responses API + LLMSim Example")
    print("=" * 60)
    print(f"\nConnecting to: {base_url}")
    print()

    # Example 1: Basic Responses API usage
    print("1. Basic Responses API")
    print("-" * 40)
    llm = ChatOpenAI(
        base_url=base_url,
        api_key="not-needed",
        model="gpt-4o",
        use_responses_api=True,
        max_tokens=100,
    )

    try:
        response = llm.invoke([HumanMessage(content="What is the capital of France?")])
        print(f"Response: {extract_text(response)[:120]}")
        print(f"Model: {response.response_metadata.get('model', 'unknown')}")
        usage = response.usage_metadata
        print(f"Tokens: prompt={usage['input_tokens']}, "
              f"completion={usage['output_tokens']}, "
              f"total={usage['total_tokens']}")
    except Exception as e:
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)
    print()

    # Example 2: Responses API with streaming
    print("2. Streaming (Responses API)")
    print("-" * 40)
    print("Response: ", end="", flush=True)
    for chunk in llm.stream([HumanMessage(content="Tell me a short story.")]):
        # In streaming mode, each chunk has content_blocks with text deltas
        for block in getattr(chunk, "content_blocks", []):
            if isinstance(block, dict) and block.get("type") == "text":
                print(block["text"], end="", flush=True)
    print("\n")

    # Example 3: Reasoning / Thinking with summary
    # Use the `reasoning` parameter to configure both effort and summary.
    print("3. Reasoning / Thinking (non-streaming)")
    print("-" * 40)
    reasoning_llm = ChatOpenAI(
        base_url=base_url,
        api_key="not-needed",
        model="o3",
        use_responses_api=True,
        reasoning={"effort": "medium", "summary": "auto"},
        max_tokens=None,
    )

    response = reasoning_llm.invoke([HumanMessage(content="What is 15 * 37?")])
    for block in response.content_blocks:
        if isinstance(block, dict) and block.get("type") == "reasoning":
            summary = block.get("reasoning", "")
            print(f"[Thinking] {summary[:200]}{'...' if len(summary) > 200 else ''}")
        elif isinstance(block, dict) and block.get("type") == "text":
            print(f"[Response] {block['text'][:120]}")

    usage = response.usage_metadata
    reasoning_tokens = usage.get("output_token_details", {}).get("reasoning", 0)
    print(f"Reasoning tokens: {reasoning_tokens}, Total: {usage['total_tokens']}")
    print()

    # Example 4: Compare reasoning effort levels
    print("4. Reasoning Effort Levels")
    print("-" * 40)
    for effort in ["low", "medium", "high"]:
        effort_llm = ChatOpenAI(
            base_url=base_url,
            api_key="not-needed",
            model="o3",
            use_responses_api=True,
            reasoning={"effort": effort, "summary": "auto"},
            max_tokens=None,
        )
        resp = effort_llm.invoke([HumanMessage(content="Solve: 2+2")])
        usage = resp.usage_metadata
        reasoning_tok = usage.get("output_token_details", {}).get("reasoning", 0)
        print(f"  effort={effort:6s}  reasoning_tokens={reasoning_tok:5d}  "
              f"total={usage['total_tokens']}")
    print()

    # Example 5: Streaming with thinking visualization
    print("5. Streaming with Thinking")
    print("-" * 40)
    streaming_llm = ChatOpenAI(
        base_url=base_url,
        api_key="not-needed",
        model="o3",
        use_responses_api=True,
        reasoning={"effort": "low", "summary": "concise"},
        max_tokens=None,
    )

    in_thinking = False
    in_response = False
    for chunk in streaming_llm.stream(
        [HumanMessage(content="Explain why 1+1=2")]
    ):
        for block in getattr(chunk, "content_blocks", []):
            if not isinstance(block, dict):
                continue
            if block.get("type") == "reasoning":
                if not in_thinking:
                    print("[Thinking] ", end="", flush=True)
                    in_thinking = True
                delta = block.get("reasoning", "")
                if delta:
                    print(delta, end="", flush=True)
            elif block.get("type") == "text":
                if not in_response:
                    if in_thinking:
                        print()
                    print("[Response] ", end="", flush=True)
                    in_response = True
                delta = block.get("text", "")
                if delta:
                    print(delta, end="", flush=True)
    print("\n")

    # Example 6: Different models with Responses API
    print("6. Different Models (Responses API)")
    print("-" * 40)
    for model in ["gpt-4o", "gpt-4o-mini", "o3-mini"]:
        model_llm = ChatOpenAI(
            base_url=base_url,
            api_key="not-needed",
            model=model,
            use_responses_api=True,
            max_tokens=50,
        )
        resp = model_llm.invoke([HumanMessage(content="Hello!")])
        print(f"  {model}: {extract_text(resp)[:60]}...")
    print()

    print("=" * 60)
    print("Examples complete!")
    print("=" * 60)


if __name__ == "__main__":
    main()
