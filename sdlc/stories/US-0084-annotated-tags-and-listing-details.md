# US-0084: annotated tags with message, tagger and creation time

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0083
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
Add the optional `message` argument to `git.tag_create` (annotated tags, byte exact message, overwrite changes the kind) and the `message`, `tagger`, `tagger_email`, `tagged_at` keys to `git.tags` entries of annotated tags.

## Requirements covered
- SPEC-0052/FR-MOD-001: The Versioning context SHALL show, for each listed tag, its name, its commit, its kind (`lightweight` or `annotated`) and, for an annotated tag, its message and tagger.
- SPEC-0052/FR-NEW-001: WHEN a member creates a tag with a name and no message, THE Versioning context SHALL create a plain tag at the checked out commit, or at the branch, tag or commit the member names.
- SPEC-0052/FR-NEW-002: WHEN a member creates a tag with a message, THE Versioning context SHALL create an annotated tag recording the message exactly as given, the member as tagger and the creation time.
- SPEC-0052/FR-NEW-003: IF a tag with the requested name exists and the member has not confirmed overwriting, THEN THE Versioning context SHALL refuse and change nothing.
- SPEC-0052/FR-NEW-013 (partial: refusal with a message and no commit): IF the project has no commit and the member creates a tag without a target, THEN THE Versioning context SHALL refuse with `Cannot tag: this project has no commit yet.`

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` green, including the 5000 tag listing test; reading a tag created with message `Été 🚀 <b>gras</b> & "quotes"` plus a second line through `git.tags` returns that exact string.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0052): An annotated tag object is written with `repo.odb()?.write(git2::ObjectType::Tag, bytes)` where bytes are `object <commit>\ntype commit\ntag <name>\ntagger <name> <<email>> <unix> +0000\n\n<message>` and the message is the caller's string unchanged (no `message_prettify`, which `commit` applies at `tools/git.rs:2160`). Obliged by: FR-NEW-002 (message kept byte for byte), FR-NEW-004 (names the branch rules accept). Alternatives: `Repository::tag` (its own stricter name validation and message handling, verified in the context7 docs of `/websites/rs_git2`).
- DEC-006 (SPEC-0052): The tagger is built exactly as a commit author is (`tools/git.rs:2137-2142`): name = the person id before `@`, email = the person id. A listing entry for an annotated tag carries `kind: "annotated"`, `message`, `tagger` (name), `tagger_email`, `tagged_at` (UTC `%Y-%m-%dT%H:%M:%SZ`); a plain tag carries `kind: "lightweight"` and none of the three. `sha` is always the peeled commit, `object` the ref's own target. Obliged by: FR-MOD-001. Alternatives: renaming `sha` (breaks readers, rejected), `commit_json`'s date format (not the format FR-MOD-001 spells).
- DEC-009 (SPEC-0052): Create, overwrite and delete run under `entry.write_lock` (the lock `branch_create` and `commit` hold, `tools/git.rs:2121-2135`), so concurrent same-name calls serialise and exactly one wins. Audit goes through `safety.record_audit(person, project, action, "/", detail)` with actions `git.tag_create`, `git.tag_overwrite`, `git.tag_delete`, `git.tag_publish` and the spec's detail lines; full 40 character ids. Obliged by: FR-NEW-012, FR-NEW-015, FR-NEW-016. Alternatives: optimistic retry (rejected, the lock already exists).
- DEC-010 (SPEC-0052): Listing reads the `git_refs` tag rows, then inside `on_git_thread` after `hydrate` peels each target with the repo (`find_object`, `peel_to_commit`, `find_tag` for annotated) and sorts by name bytes ascending (`Vec<String>` sort is byte order). Stash refs never match `refs/tags/`. Obliged by: FR-MOD-001. Alternatives: parse tag objects by hand from the blob store (brittle).
- DEC-021 (SPEC-0052): Contract changes are deliberate and reviewed: three new `#[tool]` methods in `mcp/server.rs`, three matching `#[cfg(test)]` registry entries, the `message`, `overwrite`, `expected_remote_commit` parameters with their frozen descriptions copied from `TOOL_CONTRACT.txt`, the `git.tags` description, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the 111 constants raised stepwise to 112, 113, 114 (`tools/contract_golden.rs:146,:253,:309,:323`, `mcp/server.rs:4294,4315,4316`, `crates/agent/src/mcp.rs:319`). Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: none (the golden test fails otherwise).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
**`git.tag_create`** gains `message` (string, default `""`: "Message of an annotated tag; empty creates a plain tag."). A non empty message, whitespace only included, creates an annotated tag with the tagger of DEC-006. The returned `kind` becomes `annotated`, `object` the tag object id, `sha` the commit.
**`git.tags`** entries of annotated tags add `message` (exactly as given), `tagger`, `tagger_email`, `tagged_at` (UTC `YYYY-MM-DDTHH:MM:SSZ`). Plain tags carry none of the three.
Refusal: `Cannot tag: this project has no commit yet.` also when a message is given.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (annotated creation, listing details) |
| `crates/core/src/mcp/server.rs` | modify (`message` on `GitTagCreateArgs`) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify (`message` parameter, `git.tags` shape) |
| `crates/core/src/tools/git.rs` | modify (tests only) |

