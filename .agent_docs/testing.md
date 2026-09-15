# Testing

## Layout

There is no `tests/` integration crate (the directory exists and is empty). Every
test is inline, in a `#[cfg(test)] mod tests` next to the code it covers, which
keeps a test in the same file as the invariant it protects.

Three levels, all inside the crate:

| Level | How | Where |
|---|---|---|
| Unit | plain functions and in memory SQLite (`SqliteRelationalDb::open_in_memory`, `RelationalAdminStore::in_memory`) plus `tempfile` dirs for blobs | every module |
| Conformance | one set of assertions run against **every** relational engine | `storage/conformance.rs` |
| Tool level | `tools::testkit::harness()` builds a **real `AppState`** (SQLite metadata in a temp dir, local blobs, in memory ACL, the real registry) and dispatches through `registry.call`, the same path `tools/call` uses | `tools/*` |
| Integration | the **real axum router** driven with `tower::ServiceExt::oneshot`, so requests go through routing, identity, the membership gate and the handlers | `app.rs`, `api/dataplane.rs`, `api/openapi.rs`, `git/http/mod.rs` |

Schema parity is also a test: `tools/mod.rs` and `tools/all.rs` compare all 55
descriptions and `inputSchema` values against `parity-golden.json`, serialized, so
even a property key order change fails the build. Those two tests skip with a
message when the golden file is absent, since it lives at the repo root outside
the crate.

Beyond the crate, `crates/parity-harness` is a binary that replays a corpus
against a **live server** over HTTP. Its own 32 tests cover the corpus and the
normalizer.

## Running

```bash
./test.sh                                    # cargo test --workspace
cargo test -p mcp-fs                         # the server crate only
cargo test -p mcp-fs --lib storage::         # one area
cargo test --workspace --all-features        # includes the postgres and sqlserver drivers
cargo clippy --all-targets --all-features -- -D warnings   # second half of the quality gate
```

Both must be clean before any commit. Use `--all-features` on the clippy gate, otherwise
the two optional drivers are never compiled and their warnings never surface.

## Current counts

From `cargo test --workspace` on the current tree:

| Target | Result |
|---|---|
| `mcp-fs` lib | 965 passed, 1 ignored |
| `agent` bin | 111 passed |
| `parity-harness` bin | 32 passed |
| `mcp-fs` bin | 0 (the binary is a thin `main`) |
| doctests | 0 passed, 3 ignored (wiring examples marked `ignore`) |

That is the default feature set, so SQLite only. The relational conformance cases add
themselves per engine when their dsn is present.

Per area, from `cargo test -p mcp-fs --lib -- --list` (755 tests, the ignored one
included):

| Area | Tests | Area | Tests |
|---|---|---|---|
| `tools` | 255 | `mcp` | 23 |
| `storage` | 132 | `cli` | 23 |
| `core` | 132 | `util` | 21 |
| `git` | 117 | `config` | 21 |
| `docs` | 106 | `identity` | 18 |
| `api` | 71 | `safety` | 12 |
| | | `app` | 11 |
| | | `keys` | 9 |
| | | `migrate` | 5 |
| | | `logging` | 5 |
| | | `errors` | 5 |

Largest single modules: `core::fs_ops` 101, `docs::extract` 49,
`api::dataplane` 48, `git::oauth` 41, `git::http` 40, `tools::git` 36,
`storage::meta` 32.

`storage` grew most, from 62, because the relational layer carries its own suite:
`storage::rel::dialect` 17, `storage::rel::sqlite` 14 and `storage::rel` 12 cover
placeholder rendering, upsert and DDL per dialect, `LIKE` escaping and the retry helper
with no database at all.

## Relational conformance: PostgreSQL and SQL Server

The per store test modules cover their own logic against SQLite. `storage/conformance.rs`
answers a different question: does a store behave **the same** on another engine? Every
case takes an `Engine` and runs once per backend.

SQLite always runs. The server engines run only when their dsn is in the environment, so
`cargo test --workspace` stays green with no Docker and nothing installed.

| Variable | Enables |
|---|---|
| `MCPFS_TEST_PG_DSN` | the PostgreSQL cases |
| `MCPFS_TEST_MSSQL_DSN` | the SQL Server cases |

```bash
docker compose -f docker-compose.test.yml up -d      # postgres:16 on 55432, mssql 2022 on 51433

MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs \
MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' \
  cargo test --workspace --all-features

docker compose -f docker-compose.test.yml down -v
```

