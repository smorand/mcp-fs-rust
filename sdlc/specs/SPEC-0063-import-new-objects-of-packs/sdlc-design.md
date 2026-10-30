# Import only the new objects of a push, a fetch and a pull: technical debt design

> Id: SPEC-0063
> Nature: DEBT
> Depth: L
> Depth evidence: escalated from M while writing: 3 modules touched (`crates/core/src/git/odb.rs`, `crates/core/src/tools/git.rs`, `crates/core/src/git/http/mod.rs`), 3 production call sites changed, no public contract, no persisted change; 5 requirements.
> Status: specified
> Generated on: 2026-10-30
> Behavior change: none
> From backlog: BL-0112
> Depends on: SPEC-0062 (`import_reachable`, read only), SPEC-0052 (tag reconcile, read only)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Debt

### DBT-001: three pack sites import the whole repository
- **Current structure:** after libgit2 received a pack, each of three sites calls `entry.objects.import_from_repo(&repo)`, which enumerates every object of the repository, reads each one into memory and probes the blob store for each (`crates/core/src/git/odb.rs:192-232`). The sites: push receive `receive_pack_locked` (`crates/core/src/git/http/mod.rs:572`, after `index_pack`, only when the pack is larger than 12 bytes, `:569`); `fetch_branch_inner` (`crates/core/src/tools/git.rs:3920`); the fetch step of `pull_branch_inner` (`crates/core/src/tools/git.rs:6805`).
- **Cost paid today:** each call is O(repository objects) for a pack that brings a handful. Measured on a fetch of one new remote commit (Section 2): 276 to 288 ms at 5 000 objects and 1 132 to 1 184 ms at 20 000, against 31 to 35 ms at 500, the same 55 microseconds per object as the commit path SPEC-0062 fixed.
- **Target structure:** `BlobObjectDb::import_tips(repo, tips)` imports what the new tips reach, and each site passes the tips its pack brought: the non delete `new_sha` of the pushed ref updates (`http/mod.rs:514-518`), and for fetch and pull the `new_sha` of `FetchOutcome.refs_updated` plus the `oid` of `FetchOutcome.incoming_tags` (`git/remote.rs:1168-1202`).
- **Observable behavior:** unchanged.

### DBT-002 (kept on purpose): clone
- Clone (`tools/git.rs:3326`) reads a fresh temporary repository (`TempClone`, `:3260`, `clone_to_temp` `remote.rs:408-436`), so the whole walk already equals the closure of its refs and nothing is gained. A shallow clone (`remote.rs:419`, `depth > 0`) may leave a boundary commit's parents out of the odb, which `import_reachable` would refuse. It stays on `import_from_repo`.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/odb.rs` | add `BlobObjectDb::import_tips` (after `import_reachable`, `:289`); 4 unit tests and 1 test wrapper in `mod tests` (starts at line 455) | 1 new method, 4 tests, 1 helper |
| `crates/core/src/tools/git.rs` (production, before `mod tests` at line 7657) | `fetch_branch_inner` call `:3920`; `pull_branch_inner` call `:6805`; one private helper `fetch_tips(outcome)` | 2 edit sites, 1 helper |
| `crates/core/src/tools/git.rs` (tests module) | 3 integration tests | 3 tests |
| `crates/core/src/git/http/mod.rs` (production, before `mod tests` at line 663) | `receive_pack_locked` call `:572` | 1 edit site |
| `crates/core/src/git/http/mod.rs` (tests module) | 3 integration tests (DT-008, DT-009, DT-012) | 3 tests |

