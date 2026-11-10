"""In-process llmsim server for Python tests.

Example::

    from openai import OpenAI
    from llmsim import Server

    with Server(generator="echo", latency="instant") as sim:
        client = OpenAI(base_url=sim.openai_base_url, api_key="unused")
        reply = client.chat.completions.create(
            model="gpt-5", messages=[{"role": "user", "content": "hi"}]
        )
"""

from __future__ import annotations

import json
import os
from collections.abc import Mapping
from typing import Any

from llmsim._llmsim import LlmSimError, NativeServer, __version__
from llmsim._llmsim import resolve_config as _resolve_config
from llmsim._llmsim import start as _start

__all__ = ["LlmSimError", "Server", "__version__"]


def _merge(base: dict[str, Any], overrides: Mapping[str, Any]) -> dict[str, Any]:
    out = dict(base)
    for key, value in overrides.items():
        if isinstance(value, Mapping) and isinstance(out.get(key), Mapping):
            out[key] = _merge(dict(out[key]), value)
        else:
            out[key] = value
    return out


class Server:
    """An llmsim HTTP server running inside this process.

    Configuration mirrors the ``llmsim serve`` TOML file. Pass a whole file
    (``config_file=``/``toml=``), a dict shaped like it (``config=``), and/or
    the shortcut keywords below; later sources win. By default the server
    binds ``127.0.0.1`` on a free port.

    Args:
        config: Dict shaped like the TOML config, e.g. ``{"errors": {"rate_limit_rate": 0.1}}``.
        toml: TOML config text.
        config_file: Path to a TOML config file.
        host: Bind address. Default ``127.0.0.1``.
        port: Bind port. Default ``0`` (pick a free port).
        generator: ``lorem``, ``echo``, ``random``, ``sequence`` or ``fixed:<text>``.
        target_tokens: Target response length in tokens.
        latency: Named latency profile (``instant``, ``fast``, ``gpt5``, ``claude-sonnet``...).
        script: Path to a scripted-mode YAML/JSON file.
        scenarios: Path to a scenarios file or directory.
    """

    def __init__(
        self,
        config: Mapping[str, Any] | None = None,
        *,
        toml: str | None = None,
        config_file: str | os.PathLike[str] | None = None,
        host: str = "127.0.0.1",
        port: int = 0,
        generator: str | None = None,
        target_tokens: int | None = None,
        latency: str | None = None,
        script: str | os.PathLike[str] | None = None,
        scenarios: str | os.PathLike[str] | None = None,
    ) -> None:
        if toml is not None and config_file is not None:
            raise ValueError("pass either toml= or config_file=, not both")
        if config_file is not None:
            with open(config_file, encoding="utf-8") as f:
                toml = f.read()

        overrides: dict[str, Any] = {"server": {"host": host, "port": port}}
        if config:
            overrides = _merge(overrides, config)
        response: dict[str, Any] = {}
        if generator is not None:
            response["generator"] = generator
        if target_tokens is not None:
            response["target_tokens"] = target_tokens
        if script is not None:
            response["script_path"] = os.fspath(script)
        if scenarios is not None:
            response["scenarios_path"] = os.fspath(scenarios)
        if response:
            overrides = _merge(overrides, {"response": response})
        if latency is not None:
            overrides = _merge(overrides, {"latency": {"profile": latency}})

        self._toml = toml
        self._overrides = json.dumps(overrides)
        # Validate eagerly so a bad config fails at construction, not start().
        self._config: dict[str, Any] = json.loads(_resolve_config(self._toml, self._overrides))
        self._native: NativeServer | None = None

    @property
    def config(self) -> dict[str, Any]:
        """The effective configuration (defaults + all sources), as a dict."""
        return json.loads(json.dumps(self._config))

    # -- lifecycle -----------------------------------------------------------

    def start(self) -> Server:
        """Bind and start serving. Returns ``self``. No-op if already running."""
        if self._native is None:
            self._native = _start(self._toml, self._overrides)
        return self

    def stop(self) -> None:
        """Gracefully stop the server. Idempotent."""
        native, self._native = self._native, None
        if native is not None:
            native.stop()

    def __enter__(self) -> Server:
        return self.start()

    def __exit__(self, *exc: object) -> None:
        self.stop()

    async def __aenter__(self) -> Server:
        return self.start()

    async def __aexit__(self, *exc: object) -> None:
        self.stop()

    # -- addresses -----------------------------------------------------------

    def _running(self) -> NativeServer:
        if self._native is None:
            raise LlmSimError("server is not running; call start() or use it as a context manager")
        return self._native

    @property
    def is_running(self) -> bool:
        return self._native is not None and self._native.is_running

    @property
    def url(self) -> str:
        """Base URL, e.g. ``http://127.0.0.1:41234``."""
        return self._running().url

    @property
    def port(self) -> int:
        return self._running().port

    @property
    def host(self) -> str:
        return self._running().host

    @property
    def openai_base_url(self) -> str:
        """Base URL for the OpenAI SDK (``.../openai/v1``)."""
        return f"{self.url}/openai/v1"

    @property
    def anthropic_base_url(self) -> str:
        """Base URL for the Anthropic SDK (``.../anthropic``; the SDK appends ``/v1``)."""
        return f"{self.url}/anthropic"

    @property
    def openresponses_base_url(self) -> str:
        """Base URL for OpenResponses clients (``.../openresponses/v1``)."""
        return f"{self.url}/openresponses/v1"

    def stats(self) -> dict[str, Any]:
        """Request, token, and latency counters (same as ``GET /llmsim/stats``)."""
        return json.loads(self._running().stats_json())

    def __repr__(self) -> str:
        state = self._native.url if self._native is not None else "stopped"
        return f"Server({state})"
