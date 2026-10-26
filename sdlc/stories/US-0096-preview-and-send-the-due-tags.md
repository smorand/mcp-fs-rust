# US-0096: preview and send the due tags

> Spec: SPEC-0054 (sdlc/specs/SPEC-0054-publish-all-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 12
> Closes spec: no

## Objective
Add the tool `git.remote_push_tags` for the nominal path: list every local tag with its outcome without sending anything (the preview), and send the tags that are safe to send in one push when a count is stated. It brings the multi ref push core, the classification of every tag, the remote lookup refusals, the non member refusal and the audit line per tag sent. The count guard itself (wrong or malformed numbers) is the next story; in this story a stated number equal to the count sends, anything else is out of scope and must not be relied on. It is the first of five stories of SPEC-0054 and owns the contract change.

## Requirements covered
- SPEC-0054/FR-NEW-001: WHEN a member asks to publish all tags and does not state a number, THE Remote exchange context SHALL list every local tag with its outcome and SHALL NOT send anything.
- SPEC-0054/FR-NEW-002: WHEN a member states a number equal to the count of tags to publish, THE Remote exchange context SHALL send exactly those tags and no other object, and report the outcome of every local tag.
- SPEC-0054/FR-NEW-007: The Remote exchange context SHALL report a tag the remote already holds identically as `up to date` and leave no audit entry for it.
- SPEC-0054/FR-NEW-009: WHEN a tag is sent, THE Audit context SHALL record the member and the line `published tag '<name>' at <full commit id>`.
- SPEC-0054/FR-NEW-010: IF the project tracks no remote, or not the remote the member names, THEN THE Remote exchange context SHALL refuse and send nothing.
- SPEC-0054/FR-NEW-011: IF the requester is not a member of the project, THEN THE system SHALL refuse with the refusal it gives for any other project action and reveal no tag name.

## Independent Test
`cargo test -p mcp-fs-core --lib publish_all` is green for the new tests; after a confirmed send `bare_ref_sha(remote, "refs/tags/v1.0")` equals the local commit while the remote branches are unchanged; `cargo test -p mcp-fs-core --lib tool_contract_golden` is green after the golden is regenerated and reviewed (116 tools, 54 `git.*`).

## Design excerpt
### Decisions
- DEC-002 (SPEC-0054): One new tool, `git.remote_push_tags(mount_id, remote="origin", confirm_count)`, next to `git.remote_push_tag` in the `git.remote_*` family. Tool count 115 to 116, `git.*` family 53 to 54. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-016. **Rationale:** FR-NEW-016 forbids changing the single publish; a separate tool keeps its contract exact. **Alternatives considered:** making `tag` optional on `git.remote_push_tag` (an either/or contract and an overwrite rule that bulk forbids); two tools, one to preview and one to send (the preview and the count check must share one classification, and the count guard is one action in the spec). **Source:** design. **Evidence:** `crates/core/src/mcp/server.rs:3530-3538`.
- DEC-003 (SPEC-0054): The engine lives in `crates/core/src/tools/git_tags.rs` beside single publish (`tool_remote_push_tags`, `publish_all_tags`, `classify_tags`, `parse_confirm`, `render`); the remote layer in `crates/core/src/git/remote.rs` (`push_tags_to_remote`). Tests go into the existing tests module of `tools/git.rs` where the bare remote helpers are private. **Obliged by:** SPEC-0054/FR-NEW-002. **Alternatives considered:** a new module (the helpers `commit_on_remote`, `advertised_oid`, `probe_identical`, `short7` are private to `git_tags.rs`); looping over `publish_tag` per tag (50 tags would open 100 connections, FR-NEW-017). **Source:** existing code. **Evidence:** `git_tags.rs:535`, `:986`.
- DEC-004 (SPEC-0054): The push core `push_refs` (`git/remote.rs:609`) takes a slice of `PushRefs` and returns the per ref statuses; every existing caller (`push_to_remote`, `push_tag_to_remote`, `delete_tag_on_remote`) passes a one element slice and behaves as before. The status callback keeps a list of `(refname, message)` instead of one cell, and the lease check records every mismatched destination instead of the first. A creation refspec carries the lease "absent" (the zero object id) so a remote that gained the tag after the listing aborts the push before anything is sent. **Obliged by:** SPEC-0054/FR-NEW-002, FR-NEW-008, FR-NEW-020. **Rationale:** one connection for N tags and a stale listing detected without sending. **Alternatives considered:** a second push function duplicating the credential, deadline and callback setup (two copies to keep in step); no lease and trusting the remote to refuse an update (the local transport raises one hard error for the whole push, so no per tag report). **Source:** design. **Evidence:** `git/remote.rs:609-680`.
- DEC-005 (SPEC-0054): Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then `parse_confirm` (not a count), then the count comparison (wrong number), then the push. Codes in Section 5. **Obliged by:** SPEC-0054/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-015, FR-NEW-019. **Alternatives considered:** parsing the number before the connection (rejected: FR-NEW-015 puts the connection first, E2E-050). **Source:** design. **Evidence:** `git_tags.rs:467-505`.
- DEC-006 (SPEC-0054): Classification, per local tag read from `entry.db.list_refs()` (prefix `refs/tags/`), sorted in Rust by the bytes of the name (never by the database collation). Order of checks, applied to the one listing (FR-NEW-018): commit not on remote (memoized per distinct commit with a `HashMap<Oid, bool>`, so tags sharing a commit cost one history walk), then identical (same object id, or two annotated tags on the same commit whose message the existing `probe_identical` compares), then differs (remote commit is the peeled `^{}` entry when advertised, shown with `short7`), then to publish. Time O(t log t + u d) for t tags, u distinct commits, d history depth, plus one probe connection per annotated lookalike. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-018. **Alternatives considered:** one probe fetch of all lookalikes at once (a larger refactor of `probe_remote_tag` for a case that is rare). **Source:** existing code. **Evidence:** `git_tags.rs:636-695`, `:986`.
- DEC-007 (SPEC-0054): The whole action holds the repository `write_lock`, preview included, so the count shown and the count checked come from one consistent view and two confirmations serialize; the second finds nothing to publish and gets the wrong number refusal with `Tags to publish: 0, not N` (FR-NEW-013). `reject_if_operation_in_progress` is NOT called (FR-NEW-014). The span is `trace_remote_operation("git.remote_push_tags", ...)`. **Obliged by:** SPEC-0054/FR-NEW-013, FR-NEW-014. **Alternatives considered:** no lock for the preview (a preview racing a confirm could show a stale count; harmless but untestable); taking the lock only for the send (the count check would race). **Source:** existing code. **Evidence:** `git_tags.rs:560`.
- DEC-009 (SPEC-0054): Output rendering: one `render(tags)` function builds `lines` and the multi line error message from the same list, so the preview, the report and the wrong number refusal cannot drift (FR-NEW-001, 002, 003 share the wording). The refusal line is always first. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004. **Alternatives considered:** three formatters. **Source:** design.
- DEC-010 (SPEC-0054): The push set is the `to publish` tags whose name `git2::Reference::is_valid_name` accepts; a name it refuses (for example `a//b`, accepted by the branch rules, the SPEC-0052 case of an invalid reference name) is reported `refused by remote: invalid ref name` without a refspec, as the single publish refuses it before its push. Each tag ref is first written to the on disk repository (`repo.reference`), as the single publish does, because a push names refs. **Obliged by:** SPEC-0054/FR-NEW-008. **Alternatives considered:** pushing it and letting libgit2 fail the whole push. **Source:** existing code. **Evidence:** `git_tags.rs:688-695`.
- DEC-012 (SPEC-0054): The audit line is `record_audit(person, project, "git.tag_publish", "/", "published tag '<n>' at <40>")`, once per tag reported `published`, after the push returned. Nothing else is audited (preview, refusals, skipped tags). **Obliged by:** SPEC-0054/FR-NEW-009. **Alternatives considered:** one summary line (the spec fixes one per tag). **Source:** existing code. **Evidence:** `git_tags.rs:595-600`.
- DEC-014 (SPEC-0054): Contract changes are deliberate: `#[tool]` method and args struct in `mcp/server.rs` with frozen descriptions, a `#[cfg(test)]` registry entry in `tools/git.rs`, `TOOL_CONTRACT.txt`, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current` and reviewed, every count site raised by one (`contract_golden.rs`, `mcp/server.rs`, `crates/agent/src/mcp.rs:317-319`, `tools/all.rs`, `tools/tool_surface_e2e.rs`, `ALL_GIT_TOOLS`), and the parity docs (`AGENTS.md`, `.agent_docs/tools.md`). Annotations: destructive hint false (nothing is overwritten or deleted), idempotent true (a repeat reports up to date), open world true. **Obliged by:** SPEC-0054/FR-NEW-001. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `TOOL_CONTRACT.txt:535`.
- DEC-015 (SPEC-0054): No existing test is modified; the 59 tests are all new. The assertions that change are counts, registry lengths and parity docs, as for every new tool. **Obliged by:** SPEC-0054/FR-NEW-016. **Alternatives considered:** none. **Source:** design.
- DEC-017 (SPEC-0054): An empty `remote` text means `origin` (the serde default applies only to an absent field): the tool maps an empty text to `origin` before the lookup. **Obliged by:** SPEC-0054/FR-NEW-010. **Alternatives considered:** refusing an empty name with the "does not track a remote named ''" text (the spec chose `origin`). **Source:** spec constraint. **Evidence:** `mcp/server.rs:843`.

### Data model
No schema change; no database write beyond what the single publish already does.

### Contracts
**`git.remote_push_tags`** (new in US-0096): parameters `mount_id` (string, required), `remote` (string, default `origin`, empty means `origin`), `confirm_count` (optional string or integer; absent or null is a preview). Result `{mount_id, remote, to_publish, published, sent, tags: [{name, outcome, commit, remote_commit, reason}], lines, message, auth}` with `outcome` one of `to publish`, `published`, `up to date`, `differs`, `commit not on remote`, `refused by remote`; `lines` hold the exact texts of the spec in byte order of the name. Refusals and codes: no remote `ERR_NOT_FOUND` `Cannot publish tags: this project tracks no remote.`; unknown remote `ERR_NOT_FOUND` `Cannot publish tags: this project does not track a remote named '<r>'.`; not a count `ERR_INVALID_ARGUMENT` `The number to confirm must be a whole number, zero or more.`; wrong number `ERR_NO_CLOBBER` `Tags to publish: <M>, not <N>. Review the list, then confirm again.` followed by one line per tag; connection, credential and timeout texts are the existing ones. Audit: action `git.tag_publish`, path `/`, `published tag '<n>' at <full 40>` per tag sent.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (`tool_remote_push_tags`, `publish_all_tags`, `classify_tags`, `parse_confirm`, `render`) |
| `crates/core/src/git/remote.rs` | modify (`push_refs` multi ref, `push_tags_to_remote`) |
| `crates/core/src/mcp/server.rs` | modify (tool `git.remote_push_tags`, `GitRemotePushTagsArgs`, counts) |
| `crates/core/src/tools/git.rs` | modify (registry entry, helper `call_publish_all_tags`, 16 tests) |
| `tool-contract-golden.json, TOOL_CONTRACT.txt` | regenerate, modify |
| `crates/core/src/tools/contract_golden.rs, crates/core/src/tools/all.rs, crates/core/src/tools/tool_surface_e2e.rs, crates/agent/src/mcp.rs` | modify (counts 115 to 116, git 53 to 54) |
| `AGENTS.md, .agent_docs/tools.md` | modify (count, one row) |

### Existing patterns
`tool_remote_delete_tag`, `delete_remote_tag` and `delete_on_repo` in `crates/core/src/tools/git_tags.rs` (member, lookup, credential, span, lock, listing; SPEC-0053); `call_delete_remote_tag`, `cloned_remote`, `bare_ref_sha`, `publish_audit` in the tests module of `crates/core/src/tools/git.rs`; the args struct `GitRemoteDeleteTagArgs` at `crates/core/src/mcp/server.rs:1139`; every count site listed in DEC-014.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Test functions are named `e2e_tag_<number>_publish_all_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_publish_all_tags` with a `file://` bare remote (the registered tool for the non member and credential cases). Fixtures: project `proj-a`, remote `origin` at the host `github.ibm.com`, members Alice and Bob, commits `a1b2c3d`, `b2c3d4e`, `c3d4e5f` on the remote unless stated (use the real commit ids the fixtures produce).

### SPEC-0054/E2E-001: Preview lists every tag and sends nothing
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_001_publish_all_*`
- **Given** local tags `v1.0` at `a1b2c3d`, `v1.1` at `b2c3d4e`, `v0.9` at `c3d4e5f`; the remote holds `v0.9` identically. **When** Alice asks to publish all tags without a number. **Then** Alice sees, in this order, `v0.9 up to date`, `v1.0 to publish`, `v1.1 to publish`, then `Tags to publish: 2. Confirm by stating 2 to send them.` **And** the remote still holds only `v0.9`, and the audit trail has no new entry.
- **Priority:** Critical

### SPEC-0054/E2E-002: Confirmed send publishes the due tags
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_002_publish_all_*`
- **Given** the state of E2E-001. **When** Alice confirms stating 2. **Then** Alice sees exactly the three lines `v0.9 up to date`, `v1.0 published`, `v1.1 published` and no other line. **And** the remote holds `v0.9`, `v1.0` at `a1b2c3d` and `v1.1` at `b2c3d4e`, the local tags are unchanged, and the audit trail holds exactly two new entries, both by Alice.
- **Priority:** Critical

### SPEC-0054/E2E-003: Annotated tags keep their kind and message
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_003_publish_all_*`
- **Given** the local annotated tag `v2.0` with the message `Release 2.0` at `a1b2c3d` and the lightweight tag `v2.1` at `b2c3d4e`, none on the remote. **When** Alice previews, then confirms stating 2. **Then** the report says `v2.0 published` and `v2.1 published`. **And** the remote tag `v2.0` is annotated with the message `Release 2.0` and `v2.1` is lightweight.
- **Priority:** High

### SPEC-0054/E2E-004: A single tag due
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_004_publish_all_*`
- **Given** the single local tag `v1.0` at `a1b2c3d`, not on the remote. **When** Alice previews. **Then** she sees `v1.0 to publish` and `Tags to publish: 1. Confirm by stating 1 to send them.` **And** when she confirms stating 1 the report is `v1.0 published` and the remote holds `v1.0`.
- **Priority:** Medium

### SPEC-0054/E2E-005: A named remote
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_005_publish_all_*`
- **Given** the project tracks `origin` and `backup`; local tag `v1.0` at `a1b2c3d`. **When** Alice previews and confirms stating 1 against `backup`. **Then** the report says `v1.0 published`. **And** `backup` holds `v1.0` and `origin` still has no `v1.0`.
- **Priority:** High

### SPEC-0054/E2E-006: Order is by name, byte by byte
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_006_publish_all_*`
- **Given** the local tags `Zeta`, `alpha`, `été` and `v1` on commits that are on the remote. **When** Alice previews. **Then** the list order is `Zeta`, `alpha`, `v1`, `été`.
- **Priority:** Medium

### SPEC-0054/E2E-007: A private mark is seen, removed, and not published
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_007_publish_all_*`
- **Given** the local tags `v1.0` at `a1b2c3d` and `wip-alice` at `b2c3d4e`, none on the remote. **When** Alice previews and sees `wip-alice to publish`, deletes `wip-alice` locally, previews again and confirms stating 1. **Then** the second preview says `Tags to publish: 1. Confirm by stating 1 to send them.` **And** the remote holds `v1.0` and has no `wip-alice`.
- **Priority:** High

### SPEC-0054/E2E-017: The project tracks no remote
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_017_publish_all_*`
- **Given** a project with local tags and no remote. **When** Alice previews, then confirms stating 1. **Then** both are refused with `Cannot publish tags: this project tracks no remote.`
- **Priority:** High

### SPEC-0054/E2E-018: The named remote is not tracked
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_018_publish_all_*`
- **Given** the project tracks only `origin`. **When** Alice previews against `staging`, then against `Origin`. **Then** the refusals are `Cannot publish tags: this project does not track a remote named 'staging'.` and `Cannot publish tags: this project does not track a remote named 'Origin'.` **And** `origin` is untouched.
- **Priority:** High

### SPEC-0054/E2E-019: A non member cannot probe or publish
- Level / Driver / Location: functional e2e / registered tool, `e.call` / `crates/core/src/tools/git.rs`, `e2e_tag_019_publish_all_*`
- **Given** Mallory is not a member of `proj-a`, which has the tag `secret-release`. **When** Mallory previews, then confirms stating 1. **Then** both are refused with `'mallory@example.com' is not a member of 'proj-a'`, and no tag name appears in either answer. **And** the remote and the audit trail are unchanged.
- **Priority:** Critical

### SPEC-0054/E2E-027: A name with a slash
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_027_publish_all_*`
- **Given** the local tag `release/1.0` at `a1b2c3d`. **When** Alice previews and confirms stating 1. **Then** the report says `release/1.0 published`. **And** the remote holds `release/1.0` at `a1b2c3d`.
- **Priority:** Medium

### SPEC-0054/E2E-030: Audit lines
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_030_publish_all_*`
- **Given** `v1.0` at `a1b2c3d` and `v1.1` at `b2c3d4e` due. **When** Alice confirms stating 2. **Then** the audit trail holds, for Alice, `published tag 'v1.0' at <full id of a1b2c3d>` and `published tag 'v1.1' at <full id of b2c3d4e>`. **And** no other entry was added.
- **Priority:** Critical

### SPEC-0054/E2E-031: No audit for an up to date tag
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_031_publish_all_*`
- **Given** `v0.9` identical on the remote and `v1.0` due. **When** Alice confirms stating 1. **Then** the audit trail gains one entry, for `v1.0`, and none for `v0.9`.
- **Priority:** High

### SPEC-0054/E2E-032: No audit for any refusal or preview
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_032_publish_all_*`
- **Given** two tags due. **When** Alice previews, states a wrong number, and a non member previews. **Then** the audit trail has no new entry after any of them.
- **Priority:** High

### SPEC-0054/E2E-034: Only tags are sent
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_034_publish_all_*`
- **Given** the remote branch `main` at `a1b2c3d`, the local branch `main` ahead at `b2c3d4e`, the local tag `v1.0` at `a1b2c3d`. **When** Alice confirms stating 1. **Then** the remote branch `main` is still at `a1b2c3d` and the remote holds no other new commit.
- **Priority:** High

### SPEC-0054/E2E-058: An empty remote name means origin
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_058_publish_all_*`
- **Given** the project tracks `origin` and `v1.0` is due. **When** Alice previews with the remote name left empty. **Then** the answer is exactly `v1.0 to publish`, `Tags to publish: 1. Confirm by stating 1 to send them.` **And** with a project tracking only `backup`, the same request is refused with `Cannot publish tags: this project does not track a remote named 'origin'.`
- **Priority:** Medium

## Constraints
- Files not to touch: anything under `sdlc/specs/`; `publish_tag`, `publish_inner`, `publish_on_repo` and the behaviour of `git.remote_push_tag`, `git.remote_push`, `git.remote_delete_tag` (their tests must pass unmodified).
- Dependencies not to add: none.
- Patterns to avoid: a second push function with its own callbacks; sorting by database collation; `unwrap` on fallible paths.
- Scope boundary: malformed and wrong numbers, nothing to publish, the stated 0 answer (US-0097); skipped, refused and drifting tags (US-0098); connection failures, concurrency, pause, cost (US-0099); single publish regression and docs (US-0100).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered by an earlier story may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing git tag, branch push and remote delete test passes unmodified (the multi ref refactor of `push_refs` must not change their behaviour); `tool_contract_golden_is_current` green after review of the regenerated diff.
- [ ] Docs updated: `AGENTS.md` and `.agent_docs/tools.md` (count and one row).
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0096: preview and send the due tags`, body `Spec: SPEC-0054` and the requirement ids.
