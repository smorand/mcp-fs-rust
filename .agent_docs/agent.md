# The `agent` crate: interactive CLI agent

`crates/agent` is a third workspace member producing the `agent` binary. It is a **client**,
not part of the server: it speaks MCP over HTTP to whatever endpoint the config names, so it
exercises the real wire protocol the way any other client would. That is the point, it is the
end to end test of the tool surface.

Run it with `./agent.sh`, which sources `.env`, builds, **starts the server if nothing is
listening**, and stops that server again when the agent exits.

## Setup

```bash
mkdir -p .agent_keys
./target/release/mcp-fs token you@example.com \
    --key .keys/jwt.key > .agent_keys/you                   # one raw JWT per file
export IBM_ICA_MODEL_KEY=...                                # or put it in .env
./agent.sh --user you                                       # starts the server if needed
```

## Server lifecycle in `agent.sh`

The endpoint comes from `agent --print-mcp-url`, so `--config` and `$AGENT_CONFIG` are
honoured and there is no second YAML parser in shell to drift out of step. The script probes
`/health` at that host and port, which needs no bearer and proves the HTTP server answers
rather than merely that the port is held.

| Situation | Behaviour |
|---|---|
| Something answers already | left alone, it belongs to whoever started it |
| Nothing answers | bootstrap `config/local.yaml` and `.keys` like `run.sh`, start `mcp-fs serve` in the background logging to `mcp_<datetime>.log`, wait for `/health` |
| Started but never answers | print the reason and the tail of the log, then exit non zero, having stopped it |
| Agent exits, crashes, Ctrl+C, terminal closed | the EXIT trap stops the server (SIGTERM, then SIGKILL after five seconds) |
| The wrapper itself is SIGKILLed | a watchdog on the AGENT's pid stops the server once the agent ends |

Three details are load bearing:

**Only the server we started is stopped.** `SERVER_PID` stays empty when the probe succeeded,
so a session against a server from `./run.sh` cannot take it down.

**The watchdog watches the agent, not the wrapper.** A `SIGKILL` on the wrapper leaves no
chance to run a trap, so a detached watcher keyed on the agent's pid closes that hole. Keying
it on the wrapper would be worse than the leak: it would tear the server down under a session
that is still alive and still using it. Verified by killing the wrapper with `-9`, confirming
the session survives, then killing the agent and watching the server go.

**`<&0` on the agent is not decoration.** The agent runs in the background only so the
watchdog knows its pid, and a non interactive shell points an asynchronous job's stdin at
`/dev/null` unless a redirection says otherwise. Without `<&0` the line editor has no
terminal: measured directly, `isatty` is false and `tcsetattr` fails with
`Operation not supported by device`. The editor is re verified through the wrapper, not only
against the binary.

`.agent_keys/` and `.agent_history/` are gitignored: the first holds bearer tokens, the
second holds transcripts.

## Configuration

`config/agent_test.yaml`, same keys and defaults as the reference implementation, so one file
drives either. Unknown keys are ignored, every key has a default.

| Key | Default | Notes |
|---|---|---|
| `mcp.url` | `http://127.0.0.1:5002/mcp` | the MCP endpoint |
| `mcp.token` | empty | fallback when `--user` is absent |
| `mcp.auth_header` | `X-Forwarded-Authorization` | the server reads this before `Authorization` |
| `mcp.tokens_dir` | `.agent_keys` | one file per user, file name equals the `--user` value |
| `llm.base_url` | ICA endpoint | any OpenAI compatible `/chat/completions` |
| `llm.model` | `claude-haiku-4-5` | |
| `llm.api_key_env` | `IBM_ICA_MODEL_KEY` | read before `llm.api_key`, so no secret in YAML |
| `llm.timeout_seconds` | 180 | ceiling on one turn, a hung endpoint cannot freeze the agent |
| `system_prompt` | see file | the exact tool names are appended automatically |

Config resolution: `--config`, then `$AGENT_CONFIG`, then `config/agent_test.yaml`.

## Commands

`/help`, `/history`, `/conversation ID`, `/clear`, `exit` or Ctrl+D. A trailing backslash
continues the prompt on the next line, because a terminal cannot report Shift+Enter.

## Module map

| Module | Responsibility |
|---|---|
| `config.rs` | the YAML schema, defaults, and `api_key()` preferring the environment |
| `mcp.rs` | `tools/list` and `tools/call`; accepts a plain JSON body or an SSE framed one; fuzzy name resolution |
| `llm.rs` | OpenAI compatible streaming with tool calling; reassembles fragmented `tool_calls` |
| `input.rs` | the wrap aware line editor, history, and the non TTY fallback |
| `ui.rs` | markdown to ANSI, the tool call and result lines |
| `spinner.rs` | the gated one line spinner |
| `session.rs` | `.jsonl` transcripts under `.agent_history/` |
| `main.rs` | CLI, wiring, the conversation and agentic loops |

## Things that are load bearing

