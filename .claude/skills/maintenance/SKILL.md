---
name: maintenance
description: Run the routine maintenance workflow for llmsim — dependency updates, model profile refresh, code-quality + audit/deny checks, and spec alignment. Use when performing the periodic maintenance pass defined in specs/maintenance.md.
---

# Routine Maintenance Skill

This skill walks the routine-maintenance workflow defined in
[`specs/maintenance.md`](../../../specs/maintenance.md). Use it monthly,
before a minor/major release, or whenever a major-provider releases new
models.

## Prerequisites

The maintenance flow needs three Rust tools beyond the default toolchain.
Install them once per machine; they are not cargo-installed automatically:

```bash
# Security advisory checks (RustSec)
cargo install cargo-audit

# License + duplicate-dep + ban policy checks
cargo install cargo-deny
```

CI runs both of these on every PR (`Security Audit`, `License & Dependency
Check` jobs), but running them locally surfaces issues *before* pushing.

The Rust toolchain itself is pinned via `rust-toolchain.toml` at the repo
root — rustup will pick the right channel automatically.

## Workflow

Follow the order below. Each step maps to a requirement in
`specs/maintenance.md`.

### 1. Dependency updates (R1)

```bash
# Bring the lockfile up to the latest semver-compatible versions.
cargo update

# Surface major-version updates blocked by version constraints.
cargo update --dry-run --verbose

# Security advisories. Fails the run if RustSec has reported anything.
cargo audit

# License + duplicate-dep + advisory policy. Reads ./deny.toml.
cargo deny check
```

For each entry in the `cargo update --dry-run` output, decide:

- **Apply the major bump** if the diff is small (a renamed trait import,
  a tweaked signature) — see PR #34 for the pattern.
- **Skip and note** if it requires a non-trivial refactor; record the
  reason in the maintenance PR description so the next agent doesn't
  re-evaluate from scratch.
- **Watch for archived crates.** `serde_yaml` was at `0.9.34+deprecated`
  before PR #37 migrated to TOML — `+deprecated` build metadata is a
  red flag that no semver bump exists.

### 2. Model profile refresh (R2)

- Cross-check `src/openai/models.rs` against [models.dev](https://models.dev)
  — context window, max output tokens, capabilities, knowledge cutoff,
  release timestamp.
- Add new models from OpenAI, Anthropic, and Google (and DeepSeek where
  appropriate). Mirror the new IDs in `default_models()` in
  `src/cli/config.rs` so they appear in `/openai/v1/models`.
- Update the model table in `specs/architecture.md`.
- Add focused unit tests for each new profile (one assertion per field
  worth caring about; see existing tests as a template).

### 3. Code-quality sweep (R3)

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- Grep for `TODO`, `FIXME`, `HACK`, `XXX` — resolve or document.
- Review API handlers for input validation, panicking paths on user
  input, and resource limits (body sizes, in-memory collection growth).
- Verify CORS configuration is still appropriate for the deployment
  posture.

### 4. Spec + AGENTS.md alignment (R5 + R6)

- Read each file in `specs/` and confirm it still matches the code.
  Update tables, requirement lists, and example snippets where they've
  drifted.
- Verify `AGENTS.md` lists every spec, every skill, and every command.
- Run a quick `grep -rn "PORT_NUMBER\|version-like-string"` for known
  stale references (e.g., the AGENTS.md default-port fix in PR #36).

## Shape of a maintenance pass

A standard pass produces three or four small PRs rather than one omnibus
diff:

1. **`chore(deps)`** — `cargo update` + any major bumps you decided to
   apply. Include the clippy fix if `dtolnay/rust-toolchain` rolled to
   a new version and surfaced new lints.
2. **`feat(models)`** — new model profiles + spec table updates.
3. **`docs(specs)`** — any spec drift discovered while reviewing the
   code.
4. *(optional)* **`chore(...)` / `refactor(...)`** for anything bigger
   that came out of the code-quality sweep.

Open each PR independently against `main` so they can be reviewed and
merged in isolation. See PRs #34 / #35 / #36 / #37 for a worked example.
