# Tag object import and listing lookups: technical debt design

> Id: SPEC-0061
> Nature: DEBT
> Depth: M
> Depth evidence: 2 modules (`crates/core/src/git/odb.rs`, `crates/core/src/tools/git_tags.rs`), 3 production edit sites, no public contract, no persisted change; 4 requirements.
> Status: designed
> Generated on: 2026-10-30
> Behavior change: none
> From backlog: BL-0080
> Depends on: SPEC-0052 (read only)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Debt

### DBT-001: annotated `git.tag_create` imports the whole repository
- **Current structure:** after writing one tag object into libgit2's object store, `tag_create` calls `entry.objects.import_from_repo(&repo)` (`crates/core/src/tools/git_tags.rs:149`), which enumerates every object (`odb.foreach`), reads each one fully into memory, then probes the blob store for each (`crates/core/src/git/odb.rs:188-228`), all under the repository write lock and the repository mutex (`git_tags.rs:116-124`).
- **Cost paid today:** creation is O(repository objects) in time and memory for an operation that adds one object. Measured (Section 2): 364 to 709 ms at 5 000 objects and 1 586 to 2 872 ms at 20 000, against 23 to 35 ms and 100 to 134 ms for a lightweight tag of the same repository.
- **Target structure:** `BlobObjectDb::import_object(repo, oid)` imports exactly one object; `tag_create` calls it with the tag object it just wrote.
- **Observable behavior:** unchanged.

