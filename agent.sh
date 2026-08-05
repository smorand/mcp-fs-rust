#!/usr/bin/env bash
# Interactive CLI agent driving the mcp-fs tools through an LLM.
#
# Starts the server itself when nothing is listening, logging to mcp_<datetime>.log, and
# stops it again when the agent exits. A server that was already running is left alone:
# it belongs to whoever started it.
#
# Mint a token for yourself once:
#   mkdir -p .agent_keys
#   ./target/release/mcp-fs token you@example.com --key .keys/jwt.key > .agent_keys/you
# Then:
#   ./agent.sh --user you
set -euo pipefail
cd "$(dirname "$0")"

# .env carries the LLM key and any local secrets, and is gitignored.
if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

SERVER_CONFIG="config/local.yaml"
SERVER_PID=""          # set only when this script starts the server
SERVER_LOG=""
WATCHDOG_PID=""
AGENT_RC=0

# ── lifecycle ────────────────────────────────────────────────────────────────────

# Stop the server, but only the one we started. Runs from the EXIT trap, so it also
# covers a crash, a closed terminal, and Ctrl+C reaching the script.
cleanup() {
  if [ -n "$WATCHDOG_PID" ]; then
    kill -TERM "$WATCHDOG_PID" 2>/dev/null || true
  fi
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    printf '  stopping mcp-fs (pid %s)\n' "$SERVER_PID"
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    # Give it five seconds to flush and close its database, then insist.
    for _ in $(seq 1 50); do
      kill -0 "$SERVER_PID" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 "$SERVER_PID" 2>/dev/null; then
      kill -KILL "$SERVER_PID" 2>/dev/null || true
    fi
  fi
}
trap cleanup EXIT
# Turn a signal into a normal exit so the EXIT trap runs. Without this the server would
# outlive a Ctrl+C or a closed terminal.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# Outlive our own death: a SIGKILL leaves no chance to run the EXIT trap, so a detached
# watcher tied to the AGENT's pid guarantees the server does not become an orphan.
#
# It watches the agent, not this shell, on purpose. Watching the shell would tear the
# server down under a session that is still alive and still using it.
start_watchdog() {
  (
    while kill -0 "$1" 2>/dev/null; do sleep 1; done
    kill -TERM "$2" 2>/dev/null || true
    sleep 3
    kill -KILL "$2" 2>/dev/null || true
  ) >/dev/null 2>&1 &
  WATCHDOG_PID=$!
}

# Is something answering as an mcp-fs server on this host and port?
# /health needs no bearer, and answering it proves the HTTP server is up rather than
# merely that the port is held by something.
probe() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS -m 2 "http://$1:$2/health" >/dev/null 2>&1
  else
    # No curl: fall back to a plain TCP connect.
    (exec 3<>"/dev/tcp/$1/$2") 2>/dev/null
  fi
}

# ── build ────────────────────────────────────────────────────────────────────────

cargo build --release -p agent

# The endpoint comes from the agent itself, so --config and $AGENT_CONFIG are honoured and
# there is no second YAML parser to keep in step.
MCP_URL="$(./target/release/agent --print-mcp-url "$@")"
# http://host:port/path -> host and port, with the usual defaults.
HOSTPORT="${MCP_URL#*://}"
HOSTPORT="${HOSTPORT%%/*}"
HOST="${HOSTPORT%%:*}"
PORT="${HOSTPORT##*:}"
if [ "$PORT" = "$HOST" ]; then
  case "$MCP_URL" in
    https://*) PORT=443 ;;
    *) PORT=80 ;;
  esac
fi

# ── server ───────────────────────────────────────────────────────────────────────

if probe "$HOST" "$PORT"; then
  printf '  mcp-fs already running at %s:%s, leaving it alone\n' "$HOST" "$PORT"
else
  # Same bootstrap as run.sh: a personal config and a dev keypair on first use.
  if [ ! -f "$SERVER_CONFIG" ]; then
    printf '  %s not found, copying from %s.template\n' "$SERVER_CONFIG" "$SERVER_CONFIG"
    cp "${SERVER_CONFIG}.template" "$SERVER_CONFIG"
  fi
  if [ ! -f .keys/jwt.pub ]; then
    printf '  generating a dev keypair in .keys\n'
    cargo run --release --quiet -p mcp-fs -- keys --dir .keys
  fi
  cargo build --release -p mcp-fs

  SERVER_LOG="mcp_$(date +%Y%m%d-%H%M%S).log"
  ./target/release/mcp-fs serve --config "$SERVER_CONFIG" >"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  printf '  started mcp-fs (pid %s), logging to %s\n' "$SERVER_PID" "$SERVER_LOG"

  # Wait for it to answer, and fail loudly rather than handing the agent a dead endpoint.
  up=0
  for _ in $(seq 1 100); do
    if probe "$HOST" "$PORT"; then
      up=1
      break
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      printf '\n  mcp-fs exited during startup. Last lines of %s:\n\n' "$SERVER_LOG" >&2
      tail -n 20 "$SERVER_LOG" >&2
      exit 1
    fi
    sleep 0.1
  done
  if [ "$up" -ne 1 ]; then
    printf '\n  mcp-fs did not answer at %s:%s within 10s.\n' "$HOST" "$PORT" >&2
    printf '  The agent expects %s, so check the port in %s.\n\n' "$MCP_URL" "$SERVER_CONFIG" >&2
    tail -n 20 "$SERVER_LOG" >&2
    exit 1
  fi
fi

# ── agent ────────────────────────────────────────────────────────────────────────

# Deliberately not exec: this shell has to outlive the agent to stop the server.
#
# The agent runs in the background only so its pid is known to the watchdog. `<&0` is load
# bearing: a non interactive shell points an asynchronous job's stdin at /dev/null unless
# a redirection says otherwise, and that would leave the line editor with no terminal.
./target/release/agent "$@" <&0 &
AGENT_PID=$!
if [ -n "$SERVER_PID" ]; then
  start_watchdog "$AGENT_PID" "$SERVER_PID"
fi

# The exit code is forwarded so a caller still sees success or failure.
wait "$AGENT_PID" || AGENT_RC=$?
exit "$AGENT_RC"
