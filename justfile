# llmsim development recipes

# Show available recipes
default:
    @just --list

# Run all pre-PR checks (fmt, clippy, test)
check:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Format all code
fmt:
    cargo fmt --all

# Run the server locally
run *ARGS:
    cargo run -- {{ARGS}}

# Prepare a release: bump version and remind about changelog
release-prepare version:
    #!/usr/bin/env bash
    set -euo pipefail
    CURRENT=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo "Current version: $CURRENT"
    echo "New version: {{version}}"
    sed -i 's/^version = "'"$CURRENT"'"/version = "{{version}}"/' Cargo.toml
    echo ""
    echo "Updated Cargo.toml to version {{version}}"
    echo ""
    echo "Next steps:"
    echo "  1. Update CHANGELOG.md with a new ## [{{version}}] section"
    echo "  2. Run: just release-check"
    echo "  3. Commit with: chore(release): prepare v{{version}}"

# Run release verification checks (fmt, clippy, test, dry-run publish)
release-check:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo publish --dry-run
    @echo ""
    @echo "All release checks passed."

# Create and push a release tag after verifying version consistency
release-tag:
    #!/usr/bin/env bash
    set -euo pipefail
    # Verify clean working tree
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: Working tree is not clean. Commit or stash changes first."
        exit 1
    fi
    VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    TAG="v$VERSION"
    # Check if tag already exists
    if git rev-parse "$TAG" >/dev/null 2>&1; then
        echo "Error: Tag $TAG already exists"
        exit 1
    fi
    echo "Creating tag $TAG..."
    git tag -a "$TAG" -m "Release $TAG"
    echo "Pushing tag $TAG..."
    git push origin "$TAG"
    echo "Tag $TAG created and pushed."
