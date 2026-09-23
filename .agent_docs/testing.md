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

The tool contract is also a test: `tools/mod.rs` and `tools/all.rs` compare all 57
descriptions and `inputSchema` values against `tool-contract-golden.json`,
serialized, so even a property key order change fails the build.
`tools/contract_golden.rs` owns the file and adds the both directions check, so a
tool added to the registry but never written to the contract fails too. All three
skip with a message when the file is absent, since it lives at the repo root
outside the crate.

The contract is regenerated deliberately, never hand edited:

```bash
MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current
```

Review the diff afterwards: a description edit is one line, and 57 changed tools
means something went wrong.

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
| `mcp-fs` lib | 1033 passed, 2 ignored |
| `agent` bin | 111 passed |
| `mcp-fs` bin | 0 (the binary is a thin `main`) |
| doctests | 0 passed, 3 ignored (wiring examples marked `ignore`) |

That is the default feature set, so SQLite only. The relational conformance cases add
themselves per engine when their dsn is present.

Per area, from `cargo test -p mcp-fs --lib -- --list` (1035 entries, the two ignored and
the live database cases included). The per area rows below are indicative: they are not
recounted on every change, so trust the total above:

| Area | Tests | Area | Tests |
|---|---|---|---|
| `tools` | 267 | `config` | 30 |
| `core` | 140 | `mcp` | 23 |
| `storage` | 139 | `cli` | 23 |
| `docs` | 121 | `util` | 21 |
| `git` | 117 | `identity` | 18 |
| `api` | 83 | `safety` | 18 |
| | | `app` | 11 |
| | | `keys` | 9 |
| | | `migrate` | 5 |
| | | `logging` | 5 |
| | | `errors` | 5 |

Largest single modules: `core::fs_ops` 125, `api::dataplane` 64, `docs::extract` 49,
`git::oauth` 41, `tools::git` 39, `storage::meta` 35, `config` 30.

`storage` grew most, from 62, because the relational layer carries its own suite:
`storage::rel::dialect` 17, `storage::rel::sqlite` 14 and `storage::rel` 12 cover
placeholder rendering, upsert and DDL per dialect, `LIKE` escaping and the retry helper
with no database at all.

## Capturing spans and events in a test (`logging::capture`, US-018, 2026-09-21)

`tools::git::tests::e2e_new_151` and `e2e_new_247` need to inspect exactly what a
`git.remote` tracing span or an event carried. `logging::capture::CaptureLayer` is a
`#[cfg(test)]`-only `tracing_subscriber::Layer` installed process wide by
`logging::init`, backed by two process global `Vec`s (`SPANS`, `EVENTS`): the tracing
dispatcher is process wide, `cargo test` runs the whole crate in one binary, so there is
only ever one subscriber to record into regardless of which test installs it first.

A process global buffer with no gating is a trap: `cargo test` runs hundreds of other
tests in parallel, and several hundred of them also exercise clone/push/fetch/pull
(without ever calling the capture API), so their `git.remote` spans landed in the same
buffer and inflated `e2e_new_247`'s count on a full `cargo test --workspace` run while
passing every time in isolation. `CAPTURE_LOCK` only ever serialized capturing tests
against EACH OTHER; it never stopped an unrelated, non-capturing test running
concurrently on another OS thread from polluting the shared `Vec` during the exact
window between `clear()` and the read.

The fix is a `thread_local!` `CAPTURING` flag, not a process wide atomic bool: a process
wide flag would still be `true` on every OTHER thread for as long as the capturing
test's guard is held, so an unrelated concurrent test's spans would still pass the gate.
What actually isolates the two is that every capturing test runs its whole async body on
a dedicated, freshly built **single threaded** runtime (`with_git_hosts_lock` in
`tools/git.rs`), so every span and event it causes, including ones from a
`spawn_blocking` closure awaited from that same task, opens and closes its lifecycle on
that one OS thread. `lock_for_test()` sets the flag on ITS calling thread only; every
other test's spans fire on their own OS thread, where the flag was never set, and
`CaptureLayer` drops them instead of recording them. Read `crates/mcp-fs/src/logging.rs`
before adding a third capturing test: the isolation only holds if that test also runs on
a dedicated single threaded runtime, the same way the first two do.

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
docker compose -f docker-compose.test.yml up -d      # pgvector/pgvector:pg16 on 55432, mssql 2022 on 51433

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

