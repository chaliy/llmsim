#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "requests>=2.32.0",
# ]
# ///
"""Drive llmsim's scripted mode from Python.

Prerequisites
-------------
1. Start llmsim with a script file::

    cargo run -- serve --config examples/scripted_demo.toml

   where scripted_demo.toml is::

    [response]
    script_path = "examples/scripted_demo.json"

   Or set it inline by editing the default config and pointing it at
   examples/scripted_demo.json.

2. Run this script::

    python examples/scripted_demo.py

Expected output
---------------
Three tool-call turns followed by a plain "done" assistant message. On
the 5th request the server returns HTTP 500 because the script's
``on_exhausted`` mode is ``error``.

This demo uses ``requests`` rather than the openai SDK so it's
dependency-free and easy to read.
"""

import json
import os
import sys

import requests

# LLMSIM_URL points at the OpenAI-compatible base (default matches the
# README); CI overrides it to target a dedicated scripted server.
LLMSIM_URL = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")
BASE = f"{LLMSIM_URL.rstrip('/')}/chat/completions"
HEADERS = {"Content-Type": "application/json", "Authorization": "Bearer not-needed"}


def call(turn_index: int) -> None:
    body = {
        "model": "gpt-5",
        "messages": [
            {"role": "user", "content": f"turn {turn_index}"},
        ],
    }
    resp = requests.post(BASE, headers=HEADERS, data=json.dumps(body), timeout=10)
    print(f"=== Turn {turn_index} (HTTP {resp.status_code}) ===")
    data = resp.json()
    if resp.status_code != 200:
        print(f"  error: {data.get('error', {}).get('message')}")
        return

    choice = data["choices"][0]
    msg = choice["message"]
    finish = choice.get("finish_reason")
    print(f"  finish_reason: {finish}")
    if msg.get("content"):
        print(f"  content: {msg['content']!r}")
    for tc in msg.get("tool_calls") or []:
        fn = tc["function"]
        print(f"  tool_call id={tc['id']} name={fn['name']} args={fn['arguments']}")


def main() -> int:
    # Five requests: the example script has 4 turns, so the 5th hits
    # the on_exhausted=error path.
    for i in range(5):
        call(i)
    return 0


if __name__ == "__main__":
    sys.exit(main())
