# US-0083: create a plain tag and list tags with kind and order

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 8
> Closes spec: no

## Objective
Add the `git.tag_create` tool for plain tags (target resolution, name rules, overwrite, refusal order, audit, concurrency) and make `git.tags` list `kind`, the peeled commit in `sha`, and ascending name order. This is the first of three new tools; it also introduces the module `tools/git_tags.rs`.

## Requirements covered
- SPEC-0052/FR-MOD-001 (partial: kind lightweight, commit, ascending order): The Versioning context SHALL show, for each listed tag, its name, its commit, its kind (`lightweight` or `annotated`) and, for an annotated tag, its message and tagger.
- SPEC-0052/FR-NEW-001: WHEN a member creates a tag with a name and no message, THE Versioning context SHALL create a plain tag at the checked out commit, or at the branch, tag or commit the member names.
- SPEC-0052/FR-NEW-003: IF a tag with the requested name exists and the member has not confirmed overwriting, THEN THE Versioning context SHALL refuse and change nothing.
- SPEC-0052/FR-NEW-004: IF the tag name is invalid or the target is unknown, THEN THE Versioning context SHALL refuse and create nothing.
- SPEC-0052/FR-NEW-006 (partial: creation only): WHILE a merge, rebase, cherry pick or revert is paused, THE Versioning context SHALL refuse to create or delete a tag with the same refusal a branch change gets.
- SPEC-0052/FR-NEW-012 (partial: creation and overwrite lines): WHEN a tag is created, overwritten, deleted or published, THE Audit context SHALL record the member and one line of text.
- SPEC-0052/FR-NEW-013 (partial: refusal without a target): IF the project has no commit and the member creates a tag without a target, THEN THE Versioning context SHALL refuse with `Cannot tag: this project has no commit yet.`
- SPEC-0052/FR-NEW-014 (partial: create and list): IF the requester is not a member of the project, THEN THE system SHALL refuse to create, delete, publish, list or receive tags with the refusal it gives for any other project action, and reveal no tag name.
- SPEC-0052/FR-NEW-015: WHEN two members create the same tag name at the same time, THE Versioning context SHALL let exactly one succeed and give the other the FR-NEW-003 refusal.
- SPEC-0052/FR-NEW-020 (partial: creation order): WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list.
- SPEC-0052/FR-NEW-021: WHEN a member names a target, THE Versioning context SHALL resolve it in this order: a branch of that exact name, then a tag of that exact name, then a commit whose full id equals it, then the only commit whose id begins with it when the text is at least 7 characters.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` is green; `cargo test -p mcp-fs-core --lib tool_contract_golden` is green after regenerating the golden with `MCPFS_REWRITE_TOOL_CONTRACT=1` and reviewing the diff; a created tag read back through `git.tags` shows `kind: "lightweight"` and the commit in `sha`.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0052): Three new MCP tools, `git.tag_create`, `git.tag_delete` and `git.remote_push_tag`; `git.tags` and the fetch, pull and clone results gain additive keys. Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: extra parameters on `git.remote_push` (it requires `branch`, so a tag would make it an either/or contract), one `git.tag` tool with an `action` argument (breaks the one tool one verb convention of the 111 tools). Total 114 tools.
- DEC-003 (SPEC-0052): Tag engines live in a new module `crates/core/src/tools/git_tags.rs` (precedent: `git_pr.rs`, `git_auth.rs`, registered in `tools/mod.rs:55-56`); the helpers it needs from `tools/git.rs` (`open`, `hydrate`, `on_git_thread`, `parse_oid`, `reject_if_operation_in_progress`, `validate_branch_name`, `short`) become `pub(crate)`. New tests go into the existing `#[cfg(test)] mod tests` of `tools/git.rs` because its bare remote helpers are private there. Obliged by: all requirements (structure). Alternatives: code in `git.rs` (20k lines already), tests in `git_tags.rs` (would need every helper exported).
- DEC-004 (SPEC-0052): A tag is one `git_refs` row `refs/tags/<name>` whose target is the commit sha (plain tag) or the tag object sha (annotated tag); objects reach the blob store through the existing `entry.objects.import_from_repo`. No schema change, no migration. Obliged by: FR-NEW-001, FR-NEW-002. Evidence: `git/db.rs:371` `set_ref`, `git/odb.rs:188`. Alternatives: a dedicated tags table (second source of truth, rejected).
- DEC-006 (SPEC-0052): The tagger is built exactly as a commit author is (`tools/git.rs:2137-2142`): name = the person id before `@`, email = the person id. A listing entry for an annotated tag carries `kind: "annotated"`, `message`, `tagger` (name), `tagger_email`, `tagged_at` (UTC `%Y-%m-%dT%H:%M:%SZ`); a plain tag carries `kind: "lightweight"` and none of the three. `sha` is always the peeled commit, `object` the ref's own target. Obliged by: FR-MOD-001. Alternatives: renaming `sha` (breaks readers, rejected), `commit_json`'s date format (not the format FR-MOD-001 spells).
- DEC-007 (SPEC-0052): Target resolution (FR-NEW-021) is a new function in `git_tags.rs`: `refs/heads/<t>` then `refs/tags/<t>` (peeled to its commit) then a 40 hex id that `find_commit` accepts, then a hex prefix of 7 or more characters through `repo.find_object_by_prefix`, where an ambiguous result and any shorter text mean unknown target. The existing `resolve_ref` (`tools/git.rs:1879`) is not reused: it returns an unverified hex string as is. Obliged by: FR-NEW-021, FR-NEW-004. Alternatives: reuse `resolve_ref` (no existence check, no ambiguity handling).
- DEC-008 (SPEC-0052): Refusals are produced in the order of FR-NEW-020 by the order of the calls: `open` (membership), `reject_if_operation_in_progress`, name validation, no commit, target resolution, existing name (checked again under the write lock). Codes: invalid name `invalid_argument`; unknown target, no commit and unknown tag `not_found`; existing name and remote tag refusals `no_clobber`; commit not on the remote `invalid_argument`. Messages are the spec's, verbatim. Obliged by: FR-NEW-020, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-013. Alternatives: new error codes (the 16 codes are a frozen set, `errors.rs:9-24`).
- DEC-009 (SPEC-0052): Create, overwrite and delete run under `entry.write_lock` (the lock `branch_create` and `commit` hold, `tools/git.rs:2121-2135`), so concurrent same-name calls serialise and exactly one wins. Audit goes through `safety.record_audit(person, project, action, "/", detail)` with actions `git.tag_create`, `git.tag_overwrite`, `git.tag_delete`, `git.tag_publish` and the spec's detail lines; full 40 character ids. Obliged by: FR-NEW-012, FR-NEW-015, FR-NEW-016. Alternatives: optimistic retry (rejected, the lock already exists).
- DEC-010 (SPEC-0052): Listing reads the `git_refs` tag rows, then inside `on_git_thread` after `hydrate` peels each target with the repo (`find_object`, `peel_to_commit`, `find_tag` for annotated) and sorts by name bytes ascending (`Vec<String>` sort is byte order). Stash refs never match `refs/tags/`. Obliged by: FR-MOD-001. Alternatives: parse tag objects by hand from the blob store (brittle).
- DEC-021 (SPEC-0052): Contract changes are deliberate and reviewed: three new `#[tool]` methods in `mcp/server.rs`, three matching `#[cfg(test)]` registry entries, the `message`, `overwrite`, `expected_remote_commit` parameters with their frozen descriptions copied from `TOOL_CONTRACT.txt`, the `git.tags` description, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the 111 constants raised stepwise to 112, 113, 114 (`tools/contract_golden.rs:146,:253,:309,:323`, `mcp/server.rs:4294,4315,4316`, `crates/agent/src/mcp.rs:319`). Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: none (the golden test fails otherwise).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
**`git.tag_create`** (new, annotations as `git.branch_create`): parameters `mount_id` (string, required), `name` (string, required: "Name of the tag to create, without the refs/tags/ prefix."), `target` (string, default `""`: "Branch, tag or commit id the tag points at; defaults to the currently checked out commit."), `overwrite` (bool, default `false`: "Replace a tag of the same name; false refuses when it exists."). Returns `{"mount_id","name","kind":"lightweight","sha":<commit>,"object":<commit>,"overwritten":bool,"previous_sha":<commit, only when overwritten>}`.
**`git.tags`** (changed): each entry `{"name","full_ref","sha":<peeled commit>,"object":<ref target>,"kind"}`, sorted ascending by `name` bytes; description becomes "List all tags with their kind, commit and, for annotated tags, message, tagger and creation time."
Messages (verbatim): `Tag '<n>' already exists. Choose another name or confirm overwriting it.` / `'<n>' is not a valid tag name: <reason>` (reasons of FR-NEW-004, produced by the branch validator with the noun `tag`) / `Cannot tag '<t>': no such branch, tag or commit.` / `Cannot tag: this project has no commit yet.`
Audit (action `git.tag_create` or `git.tag_overwrite`, path `/`): `created tag '<n>' at <full commit id>` / `overwrote tag '<n>' at <new full id> (was <old full id>)`; a confirmed overwrite of an unknown name writes the `created` line.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`pub mod git_tags;`) |
| `crates/core/src/tools/git.rs` | modify (`pub(crate)` helpers, `tool_tags` delegates to the new listing, `#[cfg(test)]` registry entry for `git.tag_create`, new tests `e2e_tag_NNN_*`) |
| `crates/core/src/mcp/server.rs` | modify (`GitTagCreateArgs`, `git_tag_create`, count 111 to 112 at `:4294,:4315,:4316`) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify (new tool, `git.tags` description) |
| `crates/core/src/tools/contract_golden.rs` | modify (count 111 to 112 at `:146,:253,:309,:323`) |
| `crates/agent/src/mcp.rs` | modify (count at `:319`) |

