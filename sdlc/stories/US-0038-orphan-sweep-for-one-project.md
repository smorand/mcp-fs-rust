# US-0038: Sweep the unreferenced blobs of one project

> Spec: SPEC-0034 (sdlc/specs/SPEC-0034-sweep-unreferenced-blobs/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0037
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Build the sweep for one project, tested directly but not yet wired into the purge cycle: delete old unreferenced content blobs, export zips without a row and local temp files, never git objects or anything else, re check each candidate just before deleting it, and refuse to delete content blobs when too many look unreferenced (a broken reference check would otherwise empty the project).

## Requirements covered
#### SPEC-0034/DR-003 [EARS-O]: What the sweep deletes
> IF an object qualifies THEN the sweep SHALL delete it. An object qualifies when it is older than 24 hours (strictly: modified time before now minus 86 400 s) and is one of:
> 1. a content blob: key of exactly 64 characters in `[0-9a-f]` with no `blob_refs` row for `(volume_id, sha256)`;
> 2. an export zip: key `export:{token}` with no `export_links` row for `(project_id, token)`;
> 3. a temp file: key starting with `.tmp-` (local store only).

#### SPEC-0034/DR-004 [EARS-UB]: What the sweep never deletes
> The sweep SHALL NOT delete a `git:` key, a key of any other form, a content blob with a `blob_refs` row, an export zip with an `export_links` row, or any object 24 hours old or younger.

#### SPEC-0034/DR-005 [EARS-E]: Re check before each delete
> WHEN the sweep is about to delete a candidate THE sweep SHALL re read that object's modified time and its reference (`blob_is_referenced` for a content blob, `export_link_exists` for an export zip) and SHALL skip it when it no longer qualifies.

#### SPEC-0034/DR-006 [EARS-O]: Circuit breaker
> IF the number of qualifying content blobs of a project exceeds max(1, floor(10% of the content blobs listed for that project, referenced or not, any age)) THEN the sweep SHALL delete no content blob of that project in that run, SHALL count one `orphan_breakers_tripped`, and SHALL log one warning naming the project, the qualifying count and the content blob count.
- Export zips and temp files are outside the breaker and are still swept in that run.

#### SPEC-0034/DR-008 [EARS-O]: Failures are isolated
> IF listing, a reference read or a delete fails THEN the sweep SHALL log a warning with the project and the error, SHALL skip that object (or that project, for a listing or bulk reference read failure), and SHALL NOT abort the cycle, another project or another sweep step.
- A backend answering `ERR_NOT_SUPPORTED` to `list` is skipped silently (no warning).

#### SPEC-0034/DR-010 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.


## Independent Test
`cargo test -p mcp-fs-core --lib purge::tests` passes with DT-014..DT-021, DT-024, DT-027, each red before the sweep exists (the function is absent; RED is recorded as an assertion failure by first writing the tests against a stub `sweep_project_orphans_at` returning `OrphanSweep::default()` and deleting nothing).

## Design excerpt
### Decisions
- **(SPEC-0034) DEC-001:** List the bucket, load the volume's referenced hashes in one query (`referenced_hashes`), diff in memory, then re check each candidate (DR-005). **Alternatives considered:** one reference query per listed object, rejected (one query per object, a million queries for a million objects); a mark then sweep table, rejected by the owner in favor of refreshing the local modified time (decision 4). **Rationale:** O(n) over the listing plus O(k) re checks for k candidates. **Implemented by:** DR-003, DR-005. **Evidence:** `crates/core/src/storage/meta.rs:697`.
- **(SPEC-0034) DEC-003:** "Older than 24 hours" is strict (exactly 24 hours is kept). **Alternatives considered:** inclusive. **Rationale:** the conservative side for a deleting job. **Implemented by:** DR-003, DR-004. **Evidence:** n/a.
- **(SPEC-0034) DEC-004:** The breaker denominator is every content key listed for the project (referenced or not, any age). **Alternatives considered:** only old keys, rejected because a project of young files would trip on its first leftover. **Implemented by:** DR-006. **Evidence:** n/a.
- **(SPEC-0034) DEC-006:** The breaker outcome is a counter (`orphan_breakers_tripped`) asserted by tests, plus a warning log. **Alternatives considered:** asserting the log line, rejected because it needs a log capture dependency the workspace does not have. **Implemented by:** DR-006, DR-009. **Evidence:** `Cargo.toml` has no `tracing-test`.
- **(SPEC-0034) DEC-007:** The sweep reaches the relational store through `client.trash` (the pattern of `sweep_project_artifacts_at`, `purge.rs:268-269`) and the export table through `open_meta_db` (the pattern of `sweep_project_exports`, `purge.rs:296-304`); a client with no relational store behind it is skipped. **Alternatives considered:** new `MetaBackend` trait methods, rejected because 3 test doubles implement `MetaBackend` (`fs_ops.rs:3148`, `purge.rs:727`, `purge.rs:1368`). **Implemented by:** DR-003, DR-005. **Evidence:** cited.
### Assumptions (SPEC-0034)
- **Assumption:** one bucket per project (`crates/core/src/config.rs:945-947`, `crates/core/src/storage/mod.rs:211-216`); were two projects to share a bucket, the sweep would delete the other's blobs. Not configurable today.
- **Assumption:** a residual window of milliseconds remains between the final re check and the delete, on both backends (S3 has no conditional delete here). It is the window the existing delete at zero path already accepts (`volume.rs:170-172`, SPEC-0033 assumption).
- **Assumption:** single server process per database (BL-0016); the in memory schedule is per process.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | add `RelationalMetaStore::referenced_hashes(&self) -> Result<HashSet<String>>` (`SELECT sha256 FROM blob_refs WHERE volume_id=?1`), reuse `blob_is_referenced` (`:697`); add `export_link_exists(tx, project_id, token) -> Result<bool>` beside `delete_export_link_if_live` (`:1575`) |
| `crates/core/src/purge.rs` | add `pub(crate) async fn sweep_project_orphans_at(now: f64, stores: &StoreManager, client: &VolumeClient, project_id: &str) -> Result<OrphanSweep>` with `pub(crate) struct OrphanSweep { blobs: usize, bytes: u64, breaker_tripped: bool }`; tests DT-014..DT-021, DT-024, DT-027 in its test module |
| `crates/core/src/core/fs_ops.rs` | none expected; only if a test double must be reused (prefer a new double in `purge.rs` tests) |
### Existing patterns
- `sweep_project_artifacts_at` (`purge.rs:268-278`): reach the relational store through `client.trash`, skip when absent, log and continue on a blob delete error.
- `sweep_project_exports` (`purge.rs:290-320`): open the export table with `open_meta_db(stores.config(), stores.relational(), project_id)` and a transaction.
- `PausingBlob` (`fs_ops.rs:3886`) for pausing a backend between listing and delete (DT-017, DT-018); `RecordingBlob` (`fs_ops.rs:4069`) for a failing delete (DT-024).
- The purge test fixture used by `run_cycle_sweeps_every_project_and_aggregates_counts` (`purge.rs:1013`).

## Acceptance tests
> 100% must pass, run through `make check`. Shared fixture: one project with a local blob store in a temp dir, built like the `run_cycle` tests (`crates/core/src/purge.rs:1013`); ages set with `std::fs::File::set_modified`; `now` injected. `A` a referenced content hash, `O` an orphan content hash 25 h old, `Y` an orphan 1 h old.

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| SPEC-0034/DT-014 | behavior | DR-003, DR-004 | `purge.rs` tests | given referenced `A` (25 h), `O` (25 h), `Y` (1 h), `git:<sha>` (25 h), `export:t1` (25 h) with a row, `export:t2` (25 h) without, `.tmp-x` (25 h), `.tmp-y` (1 h), `misc:k` (25 h), plus 20 referenced blobs; after the sweep exactly `O`, `export:t2`, `.tmp-x` are gone and every other key `exists()` | no sweep |
| SPEC-0034/DT-015 | edge | DR-003 | `purge.rs` tests | an orphan aged exactly 86 400 s is kept, one aged 86 401 s is deleted | no sweep |
| SPEC-0034/DT-016 | edge | DR-003, DR-004 | `purge.rs` tests | a 64 character uppercase hex key and a 63 character lowercase hex key, both 25 h old and unreferenced, are kept | no sweep |
| SPEC-0034/DT-017 | concurrency | DR-005, DR-002 | `purge.rs` tests, a pausing double over `LocalBlobStore` (pattern `PausingBlob`) | sweep paused after listing; a file whose bytes hash to `O` is written (reference plus a `put` refreshing the time); sweep resumes; `O` exists and the file reads back its bytes | no sweep |
| SPEC-0034/DT-018 | concurrency | DR-005 | same | sweep paused after listing; only a `blob_refs` row for `O` is added; `O` is kept | no sweep |
| SPEC-0034/DT-019 | guardrail | DR-006 | `purge.rs` tests | 20 content blobs, 2 old orphans, limit max(1, 2) = 2: both deleted, `orphan_breakers_tripped == 0` | no sweep |
| SPEC-0034/DT-020 | guardrail | DR-006 | `purge.rs` tests | 20 content blobs, 3 old orphans, plus `export:t2` and `.tmp-x`: the 3 orphans kept, `orphan_breakers_tripped == 1`, the export zip and the temp file deleted | no sweep |
| SPEC-0034/DT-021 | edge | DR-006 | `purge.rs` tests | a project whose only content blob is an old orphan (limit 1): deleted; a project whose 2 content blobs are both old orphans (limit 1): both kept | no sweep |
| SPEC-0034/DT-024 | failure | DR-008 | `purge.rs` tests, a double whose second `delete` fails | 3 old orphans, no breaker: 2 deleted, 1 remains, cycle `Ok`, `orphan_blobs_swept == 2` | no sweep |
| SPEC-0034/DT-027 | edge | DR-003 | `purge.rs` tests | 10 000 referenced blobs and 5 old orphans: exactly 5 deleted | no sweep |

## Constraints
- Do not call the sweep from `run_cycle` yet (US-0039).
- Never delete through `delete` directly: always `delete_entry` (DR-001).
- Content key test is exactly 64 characters of `[0-9a-f]`; anything else, including `git:` keys, is never a candidate.
- No new dependency; `volume_id` in every `WHERE`; SQL only through `Query` and the relational handle (AGENTS.md).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, in `.implement/US-0038-red.log`.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] DEBT: The existing test suite passes **unmodified** (exceptions: none, 0):

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

- [ ] Docs updated: none.
- [ ] Self review: full (state the time and space complexity of the hot path).
- [ ] One commit `US-0038: Sweep the unreferenced blobs of one project`, body `Spec: SPEC-0034` and the DR ids.
