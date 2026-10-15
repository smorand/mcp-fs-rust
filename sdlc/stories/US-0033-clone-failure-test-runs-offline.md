# US-0033: Clone failure test runs offline

> Spec: SPEC-0030 (sdlc/specs/SPEC-0030-offline-clone-failure-test/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
`remote_clone_surfaces_a_failure_without_leaking_the_token` clones from real `github.com`, so a slow network turns `make check` red with `remote timeout` instead of `clone failed:`. Point it at a local peer that drops the connection, keeping every assertion.

## Requirements covered
- SPEC-0030/DR-001: The test `remote_clone_surfaces_a_failure_without_leaking_the_token` SHALL NOT open a connection to any host other than `127.0.0.1`.
- SPEC-0030/DR-002: The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.
- SPEC-0030/DR-003: The test SHALL keep its four assertions unchanged: `err.code == code::INTERNAL_ERROR`, `err.message.starts_with("clone failed:")`, the message does not contain `gho_supersecret`, and the stored token still reads `gho_supersecret`.

## Independent Test
`cargo test -p mcp-fs-core --lib tools::git::tests::remote_clone_surfaces_a_failure_without_leaking_the_token` prints `test result: ok. 1 passed` with `finished in` under 5 s, and `rg -n "github.com/does-not/exist-mcpfs" crates` prints nothing.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0030): a loopback listener that accepts and drops the connection (`closing_remote()`, sibling of `silent_remote()`, stream dropped right after `accept`). Rejected: an unroutable address and `silent_remote()` (both hang to `remote timeout`), a stub git server (speculative).
- DEC-002 (SPEC-0030): keep provider `github` for host `127.0.0.1`; the token is stored for host `127.0.0.1`.
### Contracts
`async fn closing_remote() -> (String, tokio::task::JoinHandle<()>)`, private to `tools::git::tests`, returning `format!("https://127.0.0.1:{port}/does-not/exist-mcpfs.git")` and the accept loop handle (caller `abort()`s it).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | add `closing_remote()` next to `silent_remote()` (`:9336-9346`); in the test (`:8462`) replace `"github.com"` at `:8467`, `:8471`, `:8492` with the URL's host and the URL at `:8483` with `closing_remote()`'s URL; update the comment at `:8463` |
### Existing patterns
- `silent_remote()` at `crates/core/src/tools/git.rs:9336-9346` (bind `127.0.0.1:0`, spawn accept loop, return URL and handle).
- `e2e_new_defect_token_set_then_remote_clone_share_the_real_host` at `crates/core/src/tools/git.rs:8795-8815` (derives the host from the URL, declares it, aborts the server after the call).

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0030/DT-011: The test is fast and offline
- Level / Driver / Location: unit / `cargo test` / `crates/core/src/tools/git.rs::tests::remote_clone_surfaces_a_failure_without_leaking_the_token`
- RED first: before switching the URL, record the current run's `finished in` and the `github.com` target (a structural RED: DT-010's `rg` prints `crates/core/src/tools/git.rs:8483`).
- Then `test result: ok. 1 passed`, `finished in` under 5 s; the function body names no host other than `127.0.0.1`.

### SPEC-0030/DT-010: The external URL is gone
- `rg -n "github.com/does-not/exist-mcpfs" crates` prints nothing (before: 1 hit, `git.rs:8483`).

### SPEC-0030/DT-012: Assertions unchanged
- In `git diff main -- crates/core/src/tools/git.rs`, none of the four DR-003 assertion lines carries a `-` or `+`, except the token lookup host argument at `:8492` (`"github.com"` to the loopback host), which is setup, not the assertion value `gho_supersecret`.

## Constraints
- Only `crates/core/src/tools/git.rs` changes outside `sdlc/`; no production code.
- No new dependency.
- If the dropped connection does not yield `clone failed:`, stop and report BLOCKED rather than loosening the assertion.

## Definition of done
- [ ] RED evidence: DT-010 `rg` hit recorded before the change.
- [ ] DT-010, DT-011, DT-012 green; `make check` green with `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`, no `skipping` DB suite.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 1 (`remote_clone_surfaces_a_failure_without_leaking_the_token`, setup only, the test is the debt).
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: DT-010..DT-012 and 8.1 green; DR-001..DR-003 cited here.
- [ ] One commit `US-0033: Clone failure test runs offline`, body `Spec: SPEC-0030` and `DR-001, DR-002, DR-003`.
