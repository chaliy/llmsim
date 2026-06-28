#!/usr/bin/env npx tsx
/**
 * Anthropic client example for llmsim (TypeScript/JavaScript).
 *
 * Demonstrates connecting to a running llmsim server using the official
 * Anthropic Node.js SDK (@anthropic-ai/sdk). The server simulates Claude
 * responses with realistic latency without running actual models.
 *
 * Server endpoints:
 *     POST /anthropic/v1/messages    - Messages API (streaming supported)
 *     GET  /anthropic/v1/models      - List available Claude models
 *     GET  /anthropic/v1/models/:id  - Get model details
 *
 * Prerequisites:
 *     Start the llmsim server first:
 *         llmsim serve --port 8080
 *
 *     Install dependencies:
 *         npm install @anthropic-ai/sdk
 *
 * Usage:
 *     npx tsx examples/node/anthropic_client.ts
 *
 * Environment variables:
 *     LLMSIM_URL: Server base URL (default: http://localhost:8080/anthropic)
 */

import Anthropic from "@anthropic-ai/sdk";

async function main(): Promise<void> {
  const baseURL = process.env.LLMSIM_URL || "http://localhost:8080/anthropic";

  console.log("=".repeat(50));
  console.log("Anthropic SDK (TypeScript) + LLMSim Example");
  console.log("=".repeat(50));
  console.log(`\nConnecting to: ${baseURL}\n`);

  // The simulator ignores the API key, but the SDK requires one.
  const client = new Anthropic({ baseURL, apiKey: "not-needed" });

  // Example 1: Simple message
  console.log("1. Simple Message");
  console.log("-".repeat(30));
  try {
    const msg = await client.messages.create({
      model: "claude-opus-4-8",
      max_tokens: 128,
      system: "You are a helpful assistant.",
      messages: [{ role: "user", content: "What is the capital of France?" }],
    });
    const first = msg.content[0];
    if (first.type === "text") console.log(`Response: ${first.text}`);
    console.log(`Model: ${msg.model} | stop_reason: ${msg.stop_reason}`);
    console.log(
      `Tokens: in=${msg.usage.input_tokens} out=${msg.usage.output_tokens}`,
    );
  } catch (err) {
    console.error(`Error: ${err}`);
    console.error("\nMake sure the llmsim server is running:");
    console.error("  llmsim serve --port 8080");
    process.exit(1);
  }
  console.log();

  // Example 2: Streaming
  console.log("2. Streaming Response");
  console.log("-".repeat(30));
  process.stdout.write("Response: ");
  const stream = client.messages.stream({
    model: "claude-haiku-4-5",
    max_tokens: 128,
    messages: [{ role: "user", content: "Tell me a short story." }],
  });
  stream.on("text", (text) => process.stdout.write(text));
  const final = await stream.finalMessage();
  console.log(`\n(streamed, output tokens: ${final.usage.output_tokens})\n`);

  // Example 3: Multi-turn conversation
  console.log("3. Multi-turn Conversation");
  console.log("-".repeat(30));
  const convo = await client.messages.create({
    model: "claude-sonnet-4-6",
    max_tokens: 64,
    messages: [
      { role: "user", content: "My name is Ada." },
      { role: "assistant", content: "Hello Ada! Nice to meet you." },
      { role: "user", content: "What is my name?" },
    ],
  });
  const convoFirst = convo.content[0];
  if (convoFirst.type === "text")
    console.log(`Response: ${convoFirst.text.slice(0, 80)}\n`);

  // Example 4: List and retrieve models
  console.log("4. Available Models");
  console.log("-".repeat(30));
  const models = await client.models.list();
  for (const m of models.data.slice(0, 6)) {
    console.log(`  - ${m.id} (${m.display_name})`);
  }
  const one = await client.models.retrieve("claude-opus-4-8");
  console.log(`  retrieve: ${one.id} created_at=${one.created_at}`);
  console.log();

  console.log("=".repeat(50));
  console.log("Examples complete!");
  console.log("=".repeat(50));
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