### Existing patterns
Existing pattern, a local ref-mutating tool: `tool_branch_delete` at `crates/core/src/tools/git.rs:1255` (calls `open`, then `reject_if_operation_in_progress`, then `on_git_thread`), engine `branch_delete` at `:2476` (validate, `entry.write_lock.lock().await`, `entry.db`, audit at `:2521`). Declaration pattern: `GitBranchDeleteArgs` at `crates/core/src/mcp/server.rs:900` and `#[tool(name = "git.branch_delete", ...)]` calling `crate::tools::git::tool_branch_delete(self.git_ctx(), a, None)` then `to_call_result("git.branch_delete", out)`.

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-001: Create a plain tag at the checked out commit
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_001_*`
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** Alice is a member of `proj-a`, `main` is checked out at `a1b2c3d`, no tag exists. **When** Alice creates `v1.0` with no target and no message. **Then** `v1.0` exists at `a1b2c3d`. **And** the list shows `v1.0`, commit `a1b2c3d`, kind `lightweight`, no message.
- **Priority:** Critical

### SPEC-0052/E2E-002: Create a tag at a named branch
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_002_*`
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** `dev` is at `b2c3d4e`, `main` checked out at `a1b2c3d`. **When** Alice creates `release-2026` targeting `dev`. **Then** `release-2026` is at `b2c3d4e`. **And** the checked out commit is still `a1b2c3d`.
- **Priority:** High

