#!/usr/bin/env bash
# Decision: Agent identity detection uses a broad pattern to catch known AI/bot identities.
# Fallback chain: git config → env vars (GIT_USER_NAME/GIT_USER_EMAIL) → error.
set -euo pipefail

# Pattern matches known AI agent and bot identities
GIT_AGENT_IDENTITY_PATTERN="(claude|cursor|copilot|github-actions|bot|ai-agent|openai|anthropic|gpt)"

# Check if a git identity (name or email) looks like it belongs to an agent/bot.
# Returns 0 if agent-like, 1 if human-like.
git_identity_looks_agent_like() {
  local name="${1:-}"
  local email="${2:-}"
  if [[ "${name,,}" =~ $GIT_AGENT_IDENTITY_PATTERN ]] || [[ "${email,,}" =~ $GIT_AGENT_IDENTITY_PATTERN ]]; then
    return 0
  fi
  return 1
}

# Resolve a human git identity through a fallback chain:
# 1. Current git config user.name/user.email (if not agent-like)
# 2. GIT_USER_NAME / GIT_USER_EMAIL environment variables
# 3. Error
# Outputs: two lines — name then email
resolve_commit_git_identity() {
  local name email

  # Try current git config first
  name="$(git config user.name 2>/dev/null || true)"
  email="$(git config user.email 2>/dev/null || true)"

  if [[ -n "$name" && -n "$email" ]]; then
    if ! git_identity_looks_agent_like "$name" "$email"; then
      echo "$name"
      echo "$email"
      return 0
    fi
  fi

  # Fallback to environment variables
  name="${GIT_USER_NAME:-}"
  email="${GIT_USER_EMAIL:-}"

  if [[ -z "$name" || -z "$email" ]]; then
    echo "ERROR: git identity looks agent-like and no fallback found." >&2
    echo "Set GIT_USER_NAME and GIT_USER_EMAIL to a real user." >&2
    return 1
  fi

  # Guard: reject env vars that also look agent-like
  if git_identity_looks_agent_like "$name" "$email"; then
    echo "ERROR: GIT_USER_NAME/GIT_USER_EMAIL also look agent-like." >&2
    echo "Set them to a real human identity." >&2
    return 1
  fi

  echo "$name"
  echo "$email"
}

# Configure git user.name and user.email if the current identity looks agent-like.
configure_commit_git_identity_if_needed() {
  local name email

  # Check if current identity is fine
  name="$(git config user.name 2>/dev/null || true)"
  email="$(git config user.email 2>/dev/null || true)"

  if [[ -n "$name" && -n "$email" ]]; then
    if ! git_identity_looks_agent_like "$name" "$email"; then
      return 0
    fi
  fi

  # Resolve and apply
  local resolved
  resolved="$(resolve_commit_git_identity)"
  local resolved_name resolved_email
  resolved_name="$(echo "$resolved" | head -1)"
  resolved_email="$(echo "$resolved" | tail -1)"

  git config user.name "$resolved_name"
  git config user.email "$resolved_email"
  echo "Git identity updated: $resolved_name <$resolved_email>"
}
