#!/usr/bin/env npx tsx
/**
 * Image generation example for llmsim (TypeScript/JavaScript).
 *
 * Demonstrates the simulated OpenAI image generation endpoint. The server
 * returns a synthetic PNG of the requested size that renders the prompt text
 * and a clear "LLMSIM SIMULATED IMAGE" watermark — no real model runs.
 *
 * Server endpoint:
 *     POST /openai/v1/images/generations - Image generation (streaming supported)
 *
 * Prerequisites:
 *     Start the llmsim server first:
 *         llmsim serve --port 8080
 *
 *     Install dependencies:
 *         npm install
 *
 * Usage:
 *     npx tsx images_client.ts
 *
 * Environment variables:
 *     LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
 */

import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import OpenAI from "openai";

async function main(): Promise<void> {
  const baseURL = process.env.LLMSIM_URL || "http://localhost:8080/openai/v1";
  const outDir = mkdtempSync(join(tmpdir(), "llmsim-images-"));

  console.log("=".repeat(50));
  console.log("OpenAI Image Generation (TypeScript) + LLMSim");
  console.log("=".repeat(50));
  console.log(`\nConnecting to: ${baseURL}`);
  console.log(`Saving images to: ${outDir}\n`);

  const client = new OpenAI({ baseURL, apiKey: "not-needed" });

  // Example 1: Basic image generation (non-streaming).
  console.log("1. Generate an image");
  console.log("-".repeat(30));
  let result;
  try {
    result = await client.images.generate({
      model: "gpt-image-1",
      prompt: "a cat riding a bicycle on the moon",
      size: "1024x1024",
      quality: "low",
    });
  } catch (e) {
    console.error(`Error: ${e}`);
    console.error("\nMake sure the llmsim server is running:");
    console.error("  llmsim serve --port 8080");
    process.exit(1);
  }

  const b64 = result.data?.[0]?.b64_json ?? "";
  const png = Buffer.from(b64, "base64");
  const path = join(outDir, "moon_cat.png");
  writeFileSync(path, png);
  console.log(`Saved ${png.length.toLocaleString()} bytes -> ${path}`);
  console.log(`Usage:`, result.usage);
  console.log();

  // Example 2: Streaming with partial images.
  //
  // The image streaming API emits `image_generation.partial_image` events
  // (progressively sharper previews) then a final `image_generation.completed`
  // event with the full image and usage. We parse the SSE stream with fetch.
  console.log("2. Streaming with partial images");
  console.log("-".repeat(30));
  const resp = await fetch(`${baseURL}/images/generations`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model: "gpt-image-1",
      prompt: "sunset over snowy mountains",
      size: "1024x1024",
      quality: "medium",
      stream: true,
      partial_images: 3,
    }),
  });

  const reader = resp.body!.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let sep: number;
    while ((sep = buf.indexOf("\n\n")) !== -1) {
      const raw = buf.slice(0, sep);
      buf = buf.slice(sep + 2);
      const dataLine = raw
        .split("\n")
        .find((ln) => ln.startsWith("data: "));
      if (!dataLine) continue;
      const event = JSON.parse(dataLine.slice(6));
      const data = Buffer.from(event.b64_json, "base64");
      if (event.type === "image_generation.partial_image") {
        const p = join(outDir, `stream_partial_${event.partial_image_index}.png`);
        writeFileSync(p, data);
        console.log(
          `  partial #${event.partial_image_index}: ${data.length.toLocaleString()} bytes -> ${p}`,
        );
      } else if (event.type === "image_generation.completed") {
        const p = join(outDir, "stream_final.png");
        writeFileSync(p, data);
        console.log(`  completed : ${data.length.toLocaleString()} bytes -> ${p}`);
        console.log(`  usage     :`, event.usage);
      }
    }
  }
  console.log();

  console.log("=".repeat(50));
  console.log("Examples complete!");
  console.log("=".repeat(50));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
