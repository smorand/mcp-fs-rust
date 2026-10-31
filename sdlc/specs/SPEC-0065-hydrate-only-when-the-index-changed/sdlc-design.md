# Hydrate libgit2 only when the object index changed: technical debt design

> Id: SPEC-0065
> Nature: DEBT
> Depth: L
> Depth evidence: escalated from M while writing: 3 modules touched (`crates/core/src/git/odb.rs`, `crates/core/src/git/repo.rs`, `crates/core/src/tools/git.rs`), about 6 requirements, no public contract, no persisted change.
> Status: specified
> Generated on: 2026-10-30
> Behavior change: none
> From backlog: BL-0111
> Depends on: SPEC-0061, SPEC-0062, SPEC-0063 (the import methods, read only)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Debt

### DBT-001: every git tool call re-checks every object of the repository
- **Current structure:** `hydrate` (`crates/core/src/tools/git.rs:2069-2072`) calls `BlobObjectDb::export_to_repo` (`crates/core/src/git/odb.rs:340-370`), which reads every row of the object index (`list_objects`, `git/db.rs:321-332`) and probes libgit2's object store once per row (`odb.exists`, `odb.rs:351`) to find what is missing, then copies the missing ones. It runs on each tool call: 27 production lines of `tools/git.rs` and 6 lines of `tools/git_tags.rs` call `hydrate(`.
- **Cost paid today:** O(repository objects) on every call, even when the store holds nothing libgit2 lacks. Measured (Section 2): 135 to 157 ms at 20 000 objects against 2.4 to 3.8 ms at 500; a lightweight `git.tag_create` costs 141 to 161 ms, almost all of it this.
- **Target structure:** the entry's `BlobObjectDb` remembers the object count it last synchronised with libgit2. `hydrate` compares one `COUNT(*)` with it and exports only when they differ. Imports from the entry's own repository advance the count, so the entry's own writes do not defeat the check.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/odb.rs` | add a private `HydrationState`, fields on `BlobObjectDb`, `bind_repo`, `hydrate_if_stale`; the three import methods and `import_tips` report their stored count; `delete` resets the state; 6 unit tests and 1 test wrapper in `mod tests` (starts at line 469) | 1 struct, 1 field, 2 methods, 4 touched methods, 6 tests |
| `crates/core/src/git/repo.rs` | `create_entry` binds the path of the repository it opened (after `BlobObjectDb::new`, `:206`) | 1 edit site |
| `crates/core/src/tools/git.rs` (production, before `mod tests` at line 7665) | `hydrate` (`:2069-2072`) calls `hydrate_if_stale` | 1 edit site |
| `crates/core/src/tools/git.rs` (tests module) | 2 integration tests | 2 tests |
| `.agent_docs/git.md` | document the marker and its declared limits (lines 81 and 138 describe the hydration) | 1 section |

**Callers of `hydrate(`:** `tools/git.rs` 27 production lines (recounted with `awk '/^mod tests/{exit} /hydrate\(/{c++} END{print c}' crates/core/src/tools/git.rs`), `tools/git_tags.rs` 6 lines. None changes: they all go through `hydrate`.
**Callers of `export_to_repo(`:** `tools/git.rs:2070` (inside `hydrate`, changed), `git/http/mod.rs:434` (upload-pack, NOT changed, BL-0113), `odb.rs:340` (definition) and tests (`odb.rs`, `tools/git.rs:25529`).
**Non source occurrences:** `.agent_docs/git.md:81,138,474` mention the hydration; `rg hydrate` finds nothing in `README.md`, `AGENTS.md`, Makefile or CI files.
**Contracts touched:** `BlobObjectDb` gains two `pub` methods (internal). `BlobObjectDb::new` keeps its signature, so no existing constructor call changes (`odb.rs` tests, `repo.rs:206`). No tool schema, route or stored format changes: the object index schema is not touched, so the byte identical schema golden tests (`spec_0051_*`) are not involved.
**Lifetime of the marker:** the `BlobObjectDb` lives in a `GitRepoEntry` (`git/repo.rs:35-45`, created at `:215`) with the libgit2 repository it hydrates; `purge_repo` drops the entry first (`repo.rs:151-153`), so the marker never outlives its repository. A restart starts with no marker.
**Guardrail coverage:** `rg -c 'fn e2e_' crates/core/src/tools/git.rs` = 693 tests, plus the 34 `odb.rs` unit tests (`awk` from `mod tests` counting `#[test]` and `#[tokio::test]`). **Test command:** `cargo test -p mcp-fs-core --lib git` then `cargo test --workspace`.