**Callers of `import_from_repo` after this change (production):** `tools/git.rs:3326` (clone) and, inside `import_tips`, the fallback. The other lines of `rg -n 'import_from_repo\(' crates/core/src --glob '*.rs'` are the definition (`odb.rs:192`), tests (`odb.rs:749,792`, `tools/git.rs` `e2e_*` tests) and the three call sites this lot changes.
**Production callers of `import_reachable` today:** 7 sites in `tools/git.rs` (`:2325,4264,4893,4930,5634,6937,7180`).
**Non source occurrences:** none (`rg import_from_repo` outside `crates` and `sdlc` returns nothing); `sdlc/specs/SPEC-0007-git/design.md` and `SPEC-0062-import-reachable-objects/sdlc-design.md` mention the import and are not modified.
**Contracts touched:** `BlobObjectDb` gains one `pub` method (internal). The smart HTTP protocol (`git push` over `/git/{mount_id}/`) and the tools `git.remote_fetch`, `git.remote_pull` keep their schemas and outputs.
**Guardrail coverage:** `rg -c 'fn e2e_' crates/core/src/tools/git.rs` = 690 tests in the git tool file, plus the `http/mod.rs` push tests `receive_pack_creates_a_ref_and_reports_ok` (`:965`), `receive_pack_rejects_unknown_objects_and_non_fast_forward` (`:985`, it sends no pack, so it never enters the `pack.len() > 12` gate), `receive_pack_deletes_a_ref_on_zero_new_sha` (`:1020`) and `receive_pack_indexes_a_real_packfile_into_the_blob_store` (`:1037`, the guardrail of the push import). `a_push_then_a_fetch_round_trips_the_same_objects` (`:1368`) covers upload-pack only. **Test command:** `cargo test -p mcp-fs-core --lib git` then `cargo test --workspace`.

**Baseline** (measured 2026-10-30, debug build, local SQLite and local blob store; a temporary test, not committed: a cloned remote, N blobs written into libgit2 and imported with `import_from_repo`, then 5 times: one new remote commit with `advance_bare_remote`, timed `call_fetch_branch`; milliseconds). The pull's fetch step runs the same fetch and import, so it shares this baseline. Push receive was not measured separately: it calls the same `import_from_repo` (`ASSUMED:` the same per object cost).

| Objects in the repository | fetch |
|---|---|
| 500 | 34, 32, 31, 35, 34 |
| 5 000 | 285, 279, 288, 276, 281 |
| 20 000 | 1184, 1132, 1142, 1163, 1164 |

**Total:** 3 files, 3 production edit sites, 1 new method, 1 helper, 10 new tests, all recounted.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)

### 4.1 Structural

#### DR-001 [EARS-U]: import by tips, with the whole walk as fallback
> The object store SHALL provide `BlobObjectDb::import_tips(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize>` that calls `import_reachable(repo, tips)` and, only when that fails with the code `ERR_NOT_FOUND`, calls `import_from_repo(repo)` and returns its result.
- **Old form:** only `import_from_repo(&self, repo)` (every object), `import_object` (one) and `import_reachable` (the closure of tips, strict).
- **New form:** an error other than `ERR_NOT_FOUND` from `import_reachable` (a failed blob write, an index error) is returned as is and triggers no fallback. Empty `tips` returns `Ok(0)` with no store probe and no fallback. `import_reachable` stores nothing when it fails (all reads precede the first write, SPEC-0062 DR-001 step 7), so the fallback starts from the state the call started in.
- **Applies to:** `crates/core/src/git/odb.rs`. `import_from_repo`, `import_object` and `import_reachable` are not modified.
- **Casing convention:** Rust snake_case.

#### DR-002 [EARS-U]: the three pack sites import their tips
> Push receive, fetch and the fetch step of pull SHALL call `import_tips` with the tips their pack brought, where they called `import_from_repo`.
- **Old form:** `entry.objects.import_from_repo(&repo).await?;` (push receive, `http/mod.rs:572`); `entry_for_thread.objects.import_from_repo(&repo).await?;` (`tools/git.rs:3920` and `:6805`).
- **New form, per site:**

| Site | Line | New call |
|---|---|---|
| `receive_pack_locked` | `http/mod.rs:572` | `entry.objects.import_tips(&repo, &push_tips(updates)).await?;` where `push_tips` collects `Oid::from_str(&u.new_sha)` of each update whose `new_sha` is not `ZERO_ID` and parses, in order |
| `fetch_branch_inner` | `tools/git.rs:3920` | `entry_for_thread.objects.import_tips(&repo, &fetch_tips(&outcome)).await?;` |
| `pull_branch_inner` fetch step | `tools/git.rs:6805` | `entry_for_thread.objects.import_tips(&repo, &fetch_tips(&outcome)).await?;` |

