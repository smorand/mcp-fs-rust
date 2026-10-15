# Doc service timeout test proves the kill and survives a loaded machine: technical debt design

> Id: SPEC-0032
> Nature: DEBT
> Depth: S
> Depth evidence: one test in `crates/core/src/docs/service.rs` (test module), no public contract, no persisted data
> Status: specified
> Generated on: 2026-10-15
> Behavior change: none
> From backlog: BL-0055
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt

### DBT-001: The timeout test flakes and its kill check is hollow
- **Current structure:** `docs::service::tests::cli_timeout_kills_the_child_and_removes_the_tempdir` (`crates/core/src/docs/service.rs:829`) runs a script `pwd > "$1"; sleep 30; echo survived >> "$1"` (`:832-838`) with a 3 s timeout (`:847`). After the error it reads the marker with `std::fs::read_to_string(&marker).unwrap()` (`:854`), asserts the sandbox is gone (`:855`), sleeps 3 s (`:858`) and asserts the marker lacks `survived` (`:859-860`). The service under test wraps the run in `tokio::time::timeout(timeout_secs)` (`:268`) and on expiry calls `let _ = child.kill().await;` (`:290`) before the sandbox drops.
- **Cost paid today:**
  1. Under the full parallel suite the child did not start within 3 s, the marker never existed, and `:854` panicked with `NotFound` (US-0034 landing, BL-0055): `make check` red with no code change.
  2. The kill assertion cannot fail: `survived` is written 30 s after start, the test looks 6 s after start. Deleting `child.kill()` leaves the test green, so the property it is named after is unverified.
- **Target structure:** the script writes `pwd` then appends a tick every 0.2 s forever; the timeout is 10 s; after the error the test reads the tick count, waits 2 s, and asserts the count did not grow.
- **Observable behavior:** unchanged (test code only).

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/docs/service.rs` (test module) | rewrite the body of `cli_timeout_kills_the_child_and_removes_the_tempdir` (`:829-861`) | 1 test |

**Non source occurrences:** none.
**Contracts touched:** none; the error text `doc service timed out after {}s` (`:291-293`) is production and unchanged.
**Other timing tests in the module:** the other `cli_*` tests in `service.rs` do not set a short timeout that the child must beat; out of scope.
**Guardrail coverage:** the module's tests plus the full suite. **Test command:** see 8.1.
**Total:** 1 file, 1 test, recounted.

## 3. Class sweep
Not a security lot.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-U]: The kill is observable
> The test `cli_timeout_kills_the_child_and_removes_the_tempdir` SHALL run a script that writes its working directory on the first line of the marker, then appends the line `tick` every 0.2 s without end, and SHALL assert, after the call returns, that the number of `tick` lines does not grow over a 2 s wait.
- **Old form:** script `pwd > "$1"` / `sleep 30` / `echo survived >> "$1"`; check `!after.contains("survived")` after a 3 s sleep.
- **New form:** script `pwd > "$1"` / `while :; do echo tick >> "$1"; sleep 0.2; done`; check tick count before and after a 2 s sleep is equal.

#### DR-002 [EARS-U]: Robust to a slow start
> The test SHALL use a 10 s timeout, SHALL assert the error code `INTERNAL_ERROR` and that the message contains `timed out after 10s`, and SHALL read the marker only after the call returned.

#### DR-003 [EARS-U]: Sandbox removal still checked
> The test SHALL keep asserting that the directory named on the marker's first line no longer exists after the call returns.

### 4.2 Invariance
#### DR-004 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

## 6. Decisions
- **DEC-001:** A tick loop with an unchanged count check. **Alternatives considered:** keep `sleep 30` and wait 30 s after the timeout, rejected (30 s test); a pid file and `kill -0`, rejected because `sh` would leave an orphaned `sleep` grandchild and the pid check tests the wrong process; skip the kill check when the marker is absent, rejected because it would silently skip the property. **Rationale:** a live shell keeps writing, a killed one cannot; the loop runs in the killed process itself. **Implemented by:** DR-001. **Evidence:** `crates/core/src/docs/service.rs:290`.
- **DEC-002:** 10 s timeout. **Alternatives considered:** 3 s (current, flakes under load), 30 s (slow). **Rationale:** the failure was a start delay of over 3 s in a 286 s loaded run; 10 s leaves margin while keeping the test near 13 s. **Implemented by:** DR-002. **Evidence:** BL-0055 log, `test result: FAILED. 2619 passed; 1 failed ... finished in 286.41s`.
- **Assumption:** `sleep 0.2` is accepted by `/bin/sh` on the supported platforms (macOS and Linux both accept fractional seconds); if not, the RED step shows it at once.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the row in 8.4:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None.

### 8.3 Structural tests
#### DT-010: The kill is proven
- **Validates:** DR-001, DR-003.
- **Method:** mutation. Temporarily delete `let _ = child.kill().await;` (`service.rs:290`): the reworked test goes red on the tick count assertion; the old test, run under the same mutation before the rework, stays green (recorded, proves it was hollow). Restore the line.
- **Then** with the line present: `cargo test -p mcp-fs-core --lib docs::service::tests::cli_timeout_kills_the_child_and_removes_the_tempdir` prints `test result: ok. 1 passed`.

#### DT-011: Slow start does not panic
- **Validates:** DR-002.
- **Asserts:** the test body contains `10,` as the timeout, the assertion `timed out after 10s`, and reads the marker only after `to_markdown(...).await`; no `unwrap()` on the marker read happens before the call returns.
- **Method:** diff inspection.

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/docs/service.rs` | `cli_timeout_kills_the_child_and_removes_the_tempdir` | body rewritten per DR-001..DR-003 | the test is the debt (BL-0055): it flakes and cannot detect a missing kill |

## 9. Implementation order
1. Record the old test under the kill mutation (green, hollow).
2. Rewrite the test; record it red under the same mutation; restore; green.
3. 8.1.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 1 | IMPLEMENTABLE |

Inline checklist (depth S): 1 PASS, 2 PASS (both forms spelled), 3 n/a, 4 PASS, 5 n/a, 6 PASS, 7 PASS, 8 PASS (test only), 9 PASS (lines opened), 10 PASS, 11 PASS, 12 PASS, 13 n/a, 14 PASS (one declared test).

**Amendments applied:** A (citation): the kill line is `service.rs:290`, not `:289` as stated in the approval message.
