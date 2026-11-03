# Route the other silent on disk ref writes through sync_disk_ref: technical debt design

> Id: SPEC-0077
> Nature: DEBT
> Depth: S
> Depth evidence: 1 file, 10 call sites in 8 functions, no public contract change, no persisted change
> Status: specified
> Generated on: 2026-11-02
> Behavior change: none
> From backlog: BL-0121
> Depends on: SPEC-0071
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: ten silent on disk ref writes
- **Current structure:** ten `let _ = repo.reference(...)` calls in `crates/core/src/tools/git.rs` drop a libgit2 failure: `commit` `:2333`, `set_branch_ref` `:2870`, `merge_inner` `:4230` and `:4304`, `land_replay` `:4937`, `pause_replay` `:4971`, `merge_resolve_inner` `:5675`, `replay_abort` `:6064`, `pull_branch_inner` `:7040` and `:7074` (recounted with `rg -n 'let _ = repo\.reference\(' crates/core/src/tools/git.rs`: 12 matches, 2 of them string literals in `dt_0071_010` at `:23962` and `:23964`).
- **Cost paid today:** the index and the on disk repository can disagree with no trace; hydration exports objects only (`crates/core/src/git/odb.rs:144-157`) so nothing repairs a ref. SPEC-0071 fixed the same for `branch_create`, `tag_create` and the tag reconcile and left these ten (BL-0121).
- **Target structure:** each site calls `sync_disk_ref(repo, ref_name, oid, tool)` (`git.rs:2398`), which writes with the reflog text `mcp-fs <tool>` and logs one `on disk ref write failed` warning on failure.
- **Observable behavior:** unchanged (the call still succeeds; Section 7 for one reflog text).

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/tools/git.rs` | ten sites call `sync_disk_ref`; tests DT-001 to DT-007, DT-010, DT-011 | 10 sites |
| `crates/core/src/tools/git.rs:3716` | checked write of `push_branch`, unchanged (its error fails the push; DEC-005) | 1 site, not changed |
| `.agent_docs/git.md` | the sentence naming BL-0121 ("Ten more sites ... still discard the write") states the family is complete | 1 |
| `sdlc/backlog/BL-0121-*.md` | deleted in the landing commit | 1 |

**Non source occurrences:** none (`rg 'repo\.reference\(' --glob '!crates/**' --glob '!sdlc/**'` finds only `.agent_docs/git.md`).
**Contracts touched:** none public. `sync_disk_ref` is crate internal.
**Guardrail coverage:** the git lib tests, including the merge, rebase, cherry pick, pull and commit suites (ASSUMED: count not recounted; the command below runs them). **Test command:** `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`
**Total:** 1 source file, 10 sites changed, 1 checked site left as is (11 non test matches of `.reference(` besides `sync_disk_ref`'s own), 1 doc line.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: one writer for every on disk ref
> The git tools SHALL write an on disk ref after an index ref write only through `sync_disk_ref`.
- **Old form:** `let _ = repo.reference(<ref>, <oid>, true, "<message>");` (and `repo.reference(` over several lines at `:7040`).
- **New form:** `sync_disk_ref(<repo>, <ref>, <oid>, "<tool>");` where `<tool>` is the old message without its `mcp-fs ` prefix: `git.commit`, `git.merge fast-forward`, `git.merge`, `git.rebase pause`, `git.merge_resolve`, `replay abort`, `git.remote_pull merge`, `git.remote_pull`; at `set_branch_ref` and `land_replay` the `tool` argument the function already holds.
- **Applies to:** the ten sites of Section 2.
- **Casing convention:** snake_case functions, `git.<verb>` tool names, as in the codebase.

#### DR-002 [EARS-E]: a failed write is reported
> WHEN the on disk ref write of any of the ten sites fails THE git tools SHALL log one warning `on disk ref write failed` carrying `tool`, `ref` and `error`, and SHALL return the same result as when the write succeeds.

#### DR-003 [EARS-UB]: no discarded write
> The non test code of `crates/core/src/tools/git.rs` SHALL NOT contain `let _ = repo.reference(`, and SHALL contain `.reference(` only inside `sync_disk_ref` and at the checked write of `push_branch` (`:3716`), whose error already fails the push.

### 4.2 Invariance
#### DR-004 [EARS-U]: behavior invariance
> The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract moves.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `sync_disk_ref` | function | no | already introduced by SPEC-0071 | n/a | n/a |

**Dual maintenance semantics:** none.

## 6. Decisions
- **DEC-001:** every site calls the SPEC-0071 helper with the old message minus `mcp-fs `. **Alternatives considered:** a second helper for non branch refs (rejected: one writer is the point). **Rationale:** the reflog text stays identical at nine sites. **Implemented by:** DR-001. **Evidence:** `git.rs:2398-2403`.
- **DEC-002:** `land_replay` wrote its raw `tool` as reflog text (`git.rs:4937`); through the helper it reads `mcp-fs <tool>` like every other site. **Alternatives considered:** a helper variant taking the full message (rejected: one more form for a text a bare repository does not keep). **Implemented by:** DR-001. **Evidence:** `git.rs:4937`.
- **DEC-003:** `pause_replay` writes `refs/mcp-fs/rebase-head`, not a branch; it takes the same helper, so its failure is logged too. **Implemented by:** DR-001, DR-002. **Evidence:** `git.rs:4521`, `:4971`.
- **DEC-004:** one story: ten one line changes and nine tests in one file. **Obliged by:** n/a (no code impact): partitioning only.
- **DEC-005:** the checked write at `git.rs:3716` stays as is: its failure must fail `git.remote_push`, which names the branch by ref. **Alternatives considered:** routing it through `sync_disk_ref` (rejected: a refusal would become a warning, a behavior change). **Implemented by:** DR-003. **Evidence:** `git.rs:3711-3721`.
- **Assumption:** SPEC-0071's block mechanism (an empty `<ref>.lock` file) makes libgit2 refuse each of these writes; the RED run shows it per site.

## 7. Declared breaks
None. One internal difference: the on disk reflog text at `land_replay` gains the `mcp-fs ` prefix (DEC-002); a bare repository keeps no reflog by default (ASSUMED: `core.logAllRefUpdates` unset).

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:
    export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace

### 8.2 Behavior tests (in `crates/core/src/tools/git.rs`, with `block_disk_ref` and `disk_ref_warnings` of SPEC-0071)
#### DT-001: commit survives a blocked write
- **Validates:** DR-002. Given a project and the file `refs/heads/main.lock` in its on disk repository, When `git.commit` runs, Then it returns `Ok` and one warning for `refs/heads/main` names `git.commit`.
#### DT-002: merge survives a blocked write
- **Validates:** DR-002. Given a fast forward merge and a true merge (two cases), each with the target branch's lock file in place, Then both return `Ok` and each records one warning naming `git.merge fast-forward` and `git.merge`.
#### DT-003: merge resolve survives a blocked write
- **Validates:** DR-002. Given a conflicted merge, the lock on the branch, When it is resolved, Then `Ok` and one warning naming `git.merge_resolve`.
#### DT-004: pull survives a blocked write
- **Validates:** DR-002. Given a fast forward pull and a merging pull (two cases) with the branch lock in place, Then `Ok` and one warning each naming `git.remote_pull` and `git.remote_pull merge`.
#### DT-005: rebase paths survive a blocked write
- **Validates:** DR-002. Given a rebase that lands, one that pauses on a conflict and an abort, each with the matching lock in place (the branch, `refs/mcp-fs/rebase-head`, the branch), Then each returns as without the block and records one warning naming the tool.
#### DT-006: set_branch_ref survives a blocked write
- **Validates:** DR-002. Given the lock on the branch, When a branch reset runs, Then `Ok` and one warning for that ref.
#### DT-007: healthy writes are silent
- **Validates:** DR-002. The operations of DT-001 to DT-006 with no lock file record no warning for their refs.
- **Red evidence:** DT-001 to DT-006 fail before the change (no warning is recorded). DT-007 passes on arrival; named mutation: `sync_disk_ref` always warns, DT-007 must go red.

### 8.3 Structural tests
#### DT-010: no discarded write
- **Asserts:** the non test part of `git.rs` (text before `#[cfg(test)]\nmod tests`) contains no `let _ = repo.reference(`. **Method:** `include_str!("git.rs")` sliced at the test module.
#### DT-011: one writer
- **Asserts:** that same slice contains `.reference(` exactly twice: once inside `sync_disk_ref`, once in `push_branch` followed by `.map_err(`. **Method:** count and slice as DT-010.

### 8.4 Existing tests to modify
None. (`dt_0071_010` and `dt_0071_011` keep passing: they slice named functions and `git_tags.rs`.)

## 9. Implementation order
1. Tests DT-001 to DT-007, DT-010, DT-011 (DT-001 to DT-006, DT-010, DT-011 red); suite green otherwise.
2. Switch the ten sites to `sync_disk_ref`; tests green, suite green.
3. Update `.agent_docs/git.md`; `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none.
