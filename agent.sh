#!/usr/bin/env bash
# Interactive CLI agent driving the mcp-fs tools through an LLM.
#
# Start the server first (./run.sh), then mint a token for yourself:
#   mkdir -p .agent_keys
#   ./target/release/mcp-fs token you@example.com --key .keys/jwt.key > .agent_keys/you
# Then:
#   ./agent.sh --user you
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# .env carries the LLM key, and is gitignored.
if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

cargo build --release -p agent
exec ./target/release/agent "$@"
