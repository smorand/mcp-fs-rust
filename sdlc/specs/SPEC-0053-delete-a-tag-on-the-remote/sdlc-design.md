# SPEC-0053: Delete a tag on the remote, design

> Spec: sdlc/specs/SPEC-0053-delete-a-tag-on-the-remote/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-25
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024 workspace (`crates/core`, `crates/mcp-fs`, `crates/agent`) | per `Cargo.toml` | DEC-001 |
| Git engine | git2 (libgit2) | 0.21 (`Cargo.toml:63`) | DEC-001, DEC-004 |
| MCP | rmcp | 3.5.0 (`Cargo.toml:57`) | DEC-001, DEC-002 |

Skills loaded: `rust` (via `load_dynamic_context`): logic in `crates/core`, `ToolError` constructors, no `unwrap` on fallible paths, `make check` as the gate. No new dependency, so no context7 lookup was needed.

## 2. Approaches
### Chosen: a separate tool over the existing push core
A new tool `git.remote_delete_tag` reuses the single listing and the push core of tag publication: one listing connection decides every refusal, one delete push (with a lease) does the work. Pros: one confirmation rule for every destructive remote tag action, no duplicated credential or deadline code, the local deletion contract is untouched. Cons: a new tool (golden and counts). Affected scope: `tools/git_tags.rs`, `git/remote.rs`, the contract files.
### Rejected: a flag on an existing tool
A `delete` flag on `git.remote_push_tag` or a `remote` flag on `git.tag_delete` (DEC-002).
### Rejected: a provider API call
A REST deletion through the hosting provider: provider specific and a new dependency surface (DEC-004).

## 3. Architecture
```
git.remote_delete_tag (mcp/server.rs)
   -> tool_remote_delete_tag (git_tags.rs): open (member), remote lookup, credential
   -> delete_remote_tag: trace_remote_operation span, write lock, on_git_thread
        -> list_advertised_refs (git/remote.rs): tag state, peeled commit
        -> checks: absent, no commit, differs (stated_commit_matches)
        -> delete_tag_on_remote: push_refs with refspec :refs/tags/<n> and a lease
   -> record_audit after Ok only
```
Bounded contexts: Remote exchange = the two files above; Audit = `safety.record_audit`. Current structure: publication at `git_tags.rs:534-726`, push core at `git/remote.rs:609`, tag push at `:696`, listing at `:793`.

## 4. Data model
No entity, no migration. A remote deletion changes nothing in the database: local tags and refs are untouched (SPEC-0052/FR-NEW-005).

## 5. Contracts and interfaces
Frozen text and shapes are in DEC-002, DEC-005 and DEC-014 and are repeated in the stories that build them.

**`git.remote_delete_tag`**: parameters `mount_id`, `tag`, `remote` (default `origin`), `expected_remote_commit` (default empty); result `{mount_id, tag, remote, deleted_commit, auth}`.

| Situation | Code | Text |
|---|---|---|
| not a member | `ERR_FORBIDDEN` | existing `'<person>' is not a member of '<project>'` |
| no remote at all | `ERR_NOT_FOUND` | `Cannot delete tag '<n>': this project tracks no remote.` |
| named remote not tracked | `ERR_NOT_FOUND` | `Cannot delete tag '<n>': this project does not track a remote named '<r>'.` |
| credential, connection, timeout | as publishing | existing texts |
| remote tag absent | `ERR_NOT_FOUND` | `Remote tag '<n>' does not exist.` |
| no commit stated | `ERR_INVALID_ARGUMENT` | `Remote tag '<n>' points at <7>. State that commit to confirm its deletion.` |
| commit differs or moved | `ERR_NO_CLOBBER` | `Remote tag '<n>' has moved to <7> since you last saw it. Fetch, then confirm again.` |
| remote declines | `ERR_INTERNAL_ERROR` | `Remote refused to delete tag '<n>': <reason>` |

