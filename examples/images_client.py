#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "openai>=2.44.0",
#     "httpx>=0.27.0",
# ]
# ///
"""
Image generation example for llmsim (gpt-image / "ChatGPT Images").

This script demonstrates the simulated OpenAI image generation endpoint. The
server returns a synthetic PNG of the requested size that renders the prompt
text and a clear "LLMSIM SIMULATED IMAGE" watermark — no real model runs.

Server endpoint:
    POST /openai/v1/images/generations - Image generation (streaming supported)

Prerequisites:
    Start the llmsim server first:
        llmsim serve --port 8080

    Or from source:
        cargo run --release -- serve --port 8080

Usage:
    uv run examples/images_client.py

Environment variables:
    LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
"""

import base64
import json
import os
import sys
import tempfile

import httpx
from openai import OpenAI


def main() -> None:
    base_url = os.environ.get("LLMSIM_URL", "http://localhost:8080/openai/v1")
    out_dir = tempfile.mkdtemp(prefix="llmsim-images-")

    print("=" * 50)
    print("OpenAI Image Generation + LLMSim Example")
    print("=" * 50)
    print(f"\nConnecting to: {base_url}")
    print(f"Saving images to: {out_dir}")
    print()

    client = OpenAI(base_url=base_url, api_key="not-needed")

    # Example 1: Basic image generation (non-streaming).
    print("1. Generate an image")
    print("-" * 30)
    try:
        result = client.images.generate(
            model="gpt-image-1",
            prompt="a cat riding a bicycle on the moon",
            size="1024x1024",
            quality="low",
        )
    except Exception as e:  # noqa: BLE001
        print(f"Error: {e}")
        print("\nMake sure the llmsim server is running:")
        print("  llmsim serve --port 8080")
        sys.exit(1)

    image = result.data[0]
    png = base64.b64decode(image.b64_json)
    path = os.path.join(out_dir, "moon_cat.png")
    with open(path, "wb") as f:
        f.write(png)
    print(f"Saved {len(png):,} bytes -> {path}")
    print(f"Usage: {result.usage}")
    print()

    # Example 2: Different sizes and qualities.
    print("2. Sizes and qualities")
    print("-" * 30)
    for size, quality in [("1024x1024", "low"), ("1536x1024", "medium"), ("1024x1536", "high")]:
        result = client.images.generate(
            model="gpt-image-1",
            prompt=f"a {quality} quality landscape, {size}",
            size=size,
            quality=quality,
        )
        png = base64.b64decode(result.data[0].b64_json)
        print(f"  {size} {quality:>6}: {len(png):,} bytes, output_tokens={result.usage.output_tokens}")
    print()

    # Example 3: Streaming with partial images.
    #
    # The OpenAI image streaming API emits `image_generation.partial_image`
    # events (progressively sharper previews) followed by a final
    # `image_generation.completed` event carrying the full image and usage.
    # We parse the SSE stream directly with httpx so the event shape is explicit.
    print("3. Streaming with partial images")
    print("-" * 30)
    with httpx.Client(base_url=base_url, timeout=120.0) as http:
        with http.stream(
            "POST",
            "/images/generations",
            json={
                "model": "gpt-image-1",
                "prompt": "sunset over snowy mountains",
                "size": "1024x1024",
                "quality": "medium",
                "stream": True,
                "partial_images": 3,
            },
        ) as resp:
            buf = ""
            for chunk in resp.iter_text():
                buf += chunk
                while "\n\n" in buf:
                    raw, buf = buf.split("\n\n", 1)
                    data_line = next(
                        (ln[6:] for ln in raw.splitlines() if ln.startswith("data: ")),
                        None,
                    )
                    if not data_line:
                        continue
                    event = json.loads(data_line)
                    etype = event["type"]
                    png = base64.b64decode(event["b64_json"])
                    if etype == "image_generation.partial_image":
                        idx = event["partial_image_index"]
                        path = os.path.join(out_dir, f"stream_partial_{idx}.png")
                        with open(path, "wb") as f:
                            f.write(png)
                        print(f"  partial #{idx}: {len(png):,} bytes -> {path}")
                    elif etype == "image_generation.completed":
                        path = os.path.join(out_dir, "stream_final.png")
                        with open(path, "wb") as f:
                            f.write(png)
                        print(f"  completed : {len(png):,} bytes -> {path}")
                        print(f"  usage     : {event['usage']}")
    print()

    print("=" * 50)
    print("Examples complete!")
    print("=" * 50)


if __name__ == "__main__":
    main()
