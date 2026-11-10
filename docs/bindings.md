# Python and Node.js / TypeScript

The `llmsim` packages on [PyPI](https://pypi.org/project/llmsim/) and
[npm](https://www.npmjs.com/package/llmsim) run the llmsim server inside your
process. Start one in a test, point the official OpenAI or Anthropic SDK at it,
stop it when done. No binary to install, no Docker, no API key.

Both packages are native: the same Rust server as `llmsim serve`, prebuilt for
Linux (x86_64, arm64; glibc and, for Python, musl), macOS (Intel, Apple
silicon), and Windows x64. Python needs 3.9+, Node needs 18+.

## Python

```bash
pip install llmsim
```

```python
from openai import OpenAI
from llmsim import Server

with Server(generator="echo", latency="instant") as sim:
    client = OpenAI(base_url=sim.openai_base_url, api_key="unused")
    reply = client.chat.completions.create(
        model="gpt-5", messages=[{"role": "user", "content": "hello"}]
    )
    print(reply.choices[0].message.content)  # Echo: hello
    print(sim.stats()["total_requests"])     # 1
```

As a pytest fixture:

```python
import pytest
from llmsim import Server

@pytest.fixture(scope="session")
def llm():
    with Server(latency="instant") as sim:
        yield sim

def test_agent(llm):
    client = anthropic.Anthropic(base_url=llm.anthropic_base_url, api_key="unused")
    ...
```

`Server` also works with `async with`, and with explicit `start()` / `stop()`.

## Node.js / TypeScript

```bash
npm install --save-dev llmsim
```

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
  console.log(reply.choices[0].message.content); // Echo: hello
} finally {
  await sim.stop();
}
```

With explicit resource management (TypeScript 5.2+, Node 20.4+ or a polyfill
for `Symbol.asyncDispose`): `await using sim = await Server.start()`.

Starting and stopping never block the event loop, and a server you forget to
stop does not keep the process alive.

## Configuration

Options map to the [TOML config file](../README.md#toml-config-file). Sources
merge in this order, later wins:

1. A TOML file (`config_file` / `configFile`) or TOML text (`toml`)
2. A dict / object shaped like the TOML (`config`)
3. Shortcut options

| Python | TypeScript | Meaning |
|---|---|---|
| `host` | `host` | Bind address. Default `127.0.0.1` |
| `port` | `port` | Bind port. Default `0`, a free port |
| `generator` | `generator` | `lorem` (default), `echo`, `random`, `sequence`, `fixed:<text>` |
| `target_tokens` | `targetTokens` | Target response length in tokens |
| `latency` | `latency` | Latency profile: `instant`, `fast`, `gpt5`, `claude-sonnet`, ... |
| `script` | `script` | [Scripted mode](../knowledge/simulation/scripted-mode.md) file |
| `scenarios` | `scenarios` | [Scenario](scenarios.md) file or directory |

Error injection, for example:

```python
Server({"errors": {"rate_limit_rate": 0.2, "server_error_rate": 0.05}})
```

```ts
await Server.start({ config: { errors: { rate_limit_rate: 0.2 } } });
```

An invalid config raises `ValueError` in Python (when `Server(...)` is
constructed) and rejects `Server.start()` in Node. A port that is already in
use, or a script file that cannot be read, fails at start (`LlmSimError` in
Python).

## Server properties

| Python | TypeScript | Value |
|---|---|---|
| `url` | `url` | `http://127.0.0.1:<port>` |
| `port`, `host` | `port`, `host` | Bound address |
| `openai_base_url` | `openaiBaseUrl` | `<url>/openai/v1` (OpenAI SDK `base_url`) |
| `anthropic_base_url` | `anthropicBaseUrl` | `<url>/anthropic` (Anthropic SDK `base_url`) |
| `openresponses_base_url` | `openresponsesBaseUrl` | `<url>/openresponses/v1` |
| `config` | `config` | Effective config |
| `stats()` | `stats()` | Same as `GET /llmsim/stats` |
| `is_running` | `isRunning` | Whether it is serving |

See [api.md](api.md) for the endpoints.
