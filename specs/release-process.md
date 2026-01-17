# Release Process

## Abstract

This document describes the release process for LLMSim, including versioning strategy, release workflow, and automation.

## Versioning

LLMSim follows [Semantic Versioning](https://semver.org/):

- **MAJOR** (X.0.0): Breaking API changes
- **MINOR** (0.X.0): New features, backward compatible
- **PATCH** (0.0.X): Bug fixes, backward compatible

## Release Workflow

### Overview

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  Prepare        │     │  GitHub         │     │  crates.io      │
│  Release PR     │────>│  Release        │────>│  Publish        │
│                 │     │  (automatic)    │     │  (automatic)    │
└─────────────────┘     └─────────────────┘     └─────────────────┘
```

### Step 1: Prepare Release

1. **Update CHANGELOG.md**
   - Move items from `[Unreleased]` to new version section
   - Add release date: `## [X.Y.Z] - YYYY-MM-DD`
   - Update comparison links at bottom of file

2. **Update version in Cargo.toml**
   ```toml
   version = "X.Y.Z"
   ```

3. **Create release commit**
   ```bash
   git add CHANGELOG.md Cargo.toml
   git commit -m "chore(release): prepare vX.Y.Z"
   ```

4. **Create PR and merge to main**
   - PR title: `chore(release): prepare vX.Y.Z`
   - Get review and merge

### Step 2: Automated Release (CI)

When the release commit is pushed to `main`, the release workflow automatically:

1. Extracts version from commit message
2. Verifies `Cargo.toml` version matches
3. Extracts release notes from `CHANGELOG.md`
4. Creates GitHub Release with tag `vX.Y.Z`

### Step 3: Automated Publish (CI)

When the GitHub Release is created, the publish workflow automatically:

1. Runs verification (fmt, clippy, tests)
2. Publishes to crates.io

## Pre-Release Checklist

Before preparing a release:

- [ ] All CI checks pass on main
- [ ] `cargo fmt` - code is formatted
- [ ] `cargo clippy` - no warnings
- [ ] `cargo test` - all tests pass
- [ ] Documentation is up to date
- [ ] CHANGELOG.md has entries for all changes

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

## Example Release

```bash
# 1. Update CHANGELOG.md with new version section
# 2. Update Cargo.toml version

# 3. Commit changes
git add CHANGELOG.md Cargo.toml
git commit -m "chore(release): prepare v0.2.0"

# 4. Push (or create PR)
git push origin main

# CI automatically:
# - Creates GitHub Release v0.2.0
# - Publishes to crates.io
```

## Hotfix Releases

For urgent fixes:

1. Create fix on `main` branch
2. Follow normal release process with patch version bump
3. Example: `v0.1.0` -> `v0.1.1`

## Release Artifacts

Each release includes:

- **GitHub Release**: Tag, release notes, source archives
- **crates.io**: Published crate for `cargo install llmsim`

Future considerations:
- Pre-built binaries (Linux, macOS, Windows)
- Docker images
- Homebrew formula
