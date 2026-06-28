#!/usr/bin/env npx tsx
/**
 * Vercel AI SDK example for llmsim (TypeScript).
 *
 * This script demonstrates driving a running llmsim server with the
 * Vercel AI SDK (https://sdk.vercel.ai). The SDK talks to llmsim through its
 * OpenAI-compatible Chat Completions endpoint, so we create an OpenAI provider
 * pointed at the llmsim base URL. The server simulates LLM responses with
 * realistic latency without running actual models.
 *
 * Server endpoints:
 *     POST /openai/v1/chat/completions - Chat completions (streaming supported)
 *     GET  /openai/v1/models           - List available models
 *
 * Note:
 *     A default llmsim server returns simulated (lorem ipsum) text, so it does
 *     not emit schema-conforming JSON or tool calls. To exercise `generateObject`
 *     or tool calling deterministically, run the server in scripted mode — see
 *     specs/scripted-mode.md and examples/scripted_demo/.
 *
 * Prerequisites:
 *     Start the llmsim server first:
 *         llmsim serve --port 8080
 *
 *     Or from source:
 *         cargo run --release -- serve --port 8080
 *
 *     Install dependencies:
 *         cd examples/node && npm install
 *
 * Usage:
 *     npx tsx examples/node/vercel_ai_client.ts
 *
 * Environment variables:
 *     LLMSIM_URL: Server URL (default: http://localhost:8080/openai/v1)
 */

import { createOpenAI } from "@ai-sdk/openai";
import { generateText, streamText } from "ai";

async function main(): Promise<void> {
  const baseURL = process.env.LLMSIM_URL || "http://localhost:8080/openai/v1";

  console.log("=".repeat(50));
  console.log("Vercel AI SDK + LLMSim Example");
  console.log("=".repeat(50));
  console.log(`\nConnecting to: ${baseURL}`);
  console.log();

  // Create an OpenAI-compatible provider pointing at llmsim.
  // apiKey is required by the provider but llmsim doesn't validate it.
  const openai = createOpenAI({ baseURL, apiKey: "not-needed" });

  // `.chat(...)` targets the Chat Completions endpoint that llmsim exposes.
  const model = openai.chat("gpt-5");

  // Example 1: Simple text generation
  console.log("1. Simple Generation");
  console.log("-".repeat(30));
  try {
    const { text, usage } = await generateText({
      model,
      system: "You are a helpful assistant.",
      prompt: "What is the capital of France?",
    });
    console.log(`Response: ${text}`);
    console.log(`Usage: ${JSON.stringify(usage)}`);
  } catch (e) {
    console.log(`Error: ${e}`);
    console.log("\nMake sure the llmsim server is running:");
    console.log("  llmsim serve --port 8080");
    process.exit(1);
  }
  console.log();

  // Example 2: Streaming
  console.log("2. Streaming Response");
  console.log("-".repeat(30));
  process.stdout.write("Response: ");
  const { textStream } = streamText({
    model,
    prompt: "Tell me a short story.",
  });
  for await (const delta of textStream) {
    process.stdout.write(delta);
  }
  console.log("\n");

  // Example 3: Multi-turn conversation (messages)
  console.log("3. Multi-turn Conversation");
  console.log("-".repeat(30));
  const { text: conversationText } = await generateText({
    model,
    system: "You are a helpful assistant.",
    messages: [
      { role: "user", content: "My name is Alice." },
      { role: "assistant", content: "Hello Alice! Nice to meet you." },
      { role: "user", content: "What's my name?" },
    ],
  });
  console.log(`Response: ${conversationText}`);
  console.log();

  // Example 4: Different models
  console.log("4. Different Models");
  console.log("-".repeat(30));
  for (const name of ["gpt-5-mini", "claude-opus-4.5", "o3-mini"]) {
    const { text } = await generateText({
      model: openai.chat(name),
      prompt: "Hello!",
    });
    console.log(`${name}: ${text.slice(0, 60)}...`);
  }
  console.log();

  console.log("=".repeat(50));
  console.log("Examples complete!");
  console.log("=".repeat(50));
}

main();
