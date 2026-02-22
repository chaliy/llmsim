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
    uv run examples/openai_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import os
import sys

from openai import OpenAI


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")

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
        print("  llmsim serve --port 8080")
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

    # Example 6: Responses API with Thinking
    print("6. Responses API with Thinking (o3)")
    print("-" * 30)

    # Use httpx for Responses API since the OpenAI SDK uses a different interface
    import json
    import httpx

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

    # Example 7: Streaming Thinking with Responses API
    print("7. Streaming Thinking (Responses API)")
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
