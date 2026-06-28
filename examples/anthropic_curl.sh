#!/usr/bin/env bash
# Anthropic Messages API examples for llmsim using raw curl / HTTP.
#
# Mirrors the official Anthropic REST API shape, so these requests work
# unchanged against api.anthropic.com (with a real key) and against llmsim.
#
# Prerequisites:
#     Start the llmsim server first:
#         llmsim serve --port 8080
#
# Usage:
#     ./examples/anthropic_curl.sh
#
# Environment variables:
#     LLMSIM_URL: Server base URL (default: http://localhost:8080/anthropic)
set -euo pipefail

BASE_URL="${LLMSIM_URL:-http://localhost:8080/anthropic}"

# Common Anthropic headers. llmsim ignores auth, but real clients send these.
HEADERS=(
  -H "content-type: application/json"
  -H "x-api-key: not-needed"
  -H "anthropic-version: 2023-06-01"
)

echo "=================================================="
echo "Anthropic Messages API (curl) + LLMSim"
echo "Connecting to: ${BASE_URL}"
echo "=================================================="

echo
echo "1. Simple message"
echo "--------------------------------------------------"
curl -s "${HEADERS[@]}" "${BASE_URL}/v1/messages" \
  -d '{
    "model": "claude-opus-4-8",
    "max_tokens": 64,
    "system": "You are concise.",
    "messages": [{"role": "user", "content": "What is the capital of France?"}]
  }'
echo

echo
echo "2. Streaming message (Server-Sent Events)"
echo "--------------------------------------------------"
curl -s -N "${HEADERS[@]}" "${BASE_URL}/v1/messages" \
  -d '{
    "model": "claude-haiku-4-5",
    "max_tokens": 48,
    "stream": true,
    "messages": [{"role": "user", "content": "Write a haiku about code"}]
  }'
echo

echo
echo "3. Multimodal-style request (text + image block)"
echo "--------------------------------------------------"
curl -s "${HEADERS[@]}" "${BASE_URL}/v1/messages" \
  -d '{
    "model": "claude-opus-4-8",
    "max_tokens": 64,
    "messages": [{"role": "user", "content": [
      {"type": "text", "text": "Describe this image"},
      {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo="}}
    ]}]
  }'
echo

echo
echo "4. List models"
echo "--------------------------------------------------"
curl -s "${HEADERS[@]}" "${BASE_URL}/v1/models"
echo

echo
echo "5. Retrieve a model"
echo "--------------------------------------------------"
curl -s "${HEADERS[@]}" "${BASE_URL}/v1/models/claude-sonnet-4-6"
echo

echo
echo "Done."