### SPEC-0052/E2E-003: Create a tag at a tag or a commit
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_003_*`
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** `v1.0` is at `a1b2c3d`. **When** Alice creates `v1.0-copy` targeting `v1.0`, then `v1.1` targeting `c3d4e5f`. **Then** `v1.0-copy` is at `a1b2c3d` and `v1.1` at `c3d4e5f`. **And** both are `lightweight`.
- **Priority:** Medium

### SPEC-0052/E2E-006: Existing name refused without confirmation
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_006_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-012
- **Given** `v1.0` is at `a1b2c3d` and `main` has moved to `c3d4e5f`. **When** Alice creates `v1.0` without confirming. **Then** the refusal is `Tag 'v1.0' already exists. Choose another name or confirm overwriting it.` **And** `v1.0` is still at `a1b2c3d` and no audit entry was added.
- **Priority:** Critical

### SPEC-0052/E2E-007: Overwrite confirmed moves the tag
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_007_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** `v1.0` at `a1b2c3d`, `main` at `c3d4e5f`. **When** Alice creates `v1.0` confirming the overwrite. **Then** `v1.0` is at `c3d4e5f`. **And** the number of tags is unchanged.
- **Priority:** High

### SPEC-0052/E2E-009: Name with a space
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_009_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** Alice is a member. **When** Alice creates `my tag`. **Then** the refusal is `'my tag' is not a valid tag name: it contains the forbidden character ' '`. **And** for the name `a b~` the reason names the space (the leftmost forbidden character), and for a name holding a tab between `a` and `b` the reason is `it contains the forbidden character '\t'`. **And** no tag is created.
- **Priority:** High

### SPEC-0052/E2E-010: Other invalid names
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_010_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** Alice is a member. **When** she tries, one at a time, an empty name, `a..b`, `/v1`, `v1/`, `rel/.hidden`, `v1.lock`. **Then** each is refused with `'<name>' is not a valid tag name: <reason>` where the reasons are, in order: `it is empty`; `it contains '..'`; `it begins or ends with '/'` (for `/v1` and for `v1/`); `one of its '/' separated components begins with '.'`; `it ends with '.lock'`. **And** the list is unchanged after each attempt.
- **Priority:** High

### SPEC-0052/E2E-011: Name length boundary
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_011_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** Alice is a member. **When** she creates a name of `v` plus 254 letters `a` (255 bytes), then a name of 256 bytes, then a name of 100 accented letters `é` (200 bytes) preceded by 101 letters `a` (301 bytes). **Then** the first is created; the second is refused with `it is 256 bytes of UTF-8, over the 255 byte limit` and the third with `it is 301 bytes of UTF-8, over the 255 byte limit`. **And** only one tag exists.
- **Priority:** Medium

### SPEC-0052/E2E-012: Unknown target
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_012_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** nothing is named `nosuchref`. **When** Alice creates `v3.0` targeting `nosuchref`. **Then** the refusal is `Cannot tag 'nosuchref': no such branch, tag or commit.` **And** `v3.0` does not exist.
- **Priority:** High

### SPEC-0052/E2E-013: Unicode names accepted
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_013_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, FR-NEW-001
- **Given** Alice is a member. **When** she creates `été-2026` and `rel/2026-🚀`. **Then** both are created and listed with exactly those names.
- **Priority:** Low

### SPEC-0052/E2E-014: Project with no commit
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_014_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** `proj-empty` has no commit and Alice is a member. **When** Alice creates `v1.0` with no target. **Then** the refusal is `Cannot tag: this project has no commit yet.` **And** no tag exists.
- **Priority:** Medium

### SPEC-0052/E2E-015: Non member cannot create
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_015_*`
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-014, FR-NEW-001
- **Given** Carol is not a member of `proj-a`. **When** Carol creates `v1.0`. **Then** she receives the refusal she gets for any other project action. **And** no tag exists and no audit entry was added.
- **Priority:** Critical

