// Official SDKs pointed at the in-process server.
import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import Anthropic from "@anthropic-ai/sdk";
import OpenAI from "openai";

import { Server } from "../wrapper.js";

let sim;
before(async () => {
  sim = await Server.start({ generator: "echo", latency: "instant" });
});
after(async () => {
  await sim.stop();
});

test("openai chat completion", async () => {
  const client = new OpenAI({ baseURL: sim.openaiBaseUrl, apiKey: "unused" });
  const reply = await client.chat.completions.create({
    model: "gpt-5",
    messages: [{ role: "user", content: "hello sdk" }],
  });
  assert.equal(reply.choices[0].message.content, "Echo: hello sdk");
  assert.ok(reply.usage.total_tokens > 0);
});

test("openai streaming", async () => {
  const client = new OpenAI({ baseURL: sim.openaiBaseUrl, apiKey: "unused" });
  const stream = await client.chat.completions.create({
    model: "gpt-5",
    messages: [{ role: "user", content: "stream me" }],
    stream: true,
  });
  let text = "";
  for await (const chunk of stream) text += chunk.choices[0]?.delta?.content ?? "";
  assert.equal(text, "Echo: stream me");
});

test("openai responses", async () => {
  const client = new OpenAI({ baseURL: sim.openaiBaseUrl, apiKey: "unused" });
  const resp = await client.responses.create({ model: "gpt-5", input: "responses api" });
  assert.equal(resp.output_text, "Echo: responses api");
});

test("anthropic messages", async () => {
  const client = new Anthropic({ baseURL: sim.anthropicBaseUrl, apiKey: "unused" });
  const msg = await client.messages.create({
    model: "claude-sonnet-4-6",
    max_tokens: 100,
    messages: [{ role: "user", content: "hi claude" }],
  });
  assert.equal(msg.content[0].text, "Echo: user: hi claude");
});