- `fetch_tips(outcome: &FetchOutcome) -> Vec<Oid>` returns, in this order, the parsed `new_sha` of every `outcome.refs_updated` entry, then the `oid` of every `outcome.incoming_tags` entry; an entry whose `new_sha` does not parse as an object id is skipped.
- The ordering rule of each site is unchanged: the objects are in the blob store before the ref rows or tag rows name them; in particular `reconcile_incoming_tags` still runs after the import (`tools/git.rs:3921-3929`, `:6806-6815`).
- The push site keeps its gate: the import runs only when `pack.len() > 12` (`http/mod.rs:569`).

#### DR-003 [EARS-UB]: clone stays on the whole walk
> The server SHALL NOT change the clone import (`tools/git.rs:3326`), which keeps calling `import_from_repo(&cloned)`.
- **Reason:** DBT-002.

### 4.2 Invariance

#### DR-004 [EARS-U]: behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

#### DR-005 [EARS-U]: a malformed push is answered as before
> WHEN a push names a tip that libgit2 does not hold, or whose closure is incomplete, THE server SHALL answer exactly as before this change: through the fallback of DR-001 the whole repository is imported and the per ref refusals of `process_ref_update` (`http/mod.rs:630-654`) are produced from the same state.

### 4.3 Compatibility
No public contract is touched: no compatibility requirement.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `BlobObjectDb::import_tips` (new) | internal method | no | added; the three other import methods kept | none | none |
| Git smart HTTP push receive, `git.remote_fetch`, `git.remote_pull` results | wire protocol and tool output | yes | unchanged byte for byte (DR-004, DR-005) | none | none |

## 6. Decisions
- **DEC-001:** a thin wrapper `import_tips` (strict import, whole walk on `ERR_NOT_FOUND` only). **Alternatives considered:** call `import_reachable` strictly at the three sites (rejected: a client controlled push naming an unknown tip together with a pack would fail the whole request, while `process_ref_update` answers `ng <ref> object not found` per ref, `http/mod.rs:635-637`; the existing test at `:985` sends no pack, so DR-005 is covered by DT-002 and DT-012); filter the push tips to those libgit2 holds before importing (rejected: it keeps the incomplete closure case, a stored commit with a missing parent, which the strict walk rejects and the old walk accepted). **Rationale:** the fallback makes every input the old code accepted behave as before, and costs nothing on the normal path. **Implemented by:** DR-001, DR-005. **Evidence:** `http/mod.rs:572,630-654,985`, SPEC-0062 DR-001 step 7.
- **DEC-002:** the tips of a fetch are the updated refs' new ids plus the incoming tag oids, not every ref of the repository. **Alternatives considered:** all `references()` of `repo` (rejected: one store probe per ref, so a project with 20 000 tags pays 20 000 probes per fetch, an O(tags) cost the change set out to remove); only `refs_updated` (rejected: `reconcile_incoming_tags` calls `set_ref` for each incoming tag, so every `tag.oid` must be in the store first, `git_tags.rs:428-459`, DEC-012 of SPEC-0052). **Rationale:** everything else the fetch left in libgit2 already came from the store (`hydrate`, `tools/git.rs:3913`). **Implemented by:** DR-002. **Evidence:** `git/remote.rs:1168-1202`, `tools/git_tags.rs:428-459`.
- **DEC-003:** the push tips are the `new_sha` of the non delete commands. **Alternatives considered:** every object of the pack (rejected: the pack carries no index of its own and the closure walk finds the same objects). **Implemented by:** DR-002. **Evidence:** `http/mod.rs:514-518,523-558`.
- **DEC-004:** a repair of the whole walk is dropped on purpose at the three sites: objects a pack carried that no new tip reaches are no longer imported, and a missing index row for an existing blob is not recorded for objects the walk stops at. The fallback and the clone keep the old behavior. **Alternatives considered:** probe the index for each stopped object (rejected: doubles the probes). **Rationale:** the libgit2 repository is a cache rebuilt from the store (`hydrate`, `tools/git.rs:2069`); an unreferenced object has no ref to lose. **Implemented by:** DR-002.
- **DEC-005:** clone is not changed. **Alternatives considered:** pass every tag and the HEAD commit (rejected: no gain on a fresh repository, and the shallow case, DBT-002). **Implemented by:** DR-003.
- **DEC-006:** one private helper `fetch_tips` shared by fetch and pull, one local `push_tips` in the HTTP module. **Alternatives considered:** inline the collection at each site (rejected: two copies of the same rule). **Implemented by:** DR-002.
- **DEC-007:** the measurement is evidence in the commit body, not a gating test. **Alternatives considered:** a timing assertion (rejected: debug build timings vary). **Implemented by:** n/a, no code impact.
- **Assumption:** `FetchRefUpdate.new_sha` is a hex object id (`remote.rs:1168-1172`); a value that does not parse is skipped, which leaves the old walk's behavior unreachable only for that ref, and `refs_updated` values come from libgit2 reference targets, so none is expected.

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

