# Full-stack e2e test fails at the MCP handshake: Bug Specification

> Generated on: 2026-10-19
> Id: SPEC-0043
> Nature: BUG
> Depth: S
> Depth evidence: 1 module (one test file), 3 requirements, no public contract change, no data model change
> Status: designed
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: Low
> From backlog: BL-0068
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Defect

### BUG-001: `make test-e2e-full` never gets past the MCP `initialize` call
- **Observed behavior:** `make test-e2e-full` panics with `MCP initialize: http 406 Not Acceptable` (`crates/core/tests/full_stack_e2e.rs:175`) and make ends with `Error 101`. Reproduced twice on 2026-10-19, with and without the US-0050 toolchain change, against the compose PostgreSQL (`pgvector/pgvector:pg16`, port 55432) and a moto S3 stand in on port 9000.
- **Expected per authority:** `.agent_docs/testing.md:224-249` describes `full_stack_lifecycle` as the test that drives the whole product (MCP calls, REST plane, git CLI) against real PostgreSQL and S3 and passes; `Makefile:19-23` and `AGENTS.md:44` publish `make test-e2e-full` as the full lifecycle gate with mandatory infra.
- **Reproduction:** infra up (PostgreSQL, S3 compatible store), `MCPFS_TEST_PG_DSN`, `MCPFS_MINIO_SECRET_KEY`, `MCPFS_MINIO_ENDPOINT`, `MCPFS_MINIO_ACCESS_KEY` exported, then `make test-e2e-full`. Expected: test passes. Actual: the 406 panic above.
- **Root cause:** the test's MCP helpers POST to `/mcp` with `.bearer_auth(token).json(..)` and no `Accept` header (`crates/core/tests/full_stack_e2e.rs:169-172`, `:185-189`, `:222-227`; `rg -n -i accept crates/core/tests/full_stack_e2e.rs` finds 0 hits). The rmcp 3.5.0 service answers 406 to any POST whose `Accept` lacks both `application/json` and `text/event-stream` (`rmcp-3.5.0/src/transport/streamable_http_server/tower.rs:1991-2001`). The helpers were written for the hand rolled MCP layer that SPEC-0013 replaced (commit `7f043e5`, 2026-10-04); no version of the file ever carried an `Accept` header (`git show 7f043e5:crates/core/tests/full_stack_e2e.rs | rg -i accept` is empty).
- **Contributing factors:** the test is `#[ignore]`d (`full_stack_e2e.rs:316`) and needs live PostgreSQL and S3, so `make check` never runs it. The sibling clients already send the header (`crates/agent/src/mcp.rs:116,161,183`; `crates/core/src/app.rs:447,807`).
- **Impact:** the only test covering PostgreSQL, S3 and git together has been dead since 2026-10-04. Developer facing; no end user impact. Workaround: none.
- **Rollback posture:** a test only change; reverting restores the current failing state and loses no data.

## 2. Expected Behavior

#### BR-001 [EARS-E]: the lifecycle test passes
> WHEN a developer runs `make test-e2e-full` with PostgreSQL and an S3 compatible store reachable and the four environment variables of `.agent_docs/testing.md` set, THE full-stack e2e test SHALL complete the MCP handshake and run its whole scenario to a pass, printing `test result: ok. 1 passed`.

- **Source:** restored from `.agent_docs/testing.md:224-249` and `Makefile:19-23`.
- **Inputs:** the command and variables above.
- **Outputs:** exit code 0; `test full_stack_lifecycle ... ok`.
- **Business Rules:** none beyond the documented scenario.
- **Exact names:** command `make test-e2e-full`; test `full_stack_lifecycle`.

#### BR-002 [EARS-U]: every MCP request carries the Accept header
> The full-stack e2e test SHALL send the header `Accept: application/json, text/event-stream` on every request it makes to the `/mcp` endpoint, the `initialize`, `notifications/initialized` and `tools/call` requests included.

- **Source:** restored from the server contract the sibling clients already follow (`crates/agent/src/mcp.rs:116`).
- **Inputs:** each of the three request kinds.
- **Outputs:** none of them is answered with HTTP 406.
- **Exact names:** header `Accept`, value `application/json, text/event-stream` (exact, in that order, as in `crates/agent/src/mcp.rs:116`).