### SPEC-0052/E2E-016: Concurrent creation
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_016_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-015, FR-NEW-003
- **Given** `v5.0` does not exist. **When** Alice (target `a1b2c3d`) and Bob (target `b2c3d4e`) create `v5.0` at the same time without confirming. **Then** exactly one succeeds and the other receives `Tag 'v5.0' already exists. Choose another name or confirm overwriting it.` **And** `v5.0` is at the winner's target and exactly one creation entry exists.
- **Priority:** Critical

### SPEC-0052/E2E-017: Creation refused during a paused merge
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_017_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** a merge of `dev` into `main` is paused on a conflict. **When** Alice creates `v1.0`. **Then** the refusal is the one a branch change gets: it names the merge in progress and how to finish or abandon it. **And** no tag is created.
- **Priority:** High

### SPEC-0052/E2E-018: Creation refused during other paused operations
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_018_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** a paused rebase, then separately a paused cherry pick, then a paused revert. **When** Bob creates `v1.0` in each case. **Then** each refusal names its own operation. **And** after the operation is abandoned, the same creation succeeds.
- **Priority:** Medium

### SPEC-0052/E2E-019: Audit entry on creation and overwrite
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_019_*`
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-012
- **Given** Alice is a member. **When** she creates `v1.0` at `a1b2c3d`, then overwrites it to `c3d4e5f`. **Then** two entries exist, both by Alice: `created tag 'v1.0' at <full id of a1b2c3d>` and `overwrote tag 'v1.0' at <full id of c3d4e5f> (was <full id of a1b2c3d>)`.
- **Priority:** High

### SPEC-0052/E2E-021: Zero and one tag
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_021_*`
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** `proj-a` has no tag. **When** Alice lists tags, creates `v1.0`, lists again. **Then** the first list is empty with no error and the second holds exactly `v1.0`.
- **Priority:** Medium

