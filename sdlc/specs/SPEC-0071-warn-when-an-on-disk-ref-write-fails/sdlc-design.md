# Warn when an on disk ref write fails: technical debt design

> Id: SPEC-0071
> Nature: DEBT
> Depth: S
> Depth evidence: 2 files, 3 sites, no public contract change, no persisted change
> Status: specified
> Generated on: 2026-11-02
> Behavior change: none
> From backlog: BL-0095, BL-0105
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: three silent on disk ref writes
- **Current structure:** `branch_create` writes the on disk ref with `let _ = repo.reference(...)` (`crates/core/src/tools/git.rs:2504`); `tag_create` does the same (`crates/core/src/tools/git_tags.rs:158`); `reconcile_incoming_tags` does the same (`crates/core/src/tools/git_tags.rs:449`) and, unlike the publish path (`git_tags.rs:702`), never checks `Reference::is_valid_name`, so a name libgit2 normalises (`a//b` to `a/b`) lands on disk under another name than the index row.
- **Cost paid today:** any libgit2 failure (a locked ref, a full disk) is dropped with no trace, so the index and the on disk repository disagree and an operator has nothing to read. Hydration exports objects only (`crates/core/src/git/odb.rs:144-157`, `:407`), never refs, so nothing repairs it.
- **Target structure:** one function `sync_disk_ref` in `crates/core/src/tools/git.rs` performs the write, and on failure emits one warning. The three sites call it. `reconcile_incoming_tags` calls it only for a name accepted by `Reference::is_valid_name`; for any other name it emits the same warning and skips the write.
- **Observable behavior:** unchanged. The call still succeeds, the index row is still written first, and the response is the same (DEC-001).

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/tools/git.rs` | add `sync_disk_ref`; `branch_create` calls it | 1 site (`:2504`) |
| `crates/core/src/tools/git_tags.rs` | `tag_create` and `reconcile_incoming_tags` call it; import | 2 sites (`:158`, `:449`) |
| `.agent_docs/git.md` | the sentence at `:942` that names BL-0095 now states the warning | 1 |
| `sdlc/backlog/BL-0095-*.md`, `BL-0105-*.md` | deleted in the commit that lands this document | 2 |

**Non source occurrences:** `.env.example`, CI, `Makefile`, container files, scripts: none (rg for `repo.reference` outside `crates/` and `.agent_docs/` returns 0 hits).
**Contracts touched:** none public. `sync_disk_ref` is internal to the crate.
**Out of scope:** the other 10 `let _ = repo.reference(` sites in `crates/core/src/tools/git.rs` (`:2333`, `:2840`, `:4200`, `:4274`, `:4907`, `:4941`, `:5645`, `:6034`, `:6952`, `:6986`); recounted: 13 sites in total (rg `let _ = repo\.reference\(` over `crates/core/src`, includes the 3 above). They become BL-0121 (DEBT) in the commit of this document.
**Guardrail coverage:** the existing git lib tests (ASSUMED: count not recounted; the command below runs them all) plus `ref_name_validator_matches_libgit2` (`git.rs:16783`). **Test command:** `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`
**Total:** 2 source files, 3 sites, plus 1 doc line, all recounted.

## 3. Class sweep
Not applicable (`Security: n/a`, no vulnerability class).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: one function owns the on disk ref write
> The git tools SHALL write an on disk ref after an index ref write through `sync_disk_ref` at the three sites of Section 2.
- **Old form:** `let _ = repo.reference(<ref>, <oid>, true, "<message>");`
- **New form:** `sync_disk_ref(&repo, <ref>, <oid>, "<tool>");` where `<tool>` is `git.branch_create`, `git.tag_create` or `git.remote_fetch`; the reflog message stays `"mcp-fs <tool>"`, unchanged.
- **Applies to:** `git.rs:2504`, `git_tags.rs:158`, `git_tags.rs:449`.
- **Casing convention:** snake_case functions, `git.<verb>` tool names, as in the codebase.

#### DR-002 [EARS-E]: a failed write is reported
> WHEN the on disk ref write fails THE git tools SHALL log one warning that carries the fields `tool`, `ref` and `error`, with the message `on disk ref write failed`, and SHALL return the same result as when the write succeeds.

#### DR-003 [EARS-O]: a name libgit2 would rewrite is not written
> IF `Reference::is_valid_name(<ref>)` is false for a ref that `reconcile_incoming_tags` is about to write THEN THE git tools SHALL skip the on disk write, keep the index row, and log the warning of DR-002 with `error` set to `invalid reference name`.

#### DR-004 [EARS-UB]: no silent success
> The git tools SHALL NOT discard the result of an on disk ref write at the three sites of Section 2.

### 4.2 Invariance
#### DR-005 [EARS-U]: behavior invariance
> The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract moves.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `sync_disk_ref` | function | no | introduced | n/a | n/a |

**Dual maintenance semantics:** none. The warning is one `tracing::warn!` event per failed write.

## 6. Decisions
- **DEC-001:** a failed on disk write does not fail the call. **Alternatives considered:** fail the call and roll the index row back (rejected: the index is the source of truth, a rollback of a committed row plus a new error contract is a behavior change outside a debt; decided by the owner on 2026-11-02). **Rationale:** the index is authoritative; the on disk repository is a cache. **Implemented by:** DR-002, DR-004. **Evidence:** `git_tags.rs:154-158` (existing comment: "The index row is the source of truth").
- **DEC-002:** scope is the 3 sites of BL-0095 and BL-0105. **Alternatives considered:** all 13 sites (rejected by the owner's default; the other 10 become BL-0121). **Implemented by:** n/a, no code impact.
- **DEC-003:** BL-0105's `tag_create` half is already done: the name is refused before any write (`git_tags.rs:46`, `git.rs:2372-2395`). Only `reconcile_incoming_tags` remains. **Implemented by:** DR-003.
- **Assumption:** whether libgit2 can hand `reconcile_incoming_tags` a name such as `a//b` from a remote advertisement is ASSUMED (not verified); DR-003 is a guard that holds either way.

## 7. Declared breaks
None.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:
    export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace

### 8.2 Behavior tests (in `crates/core/src/tools/git.rs`, using the existing `CapturedLog`, `git.rs:23637`)
#### DT-001: branch_create survives a blocked on disk write
- **Validates:** DR-002, DR-004, DR-005
- Given a project with one commit and a file `refs/heads/feat.lock` created in its on disk repository, When `git.branch_create` is called with `feat`, Then the call returns `Ok`, the index holds `refs/heads/feat`, and the captured log holds one line with `on disk ref write failed`, `git.branch_create` and `refs/heads/feat`.
#### DT-002: tag_create survives a blocked on disk write
- **Validates:** DR-002, DR-004. Same, with `refs/tags/v1.lock`, tool `git.tag_create`, tag `v1`.
#### DT-003: the reconcile skips a rewritable name
- **Validates:** DR-003. Given an incoming tag named `a//b`, When `reconcile_incoming_tags` runs, Then the index holds `refs/tags/a//b`, no `refs/tags/a/b` exists in the on disk repository, and the log holds `invalid reference name`.
#### DT-004: a healthy write is silent
- **Validates:** DR-002. Given a normal `branch_create`, `tag_create` and reconcile, Then the captured log holds no `on disk ref write failed`.
- **Red evidence:** DT-001 to DT-003 fail first (no warning exists today). DT-004 passes on arrival; named mutation: `sync_disk_ref` always warns, DT-004 must go red.

### 8.3 Structural tests
#### DT-010: no discarded ref write at the three sites
- **Asserts:** `git_tags.rs` contains no `let _ = repo.reference(`; the body of `branch_create` has none.
- **Method:** a unit test reading the two source files with `include_str!` and asserting on the extracted function bodies.
#### DT-011: one writer
- **Asserts:** outside `sync_disk_ref`, the three sites contain no `.reference(` call. **Method:** same as DT-010.

### 8.4 Existing tests to modify
None.

## 9. Implementation order
1. Tests DT-001 to DT-004, DT-010, DT-011 (red); suite green.
2. Add `sync_disk_ref`, switch the three sites; guard in the reconcile; suite and new tests green.
3. Update `.agent_docs/git.md:942`; `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none.
