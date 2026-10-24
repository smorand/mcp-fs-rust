# US-0085: delete a local tag

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0083
> min_tier: 1
> Files touched: 7
> Closes spec: no

## Objective
Add the `git.tag_delete` tool: remove one local tag under the repository write lock, refuse while an operation is paused, give the unknown tag refusal, leave the remote and every other ref untouched, write the audit line.

## Requirements covered
- SPEC-0052/FR-NEW-005: WHEN a member deletes a tag, THE Versioning context SHALL remove that tag only, leaving every commit, every branch, every other tag and the remote untouched.
- SPEC-0052/FR-NEW-006 (partial: deletion only): WHILE a merge, rebase, cherry pick or revert is paused, THE Versioning context SHALL refuse to create or delete a tag with the same refusal a branch change gets.
- SPEC-0052/FR-NEW-012 (partial: deletion line): WHEN a tag is created, overwritten, deleted or published, THE Audit context SHALL record the member and one line of text.
- SPEC-0052/FR-NEW-014 (partial: delete): IF the requester is not a member of the project, THEN THE system SHALL refuse to create, delete, publish, list or receive tags with the refusal it gives for any other project action, and reveal no tag name.
- SPEC-0052/FR-NEW-016: WHEN two members delete the same tag at the same time, THE Versioning context SHALL let exactly one succeed and give the other the FR-NEW-005 refusal. Behavioral and technology agnostic. Users: Alice and Bob are members of `proj-a`; Carol is not. Remote: a GitHub Enterprise repository tracked by `proj-a`. `main` is checked out at `a1b2c3d`; `dev` is at `b2c3d4e`; other commits `c3d4e5f`, `d4e5f6a`, `e5f6a7b`.
- SPEC-0052/FR-NEW-020 (partial: deletion order): WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` green; after `git.tag_delete` of `v1.0`, `git.tags` no longer lists it and `git.log` still shows its commit.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0052): Three new MCP tools, `git.tag_create`, `git.tag_delete` and `git.remote_push_tag`; `git.tags` and the fetch, pull and clone results gain additive keys. Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: extra parameters on `git.remote_push` (it requires `branch`, so a tag would make it an either/or contract), one `git.tag` tool with an `action` argument (breaks the one tool one verb convention of the 111 tools). Total 114 tools.
- DEC-008 (SPEC-0052): Refusals are produced in the order of FR-NEW-020 by the order of the calls: `open` (membership), `reject_if_operation_in_progress`, name validation, no commit, target resolution, existing name (checked again under the write lock). Codes: invalid name `invalid_argument`; unknown target, no commit and unknown tag `not_found`; existing name and remote tag refusals `no_clobber`; commit not on the remote `invalid_argument`. Messages are the spec's, verbatim. Obliged by: FR-NEW-020, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-013. Alternatives: new error codes (the 16 codes are a frozen set, `errors.rs:9-24`).
- DEC-009 (SPEC-0052): Create, overwrite and delete run under `entry.write_lock` (the lock `branch_create` and `commit` hold, `tools/git.rs:2121-2135`), so concurrent same-name calls serialise and exactly one wins. Audit goes through `safety.record_audit(person, project, action, "/", detail)` with actions `git.tag_create`, `git.tag_overwrite`, `git.tag_delete`, `git.tag_publish` and the spec's detail lines; full 40 character ids. Obliged by: FR-NEW-012, FR-NEW-015, FR-NEW-016. Alternatives: optimistic retry (rejected, the lock already exists).
- DEC-021 (SPEC-0052): Contract changes are deliberate and reviewed: three new `#[tool]` methods in `mcp/server.rs`, three matching `#[cfg(test)]` registry entries, the `message`, `overwrite`, `expected_remote_commit` parameters with their frozen descriptions copied from `TOOL_CONTRACT.txt`, the `git.tags` description, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the 111 constants raised stepwise to 112, 113, 114 (`tools/contract_golden.rs:146,:253,:309,:323`, `mcp/server.rs:4294,4315,4316`, `crates/agent/src/mcp.rs:319`). Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: none (the golden test fails otherwise).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
**`git.tag_delete`** (new): parameters `mount_id` (string, required), `name` (string, required: "Name of the local tag to delete, without the refs/tags/ prefix."). Returns `{"mount_id","name","sha":<commit the tag pointed at>}`.
Refusal: `Tag '<n>' does not exist.` (code `not_found`). Order: member, paused operation, unknown tag.
Audit (action `git.tag_delete`): `deleted tag '<n>' (was at <full commit id>)`.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (`tool_tag_delete` and engine) |
| `crates/core/src/tools/git.rs` | modify (`#[cfg(test)]` registry entry, tests) |
| `crates/core/src/mcp/server.rs` | modify (`GitTagDeleteArgs`, `git_tag_delete`, count 112 to 113) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify |
| `crates/core/src/tools/contract_golden.rs` | modify (count) |
| `crates/agent/src/mcp.rs` | modify (count) |