Audit: action `git.tag_remote_delete`, path `/`, `deleted tag '<n>' on remote '<r>' (was at <full 40>)`. Config keys: none.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git_tags.rs` | modify | FR-NEW-001..015 | 1, 2, 3 |
| `crates/core/src/git/remote.rs` | modify (delete form, declined mapping) | FR-NEW-001, 006 | 1, 2 |
| `crates/core/src/mcp/server.rs` | modify (tool, args, counts) | FR-NEW-001 | 1 |
| `crates/core/src/tools/git.rs` | modify (registry entry, helpers, 52 tests) | all | 1, 2, 3, 4 |
| `tool-contract-golden.json, TOOL_CONTRACT.txt` | regenerate, modify | FR-NEW-001 | 1 |
| `crates/core/src/tools/{contract_golden,all,tool_surface_e2e}.rs, crates/agent/src/mcp.rs` | modify (counts) | FR-NEW-001 | 1 |
| `AGENTS.md, .agent_docs/{tools,git,git-divergences}.md` | modify | docs | 1, 4 |

Affected existing tests: none modified (DEC-016); counts, registry lengths and parity docs change as for every new tool. Affected documentation: listed in the stories.

## 7. Compatibility and risks
- No consumer sees a change: a new tool, no key removed, no behaviour of an existing tool altered.
- Risks: (1) a delete refspec through the local file transport and libgit2's reading of the pre-push value for a delete (DEC-004): verified first by E2E-001 in story 1, fallback: read the remote tag from the listing and compare only. (2) A remote declining a deletion and a connection dropping after the remote applied it cannot be reproduced with a file remote: test seams (DEC-015). (3) An in-process write lock does not protect two server processes: the lease does (DEC-008, DEC-009).
- Ordering: story 1 owns the contract change and the count edits; stories 2 and 3 add no tool.
- Rollback: revert the commits; no data to undo. Dependencies added or removed: none.

## 9. Test strategy
All 52 acceptance tests are functional e2e tests through the real functions of the crate against a real bare remote (`file://`) reached through a direct helper (the registered tool refuses `file://`); non member and expired credential go through the registered tool. Seams per DEC-015. Test data: generated by the existing helpers; none user provided.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_001_*` | US-0092 |
| E2E-002 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_002_*` | US-0092 |
| E2E-003 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_003_*` | US-0092 |
| E2E-004 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_004_*` | US-0092 |
| E2E-005 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_005_*` | US-0092 |
| E2E-006 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_006_*` | US-0092 |
| E2E-007 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_007_*` | US-0092 |
| E2E-008 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_008_*` | US-0092 |
| E2E-009 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_009_*` | US-0092 |
| E2E-010 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_010_*` | US-0092 |
| E2E-011 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_011_*` | US-0092 |
| E2E-012 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_012_*` | US-0092 |
| E2E-013 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_013_*` | US-0092 |
| E2E-014 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_014_*` | US-0092 |
| E2E-015 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_015_*` | US-0092 |
| E2E-016 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_016_*` | US-0092 |
| E2E-017 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_017_*` | US-0093 |
| E2E-018 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_018_*` | US-0093 |
| E2E-019 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_019_*` | US-0093 |
| E2E-020 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_020_*` | US-0093 |
| E2E-021 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_021_*` | US-0093 |
| E2E-022 | functional e2e | registered tool, `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_022_*` | US-0092 |
| E2E-023 | functional e2e | registered tool, `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_023_*` | US-0092 |
| E2E-024 | functional e2e | registered tool, `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_024_*` | US-0093 |
| E2E-025 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_025_*` | US-0092 |
| E2E-026 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_026_*` | US-0092 |
| E2E-027 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_027_*` | US-0092 |
| E2E-028 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_028_*` | US-0092 |
| E2E-029 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_029_*` | US-0094 |
| E2E-030 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_030_*` | US-0094 |
| E2E-031 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_031_*` | US-0094 |
| E2E-032 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_032_*` | US-0093 |
| E2E-033 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_033_*` | US-0093 |
| E2E-034 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_034_*` | US-0093 |
| E2E-035 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_035_*` | US-0095 |
| E2E-036 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_036_*` | US-0092 |
| E2E-037 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_037_*` | US-0092 |
| E2E-038 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_038_*` | US-0094 |
| E2E-039 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_039_*` | US-0094 |
| E2E-040 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_040_*` | US-0094 |
| E2E-041 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_041_*` | US-0093 |
| E2E-042 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_042_*` | US-0093 |
| E2E-043 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_043_*` | US-0093 |
| E2E-044 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_044_*` | US-0093 |
| E2E-045 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_045_*` | US-0093 |
| E2E-046 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_046_*` | US-0093 |
| E2E-047 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_047_*` | US-0092 |
| E2E-048 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_048_*` | US-0093 |
| E2E-049 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_049_*` | US-0092 |
| E2E-050 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_050_*` | US-0092 |
| E2E-051 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_051_*` | US-0092 |
| E2E-052 | functional e2e | direct helper `call_delete_remote_tag`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_052_*` | US-0093 |

