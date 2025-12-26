#!/usr/bin/env npx tsx
/**
 * OpenAI client example for llmsim (TypeScript/JavaScript).
 *
 * This script demonstrates connecting to a running llmsim server using the
 * official OpenAI Node.js library. The server simulates LLM responses with
 * realistic latency without running actual models.
 *
 * Server endpoints:
 *     POST /openai/chat/completions - Chat completions (streaming supported)
 *     GET  /openai/models           - List available models
 *     GET  /openai/models/:id       - Get model details
 *
 * Prerequisites:
 *     Start the llmsim server first:
 *         llmsim serve --port 8080
 *
 *     Or from source:
 *         cargo run --release -- serve --port 8080
 *
 *     Install dependencies:
 *         npm install openai
 *
 * Usage:
 *     npx tsx examples/openai_client.ts
 *
 * Environment variables:
 *     LLMSIM_URL: Server URL (default: http://localhost:8080/openai)
 */

import OpenAI from "openai";

async function main(): Promise<void> {
  const baseURL = process.env.LLMSIM_URL || "http://localhost:8080/openai";

  console.log("=".repeat(50));
  console.log("OpenAI SDK (TypeScript) + LLMSim Example");
  console.log("=".repeat(50));
  console.log(`\nConnecting to: ${baseURL}`);
  console.log();

  // Create OpenAI client pointing to llmsim
  const client = new OpenAI({
    baseURL,
    apiKey: "not-needed", // llmsim doesn't require auth
  });

  // Example 1: Simple completion
  console.log("1. Simple Completion");
  console.log("-".repeat(30));
  try {
    const response = await client.chat.completions.create({
      model: "gpt-5",
      messages: [
        { role: "system", content: "You are a helpful assistant." },
        { role: "user", content: "What is the capital of France?" },
      ],
      max_tokens: 100,
    });
    console.log(`Response: ${response.choices[0].message.content}`);
    console.log(`Model: ${response.model}`);
    console.log(`Tokens: ${JSON.stringify(response.usage)}`);
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

  const stream = await client.chat.completions.create({
    model: "gpt-5",
    messages: [{ role: "user", content: "Tell me a short story." }],
    max_tokens: 100,
    stream: true,
  });

  for await (const chunk of stream) {
    const content = chunk.choices[0]?.delta?.content;
    if (content) {
      process.stdout.write(content);
    }
  }
  console.log("\n");

  // Example 3: Different models
  console.log("3. Different Models");
  console.log("-".repeat(30));

  const models = ["gpt-5-mini", "claude-opus-4.5", "o3-mini"];
  for (const model of models) {
    const response = await client.chat.completions.create({
      model,
      messages: [{ role: "user", content: "Hello!" }],
      max_tokens: 50,
    });
    const content = response.choices[0].message.content || "";
    console.log(`${model}: ${content.slice(0, 60)}...`);
  }
  console.log();

  // Example 4: List available models
  console.log("4. Available Models");
  console.log("-".repeat(30));
  const modelsList = await client.models.list();
  const modelsArray = [];
  for await (const model of modelsList) {
    modelsArray.push(model);
  }
  for (const model of modelsArray.slice(0, 5)) {
    console.log(`  - ${model.id} (owned by: ${model.owned_by})`);
  }
  if (modelsArray.length > 5) {
    console.log(`  ... and ${modelsArray.length - 5} more`);
  }
  console.log();

  // Example 5: Multiple messages (conversation)
  console.log("5. Multi-turn Conversation");
  console.log("-".repeat(30));
  const conversationResponse = await client.chat.completions.create({
    model: "gpt-5",
    messages: [
      { role: "system", content: "You are a helpful assistant." },
      { role: "user", content: "My name is Alice." },
      { role: "assistant", content: "Hello Alice! Nice to meet you." },
      { role: "user", content: "What's my name?" },
    ],
    max_tokens: 50,
  });
  console.log(`Response: ${conversationResponse.choices[0].message.content}`);
  console.log();

  console.log("=".repeat(50));
  console.log("Examples complete!");
  console.log("=".repeat(50));
}

main();
