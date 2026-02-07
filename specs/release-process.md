# Release Process

## Abstract

This document describes the release process for LLMSim. Releases are initiated by asking a coding agent to prepare the release, with CI automation handling the rest.

## Versioning

LLMSim follows [Semantic Versioning](https://semver.org/):

- **MAJOR** (X.0.0): Breaking API changes
- **MINOR** (0.X.0): New features, backward compatible
- **PATCH** (0.0.X): Bug fixes, backward compatible

## Release Workflow

### Overview

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  Human asks     │     │  Agent creates  │     │  GitHub         │     │  crates.io      │
│  "release v0.2" │────>│  release PR     │────>│  Release        │────>│  Publish        │
│                 │     │                 │     │  (automatic)    │     │  (automatic)    │
└─────────────────┘     └─────────────────┘     └─────────────────┘     └─────────────────┘
```

### Human Steps

1. **Ask the agent** to create a release:
   - "Create release v0.2.0"
   - "Prepare a patch release"
   - "Release the current changes as v0.2.0"

2. **Review the PR** created by the agent

3. **Merge to main** - CI handles GitHub Release and crates.io publish

### Agent Steps (automated)

When asked to create a release, the agent:

1. **Determine version**
   - Use version specified by human, OR
   - Suggest next version based on changes (patch/minor/major)

2. **Update CHANGELOG.md**
   - Move items from `[Unreleased]` to new version section
   - Add release date: `## [X.Y.Z] - YYYY-MM-DD`
   - Add breaking changes section if applicable (see format below)
   - List commits in GitHub-style format with PR links and contributors
   - Update comparison links at bottom of file

3. **Update Cargo.toml**
   - Set `version = "X.Y.Z"`

4. **Run verification**
   - `just release-check` (runs fmt, clippy, test, and dry-run publish)
   - Or manually: `cargo fmt --check`, `cargo clippy`, `cargo test`, `cargo publish --dry-run`

5. **Commit and push**
   - Commit message: `chore(release): prepare vX.Y.Z`
   - Push to feature branch

6. **Create PR**
   - Title: `chore(release): prepare vX.Y.Z`
   - Include changelog excerpt in description

### CI Automation

**On merge to main** (release.yml):
- Detects commit message `chore(release): prepare vX.Y.Z`
- Extracts release notes from CHANGELOG.md
- Creates GitHub Release with tag `vX.Y.Z`
- Also supports `workflow_dispatch` for manual re-runs (reads version from Cargo.toml)

**On GitHub Release created** (publish.yml):
- Verifies release tag matches Cargo.toml version
- Runs verification (fmt, clippy, tests)
- Runs `cargo publish --dry-run` before actual publish
- Publishes to crates.io

## Pre-Release Checklist

The agent verifies before creating a release PR:

- [ ] All CI checks pass on main
- [ ] `cargo fmt` - code is formatted
- [ ] `cargo clippy` - no warnings
- [ ] `cargo test` - all tests pass
- [ ] `cargo publish --dry-run` - package is valid
- [ ] CHANGELOG.md has entries for changes since last release

## Justfile Recipes

The project provides `just` recipes for common release tasks:

| Recipe | Description |
|--------|-------------|
| `just release-prepare <version>` | Bumps Cargo.toml version and shows next steps |
| `just release-check` | Runs fmt, clippy, test, and dry-run publish |
| `just release-tag` | Creates and pushes a git tag (with safety checks) |
| `just check` | Runs pre-PR checks (fmt, clippy, test) |

## Changelog Format

The changelog follows [Keep a Changelog](https://keepachangelog.com/) with GitHub-style commit listings.

### Structure

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Breaking Changes

- **Short description**: Detailed explanation of what changed and migration steps.
  - Before: `/old/path`
  - After: `/new/path`

### What's Changed

- Commit message ([#PR](https://github.com/chaliy/llmsim/pull/PR)) by @contributor
- Another commit ([#PR](https://github.com/chaliy/llmsim/pull/PR)) by @contributor
```

### Generating Commit List

Get commits since last release, excluding chore/ci/bench commits:

```bash
git log --oneline | grep -v -E "^.{7} (chore|ci|bench)"
```

Format each commit as:
```
- <commit message> ([#<PR>](https://github.com/chaliy/llmsim/pull/<PR>)) by @<author>
```

### Breaking Changes Section

Include when the release has breaking changes (typically MINOR or MAJOR versions):

1. **Bold summary** of the breaking change
2. **Migration guide** showing before/after
3. **Code examples** if helpful

Example:
```markdown
### Breaking Changes

- **API endpoints now require provider prefix**: All provider-specific endpoints are prefixed with the provider name.
  - `/v1/chat/completions` → `/openai/v1/chat/completions`
  - When using SDKs, configure base URL with provider prefix:
    ```python
    client = OpenAI(base_url="http://localhost:3000/openai/v1", api_key="not-needed")
    ```
```

## CI Security & Quality Checks

CI runs the following checks on every push and PR:

| Check | Tool | Purpose |
|-------|------|---------|
| Vulnerability audit | `cargo-audit` | Detects known vulnerabilities in dependencies |
| License compliance | `cargo-deny` | Ensures all dependencies use permissive licenses |
| Documentation | `cargo doc -D warnings` | Catches broken doc links and warnings |
| Formatting | `cargo fmt --check` | Consistent code style |
| Linting | `cargo clippy -D warnings` | Code quality and correctness |
| Tests | `cargo test` | Unit and integration tests |
| Cross-platform build | `cargo build --release` | Linux, macOS, Windows |

Configuration for `cargo-deny` is in `deny.toml` at the repository root.

## Workflows

### release.yml

- **Trigger**: Push to `main` with commit message starting with `chore(release): prepare v`, or manual `workflow_dispatch`
- **Actions**: Creates GitHub Release with tag and release notes
- **Permissions**: `contents: write`, `actions: write`
- **File**: `.github/workflows/release.yml`

### publish.yml

- **Trigger**: GitHub Release published
- **Actions**: Verifies tag/version match, runs checks, dry-run publish, then publishes to crates.io
- **File**: `.github/workflows/publish.yml`
- **Secret required**: `CARGO_REGISTRY_TOKEN`

## Example Conversation

```
Human: Create release v0.2.0

Agent: I'll prepare the v0.2.0 release. Let me:
1. Update CHANGELOG.md with the v0.2.0 section
2. Update Cargo.toml version to 0.2.0
3. Run `just release-check` for verification
4. Create the release PR

[Agent performs steps...]

Done. PR created: https://github.com/llmsim/llmsim/pull/XX
Please review and merge to trigger the release.
```

## Hotfix Releases

For urgent fixes:

1. Ask agent: "Create patch release v0.1.1 for the auth fix"
2. Agent prepares release with patch version
3. Review and merge

## Release Artifacts

Each release includes:

- **GitHub Release**: Tag, release notes, source archives
- **crates.io**: Published crate for `cargo install llmsim`

Future considerations:
- Pre-built binaries (Linux, macOS, Windows)
- Docker images
- Homebrew formula
