# Import only the new objects on the bounded git write paths: technical debt design

> Id: SPEC-0062
> Nature: DEBT
> Depth: M
> Depth evidence: 2 modules (`crates/core/src/git/odb.rs`, `crates/core/src/tools/git.rs`), 7 call sites changed, no public contract, no persisted change; 4 requirements.
> Status: designed
> Generated on: 2026-10-30
> Behavior change: none
> From backlog: BL-0110
> Depends on: SPEC-0061 (read only: `import_object` stays)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Debt

### DBT-001: seven bounded write paths import the whole repository
- **Current structure:** after writing one new commit (or a short chain of replayed commits) into libgit2, each of seven tools calls `entry.objects.import_from_repo(&repo)`, which enumerates every object (`odb.foreach`), reads each one fully into memory, then probes the blob store for each (`crates/core/src/git/odb.rs:188-228`). The seven sites, all in `crates/core/src/tools/git.rs`: `commit` (`:2323`, new commit, tree and blobs); `merge_inner` clean arm (`:4262`); `land_replay` (`:4891`); `pause_replay` (`:4928`); `merge_resolve_inner` merge commit (`:5632`); the non fast forward merge commit of `pull_branch_inner` (`:6935`); `stash_save` (`:7178`).
- **Cost paid today:** each call is O(repository objects) in time and memory for a bounded change. Measured on `git.commit` (Section 2): 265 to 273 ms at 5 000 objects and 1 099 to 1 173 ms at 20 000, against 32 to 39 ms at 500, about 55 microseconds per object, of which about 5 are the per call hydrate (BL-0111).
- **Target structure:** `BlobObjectDb::import_reachable(repo, tips)` stores the objects reachable from the new tip that the store lacks, stopping at any object the store holds, comparing each tree with its first parent's tree so only changed entries cost a store probe. Each of the seven sites calls it with the tip it just created.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/odb.rs` | add `BlobObjectDb::import_reachable`; `import_from_repo` and `import_object` untouched; 10 unit tests plus 2 test backend wrappers in `mod tests` (starts at line 330) | 1 new method, 10 tests, 2 helpers |
| `crates/core/src/tools/git.rs` (production, before `mod tests` at line 7655) | 7 calls switched | 7 edit sites (`:2323,4262,4891,4928,5632,6935,7178`) |
| `crates/core/src/tools/git.rs` (tests module) | 7 integration tests and 1 closure helper added | 7 tests, 1 helper |

**Callers of `import_from_repo` NOT changed:** `crates/core/src/git/http/mod.rs:572` (push receive), `crates/core/src/tools/git.rs:3324` (clone), `:3918` (fetch), `:6803` (pull's fetch): 4 production sites, deferred as BL-0112. The remaining matches of `rg -n import_from_repo crates/core/src --glob '*.rs'` (20 lines in all: 1 in `git/http/mod.rs`, 6 in `odb.rs`, 13 in `tools/git.rs` of which 10 are the production sites) are the definition (`odb.rs:188`), the module comment (`odb.rs:26`), the `import_object` doc comment (`odb.rs:237`) and its mention of the old walk (`odb.rs:256`), the comment at `tools/git.rs:17`, and tests (`odb.rs:495,538`, `tools/git.rs:24968,25107`).
**Non source occurrences:** `sdlc/specs/SPEC-0007-git/design.md:24,80` and `SPEC-0052-git-tags/sdlc-design.md:44` mention the import; existing designs are not modified. `rg import_from_repo` outside `crates` and `sdlc` returns nothing.
**Contracts touched:** `BlobObjectDb` gains one `pub` method (internal). No tool schema, route or stored format changes.
**Guardrail coverage:** the git suite in `crates/core/src/tools/git.rs` (`rg -c 'fn e2e_' crates/core/src/tools/git.rs` = 683) plus the `odb.rs` unit tests. **Test command:** `cargo test -p mcp-fs-core --lib git` then `cargo test --workspace`.

**Baseline** (measured 2026-10-30, debug build, local SQLite and local blob store; a temporary test, not committed: N blobs written with `repo.blob`, `import_from_repo`, then 5 file changes each followed by `git.commit`; milliseconds):

| Objects in the repository | `git.commit` |
|---|---|
| 500 | 39, 32, 32, 32, 33 |
| 5 000 | 273, 269, 265, 265, 270 |
| 20 000 | 1173, 1099, 1107, 1103, 1173 |

**Total:** 2 files, 7 production edit sites, 1 new method, 17 new tests, all recounted.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)

### 4.1 Structural