Unit tests in `crates/core/src/git/odb.rs` `mod tests` (helpers `odb()` and `counting_odb()` from SPEC-0062; one new wrapper `FailingPutBlobs` over `LocalBlobStore` whose `put` returns `ToolError::internal("disk full")` and counts attempts):

#### DT-001: one new commit among strays
- **Validates:** DR-001. Given a bare repository with 2 000 unimported blobs and one commit (one blob, its tree, the commit); When `import_tips(&repo, &[commit])`; Then it returns `3` and `o.list().await.unwrap().len()` equals `3`.
- Red before the fix because (mutation): make `import_tips` call `import_from_repo` only; the length is `2003`.

#### DT-002: an unreadable tip falls back to the whole walk
- **Validates:** DR-001, DR-005. Given the repository of DT-001 and the tip list `[Oid::from_str("1111111111111111111111111111111111111111")]`; When `import_tips`; Then it returns `Ok(2003)` (every object `import_from_repo` stores) and the store holds all 2 003.
- Red before the fix because (mutation): remove the fallback; the call fails with `ERR_NOT_FOUND`.

#### DT-003: a write failure is not retried by the whole walk
- **Validates:** DR-001. Given a `FailingPutBlobs` store and the repository of DT-001; When `import_tips(&repo, &[commit])`; Then it fails with the code and message of the injected error (`disk full`) and exactly `1` `put` was attempted (the blob, first in post order).
- Red before the fix because (mutation): fall back on any error; more than `1` `put` is attempted.

#### DT-004: empty tips
- **Validates:** DR-001. Given the counting wrapper and a repository with objects; When `import_tips(&repo, &[])`; Then it returns `0` after `0` `exists` calls and the store is empty.

Integration tests in `crates/core/src/tools/git.rs` `mod tests` (helpers `plant_strays`, `assert_closure_in_store`, `cloned_remote`, `advance_bare_remote`, `annotated_tag_bare_remote`, `call_fetch_branch`, `seed_pull_disjoint`, `call_pull_branch`):

#### DT-005: fetch of one new commit among strays
- **Validates:** DR-002, DR-004. Given a project cloned from a bare remote (`cloned_remote`), 2 000 unimported strays planted (`plant_strays`) and `before` the store length; When the remote gains a commit (`advance_bare_remote`) and `git.remote_fetch` succeeds; Then the store length is below `before + 200` and `assert_closure_in_store` holds for the remote's new tip.
- Red before the fix because (mutation): restore `import_from_repo` at `tools/git.rs:3920`; the length is above `before + 2000`.

#### DT-006: fetch with an annotated tag
- **Validates:** DR-002. Given the setup of DT-005 and an annotated tag `v1.0` (message `m`) on the new commit (`annotated_tag_bare_remote`); When `git.remote_fetch` succeeds; Then `tags_added` lists `v1.0`, `entry.objects.exists` holds for the tag object, the length is below `before + 200`.
- Red before the fix because (mutation): leave the incoming tag oids out of `fetch_tips`; the fetch fails or the tag object is missing from the store.

