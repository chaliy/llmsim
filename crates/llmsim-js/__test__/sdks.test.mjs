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
  assert.deepEqual((await client.responses.retrieve(resp.id)).output, resp.output);
});

test("openai responses continues a streamed tool call by response id", async () => {
  const client = new OpenAI({ baseURL: sim.openaiBaseUrl, apiKey: "unused", maxRetries: 0 });
  const tools = [{
    type: "function",
    name: "read_file",
    parameters: { type: "object", properties: { path: { type: "string" } }, required: ["path"] },
    strict: true,
  }];
  const stream = await client.responses.create({
    model: "gpt-5",
    input: "Check the files [[llmsim:parallel-tools speed=0.001]]",
    tools,
    stream: true,
  });
  let first;
  for await (const event of stream) {
    if (event.type === "response.completed") first = event.response;
  }
  assert.ok(first);
  const calls = first.output.filter((item) => item.type === "function_call");
  assert.equal(calls.length, 4);
  assert.ok(calls.every((call) => call.name === "read_file"));
  assert.deepEqual((await client.responses.retrieve(first.id)).output, first.output);

  const next = await client.responses.create({
    model: "gpt-5",
    previous_response_id: first.id,
    input: calls.map((call) => ({
      type: "function_call_output", call_id: call.call_id, output: "fixture contents",
    })),
    store: false,
  }).withResponse();
  assert.equal(next.response.headers.get("x-llmsim-step"), "1");
  assert.ok(next.data.output_text);
  await assert.rejects(client.responses.retrieve(next.data.id), { status: 404 });
  await assert.rejects(client.responses.create({
    model: "gpt-5", previous_response_id: next.data.id, input: "Continue",
  }), { status: 404, code: "previous_response_not_found" });
  assert.equal((await client.responses.delete(first.id)).deleted, true);
  await assert.rejects(client.responses.retrieve(first.id), { status: 404 });
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
