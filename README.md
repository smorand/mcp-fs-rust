# mcp-fs (Rust)

A **streamable-HTTP MCP server** exposing a **simulated multi-project filesystem**:
55 `fs.*` / `admin.*` / `git.*` tools, a parallel REST data plane at `/api/fs` with
OpenAPI docs, and an optional Git HTTP smart server. Runs from a single binary with
**no external service by default**.

Server state (metadata tree, ACL, git index, OAuth tokens) = **SQLite** by default, or
**PostgreSQL** or **SQL Server** chosen per store in the config. File bytes = **local
filesystem** (default) or **MinIO/S3**, content-addressed by sha256. Auth = verified
**RS256 bearer JWT**.

This began as a Rust port of the
[C# implementation](https://github.com/smorand/mcp-fs-csharp). **That 1:1 parity
constraint is retired**: the project now has its own lifecycle, which is what allowed the
relational backends to exist. The tool surface is still a contract and is still pinned by
tests. See [`.agent_docs/parity.md`](.agent_docs/parity.md) for the lineage and the full
record of where behaviour diverged.

## Cargo features

A default build is SQLite only and carries no database driver beyond bundled `rusqlite`.

| Feature | Adds | Driver |
|---|---|---|
| (default) | SQLite | `rusqlite`, bundled |
| `postgres` | PostgreSQL | `sqlx` |
| `sqlserver` | SQL Server | `tiberius-ng` + `bb8` |
| `all-backends` | both | |

```bash
cargo build --release --features postgres
cargo build --release --features all-backends
```

SQL Server needs a second driver because **sqlx removed its MSSQL support in 0.7** and the
rewrite has never shipped. Both are optional so a default build carries neither the
dependency nor its TLS stack.

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
mcp-fs migrate --from A.yaml --to B.yaml   copy relational state between backends
mcp-fs version
```

Config resolution, highest priority first: `--config`, else `$MCP_FS_CONFIG`, else
`~/.config/mcp-fs/config.yaml` when it exists, else
`${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}.yaml`. The first two are used
as given; the last two are probed, and when neither exists the error lists every path
tried.

### Switching backend with data in place

`migrate` is how you move an existing deployment onto PostgreSQL or SQL Server. It copies
every row the server owns, from the engines one config names to the engines the other
names.

```bash
mcp-fs migrate --from config/sqlite.yaml --to config/postgres.yaml
```

**Run it with the server stopped**: it takes no locks against a live writer, so a
concurrent write would be missed. The copy is row for row rather than a replay through the
store API, so every `mtime` and `ctime` is preserved exactly.

Two things are deliberately not moved. **Blob bytes** stay put, because the blob store is
configured separately and a relational change does not affect it; every `sha256` is
instead checked against the destination blob store and reported when missing. **OAuth
tokens** are skipped, because they are session state encrypted with `MCPFS_TOKEN_KEY` and
copying ciphertext to a deployment with a different key would produce rows that never
decrypt. A device flow re establishes them.

## Configuration

`config/local.yaml` is your personal working copy and is **gitignored**. Two templates
are tracked, copy one:

```bash
cp config/local.yaml.template config/local.yaml   # SQLite + local blobs, zero services
cp config/minio.yaml.template config/local.yaml   # SQLite + MinIO/S3 blobs
```

Each of the four relational stores is configured independently under `infra`, so you can
move them one at a time:

```yaml
infra:
  meta:                          # the per volume file tree
    backend: postgres            # sqlite | postgres | sqlserver
    dsn: "${MCPFS_META_DSN}"     # secret, required for a server engine
    schema: mcpfs                # postgres only
    pool: { max_connections: 20, acquire_timeout_secs: 10 }
  admin: { backend: postgres, dsn: "${MCPFS_META_DSN}" }   # projects and ACL
  git:   { backend: postgres, dsn: "${MCPFS_META_DSN}" }   # git index
  oauth: { backend: sqlite }                               # OAuth tokens
```

Pointing several stores at one dsn is the expected setup: table names do not collide and
one connection pool is shared. Misconfiguration fails at boot, not on first use: a missing
dsn, a dsn set on a `sqlite` store, an unknown backend name, or a backend whose cargo
feature was not compiled in are each rejected with the offending key named.

`${VAR}` and `${VAR:-default}` are expanded from the environment before the YAML is
parsed, so **secrets never live in a committed file**. Put them in a gitignored `.env`
(template: `.env.example`), which `run.sh` sources:

| Variable | Purpose |
|---|---|
| `MCPFS_MINIO_SECRET_KEY` | S3/MinIO secret key |
| `MCPFS_GITHUB_CLIENT_SECRET` | GitHub App secret for the `git.auth` device flow |
| `MCPFS_TOKEN_KEY` | 32 byte base64 key; when set, OAuth tokens are persisted encrypted (AES-256-GCM). Unset means in-memory only |
| any name you pick | a store `dsn`, for example `${MCPFS_META_DSN}` |

A dsn normally carries a password, so it is treated as a secret: the type redacts itself in
both `Debug` and `Display`, and a test asserts it never reaches a log line, a boot banner
or an error message.

Full schema in [`.agent_docs/config.md`](.agent_docs/config.md).

## What lives where

| Data | Backend | Location under the SQLite default |
|---|---|---|
| File tree metadata | `infra.meta` | `state/volumes/{project}.db` |
| File bytes | local fs or S3 | `state/blobs/{bucket}/{sha[..2]}/{sha}` or bucket `mcpfs-{project}` |
| Projects and ACL | `infra.admin` | `state/admin.db` |
| Git objects and refs | blob store + `infra.git` | key `git:{sha}`, `state/git/{project}.db` |
| OAuth tokens (opt-in) | `infra.oauth`, encrypted | `state/oauth.db` |
| Git working directories | always on disk | `state/git-repos/{project}/` |

`state/` is the whole database under the default. Back it up, and note that in MinIO mode
the bytes live in the bucket while the metadata stays in `state/`: a project needs **both
halves**.

On PostgreSQL or SQL Server one database holds every volume, discriminated by a
`volume_id` column, since creating a database per project on the fly is not viable. Blob
bytes never move into the relational database, and `state/git-repos/` stays on disk
because libgit2 needs a real working directory.

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
cargo clippy --all-targets --all-features -- -D warnings
```

The suite is the quality gate and must be green before any commit. Use `--all-features` on
clippy, otherwise the two optional drivers are never compiled and their warnings never
surface.

The default run needs no database and no Docker. To exercise PostgreSQL and SQL Server,
bring the services up and pass their dsn; the cases skip themselves when the variables are
absent:

```bash
docker compose -f docker-compose.test.yml up -d

MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs \
MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' \
  cargo test --workspace --all-features

docker compose -f docker-compose.test.yml down -v
```

One conformance suite runs the same assertions against every engine, which is what proves a
store behaves identically on all three. See
[`.agent_docs/testing.md`](.agent_docs/testing.md).

## Interactive CLI agent

`crates/agent` builds an `agent` binary that drives the 55 tools through an LLM. It is a
**client**, so it exercises the real MCP wire protocol the way any other client would.

```bash
mkdir -p .agent_keys
./target/release/mcp-fs token you@example.com \
    --key .keys/jwt.key > .agent_keys/you                   # one raw JWT per file
export IBM_ICA_MODEL_KEY=...                                # or put it in .env
./agent.sh --user you
```

`agent.sh` starts the server itself when nothing answers on the configured endpoint, logging
to `mcp_<datetime>.log`, and stops it again when the agent exits. A server that was already
running is left alone. A watchdog keyed on the agent's pid covers the case where the wrapper
is killed outright.

Configured by `config/agent_test.yaml`: MCP endpoint, token directory, and any OpenAI
compatible chat endpoint. `--conversation ID` resumes a transcript, `/help` lists the
commands. It also works non interactively, which makes it scriptable:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you
```

`.agent_keys/` (bearer tokens) and `.agent_history/` (transcripts) are gitignored. See
[`.agent_docs/agent.md`](.agent_docs/agent.md) for the terminal invariants it relies on.

The line editor is verified on a real pty, because terminal geometry bugs pass every unit
test: a wrong prompt width is a mistake at the call site, not in the width function.

```bash
cargo build -p agent -p mcp-fs && python3 scripts/pty_check.py
```

## Regression harness

A 128 step corpus of MCP and REST calls, replayed against a running server and diffed
against a golden capture. It was built as the 1:1 parity judge and is **no longer a gate**,
since there is nothing left to be equal to; it is kept because the corpus covers the MCP
surface, the REST plane and every error path more broadly than the unit tests do. Point it
at a previous build to use it as a regression check:

```bash
cargo run -p parity-harness -- capture \
  --base http://127.0.0.1:5002 --token "$TOKEN" \
  --owner admin@example.com --out baseline.json

# change something, restart, then
cargo run -p parity-harness -- compare \
  --base http://127.0.0.1:5002 --token "$TOKEN" \
  --owner admin@example.com --golden baseline.json
```

Volatile values (timestamps, version, host paths) are normalized, and an error text is
reduced to `tool + ERR_* code` so a reworded message passes while a wrong code fails.
`parity-golden.json` is the committed C# baseline and still backs the tool schema equality
tests.

## Divergences from the C# origin

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
(`.doc`, `.xls`, `.ppt`). `object_format: sha256` is accepted and ignored: the bundled
libgit2 is sha1 only.

On the relational side: there is no online migration (`mcp-fs migrate` is offline, run with
the server stopped) and no automated downgrade, since a database written by the current
code carries a `volume_id` column older code does not know about. Blob bytes are never
stored in the relational database.

## Documentation

`AGENTS.md` is the compact index. Details live in [`.agent_docs/`](.agent_docs/):
architecture, tools, api, git, config, backends, testing, parity, agent.

## License

MIT
