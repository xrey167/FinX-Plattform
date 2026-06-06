#!/usr/bin/env bash
# Idempotent first-run setup for the local Docker Compose stack.
#
# Copies .env.example to .env if .env does not already exist, then fills the
# TDW_MCP_HTTP_TOKEN placeholder with a securely random hex-32 value so a
# non-loopback MCP bind starts. Re-running is safe: an existing .env is left
# untouched, and a TDW_MCP_HTTP_TOKEN that already has a non-placeholder value
# is preserved. Prints what it did.
#
# See docs/CONFIGURATION.md and docs/release/secrets-and-tls.md.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
env_path="$root/.env"
example_path="$root/.env.example"
placeholder="change-me-before-exposing"

if [ ! -f "$example_path" ]; then
  echo ".env.example not found at $example_path" >&2
  exit 1
fi

if [ -f "$env_path" ]; then
  echo ".env already exists at $env_path — leaving it untouched."
else
  cp "$example_path" "$env_path"
  echo "Created .env from .env.example."
fi

token="$(openssl rand -hex 32)"

if grep -qE '^[[:space:]]*TDW_MCP_HTTP_TOKEN[[:space:]]*=' "$env_path"; then
  current="$(grep -E '^[[:space:]]*TDW_MCP_HTTP_TOKEN[[:space:]]*=' "$env_path" | head -n1 | cut -d'=' -f2-)"
  if [ -z "${current// }" ] || [ "$current" = "$placeholder" ]; then
    tmp="$(mktemp)"
    sed "s|^[[:space:]]*TDW_MCP_HTTP_TOKEN[[:space:]]*=.*|TDW_MCP_HTTP_TOKEN=$token|" "$env_path" > "$tmp"
    mv "$tmp" "$env_path"
    echo "Set TDW_MCP_HTTP_TOKEN to a random hex-32 value."
  else
    echo "TDW_MCP_HTTP_TOKEN already set to a non-placeholder value — preserved."
  fi
else
  printf 'TDW_MCP_HTTP_TOKEN=%s\n' "$token" >> "$env_path"
  echo "Appended TDW_MCP_HTTP_TOKEN with a random hex-32 value."
fi

echo "Done. Edit .env to add provider/LLM keys, then: docker compose --profile live up -d --build"
