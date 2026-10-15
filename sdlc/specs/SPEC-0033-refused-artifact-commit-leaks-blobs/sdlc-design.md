# A refused artifact commit leaves no unreferenced blob: technical debt design

> Id: SPEC-0033
> Nature: DEBT
> Depth: M
> Depth evidence: 2 modules (`docs/artifacts`, `storage/meta`), 7 requirements, no public contract, no persisted data change
> Status: implemented
> Generated on: 2026-10-15
> Behavior change: none
> From backlog: BL-0040
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt

### DBT-001: A refused artifact commit leaks the blobs it stored
- **Current structure:** `PendingSet::commit` (`crates/core/src/docs/artifacts/capture.rs:150`) stores the conversion text blob (`:161`) and every picture blob (`:167`), then calls `RelationalMetaStore::commit_artifact_set` (`:206`). That commit checks, inside its transaction, that the node still carries the captured revision and refuses otherwise (`crates/core/src/storage/meta.rs:706-708`) with `ERR_INVALID_ARGUMENT` and the message `Document changed during conversion: <path>` (`meta.rs:70`, `:1155-1156`). The blobs' references are taken only inside that same transaction (`tx_incref`, `meta.rs:779`), so a refused commit leaves them with no `blob_refs` row. The comment at `capture.rs:147-149` records it as accepted.
- **Cost paid today:** every refused commit (a document edited while it converts, the realistic trigger; or a database error) leaves the text and picture bytes in the project's blob store forever. No sweep lists blob stores (`crates/core/src/storage/traits.rs:232-236`: `put`, `exists`, `delete`, no `list`), and the quota does not count them (SPEC-0020/FR-NEW-002 excludes document artifacts), so the leak is invisible to the user and to the quota.
- **Target structure:** when `commit_artifact_set` returns an error, `PendingSet::commit` deletes each blob it stored in this call that the volume does not reference, then returns the original error unchanged.
- **Observable behavior:** unchanged. The caller sees the same error; only unreferenced storage disappears.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/docs/artifacts/capture.rs` | `PendingSet::commit` (`:150-207`): on an `Err` from `commit_artifact_set`, release the hashes it stored; update the comment at `:147-149` | 1 function |
| `crates/core/src/storage/meta.rs` | a read of whether a hash has a `blob_refs` row in this volume (same table and key as `tx_decref`, `:1214-1230`; the test only reader `blob_refcount_for_test` at `:2069` already reads it) | 1 new method |
| `crates/core/src/core/fs_ops.rs` (test module) | new tests DT-010..DT-014 beside `late_refusal` (`:3984`) using the `PausingBlob` wrapper (`:3886`) | tests only |

**Non source occurrences:** none (no doc names this behavior; `.agent_docs/` is silent on it).
**Contracts touched:** none public. `PendingSet::commit` and `RelationalMetaStore` are crate internal.
**Callers of `PendingSet::commit`:** `fs_ops.rs:885` (converter path) and the builtin path built at `fs_ops.rs:1731`; both reach the change.
**Guardrail coverage:** `a_late_refusal_restores_the_former_sibling_and_refunds` (`fs_ops.rs:4026`), `a_late_refusal_deletes_a_sibling_that_did_not_exist` (`:4035`), the refusal assertion at `crates/core/src/tools/artifacts.rs:3503`, plus the full suite. **Test command:** see 8.1.
**Total:** 3 files, recounted.

## 3. Class sweep
Not a security lot.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-O]: Release on refusal
> IF `commit_artifact_set` returns an error THEN `PendingSet::commit` SHALL delete, for each hash it stored in this call (the text hash and every picture hash), the blob of that hash when the volume has no `blob_refs` row for it.

#### DR-002 [EARS-UB]: Never delete a referenced blob
> `PendingSet::commit` SHALL NOT delete a blob whose hash has a `blob_refs` row in the volume, whatever referenced it (a file node or another artifact set).

#### DR-003 [EARS-U]: The refusal is returned unchanged
> On a refused commit, `PendingSet::commit` SHALL return the error `commit_artifact_set` returned, code and message byte identical.

#### DR-004 [EARS-O]: A cleanup failure never masks the refusal
> IF deleting a blob fails during the release THEN `PendingSet::commit` SHALL log the failure at warn level with the hash and continue with the remaining hashes, and SHALL still return the original commit error.

#### DR-005 [EARS-U]: Each hash released once
> The release SHALL consider each distinct hash once, even when the text and a picture, or two pictures, share a hash.

#### DR-006 [EARS-UB]: A committed set keeps its blobs
> On a successful commit, `PendingSet::commit` SHALL NOT delete any blob.

### 4.2 Invariance
#### DR-007 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

## 6. Decisions
- **DEC-001:** Compensating release after a refused commit. **Alternatives considered:** a general orphan sweep over blob stores, rejected for this lot (needs a new `list` operation on the local and S3 backends, must skip `git:` and `export:` keys, needs an age threshold for in flight writes; deferred to BL-0056); checking the revision before storing, rejected because it narrows the window without closing it; storing blobs inside the database transaction, rejected because it holds a transaction across blob I/O. **Rationale:** closes exactly the leak BL-0040 names with the smallest change. **Implemented by:** DR-001, DR-006. **Evidence:** `capture.rs:150-207`, `meta.rs:706-708`.
- **DEC-002:** "Referenced" means a `blob_refs` row for `(volume_id, sha256)`, the one reference count files and artifacts already share. **Alternatives considered:** checking `nodes`, `doc_artifact_sets` and `doc_images` separately, rejected because it duplicates what `tx_incref`/`tx_decref` maintain (`meta.rs:1179`, `:1214`) and could disagree with it. **Implemented by:** DR-001, DR-002. **Evidence:** `meta.rs:779`, `:1218`.
- **DEC-003:** The release is best effort: delete errors are logged and do not replace the refusal. **Alternatives considered:** returning the delete error, rejected because it would change the error a caller sees (DR-003). **Implemented by:** DR-003, DR-004. **Evidence:** n/a.
- **Assumption:** a concurrent writer storing the same bytes between the reference check and the delete could lose its blob. This window already exists for every content addressed GC in the tree (`crates/core/src/storage/volume.rs:170-172` deletes after the node transaction), so the release adds no new class of race; noted, not solved here.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None.

### 8.3 Structural tests
All run through the real conversion path (`fs_ops` converter, `fs_ops.rs:885`) with a pausing blob backend modeled on `PausingBlob` (`fs_ops.rs:3886`): the commit is held, the document is rewritten, the commit is released and refused.

#### DT-010: A refused commit leaves no unreferenced blob
- **Validates:** DR-001.
- Given a document `/deck.pptx` whose conversion yields the text `# deck\n` and one picture / When the document is rewritten while the commit is held, then the commit is released / Then the call fails with `Document changed during conversion: /deck.pptx` / And `blob.exists(<text sha>)` and `blob.exists(<picture sha>)` are both `false`.
- **Red before the change because:** both `exists` calls return `true`.

