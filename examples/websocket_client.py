#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "openai[realtime]>=2.22.0",
# ]
# ///
"""
OpenAI Responses API WebSocket mode client example for llmsim.

This script demonstrates connecting to a running llmsim server using the
official OpenAI Python SDK's WebSocket transport for the Responses API.
WebSocket mode keeps a persistent connection and is ideal for multi-turn,
tool-heavy agentic workflows.

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
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import os
import sys

from openai import OpenAI
from openai.types.responses import (
    ResponseCompletedEvent,
    ResponseCreatedEvent,
    ResponseErrorEvent,
    ResponseOutputItemAddedEvent,
    ResponseReasoningSummaryTextDeltaEvent,
    ResponseTextDeltaEvent,
)


def collect_events(connection) -> list:
    """Receive server events until response.completed or error."""
    events = []
    for event in connection:
        events.append(event)
        if event.type in ("response.completed", "error"):
            break
    return events


def print_streaming_events(events: list) -> None:
    """Print streaming events in a human-readable format."""
    for event in events:
        if isinstance(event, ResponseCreatedEvent):
            print(f"  [created] id={event.response.id}")

        elif isinstance(event, ResponseOutputItemAddedEvent):
            item_type = event.item.type
            if item_type == "reasoning":
                print("  [thinking] ", end="", flush=True)
            elif item_type == "message":
                print("\n  [response] ", end="", flush=True)

        elif isinstance(event, ResponseReasoningSummaryTextDeltaEvent):
            print(event.delta, end="", flush=True)

        elif isinstance(event, ResponseTextDeltaEvent):
            print(event.delta, end="", flush=True)

        elif isinstance(event, ResponseCompletedEvent):
            usage = event.response.usage
            print(f"\n  [completed] tokens: input={usage.input_tokens}, "
                  f"output={usage.output_tokens}, total={usage.total_tokens}")

        elif isinstance(event, ResponseErrorEvent):
            print(f"  [error] {event.code}: {event.message}")


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")

    # Derive WebSocket URL: http -> ws, https -> wss
    ws_url = base_url.replace("https://", "wss://").replace("http://", "ws://")

    print("=" * 60)
    print("OpenAI Responses API WebSocket Mode + LLMSim Example")
    print("=" * 60)
    print(f"\nConnecting to: {ws_url}/responses")

    client = OpenAI(
        base_url=base_url,
        api_key="not-needed",  # llmsim doesn't require auth
        websocket_base_url=ws_url,
    )

    try:
        with client.responses.connect() as connection:
            print("Connected!\n")

            # Example 1: Simple text input
            print("1. Simple Text Input")
            print("-" * 40)
            connection.response.create(
                model="gpt-5",
                input="What is the capital of France?",
            )
            events = collect_events(connection)
            print_streaming_events(events)
            print()

            # Example 2: Message array input
            print("2. Message Array Input")
            print("-" * 40)
            connection.response.create(
                model="gpt-5",
                input=[
                    {"type": "message", "role": "user", "content": "Tell me a joke."},
                ],
            )
            events = collect_events(connection)
            print_streaming_events(events)
            print()

            # Example 3: Multi-turn with previous_response_id
            print("3. Multi-turn Conversation (previous_response_id)")
            print("-" * 40)

            # First turn
            connection.response.create(
                model="gpt-5",
                input="Hello, I have a question.",
            )
            events1 = collect_events(connection)
            resp_id = None
            for event in events1:
                if isinstance(event, ResponseCompletedEvent):
                    resp_id = event.response.id
            print(f"  Turn 1 completed, response_id={resp_id}")

            # Second turn, referencing the first
            connection.response.create(
                model="gpt-5",
                input="What was I saying?",
                previous_response_id=resp_id,
            )
            events2 = collect_events(connection)
            print("  Turn 2:")
            print_streaming_events(events2)
            print()

            # Example 4: previous_response_id not found
            print("4. Error: previous_response_id Not Found")
            print("-" * 40)
            connection.response.create(
                model="gpt-5",
                input="Hello",
                previous_response_id="resp_does_not_exist",
            )
            events = collect_events(connection)
            print_streaming_events(events)
            print()

            # Example 5: Reasoning model over WebSocket
            print("5. Reasoning Model (o3)")
            print("-" * 40)
            connection.response.create(
                model="o3",
                input="What is 123 * 456?",
                reasoning={"effort": "medium", "summary": "auto"},
            )
            events = collect_events(connection)
            print_streaming_events(events)
            print()

            # Example 6: Different models on the same connection
            print("6. Multiple Models on Same Connection")
            print("-" * 40)
            for model in ["gpt-5", "gpt-4o", "gpt-5-mini"]:
                connection.response.create(model=model, input="Hi!")
                events = collect_events(connection)
                for event in events:
                    if isinstance(event, ResponseCompletedEvent):
                        usage = event.response.usage
                        print(f"  {model}: output_tokens={usage.output_tokens}")
            print()

    except Exception as e:
        print(f"\nError: Could not connect to {ws_url}/responses")
        print(f"  {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)

    print("=" * 60)
    print("Examples complete!")
    print("=" * 60)


if __name__ == "__main__":
    main()