### DBT-002: the tag listing looks each annotated tag up up to three times
- **Current structure:** for each tag, `tag_entry` calls `repo.find_object` to learn the kind (`git_tags.rs:341-345`), `peel_to_commit` calls `repo.find_object` again (`git_tags.rs:307-311`, called at `:348`), and for an annotated tag `annotation_details` calls `repo.find_tag` a third time (`git_tags.rs:366-368`, called at `:358`).
- **Cost paid today:** up to 3 object lookups per annotated tag, 2 per plain tag. The share of the listing time they cost was not isolated: listing 5 000 tags (half annotated) takes 65 to 78 ms including the hydrate, and 20 000 tags 656 to 1 950 ms, noisy (Section 2). No speed figure is promised for the listing.
- **Target structure:** one `find_object` per tag; kind, peeled commit and annotation details all derive from that one object.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/odb.rs` | add `BlobObjectDb::import_object`; `import_from_repo` untouched; 4 unit tests added in `mod tests` (starts at line 296) | 1 new method, 4 tests |
| `crates/core/src/tools/git_tags.rs` | `tag_create` (line 149) calls `import_object`; `tag_entry` (340-364) and `annotation_details` (366) take one looked up object | 3 edit sites |
| `crates/core/src/tools/git.rs` (tests module, starts at line 7655) | 2 integration tests added | 2 tests |

**Other callers of `import_from_repo`, not changed (BL-0110):** 11 production call sites: `crates/core/src/git/http/mod.rs:572`; `crates/core/src/tools/git.rs:2323,3324,3918,4262,4891,4928,5632,6803,6935,7178`. Also: the changed site `git_tags.rs:149`, the definition `odb.rs:188`, two comment mentions (`odb.rs:26`, `tools/git.rs:17`) and four test uses (`odb.rs:461,504`, `tools/git.rs:24968,25107`). That is 19 lines in `rg -n import_from_repo --glob '*.rs' crates` (the 11 sites, the changed site, the definition, the two comments, the four tests).
**Other callers of the helpers:** `peel_to_commit` is also called at `git_tags.rs:162,210,281,641,1077`; it stays unchanged.
**Non source occurrences:** `sdlc/specs/SPEC-0052-git-tags/sdlc-design.md:44` and `sdlc/specs/SPEC-0007-git/design.md:24,80` mention `import_from_repo`; existing designs are not modified. No `.env.example`, CI, Makefile, container or `.agent_docs` line names the import (`rg import_from_repo` outside `crates` and `sdlc` returns nothing).
**Contracts touched:** `BlobObjectDb` gains one `pub` method (internal: only this workspace calls it). No tool schema, route or stored format changes.
**Guardrail coverage:** `rg -c "fn e2e_tag_" crates/core/src/tools/git.rs` = 212 tests, plus the existing `git/odb.rs` unit tests. **Test command:** `cargo test -p mcp-fs-core --lib e2e_tag` then `cargo test --workspace`.

**Baseline** (measured 2026-10-30, debug build, local SQLite and local blob store, one temporary test not committed; 5 calls each, milliseconds):

| Objects in the repository | `git.tag_create` lightweight | `git.tag_create` annotated |
|---|---|---|
| 500 | 3, 7, 3, 8, 3 | 33, 48, 43, 49, 36 |
| 5 000 | 24, 23, 32, 35, 112 | 364, 709, 671, 647, 410 |
| 20 000 | 104, 104, 100, 134, 106 | 2041, 1586, 2190, 2781, 2872 |

The lightweight column is the floor of the per call hydrate (`crates/core/src/tools/git.rs:2069-2072`, `export_to_repo`), which this lot leaves alone (BL-0111).
**Total:** 3 files, 3 production edit sites, 1 new method, 6 new tests, all recounted.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)

### 4.1 Structural

#### DR-001 [EARS-U]: single object import
> The object store SHALL provide `BlobObjectDb::import_object(&self, repo: &git2::Repository, oid: git2::Oid) -> Result<usize>` that copies exactly the object `oid` from the libgit2 object store of `repo` into the blob store and the object index.
- **Old form:** only `import_from_repo(&self, repo) -> Result<usize>`, which copies every object.
- **New form:** `import_object` returns 1 when it stored the object (blob put through `write_with_sha`, which also records the index row), 0 when the blob store already held it (after recording the index row if it was missing, the same repair as `import_from_repo`, `odb.rs:215-222`), and `ToolError::not_found(format!("git object '{oid}' not found in the repository"))` with nothing stored when libgit2 does not hold the object. It reads only that one object. A libgit2 object of an unstorable type (`object_type_name` fails) is skipped like `import_from_repo` does (`odb.rs:206`): returns 0.
- **Applies to:** `crates/core/src/git/odb.rs`. `import_from_repo` is not modified.
- **Casing convention:** Rust snake_case.

#### DR-002 [EARS-U]: annotated tag creation imports one object
> `tag_create` SHALL import only the tag object it wrote, by calling `entry.objects.import_object(&repo, tag_oid)` before the ref row is written, and SHALL NOT call `import_from_repo`.
- **Old form:** `entry.objects.import_from_repo(&repo).await?;` at `crates/core/src/tools/git_tags.rs:149`.
- **New form:** `entry.objects.import_object(&repo, tag_oid).await?;`
- **Applies to:** `git_tags.rs`, the annotated branch of `tag_create` only. The ordering rule of the existing comment stays: the object is in the blob store before the ref row names it.

#### DR-003 [EARS-U]: one lookup per listed tag
> `tag_entry` SHALL perform one `repo.find_object` per tag and SHALL derive the kind (`annotated` iff the object is a tag object), the peeled commit and the annotation details from that object, with no further object lookup in the listing path.
- **Old form:** `tag_entry` calls `find_object`, then `peel_to_commit(repo, &target)` (a second `find_object`), then `annotation_details(repo, &target, map)` (`find_tag`, a third lookup).
- **New form:** `tag_entry` looks the target up once with `Oid::from_str(&target).ok().and_then(|oid| repo.find_object(oid, None).ok())`; `sha` is `object.peel_to_commit()` mapped to its id string, falling back to `target` when the object is missing or does not peel (same fallback as today, `git_tags.rs:346-348`); `annotation_details` takes `&git2::Tag` obtained with `object.as_tag()`. `peel_to_commit` stays for its other callers (Section 2).
- **Applies to:** `git_tags.rs:340-380`.

### 4.2 Invariance

#### DR-004 [EARS-U]: behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; the object stored for an annotated tag has the same id, type, payload, blob key and index row as before.

### 4.3 Compatibility
No public contract is touched: no compatibility requirement.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `BlobObjectDb::import_object` (new) | internal method | no | added; `import_from_repo` kept | none | none |
| `git.tag_create`, `git.tags` results | tool output | yes | unchanged byte for byte (DR-004) | none | none |

## 6. Decisions
- **DEC-001:** a new targeted method rather than changing `import_from_repo`. **Alternatives considered:** make `import_from_repo` take an optional oid list (rejected: 11 other callers rely on the whole walk, and an option on the old method hides which callers are cheap); import all objects reachable from the tag (rejected: the target commit is already in the store, hydrated from it at `git_tags.rs:124`). **Rationale:** smallest change that makes DBT-001 O(1) in the repository size. **Implemented by:** DR-001, DR-002. **Evidence:** `odb.rs:188-228`, `git_tags.rs:116-149`.
- **DEC-002:** `import_object` returns `Result<usize>` (1 or 0), not `bool`. **Alternatives considered:** `bool` (rejected: parity with `import_from_repo` so a test reads both the same way). **Implemented by:** DR-001.
- **DEC-003:** an object libgit2 does not hold is `ERR_NOT_FOUND`, not a silent 0. **Alternatives considered:** return 0 (rejected: `tag_create` would then write a ref row naming an object the store lacks, the exact failure the ordering comment guards against, `git_tags.rs:146-148`). **Implemented by:** DR-001.
- **DEC-004:** the tag listing is not given a speed target. **Alternatives considered:** assert a timing (rejected: debug build timings are noisy, 656 to 1 950 ms at 20 000 tags in the baseline); count lookups with a test seam (rejected: adds production code for a test). The structural check DT-007 counts lookups in the function bodies instead. **Implemented by:** n/a, no code impact.
- **DEC-005:** a side effect is dropped on purpose. Today `import_from_repo` in `tag_create` also stores any object libgit2 holds that the store lacks. After the change, only the tag object is imported. **Alternatives considered:** keep the walk (rejected: it is the debt). **Rationale:** every other writer imports what it creates: the 11 production sites of Section 2 and the tests' explicit calls; nothing relies on `tag_create` repairing them. **Implemented by:** DR-002. **Evidence:** the 11 call sites; DT-005 proves the new behavior.
- **DEC-006:** the measurement (DT-008) is evidence in the story commit body, not a gating test. **Alternatives considered:** a criterion benchmark (rejected: this repository has no `benches/` directory, `ls crates/core/benches` fails). **Implemented by:** n/a, no code impact.
- **Assumption:** `Oid::from_str` of a ref target that is not a valid id yields `None` and the tag is listed with its own text as `sha`, exactly as today (`git_tags.rs:346-348`); not changed.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    cargo test -p mcp-fs-core --lib e2e_tag
    cargo test --workspace
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt --all -- --check

### 8.2 Compatibility tests
None (no contract).

### 8.3 New tests and structural tests

#### DT-001: import_object stores exactly one object
- **Validates:** DR-001. **Location:** `crates/core/src/git/odb.rs`, `mod tests`.
- Given a bare libgit2 repository holding one blob written with `repo.blob(b"one")`, and an empty store (`odb()` helper, `odb.rs:300`)
- When `o.import_object(&repo, oid)` is called twice
- Then the first call returns `1`, the second `0`; `o.read(&oid.to_string())` returns `(ObjectType::Blob, b"one")`; `o.list().await.unwrap().len()` equals `1`.

#### DT-002: import_object leaves other objects alone
- **Validates:** DR-001, DR-002. **Location:** `odb.rs`, `mod tests`.
- Given a bare repository holding 2 000 blobs `blob 0` to `blob 1999` plus one target blob, none imported
- When `o.import_object(&repo, target)` is called
- Then it returns `1` and `o.list().await.unwrap().len()` equals `1`
- Red before the fix because (mutation): replace the body of `import_object` by a call to `import_from_repo`; the list length is `2001`, not `1`.

#### DT-003: import_object repairs a missing index row
- **Validates:** DR-001. **Location:** `odb.rs`, `mod tests`.
- Given the blob already put in the blob store (`o.blobs().put(&blob_key(&sha), &serialize(ObjectType::Blob, b"orphan")?)`, as the existing test at `odb.rs:487-505` does) and no index row
- When `o.import_object(&repo, oid)` is called
- Then it returns `0` and `o.index().object_exists(&sha)` is true.

#### DT-004: import_object on an unknown id
- **Validates:** DR-001. **Location:** `odb.rs`, `mod tests`.
- Given an empty bare repository
- When `o.import_object(&repo, Oid::from_str("1111111111111111111111111111111111111111")?)` is called
- Then it fails with code `ERR_NOT_FOUND` and message `git object '1111111111111111111111111111111111111111' not found in the repository`; `o.list()` is empty.

#### DT-005: annotated tag creation adds exactly one object
- **Validates:** DR-002, DR-004. **Location:** `crates/core/src/tools/git.rs`, `mod tests`, near `e2e_tag_026_delete_an_annotated_tag_reports_the_commit`.
- Given a project with commits (`seed_tags(&e)`), hydrated, then 2 000 extra blobs written straight into libgit2's object store (`repo.blob(...)`, not imported), and `before = entry.objects.list().await.unwrap().len()`
- When `tag(&e, json!({"name": "v1.0", "message": "m"}))` succeeds
- Then `entry.objects.list().await.unwrap().len() == before + 1`; the new object is the tag object (`out["object"]` is in the store with type `tag`); `git.tags` lists `v1.0` as `annotated` with message `m`.
- Red before the fix because (mutation): restore `import_from_repo` at `git_tags.rs:149`; the length is `before + 2001`.

#### DT-006: the stored tag object round trips
- **Validates:** DR-002, DR-004. **Location:** `tools/git.rs`, `mod tests`.
- Given an annotated tag `v2.0` with message `second` created through `git.tag_create`
- When a fresh `git2::Repository::init_bare` is hydrated with `entry.objects.export_to_repo` and the tag object is read there
- Then it parses as a tag whose target is the commit, message `second` and tagger name equal to the person's name part; the blob key `git:<id>` and the index row exist.

#### DT-007: structural, no whole import in tag_create and one lookup per listed tag
- **Asserts:** the old form survives nowhere in the two edit sites.
- **Method:** `awk '/^async fn tag_create/,/^}/' crates/core/src/tools/git_tags.rs | rg 'import_from_repo'` prints nothing (rg exit status 1); `rg -c 'import_object' crates/core/src/tools/git_tags.rs` prints `1`. Lookups in the listing path: `awk '/^fn tag_entry/,/^}/' crates/core/src/tools/git_tags.rs | rg -c 'find_object|find_tag|\bpeel_to_commit\(repo'` prints `1` (the pattern matches the free helper called as `peel_to_commit(repo, ...)` and not the `object.peel_to_commit()` method), and `awk '/^fn annotation_details/,/^}/' crates/core/src/tools/git_tags.rs | rg 'find_object|find_tag'` prints nothing.

#### DT-008: measurement, not gating
- **Method:** the temporary test of Section 2 (create N blobs with `repo.blob`, `import_from_repo`, then time 5 `tag(&e, json!({"name": ..}))` and 5 `tag(&e, json!({"name": .., "message": "m"}))`), run before and after, for N in 500, 5 000, 20 000; not committed. After the change the annotated call at 20 000 objects SHALL be within 2 times the lightweight call plus 20 ms in each of the 5 samples, and the commit body lists both tables.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| none | none | none | none |

### 8.5 Security tests
Not applicable.

## 9. Implementation order
1. Add `import_object` and DT-001 to DT-004 in `odb.rs` (green; nothing calls it yet).
2. Switch `tag_create` to `import_object` and add DT-005, DT-006; prove DT-002 and DT-005 red by mutation, revert.
3. Single lookup in `tag_entry` and `annotation_details` (DR-003); run DT-007 and the full suite.
4. Run DT-008, record the numbers.
Each step leaves the suite green; steps 1 and 2 may be one story, step 3 another.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 2 (MINOR) | IMPLEMENTABLE |

**Amendments applied:** DT-007 commands made literal (the method `peel_to_commit` no longer matches; no output is not `0`).

## Design additions (2026-10-30)

Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`). Skill: `rust` (loaded earlier in this session; the project gate is `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, and `make check`). Spot checks of cited code done: `git_tags.rs:149` (`import_from_repo` call), `odb.rs:188` (definition), `tools/git.rs:2069` (`hydrate`), `git_tags.rs:340-346` (`tag_entry` first lookup).

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/odb.rs` | modify: add `import_object`; 4 unit tests in `mod tests` | DR-001 | US-0116 |
| `crates/core/src/tools/git_tags.rs` | modify: `tag_create` calls `import_object` | DR-002 | US-0117 |
| `crates/core/src/tools/git.rs` (tests module) | modify: 2 integration tests | DR-002, DR-004 | US-0117 |
| `crates/core/src/tools/git_tags.rs` | modify: `tag_entry` and `annotation_details` take one looked up object | DR-003 | US-0118 |

Affected existing tests: none to modify (Section 8.4). Affected documentation: none (`rg import_from_repo` outside `crates` and `sdlc` returns nothing).

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001, DT-002, DT-003, DT-004 | unit | direct call on `BlobObjectDb` | `crates/core/src/git/odb.rs` `mod tests` | US-0116 |
| DT-005, DT-006 | integration | `tag` / `tag_list` through the `Env` harness | `crates/core/src/tools/git.rs` `mod tests` | US-0117 |
| DT-007 | structural | `awk` and `rg` on the source | repository | US-0118 |
| DT-008 | measurement, not gating | temporary test, not committed | `crates/core/src/tools/git.rs` (scratch) | US-0118 |

Test data: generated (blobs written with `repo.blob`). The mutation proofs of DT-002 and DT-005 are run by their stories.

### Decisions (continued)
- **DEC-007:** three stories in the order of Section 9, serialized: US-0116 (the method), US-0117 (the switch), US-0118 (the listing, measurement and close). **Obliged by:** DR-004 (invariance; each step leaves the suite green). **Rationale:** step 1 adds an unused method and cannot change behavior; step 2 is the behavior bearing switch with its mutation proof; step 3 is independent of the import but ends the spec with the measurement, so it closes. **Alternatives considered:** one story (rejected: two test files, three edit sites, a measurement, too wide for a tier 2 review); listing story before the switch (rejected: the closing story carries the measurement of the import fix, so it must come last). **Source:** design.
- **DEC-008:** every story `min_tier` 2: each reads existing patterns by path in a 29k line test module and `odb.rs`, and DT-002 and DT-005 need a mutation proof, which tier 3 and 4 budgets cannot hold. **Obliged by:** n/a (no code impact): routing choice. **Source:** design.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0116 | import a single git object into the blob store | P1 | DR-001 | main | 2 | no |
| US-0117 | import only the tag object when creating an annotated tag | P1 | DR-002, DR-004 | US-0116 | 2 | no |
| US-0118 | look up each listed tag once and close the spec | P2 | DR-003, DR-004 | US-0117 | 2 | yes |