The ports are deliberately unusual so this never collides with a real PostgreSQL on 5432
or SQL Server on 1433. Both services declare a healthcheck because the tests connect
immediately, and SQL Server needs tens of seconds before it accepts a login. PostgreSQL
data is on `tmpfs`: throwaway, and faster.

`--all-features` is required, since the drivers are behind the `postgres` and `sqlserver`
cargo features. Without them the dsn variables are ignored.

**Isolation differs per engine, and that is the point.** SQLite hands out a private in
memory database per call, while the server engines share one instance, so every case
derives unique ids from a per run tag. A case that passes on all three has been proven not
to depend on having the database to itself, which is what catches a statement missing its
`volume_id` predicate.

## Opt in test

One test is `#[ignore]`: `storage::blob::s3::tests::integration_put_get_range_delete`.
It needs a live S3 compatible service, because faking S3 would test the fake and
not the SDK wiring (path style addressing, range requests, bucket lifecycle).

Requirements: a server on `http://127.0.0.1:9000` with access key `admin` (both
hardcoded in the test fixture) and the secret in `MCPFS_MINIO_SECRET_KEY`. It
creates and removes its own random bucket, so it leaves nothing behind.

```bash
docker run -d -p 9000:9000 -e MINIO_ROOT_USER=admin -e MINIO_ROOT_PASSWORD=secret \
  quay.io/minio/minio server /data

MCPFS_MINIO_SECRET_KEY=secret \
  cargo test -p mcp-fs --lib storage::blob::s3 -- --ignored
```

Without the service the test does not run at all (`--ignored` is required), which
is why `./test.sh` is green on a machine with nothing installed.

## Parity harness

**No longer a gate.** Parity with the C# is retired (see [`parity.md`](parity.md)), so a
difference here is information rather than a failure. It is kept because the 128 step
corpus is a real regression suite over the MCP surface, the REST plane and every error
path. Point it at a previous build of this server to use it that way.

Two modes, so two servers never need to be up at the same time:

```bash
# capture the reference (C#) into a golden file
cargo run -p parity-harness -- capture \
  --base http://127.0.0.1:5002 --token "$CS_TOKEN" \
  --owner admin@example.com --out parity-golden.json

# compare this implementation against that file
cargo run -p parity-harness -- compare \
  --base http://127.0.0.1:5003 --token "$RUST_TOKEN" \
  --owner admin@example.com --golden parity-golden.json
```

| Flag | Meaning |
|---|---|
| `--base` | server URL (capture defaults to `:5002`, compare to `:5003`) |
| `--token` | bearer for that server. The identity must be a platform admin (the harness provisions with `admin.create_project`) and in practice the same person as `--owner`, because the `fs.*` steps run as the token identity |
| `--owner` | owner of the corpus project; also added as a member during provisioning |
| `--project` | reuse a fixed project id; the default is a fresh id per run, so a replay never inherits state |
| `--out` / `--golden` | golden file to write / to compare against (`parity-golden.json`) |
| `--relax-messages` | blank out free form message fields as well as error sentences |

`parity-golden.json` is the committed baseline. Recapture it after touching the
corpus and commit it with the change. Volatile values (timestamps, version, host
paths) are normalized, and an error text is reduced to `tool + ERR_* code`, so a
reworded message passes while a wrong code fails.

**Interpreting the output is documented once, in
[`.agent_docs/parity.md`](parity.md)**: what is inside the contract, what is
deliberately not compared, and the table of the differences that are expected to
show up. A non zero difference count is not automatically a failure; it is a
failure unless it is in that table.

## The `agent` crate

96 unit tests, all pure: none needs a server, an LLM or a terminal. The streaming
accumulator is fed chunk JSON directly, which covers what actually breaks in the wild (a
tool name split across chunks, out of order indices, a missing call id, a trailing nameless
delta, malformed arguments).

The interactive path cannot be unit tested, so `scripts/pty_check.py` drives the real binary
through a pty and reads back a virtual screen. Self contained: ephemeral port, temp state,
its own token, seeded readline history, no LLM key needed.

```bash
cargo build -p agent -p mcp-fs && python3 scripts/pty_check.py
```

Eleven editor checks. Every one was validated by reintroducing the bug it guards, including
the case where the repainted text is correct but sits one row too high. The spinner bugs and
the backspace gap described in `.agent_docs/agent.md` were all found this way, not by the
test suite.

Smoke test both stdin modes, since they take different code paths:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you   # piped
./agent.sh --user you                                                     # interactive
```