### Existing patterns
Existing pattern, a local ref-mutating tool: `tool_branch_delete` at `crates/core/src/tools/git.rs:1255` (calls `open`, then `reject_if_operation_in_progress`, then `on_git_thread`), engine `branch_delete` at `:2476` (validate, `entry.write_lock.lock().await`, `entry.db`, audit at `:2521`). Declaration pattern: `GitBranchDeleteArgs` at `crates/core/src/mcp/server.rs:900` and `#[tool(name = "git.branch_delete", ...)]` calling `crate::tools::git::tool_branch_delete(self.git_ctx(), a, None)` then `to_call_result("git.branch_delete", out)`.

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-026: Delete a local tag
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_026_*`
- **Category:** journey (happy). **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** tags `v1.0` (`a1b2c3d`), `v2.0` (`c3d4e5f`), `v3.0`. **When** Alice deletes `v1.0`. **Then** the list shows `v2.0` and `v3.0` only. **And** commit `a1b2c3d` is still readable and `main` is unchanged.
- **Priority:** Critical

### SPEC-0052/E2E-027: Delete an unknown tag
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_027_*`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** no `v9.9`. **When** Alice deletes `v9.9`. **Then** the refusal is `Tag 'v9.9' does not exist.` **And** the list is unchanged.
- **Priority:** High

### SPEC-0052/E2E-028: Delete the last tag, twice
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_028_*`
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** only `v1.0` exists. **When** Alice deletes it, then deletes it again. **Then** the first succeeds and the list is empty; the second is refused with `Tag 'v1.0' does not exist.`
- **Priority:** Medium

### SPEC-0052/E2E-029: Deletion refused during a paused operation
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_029_*`
- **Category:** state transition. **Scenario:** SC-003. **Requirements:** FR-NEW-006
- **Given** a cherry pick is paused and `v1.0` exists. **When** Bob deletes `v1.0`. **Then** the refusal names the cherry pick in progress and how to finish or abandon it. **And** `v1.0` still exists.
- **Priority:** High

### SPEC-0052/E2E-030: Non member cannot delete
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_030_*`
- **Category:** security. **Scenario:** SC-003. **Requirements:** FR-NEW-014, FR-NEW-005
- **Given** Carol is not a member and `v1.0` exists. **When** Carol deletes `v1.0`. **Then** she receives the usual refusal. **And** `v1.0` remains and no audit entry was added.
- **Priority:** Critical

### SPEC-0052/E2E-031: Deletion is local and audited
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_031_*`
- **Category:** side effect. **Scenario:** SC-003. **Requirements:** FR-NEW-005, FR-NEW-012
- **Given** `v1.0` at `a1b2c3d` exists locally and on the remote. **When** Alice deletes `v1.0`. **Then** the remote still has `v1.0`. **And** an audit entry by Alice reads `deleted tag 'v1.0' (was at <full id of a1b2c3d>)`.
- **Priority:** High

### SPEC-0052/E2E-032: Concurrent deletion
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_032_*`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-016, FR-NEW-005
- **Given** `v1.0` and `v2.0` exist. **When** Alice and Bob delete `v1.0` at the same time. **Then** one succeeds and the other receives `Tag 'v1.0' does not exist.` **And** `v2.0` is untouched.
- **Priority:** Medium

### SPEC-0052/E2E-053: Delete in a project with no commit
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_053_*`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-005
- **Given** `proj-empty` has no commit and no tag. **When** Alice deletes `v1.0`. **Then** the refusal is `Tag 'v1.0' does not exist.` **And** nothing changes.
- **Priority:** Medium

### SPEC-0052/E2E-065: Concurrent deletion of different tags
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_065_*`
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-016
- **Given** `v1.0` and `v2.0` exist. **When** Alice deletes `v1.0` and Bob deletes `v2.0` at the same time. **Then** both are deleted and the list is empty.
- **Priority:** Medium

### SPEC-0052/E2E-066: One audit entry for a concurrent deletion
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_066_*`
- **Category:** side effect. **Scenario:** SC-003. **Requirements:** FR-NEW-016, FR-NEW-012
- **Given** `v1.0` exists. **When** Alice and Bob delete `v1.0` at the same time. **Then** exactly one deletion entry exists. **And** it names the member whose deletion succeeded.
- **Priority:** Medium

## Constraints
- Local only: never contact the remote.
- Delete the db row and the on-disk ref (as `branch_delete` does); keep commits and other tags.
- Counts are 112 to 113 if this story lands before the publish story; use the live number found, never a guess.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0085: delete a local tag`, body `Spec: SPEC-0052` and the requirement ids.
