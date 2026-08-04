# mcp-fs (Rust)

A **streamable-HTTP MCP server** exposing a **simulated multi-project filesystem**:
55 `fs.*` / `admin.*` / `git.*` tools, a parallel REST data plane at `/api/fs` with
OpenAPI docs, and an optional Git HTTP smart server. Runs from a single binary with
**no external service by default**.

Metadata tree = **SQLite** (one db per volume). File bytes = **local filesystem**
(default) or **MinIO/S3**, content-addressed by sha256. ACL = **SQLite**. Auth =
verified **RS256 bearer JWT**.

This is a Rust port of the [C# implementation](https://github.com/smorand/mcp-fs-csharp)
with **strict 1:1 external parity**: same tool names, same snake_case parameters, same
`ERR_*` codes, same JSON shapes, same SQLite schemas, same git wire protocol, same REST
routes. An existing MCP, REST or git client cannot tell the two apart, and a volume
written by one is readable by the other.

## Quickstart

```bash
git clone git@github.com:smorand/mcp-fs-rust.git && cd mcp-fs-rust
./run.sh            # generates keys, bootstraps config/local.yaml, builds, serves :5002
```

In another terminal:

```bash
TOKEN=$(./target/release/mcp-fs token you@example.com --key .keys/jwt.key)

curl http://127.0.0.1:5002/health

# list the tools
curl -s -X POST http://127.0.0.1:5002/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "X-Forwarded-Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# call one
curl -s -X POST http://127.0.0.1:5002/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "X-Forwarded-Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call",
       "params":{"name":"admin.list_projects","arguments":{}}}'
```

Interactive API docs: <http://127.0.0.1:5002/api/docs> (spec at `/api/swagger.json`).

## CLI

```
mcp-fs serve [--config PATH]      run the server
mcp-fs keys  [--dir DIR]          generate an RS256 keypair (default .keys)
mcp-fs token <email> [--key PATH] [--ttl SECONDS]
mcp-fs version
```

Config resolution: `--config`, else `$MCP_FS_CONFIG`, else
`${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}.yaml`.

## Configuration

`config/local.yaml` is your personal working copy and is **gitignored**. Two templates
are tracked, copy one:

```bash
cp config/local.yaml.template config/local.yaml   # SQLite + local blobs, zero services
cp config/minio.yaml.template config/local.yaml   # SQLite + MinIO/S3 blobs
```

`${VAR}` and `${VAR:-default}` are expanded from the environment before the YAML is
parsed, so **secrets never live in a committed file**. Put them in a gitignored `.env`
(template: `.env.example`), which `run.sh` sources:

| Variable | Purpose |
|---|---|
| `MCPFS_MINIO_SECRET_KEY` | S3/MinIO secret key |
| `MCPFS_GITHUB_CLIENT_SECRET` | GitHub App secret for the `git.auth` device flow |
| `MCPFS_TOKEN_KEY` | 32 byte base64 key; when set, OAuth tokens are persisted encrypted (AES-256-GCM). Unset means in-memory only |

Full schema in [`.agent_docs/config.md`](.agent_docs/config.md).

## What lives where

| Data | Backend | Location |
|---|---|---|
| File tree metadata | SQLite | `state/volumes/{project}.db` |
| File bytes | local fs or S3 | `state/blobs/{bucket}/{sha[..2]}/{sha}` or bucket `mcpfs-{project}` |
| Projects and ACL | SQLite | `state/admin.db` |
| Git objects and refs | blob store + SQLite | key `git:{sha}`, `state/git/{project}.db` |
| OAuth tokens (opt-in) | encrypted SQLite | `state/oauth.db` |

`state/` is the whole database. Back it up, and note that in MinIO mode the bytes live
in the bucket while the metadata stays in `state/`: a project needs **both halves**.

## Security model

- **Authentication**: RS256 JWT, signature, issuer and expiry verified (30s clock skew,
  matching the reference implementation). Read from `X-Forwarded-Authorization`, then
  `Authorization`. Basic auth is accepted with the token as the password, for git CLI use.
- **Authorization**: a project has an owner and members. Every `fs.*` and `git.*` tool
  requires membership.
- **Separation of duties**: a *platform admin* (`auth.admins`) manages projects and
  membership and can list everything, but does **not** get implicit access to a
  project's files. An admin who needs the files adds itself as a member.
- **Safety rails**: path normalization (NUL rejected, traversal contained), must-read
  before-write, per-session write quota, capped audit log, soft delete to
  `.mcp_trash/` unless hard delete is enabled.
- Secrets are read from the environment and never logged.

## Build and test

```bash
./build.sh                       # cargo build --release
./test.sh                        # cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

The suite is the quality gate and must be green before any commit.

## Interactive CLI agent

`crates/agent` builds an `agent` binary that drives the 55 tools through an LLM. It is a
**client**, so it exercises the real MCP wire protocol the way any other client would.

```bash
./run.sh &                                                  # server on :5002
mkdir -p .agent_keys
./target/release/mcp-fs token you@example.com \
    --key .keys/jwt.key > .agent_keys/you                   # one raw JWT per file
export IBM_ICA_MODEL_KEY=...                                # or put it in .env
./agent.sh --user you
```

Configured by `config/agent_test.yaml`: MCP endpoint, token directory, and any OpenAI
compatible chat endpoint. `--conversation ID` resumes a transcript, `/help` lists the
commands. It also works non interactively, which makes it scriptable:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you
```

`.agent_keys/` (bearer tokens) and `.agent_history/` (transcripts) are gitignored. See
[`.agent_docs/agent.md`](.agent_docs/agent.md) for the terminal invariants it relies on.

## Parity harness

The objective judge of 1:1 parity. It replays a corpus of MCP and REST calls against a
server and diffs against a golden capture of the C# reference:

```bash
# capture from the reference implementation
cargo run -p parity-harness -- capture \
  --base http://127.0.0.1:5002 --token "$CS_TOKEN" \
  --owner admin@example.com --out parity-golden.json

# compare this implementation against it
cargo run -p parity-harness -- compare \
  --base http://127.0.0.1:5003 --token "$RUST_TOKEN" \
  --owner admin@example.com --golden parity-golden.json
```

Volatile values (timestamps, version, host paths) are normalized, and an error text is
reduced to `tool + ERR_* code` so a reworded message passes while a wrong code fails.
`parity-golden.json` is the committed baseline.

## Deliberate divergences from the C#

Each is a case where mirroring the reference would mirror a defect. The full table, with
the harness step that proves each one, is in [`.agent_docs/parity.md`](.agent_docs/parity.md).

**Errors are usable.** The reference answers a missing file or a missing argument with
`"An error occurred invoking 'fs.read'."` carrying **no error code** (its storage layer
raises a bare `IOException`, which is not an `McpException`), so a client cannot tell a
missing file from a bad argument from a crash. Here every failure carries its `ERR_*`
code. On the REST plane the reference maps six codes and defaults the rest to a generic
400, so a spent quota, a missing read precondition, an ambiguous match and an unsupported
format were indistinguishable by status; here they are 429, 428, 409 and 501, and a
missing file is 404 rather than 500.

**A real `git clone` and `git push` work.** Three reference defects made the documented
git protocol unusable: `upload-pack` did not advertise `multi_ack_detailed` (which git
requires over smart HTTP), the pack was built without a commit's ancestry so any
repository with more than one commit was incomplete, and the `receive-pack` report was not
side-band framed so git aborted after the push had landed. Also: `unpack ok` is sent,
project membership is enforced on the git routes (the reference let any verified token
read or write any project), `max_pack_size_mb` is enforced, and pushed objects are really
indexed.

**Data safety and accounting on the REST plane.** Four routes called the storage layer
directly instead of the engine, so the REST door behaved differently from the tool for the
same operation: `delete` skipped the trash, ignored `allow_hard_delete` and removed a whole
tree without asking for `recursive`; `move` had no no clobber rule; `upload` charged
nothing against the write quota, making the highest volume write path the only one with no
accounting; and none of them wrote an audit entry, so a REST mutation left no trace. All
four now go through the engine. The git write paths had the same gap: `git.remote_clone`
imported a whole working tree and `git.checkout_file` restored a file with nothing charged
against the quota, so git was a way around it. The clone is now charged up front, before
the first write, so an import that does not fit leaves the volume untouched instead of half
populated. Related engine bug found on the way: `fs.move` with `overwrite: true` always
failed, because the flag was checked and then ignored.

**Correctness fixes.** `auth.jwt.algorithms` is honoured instead of parsed and ignored
(with unsupported names logged at startup and the HMAC family refused on purpose).
Listing a file is a 400 rather than a 200 with an invented empty listing. `fs.tree` at
exactly the node cap returns every node instead of dropping the last one and claiming to
be truncated. Symbol references come back ordered by line. An invalid `fs.grep` regex is
a stable 400.

**Structural.** One implementation per operation in `core::fs_ops`, shared by the MCP
surface and the REST plane, including the V4A patch engine. No custom libgit2 ODB backend
(`git2` cannot express one from safe Rust): the blob store is the source of truth and is
synced around libgit2 calls, with identical stored bytes.

## Not supported

Audio and video extraction (needs a speech model) and legacy binary Office formats
(`.doc`, `.xls`, `.ppt`), same as the reference. `object_format: sha256` is accepted and
ignored: the bundled libgit2 is sha1 only.

## Documentation

`AGENTS.md` is the compact index. Details live in [`.agent_docs/`](.agent_docs/):
architecture, tools, api, git, config, testing, parity, agent.

## License

MIT
