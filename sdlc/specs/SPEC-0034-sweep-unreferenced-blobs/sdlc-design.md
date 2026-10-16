# Sweep blobs no reference holds: technical debt design

> Id: SPEC-0034
> Nature: DEBT
> Depth: L
> Depth evidence: 4 modules (storage traits, local and S3 blob backends, storage meta, purge), a deleting background job, 10 requirements, no public contract (owner chose option (i): no setting, no command)
> Status: designed
> Generated on: 2026-10-15
> Behavior change: none for users and API consumers; the purge cycle log gains three counters
> From backlog: BL-0056
> Depends on: SPEC-0033 (`RelationalMetaStore::blob_is_referenced`)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt

### DBT-001: Blobs left with no reference are never collected
- **Current structure:** every write stores its blob before its metadata transaction (`crates/core/src/storage/volume.rs:161-172`, `crates/core/src/docs/artifacts/capture.rs:150-213`, `crates/core/src/tools/export.rs:90-97`). A crash or database error between the two leaves an object that nothing references. Blobs are only ever deleted when a known reference reaches zero (`.agent_docs/architecture.md:62-64`) or by an export's expiry, which starts from its `export_links` row (`crates/core/src/purge.rs:282-320`). The blob store cannot even enumerate its objects: `BlobBackend` (`crates/core/src/storage/traits.rs:231-238`) has `put`, `get`, `exists`, `delete`, `ensure_bucket`, `remove_bucket`, no `list`. On local disk a crash mid write also leaves a `.tmp-{sha}-{uuid}` file (`crates/core/src/storage/blob/local.rs:48`).
- **Cost paid today:** leftover bytes accumulate forever in each project's bucket, invisible to users and outside the quota (SPEC-0020/FR-NEW-002 counts nodes, not bucket bytes). Small and rare (SPEC-0033 closed the frequent refused artifact case), but unbounded.
- **Target structure:** the purge cycle runs, once a day per project, a sweep that lists the bucket and deletes, after a 24 hour safety age, the content blobs with no `blob_refs` row, the export zips with no `export_links` row, and the local temp files; protected by a re check before each delete and a circuit breaker.
- **Observable behavior:** unchanged for users and API consumers. Operators see three new counters in the purge cycle log line.

