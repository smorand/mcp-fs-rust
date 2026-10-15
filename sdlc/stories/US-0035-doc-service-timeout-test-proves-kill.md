# US-0035: Doc service timeout test proves the kill

> Spec: SPEC-0032 (sdlc/specs/SPEC-0032-doc-service-timeout-test-is-real/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
`cli_timeout_kills_the_child_and_removes_the_tempdir` flakes when a loaded machine starts the child after its 3 s timeout, and its kill check can never fail (it looks for a line written 30 s after start, 6 s after start). Rewrite its body so the kill is observable and a slow start does not panic; no production code changes.

## Requirements covered
- SPEC-0032/DR-001: The test `cli_timeout_kills_the_child_and_removes_the_tempdir` SHALL run a script that writes its working directory on the first line of the marker, then appends the line `tick` every 0.2 s without end, and SHALL assert, after the call returns, that the number of `tick` lines does not grow over a 2 s wait.
- SPEC-0032/DR-002: The test SHALL use a 10 s timeout, SHALL assert the error code `INTERNAL_ERROR` and that the message contains `timed out after 10s`, and SHALL read the marker only after the call returned.
- SPEC-0032/DR-003: The test SHALL keep asserting that the directory named on the marker's first line no longer exists after the call returns.
- SPEC-0032/DR-004: The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib docs::service::tests::cli_timeout_kills_the_child_and_removes_the_tempdir` prints `test result: ok. 1 passed`; with `let _ = child.kill().await;` (`crates/core/src/docs/service.rs:290`) temporarily deleted it fails on the tick count assertion.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0032): tick loop with an unchanged count check (the killed `sh` itself writes, so a live process keeps adding lines). Rejected: waiting 30 s, a pid file (orphaned `sleep` grandchild), skipping the check when the marker is absent.
- DEC-002 (SPEC-0032): 10 s timeout (start delay over 3 s observed in a 286 s loaded run).
### Contracts (production, unchanged)
- Timeout wraps the run at `service.rs:268`; on expiry `let _ = child.kill().await;` (`:290`) then `doc service timed out after {}s` (`:291-293`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/service.rs` | rewrite the body of the test at `:829-861` |
### Existing patterns
- `script(dir, name, body)` (`service.rs:755`) writes an executable `#!/bin/sh` script; `cli_service(argv, timeout_secs)` (`:765`); `crate::config::DOC_PLACEHOLDER` for the document argument; the current test body (`:829-861`) for the call and error assertions.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0032/DT-010: The kill is proven
- Level / Driver / Location: integration / real `/bin/sh` child / `crates/core/src/docs/service.rs::tests::cli_timeout_kills_the_child_and_removes_the_tempdir`
- Script body: `pwd > "$1"` then `while :; do echo tick >> "$1"; sleep 0.2; done`.
- Then: `e.code == code::INTERNAL_ERROR`, `e.message.contains("timed out after 10s")`; the marker's first line names a directory that does not exist; tick count read, `tokio::time::sleep(Duration::from_secs(2))`, tick count read again, `assert_eq!(before, after)`.
- RED by mutation: (1) before the rewrite, delete `service.rs:290` and run the OLD test: record it stays green (hollow). (2) After the rewrite, same deletion: the new test fails on `assert_eq!(before, after)`. Restore the line both times.

### SPEC-0032/DT-011: Slow start does not panic
- The test body passes `10` as the timeout, asserts `timed out after 10s`, and reads the marker only after `to_markdown(...).await` returned.

## Constraints
- Only the test body in `crates/core/src/docs/service.rs` changes; production code untouched (the mutation is temporary and restored).
- No new dependency.
- If `sleep 0.2` is rejected by `/bin/sh`, report BLOCKED rather than lengthening the interval past 1 s silently.

## Definition of done
- [ ] RED evidence: both mutation runs recorded in `.implement/US-0035-red.log`, `service.rs:290` restored (`git diff` shows no production change).
- [ ] DT-010, DT-011 green; `make check` green with `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`, no `skipping` DB suite.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 1 (`cli_timeout_kills_the_child_and_removes_the_tempdir`, the test is the debt).
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: DT-010, DT-011 and 8.1 green; DR-001..DR-004 cited here.
- [ ] One commit `US-0035: Doc service timeout test proves the kill`, body `Spec: SPEC-0032` and the DR ids.