#### BR-003 [EARS-UB]: the server contract does not move
> The fix SHALL NOT change the server's treatment of a request to `/mcp` that lacks the `Accept` header, and SHALL NOT remove the `#[ignore]` attribute of `full_stack_lifecycle`.

- **Source:** decided here: ASSUMED, the defect is in the client of the test, the server follows the rmcp contract of SPEC-0013; the test stays opt in so `cargo test --workspace` stays independent of live infra (`full_stack_e2e.rs:316`).

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| REST calls of the same test (`/api/fs/...`) | unchanged; they may or may not carry the header, no requirement | BR-002 scope is `/mcp` only |
| S3 store is moto instead of MinIO | documented stand in (`.agent_docs/testing.md:206-214`); BR-001 holds with either | BR-001 |
| infra variables missing | the test still fails fast with its own `expect` messages (`full_stack_e2e.rs:76-`), not a 406 | BR-001 |
| plain `cargo test --workspace` | test is skipped (ignored), result unchanged | BR-003 |

## 4. Tests

### 4.1 End to end reproduction (mandatory)
#### BT-001: the lifecycle command reaches past the handshake
- **Reproduces:** BUG-001
- **Driver:** CLI (`make test-e2e-full`)
- **Fails today because:** the `initialize` POST carries no `Accept` header, so the server answers 406 (`full_stack_e2e.rs:169-175`).
- Given PostgreSQL (`postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs`) and an S3 compatible server on `http://127.0.0.1:9000` with access key `admin`, and `MCPFS_MINIO_SECRET_KEY=any`
- When the developer runs `make test-e2e-full`
- Then the output contains neither `MCP initialize: http 406 Not Acceptable` nor `panicked`, and the process exits 0.

### 4.2 New behavior
#### BT-010: the full lifecycle passes
- **Validates:** BR-001, BR-002
- Given the same infra as BT-001
- When `make test-e2e-full` runs
- Then the output contains `test full_stack_lifecycle ... ok` and `test result: ok. 1 passed; 0 failed`.
- Observed on 2026-10-19: with the header added to every `bearer_auth` call the run printed `test result: ok. 1 passed; 0 failed` in 4.27 s (moto stand in).

### 4.3 Non regression
#### BT-020: the default suite is unchanged
- **Protects:** BR-003
- Given no live infra
- When `make test` runs (`./test.sh`, `cargo test --workspace`)
- Then it reports 0 failed and `full_stack_lifecycle` is listed as `ignored`.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/tests/full_stack_e2e.rs` | `full_stack_lifecycle` | Kept as is (same name, same scenario) | only its request headers change |

## 5. Existing Specs Affected
SPEC-0013 (rmcp migration) introduced the dead state by changing the handshake the helper targets; it is not modified. SPEC-0042 found the defect and is not modified. None else.

## 6. Assumptions
- **Assumption:** BR-003 (leave the server and the `#[ignore]` alone) shaped by the evidence that the server behavior is the rmcp contract and all other clients already comply.
- **Assumption:** verification used a moto S3 stand in, not a real MinIO (the MinIO image could not be pulled, quay.io returned 401). BR-001 and BT-010 are stated for either; a real MinIO run is part of the story's definition of done when an image is reachable.
- **Backlog entries created by this run:** none. Consumed: BL-0068.

## 7. Implementability Checklist

| Item | Result |
|---|---|
| 1 No technical decision | PASS (root cause `file:LINE` only; no fix design) |
| 2 No buried requirement | PASS |
| 3 Observable names spelled | PASS (command, test name, header name and value) |
| 4 No forced choice | PASS |
| 5 Every test specified | PASS (BT-001, BT-010, BT-020, 4.4 lists the one touched test) |
| 6 Reproduction is end to end | PASS (CLI driver, asserts the visible exit code and output) |
| 7ante Security misclassification | PASS (test only defect, no security content) |
| 7bis Class sweep | n/a |
| 8 No hidden prerequisite | PASS (infra stated in the Given) |
| 9 EARS clean | PASS (E, U, UB; no forbidden modal) |
| 10 Right tool | PASS (behavior of a published command changes; depth S, 3 requirements) |
| 11 Length | PASS |
| 12 Claims cited | PASS |
| 13 Round trip verified | PASS (writer: test helpers `:169,:185,:222`; reader: rmcp `tower.rs:1991-2001`) |
| 14 Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