#### DT-007: pull with a diverged branch
- **Validates:** DR-002, DR-004. Given `seed_pull_disjoint` and 2 000 strays planted before the call; When `git.remote_pull` merges; Then `assert_closure_in_store` holds for the merge commit and the store length is below `before + 200`.
- Red before the fix because (mutation): restore `import_from_repo` at `tools/git.rs:6805`; the length is above `before + 2000`.

Integration tests in `crates/core/src/git/http/mod.rs` `mod tests` (pattern `receive_pack_indexes_a_real_packfile_into_the_blob_store`, `:1037`):

#### DT-008: push of a real pack among strays
- **Validates:** DR-002, DR-004. Given the project repository holding 2 000 unimported strays (`entry.repo`, `repo.blob`) and a pack built like `:1037` (blob, tree, commit); When the push to `refs/heads/main` is received; Then the report says `ok refs/heads/main`, the commit, its tree and its blob are in the store, and the store length grew by less than `200`.
- Red before the fix because (mutation): restore `import_from_repo` at `http/mod.rs:572`; the length grows by more than `2000`.

#### DT-009: push of an annotated tag object
- **Validates:** DR-002. Given a pack holding a commit and an annotated tag object `v1` on it, and the command `refs/tags/v1` at the tag object id; When the push is received; Then the report says `ok refs/tags/v1` and the tag object, the commit, its tree and its blob are stored.

#### DT-012: a push naming a ghost tip together with a pack
- **Validates:** DR-005, DR-004. **Location:** `crates/core/src/git/http/mod.rs` `mod tests`.
- Given the entry of the other http tests, a real pack built as in `:1037` (so `pack.len() > 12`), and two commands in one request: `refs/heads/main` from `ZERO_ID` to the pushed commit, and `refs/heads/ghost` from `ZERO_ID` to `"b".repeat(40)`
- When the push is received
- Then the report lines are `unpack ok`, `ok refs/heads/main`, `ng refs/heads/ghost object not found` (the text of `process_ref_update`, `http/mod.rs:635-637`), the commit, its tree and its blob are in the store, and `refs/heads/ghost` has no row
- Red before the fix because (mutation): remove the fallback from `import_tips`; the whole request fails with `ERR_NOT_FOUND` instead of producing the report.

#### DT-010: structural, the three sites changed and clone kept
- **Method:** `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_from_repo\('` prints `1` (clone); `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg 'import_from_repo'` prints nothing (rg exit status 1); `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_tips\('` prints `2`; `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg -c 'import_tips\('` prints `1`; `awk '/^mod tests/{exit} {print}' crates/core/src/git/odb.rs | rg -c 'import_from_repo\('` prints `2` (the definition at `:192` and the fallback call).

#### DT-011: measurement, not gating
- **Method:** the temporary test of Section 2 (clone a remote, write N blobs, `import_from_repo`, then 5 times: advance the remote and time `call_fetch_branch`), run before and after for N in 500, 5 000, 20 000; not committed. The fetch at 20 000 objects was under 250 ms in each of 5 samples after the change (recorded evidence, enforced by no test); both tables go in the commit body.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| none | none | none | none |

### 8.5 Security tests
Not applicable.

## 9. Implementation order
1. Add `import_tips` and DT-001 to DT-004 in `odb.rs` (green; nothing calls it yet). The `FailingPutBlobs` wrapper lives there.
2. Switch fetch and pull in `tools/git.rs` with `fetch_tips` and add DT-005 to DT-007; prove DT-001, DT-005, DT-006 and DT-007 red by mutation, revert.
3. Switch push receive in `http/mod.rs` and add DT-008, DT-009, DT-012; prove DT-008 and DT-012 red by mutation, revert.
4. Run DT-010, then DT-011, record the numbers.
Each step leaves the suite green; steps 1, 2, 3 are separate stories along these lines.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 3 (MINOR) | IMPLEMENTABLE |

**Amendments applied:** guardrail list corrected (`:1368` covers upload-pack only; `:985` sends no pack); DR-005 given its own test DT-012; DT-011 wording made non gating.