**Baseline** (measured 2026-10-30, debug build, local SQLite and local blob store; a temporary test, not committed: N blobs written into libgit2 and imported, so the store and libgit2 are in sync; then 5 times: time `hydrate`, time `count_objects`, time a lightweight `git.tag_create`):

| Objects | `hydrate` alone (µs) | `count_objects` (µs) | lightweight `git.tag_create` (ms) |
|---|---|---|---|
| 500 | 3437, 3833, 3206, 2776, 2425 | 185, 126, 114, 104, 60 | 5, 3, 3, 3, 5 |
| 5 000 | 20184, 19441, 20920, 20035, 24003 | 296, 259, 275, 269, 303 | 19, 22, 22, 22, 23 |
| 20 000 | 134569, 139681, 147705, 144816, 156676 | 1937, 2719, 7978, 1216, 3052 | 153, 147, 161, 156, 141 |

**Total:** 4 source files, 3 production edit sites, 1 doc file, all recounted.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)

### 4.1 Structural

#### DR-001 [EARS-U]: hydrate only when the object count changed
> The object store SHALL provide `BlobObjectDb::bind_repo(&self, path: &Path)` and `BlobObjectDb::hydrate_if_stale(&self, repo: &git2::Repository) -> Result<usize>`; `hydrate_if_stale` SHALL skip the export when the index holds the same number of objects as at the last synchronisation of the bound repository, and SHALL otherwise run `export_to_repo(repo)`.
- **Old form:** `hydrate` always calls `export_to_repo(repo)`.
- **New form (normative):**
  1. `bind_repo` records the path of the entry's own libgit2 repository, once; a second call is ignored. A `BlobObjectDb` never bound has no own repository.
  2. `hydrate_if_stale(repo)`: paths are compared after `std::fs::canonicalize` of both sides (the bound side computed once at `bind_repo` time, the other once per call); a canonicalisation failure compares as not equal. When `repo.path()` is not the bound path (or nothing is bound), it runs `export_to_repo(repo)` and returns its result without reading or writing the marker.
  3. Otherwise it reads `n = index.count_objects()`. When the marker holds `Some(n)` it returns `Ok(0)`: no row is listed, no blob store or libgit2 probe is made.
  4. Otherwise it runs `export_to_repo(repo)`, then sets the marker to `Some(n)`, the count read BEFORE the export started, so an object stored while the export ran is seen as a difference by the next call. It returns the export's result.
  5. A failing export leaves the marker unchanged.
- **Applies to:** `crates/core/src/git/odb.rs`. `export_to_repo` is not modified.
- **Casing convention:** Rust snake_case.

