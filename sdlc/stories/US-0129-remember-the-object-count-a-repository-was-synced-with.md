# US-0129: remember the object count a libgit2 repository was synchronised with

> Spec: SPEC-0065 (sdlc/specs/SPEC-0065-hydrate-only-when-the-index-changed/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0128
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Add the marker and the check without calling them from anywhere: a bound repository path, a stored object count, `hydrate_if_stale`, the advance by the entry's own imports and the reset by `delete`. Six unit tests prove each rule.

## Requirements covered
- SPEC-0065/DR-001 [EARS-U]: The object store SHALL provide `BlobObjectDb::bind_repo(&self, path: &Path)` and `BlobObjectDb::hydrate_if_stale(&self, repo: &git2::Repository) -> Result<usize>`; `hydrate_if_stale` SHALL skip the export when the index holds the same number of objects as at the last synchronisation of the bound repository, and SHALL otherwise run `export_to_repo(repo)`.
  - **Old form:** `hydrate` always calls `export_to_repo(repo)`.
  - **New form (normative):**
    1. `bind_repo` records the path of the entry's own libgit2 repository, once; a second call is ignored. A `BlobObjectDb` never bound has no own repository.
    2. `hydrate_if_stale(repo)`: paths are compared after `std::fs::canonicalize` of both sides (the bound side computed once at `bind_repo` time, the other once per call); a canonicalisation failure compares as not equal. When `repo.path()` is not the bound path (or nothing is bound), it runs `export_to_repo(repo)` and returns its result without reading or writing the marker.
    3. Otherwise it reads `n = index.count_objects()`. When the marker holds `Some(n)` it returns `Ok(0)`: no row is listed, no blob store or libgit2 probe is made.
    4. Otherwise it runs `export_to_repo(repo)`, then sets the marker to `Some(n)`, the count read BEFORE the export started, so an object stored while the export ran is seen as a difference by the next call. It returns the export's result.
    5. A failing export leaves the marker unchanged.
  - **Applies to:** `crates/core/src/git/odb.rs`. `export_to_repo` is not modified.
  - **Casing convention:** Rust snake_case.
- SPEC-0065/DR-002 [EARS-U]: WHEN `import_from_repo`, `import_object`, `import_reachable` or `import_tips` is called with the bound repository and stores `k` objects, THE object store SHALL advance a held marker by `k`.
  - **New form:** after the import returns with `k` newly stored objects (the value it returns), `marker = marker.map(|c| c + k)` when `repo.path()` equals the bound path and the marker is `Some`. A repository with another path (clone's temporary repository, `tools/git.rs:3260`) never advances it. The `import_tips` fallback counts through `import_from_repo`, and `import_tips` itself does not advance a second time. The advance is conservative: if a foreign writer stored objects meanwhile, or the marker was behind, the next `count_objects` differs from the marker and the next `hydrate_if_stale` exports.
  - **Applies to:** `odb.rs`, the four import methods.
- SPEC-0065/DR-003 [EARS-U]: WHEN `BlobObjectDb::delete` is called, THE object store SHALL reset the marker to `None`.
  - **New form:** `delete` (`odb.rs:181`) sets the marker to `None` after deleting. `delete` removes the blob and leaves the index row (`odb.rs:181`), so the count does not change; the reset is what forces the next call to re-check. Object writes through `write` and `write_with_sha` leave the marker untouched: they store objects libgit2 may lack, so a NEW sha makes the count differ and the next call exports; re-writing a sha that already has a row changes no count and is not detected (same class as the limit (b) of DEC-006).

## Independent Test
`cargo test -p mcp-fs-core --lib git::odb` runs DT-001 to DT-006 green. The non mock assertions are the counted blob probes of DT-001 and DT-003 and the objects really present in the bound repository (DT-002, DT-004, DT-006). Then apply the mutations of the tests, see each fail, and restore your real implementation (keep a copy first).

## Design excerpt
### Decisions
- DEC-001 (SPEC-0065): an in-memory object count as the marker (no persisted sequence column, no migration).
- DEC-002: the marker lives in `BlobObjectDb` and is bound to one repository path, so imports from another repository (clone's temporary repo) never advance it.
- DEC-003: the stored count is the one read BEFORE the export.
- DEC-004: an own import advances the marker by the count it returns, with no re-count; a foreign write or a stale marker shows up as a difference, the safe direction.
- DEC-005: `BlobObjectDb::new` keeps its signature; the repository is bound by a separate call.
### Data model
None. The object index schema is not touched; the marker is process memory only.
### Contracts
`pub fn bind_repo(&self, path: &std::path::Path)` and `pub async fn hydrate_if_stale(&self, repo: &git2::Repository) -> Result<usize>` on `BlobObjectDb` (internal). `BlobObjectDb::new(blobs, index)` and `export_to_repo` are unchanged.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/odb.rs` | modify (state, `bind_repo`, `hydrate_if_stale`, advance in `import_from_repo`, `import_object`, `import_reachable` (and so `import_tips`, which delegates), reset in `delete`; 6 tests and 1 hook wrapper in `mod tests`, which starts at line 469) |
### Existing patterns
Shape (adapt names, keep the rules):
```rust
struct HydrationState { own: Option<PathBuf>, synced: Option<i64> }   // own is canonicalised
pub struct BlobObjectDb { blobs, index, hydration: std::sync::Mutex<HydrationState> }   // new() starts with own None, synced None
pub fn bind_repo(&self, path: &Path) { let mut s = self.hydration.lock().unwrap(); if s.own.is_none() { s.own = std::fs::canonicalize(path).ok(); } }
pub async fn hydrate_if_stale(&self, repo: &git2::Repository) -> Result<usize> {
    if !self.is_own(repo) { return self.export_to_repo(repo).await; }     // another repository: full export, marker untouched
    let n = self.index.count_objects().await?;                              // read BEFORE exporting
    if self.hydration.lock().unwrap().synced == Some(n) { return Ok(0); }
    let written = self.export_to_repo(repo).await?;                         // an error leaves the marker unchanged
    self.hydration.lock().unwrap().synced = Some(n);
    Ok(written)
}
fn advance(&self, repo: &git2::Repository, stored: usize) { /* if is_own(repo) and synced is Some(c): synced = Some(c + stored as i64) */ }
```
Never hold the std `Mutex` across an `.await`: take it, read or write, drop it. `is_own` canonicalises `repo.path()` and compares with `own`; a canonicalisation failure is `false`. Call `advance` at the end of `import_from_repo`, `import_object` and `import_reachable` with the count they return (`import_tips` delegates, so it must not advance a second time); `delete` sets `synced = None` after deleting. Test helpers in `mod tests`: `odb()` (`:631`), `counting_odb()` (`:569`), `CountingBlobs` (`:478`, counts `exists`; extend it or add a sibling that also counts `get`), `bare()`, `commit_on`, `flat_tree` (see the SPEC-0062 and SPEC-0063 tests `dt001_*`, `us0123_*`). A second `BlobObjectDb` over the same store: `BlobObjectDb::new(o.blobs().clone(), o.index().clone())`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0065/DT-001: an unchanged index is not re-checked
- **Validates:** DR-001. Given a bound repository, a store holding 200 objects created with `write` that a first `hydrate_if_stale` exported, and the counting wrapper reset; When `hydrate_if_stale` is called again; Then it returns `0` after `0` blob `exists` calls and `0` blob `get` calls.
- Red before the fix because (mutation): make `hydrate_if_stale` always call `export_to_repo`; the call makes `200` `exists` calls.

### SPEC-0065/DT-002: an object added by another writer is exported
- **Validates:** DR-001. Given the setup of DT-001 and a second `BlobObjectDb` over the same blob store and index writing one more object with `write` (another replica); When the first handle calls `hydrate_if_stale`; Then it returns `1` and the object is in the bound repository (`repo.odb().unwrap().exists(oid)`).

### SPEC-0065/DT-003: the entry's own imports keep the fast path
- **Validates:** DR-002. Given the bound repository synchronised, one blob written into it with `repo.blob` and `import_object(&repo, oid)` returning `1`, and the counting wrapper reset; When `hydrate_if_stale` is called; Then it returns `0` after `0` blob `exists` calls. The same expectation holds after `import_reachable` of a commit (3 objects) and after `import_from_repo`.
- Red before the fix because (mutation): remove the advance; the call exports and makes more than `0` `exists` calls.

### SPEC-0065/DT-004: another repository's imports do not advance the marker
- **Validates:** DR-002. Given the bound repository synchronised and a second bare repository (a clone's temporary one) holding one blob not in the first, imported with `import_from_repo(&other)` returning `1`; When `hydrate_if_stale(&bound)` is called; Then it returns `1` and the blob is in the bound repository.
- Red before the fix because (mutation): advance for any repository; the call returns `0` and the blob is missing.

### SPEC-0065/DT-005: a delete forgets the marker
- **Validates:** DR-003. Given the bound repository synchronised with 200 objects, the counting wrapper reset, and `delete(sha)` of one object (the blob goes, the index row stays); When `hydrate_if_stale` is called; Then it makes `199` or more blob `exists`/`get` probes, that is the full check ran, where without the delete it makes `0` (DT-001).
- Red before the fix because (mutation): remove the reset from `delete`; the call makes `0` probes.

### SPEC-0065/DT-006: an object stored during an export is seen by the next one
- **Validates:** DR-001 step 4, DEC-003. Given a store whose blob wrapper holds an `Arc<OnceLock<BlobObjectDb>>` filled after construction with a second `BlobObjectDb` built over the same wrapper and index, and which, on its first `get`, writes one more object through that second handle; When the first `hydrate_if_stale` runs (the export reads the listing taken before the extra object), then a second `hydrate_if_stale` runs; Then the second returns `1` and the extra object is in the repository.
- Red before the fix because (mutation): store the count read after the export; the second call returns `0`.

Integration tests in `crates/core/src/tools/git.rs` `mod tests`:

## Constraints
- Files not to touch: every file other than `crates/core/src/git/odb.rs`; `export_to_repo` and `BlobObjectDb::new` must not change.
- Dependencies not to add: none.
- Patterns to avoid: a lock held across an await; re-counting after an import; an unbound marker.
- Scope boundary: binding the repository and switching `hydrate` is US-0130.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a stub, or the mutation named for it where the behavior pre exists); mutations reverted with `git checkout -- <file>`.
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-006): every object `export_to_repo` copies into libgit2 is the same object as before.
- [ ] Docs: none in this story (US-0131 documents the marker)
- [ ] Self review: time and space of the critical path stated (one `COUNT(*)` and no per row work on the fast path; the full export, unchanged, otherwise), no lock held across an await, no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Mutations reverted: `git diff main -- crates/core/src/git/odb.rs` shows only the real implementation and tests.
- [ ] One commit `US-0129: remember the object count a libgit2 repository was synchronised with`, body `Spec: SPEC-0065` and the requirement ids.
