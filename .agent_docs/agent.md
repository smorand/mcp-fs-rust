# The `agent` crate: interactive CLI agent

`crates/agent` is a third workspace member producing the `agent` binary. It is a **client**,
not part of the server: it speaks MCP over HTTP to whatever endpoint the config names, so it
exercises the real wire protocol the way any other client would. That is the point, it is the
end to end test of the tool surface.

Run it with `./agent.sh` (sources `.env`, builds release, execs the binary).

## Setup

```bash
./run.sh &                                                  # server on :5002
mkdir -p .agent_keys
./target/release/mcp-fs token you@example.com \
    --key .keys/jwt.key > .agent_keys/you                   # one raw JWT per file
export IBM_ICA_MODEL_KEY=...                                # or put it in .env
./agent.sh --user you
```

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

**Widths are display widths.** `input.rs` measures with `unicode-width`, so a CJK or emoji
character occupying two cells does not desynchronise the wrap arithmetic. The reference
counted characters.

**The redraw invariant.** Every redraw takes `from` (the content cursor where the *physical*
cursor currently sits) and `to` (where it must end up). Deriving the row from the buffer length
instead would move up the wrong number of rows whenever the cursor is not at the end. The
reference passed the old end when swapping in a history entry, which over scrolled after
`Home` then `Up` on a wrapped line; the port passes the real cursor.

**No TTY means no editing.** `crossterm` raw mode fails on a pipe, so a non terminal stdin
falls back to a plain line read. That is what makes the agent scriptable:
`echo "list my projects" | ./agent.sh --user me`.

## Testing

96 unit tests in the crate, all pure: no test needs a server, an LLM or a terminal. The
streaming accumulator is tested by feeding it chunk JSON directly, which covers the cases that
actually break in the wild: a name split across chunks, out of order indices, a missing call
id, a trailing nameless delta, malformed arguments.

The interactive path cannot be unit tested, so it is verified by driving a real pty. The driver
waits for the prompt, types with a typo, fixes it with backspaces, submits, recalls the line
with the up arrow, abandons it with Ctrl+C, then asks for markdown and checks the rendered
escapes. That is how both spinner bugs were found.

Manual smoke test, both stdin modes:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you   # piped
./agent.sh --user you                                                     # interactive
./agent.sh --user you --conversation memtest                              # resume
```