#### DR-001 [EARS-U]: import by reachability
> The object store SHALL provide `BlobObjectDb::import_reachable(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize>` that stores, in the blob store and the object index, every object reachable from `tips` that the blob store lacks, and returns how many it stored.
- **Old form:** only `import_from_repo(&self, repo) -> Result<usize>` (every object) and `import_object(&self, repo, oid) -> Result<usize>` (one object, SPEC-0061).
- **New form (normative algorithm):**
  1. An object the blob store already holds (`self.exists(&sha)`) is not read, stored or descended into: a stored object implies its closure is stored.
  2. A tip that is a commit: store it; visit each parent as an object; visit its tree with `base` = the tree id of its first parent when that parent commit is readable in `repo`, else none. A parent id `repo` does not hold is an error under step 7 and the call stores nothing. A parent the store holds but `repo` does not hold is not descended into (step 1), so the check of step 7 does not apply to it, and `base` is then none.
  3. A tip that is an annotated tag: store it and visit its target. A tip that is a tree: visit it with no base. A tip that is a blob: store it.
  4. Visiting a tree with `base`: skip it if its id equals `base`; skip it if the store holds it; otherwise store it and, for each entry: skip a gitlink (mode `0o160000`, a submodule commit never stored); skip an entry whose name and id equal the same named entry of the base tree; otherwise visit the entry (a blob as an object, a tree with `base` = the same named base entry when it is a tree, else none).
  5. An id is processed at most once per call.
  6. **Order:** an object is written only after every object it references that must be stored has been written (post order: blobs, then trees, then commits, parents before children), so that a crash never leaves a stored object whose closure is missing.
  7. An id `repo` does not hold is `ToolError::not_found(format!("git object '{oid}' not found in the repository"))` and nothing is stored from that call (all reads happen before the first write).
  8. An object of an unstorable type (`object_type_name` fails) is skipped like `import_from_repo` does (the `object_type_name(obj.kind()).is_err()` check, about `odb.rs:208-210`).
  Objects are written with `write_with_sha`, which also records the index row, exactly like `import_from_repo` and `import_object`.
- **Applies to:** `crates/core/src/git/odb.rs`. `import_from_repo` and `import_object` are not modified.
- **Casing convention:** Rust snake_case.

#### DR-002 [EARS-U]: the seven bounded sites import by reachability
> Each of the seven bounded write paths SHALL call `entry.objects.import_reachable(&repo, &[<tip>])` where it called `import_from_repo`, and SHALL NOT call `import_from_repo`.
- **Old form:** `entry.objects.import_from_repo(&repo).await?;`
- **New form, per site** (the tip is the commit the site just created or the one it moves the branch or pause ref to):

| Site | Line | New call |
|---|---|---|
| `commit` | `tools/git.rs:2323` | `import_reachable(&repo, &[oid])` |
| `merge_inner`, clean arm | `:4262` | `import_reachable(&repo, &[oid])` |
| `land_replay` | `:4891` | `import_reachable(repo, &[new_tip])` |
| `pause_replay` | `:4928` | `import_reachable(repo, &[replay_tip])` |
| `merge_resolve_inner` | `:5632` | `import_reachable(&repo, &[oid])` |
| `pull_branch_inner`, merge commit | `:6935` | `import_reachable(&repo, &[merge_oid])` |
| `stash_save` | `:7178` | `import_reachable(&repo, &[oid])` |

- The ordering rule of each site is unchanged: the objects are in the blob store before the ref row names them.

#### DR-003 [EARS-UB]: the four unbounded sites stay on the whole walk
> The server SHALL NOT change the four sites that import a pack: push receive (`git/http/mod.rs:572`), clone (`tools/git.rs:3324`), fetch (`:3918`) and the fetch step of pull (`:6803`).
- **Reason:** a pack brings an unbounded set, and `reconcile_incoming_tags` relies on the tag objects it brings (BL-0112).

### 4.2 Invariance

#### DR-004 [EARS-U]: behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

### 4.3 Compatibility
No public contract is touched: no compatibility requirement.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `BlobObjectDb::import_reachable` (new) | internal method | no | added; `import_from_repo` and `import_object` kept | none | none |
| `git.commit`, `git.merge`, `git.rebase`, `git.cherry_pick`, `git.revert`, `git.merge_resolve`, `git.remote_pull`, `git.stash_save` results | tool output | yes | unchanged byte for byte (DR-004) | none | none |

