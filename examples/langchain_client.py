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

Server endpoints:
    POST /openai/v1/chat/completions - Chat completions (streaming supported)
    POST /openai/v1/responses        - Responses API
    GET  /openai/v1/models           - List available models
    GET  /openai/v1/models/:id       - Get model details

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/langchain_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import json
import os
import sys

import httpx
from langchain_openai import ChatOpenAI
from langchain_core.messages import HumanMessage, SystemMessage


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")

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
        print("  llmsim serve --port 8080")
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

    # Example 5: Reasoning / Thinking with Responses API
    # LangChain's ChatOpenAI uses the Chat Completions API, which doesn't support
    # reasoning output items. We use httpx to call the Responses API directly.
    print("5. Reasoning / Thinking (Responses API)")
    print("-" * 30)

    responses_url = base_url.replace("/openai/v1", "")
    with httpx.Client(base_url=responses_url, timeout=60.0) as http_client:
        resp = http_client.post(
            "/openai/v1/responses",
            json={
                "model": "o3",
                "input": "What is 15 * 37?",
                "reasoning": {"effort": "medium", "summary": "auto"},
            },
        )
        data = resp.json()
        for item in data.get("output", []):
            if item["type"] == "reasoning":
                print("[Thinking]")
                if item.get("summary"):
                    for s in item["summary"]:
                        print(f"  {s['text']}")
            elif item["type"] == "message":
                for c in item.get("content", []):
                    if c["type"] == "output_text":
                        print(f"[Response] {c['text'][:80]}")
        usage = data.get("usage", {})
        reasoning_tok = usage.get("output_tokens_details", {}).get("reasoning_tokens", 0)
        print(f"Reasoning tokens: {reasoning_tok}, Total: {usage.get('total_tokens', 0)}")
    print()

    # Example 6: Streaming Thinking with Responses API
    print("6. Streaming Thinking (Responses API)")
    print("-" * 30)
    with httpx.Client(base_url=responses_url, timeout=60.0) as http_client:
        with http_client.stream(
            "POST",
            "/openai/v1/responses",
            json={
                "model": "o3",
                "input": "Explain why 1+1=2",
                "stream": True,
                "reasoning": {"effort": "low", "summary": "concise"},
            },
        ) as stream_resp:
            buf = ""
            for chunk in stream_resp.iter_text():
                buf += chunk
                while "\n\n" in buf:
                    event_str, buf = buf.split("\n\n", 1)
                    if not event_str.strip():
                        continue
                    ev = {}
                    for line in event_str.strip().split("\n"):
                        if line.startswith("event: "):
                            ev["event"] = line[7:]
                        elif line.startswith("data: "):
                            try:
                                ev["data"] = json.loads(line[6:])
                            except json.JSONDecodeError:
                                pass
                    if not ev:
                        continue
                    etype = ev.get("event", "")
                    edata = ev.get("data", {})
                    if etype == "response.output_item.added":
                        item = edata.get("item", {})
                        if item.get("type") == "reasoning":
                            print("[Thinking] ", end="", flush=True)
                        elif item.get("type") == "message":
                            print("\n[Response] ", end="", flush=True)
                    elif etype == "response.reasoning_summary_text.delta":
                        print(edata.get("delta", ""), end="", flush=True)
                    elif etype == "response.output_text.delta":
                        print(edata.get("delta", ""), end="", flush=True)
                    elif etype == "response.completed":
                        usage = edata.get("response", {}).get("usage", {})
                        rtok = usage.get("output_tokens_details", {}).get("reasoning_tokens", 0)
                        print(f"\n  Tokens: reasoning={rtok}, total={usage.get('total_tokens', 0)}")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
