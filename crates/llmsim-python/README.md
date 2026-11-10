# llmsim (Python)

In-process [llmsim](https://github.com/chaliy/llmsim) server for Python tests:
a fast LLM API simulator for the OpenAI (Chat Completions, Responses, Images),
Anthropic Messages, and OpenResponses APIs. No model, no network, no API key.
The server is the same Rust code as the `llmsim` CLI, shipped as a native wheel.

```bash
pip install llmsim
```

## Usage

```python
from openai import OpenAI
from llmsim import Server

with Server(generator="echo", latency="instant") as sim:
    client = OpenAI(base_url=sim.openai_base_url, api_key="unused")
    reply = client.chat.completions.create(model="gpt-5", messages=[{"role": "user", "content": "hello"}])
    assert reply.choices[0].message.content == "Echo: hello"
    print(sim.stats()["total_requests"])  # 1
```

Anthropic:

```python
import anthropic

with Server(latency="instant") as sim:
    client = anthropic.Anthropic(base_url=sim.anthropic_base_url, api_key="unused")
```

pytest fixture:

```python
import pytest
from llmsim import Server


@pytest.fixture(scope="session")
def llm():
    with Server(latency="instant") as sim:
        yield sim
```

## Configuration

`Server(...)` accepts the same settings as the `llmsim serve` TOML file. Sources
merge in this order, later wins: `config_file=` or `toml=`, then `config=` (a
dict shaped like the TOML), then the shortcut keywords.

| Keyword | Meaning |
|---|---|
| `host`, `port` | Bind address. Defaults `127.0.0.1` and `0` (a free port). |
| `generator` | `lorem` (default), `echo`, `random`, `sequence`, `fixed:<text>` |
| `target_tokens` | Target response length |
| `latency` | Named profile: `instant`, `fast`, `gpt5`, `claude-sonnet`, ... |
| `script` | Scripted-mode file (exact replies, tool calls, per turn) |
| `scenarios` | Load-test scenario file or directory |

```python
Server({"errors": {"rate_limit_rate": 0.2, "server_error_rate": 0.05}})
Server(config_file="llmsim.toml", port=8080)
```

Properties: `url`, `port`, `host`, `openai_base_url`, `anthropic_base_url`,
`openresponses_base_url`, `config`, `is_running`. Methods: `start()`, `stop()`,
`stats()`. `Server` is a sync and async context manager.

See the [configuration docs](https://github.com/chaliy/llmsim/tree/main/docs)
for every option.