## Opt in tests

Two tests are `#[ignore]`, because each needs something the machine may not have.
Both skip with a message rather than fail when it is missing, since "not
installed" is not a regression.

### The real document converter

`docs::service::tests::real_doc_convert_produces_markdown_and_leaves_nothing_behind`
shells out to a real `doc-convert` on `PATH` with a one page PDF built in the test
(the repo carries no binary fixture). It asserts the Markdown comes back non
empty, that the sandbox directory is gone afterwards, and that no `*_docling`
leftovers were created next to the server's working directory. It takes tens of
seconds, which is why it is not in the default run:

```bash
cargo test -p mcp-fs --lib docs::service -- --ignored
```

The other `docs::service` tests need nothing: the cli ones build a three line
`/bin/sh` script as the converter, and the api ones drive an in process axum stub.

### S3

`storage::blob::s3::tests::integration_put_get_range_delete`
needs a live S3 compatible service, because faking S3 would test the fake and
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

## Functional scenarios

`tests/functional/run_all.sh [--user NAME] [FILTER]` sources every
`tests/functional/scenarios/[0-9][0-9]_*.sh`, starting and stopping the server
itself when nothing is listening. Most scenarios drive the CLI agent with piped
prompts, so they need an LLM key; they assert loosely on the agent's prose.

The two document service scenarios are the exception: they verify an exact HTTP
chain, so they speak curl rather than English, and each starts **its own server**
on its own port with its own throwaway state, because the runner's server has
`doc_service` disabled. Their shared plumbing is
`scenarios/_doc_service_common.sh` (the leading underscore keeps it out of the
runner's glob; the scenarios source it explicitly).

| Scenario | Needs | Covers |
|---|---|---|
| `25_doc_service_cli.sh` | `doc-convert` on `PATH` | `doc_service.mode: cli`: upload with the flag, both files listed, the companion non empty, `fs.documentize` no clobber then overwrite, an ineligible extension refused with nothing written |
| `26_doc_service_api.sh` | `python3` and `doc-convert` | `doc_service.mode: api` against `scripts/doc_service_fake.py`, same chain over HTTP, plus a wrong token failing the conversion without losing the uploaded file |

```bash
./tests/functional/run_all.sh doc_service        # the two of them, roughly two minutes
```

Each skips with a clear message when its prerequisite is absent.

`scripts/doc_service_fake.py` is stdlib only and implements exactly the contract
`ApiDocService` speaks: `POST` multipart with a part named by `--file-field`
(`file` by default, mirroring `doc_service.api.file_field`), an optional
auth header checked by name and exact value, `200 text/markdown` with the
Markdown as the body. It delegates to `doc-convert --stdout --quiet` in a
throwaway directory. It is also the quickest way to develop against api mode by
hand:

```bash
python3 scripts/doc_service_fake.py --port 8099 --auth-header X-Convert-Key --auth-token s3cret
```

## The differential harness is gone

`crates/parity-harness` replayed a corpus against a live server and compared the result
to a C# capture. It has been **deleted**: the C# is no longer a reference (see
[`lineage.md`](lineage.md)), so a difference against it was a false signal rather than a
regression. Its 32 tests covered the corpus and the normalizer, both of which existed only
to serve that comparison.

What replaced it, and why nothing was lost that mattered:

| Was covered by the harness | Now covered by |
|---|---|
| the 94 tool schemas and descriptions | `tool-contract-golden.json` plus the three contract tests |
| the MCP wire framing and JSON-RPC behaviour | `app.rs` router tests driven with `oneshot` |
| the REST plane, every route | `api/dataplane.rs` and `api/openapi.rs` tests |
| every error path and `ERR_*` code | per module tests next to each error |
| the git smart protocol | `git/http/mod.rs` tests, plus a real clone/push/reclone |
| behaviour across engines | `storage/conformance.rs`, one suite per engine |

To compare this server against a **previous build of itself**, which is what the harness
was useful for after parity was retired, drive the real surface instead: the tool level
tests and the conformance suite already cover it, and `mcp-fs migrate` round trips state
between engines so a copy can be diffed row for row.

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