## 6. Decisions
- **DEC-001:** a new method that walks from the new tip, rather than tracking the ids each writer creates. **Alternatives considered:** return the created ids from every writer (rejected: libgit2 writes merge trees and blobs internally, `merge.rs:359,420`, with no id list, and 7 sites would each need a different plumbing); keep the whole walk (rejected: it is the debt). **Rationale:** the tip is known at every bounded site, and reachability covers merge, replay and stash alike. **Implemented by:** DR-001, DR-002. **Evidence:** `odb.rs:188-228`, `merge.rs:359,420`.
- **DEC-002:** compare each tree with its first parent's tree and probe the store only for changed entries. **Alternatives considered:** probe the store for every entry of every new tree (rejected: one existence check per entry of each changed directory, which is a network request per entry on the S3 backend, `storage/blob/s3.rs:128`); compare with every parent (rejected: a merge commit's other parents are probed through the store anyway, step 4 of DR-001, and one base keeps the diff linear). **Rationale:** O(changed entries) store probes. **Implemented by:** DR-001.
- **DEC-003:** "the store holds it" stops the walk. **Alternatives considered:** always descend (rejected: that is the whole walk again). **Rationale:** the invariant that makes this safe is the write order (DEC-004). **Implemented by:** DR-001 step 1.
- **DEC-004:** post order writes. **Alternatives considered:** pre order (rejected: a crash after a commit and before its tree would leave a stored commit with a missing closure that every later call would trust and stop at). **Implemented by:** DR-001 step 6.
- **DEC-005:** two repairs of the whole walk are dropped on purpose for the seven sites: storing objects libgit2 holds that no new tip reaches, and recording a missing index row for an object whose blob exists. `import_object` and the four unbounded sites keep their behavior. **Alternatives considered:** probe the index for each stopped object (rejected: doubles the probes). **Rationale:** the libgit2 repository is a cache rebuilt from the store (`hydrate`, `tools/git.rs:2069`), so an object the store lacks is lost at the next rebuild whichever tool created it; every bounded site names its new objects through its tip. **Implemented by:** DR-002. **Evidence:** `tools/git.rs:2069-2072`; DT-011 to DT-017 prove the closure of each tip.
- **DEC-006:** a gitlink entry is skipped. **Alternatives considered:** fail on it (rejected: `import_from_repo` imports only objects libgit2 holds, and a submodule commit is not in the odb). **Implemented by:** DR-001 step 4.
- **DEC-007:** the measurement is evidence in the commit body, not a gating test. **Alternatives considered:** a timing assertion (rejected: debug build timings vary by machine). **Implemented by:** n/a, no code impact.
- **Assumption:** `pause_replay` needs nothing beyond what is reachable from `replay_tip`: the operation row stores ids of commits already in the store and conflict paths, not object ids (`tools/git.rs:4938-4955`).

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

Unit tests in `crates/core/src/git/odb.rs` `mod tests` (helper `odb()` at line 334; two wrappers over `Arc<dyn BlobBackend>` delegating to `LocalBlobStore`: a counting one that counts `exists` calls and a recording one that records the order of `put` keys).

#### DT-001: a commit tip imports exactly its three objects
- **Validates:** DR-001. Given a bare repository with 2 000 blobs (`format!("blob {i}")`) not imported and one commit (one blob `a.txt`, its tree, the commit); When `import_reachable(&repo, &[commit])`; Then it returns `3` and `o.list().await.unwrap().len()` equals `3`.
- Red before the fix because (mutation): make the body call `import_from_repo`; the length is `2003`.

#### DT-002: a second commit stores only what changed
- **Validates:** DR-001. Given the first commit already imported, holding a directory `d` of 200 files `f000` to `f199` (distinct contents) and a file `top`; and a second commit (parent the first) changing only `d/f007`; When `import_reachable(&repo, &[second])`; Then it returns `4` (commit, root tree, tree `d`, the new blob) and the store length grew by `4`.

#### DT-003: the store probes are bounded
- **Validates:** DR-001, DEC-002. Same repository as DT-002 with the counting wrapper reset after the first import; When the second import runs; Then the number of `exists` calls is at most `8` (the tip, its parent, the root tree, `d`, `f007`, plus slack), far under `200`.
- Red before the fix because (mutation): probe every entry of every new tree; the count is above `200`.

#### DT-004: a merge commit sharing its second parent's tree
- **Validates:** DR-001. Given two imported commits `p1`, `p2` with different trees and a merge commit with parents `[p1, p2]` and the tree of `p2`; When `import_reachable(&repo, &[merge])`; Then it returns `1` (the commit only) because the tree is held by the store.

#### DT-005: a root commit imports all its entries
- **Validates:** DR-001. Given a commit with no parent holding `x.txt`, `y.txt` and a directory `s` with `z.txt` (three distinct contents); When imported into an empty store; Then it returns `6` (1 commit, 2 trees, 3 blobs).

