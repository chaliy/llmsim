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
   - Update comparison links at bottom of file

3. **Update Cargo.toml**
   - Set `version = "X.Y.Z"`

4. **Run verification**
   - `cargo fmt --check`
   - `cargo clippy`
   - `cargo test`

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

**On GitHub Release created** (publish.yml):
- Runs verification (fmt, clippy, tests)
- Publishes to crates.io

## Pre-Release Checklist

The agent verifies before creating a release PR:

- [ ] All CI checks pass on main
- [ ] `cargo fmt` - code is formatted
- [ ] `cargo clippy` - no warnings
- [ ] `cargo test` - all tests pass
- [ ] CHANGELOG.md has entries for changes since last release

## Workflows

### release.yml

- **Trigger**: Push to `main` with commit message starting with `chore(release): prepare v`
- **Actions**: Creates GitHub Release with tag and release notes
- **File**: `.github/workflows/release.yml`

### publish.yml

- **Trigger**: GitHub Release published
- **Actions**: Verifies and publishes to crates.io
- **File**: `.github/workflows/publish.yml`
- **Secret required**: `CARGO_REGISTRY_TOKEN`

## Example Conversation

```
Human: Create release v0.2.0

Agent: I'll prepare the v0.2.0 release. Let me:
1. Update CHANGELOG.md with the v0.2.0 section
2. Update Cargo.toml version to 0.2.0
3. Run verification checks
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
