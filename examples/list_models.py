#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "openai>=1.60.0",
# ]
# ///
"""
List models example for llmsim.

Demonstrates the /openai/v1/models endpoint which returns realistic model
profiles sourced from https://models.dev, including context window sizes
and max output token limits.

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

Usage:
    uv run examples/list_models.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import os
import sys

from openai import OpenAI


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")

    print("=" * 60)
    print("LLMSim Models Endpoint - Profiles from models.dev")
    print("=" * 60)
    print(f"\nConnecting to: {base_url}")
    print()

    client = OpenAI(
        base_url=base_url,
        api_key="not-needed",
    )

    try:
        models = client.models.list()
    except Exception as e:
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)

    models_list = list(models)
    print(f"Available models: {len(models_list)}\n")

    # Group by owner
    by_owner: dict[str, list] = {}
    for model in models_list:
        owner = model.owned_by
        if owner not in by_owner:
            by_owner[owner] = []
        by_owner[owner].append(model)

    for owner, owner_models in sorted(by_owner.items()):
        print(f"{owner.upper()} ({len(owner_models)} models)")
        print("-" * 60)

        for model in sorted(owner_models, key=lambda m: m.id):
            # Access extended fields from model data
            model_data = model.model_dump()
            context = model_data.get("context_window")
            max_output = model_data.get("max_output_tokens")

            context_str = f"{context:,}" if context else "N/A"
            output_str = f"{max_output:,}" if max_output else "N/A"

            print(f"  {model.id:<25} context: {context_str:>10}  max_output: {output_str:>10}")

        print()

    # Fetch a specific model
    print("Fetching specific model: gpt-5")
    print("-" * 60)
    model = client.models.retrieve("gpt-5")
    model_data = model.model_dump()

    print(f"  ID:              {model.id}")
    print(f"  Owned by:        {model.owned_by}")
    print(f"  Created:         {model.created}")
    print(f"  Context window:  {model_data.get('context_window', 'N/A'):,}")
    print(f"  Max output:      {model_data.get('max_output_tokens', 'N/A'):,}")
    print()

    print("=" * 60)
    print("Model profiles sourced from https://models.dev")
    print("=" * 60)


if __name__ == "__main__":
    main()