**The server is stateless, so there is no `initialize` handshake.** `mcp.rs` posts a bare
`tools/call`. This is why the reference had to run with `Stateless = true` and why the port
hand rolls the JSON-RPC layer instead of using an SDK that mandates the handshake.

**Models mangle tool names.** `admin_list_projects` arrives instead of `admin.list_projects`,
because many function calling schemas forbid a dot. `mcp::resolve_name` matches with dots and
underscores treated as equivalent. Observed live: the model emitted the underscored form and
the call still landed. Removing this makes roughly one call in ten fail for no good reason.

**The streaming callback is synchronous, so the spinner needs a sync stop.** Tokens arrive on
the main task while the spinner draws on its own. Both writers share a gate, and the stop flag
is checked *inside* it, so once `Silencer::silence()` returns no further frame can appear. Two
bugs were found here by watching a real pty session, both fixed and both regression tested:

- Assigning `None` to the spinner slot dropped the struct without stopping the task, leaking
  it for the rest of the process; frames then accumulated over every later prompt. `Spinner`
  now has a `Drop` guard, and the loop always goes through `stop_if_running`.
- Before the gate existed, frames landed between tokens (`Je ne suis pas certain` `- thinking`
  ` de comprendre`). Measured on a driven pty: 6 interleavings before, 0 after.

**The markdown repaint only works on a terminal.** The agent streams raw tokens, then rewinds
the cursor and repaints the answer as markdown. Piped or redirected, the escapes erase nothing
and the repaint would duplicate the whole answer, so the raw stream is left as the final text.
The rewind counts **real screen rows**, not newlines: `rows_consumed` adds `width / cols` per
line, so a wrapped line does not leave debris. Counting newlines alone, as the reference did,
breaks as soon as one line is longer than the terminal.

**Widths are display widths, and escapes count for nothing.** `input.rs` measures with
`unicode-width`, so a CJK or emoji character occupying two cells does not desynchronise the
wrap arithmetic; the reference counted characters. It also measures through
`visible_width`, which skips ANSI sequences. That second part was a bug I shipped: the
prompt is `"\n\x1b[36m❯\x1b[0m "`, whose escape bytes were counted as printable, so
`prompt_cols` was 9 instead of 2. Every repositioning then landed **seven columns too far
right**. Plain typing takes a fast path that never repositions, so nothing showed until a
redraw ran, and it surfaced as an unerasable gap after a backspace:
`❯ abcd       XY`. Verified both ways on a pty.

**The redraw invariant.** Every redraw takes `from_row` (the screen row where the *physical*
cursor currently sits) and `to` (the content cursor it must end up at). `from_row` is computed
by the caller **before** mutating the buffer, because after an edit the widths have shifted
while the terminal cursor has not moved: deriving it inside `redraw` from the new buffer is
wrong whenever the edited characters have different widths. The reference passed the old end
when swapping in a history entry, which over scrolled after `Home` then `Up` on a wrapped
line, repainting one row too high and silently overwriting the output above.

**No TTY means no editing.** `crossterm` raw mode fails on a pipe, so a non terminal stdin
falls back to a plain line read. That is what makes the agent scriptable:
`echo "list my projects" | ./agent.sh --user me`.

## Testing

96 unit tests in the crate, all pure: no test needs a server, an LLM or a terminal. The
streaming accumulator is tested by feeding it chunk JSON directly, which covers the cases that
actually break in the wild: a name split across chunks, out of order indices, a missing call
id, a trailing nameless delta, malformed arguments.

The interactive path cannot be unit tested: a wrong prompt width still passes every assertion
about the width function, because the mistake is at the call site. `scripts/pty_check.py`
drives the real binary through a pty, applies its output to a small virtual screen, and reads
back what a human would see. It is self contained: it starts a server on an ephemeral port in
a temp directory, mints its own token, runs the agent with its cwd inside that directory, and
seeds `.agent_history/readline.txt` so the recall checks never submit a line and never need a
working LLM key.

```bash
cargo build -p agent -p mcp-fs && python3 scripts/pty_check.py
```

Eleven checks: backspace leaving no gap, typing resuming on the text, `Ctrl+U` killing back to
the cursor, `Ctrl+A` then `Ctrl+K`, insert and backspace mid line, a wrapped line staying
intact, backspace across the wrap, recall with the cursor on the first row of a wrapped line,
the recall staying on its own screen row, `Down` restoring the stashed line, and plain recall.

Each check earns its place by failing when the corresponding bug is reintroduced, which was
verified by putting each one back:

| Reintroduced bug | Caught by |
|---|---|
| prompt width counting ANSI escapes | `typing resumes on the text`, `a wrapped line is intact` |
| `from_row` measured from the old end | `the recall stays on its own row` |

The second one is worth noting: the repainted **text** is correct, it just sits one row too
high, so only pinning the absolute row catches it. Checking the input area alone passes.

The spinner bugs were found the same way, by watching a driven pty session rather than by the
test suite.

Manual smoke test, both stdin modes:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you   # piped
./agent.sh --user you                                                     # interactive
./agent.sh --user you --conversation memtest                              # resume
```
