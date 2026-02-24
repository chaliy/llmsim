#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "websocket-client>=1.8.0",
# ]
# ///
"""
OpenAI Responses API WebSocket mode client example for llmsim.

This script demonstrates connecting to a running llmsim server using the
WebSocket transport for the Responses API. WebSocket mode keeps a persistent
connection and is ideal for multi-turn, tool-heavy agentic workflows.

Protocol reference:
    https://platform.openai.com/docs/guides/websocket-mode

Server endpoints:
    WS /openai/v1/responses - WebSocket Responses API

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/websocket_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: ws://localhost:8080)
"""

import json
import os
import sys

from websocket import create_connection


def send_response_create(ws, model: str, input_data, **kwargs) -> list[dict]:
    """Send a response.create event and collect all server events until response.completed."""
    payload = {
        "type": "response.create",
        "response": {
            "model": model,
            "input": input_data,
            **kwargs,
        },
    }

    ws.send(json.dumps(payload))

    events = []
    while True:
        raw = ws.recv()
        event = json.loads(raw)
        events.append(event)

        event_type = event.get("type", "")
        if event_type == "response.completed" or event_type == "error":
            break

    return events


def print_streaming_events(events: list[dict]) -> None:
    """Print streaming events in a human-readable format."""
    for event in events:
        event_type = event.get("type", "")

        if event_type == "response.created":
            resp = event.get("response", {})
            print(f"  [created] id={resp.get('id')}")

        elif event_type == "response.output_text.delta":
            print(event.get("delta", ""), end="", flush=True)

        elif event_type == "response.reasoning_summary_text.delta":
            print(event.get("delta", ""), end="", flush=True)

        elif event_type == "response.output_item.added":
            item = event.get("item", {})
            item_type = item.get("type", "")
            if item_type == "reasoning":
                print("  [thinking] ", end="", flush=True)
            elif item_type == "message":
                print("\n  [response] ", end="", flush=True)

        elif event_type == "response.completed":
            resp = event.get("response", {})
            usage = resp.get("usage", {})
            print(f"\n  [completed] tokens={usage}")

        elif event_type == "error":
            error = event.get("error", {})
            print(f"  [error] {error.get('type')}: {error.get('message')}")


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "ws://localhost:8080")
    ws_url = f"{base_url}/openai/v1/responses"

    print("=" * 60)
    print("OpenAI Responses API WebSocket Mode + LLMSim Example")
    print("=" * 60)
    print(f"\nConnecting to: {ws_url}")

    try:
        ws = create_connection(ws_url)
    except Exception as e:
        print(f"\nError: Could not connect to {ws_url}")
        print(f"  {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)

    print("Connected!\n")

    # Example 1: Simple text input
    print("1. Simple Text Input")
    print("-" * 40)
    events = send_response_create(ws, "gpt-5", "What is the capital of France?")
    print_streaming_events(events)
    print()

    # Example 2: Message array input
    print("2. Message Array Input")
    print("-" * 40)
    events = send_response_create(
        ws,
        "gpt-5",
        [
            {"type": "message", "role": "user", "content": "Tell me a joke."},
        ],
    )
    print_streaming_events(events)
    print()

    # Example 3: Multi-turn with previous_response_id
    print("3. Multi-turn Conversation (previous_response_id)")
    print("-" * 40)
    # First turn
    events1 = send_response_create(ws, "gpt-5", "Hello, I have a question.")
    resp_id = None
    for event in events1:
        if event.get("type") == "response.completed":
            resp_id = event.get("response", {}).get("id")
    print(f"  Turn 1 completed, response_id={resp_id}")

    # Second turn, referencing the first
    events2 = send_response_create(
        ws,
        "gpt-5",
        "What was I saying?",
        previous_response_id=resp_id,
    )
    print(f"  Turn 2:")
    print_streaming_events(events2)
    print()

    # Example 4: previous_response_id not found
    print("4. Error: previous_response_id Not Found")
    print("-" * 40)
    events = send_response_create(
        ws,
        "gpt-5",
        "Hello",
        previous_response_id="resp_does_not_exist",
    )
    print_streaming_events(events)
    print()

    # Example 5: Reasoning model over WebSocket
    print("5. Reasoning Model (o3)")
    print("-" * 40)
    events = send_response_create(
        ws,
        "o3",
        "What is 123 * 456?",
        reasoning={"effort": "medium", "summary": "auto"},
    )
    print_streaming_events(events)
    print()

    # Example 6: Different models on the same connection
    print("6. Multiple Models on Same Connection")
    print("-" * 40)
    for model in ["gpt-5", "gpt-4o", "gpt-5-mini"]:
        events = send_response_create(ws, model, "Hi!")
        for event in events:
            if event.get("type") == "response.completed":
                resp = event.get("response", {})
                usage = resp.get("usage", {})
                print(f"  {model}: output_tokens={usage.get('output_tokens', 0)}")
    print()

    ws.close()

    print("=" * 60)
    print("Examples complete!")
    print("=" * 60)


if __name__ == "__main__":
    main()
