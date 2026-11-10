# llmsim (Node.js / TypeScript)

In-process [llmsim](https://github.com/chaliy/llmsim) server for Node.js and
TypeScript tests: a fast LLM API simulator for the OpenAI (Chat Completions,
Responses, Images), Anthropic Messages, and OpenResponses APIs. No model, no
network, no API key. The server is the same Rust code as the `llmsim` CLI,
shipped as a prebuilt native addon (Linux x64/arm64, macOS, Windows x64).

```bash
npm install --save-dev llmsim
```

## Usage

```ts
import OpenAI from "openai";
import { Server } from "llmsim";

const sim = await Server.start({ generator: "echo", latency: "instant" });
try {
  const client = new OpenAI({ baseURL: sim.openaiBaseUrl, apiKey: "unused" });
  const reply = await client.chat.completions.create({
    model: "gpt-5",
    messages: [{ role: "user", content: "hello" }],
  });
  console.log(reply.choices[0].message.content); // "Echo: hello"
  console.log(sim.stats().total_requests); // 1
} finally {
  await sim.stop();
}
```

Anthropic: `new Anthropic({ baseURL: sim.anthropicBaseUrl, apiKey: "unused" })`.

`Server` implements `Symbol.asyncDispose`, so `await using sim = await Server.start()`
works where explicit resource management is available.

## Options

`Server.start(options)` accepts the same settings as the `llmsim serve` TOML
file. Sources merge in this order, later wins: `configFile` or `toml`, then
`config` (an object shaped like the TOML), then the shortcut options.

| Option | Meaning |
|---|---|
| `host`, `port` | Bind address. Defaults `127.0.0.1` and `0` (a free port) |
| `generator` | `lorem` (default), `echo`, `random`, `sequence`, `fixed:<text>` |
| `targetTokens` | Target response length |
| `latency` | Named profile: `instant`, `fast`, `gpt5`, `claude-sonnet`, ... |
| `script` | Scripted-mode file (exact replies, tool calls, per turn) |
| `scenarios` | Load-test scenario file or directory |

```ts
await Server.start({ config: { errors: { rate_limit_rate: 0.2 } } });
await Server.start({ configFile: "llmsim.toml", port: 8080 });
```

Properties: `url`, `port`, `host`, `openaiBaseUrl`, `anthropicBaseUrl`,
`openresponsesBaseUrl`, `config`, `isRunning`. Methods: `stats()`, `stop()`.
Also exported: `startServer(options)`, `resolveConfig(options)`, `version`.

Full docs: <https://github.com/chaliy/llmsim/blob/main/docs/bindings.md>
