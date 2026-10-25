# US-0092: delete a confirmed remote tag

> Spec: SPEC-0053 (sdlc/specs/SPEC-0053-delete-a-tag-on-the-remote/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 12
> Closes spec: no

## Objective
Add the tool `git.remote_delete_tag` for the nominal path: delete one tag on the remote when the member states its commit, refuse an absent tag, a missing commit and a differing commit with the exact texts, report the deleted commit, audit a real deletion, and refuse a non member. It is the first of two stories touching the contract (tool count 114 to 115).

## Requirements covered
- SPEC-0053/FR-NEW-001: WHEN a member deletes a named tag on the remote and states the commit the remote tag points at now, THE Remote exchange context SHALL remove that tag from the remote and only that tag.
- SPEC-0053/FR-NEW-002: IF the member states no commit, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.
- SPEC-0053/FR-NEW-003: IF the stated commit is not the commit the remote tag points at now, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.
- SPEC-0053/FR-NEW-004: IF the remote has no tag of that name, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v9.9' does not exist.`
- SPEC-0053/FR-NEW-007 (partial: absent tag, then no commit, then differing commit): WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list: not a member (FR-NEW-009); the project tracks no remote at all (FR-NEW-005); the named remote is not tracked (FR-NEW-014); the remote or the connection fails (FR-NEW-012, FR-NEW-013); the remote tag is absent (FR-NEW-004); no commit is stated (FR-NEW-002); the stated commit differs (FR-NEW-003); the remote declines (FR-NEW-006).
- SPEC-0053/FR-NEW-008: WHEN a remote tag is really deleted, THE Audit context SHALL record the member and one line of text.
- SPEC-0053/FR-NEW-009 (partial: the registered tool path): IF the requester is not a member of the project, THEN THE system SHALL refuse with `'carol@example.com' is not a member of 'proj-a'` (the requester's identifier and the project name), and reveal no tag name, commit or sign that the tag exists.
- SPEC-0053/FR-NEW-015: WHEN a remote tag is really deleted, THE Remote exchange context SHALL tell the member the tag name as given, the remote name used and the full 40 character id of the commit the tag had. Behavioral and technology agnostic. Users: Alice and Bob are members of `proj-a`; Carol is not. The remote is a GitHub Enterprise repository tracked by `proj-a` under the name `origin`. Unless stated, the remote tag `v1.0` points at the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5` (shown as `f6a7b8c`) and the local tag `v1.0` points at `c3d4e5f`; the checked out commit is `a1b2c3d`.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` is green for the new tests; `bare_ref_sha(remote, "refs/tags/v1.0")` no longer exists after a confirmed deletion while `refs/tags/v1.1` is unchanged; `cargo test -p mcp-fs-core --lib tool_contract_golden` is green after the golden is regenerated and reviewed.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0053): One new tool, `git.remote_delete_tag(mount_id, tag, remote="origin", expected_remote_commit="")`, next to `git.remote_push_tag` in the `git.remote_*` family. Tool count 114 to 115, `git.*` family up by one.
- DEC-003 (SPEC-0053): The engine lives in `crates/core/src/tools/git_tags.rs` beside publication (`tool_remote_delete_tag`, `delete_remote_tag`, `delete_on_repo`); the remote layer in `crates/core/src/git/remote.rs` (`delete_tag_on_remote`). Tests go into the existing tests module of `tools/git.rs` where the bare remote helpers are private.
- DEC-004 (SPEC-0053): Deletion reuses the push core of `git/remote.rs:609` (`push_refs`): `PushRefs` gains the delete form, an empty source giving the refspec `:refs/tags/<n>` (never `+`), with the lease on the advertised tag object always set. The refspec is built only for a tag found in the advertised refs, so an invalid name never reaches libgit2.
- DEC-005 (SPEC-0053): Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then the tag checks (absent, no commit, differs), then the delete push (declined). Codes: not stated `invalid_argument`; differs `no_clobber` (existing `moved`); absent, no remote, named remote not tracked `not_found`; declined and connection failures `internal`.
- DEC-006 (SPEC-0053): The stated commit is checked with the existing `stated_commit_matches` (`git_tags.rs:799`): trimmed, case insensitive, at least 7 hexadecimal characters, a prefix of the remote tag's peeled commit; a blank text is "not stated" (decided before matching). The commits shown in texts use `short7` (`git_tags.rs:812`).
- DEC-007 (SPEC-0053): The remote tag state comes from the same single listing: the ref's own object and, for an annotated tag, its peeled `^{}` commit; an annotated tag is confirmed by its peeled commit. Absent covers every name not advertised, an empty, blank or invalid name included (shown as given).
- DEC-008 (SPEC-0053): When the push reports a lost lease (the remote tag moved or was deleted between listing and push), re list: the tag absent gives `Remote tag '<n>' does not exist.`, otherwise the `moved` refusal with the new commit; the push is never retried.
- DEC-010 (SPEC-0053): The audit line is written with `safety.record_audit(person, project, "git.tag_remote_delete", "/", "deleted tag '<n>' on remote '<r>' (was at <40>)")` only after the push returned Ok; the operation runs inside `trace_remote_operation` (span, no audit), never `run_remote_operation`. The result is `{mount_id, tag, remote, deleted_commit, auth}` with the full commit id. A push that fails after the remote applied it returns an error, so no line is written.
- DEC-014 (SPEC-0053): Contract changes are deliberate: `#[tool]` method and args struct in `mcp/server.rs` with the frozen descriptions, a `#[cfg(test)]` registry entry in `tools/git.rs`, `TOOL_CONTRACT.txt`, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current` and reviewed, every count site raised by one (`contract_golden.rs`, `mcp/server.rs`, `crates/agent/src/mcp.rs:317-319`, `tools/all.rs`, `tools/tool_surface_e2e.rs`, `ALL_GIT_TOOLS`), and the parity docs (`AGENTS.md`, `.agent_docs/tools.md`, `e2e_new_957`/`958`).
- DEC-016 (SPEC-0053): No existing test is modified; the 52 tests are all new. The existing assertions that change are only counts, registry lengths and the parity docs, as for every new tool.

### Data model
No schema change; a remote deletion writes nothing to the database.

### Contracts
**`git.remote_delete_tag`** (new): `mount_id` (string, required), `tag` (string, required: "Name of the tag to delete on the remote, without the refs/tags/ prefix."), `remote` (string, default `origin`: "Declared remote to delete the tag on."), `expected_remote_commit` (string, default `""`: "Commit the remote tag points at now; at least 7 hexadecimal characters."). Annotations: destructive true, read only false, idempotent false, open world true. Result `{"mount_id","tag","remote","deleted_commit":<full 40 char id>,"auth"}`.
Description (written here, the spec gives none): "Delete one tag on a declared remote. Only that tag is removed: no branch, no other tag, no commit and no local tag. The member must state the commit the remote tag points at now (expected_remote_commit, at least 7 hexadecimal characters); a missing or different commit is refused with the commit the remote tag really has."
Texts (verbatim) and codes: `Remote tag '<n>' points at <7>. State that commit to confirm its deletion.` (`invalid_argument`); `Remote tag '<n>' has moved to <7> since you last saw it. Fetch, then confirm again.` (`no_clobber`, the existing `moved`); `Remote tag '<n>' does not exist.` (`not_found`); non member: the existing `open()` refusal `'<person>' is not a member of '<project>'`.
Refusal order in this story: member, remote tag absent, no commit stated, commit differs. A lost lease (the tag moved or vanished between listing and push) re-lists once: absent gives the absent text, otherwise the `moved` text with the new commit. Remote lookup and connection refusals come in the next story; here the remote is the one named (default `origin`) and exists.
Audit (action `git.tag_remote_delete`, path `/`), only after a real deletion: `deleted tag '<n>' on remote '<remote>' (was at <full 40 char commit id>)`.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (`tool_remote_delete_tag`, `delete_remote_tag`, `delete_on_repo`, the three refusal helpers) |
| `crates/core/src/git/remote.rs` | modify (`PushRefs` delete form, `delete_tag_on_remote`) |
| `crates/core/src/mcp/server.rs` | modify (`GitRemoteDeleteTagArgs`, `git_remote_delete_tag`, counts 114 to 115) |
| `crates/core/src/tools/git.rs` | modify (`#[cfg(test)]` registry entry, helper `call_delete_remote_tag`, tests, registry counts) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify |
| `crates/core/src/tools/contract_golden.rs` | modify (counts) |
| `crates/agent/src/mcp.rs` | modify (count) |
| `crates/core/src/tools/all.rs` | modify (counts) |
| `crates/core/src/tools/tool_surface_e2e.rs` | modify (counts) |
| `AGENTS.md` | modify only the counts the parity tests force |
| `.agent_docs/tools.md` | modify (counts and one tool row) |

### Existing patterns
Existing pattern, a tool built on an engine: `tool_remote_push_tag` and `publish_tag` in `crates/core/src/tools/git_tags.rs:466-611` (`open`, remote lookup via `entry.db.list_remotes()`, `resolve_clone_credential`, `trace_remote_operation`, `record_audit` after success); declaration pattern: `GitRemotePushTagArgs` and `git_remote_push_tag` in `crates/core/src/mcp/server.rs` (~3516) with the `to_call_result(...)` wrapper.

Existing pattern, the remote layer: `push_refs` (`crates/core/src/git/remote.rs:609`, `PushRefs`, `PushFailure`, lease in `push_negotiation`), `push_tag_to_remote` (`:696`), `tag_push_error` (`:767`), `list_advertised_refs` (`:793`), `connection_error` (`:1226`), `timeout_error` (`:1269`); stated commit rules `stated_commit_matches` (`git_tags.rs:799`), `short7` (`:812`), `moved` (`:774`).

Test helpers (private to the tests module of `crates/core/src/tools/git.rs`): `call_publish_tag`, `publish_as`, `bare_refs`, `publish_audit`, `cloned_remote`, `advance_bare_remote`, `plain_tag_bare_remote`, `bare_ref_sha`, `closing_remote`, `with_git_hosts_lock`; the seam `after_listing` in `git_tags.rs`; the SPEC-0052 tests `e2e_tag_033` to `e2e_tag_085` show the shapes (audit assertions read the audit log like `publish_audit`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_delete_remote_tag` with a `file://` bare remote (the registered tool for non member and expired credential cases).

### SPEC-0053/E2E-001: Delete with the full commit id
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_001_*`
- **Category:** journey happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-015
- **Given** Alice is a member of `proj-a`; the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`; the remote also has tags `v1.1` and `release-2026`. **When** Alice deletes the remote tag `v1.0` and states `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** the remote tags `v1.1` and `release-2026` still exist, the local tag `v1.0` still points at `c3d4e5f`, and Alice is told the tag `v1.0`, the remote `origin` and the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`.
- **Priority:** Critical

### SPEC-0053/E2E-002: Seven character abbreviation
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_002_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the same state as E2E-001. **When** Bob, a member, deletes the remote tag `v1.0` and states `f6a7b8c`. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** every branch and commit of the remote is unchanged.
- **Priority:** Critical

### SPEC-0053/E2E-003: Uppercase id with surrounding spaces
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_003_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the same state as E2E-001. **When** Alice states `  F6A7B8C9D0E1F2A3B4C5D6E7F8A9B0C1D2E3F4A5  `. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** only `v1.0` is removed from the remote.
- **Priority:** High

### SPEC-0053/E2E-004: Mixed case nine character abbreviation
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_004_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the same state as E2E-001. **When** Alice states ` F6a7B8c9D `. **Then** the deletion succeeds and the remote no longer has `v1.0`. **And** the other remote tags are unchanged.
- **Priority:** Medium

### SPEC-0053/E2E-005: Only the named tag is removed
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_005_*`
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the remote has branches `main` and `dev` and tags `v1.0`, `v1.1`, `v2.0`; the local project has tags `v1.0` and `v1.1` and the commits `a1b2c3d` and `e5f6a7b`. **When** Alice deletes the remote tag `v1.0` stating `f6a7b8c`. **Then** the remote keeps `v1.1`, `v2.0`, `main` and `dev` at their previous commits. **And** the local tags `v1.0` and `v1.1` and all local commits are unchanged.
- **Priority:** Critical

### SPEC-0053/E2E-006: Six characters never match
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_006_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice states `f6a7b8` (six characters, a true beginning of the commit). **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the remote tag `v1.0` still exists and no audit entry was added.
- **Priority:** High

### SPEC-0053/E2E-007: Stated commit is another commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_007_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the same state as E2E-006. **When** Alice states `c3d4e5f`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the remote is untouched and no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-008: Same first seven characters, different afterwards
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_008_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the same state as E2E-006. **When** Alice states `f6a7b8cffffffffffffffffffffffffffffffff0`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** `v1.0` remains on the remote.
- **Priority:** Medium

### SPEC-0053/E2E-009: Tag moved after the member looked at it
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_009_*`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** Alice looked at the remote tag `v1.0` at `f6a7b8c`, then someone moved it to `e5f6a7b`. **When** Alice states `f6a7b8c`. **Then** the refusal is `Remote tag 'v1.0' has moved to e5f6a7b since you last saw it. Fetch, then confirm again.`. **And** the remote tag `v1.0` still points at `e5f6a7b` and no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-010: No commit stated
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_010_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice deletes the remote tag `v1.0` and states no commit. **Then** the refusal is `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`. **And** the remote is untouched and no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-011: Empty text counts as none
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_011_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the same state as E2E-010. **When** Alice states an empty text. **Then** the refusal is `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`. **And** the remote is untouched.
- **Priority:** High

### SPEC-0053/E2E-012: Only spaces counts as none
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_012_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the same state as E2E-010. **When** Alice states five spaces. **Then** the refusal is `Remote tag 'v1.0' points at f6a7b8c. State that commit to confirm its deletion.`. **And** the remote is untouched.
- **Priority:** High

### SPEC-0053/E2E-013: The hint shows seven characters
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_013_*`
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Bob states no commit. **Then** the refusal contains `f6a7b8c`. **And** the refusal does not contain the rest of the commit, `9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`.
- **Priority:** Medium

### SPEC-0053/E2E-014: Remote tag does not exist
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_014_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the remote has no tag `v9.9`. **When** Alice deletes the remote tag `v9.9` and states `f6a7b8c`. **Then** the refusal is `Remote tag 'v9.9' does not exist.`. **And** no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-015: Absent tag outranks a missing commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_015_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004, FR-NEW-007
- **Given** the remote has no tag `v9.9`. **When** Alice deletes `v9.9` and states no commit. **Then** the refusal is `Remote tag 'v9.9' does not exist.`. **And** the remote is untouched.
- **Priority:** High

### SPEC-0053/E2E-016: Tag exists only locally
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_016_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** a local tag `release-2026` points at `e5f6a7b` and the remote has no `release-2026`. **When** Alice deletes the remote tag `release-2026` and states `e5f6a7b`. **Then** the refusal is `Remote tag 'release-2026' does not exist.`. **And** the local tag `release-2026` still exists.
- **Priority:** High

### SPEC-0053/E2E-022: Non member is refused
- Level / Driver / Location: functional e2e / registered tool / `crates/core/src/tools/git.rs`, `e2e_tag_022_*`
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-009, FR-NEW-007
- **Given** Carol is not a member of `proj-a`. **When** Carol deletes `v1.0` and states `f6a7b8c`. **Then** the refusal is `'carol@example.com' is not a member of 'proj-a'`, with no tag name, commit or hint that the tag exists. **And** the remote is untouched and no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-023: Existence cannot be probed by a non member
- Level / Driver / Location: functional e2e / registered tool / `crates/core/src/tools/git.rs`, `e2e_tag_023_*`
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-009
- **Given** Carol is not a member; the remote has `v1.0` but no `v9.9`. **When** Carol deletes `v9.9` with no commit, then `v1.0` with no commit. **Then** both refusals are exactly `'carol@example.com' is not a member of 'proj-a'`. **And** nothing about the remote is revealed.
- **Priority:** High

### SPEC-0053/E2E-025: Audit entry for a real deletion
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_025_*`
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice deletes `v1.0` stating `f6a7b8c`. **Then** exactly one audit entry exists, by Alice, reading `deleted tag 'v1.0' on remote 'origin' (was at f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5)`. **And** the entry shows the full id although seven characters were stated.
- **Priority:** Critical

### SPEC-0053/E2E-026: No audit entry for any refusal
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_026_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** the audit trail holds some number of entries. **When** Alice tries once with no commit, once with a wrong commit, once on the absent tag `v9.9`, once on a protected tag; Carol tries once. **Then** each attempt is refused. **And** the audit trail holds the same number of entries as before.
- **Priority:** High

### SPEC-0053/E2E-027: Annotated tag is confirmed by its commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_027_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-008
- **Given** the remote annotated tag `release-2026` has its own identifier `9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b` and points at the commit `e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4`. **When** Alice states no commit, then states `9a8b7c6`, then states `e5f6a7b`. **Then** the first attempt is refused with `Remote tag 'release-2026' points at e5f6a7b. State that commit to confirm its deletion.`, the second with `Remote tag 'release-2026' has moved to e5f6a7b since you last saw it. Fetch, then confirm again.`, and the third succeeds. **And** the audit line shows the full commit id `e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4`.
- **Priority:** High

### SPEC-0053/E2E-028: Names with a slash and an accent
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_028_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** the remote tags `rel/2026` and `été-1` both point at `f6a7b8c`. **When** Alice deletes `rel/2026` stating `f6a7b8c`, then `été-1`, then `rel/2027`. **Then** the first two succeed and only those tags are removed; the third is refused with `Remote tag 'rel/2027' does not exist.`. **And** the other remote tags are unchanged.
- **Priority:** Medium

### SPEC-0053/E2E-036: Remote tag without a local copy
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_036_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** the remote tag `v2.0` points at `f6a7b8c` and `proj-a` has no local `v2.0`. **When** Bob deletes `v2.0` stating `f6a7b8c`. **Then** the deletion succeeds. **And** no local tag is created or changed.
- **Priority:** High

### SPEC-0053/E2E-037: Many remote tags
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_037_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** the remote has 5000 tags `t0001` to `t5000`. **When** Alice deletes `t2500` stating its commit. **Then** exactly `t2500` is removed. **And** 4999 remote tags remain, and a second attempt on `t2500` is refused with `Remote tag 't2500' does not exist.`.
- **Priority:** Low

### SPEC-0053/E2E-047: The reported commit is the full id
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_047_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-015
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice deletes `v1.0` stating only `f6a7b8c`. **Then** Alice is told the tag `v1.0`, the remote `origin` and the full commit id `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`, not the seven characters. **And** the audit line shows the same full id.
- **Priority:** Medium

### SPEC-0053/E2E-049: An empty or blank tag name counts as absent
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_049_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the remote has tags `v1.0` and `v1.1`. **When** Alice deletes a tag named with an empty text, then a tag named with three spaces, stating `f6a7b8c`. **Then** the refusals are `Remote tag '' does not exist.` and `Remote tag '   ' does not exist.`. **And** both remote tags are untouched and no audit entry was added.
- **Priority:** Medium

### SPEC-0053/E2E-050: An invalid tag name counts as absent
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_050_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-004
- **Given** the remote has the tag `v1.0`. **When** Alice deletes the tag named `v1..0` stating `f6a7b8c`. **Then** the refusal is `Remote tag 'v1..0' does not exist.`. **And** the remote tag `v1.0` is untouched and no audit entry was added.
- **Priority:** Medium

### SPEC-0053/E2E-051: A stated text that is not hexadecimal never matches
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_051_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the remote tag `v1.0` points at `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`. **When** Alice states `zzzzzzz`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`. **And** the remote tag `v1.0` still exists and no audit entry was added.
- **Priority:** Medium

## Constraints
- The registered tool refuses `file://` remotes by design: tests reach the engine through a direct helper, as the publish tests do.
- Do not call `reject_if_operation_in_progress` (FR-NEW-010).
- No new dependency; do not touch fetch, pull, clone, branch push or tag publish behaviour.
- Counts: use the live numbers (`rg -n '114' crates`), never a guess; the 11 listed files besides the engine are mechanical count and parity edits.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing git test stays green, in particular the tag publish and branch push tests; `tool_contract_golden_is_current` green after review of the regenerated diff.
- [ ] Docs updated: only the counts the parity tests force; the closing story documents the action.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0092: delete a confirmed remote tag`, body `Spec: SPEC-0053` and the requirement ids.
