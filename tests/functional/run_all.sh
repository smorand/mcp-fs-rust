#!/usr/bin/env bash
# Functional test runner — drives the agent CLI with piped prompts and validates outputs.
#
# Usage:
#   ./tests/functional/run_all.sh [--user NAME] [FILTER]
#
#   --user NAME   agent key file under .agent_keys/ (default: admin)
#   FILTER        run only tests whose name contains this string
#
# Examples:
#   ./tests/functional/run_all.sh
#   ./tests/functional/run_all.sh --user seb.morand@gmail.com
#   ./tests/functional/run_all.sh crud
#   ./tests/functional/run_all.sh --user admin git
#
# Prerequisites:
#   1. Server running:  ./run.sh
#   2. Agent built:     cargo build --release -p agent
#   3. Token file:      .agent_keys/<user>  (see agent.sh header)
#   4. LLM key set:     export IBM_ICA_MODEL_KEY=...  (or whichever api_key_env names)
#
# The runner starts and stops the server itself when it is not already up.

set -euo pipefail
cd "$(dirname "$0")/../.."     # project root

# ── colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'
pass() { echo -e "  ${GREEN}✓${RESET} $1"; }
fail() { echo -e "  ${RED}✗${RESET} $1"; }
info() { echo -e "  ${CYAN}→${RESET} $1"; }

# ── args ──────────────────────────────────────────────────────────────────────
USER_ARG="admin"
FILTER=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --user) USER_ARG="$2"; shift 2 ;;
    *)      FILTER="$1";   shift   ;;
  esac
done

# ── server lifecycle ──────────────────────────────────────────────────────────
SERVER_PID=""
probe() { curl -fsS -m 2 "http://127.0.0.1:5002/health" >/dev/null 2>&1; }

if ! probe; then
  info "Starting mcp-fs server..."
  if [ ! -f config/local.yaml ]; then cp config/local.yaml.template config/local.yaml; fi
  if [ ! -f .keys/jwt.pub ]; then cargo run --release -q -p mcp-fs -- keys --dir .keys; fi
  cargo build --release -p mcp-fs -q
  ./target/release/mcp-fs serve --config config/local.yaml >/tmp/mcp-fs-test.log 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 50); do probe && break; sleep 0.2; done
  probe || { echo "Server failed to start. Log:"; tail -20 /tmp/mcp-fs-test.log; exit 1; }
  info "Server started (pid $SERVER_PID)"
else
  info "Server already running"
fi

cleanup() {
  [[ -n "$SERVER_PID" ]] && kill -TERM "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# ── agent build ───────────────────────────────────────────────────────────────
cargo build --release -p agent -q

# ── agent wrapper ─────────────────────────────────────────────────────────────
# Run the agent with piped prompts. Returns the full stdout.
# Usage: run_agent "prompt one\nprompt two\n..."
run_agent() {
  local prompts="$1"
  printf '%s' "$prompts" | \
    ./target/release/agent --user "$USER_ARG" 2>/dev/null || true
}

# ── assertion helpers ─────────────────────────────────────────────────────────
PASS=0; FAIL=0; SKIP=0
CURRENT_SUITE=""

suite() {
  CURRENT_SUITE="$1"
  echo ""
  echo -e "${BOLD}$1${RESET}"
}

assert_contains() {
  local label="$1" output="$2" expected="$3"
  if echo "$output" | grep -qi "$expected"; then
    pass "$label"
    PASS=$((PASS+1))
  else
    fail "$label  (expected to find: '$expected')"
    FAIL=$((FAIL+1))
    if [[ "${VERBOSE:-0}" == "1" ]]; then
      echo "    --- output ---"
      echo "$output" | head -30 | sed 's/^/    /'
      echo "    ---"
    fi
  fi
}

assert_not_contains() {
  local label="$1" output="$2" not_expected="$3"
  if echo "$output" | grep -qi "$not_expected"; then
    fail "$label  (must NOT contain: '$not_expected')"
    FAIL=$((FAIL+1))
  else
    pass "$label"
    PASS=$((PASS+1))
  fi
}

skip() {
  echo -e "  ${YELLOW}−${RESET} $1 (skipped: $2)"
  SKIP=$((SKIP+1))
}

should_run() { [[ -z "$FILTER" ]] || echo "$1" | grep -qi "$FILTER"; }

# ── load individual test scripts ──────────────────────────────────────────────
SCRIPT_DIR="$(dirname "$0")/scenarios"
for f in "$SCRIPT_DIR"/[0-9][0-9]_*.sh; do
  name="$(basename "$f" .sh | sed 's/^[0-9]*_//')"
  if should_run "$name"; then
    # shellcheck source=/dev/null
    source "$f"
  fi
done

# ── summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}─────────────────────────────────${RESET}"
TOTAL=$((PASS+FAIL+SKIP))
echo -e "  Total: $TOTAL  ${GREEN}✓ $PASS${RESET}  ${RED}✗ $FAIL${RESET}  ${YELLOW}− $SKIP${RESET}"
echo ""
[[ $FAIL -eq 0 ]]