#### DT-006: a gitlink entry is skipped
- **Validates:** DR-001, DEC-006. Given a tree whose entry `sub` has mode `0o160000` and an id libgit2 does not hold (`TreeBuilder::insert("sub", oid, 0o160000)`) and a commit on it; When imported; Then it succeeds, returns `2` (commit and tree) and the store does not hold the `sub` id.

#### DT-007: an unknown tip
- **Validates:** DR-001. Given an empty bare repository; When `import_reachable(&repo, &[Oid::from_str("1111111111111111111111111111111111111111")])`; Then it fails with code `ERR_NOT_FOUND` and message `git object '1111111111111111111111111111111111111111' not found in the repository`, and the store is empty.

#### DT-008: an annotated tag tip
- **Validates:** DR-001. Given an annotated tag object on an unimported commit; When imported with the tag as tip; Then the tag, the commit, its tree and its blobs are stored.

#### DT-009: a tip the store already holds
- **Validates:** DR-001. Given a commit fully imported and the counting wrapper reset; When imported again; Then it returns `0` after exactly `1` `exists` call.

#### DT-010: dependencies are written first
- **Validates:** DR-001 step 6, DEC-004. Given the recording wrapper and a commit with a tree and a blob; When imported; Then the recorded `put` keys are in the order blob, tree, commit.
- Red before the fix because (mutation): write in discovery order; the commit is recorded first.

Integration tests in `crates/core/src/tools/git.rs` `mod tests`, using a helper `assert_closure_in_store(entry, tip)` that walks every object reachable from `tip` in the libgit2 repository (commits, trees, blobs) and asserts `entry.objects.exists` for each, and the harness `Env`:

#### DT-011: `git.commit` adds exactly three objects
- **Validates:** DR-002, DR-004. Given a project with a commit (`seed_tags(&e)`), hydrated, then 2 000 blobs written into libgit2 and not imported, `before = entry.objects.list().await.unwrap().len()`; When a file `/c.txt` is written and `git.commit` succeeds; Then the store length is `before + 3` (blob, tree, commit) and `assert_closure_in_store` holds for the new commit.
- Red before the fix because (mutation): restore `import_from_repo` at `git.rs:2323`; the length is `before + 2003`.

#### DT-012 to DT-017: the other six sites
Each: Given the same 2 000 unimported strays (DT-016: see below); When the tool runs; Then `assert_closure_in_store` holds for the new tip, and for DT-012 to DT-015 and DT-017 the store length is below `before + 200` (never the strays).
- **DT-012:** `git.merge` clean (two branches changing different files, `merge_inner`).
- **DT-013:** a rebase of two commits landing (`land_replay`).
- **DT-014:** a rebase pausing on a conflict (`pause_replay`); the closure of the replay ref tip.
- **DT-015:** `git.merge_resolve` completing with a merge commit.
- **DT-016:** `git.remote_pull` with a diverged branch producing a merge commit (pattern of `e2e_new_940`, `tools/git.rs:12151`); `assert_closure_in_store` holds for the merge commit. No store length bound applies: the fetch step of the same pull imports every object (`:6803`, unchanged) and nothing can inject strays between the two steps.
- **DT-017:** `git.stash_save` (the closure of the stash commit).
- Red before the fix because (mutation): for DT-012 to DT-015 and DT-017 restore `import_from_repo` at the site; the length is above `before + 2000`. For DT-016 delete the import call at `:6935`; the closure assertion fails on the merge commit's tree or blob.

#### DT-018: structural, only the four pack sites keep the whole walk
- **Method:** `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_from_repo\('` prints `3`; `rg -c 'import_from_repo\(' crates/core/src/git/http/mod.rs` prints `1`; `rg 'import_from_repo\(' crates/core/src/tools/git_tags.rs` prints nothing; `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_reachable\('` prints `7`.

#### DT-019: measurement, not gating
- **Method:** the temporary test of Section 2, run before and after, for N in 500, 5 000, 20 000; not committed. After the change `git.commit` at 20 000 objects SHALL be under 250 ms in each of 5 samples; both tables go in the commit body.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| none | none | none | none |

### 8.5 Security tests
Not applicable.

