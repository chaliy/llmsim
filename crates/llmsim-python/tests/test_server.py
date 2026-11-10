import json
import socket
import urllib.error
import urllib.request

import pytest

import llmsim
from llmsim import LlmSimError, Server


def _get(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=10) as resp:
        return json.loads(resp.read())


def _post(url: str, body: dict) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps(body).encode(), headers={"content-type": "application/json"}, method="POST"
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def test_version_matches_package():
    assert llmsim.__version__.count(".") == 2


def test_context_manager_serves_and_stops():
    with Server(latency="instant") as sim:
        assert sim.is_running
        assert sim.port > 0
        assert sim.url == f"http://127.0.0.1:{sim.port}"
        assert _get(f"{sim.url}/health")["status"] == "ok"
        port = sim.port
    assert not sim.is_running
    with pytest.raises(OSError):
        socket.create_connection(("127.0.0.1", port), timeout=1)


def test_echo_generator_via_raw_http_and_stats():
    with Server(generator="echo", latency="instant") as sim:
        reply = _post(
            f"{sim.openai_base_url}/chat/completions",
            {"model": "gpt-5", "messages": [{"role": "user", "content": "ping pong"}]},
        )
        assert reply["choices"][0]["message"]["content"] == "Echo: ping pong"
        stats = sim.stats()
        assert stats["total_requests"] == 1
        assert stats["completions_requests"] == 1


def test_config_dict_and_toml_merge():
    toml = '[response]\ngenerator = "fixed:from toml"\ntarget_tokens = 5\n'
    sim = Server({"latency": {"profile": "instant"}}, toml=toml, target_tokens=9)
    assert sim.config["response"]["generator"] == "fixed:from toml"
    assert sim.config["response"]["target_tokens"] == 9
    assert sim.config["latency"]["profile"] == "instant"
    with sim:
        reply = _post(
            f"{sim.openai_base_url}/chat/completions",
            {"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]},
        )
        assert reply["choices"][0]["message"]["content"] == "from toml"


def test_config_file(tmp_path):
    path = tmp_path / "llmsim.toml"
    path.write_text('[response]\ngenerator = "fixed:from file"\n[latency]\nprofile = "instant"\n')
    with Server(config_file=path) as sim:
        assert sim.config["response"]["generator"] == "fixed:from file"


def test_error_injection_returns_429():
    with Server({"errors": {"rate_limit_rate": 1.0}}, latency="instant") as sim:
        with pytest.raises(urllib.error.HTTPError) as err:
            _post(
                f"{sim.openai_base_url}/chat/completions",
                {"model": "gpt-5", "messages": [{"role": "user", "content": "x"}]},
            )
        assert err.value.code == 429
        assert sim.stats()["rate_limit_errors"] == 1


def test_start_stop_idempotent_and_restart():
    sim = Server(latency="instant")
    assert sim.start() is sim
    first = sim.url
    sim.start()
    assert sim.url == first
    sim.stop()
    sim.stop()
    with pytest.raises(LlmSimError):
        _ = sim.url
    sim.start()
    assert _get(f"{sim.url}/health")["status"] == "ok"
    sim.stop()


def test_invalid_config_fails_at_construction():
    with pytest.raises(ValueError):
        Server(toml="not = [valid")
    with pytest.raises(ValueError):
        Server({"server": {"port": "eighty"}})
    with pytest.raises(ValueError):
        Server(toml="", config_file="x.toml")


def test_bind_conflict_raises():
    with Server() as first:
        with pytest.raises(LlmSimError):
            Server(port=first.port).start()


def test_missing_script_raises():
    with pytest.raises(LlmSimError):
        Server(script="/nonexistent/script.yaml").start()


def test_many_servers_in_parallel():
    servers = [Server(latency="instant").start() for _ in range(5)]
    try:
        assert len({s.port for s in servers}) == 5
        for s in servers:
            assert _get(f"{s.url}/health")["status"] == "ok"
    finally:
        for s in servers:
            s.stop()


def test_async_context_manager():
    import asyncio

    async def main():
        async with Server(latency="instant") as sim:
            return _get(f"{sim.url}/health")["status"]

    assert asyncio.run(main()) == "ok"