#### DR-002 [EARS-U]: the entry's own imports advance the marker
> WHEN `import_from_repo`, `import_object`, `import_reachable` or `import_tips` is called with the bound repository and stores `k` objects, THE object store SHALL advance a held marker by `k`.
- **New form:** after the import returns with `k` newly stored objects (the value it returns), `marker = marker.map(|c| c + k)` when `repo.path()` equals the bound path and the marker is `Some`. A repository with another path (clone's temporary repository, `tools/git.rs:3260`) never advances it. The `import_tips` fallback counts through `import_from_repo`, and `import_tips` itself does not advance a second time. The advance is conservative: if a foreign writer stored objects meanwhile, or the marker was behind, the next `count_objects` differs from the marker and the next `hydrate_if_stale` exports.
- **Applies to:** `odb.rs`, the four import methods.

#### DR-003 [EARS-U]: a delete forgets the marker
> WHEN `BlobObjectDb::delete` is called, THE object store SHALL reset the marker to `None`.
- **New form:** `delete` (`odb.rs:181`) sets the marker to `None` after deleting. `delete` removes the blob and leaves the index row (`odb.rs:181`), so the count does not change; the reset is what forces the next call to re-check. Object writes through `write` and `write_with_sha` leave the marker untouched: they store objects libgit2 may lack, so a NEW sha makes the count differ and the next call exports; re-writing a sha that already has a row changes no count and is not detected (same class as the limit (b) of DEC-006).

#### DR-004 [EARS-U]: the entry binds its repository and `hydrate` uses the check
> `create_entry` SHALL bind the path of the repository it opened, and `hydrate` SHALL call `hydrate_if_stale`.
- **Old form:** `hydrate` is `entry.objects.export_to_repo(repo).await?; Ok(())` (`tools/git.rs:2069-2072`); `create_entry` builds `BlobObjectDb::new(blobs.clone(), db.clone())` (`repo.rs:206`) and nothing else.
- **New form:** `objects.bind_repo(repo.path())` right after `create_entry` opened or initialised the repository (`repo.rs:192-198`, before the entry is built at `:215`); `hydrate` calls `entry.objects.hydrate_if_stale(repo).await?; Ok(())`.
- **Applies to:** `repo.rs`, `tools/git.rs:2069-2072`.

#### DR-005 [EARS-UB]: the upload-pack path is not changed
> The server SHALL NOT change the upload-pack export (`git/http/mod.rs:434`), which keeps calling `export_to_repo`.
- **Reason:** it is a protocol path, not a tool call (BL-0113).

### 4.2 Invariance

#### DR-006 [EARS-U]: behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object `export_to_repo` copies into libgit2 is the same object as before.

### 4.3 Compatibility
No public contract is touched: no compatibility requirement.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `BlobObjectDb::bind_repo`, `hydrate_if_stale` (new) | internal methods | no | added; `new` and `export_to_repo` unchanged | none | none |
| every git tool and the git smart HTTP protocol | tool output, wire | yes | unchanged byte for byte (DR-006) | none | none |

## 6. Decisions
- **DEC-001:** an in-memory object count as the marker. **Approaches considered.** (A, chosen) a count in memory: one `COUNT(*)` per call, no schema change, resets on restart. (B) a persisted monotonic sequence column on `git_objects`: gives "objects added since N" exactly, but changes the schema on SQLite and PostgreSQL, needs a migration for existing rows and the byte identical schema golden tests (`spec_0051_*`), for a gain the count already gives; high cost of reversal. (C) hydrate only the objects reachable from the refs a tool touches: removes the floor entirely for narrow tools but every tool would have to name its roots, 33 call sites, and a missed root is a missing object. **Rationale:** A is the smallest change that removes the floor and is fully reversible. **Implemented by:** DR-001. **Evidence:** `git/db.rs:155-165` (no sequence column), `git/db.rs:334-346` (`count_objects`).
- **DEC-002:** the marker lives in `BlobObjectDb` and is bound to one repository path. **Alternatives considered:** a field on `GitRepoEntry` (rejected: the import methods that must advance it live in `BlobObjectDb`, and `GitRepoEntry` is built by one function only); an unbound marker (rejected: `export_to_repo` is also used on fresh repositories in tests and `import_*` on clone's temporary repository, `tools/git.rs:3326`; an unbound marker would treat objects absent from the entry's libgit2 as synchronised). **Implemented by:** DR-001, DR-002. **Evidence:** `tools/git.rs:3260,3326`.
- **DEC-003:** the marker stores the count read before the export. **Alternatives considered:** the count read after (rejected: an object stored by another replica between the listing and the second read would be skipped forever). **Implemented by:** DR-001 step 4.
- **DEC-004:** import advances the marker by the count the import returns, without a re-count. **Alternatives considered:** re-read `count_objects` after each import (rejected: one more query per write, and it would hide a foreign write as synchronised). A foreign write or a stale marker shows up as a difference at the next `hydrate_if_stale`, which is the safe direction. **Implemented by:** DR-002.
- **DEC-005:** `BlobObjectDb::new` keeps its signature; the repository is bound by a separate call. **Alternatives considered:** a new constructor argument (rejected: it would edit every existing test constructor, which the success criterion forbids without a declared exception). **Implemented by:** DR-004.
- **DEC-006:** declared limits, written into `.agent_docs/git.md`. (a) The object index only grows in production: `delete` removes a blob but never an index row (`odb.rs:181`), and no code path deletes `git_objects` rows except `purge_repo`, which drops the entry first (`repo.rs:151-161`); a future change that removes rows must reset the marker. (b) The on disk libgit2 repository is a cache rebuilt at restart; wiping it while the server runs is no longer repaired by the next call, only at restart. (c) Several replicas are supported: a replica that did not store an object sees a different count and exports. **Implemented by:** DR-003, DR-004 (documentation requirement in 8.3 DT-009). **Evidence:** `odb.rs:177-181`, `repo.rs:151-153`.
- **DEC-007:** the upload-pack path stays on `export_to_repo`. **Alternatives considered:** switching it too (rejected here: a protocol path with its own tests; BL-0113 records it). **Implemented by:** DR-005.
- **DEC-008:** the measurement is evidence in the commit body, not a gating test. **Alternatives considered:** a timing assertion (rejected: debug build timings vary). **Implemented by:** n/a, no code impact.
- **Assumption:** `Repository::path()` of the same bare repository returns the same path for every handle the entry holds (`repo.rs:192-198` opens one handle that lives in `GitRepoEntry.repo`); `ASSUMED:` the path compares equal as returned, so the comparison canonicalises both sides, and a path that cannot be canonicalised compares as not equal (full export, the safe direction).

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    cargo test -p mcp-fs-core --lib git
    cargo test --workspace
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt --all -- --check

### 8.2 Compatibility tests
None (no contract).

### 8.3 New tests

Unit tests in `crates/core/src/git/odb.rs` `mod tests` (helpers `odb()` at `odb.rs:631`, `counting_odb()` at `:569`, `CountingBlobs` at `:478`; the counting wrapper counts `exists` and `get` calls; a bare repository stands for the entry's own repository and is bound with `o.bind_repo(repo.path())`):

#### DT-001: an unchanged index is not re-checked
- **Validates:** DR-001. Given a bound repository, a store holding 200 objects created with `write` that a first `hydrate_if_stale` exported, and the counting wrapper reset; When `hydrate_if_stale` is called again; Then it returns `0` after `0` blob `exists` calls and `0` blob `get` calls.
- Red before the fix because (mutation): make `hydrate_if_stale` always call `export_to_repo`; the call makes `200` `exists` calls.

#### DT-002: an object added by another writer is exported
- **Validates:** DR-001. Given the setup of DT-001 and a second `BlobObjectDb` over the same blob store and index writing one more object with `write` (another replica); When the first handle calls `hydrate_if_stale`; Then it returns `1` and the object is in the bound repository (`repo.odb().unwrap().exists(oid)`).

#### DT-003: the entry's own imports keep the fast path
- **Validates:** DR-002. Given the bound repository synchronised, one blob written into it with `repo.blob` and `import_object(&repo, oid)` returning `1`, and the counting wrapper reset; When `hydrate_if_stale` is called; Then it returns `0` after `0` blob `exists` calls. The same expectation holds after `import_reachable` of a commit (3 objects) and after `import_from_repo`.
- Red before the fix because (mutation): remove the advance; the call exports and makes more than `0` `exists` calls.

#### DT-004: another repository's imports do not advance the marker
- **Validates:** DR-002. Given the bound repository synchronised and a second bare repository (a clone's temporary one) holding one blob not in the first, imported with `import_from_repo(&other)` returning `1`; When `hydrate_if_stale(&bound)` is called; Then it returns `1` and the blob is in the bound repository.
- Red before the fix because (mutation): advance for any repository; the call returns `0` and the blob is missing.

#### DT-005: a delete forgets the marker
- **Validates:** DR-003. Given the bound repository synchronised with 200 objects, the counting wrapper reset, and `delete(sha)` of one object (the blob goes, the index row stays); When `hydrate_if_stale` is called; Then it makes `199` or more blob `exists`/`get` probes, that is the full check ran, where without the delete it makes `0` (DT-001).
- Red before the fix because (mutation): remove the reset from `delete`; the call makes `0` probes.

#### DT-006: an object stored during an export is seen by the next one
- **Validates:** DR-001 step 4, DEC-003. Given a store whose blob wrapper holds an `Arc<OnceLock<BlobObjectDb>>` filled after construction with a second `BlobObjectDb` built over the same wrapper and index, and which, on its first `get`, writes one more object through that second handle; When the first `hydrate_if_stale` runs (the export reads the listing taken before the extra object), then a second `hydrate_if_stale` runs; Then the second returns `1` and the extra object is in the repository.
- Red before the fix because (mutation): store the count read after the export; the second call returns `0`.

Integration tests in `crates/core/src/tools/git.rs` `mod tests`:

#### DT-007: a commit written by another replica is visible
- **Validates:** DR-001, DR-004, DR-006. Given `Env` with a project that has a commit, and a second `BlobObjectDb` over the same blob store and index writing a commit with `seed_commit` and a branch ref `refs/heads/other` set through `entry.db.set_ref`; When `git.log` for `other` and `git.show` of the commit run through the first entry; Then both succeed and show the commit message `seeded elsewhere`.
- Red before the fix because (mutation): make `hydrate_if_stale` return early whenever a marker exists regardless of the count; `git.log` fails with a missing object.

#### DT-008: the entry's own commit is read back consistently
- **Validates:** DR-002, DR-004, DR-006. Given `Env` with `seed_tags(&e)` then `git.commit` of `/c.txt`; When `git.log` and `git.show` of the new commit run; Then both show the commit, and a second `git.commit` and `git.log` also succeed (the marker moved with the import).

#### DT-009: structural, callers and documentation
- **Method:** `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'export_to_repo\('` prints nothing (rg exit status 1) and the same with `hydrate_if_stale\(` prints `1`; `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg -c 'export_to_repo\('` prints `1`; `rg -c 'bind_repo\(' crates/core/src/git/repo.rs` prints `1`; `rg -n 'hydrate_if_stale' .agent_docs/git.md` prints at least one line (the limits of DEC-006 documented).

#### DT-010: measurement, not gating
- **Method:** the temporary test of Section 2 (write N blobs, import, time `hydrate` and a lightweight `git.tag_create` 5 times), run before and after for N in 500, 5 000, 20 000; not committed. After the change `hydrate` at 20 000 objects was under 10 ms and the lightweight tag under 30 ms in each of 5 samples (recorded evidence, enforced by no test); both tables go in the commit body.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| none | none | none | none |

### 8.5 Security tests
Not applicable.

## 9. Implementation order
1. Add `HydrationState`, `bind_repo`, `hydrate_if_stale`, the advance in the four import methods and the reset in `delete`, with DT-001 to DT-006 in `odb.rs` (green; nothing calls `hydrate_if_stale` yet). Mutation proofs of DT-001, DT-003, DT-004, DT-006.
2. Bind the repository in `create_entry`, switch `hydrate`, add DT-007 and DT-008; mutation proof of DT-007; run the full suite.
3. Document the marker and its limits in `.agent_docs/git.md`, run DT-009 and DT-010, record the numbers.
Each step leaves the suite green: step 1 changes no caller, step 2 is the behavior bearing switch.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 (MINOR) | 4 (MINOR) | IMPLEMENTABLE |

**Amendments applied:** DR-001 step 2 spells the path comparison (canonicalised both sides); DR-003 corrected (`delete` removes the blob, not the row; re-writing an existing sha is not detected); DEC-006(a) corrected; DT-005 rebuilt around probe counts with its own mutation; DT-006 says how the hook wrapper is built (`Arc<OnceLock<BlobObjectDb>>`); line citations corrected (`tools/git.rs` `mod tests` at 7665, `CountingBlobs` at `odb.rs:478`, `counting_odb` 569, `odb()` 631). Self consistency re-read after the amendments: ids present, every requirement tested (DR-001: DT-001, 002, 006; DR-002: DT-003, 004; DR-003: DT-005; DR-004: DT-007, 008, 009; DR-005: DT-009; DR-006: DT-007, 008), every DEC names its DR.