#### DT-011: A shared hash is kept
- **Validates:** DR-002.
- Given a file `/copy.md` whose bytes are exactly `# deck\n` (same hash as the conversion text) / When the commit is refused as in DT-010 / Then `blob.exists(<text sha>)` is `true` and reading `/copy.md` returns `# deck\n`.

#### DT-012: The refusal is unchanged
- **Validates:** DR-003.
- Then the error code is `ERR_INVALID_ARGUMENT` and the message is exactly `Document changed during conversion: /deck.pptx`.

#### DT-013: A failing delete does not mask the refusal
- **Validates:** DR-004.
- Given a blob backend whose `delete` returns an error / When the commit is refused / Then the caller still receives the DT-012 error, and the picture blob is still attempted after the text delete fails (the backend records 2 delete calls).

#### DT-014: Shared hashes released once; a committed set keeps its blobs
- **Validates:** DR-005, DR-006.
- Given a conversion whose text and picture share one hash / When the commit is refused / Then the backend records exactly 1 delete for that hash. Given an unrefused commit / Then the backend records 0 deletes and `blob.exists` is `true` for every stored hash.

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|

Empty.

## 9. Implementation order
1. Add the reference read on `RelationalMetaStore` (unused, suite green).
2. Write DT-010..DT-014, record DT-010 red.
3. Add the release in `PendingSet::commit`, update its comment; all green; 8.1.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none. Auditor: fresh context subagent, path and contract only; every citation opened, no finding.

**Process note:** the test plan was designed in the main run and approved by the owner before the depth M test designer subagent ran; no separate test designer was spawned.

## Design additions (2026-10-15)

### File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | add `pub(crate) async fn blob_is_referenced(&self, sha: &str) -> Result<bool>` (`SELECT 1 FROM blob_refs WHERE volume_id=?1 AND sha256=?2`) | DR-001, DR-002 | US-0036 |
| `crates/core/src/docs/artifacts/capture.rs` | `PendingSet::commit`: on `Err`, release distinct stored hashes not referenced, warn on delete error, return the original error; rewrite the comment at `:147-149` | DR-001..DR-006 | US-0036 |
| `crates/core/src/core/fs_ops.rs` (test module) | DT-010..DT-014 | DR-001..DR-006 | US-0036 |

Docs: none.

### Test strategy

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-010..DT-014 | integration | converter path with a pausing (and, for DT-013, delete failing) blob backend | `crates/core/src/core/fs_ops.rs` tests, beside `late_refusal` (`:3984`) | US-0036 |
| 8.1 | non regression | `make check` with DB DSNs | workspace | US-0036 |

- **DEC-004:** One story at `min_tier: 1`. **Obliged by:** SPEC-0033/DR-001..DR-007. **Rationale:** the release and its two guards (never a referenced blob, never mask the refusal) are not separately assertable, so the unit stays whole (floor check); 7 requirements and a concurrency harness exceed the tier 2 budget. **Alternatives considered:** a reference read story then a release story, rejected because the first would land an unused method with no observable test. **Source:** design. **Evidence:** Section 2.

### Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0036 | A refused artifact commit releases its blobs | P1 | DR-001..DR-007 | main | 1 | yes |
