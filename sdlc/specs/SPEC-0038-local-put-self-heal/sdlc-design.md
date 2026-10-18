# A local blob put that loses its blob mid call rewrites it: technical debt design

> Id: SPEC-0038
> Nature: DEBT
> Depth: S
> Depth evidence: one function (`LocalBlobStore::put`, `crates/core/src/storage/blob/local.rs:40-70`), one module, no public contract, no persisted change
> Status: implemented
> Generated on: 2026-10-18
> Behavior change: declared delta, see Section 7 (a race that returned an error now succeeds)
> From backlog: BL-0057
> Depends on: SPEC-0034 (implemented, mtime refresh on put)
> Security: n/a (internal race on the content-addressed store; no untrusted input)
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: put fails when its blob is deleted between the existence check and the refresh
- **Current structure:** `put` checks `try_exists` (`local.rs:47`), then opens the file to refresh its modified time (`local.rs:48-53`, SPEC-0034/DR-002). A delete between the two (orphan sweep `purge.rs` `sweep_project_orphans_at`, or refcount GC) makes `File::open` fail `NotFound`, which `put` returns as an error.
- **Cost paid today:** a write fails for a reason the store can repair itself; the bytes are in hand.
- **Target structure:** `put` opens the blob directly; `NotFound` falls through to the existing temp file then rename write (`local.rs:56-69`); any other error is returned unchanged.
- **Observable behavior:** unchanged except Section 7.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/storage/blob/local.rs` | `put` `:47-55`: drop `try_exists`, open with `NotFound` fall through; one `#[cfg(test)]` hook before the refresh; tests DT-001..DT-003 in the tests module (`:188`) | 1 production site, 3 new tests |

**Non source occurrences:** none.
**Contracts touched:** `BlobBackend::put` (internal, `storage/traits.rs:244`); S3 `put` is a single upload with no check then open (`storage/blob/s3.rs:48-58`), unchanged.
**Guardrail coverage:** `put_get_roundtrip` `:199`, `put_is_idempotent` `:237`, `put_of_existing_bytes_refreshes_the_modified_time` `:325`; none exercises the race. **Test command:** `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
**Total:** 1 file, 1 production site.

## 3. Class sweep
n/a.

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-O]: Self heal on a vanished blob
> IF opening an existing blob for the modified time refresh fails with `NotFound` THEN `LocalBlobStore::put` SHALL write the bytes through the temp file then rename path and return `Ok`.
- **Old form:** `if tokio::fs::try_exists(&final_path).await.unwrap_or(false) { open()?.set_modified(..); return Ok(()) }`
- **New form:** `match open { Ok(f) => { f.set_modified(..)?; return Ok(()) } Err(NotFound) => {} Err(e) => return Err(e) }` then the existing write
- **Casing convention:** Rust snake_case

#### DR-002 [EARS-UB]: Other errors are not swallowed
> `LocalBlobStore::put` SHALL NOT rewrite the blob when opening it fails with an error other than `NotFound`; it SHALL return that error.

#### DR-003 [EARS-U]: Test seam
> `local.rs` SHALL call a `#[cfg(test)]` only hook after deciding the blob exists and before refreshing it, absent from non test builds.

### 4.2 Invariance
#### DR-004 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change, and `tool-contract-golden.json` SHALL be unchanged.

### 4.3 Compatibility
None.

## 5. Compatibility plan
None (internal contract).

## 6. Decisions
- **DEC-001:** Open directly and fall through on `NotFound`. **Alternatives considered:** retry the whole `put` (rejected: the fall through already performs exactly one write); lock against the sweep (rejected: cross component locking for a race the fall through makes harmless). **Implemented by:** DR-001, DR-002. **Evidence:** `local.rs:47-69`.
- **DEC-002:** A `#[cfg(test)]` hook as the seam. **Alternatives considered:** a pausing filesystem wrapper behind a trait (rejected: far more code for the same signal); no deterministic test (rejected: the fix would be unproven). **Implemented by:** DR-003. **Evidence:** BL-0057 notes.
- **Assumption:** a delete landing after the open and before the refresh leaves `put` returning `Ok` with no file, as before US-0037; this window is the generic "deleted after put" case, which callers cover through references and the sweep's recheck (SPEC-0034). Not in scope.

## 7. Declared breaks
Not pure debt, one delta: a `put` whose blob is deleted between the existence decision and the refresh now succeeds and rewrites the blob instead of returning a not found error. Affected: no caller relies on that error (it surfaced only as a failed write). Authorized: owner, 2026-10-18 (BL-0057 spec approval).

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None.

### 8.3 Structural and behavior tests
#### DT-001: vanished blob is rewritten
- **Validates:** DR-001, DR-003
- **Location:** `local.rs` tests
- **Given** `put("racy", b"data")` done, and the hook set to delete `{root}/ra/racy` / **When** `put("racy", b"data")` again / **Then** it returns `Ok`; `get("racy", 0, None)` returns `data`; the file exists on disk (second channel: `std::fs::metadata`).
- **Red before the fix because:** `put` returns a `NotFound` error from `File::open`.

#### DT-002: refresh path leaves no temp file
- **Validates:** DR-004 (refresh unchanged)
- **Given** a blob `kept` backdated 48 h / **When** `put("kept", b"data")` / **Then** its modified time is under 2 s old, bytes unchanged, and the shard folder holds no `.tmp-` entry.
- **Red before:** guardrail; green before and after.

#### DT-003: permission denied is not swallowed
- **Validates:** DR-002
- **Given** (Unix) blob `locked` stored, file mode set to `000` / **When** `put("locked", b"data")` / **Then** it returns an error, and the file still has mode `000` (no rewrite happened); mode restored in cleanup. Skipped when running as root (open succeeds).
- **Red before:** guardrail; mutation RED: treating every open error as `NotFound` makes `put` rewrite and return `Ok`.

#### DT-004: guardrail
- **Validates:** DR-004. 8.1 green; `git diff main -- tool-contract-golden.json` empty.

### 8.4 Existing tests to modify
None.

## 9. Implementation order
1. Add the hook at the current refresh point (no behavior change), write DT-001 and record RED; write DT-002, DT-003 (mutation RED for DT-003).
2. Restructure `put` per DR-001, DR-002; 8.1 green. One story.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline, depth S): no forced choice; names and both forms spelled; tests concrete with a second channel; no open question; EARS clean, no forbidden modal; delta declared (Section 7); citations opened (`local.rs:40-70`, `:188`, `:199`, `:237`, `:325`, `s3.rs:48-58`, `traits.rs:244`); G13 not triggered; existing tests untouched |

**Amendments applied:** none.

## Design additions (2026-10-18)

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/blob/local.rs` | `put` restructure, test hook, DT-001..DT-003 | DR-001..DR-004 | US-0045 |

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001, DT-002, DT-003 | unit | `LocalBlobStore` direct call, tempdir | `local.rs` tests | US-0045 |
| DT-004 | guardrail | full suite | `make check` | US-0045 |

### Additional decisions
- **DEC-003:** One story at `min_tier: 2`, the hook as a `thread_local!` `Cell<Option<fn(&Path)>>` read in async context before `spawn_blocking`, so a `#[tokio::test]` (current thread runtime) sets it on the same thread. **Obliged by:** SPEC-0038/DR-001..DR-004. **Alternatives considered:** a global `static` hook (rejected: parallel tests would race on it). **Source:** design. **Evidence:** `local.rs:47-53`.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0045 | local put rewrites a blob deleted mid call | P1 | DR-001..DR-004 | main | 2 | yes |
