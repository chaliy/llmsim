#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "typesafe-sdk>=0.7.4",
# ]
# ///
"""
TypeSafe System One client example for llmsim.

Demonstrates connecting to a running llmsim server using the official TypeSafe
Python SDK (`typesafe-sdk`). The server answers typed questions (noul, choice,
score) with deterministic, well-formed calibrated answers from a simulated Jev
model, without calling the real API.

Server endpoints:
    POST /typesafe/v1/systemone  - System One evaluation
    GET  /typesafe/v1/models     - List available Jev models and aliases

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/typesafe_client.py

Environment variables:
    LLMSIM_URL: Server base URL (default: http://localhost:8080/typesafe)
"""

import os
import sys

from typesafe_sdk import Choice, Noul, Score, TypeSafeAPIError, TypeSafeClient


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/typesafe")

    print("=" * 50)
    print("TypeSafe SDK + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}\n")

    # Point the official TypeSafe client at llmsim. The simulator ignores the
    # API key, but the SDK requires one to be set.
    with TypeSafeClient(base_url=base_url, api_key="not-needed") as client:
        # Example 1: One request, all three primitives
        print("1. System One (noul, choice, score)")
        print("-" * 30)
        try:
            response = client.system_one(
                state={"document": "I was charged twice. Please fix this ASAP."},
                questions={
                    "billing": Noul(instructions="Is this message about billing?"),
                    "tone": Choice(
                        instructions="What is the tone of this message?",
                        criteria={
                            "angry": "Upset or hostile",
                            "calm": "Neutral or polite",
                            "excited": "Enthusiastic or eager",
                        },
                    ),
                    "urgency": Score(
                        instructions="How urgent is this message?",
                        criteria=["Can wait", "Needs attention this week", "Needs attention today"],
                    ),
                },
            )
        except Exception as e:  # noqa: BLE001
            print(f"Error: {e}")
            print("\nMake sure the llmsim server is running:")
            print("  llmsim serve --port 8080")
            sys.exit(1)

        billing = response.nouls["billing"]
        tone = response.choices["tone"]
        urgency = response.scores["urgency"]
        print(f"Model: {response.model}")
        print(f"billing (noul):  p(yes)={billing.noul:.3f}")
        print(f"tone (choice):   {tone.choice} (confidence={tone.confidence:.3f})")
        print(f"urgency (score): {urgency.score:.2f} -> {urgency.legend[round(urgency.score)]!r}")
        print(f"Tokens: in={response.usage.input_tokens} out={response.usage.output_tokens}")
        assert 0.0 <= billing.noul <= 1.0
        assert tone.choice in tone.probabilities
        assert abs(sum(tone.probabilities.values()) - 1.0) < 1e-3
        assert abs(sum(urgency.probabilities.values()) - 1.0) < 1e-3
        print()

        # Example 2: Answers are deterministic for the same request
        print("2. Deterministic answers")
        print("-" * 30)
        again = client.system_one(
            state={"document": "I was charged twice. Please fix this ASAP."},
            questions={"billing": Noul(instructions="Is this message about billing?")},
        )
        assert again.nouls["billing"].noul == billing.noul
        print(f"Same question, same answer: {again.nouls['billing'].noul:.3f}")
        print()

        # Example 3: Validation errors surface as 422s, as with the real API
        print("3. Validation error")
        print("-" * 30)
        try:
            # The API accepts at most 10 score levels; the SDK leaves that check
            # to the server, so this round-trips to llmsim and comes back a 422.
            client.system_one(
                state="Hello",
                questions={"too_fine": Score(instructions="Rate it", criteria=[str(i) for i in range(11)])},
            )
            print("Unexpected success")
            sys.exit(1)
        except TypeSafeAPIError as e:
            print(f"Rejected with HTTP {e.status}: {e}")
            assert e.status == 422
        print()

        # Example 4: List models
        print("4. Available Models")
        print("-" * 30)
        for m in client.models.list().models:
            print(f"  - {m.name} ({m.release_date}): {m.description}")
        print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
