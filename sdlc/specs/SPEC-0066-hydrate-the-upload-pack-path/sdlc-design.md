# Hydrate the upload-pack path only when the object index changed: technical debt design

> Id: SPEC-0066
> Nature: DEBT
> Depth: S
> Depth evidence: DEBT / depth S: 1 production line in 1 module (`crates/core/src/git/http/mod.rs`), 3 tests (2 in `http/mod.rs`, 1 in `git/repo.rs`), no public contract, no persisted change.
> Status: draft
> Generated on: 2026-10-30
> Behavior change: none
> From backlog: BL-0113
> Depends on: SPEC-0065 (`hydrate_if_stale`, read only)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Debt
### DBT-001: every clone and fetch re-checks every object of the repository
- **Current structure:** `handle_upload_pack` locks the entry's libgit2 repository and calls `entry.objects.export_to_repo(&repo)` (`crates/core/src/git/http/mod.rs:433-434`) before building the pack. That lists every row of the object index and probes libgit2 once per row (`git/odb.rs:340-370`), although `hydrate` already skips it when nothing changed (SPEC-0065).
- **Cost paid today:** O(repository objects) per request: 124 to 172 ms at 20 000 reachable objects out of a 1 590 to 1 806 ms request (Section 2). The rest of the request (the pack build) is not touched by this lot (BL-0114).
- **Target structure:** the handler calls `entry.objects.hydrate_if_stale(&repo)`, the same method `hydrate` uses on the same bound repository (`entry.repo`).
- **Observable behavior:** unchanged: the same objects are materialised before `build_pack`, and the same pack comes out.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/http/mod.rs` (production, before `mod tests` at line 673) | `handle_upload_pack`: `export_to_repo` becomes `hydrate_if_stale` (`:434`) | 1 line |
| `crates/core/src/git/http/mod.rs` (tests) | 2 tests | 2 tests |
| `crates/core/src/git/repo.rs` (tests, `mod tests` at line 229) | 1 test | 1 test |
| `.agent_docs/git.md` | the "Hydration marker" section: remove the upload-pack limit line, name the new state | 1 section |

**Other callers of `export_to_repo(`:** `tools/git.rs` none in production (`hydrate` uses `hydrate_if_stale` since SPEC-0065); `git/odb.rs:340` (definition, kept) and tests. After this lot no production code calls it.
**Callers of `handle_upload_pack`:** `http/mod.rs:179` (the route), tests at `:851,870,1504`. Unchanged.
**Non source occurrences:** `.agent_docs/git.md` (Hydration marker section, last bullet names BL-0113). `rg export_to_repo` finds nothing in `README.md`, `AGENTS.md`, Makefile or CI files.
**Contracts touched:** none. The git smart HTTP protocol response is byte for byte the same pack content for the same repository.
**Guardrail coverage:** the three existing upload-pack tests (`http/mod.rs:851` empty want, `:870`, `:1504` push then fetch round trip) and the 32 `repo.rs`/`http` push and entry tests. **Test command:** `cargo test -p mcp-fs-core --lib git` then `cargo test --workspace`.

**Baseline** (measured 2026-10-30, debug build, local SQLite and local blob store; a temporary test, not committed: a commit whose tree holds N files, imported with `import_reachable`, `refs/heads/main` set, then 5 times `handle_upload_pack` for that commit and `export_to_repo` alone; milliseconds):

| Reachable objects | whole request | `export_to_repo` alone |
|---|---|---|
| 500 | 26, 26, 25, 26, 24 | 2, 2, 2, 2, 2 |
| 5 000 | 259, 244, 259, 311, 257 | 20, 22, 29, 22, 22 |
| 20 000 | 1645, 1621, 1806, 1799, 1590 | 144, 172, 162, 138, 124 |

**Total:** 3 files, 1 production line, 3 tests, 1 doc, all recounted.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: the upload-pack path hydrates through the marker
> `handle_upload_pack` SHALL call `entry.objects.hydrate_if_stale(&repo)` where it called `entry.objects.export_to_repo(&repo)`.
- **Old form:** `entry.objects.export_to_repo(&repo).await?;` (`http/mod.rs:434`). **New form:** `entry.objects.hydrate_if_stale(&repo).await?;`. The lock on `entry.repo` is held across the call exactly as before, so the repository passed is the bound one.
- **Applies to:** `crates/core/src/git/http/mod.rs`.

#### DR-002 [EARS-U]: the entry's repository is really bound
> The entry created by `GitRepoStore` SHALL take the `hydrate_if_stale` fast path for its own repository.
- **New form:** this already holds through `create_entry`'s `objects.bind_repo(repo.path())` (`git/repo.rs:208`); this requirement adds the test that fails if the call is dropped (DT-003). No production line changes for it.

### 4.2 Invariance
#### DR-003 [EARS-U]: behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object `export_to_repo` copies into libgit2 is the same object as before.

### 4.3 Compatibility
No public contract is touched: no compatibility requirement.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| git smart HTTP upload-pack response | wire protocol | yes | unchanged (DR-003) | none | none |

## 6. Decisions
- **DEC-001:** reuse `hydrate_if_stale` with no new mechanism. **Alternatives considered:** a separate marker for the protocol path (rejected: one marker per entry is correct because both paths hydrate the same repository); skipping hydration when the pack build can succeed without it (rejected: libgit2 packs from its own object store, so a missing object would produce an incomplete pack). **Implemented by:** DR-001. **Evidence:** `http/mod.rs:433-436`, `git/odb.rs:144-158`.
- **DEC-002:** the gain is stated as it is: about 8 to 10% of a request at 20 000 reachable objects. The pack build, which ignores `have` lines (`http/mod.rs:408-410,432`), is the larger cost and is deferred as BL-0114 because negotiating changes the protocol exchange. **Implemented by:** n/a, no code impact.
- **DEC-003:** DT-003 proves the binding by the declared limit itself: after a synchronisation, wiping the repository's loose objects is NOT repaired by the next `hydrate_if_stale` (the marker holds), whereas an unbound repository would be re-exported. **Alternatives considered:** a counting blob backend injected into the entry (rejected: `create_entry` builds its blob store from configuration, `repo.rs:204`); a `#[cfg(test)]` accessor on the marker (rejected: production code for a test). **Implemented by:** DR-002. **Evidence:** `git/odb.rs:1211` (`forget_loose_objects`, the same trick, private to the `odb.rs` tests; DT-003 carries its own three line helper).
- **Assumption:** the loose object directories of a bare repository are the two hex character directories under `objects/` (as `forget_loose_objects` assumes), and the objects `export_to_repo` writes are loose objects.

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
#### DT-001: a commit written by another handle is served
- **Validates:** DR-001, DR-003. **Location:** `crates/core/src/git/http/mod.rs` `mod tests`. Given the entry of the other http tests, a first commit seeded with `seed_commit(&e.objects, ..)` and a first upload-pack for it, then a second `BlobObjectDb` over the same store (`BlobObjectDb::new(e.objects.blobs().clone(), e.objects.index().clone())`) seeding a second commit, and `refs/heads/main` set to it; When an upload-pack for the second commit is handled; Then the response pack, indexed into a fresh bare repository as in `a_push_then_a_fetch_round_trips_the_same_objects`, contains the second commit and its blob.
- Red before the fix because (mutation): make `hydrate_if_stale` return `Ok(0)` whenever a marker exists, ignoring the count (`git/odb.rs`); the pack lacks the second commit and the assertion fails.

#### DT-002: two requests in a row both serve the commit
- **Validates:** DR-001, DR-003. **Location:** `http/mod.rs` `mod tests`. Given one seeded commit; When two identical upload-pack requests are handled one after the other; Then each response pack indexes into a fresh bare repository and contains the commit and its blob.

#### DT-003: the store's entry takes the fast path
- **Validates:** DR-002. **Location:** `crates/core/src/git/repo.rs` `mod tests` (the entry comes from `GitRepoStore::init_repo`). Given an entry holding 3 seeded objects (`seed_commit`) and a first `entry.objects.hydrate_if_stale(&repo)` that exports them (returns 3); When the repository's loose object directories are removed (the two hex character directories under `repo.path().join("objects")`) and `hydrate_if_stale` is called again; Then it returns `0` and `repo.odb().unwrap().exists(commit_oid)` is false; And a following `export_to_repo` returns `3` and the commit is present again.
- Red before the fix because (mutation): remove the `bind_repo` call in `create_entry` (`repo.rs:208`); the second call re-exports and returns `3`, the first assertion fails.

#### DT-004: structural
- **Method:** `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg -c 'hydrate_if_stale\('` prints `1`; the same with `export_to_repo\(` prints nothing (rg exit status 1); `rg -n 'export_to_repo\(' crates/core/src --glob '*.rs'` lists only `git/odb.rs` lines (definition, doc, tests) and `tools/git.rs` test lines; `.agent_docs/git.md` no longer carries the line naming the upload-pack path as an exception.

#### DT-005: measurement, not gating
- **Method:** the temporary test of Section 2, run after the change: the hydration part of the request (time `hydrate_if_stale` on the synchronised repository) is under 10 ms at 20 000 objects in each of 5 samples (baseline 124 to 172 ms); the whole request numbers are recorded in the commit body; not committed, not enforced.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| none | none | none | none |

## 9. Implementation order
1. Write DT-003 (the binding test) and prove it red by mutation; it needs no production change.
2. Write DT-001 and DT-002, switch the one line in `handle_upload_pack`; prove DT-001 red by mutation.
3. Update `.agent_docs/git.md`, run DT-004 and DT-005, record the numbers.
Each step leaves the suite green.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | pending | pending | pending |

**Amendments applied:** none yet
