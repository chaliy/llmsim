import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { Server, resolveConfig, startServer, version } from "../wrapper.js";

const chat = (sim, content) =>
  fetch(`${sim.openaiBaseUrl}/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ model: "gpt-5", messages: [{ role: "user", content }] }),
  });

const portIsClosed = (port) =>
  new Promise((resolve) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    socket.once("connect", () => {
      socket.destroy();
      resolve(false);
    });
    socket.once("error", () => resolve(true));
  });

test("version is semver", () => {
  assert.match(version, /^\d+\.\d+\.\d+/);
});

test("starts on a free port, serves, and stops", async () => {
  const sim = await Server.start({ latency: "instant" });
  assert.ok(sim.isRunning);
  assert.ok(sim.port > 0);
  assert.equal(sim.url, `http://127.0.0.1:${sim.port}`);
  const health = await (await fetch(`${sim.url}/health`)).json();
  assert.equal(health.status, "ok");
  await sim.stop();
  assert.equal(sim.isRunning, false);
  assert.ok(await portIsClosed(sim.port));
  await sim.stop(); // idempotent
  assert.throws(() => sim.stats(), /stopped/);
});

test("echo generator and stats", async () => {
  const sim = await startServer({ generator: "echo", latency: "instant" });
  try {
    const reply = await (await chat(sim, "ping pong")).json();
    assert.equal(reply.choices[0].message.content, "Echo: ping pong");
    const stats = sim.stats();
    assert.equal(stats.total_requests, 1);
    assert.equal(stats.completions_requests, 1);
  } finally {
    await sim.stop();
  }
});

test("config sources merge: toml < config < shortcuts", async () => {
  const toml = '[response]\ngenerator = "fixed:from toml"\ntarget_tokens = 5\n';
  const cfg = resolveConfig({ toml, config: { latency: { profile: "instant" } }, targetTokens: 9 });
  assert.equal(cfg.response.generator, "fixed:from toml");
  assert.equal(cfg.response.target_tokens, 9);
  assert.equal(cfg.latency.profile, "instant");

  const sim = await Server.start({ toml, latency: "instant" });
  try {
    const reply = await (await chat(sim, "x")).json();
    assert.equal(reply.choices[0].message.content, "from toml");
    assert.equal(sim.config.response.generator, "fixed:from toml");
  } finally {
    await sim.stop();
  }
});

test("configFile", async () => {
  const dir = mkdtempSync(join(tmpdir(), "llmsim-"));
  const file = join(dir, "llmsim.toml");
  writeFileSync(file, '[response]\ngenerator = "fixed:from file"\n');
  assert.equal(resolveConfig({ configFile: file }).response.generator, "fixed:from file");
});

test("error injection returns 429", async () => {
  const sim = await Server.start({ config: { errors: { rate_limit_rate: 1.0 } }, latency: "instant" });
  try {
    const res = await chat(sim, "x");
    assert.equal(res.status, 429);
    await res.body?.cancel();
    assert.equal(sim.stats().rate_limit_errors, 1);
  } finally {
    await sim.stop();
  }
});

test("invalid config is rejected", async () => {
  assert.throws(() => resolveConfig({ toml: "not = [valid" }));
  assert.throws(() => resolveConfig({ config: { server: { port: "eighty" } } }));
  assert.throws(() => resolveConfig({ toml: "", configFile: "x.toml" }), TypeError);
  await assert.rejects(Server.start({ config: { server: { port: "eighty" } } }));
});

test("bind conflict and missing script reject", async () => {
  const first = await Server.start();
  try {
    await assert.rejects(Server.start({ port: first.port }), /bind/i);
  } finally {
    await first.stop();
  }
  await assert.rejects(Server.start({ script: "/nonexistent/script.yaml" }), /script/i);
});

test("many servers in parallel get distinct ports", async () => {
  const servers = await Promise.all(Array.from({ length: 5 }, () => Server.start({ latency: "instant" })));
  try {
    assert.equal(new Set(servers.map((s) => s.port)).size, 5);
  } finally {
    await Promise.all(servers.map((s) => s.stop()));
  }
});

test("running server does not keep the process alive after stop", async () => {
  // node --test fails the file on a hang, so reaching the end is the check.
  const sim = await Server.start();
  await sim.stop();
});