## 10. Decisions log
- **DEC-001:** No new dependency and no version change: the stack is the manifests' (git2 0.21, `Cargo.toml:63`; rmcp 3.5.0, `Cargo.toml:57`; Rust 2024). Skill loaded: rust. **Obliged by:** n/a (no code impact), records the stack. **Alternatives considered:** none.
- **DEC-002:** One new tool, `git.remote_delete_tag(mount_id, tag, remote="origin", expected_remote_commit="")`, next to `git.remote_push_tag` in the `git.remote_*` family. Tool count 114 to 115, `git.*` family up by one. **Obliged by:** SPEC-0053/FR-NEW-001. **Alternatives considered:** a `delete` flag on `git.remote_push_tag` (an either/or contract on a tool that sends); a `remote` flag on `git.tag_delete` (breaks SPEC-0052/FR-NEW-005, the local deletion never touches the remote).
- **DEC-003:** The engine lives in `crates/core/src/tools/git_tags.rs` beside publication (`tool_remote_delete_tag`, `delete_remote_tag`, `delete_on_repo`); the remote layer in `crates/core/src/git/remote.rs` (`delete_tag_on_remote`). Tests go into the existing tests module of `tools/git.rs` where the bare remote helpers are private. **Obliged by:** SPEC-0053/FR-NEW-001. **Alternatives considered:** a new module (the helpers it needs are in `git_tags.rs`), tests in `git_tags.rs` (helpers not exported).
- **DEC-004:** Deletion reuses the push core of `git/remote.rs:609` (`push_refs`): `PushRefs` gains the delete form, an empty source giving the refspec `:refs/tags/<n>` (never `+`), with the lease on the advertised tag object always set. The refspec is built only for a tag found in the advertised refs, so an invalid name never reaches libgit2. **Obliged by:** SPEC-0053/FR-NEW-001, FR-NEW-003, FR-NEW-004. **Alternatives considered:** `git2::Remote::push` through a second code path (duplicates credentials, deadline and lease handling), a REST call to the provider (provider specific, new dependency surface).
- **DEC-005:** Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then the tag checks (absent, no commit, differs), then the delete push (declined). Codes: not stated `invalid_argument`; differs `no_clobber` (existing `moved`); absent, no remote, named remote not tracked `not_found`; declined and connection failures `internal`. **Obliged by:** SPEC-0053/FR-NEW-007, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-014. **Alternatives considered:** a validation pass up front (would change the order the spec fixes).
- **DEC-006:** The stated commit is checked with the existing `stated_commit_matches` (`git_tags.rs:799`): trimmed, case insensitive, at least 7 hexadecimal characters, a prefix of the remote tag's peeled commit; a blank text is "not stated" (decided before matching). The commits shown in texts use `short7` (`git_tags.rs:812`). **Obliged by:** SPEC-0053/FR-NEW-001, FR-NEW-002, FR-NEW-003. **Alternatives considered:** a new matcher (two rules for one confirmation), `short` of 8 characters (the spec says 7).
- **DEC-007:** The remote tag state comes from the same single listing: the ref's own object and, for an annotated tag, its peeled `^{}` commit; an annotated tag is confirmed by its peeled commit. Absent covers every name not advertised, an empty, blank or invalid name included (shown as given). **Obliged by:** SPEC-0053/FR-NEW-001, FR-NEW-004. **Alternatives considered:** validating the name first (a second message the spec does not define).
- **DEC-008:** When the push reports a lost lease (the remote tag moved or was deleted between listing and push), re list: the tag absent gives `Remote tag '<n>' does not exist.`, otherwise the `moved` refusal with the new commit; the push is never retried. **Obliged by:** SPEC-0053/FR-NEW-003, FR-NEW-011. **Alternatives considered:** retrying the push with the new oid (would delete a tag the member never confirmed).
- **DEC-009:** The whole operation holds the repository `write_lock` (as publication does, `git_tags.rs:551`) so two members of one server serialize; the lease covers other processes. `reject_if_operation_in_progress` is NOT called: a paused merge, rebase, cherry pick or revert does not block the deletion. **Obliged by:** SPEC-0053/FR-NEW-010, FR-NEW-011. **Alternatives considered:** no lock (relies only on the lease: the loser would get a lease error instead of the absent text).
- **DEC-010:** The audit line is written with `safety.record_audit(person, project, "git.tag_remote_delete", "/", "deleted tag '<n>' on remote '<r>' (was at <40>)")` only after the push returned Ok; the operation runs inside `trace_remote_operation` (span, no audit), never `run_remote_operation`. The result is `{mount_id, tag, remote, deleted_commit, auth}` with the full commit id. A push that fails after the remote applied it returns an error, so no line is written. **Obliged by:** SPEC-0053/FR-NEW-008, FR-NEW-013, FR-NEW-015. **Alternatives considered:** `run_remote_operation` (audits failures, forbidden by FR-NEW-008).
- **DEC-011:** `remote` defaults to `origin` through the serde default, as in `git.remote_push_tag`; a blank text is a given name. Remote lookup: `list_remotes()` empty gives the "tracks no remote" text, non empty without the name gives "does not track a remote named '<r>'" with the name as given, case sensitive. **Obliged by:** SPEC-0053/FR-NEW-005, FR-NEW-014. **Alternatives considered:** one text for both cases (untrue when other remotes exist).
- **DEC-012:** A per ref rejection from the remote (`PushFailure::Rejected`) becomes `Remote refused to delete tag '<n>': <reason>` through a new helper in `git/remote.rs`; `remote_refused_tag` (`git/remote.rs:784`) words a name refusal and is not reused. **Obliged by:** SPEC-0053/FR-NEW-006. **Alternatives considered:** reusing `remote_refused_tag` (wrong sentence).
- **DEC-013:** Connection, rate limit, timeout and credential refusals are produced by the code paths publication already uses (`connection_error`, `timeout_error`, `resolve_clone_credential`), unchanged, so one vocabulary covers every remote action. **Obliged by:** SPEC-0053/FR-NEW-012. **Alternatives considered:** new texts (the spec fixes the existing ones).
- **DEC-014:** Contract changes are deliberate: `#[tool]` method and args struct in `mcp/server.rs` with the frozen descriptions, a `#[cfg(test)]` registry entry in `tools/git.rs`, `TOOL_CONTRACT.txt`, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current` and reviewed, every count site raised by one (`contract_golden.rs`, `mcp/server.rs`, `crates/agent/src/mcp.rs:317-319`, `tools/all.rs`, `tools/tool_surface_e2e.rs`, `ALL_GIT_TOOLS`), and the parity docs (`AGENTS.md`, `.agent_docs/tools.md`, `e2e_new_957`/`958`). **Obliged by:** SPEC-0053/FR-NEW-001. **Alternatives considered:** none (the golden test fails otherwise).
- **DEC-015:** Three `#[cfg(test)]` seams carry the cases a local file remote cannot produce: `after_listing(url)` (exists, `git_tags.rs`, reused: moves or deletes the remote tag between listing and push), `forced_rejection(url)` (the delete push reports a per ref rejection, since the local transport runs no hooks) and `fault_after_push(url)` (a successful delete push is reported as a connection error). `closing_remote` simulates a dropped connection. Production builds contain none of them. **Obliged by:** SPEC-0053/FR-NEW-006, FR-NEW-011, FR-NEW-013. **Alternatives considered:** a mock transport (new test infrastructure).
- **DEC-016:** No existing test is modified; the 52 tests are all new. The existing assertions that change are only counts, registry lengths and the parity docs, as for every new tool. **Obliged by:** SPEC-0053/FR-NEW-001. **Alternatives considered:** none.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0092 | delete a confirmed remote tag | P1 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-015 | main | 1 | no |
| US-0093 | remote lookup, connection failures and declined deletions | P1 | FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-009, FR-NEW-012, FR-NEW-013, FR-NEW-014, FR-NEW-015 | US-0092 | 2 | no |
| US-0094 | paused operations and concurrent deletions | P2 | FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-010, FR-NEW-011 | US-0092 | 2 | no |
| US-0095 | documentation, round trip test and spec close | P2 | FR-NEW-001, FR-NEW-004 | US-0093, US-0094 | 2 | yes |