## 9. Implementation order
1. Add `import_reachable` and DT-001 to DT-010 in `odb.rs` (green; nothing calls it yet). The two test wrappers live there.
2. Switch `commit` (`:2323`) and add DT-011 with its closure helper; prove DT-001 and DT-011 red by mutation, revert.
3. Switch the other six sites and add DT-012 to DT-017.
4. Run DT-018, then DT-019, record the numbers.
Each step leaves the suite green; steps 1, 2 and 3 are separate stories along these lines.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 (+2 MINOR) | 1 (MINOR) | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** round 1: DT-016 no store length bound (the pull's fetch step imports everything) with its own mutation; DR-001 step 2 on an unreadable parent; distinct contents stated for DT-002 and DT-005; `odb.rs` citation corrected to the `object_type_name` check (`:208-210`).

## Design additions (2026-10-30)

Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`). Skill: `rust` (loaded earlier in this session; project gate: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, and `make check`). Spot checks of cited code done: the seven production call sites (`tools/git.rs:2323,4262,4891,4928,5632,6935,7178`), the four kept sites (`git/http/mod.rs:572`, `tools/git.rs:3324,3918,6803`), `hydrate` (`tools/git.rs:2069`), `S3BlobStore::exists` (`storage/blob/s3.rs:128`).

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/odb.rs` | modify: add `import_reachable`; 10 unit tests and 2 test wrappers in `mod tests` | DR-001 | US-0119 |
| `crates/core/src/tools/git.rs` (production) | modify: `commit` call (`:2323`) | DR-002 | US-0120 |
| `crates/core/src/tools/git.rs` (tests) | modify: closure helper and DT-011 | DR-002, DR-004 | US-0120 |
| `crates/core/src/tools/git.rs` (production) | modify: six calls (`:4262,4891,4928,5632,6935,7178`) | DR-002 | US-0121 |
| `crates/core/src/tools/git.rs` (tests) | modify: DT-012 to DT-017 | DR-002 | US-0121 |

Affected existing tests: none to modify (Section 8.4). Affected documentation: none.

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001 to DT-010 | unit | direct call on `BlobObjectDb`, counting and recording wrappers | `crates/core/src/git/odb.rs` `mod tests` | US-0119 |
| DT-011 | integration | `Env` harness, `git.commit` | `crates/core/src/tools/git.rs` `mod tests`, near `e2e_tag_028_annotated_tag_imports_only_its_own_object` | US-0120 |
| DT-012 to DT-017 | integration | `Env` harness (merge, rebase, merge_resolve, pull, stash) | `crates/core/src/tools/git.rs` `mod tests`; pull pattern `e2e_new_940` (`:12151`) | US-0121 |
| DT-018 | structural | `awk` and `rg` on the source | repository | US-0122 |
| DT-019 | measurement, not gating | temporary test, not committed | scratch in `tools/git.rs` | US-0122 |

Test data: generated (blobs written with `repo.blob`, repositories built with libgit2 in temp dirs). The mutation proofs of DT-001, DT-003, DT-010, DT-011 to DT-017 are run by their stories.

### Decisions (continued)
- **DEC-008:** four stories in the order of Section 9, serialized: US-0119 (the method and its unit tests), US-0120 (the first site and the closure helper), US-0121 (the six other sites), US-0122 (structural check, measurement, close). **Obliged by:** DR-004 (invariance; each step leaves the suite green). **Rationale:** the method alone cannot change behavior; the first site proves the pattern and builds the closure helper the six others reuse; the six others are one story because they share the helper and the 2 000 stray setup; the close is independent and carries the numbers. **Alternatives considered:** one story per site (rejected: seven stories of one edit and one test, the same re read cost seven times); one story for everything (rejected: 17 tests and 7 edits, too wide for one tier 2 review). **Source:** design.
- **DEC-009:** every story `min_tier` 2: each reads existing patterns by path in a 29k line test module and needs a mutation proof, which tier 3 and 4 budgets cannot hold; US-0119 carries a 10 test algorithm with two backend wrappers, a decision density tier 3 cannot absorb. **Obliged by:** n/a (no code impact): routing choice. **Source:** design.
- **DEC-010:** the closure helper `assert_closure_in_store(entry, tip)` is written in US-0120 and reused by US-0121. **Obliged by:** DR-002 (it is how the tests prove the dropped repairs of DEC-005 cost nothing). **Rationale:** one definition of "the whole tip is stored". **Alternatives considered:** a copy per test (rejected: seven copies). **Source:** design.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0119 | import the objects reachable from a tip | P1 | DR-001 | main | 2 | no |
| US-0120 | import only the new objects when committing | P1 | DR-002 (commit site), DR-004 | US-0119 | 2 | no |
| US-0121 | import only the new objects on merge, replay, resolve, pull and stash | P1 | DR-002 (six sites), DR-004 | US-0120 | 2 | no |
| US-0122 | keep the pack sites on the whole walk and close the spec | P2 | DR-003, DR-004 | US-0121 | 2 | yes |

