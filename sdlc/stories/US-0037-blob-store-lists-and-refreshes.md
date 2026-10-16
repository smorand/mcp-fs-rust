# US-0037: The blob store lists its objects and refreshes re stored blobs

> Spec: SPEC-0034 (sdlc/specs/SPEC-0034-sweep-unreferenced-blobs/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A sweep cannot collect what the blob store cannot enumerate. Give `BlobBackend` a `list` (and `delete_entry`, so a local temp file can be removed although its name does not map to its directory), implement both on the local and S3 stores, and make a local `put` of existing bytes refresh their modified time so an age is trustworthy. Nothing calls `list` yet.

## Requirements covered
#### SPEC-0034/DR-001 [EARS-U]: The blob store can list its objects
> `BlobBackend` SHALL provide `list`, returning every object of the bucket as a `BlobEntry` with its key, its size in bytes and its last modified time; `LocalBlobStore` and `S3BlobStore` SHALL implement it, and the default implementation SHALL return an `ERR_NOT_SUPPORTED` error.
- **Local:** every file under every shard directory of the bucket root, `.tmp-*` files included, the key being the file name and `shard` the directory it was found in.
- `BlobBackend` SHALL also provide `delete_entry(&BlobEntry)`, defaulting to `delete(&entry.key)`; `LocalBlobStore` SHALL remove exactly the listed file (`{root}/{shard}/{key}`), so a temp file is deletable although its name does not map to its directory.
- The sweep SHALL delete every object through `delete_entry`.
- **S3:** every key across all `list_objects_v2` pages.

#### SPEC-0034/DR-002 [EARS-E]: Re storing existing bytes refreshes their age
> WHEN `LocalBlobStore::put` finds the blob already present THE store SHALL set its modified time to now and leave its bytes unchanged.

#### SPEC-0034/DR-010 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.


## Independent Test
`cargo test -p mcp-fs-core --lib storage::blob` passes with DT-010 and DT-013 (both red on `main`: `list` absent, re `put` keeps the 48 h old time); `cargo test -p mcp-fs-core --lib core::fs_ops::tests` passes with DT-011.

## Design excerpt
### Decisions
- **(SPEC-0034) DEC-002:** `list` is a trait method with a default `ERR_NOT_SUPPORTED` body. **Alternatives considered:** a required method, rejected because the two test doubles (`fs_ops.rs:3895`, `:4069`) would have to change; a separate listing trait, rejected as a second abstraction for one call. **Implemented by:** DR-001, DR-008. **Evidence:** `crates/core/src/storage/traits.rs:231-238`.
- **(SPEC-0034) DEC-008:** Tests backdate files with the standard library (`std::fs::File::set_modified`). **Alternatives considered:** the `filetime` crate, rejected as a new dependency. **Implemented by:** DR-002, DR-003 tests. **Evidence:** n/a.
### Assumptions (SPEC-0034)
- **Assumption:** one bucket per project (`crates/core/src/config.rs:945-947`, `crates/core/src/storage/mod.rs:211-216`); were two projects to share a bucket, the sweep would delete the other's blobs. Not configurable today.
- **Assumption:** a residual window of milliseconds remains between the final re check and the delete, on both backends (S3 has no conditional delete here). It is the window the existing delete at zero path already accepts (`volume.rs:170-172`, SPEC-0033 assumption).
- **Assumption:** single server process per database (BL-0016); the in memory schedule is per process.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/traits.rs` | `BlobBackend` (`:231-238`): add `list` (default `ERR_NOT_SUPPORTED`) and `delete_entry` (default `delete(&entry.key)`); add `BlobEntry { key: String, size: u64, modified: f64, shard: Option<String> }` |
| `crates/core/src/storage/blob/local.rs` | `list` over every shard directory (`path_for` `:29-35`), `.tmp-*` included with `shard`; override `delete_entry` to remove `{root}/{shard}/{key}`; `put` early return (`:44-46`) refreshes the modified time first |
| `crates/core/src/storage/blob/s3.rs` | `list` paging `list_objects_v2` (pattern of `remove_bucket`, `:137-166`) |
| `crates/core/src/core/fs_ops.rs` (test module) | DT-011 beside `PausingBlob` (`:3895`), no change to the double itself |
### Existing patterns
- `LocalBlobStore::remove_bucket` and its tests (`local.rs:105`, tests `:124-227`, including `on_disk_layout_is_sharded_by_first_two_chars`, `no_temp_files_left_behind`).
- `S3BlobStore::remove_bucket` paging loop (`s3.rs:137-166`); the `#[ignore]` MinIO test `integration_put_get_range_delete` (`s3.rs:194-212`).
- `ERR_NOT_SUPPORTED` constructor in `crates/core/src/errors.rs` (`ToolError::not_supported`).

## Acceptance tests
> 100% must pass, run through `make check`. Shared fixture: one project with a local blob store in a temp dir, built like the `run_cycle` tests (`crates/core/src/purge.rs:1013`); ages set with `std::fs::File::set_modified`; `now` injected. `A` a referenced content hash, `O` an orphan content hash 25 h old, `Y` an orphan 1 h old.

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| SPEC-0034/DT-010 | structural | DR-001 | `blob/local.rs` tests | Given an empty store, `list()` returns `[]`; given puts of 3 and 5 bytes plus a file `.tmp-ab-x` written by hand in shard `ab`, `list()` returns 3 entries of sizes {3, 5, len}, `modified` within 2 s of now, the temp entry with `shard == Some("ab")`; `delete_entry` on it removes `{root}/ab/.tmp-ab-x` | `list` does not exist |
| SPEC-0034/DT-011 | structural | DR-001 | `fs_ops.rs` tests, `PausingBlob` | a double not overriding `list` returns an error with code `ERR_NOT_SUPPORTED` | no default method |
| SPEC-0034/DT-012 | edge | DR-001 | `blob/s3.rs` `#[ignore]` MinIO suite (beside `s3.rs:194-212`) | 1001 keys put, `list()` returns 1001 entries with correct sizes (pagination followed) | no `list` |
| SPEC-0034/DT-013 | behavior | DR-002 | `blob/local.rs` tests | blob `X` backdated 48 h, `put(X)` again, its modified time is within 2 s of now and its bytes unchanged | early return at `local.rs:44-46` keeps the old time |

## Constraints
- Do not call `list` from production code (US-0038 does).
- Do not change the on disk layout (`AGENTS.md:184`).
- No new dependency: backdate with `std::fs::File::set_modified` (DEC-008).
- Do not modify `PausingBlob` or `RecordingBlob`: the default methods keep them compiling.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, in `.implement/US-0037-red.log`.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] DEBT: The existing test suite passes **unmodified** (exceptions: none, 0):

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

- [ ] Docs updated: none.
- [ ] Self review: full (state the time and space complexity of the hot path).
- [ ] One commit `US-0037: The blob store lists its objects and refreshes re stored blobs`, body `Spec: SPEC-0034` and the DR ids.
