import assert from "node:assert/strict";
import OpenAI from "openai";

const server = process.env.LLMSIM_SERVER_URL ?? "http://127.0.0.1:8080";
const client = new OpenAI({ baseURL: new URL("/openai/v1", server).href, apiKey: "unused", maxRetries: 1 });
const model = "gpt-5";
const tools: OpenAI.Responses.FunctionTool[] = [{
  type: "function",
  name: "read_file",
  parameters: {
    type: "object",
    properties: { path: { type: "string" } },
    required: ["path"],
    additionalProperties: false,
  },
  strict: true,
}];

type LoggedCall = {
  path: string;
  status: number;
  scenario: string;
  step: number;
  attempt: number;
  body: {
    input: OpenAI.Responses.ResponseCreateParams["input"];
    tools?: OpenAI.Responses.Tool[];
  };
};

async function control(path: string, method: "GET" | "POST"): Promise<Response> {
  const response = await fetch(new URL(path, server), { method, signal: AbortSignal.timeout(5_000) });
  assert.ok(response.ok, `${method} ${path} returned ${response.status}`);
  return response;
}

async function main() {
  await control("/llmsim/calls/reset", "POST");
  const input = "Read the project files [[llmsim:flaky-429 speed=0.001]]";
  let response = await client.responses.create({ model, input, tools });
  const sentResults = [];
  for (const [step, count] of [1, 1, 2].entries()) {
    const calls = response.output.filter((item) => item.type === "function_call");
    assert.equal(calls.length, count);
    const results = calls.map((call) => {
      assert.equal(call.name, "read_file");
      assert.deepEqual(JSON.parse(call.arguments), { path: "." });
      return { type: "function_call_output" as const, call_id: call.call_id, output: "Fixture contents" };
    });
    sentResults.push(results);
    response = await client.responses.create({
      model, tools, previous_response_id: response.id, input: results, store: step < 2,
    });
  }
  assert.ok(response.output_text);
  assert.ok(response.output.every((item) => item.type !== "function_call"));

  const log = await (await control("/llmsim/calls", "GET")).json() as LoggedCall[];
  const calls = log.filter((call) => call.path === "/openai/v1/responses" && call.scenario === "flaky-429");
  assert.deepEqual(calls.map(({ status, step, attempt }) => [status, step, attempt]), [
    [429, 0, 0], [200, 0, 1], [200, 1, 0], [200, 2, 0], [200, 3, 0],
  ]);
  assert.deepEqual(calls.slice(0, 2).map((call) => call.body.input), [input, input]);
  assert.deepEqual(calls[0].body.tools, tools);
  assert.deepEqual(calls.slice(2).map((call) => call.body.input), sentResults);
  console.log("Verified 4 read_file calls, 1 SDK retry, and a completed tool loop.");
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