### Existing patterns
Existing pattern, a local ref-mutating tool: `tool_branch_delete` at `crates/core/src/tools/git.rs:1255` (calls `open`, then `reject_if_operation_in_progress`, then `on_git_thread`), engine `branch_delete` at `:2476` (validate, `entry.write_lock.lock().await`, `entry.db`, audit at `:2521`). Declaration pattern: `GitBranchDeleteArgs` at `crates/core/src/mcp/server.rs:900` and `#[tool(name = "git.branch_delete", ...)]` calling `crate::tools::git::tool_branch_delete(self.git_ctx(), a, None)` then `to_call_result("git.branch_delete", out)`.

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-004: Create an annotated tag
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_004_*`
- **Category:** journey (happy). **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** Alice is a member, `main` at `a1b2c3d`. **When** Alice creates `v2.0` with message `Release 2.0`. **Then** the list shows `v2.0`, kind `annotated`, message `Release 2.0`, commit `a1b2c3d`. **And** the tagger is Alice, shown as her identifier on her commits, `alice@example.com`, and the creation time is shown in UTC as `YYYY-MM-DDTHH:MM:SSZ` within 5 seconds of now.
- **Priority:** Critical

### SPEC-0052/E2E-005: Message kept verbatim
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_005_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** Alice is a member. **When** Alice creates `v2.1` with a two line message whose first line is `Été 🚀 <b>gras</b> & "quotes"`. **Then** the list returns both lines exactly, with no escaping and no truncation.
- **Priority:** Medium

### SPEC-0052/E2E-008: Overwrite changes the kind
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_008_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-002
- **Given** plain `v1.0` exists. **When** Bob creates `v1.0` with message `Re-cut` confirming the overwrite. **Then** `v1.0` is `annotated`, message `Re-cut`, tagger Bob.
- **Priority:** Medium

### SPEC-0052/E2E-020: List shows kind and message
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_020_*`
- **Category:** journey (happy). **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** plain `v1.0` at `a1b2c3d` and annotated `v2.0` at `c3d4e5f` with message `Release 2.0`. **When** Bob lists tags. **Then** two entries appear: `v1.0` `lightweight` with no message; `v2.0` `annotated` with message `Release 2.0`. **And** the entries appear in the order `v1.0`, `v2.0`.
- **Priority:** Critical

### SPEC-0052/E2E-022: 5000 tags
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_022_*`
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Given** `proj-a` holds 5000 tags, plain and annotated. **When** Alice lists tags. **Then** all 5000 are returned with kind and message, in ascending name order.
- **Priority:** High

### SPEC-0052/E2E-067: Annotated tag in a project with no commit
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_067_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** `proj-empty` has no commit. **When** Alice creates `v1.0` with message `First` and no target. **Then** the refusal is `Cannot tag: this project has no commit yet.` **And** no tag exists.
- **Priority:** Medium

### SPEC-0052/E2E-078: Empty message gives a plain tag
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_078_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-001
- **Given** Alice is a member and `main` is at `a1b2c3d`. **When** she creates `v7.0` with an empty message, then `v7.1` with a message of three spaces. **Then** `v7.0` is `lightweight` with no message. **And** `v7.1` is `annotated` with a message of three spaces.
- **Priority:** Medium

### SPEC-0052/E2E-MOD-001: Listing now shows kind and message (was: listing returns name and commit only)
- Level / Driver / Location: functional e2e / tool registry `e.call` / `crates/core/src/tools/git.rs` tests module, `e2e_tag_mod_001_*`
- **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Validated before:** a listing returned name and commit only.
- **Validates now:** the listing also shows kind and message.
- **Given** annotated `v2.0` with message `Release 2.0` at `c3d4e5f`. **When** Alice lists tags. **Then** `v2.0` shows commit `c3d4e5f`, kind `annotated`, message `Release 2.0`. **And** name and commit are exactly what they were before.
- **Category:** state transition. **Priority:** High

## Constraints
- Do not call `message_prettify` or `Repository::tag`: the message must stay byte exact (DEC-005).
- Do not change the tool count.
- Remote exchange code is out of scope.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0084: annotated tags with message, tagger and creation time`, body `Spec: SPEC-0052` and the requirement ids.