### Owner decisions (interview, 2026-10-15)
1. No setting and no command: the sweep always runs inside the purge cycle (option (i)).
2. In scope: content blobs, `export:` zips without a row, local `.tmp-*` files. Out: `git:` objects (git's own reachability rules apply; unreachable objects are kept on purpose so a reset can be undone, `crates/core/src/mcp/server.rs:3188`).
3. Safety age: 24 hours.
4. A local `put` that finds the blob already present refreshes its modified time; the sweep re checks age and reference just before each delete.
5. Frequency: at most once every 24 hours per project per process, last run kept in memory.
6. A circuit breaker protects against a broken reference check.
7. Breaker limit: max(1, floor(10% of the project's content blobs)).

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/storage/traits.rs` | `BlobBackend` (`:231-238`) gains `async fn list(&self) -> Result<Vec<BlobEntry>>` with a default body returning `ERR_NOT_SUPPORTED`, and `async fn delete_entry(&self, entry: &BlobEntry) -> Result<()>` with a default body calling `delete(&entry.key)`; new `pub struct BlobEntry { pub key: String, pub size: u64, pub modified: f64, pub shard: Option<String> }` (`modified` in unix seconds; `shard` set by the local store to the directory the file was found in) | 2 trait methods, 1 struct |
| `crates/core/src/storage/blob/local.rs` | `list` walks every shard directory under the bucket root (layout `{root}/{key[..2]}/{key}`, `path_for` `:29-35`), including `.tmp-*` entries (a temp file lives in the shard directory of the blob it was writing, `local.rs:41-48`, not under `path_for` of its own name), and overrides `delete_entry` to remove exactly `{root}/{shard}/{key}`; `put` (`:39-62`): the early return at `:44-46` becomes "refresh the modified time, then return" | 3 methods |
| `crates/core/src/storage/blob/s3.rs` | `list` pages `list_objects_v2` with continuation tokens (pattern already in `remove_bucket`, `:137-166`), key, size, `last_modified` | 1 method |
| `crates/core/src/storage/meta.rs` | `RelationalMetaStore::referenced_hashes(&self) -> Result<HashSet<String>>` (`SELECT sha256 FROM blob_refs WHERE volume_id=?1`); reuse `blob_is_referenced` (`:697`); a free function `export_link_exists(tx, project_id, token) -> Result<bool>` beside `delete_export_link_if_live` (`:1575`) and `select_expired_export_links` (`:1593`) | 2 new functions |
| `crates/core/src/purge.rs` | new `sweep_project_orphans_at(now, stores, client, project_id) -> Result<OrphanSweep>`; called from `run_cycle` (`:210-255`) after `sweep_project_artifacts` (`:248`), gated by the per project schedule; `CycleSummary` (`:198-203`) gains `orphan_blobs_swept`, `orphan_bytes_swept`, `orphan_breakers_tripped` | 1 function, 3 fields |
| `crates/core/src/storage/mod.rs` | `StoreManager` (`:416-424`) gains the in memory schedule `orphan_sweeps: Mutex<HashMap<String, f64>>` (project id to last run, unix seconds), so a fresh manager is a fresh process | 1 field |
| `crates/core/src/app.rs` | the cycle log line (`:223-227`) adds the three counters | 1 call |
| `.agent_docs/architecture.md` | document the sweep next to "GC at refcount 0" (`:61-64`) | 1 section |

**Non source occurrences:** `.agent_docs/architecture.md:61-64` (documentation of GC), nothing in CI, Makefile, container files or `.env.example`.
**Contracts touched:** all internal. `BlobBackend`, `RelationalMetaStore`, `StoreManager`, `CycleSummary` are crate internal; the purge log line is operational output, not a documented contract. The local on disk layout (`AGENTS.md:184`, `.agent_docs/architecture.md:68-69`) is unchanged: `list` reads it, nothing rewrites it.
**Implementers of `BlobBackend`:** 2 production (`local.rs:38`, `s3.rs:47`), 2 test doubles (`crates/core/src/core/fs_ops.rs:3895` `PausingBlob`, `:4069` `RecordingBlob`); the default `list` keeps the doubles compiling unchanged.
**Callers of `run_cycle`:** 14 call sites, the signature does not change: `crates/core/src/app.rs:215`; 11 tests in `crates/core/src/purge.rs` (`:1041`, `:1057`, `:1088`, `:1505`, `:1515`, `:1526`, `:1540`, `:1556`, `:1571`, `:1627`, `:1688`); `crates/core/src/tools/teardown_e2e.rs:408`; `crates/core/src/tools/trash_search_e2e.rs:169`. `CycleSummary` is never built as a struct literal outside its definition (`rg "CycleSummary \{"` finds only `purge.rs:198`).
**Guardrail coverage:** local blob tests `crates/core/src/storage/blob/local.rs:124-227` (9 tests including `put_is_idempotent`, `no_temp_files_left_behind`, `on_disk_layout_is_sharded_by_first_two_chars`), S3 `integration_put_get_range_delete` (`s3.rs:194-212`, `#[ignore]`, needs `MCPFS_MINIO_SECRET_KEY`), purge cycle tests at the call sites above. **Test command:** see 8.1.
**Total:** 7 source files plus 1 doc, recounted.

## 3. Class sweep
Not a security lot.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-U]: The blob store can list its objects
> `BlobBackend` SHALL provide `list`, returning every object of the bucket as a `BlobEntry` with its key, its size in bytes and its last modified time; `LocalBlobStore` and `S3BlobStore` SHALL implement it, and the default implementation SHALL return an `ERR_NOT_SUPPORTED` error.
- **Local:** every file under every shard directory of the bucket root, `.tmp-*` files included, the key being the file name and `shard` the directory it was found in.
- `BlobBackend` SHALL also provide `delete_entry(&BlobEntry)`, defaulting to `delete(&entry.key)`; `LocalBlobStore` SHALL remove exactly the listed file (`{root}/{shard}/{key}`), so a temp file is deletable although its name does not map to its directory.
- The sweep SHALL delete every object through `delete_entry`.
- **S3:** every key across all `list_objects_v2` pages.

#### DR-002 [EARS-E]: Re storing existing bytes refreshes their age
> WHEN `LocalBlobStore::put` finds the blob already present THE store SHALL set its modified time to now and leave its bytes unchanged.

#### DR-003 [EARS-O]: What the sweep deletes
> IF an object qualifies THEN the sweep SHALL delete it. An object qualifies when it is older than 24 hours (strictly: modified time before now minus 86 400 s) and is one of:
> 1. a content blob: key of exactly 64 characters in `[0-9a-f]` with no `blob_refs` row for `(volume_id, sha256)`;
> 2. an export zip: key `export:{token}` with no `export_links` row for `(project_id, token)`;
> 3. a temp file: key starting with `.tmp-` (local store only).

#### DR-004 [EARS-UB]: What the sweep never deletes
> The sweep SHALL NOT delete a `git:` key, a key of any other form, a content blob with a `blob_refs` row, an export zip with an `export_links` row, or any object 24 hours old or younger.

#### DR-005 [EARS-E]: Re check before each delete
> WHEN the sweep is about to delete a candidate THE sweep SHALL re read that object's modified time and its reference (`blob_is_referenced` for a content blob, `export_link_exists` for an export zip) and SHALL skip it when it no longer qualifies.

#### DR-006 [EARS-O]: Circuit breaker
> IF the number of qualifying content blobs of a project exceeds max(1, floor(10% of the content blobs listed for that project, referenced or not, any age)) THEN the sweep SHALL delete no content blob of that project in that run, SHALL count one `orphan_breakers_tripped`, and SHALL log one warning naming the project, the qualifying count and the content blob count.
- Export zips and temp files are outside the breaker and are still swept in that run.

#### DR-007 [EARS-S]: Once a day per project
> WHILE less than 24 hours have passed since the last sweep of a project by this process THE purge cycle SHALL skip that project's sweep; a process that has not swept a project yet SHALL sweep it on its first cycle.
- The last run time is recorded when the sweep of the project completes or fails, so a failing project is retried after 24 hours, not every cycle.

#### DR-008 [EARS-O]: Failures are isolated
> IF listing, a reference read or a delete fails THEN the sweep SHALL log a warning with the project and the error, SHALL skip that object (or that project, for a listing or bulk reference read failure), and SHALL NOT abort the cycle, another project or another sweep step.
- A backend answering `ERR_NOT_SUPPORTED` to `list` is skipped silently (no warning).

#### DR-009 [EARS-U]: Counters
> `CycleSummary` SHALL carry `orphan_blobs_swept` (objects deleted), `orphan_bytes_swept` (sum of their listed sizes) and `orphan_breakers_tripped`, summed over projects, and the purge loop log line SHALL include all three.

### 4.2 Invariance
#### DR-010 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

**Rollback:** reverting the change stops the sweep; nothing persisted depends on it (the schedule is in memory, no table, no migration).

## 6. Decisions
- **DEC-001:** List the bucket, load the volume's referenced hashes in one query (`referenced_hashes`), diff in memory, then re check each candidate (DR-005). **Alternatives considered:** one reference query per listed object, rejected (one query per object, a million queries for a million objects); a mark then sweep table, rejected by the owner in favor of refreshing the local modified time (decision 4). **Rationale:** O(n) over the listing plus O(k) re checks for k candidates. **Implemented by:** DR-003, DR-005. **Evidence:** `crates/core/src/storage/meta.rs:697`.
- **DEC-002:** `list` is a trait method with a default `ERR_NOT_SUPPORTED` body. **Alternatives considered:** a required method, rejected because the two test doubles (`fs_ops.rs:3895`, `:4069`) would have to change; a separate listing trait, rejected as a second abstraction for one call. **Implemented by:** DR-001, DR-008. **Evidence:** `crates/core/src/storage/traits.rs:231-238`.
- **DEC-003:** "Older than 24 hours" is strict (exactly 24 hours is kept). **Alternatives considered:** inclusive. **Rationale:** the conservative side for a deleting job. **Implemented by:** DR-003, DR-004. **Evidence:** n/a.
- **DEC-004:** The breaker denominator is every content key listed for the project (referenced or not, any age). **Alternatives considered:** only old keys, rejected because a project of young files would trip on its first leftover. **Implemented by:** DR-006. **Evidence:** n/a.
- **DEC-005:** The schedule lives in `StoreManager`, keyed by project id, and the sweep takes `now` as a parameter (`sweep_project_orphans_at`, mirroring `sweep_project_files_at` and `sweep_project_artifacts_at`, `purge.rs:268`); `run_cycle` keeps its signature and passes the current time. **Alternatives considered:** a process global static, rejected because parallel tests would share it; a new `run_cycle` parameter, rejected because 13 existing tests and the loop call it (14 call sites, Section 2). **Implemented by:** DR-007. **Evidence:** `crates/core/src/storage/mod.rs:416-424`, `crates/core/src/purge.rs:210-215`.
- **DEC-006:** The breaker outcome is a counter (`orphan_breakers_tripped`) asserted by tests, plus a warning log. **Alternatives considered:** asserting the log line, rejected because it needs a log capture dependency the workspace does not have. **Implemented by:** DR-006, DR-009. **Evidence:** `Cargo.toml` has no `tracing-test`.
- **DEC-007:** The sweep reaches the relational store through `client.trash` (the pattern of `sweep_project_artifacts_at`, `purge.rs:268-269`) and the export table through `open_meta_db` (the pattern of `sweep_project_exports`, `purge.rs:296-304`); a client with no relational store behind it is skipped. **Alternatives considered:** new `MetaBackend` trait methods, rejected because 3 test doubles implement `MetaBackend` (`fs_ops.rs:3148`, `purge.rs:727`, `purge.rs:1368`). **Implemented by:** DR-003, DR-005. **Evidence:** cited.
- **DEC-008:** Tests backdate files with the standard library (`std::fs::File::set_modified`). **Alternatives considered:** the `filetime` crate, rejected as a new dependency. **Implemented by:** DR-002, DR-003 tests. **Evidence:** n/a.
- **Assumption:** one bucket per project (`crates/core/src/config.rs:945-947`, `crates/core/src/storage/mod.rs:211-216`); were two projects to share a bucket, the sweep would delete the other's blobs. Not configurable today.
- **Assumption:** a residual window of milliseconds remains between the final re check and the delete, on both backends (S3 has no conditional delete here). It is the window the existing delete at zero path already accepts (`volume.rs:170-172`, SPEC-0033 assumption).
- **Assumption:** single server process per database (BL-0016); the in memory schedule is per process.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

Designed by an independent test designer subagent (2026-10-15), approved by the owner.

Shared fixture for purge tests: one project with a local blob store in a temp dir, built like the `run_cycle` tests (`purge.rs:1013`); ages set with `File::set_modified`; `now` injected. `A` a referenced content hash, `O` an orphan content hash 25 h old, `Y` an orphan 1 h old.

### 8.2 Compatibility tests
None.

### 8.3 Structural and behavior tests
All tests below close DBT-001 (blobs left with no reference are never collected) through the requirement each one names.

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| DT-010 | structural | DR-001 | `blob/local.rs` tests | Given an empty store, `list()` returns `[]`; given puts of 3 and 5 bytes plus a file `.tmp-ab-x` written by hand in shard `ab`, `list()` returns 3 entries of sizes {3, 5, len}, `modified` within 2 s of now, the temp entry with `shard == Some("ab")`; `delete_entry` on it removes `{root}/ab/.tmp-ab-x` | `list` does not exist |
| DT-011 | structural | DR-001 | `fs_ops.rs` tests, `PausingBlob` | a double not overriding `list` returns an error with code `ERR_NOT_SUPPORTED` | no default method |
| DT-012 | edge | DR-001 | `blob/s3.rs` `#[ignore]` MinIO suite (beside `s3.rs:194-212`) | 1001 keys put, `list()` returns 1001 entries with correct sizes (pagination followed) | no `list` |
| DT-013 | behavior | DR-002 | `blob/local.rs` tests | blob `X` backdated 48 h, `put(X)` again, its modified time is within 2 s of now and its bytes unchanged | early return at `local.rs:44-46` keeps the old time |
| DT-014 | behavior | DR-003, DR-004 | `purge.rs` tests | given referenced `A` (25 h), `O` (25 h), `Y` (1 h), `git:<sha>` (25 h), `export:t1` (25 h) with a row, `export:t2` (25 h) without, `.tmp-x` (25 h), `.tmp-y` (1 h), `misc:k` (25 h), plus 20 referenced blobs; after the sweep exactly `O`, `export:t2`, `.tmp-x` are gone and every other key `exists()` | no sweep |
| DT-015 | edge | DR-003 | `purge.rs` tests | an orphan aged exactly 86 400 s is kept, one aged 86 401 s is deleted | no sweep |
| DT-016 | edge | DR-003, DR-004 | `purge.rs` tests | a 64 character uppercase hex key and a 63 character lowercase hex key, both 25 h old and unreferenced, are kept | no sweep |
| DT-017 | concurrency | DR-005, DR-002 | `purge.rs` tests, a pausing double over `LocalBlobStore` (pattern `PausingBlob`) | sweep paused after listing; a file whose bytes hash to `O` is written (reference plus a `put` refreshing the time); sweep resumes; `O` exists and the file reads back its bytes | no sweep |
| DT-018 | concurrency | DR-005 | same | sweep paused after listing; only a `blob_refs` row for `O` is added; `O` is kept | no sweep |
| DT-019 | guardrail | DR-006 | `purge.rs` tests | 20 content blobs, 2 old orphans, limit max(1, 2) = 2: both deleted, `orphan_breakers_tripped == 0` | no sweep |
| DT-020 | guardrail | DR-006 | `purge.rs` tests | 20 content blobs, 3 old orphans, plus `export:t2` and `.tmp-x`: the 3 orphans kept, `orphan_breakers_tripped == 1`, the export zip and the temp file deleted | no sweep |
| DT-021 | edge | DR-006 | `purge.rs` tests | a project whose only content blob is an old orphan (limit 1): deleted; a project whose 2 content blobs are both old orphans (limit 1): both kept | no sweep |
| DT-022 | behavior | DR-007 | `purge.rs` tests | fresh `StoreManager`, old orphan: swept on the first cycle; a second orphan added, cycle at +23 h: kept; cycle at +24 h 1 min: swept; a new `StoreManager` over the same store sweeps on its first cycle | no sweep |
| DT-023 | failure | DR-008, DR-001 | `purge.rs` tests | project P1 whose backend `list` fails, P2 with an old orphan and a stale file: `run_cycle` returns `Ok`, P2's orphan swept, P2's existing purge counts unchanged | no sweep |
| DT-024 | failure | DR-008 | `purge.rs` tests, a double whose second `delete` fails | 3 old orphans, no breaker: 2 deleted, 1 remains, cycle `Ok`, `orphan_blobs_swept == 2` | no sweep |
| DT-025 | behavior | DR-009 | `purge.rs` tests | DT-014 setup with `O` 7 bytes, `export:t2` 11 bytes, `.tmp-x` 5 bytes: `orphan_blobs_swept == 3`, `orphan_bytes_swept == 23` | fields do not exist |
| DT-026 | edge | DR-001, DR-006 | `purge.rs` tests | empty bucket: counters 0, `orphan_breakers_tripped == 0`, cycle `Ok` | no sweep |
| DT-027 | edge | DR-003 | `purge.rs` tests | 10 000 referenced blobs and 5 old orphans: exactly 5 deleted | no sweep |
| DT-028 | guardrail | DR-010 | full suite | 8.1 green with no edit to an existing test, `tool-contract-golden.json` unchanged | n/a (invariance) |

By category: structural 2, behavior 4, guardrail 3, concurrency 2, failure 2, edge 6.

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|

Empty. `put_is_idempotent` (`local.rs` tests) asserts bytes, not time, so DR-002 keeps it green (to be confirmed by the RED run).

## 9. Implementation order
1. DR-001, DR-002: `list` on the trait, local and S3, and the local time refresh, with DT-010..DT-013. Nothing calls `list` yet; the suite stays green.
2. DR-003..DR-006, DR-008, DR-010: `referenced_hashes`, `export_link_exists`, `sweep_project_orphans_at` with the re check and the breaker, tested directly, DT-014..DT-021, DT-024, DT-027. Not wired into `run_cycle` yet.
3. DR-007, DR-009: the schedule in `StoreManager`, the call from `run_cycle`, the counters and the log line, `.agent_docs/architecture.md`, DT-022, DT-023, DT-025, DT-026, DT-028.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 2 | 0 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** round 1: F-001 caller count recounted to 14 (Section 2, DEC-005); F-002 test citations corrected (`local.rs:124-227`, `s3.rs:194-212`); self consistency check 1: DBT-001 referenced from 8.3. Before the gate, the author added `delete_entry` and `BlobEntry.shard` (DR-001) after finding that a temp file's name does not map to its directory (`local.rs:41-48`).

**Gate process:** depth L. Round 1: self consistency reviewer (fresh subagent) FAIL on check 1 only (DBT-001 not referenced by the tests section), auditor (fresh subagent, path and contract only) 2 F (perimeter count, citations). Round 2: a new auditor, path and contract only, every citation reopened and every count recounted: no finding. Tests designed by an independent test designer subagent before owner approval.

## Design additions (2026-10-16)

### File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/traits.rs` | `list`, `delete_entry`, `BlobEntry` | DR-001 | US-0037 |
| `crates/core/src/storage/blob/local.rs` | `list`, `delete_entry`, `put` refresh | DR-001, DR-002 | US-0037 |
| `crates/core/src/storage/blob/s3.rs` | `list` | DR-001 | US-0037 |
| `crates/core/src/core/fs_ops.rs` (tests) | DT-011 | DR-001 | US-0037 |
| `crates/core/src/storage/meta.rs` | `referenced_hashes`, `export_link_exists` | DR-003, DR-005 | US-0038 |
| `crates/core/src/purge.rs` | `sweep_project_orphans_at`, `OrphanSweep` | DR-003..DR-006, DR-008 | US-0038 |
| `crates/core/src/storage/mod.rs` | schedule in `StoreManager` | DR-007 | US-0039 |
| `crates/core/src/purge.rs` | `run_cycle` wiring, `run_cycle_at`, `CycleSummary` counters | DR-007..DR-009 | US-0039 |
| `crates/core/src/app.rs` | cycle log counters | DR-009 | US-0039 |
| `.agent_docs/architecture.md` | the sweep next to GC at refcount 0 | DR-010 (docs) | US-0039 |

### Test strategy

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-010, DT-013 | unit | direct call | `storage/blob/local.rs` tests | US-0037 |
| DT-011 | unit | direct call on a test double | `core/fs_ops.rs` tests | US-0037 |
| DT-012 | integration (`#[ignore]`, MinIO) | direct call | `storage/blob/s3.rs` tests | US-0037 |
| DT-014..DT-021, DT-024, DT-027 | integration | `sweep_project_orphans_at` over a real local store and relational store | `purge.rs` tests | US-0038 |
| DT-022, DT-023, DT-025, DT-026 | integration | `run_cycle_at` | `purge.rs` tests | US-0039 |
| DT-028 | non regression | `make check` with DB DSNs | workspace | US-0039 |

- **DEC-009:** Three stories along Section 9, the middle one at `min_tier: 1`. **Obliged by:** SPEC-0034/DR-001..DR-010. **Rationale:** each step leaves main green and is independently testable (the listing before any caller, the sweep tested directly before wiring); the sweep combines a concurrency harness, a breaker and a re check whose halves are not separately assertable, above the tier 2 decision budget. **Alternatives considered:** one story (above every tier budget: 10 requirements, 19 tests, 8 files); splitting the sweep into breaker and re check stories (not separately assertable). **Source:** design. **Evidence:** Section 9.
- **DEC-010:** A test only `run_cycle_at(now, ...)` beside `run_cycle`, which passes the current time. **Obliged by:** SPEC-0034/DR-007 (DT-022 needs an injected clock). **Rationale:** the `_at` pattern already exists (`purge.rs:268`) and the 14 callers keep `run_cycle`. **Alternatives considered:** a clock trait object on `StoreManager` (more surface for one test). **Source:** existing code `purge.rs:268`. **Evidence:** cited.

### Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0037 | The blob store lists its objects and refreshes re stored blobs | P1 | DR-001, DR-002, DR-010 | main | 2 | no |
| US-0038 | Sweep the unreferenced blobs of one project | P1 | DR-003..DR-006, DR-008, DR-010 | US-0037 | 1 | no |
| US-0039 | The purge cycle runs the orphan sweep once a day | P1 | DR-007..DR-010 | US-0038 | 2 | yes |
