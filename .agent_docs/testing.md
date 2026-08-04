# Testing

## Layout

There is no `tests/` integration crate (the directory exists and is empty). Every
test is inline, in a `#[cfg(test)] mod tests` next to the code it covers, which
keeps a test in the same file as the invariant it protects.

Three levels, all inside the crate:

| Level | How | Where |
|---|---|---|
| Unit | plain functions and in memory SQLite (`SqliteDb::open_in_memory`, `SqliteAdminStore::in_memory`) plus `tempfile` dirs for blobs | every module |
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
cargo clippy --all-targets -- -D warnings    # second half of the quality gate
```

Both must be clean before any commit.

## Current counts

From `cargo test --workspace` on the current tree:

| Target | Result |
|---|---|
| `mcp-fs` lib | 754 passed, 1 ignored |
| `parity-harness` bin | 32 passed |
| `mcp-fs` bin | 0 (the binary is a thin `main`) |
| doctests | 0 passed, 3 ignored (wiring examples marked `ignore`) |

Per area, from `cargo test -p mcp-fs --lib -- --list` (755 tests, the ignored one
included):

| Area | Tests | Area | Tests |
|---|---|---|---|
| `tools` | 176 | `mcp` | 23 |
| `git` | 117 | `util` | 20 |
| `core` | 113 | `cli` | 15 |
| `docs` | 106 | `safety` | 11 |
| `api` | 65 | `app` | 11 |
| `storage` | 62 | `identity` | 10 |
| | | `keys` | 9 |
| | | `config` | 9 |
| | | `logging` | 5 |
| | | `errors` | 3 |

Largest single modules: `core::fs_ops` 101, `docs::extract` 49,
`api::dataplane` 48, `git::oauth` 41, `git::http` 40, `tools::git` 36.

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

The objective judge of 1:1 parity. Two modes, so the two servers never need to be
up at the same time:

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

The interactive path cannot be unit tested. It is verified by driving a real pty: wait for
the prompt, type with a typo, fix it with backspaces, submit, recall with the up arrow,
abandon with Ctrl+C, then ask for markdown and assert on the rendered escapes. Both spinner
bugs described in `.agent_docs/agent.md` were found that way, not by the test suite.

Smoke test both stdin modes, since they take different code paths:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you   # piped
./agent.sh --user you                                                     # interactive
```