### SPEC-0052/E2E-023: Non member cannot list
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_023_*`
- **Category:** security. **Scenario:** SC-002. **Requirements:** FR-NEW-014
- **Given** Carol is not a member and `v1.0` exists. **When** Carol lists tags of `proj-a`. **Then** she receives the usual refusal. **And** her answer contains no tag name.
- **Priority:** High

### SPEC-0052/E2E-024: Listing works during a paused operation
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_024_*`
- **Category:** state transition. **Scenario:** SC-002. **Requirements:** FR-NEW-006, FR-MOD-001
- **Given** a merge is paused and `v1.0` exists. **When** Alice lists tags. **Then** `v1.0` is shown with its kind.
- **Priority:** Medium

### SPEC-0052/E2E-025: Listing order
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_025_*`
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** tags `v2.0`, `v10.0`, `Alpha`, `été`, `v1.0` exist. **When** Bob lists tags. **Then** the order is `Alpha`, `v1.0`, `v10.0`, `v2.0`, `été` (ascending, compared byte by byte).
- **Priority:** Medium

### SPEC-0052/E2E-052: Tagging works after the first commit
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_052_*`
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-013, FR-NEW-001
- **Given** `proj-empty` has no commit. **When** Alice commits her first file at `f0a1b2c`, then creates `v0.1` with no target. **Then** `v0.1` exists at `f0a1b2c`.
- **Priority:** Medium

### SPEC-0052/E2E-061: Concurrent creation of different names
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_061_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-015
- **Given** neither `v6.0` nor `v6.1` exists. **When** Alice creates `v6.0` and Bob creates `v6.1` at the same time. **Then** both exist. **And** exactly two creation entries exist.
- **Priority:** Medium

### SPEC-0052/E2E-068: The loser of a concurrent creation can retry under another name
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_068_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-015, FR-NEW-003
- **Given** Alice and Bob created `v5.0` at the same time and Bob received the already exists refusal. **When** Bob creates `v5.1` at `b2c3d4e`. **Then** `v5.1` exists at `b2c3d4e`. **And** `v5.0` is unchanged.
- **Priority:** Medium

### SPEC-0052/E2E-070: Paused operation outranks an invalid name and unknown target
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_070_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-020
- **Given** a merge is paused and Alice is a member. **When** she creates `my tag` targeting `nosuchref`. **Then** she gets the paused merge refusal only. **And** no tag is created.
- **Priority:** High

### SPEC-0052/E2E-079: Overwrite to the same commit is audited
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_079_*`
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-012, FR-NEW-003
- **Given** plain `v1.0` at `a1b2c3d`. **When** Alice creates `v1.0` at `a1b2c3d` confirming the overwrite, then creates `v8.0` at `a1b2c3d` confirming the overwrite. **Then** two new entries exist: `overwrote tag 'v1.0' at <full id of a1b2c3d> (was <full id of a1b2c3d>)` and `created tag 'v8.0' at <full id of a1b2c3d>`.
- **Priority:** Medium

### SPEC-0052/E2E-080: Branch and tag share a name
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_080_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-021, FR-NEW-001
- **Given** branch `rel` at `b2c3d4e` and tag `rel` at `a1b2c3d`. **When** Alice creates `v1` targeting `rel`. **Then** `v1` is at `b2c3d4e`.
- **Priority:** Medium

### SPEC-0052/E2E-081: Short commit prefix
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_081_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-021, FR-NEW-004
- **Given** commit `a1b2c3d` exists and no branch or tag is named `a1b2c`. **When** Alice creates `v1` targeting `a1b2c`. **Then** the refusal is `Cannot tag 'a1b2c': no such branch, tag or commit.` **And** no tag is created.
- **Priority:** Medium

### SPEC-0052/E2E-082: Ambiguous commit prefix
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_082_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-021, FR-NEW-004
- **Given** two commits whose ids begin with `a1b2c3d`. **When** Alice creates `v1` targeting `a1b2c3d`. **Then** the refusal is `Cannot tag 'a1b2c3d': no such branch, tag or commit.` **And** no tag is created.
- **Priority:** Medium

## Constraints
- Do not touch `fetch`, `pull`, `clone` or `push` code (later stories).
- Do not add a dependency.
- Do not change the existing branch error text: `validate_branch_name` keeps saying `branch name`; add a noun parameter, default call sites unchanged.
- Annotated tags and `message` belong to the next story.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0083: create a plain tag and list tags with kind and order`, body `Spec: SPEC-0052` and the requirement ids.
