# US-0089: publish one tag to the remote

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0083
> min_tier: 1
> Files touched: 8
> Closes spec: no

## Objective
Add the `git.remote_push_tag` tool: send exactly one tag, refuse when the project tracks no remote, when the tag is unknown, when its commit is not on the remote, map remote and connection failures, report `up_to_date` for an identical remote tag, and audit only a real publication. Overwrite is the next story.

## Requirements covered
- SPEC-0052/FR-NEW-007: WHEN a member publishes a named tag, THE Remote exchange context SHALL send that tag and no other.
- SPEC-0052/FR-NEW-012 (partial: publication line): WHEN a tag is created, overwritten, deleted or published, THE Audit context SHALL record the member and one line of text.
- SPEC-0052/FR-NEW-014 (partial: publish): IF the requester is not a member of the project, THEN THE system SHALL refuse to create, delete, publish, list or receive tags with the refusal it gives for any other project action, and reveal no tag name.
- SPEC-0052/FR-NEW-018 (partial: no remote and name refusal order): IF the project tracks no remote when a member publishes a tag, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag 'v1.0': this project tracks no remote.` IF a member confirms an overwrite while the remote has no tag of that name, THEN THE Remote exchange context SHALL publish the tag as a creation. IF a member confirms an overwrite while the remote tag is identical to the local one, THEN THE Remote exchange context SHALL report `up to date` and leave no audit entry. IF a member confirms an overwrite without stating the remote tag's current commit while the remote has the tag at a different commit, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` and leave the remote untouched. IF the commit the member states is not the remote tag's current commit, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.` and leave the remote untouched.
- SPEC-0052/FR-NEW-019: IF the commit a member publishes a tag for is not on the remote, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.` and leave the remote untouched.
- SPEC-0052/FR-NEW-020 (partial: publication order): WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` green; publishing `v1.0` to a bare remote makes `bare_ref_sha(remote, "refs/tags/v1.0")` equals the tag's commit and the bare remote gained no other reference.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0052): Three new MCP tools, `git.tag_create`, `git.tag_delete` and `git.remote_push_tag`; `git.tags` and the fetch, pull and clone results gain additive keys. Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: extra parameters on `git.remote_push` (it requires `branch`, so a tag would make it an either/or contract), one `git.tag` tool with an `action` argument (breaks the one tool one verb convention of the 111 tools). Total 114 tools.
- DEC-008 (SPEC-0052): Refusals are produced in the order of FR-NEW-020 by the order of the calls: `open` (membership), `reject_if_operation_in_progress`, name validation, no commit, target resolution, existing name (checked again under the write lock). Codes: invalid name `invalid_argument`; unknown target, no commit and unknown tag `not_found`; existing name and remote tag refusals `no_clobber`; commit not on the remote `invalid_argument`. Messages are the spec's, verbatim. Obliged by: FR-NEW-020, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-013. Alternatives: new error codes (the 16 codes are a frozen set, `errors.rs:9-24`).
- DEC-009 (SPEC-0052): Create, overwrite and delete run under `entry.write_lock` (the lock `branch_create` and `commit` hold, `tools/git.rs:2121-2135`), so concurrent same-name calls serialise and exactly one wins. Audit goes through `safety.record_audit(person, project, action, "/", detail)` with actions `git.tag_create`, `git.tag_overwrite`, `git.tag_delete`, `git.tag_publish` and the spec's detail lines; full 40 character ids. Obliged by: FR-NEW-012, FR-NEW-015, FR-NEW-016. Alternatives: optimistic retry (rejected, the lock already exists).
- DEC-015 (SPEC-0052): Publishing is the tool `git.remote_push_tag(mount_id, tag, remote="origin", overwrite=false, expected_remote_commit="")`. Flow: `authorize`, local tag exists, `require_remote`, credential resolution (`resolve_clone_credential`), one connection to list the advertised refs, the checks of FR-NEW-019 and FR-NEW-008, then one push of `refs/tags/<n>:refs/tags/<n>` (a `+` prefix only when overwrite is confirmed and the stated commit matched). The push core of `push_to_remote` (`git/remote.rs:516`, hardcoded `refs/heads` refspec at `:601`) is split into a ref name taking private function that both the branch and the tag entry points call; branch behaviour stays byte identical. Obliged by: FR-NEW-007, FR-NEW-008, FR-NEW-018, FR-NEW-019. Alternatives: libgit2 `Remote::push` with both connections merged (the branch path keeps its own negotiation view).
- DEC-016 (SPEC-0052): Tag publication cannot use `run_remote_operation` (`git/remote.rs:338`): it audits every outcome, failures included, which FR-NEW-012 forbids. A sibling `trace_remote_operation` keeps the same `git.remote` span and records nothing; `git_tags.rs` calls `record_audit` itself, only after a real publication. This deliberately departs from the one audit per remote call rule of SPEC-0011 FR-NEW-057/072 for this one tool. Obliged by: FR-NEW-012. Alternatives: auditing failures too (contradicts the spec).
- DEC-019 (SPEC-0052): Name refusals on publish: libgit2 validates ref names more strictly than the branch rules (`//`, trailing `.`, `@{` pass `validate_branch_name`, `tools/git.rs:2200-2236`). A tag with such a name is stored (db row, odb written tag object) and listed, but syncing its ref into the on-disk repo or the remote's per ref rejection both produce `Remote refused tag '<n>': <reason>` (code `internal`), no audit entry, nothing changed. E2E-085 is realised through the push status callback with a rejection message (the local file transport runs no hooks). This is a recorded concern, not a spec change. Obliged by: FR-NEW-007. Alternatives: tightening the name rules (contradicts FR-NEW-004), refusing at create (contradicts SC-001).
- DEC-020 (SPEC-0052): "On the remote" (FR-NEW-019): the tag's commit equals an advertised branch tip, or is an ancestor of one whose objects the project already holds (`graph_descendant_of`); tips absent from the local odb are skipped; the publish never fetches. Obliged by: FR-NEW-019. Alternatives: asking the remote with a want/have probe (a fetch in disguise).
- DEC-021 (SPEC-0052): Contract changes are deliberate and reviewed: three new `#[tool]` methods in `mcp/server.rs`, three matching `#[cfg(test)]` registry entries, the `message`, `overwrite`, `expected_remote_commit` parameters with their frozen descriptions copied from `TOOL_CONTRACT.txt`, the `git.tags` description, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the 111 constants raised stepwise to 112, 113, 114 (`tools/contract_golden.rs:146,:253,:309,:323`, `mcp/server.rs:4294,4315,4316`, `crates/agent/src/mcp.rs:319`). Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: none (the golden test fails otherwise).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
**`git.remote_push_tag`** (new): `mount_id` (string, required), `tag` (string, required: "Name of the local tag to publish, without the refs/tags/ prefix."), `remote` (string, default `origin`: "Declared remote to publish to."). Returns `{"mount_id","tag","remote","created":bool,"up_to_date":bool,"remote_sha":<commit>,"auth"}`.
Order of refusals: member; `Tag '<n>' does not exist.`; `Cannot publish tag '<n>': this project tracks no remote.`; credential and connection refusals as for a branch push; `Cannot publish tag '<n>': commit <7 chars> is not on the remote. Publish its branch first.`; (next story: differing remote tag); remote name rejection `Remote refused tag '<n>': <reason>`.
Audit (action `git.tag_publish`, only on a real publication): `published tag '<n>' at <full commit id>`; `up_to_date`, refusals and connection failures write nothing. A new `trace_remote_operation` in `git/remote.rs` keeps the `git.remote` span without auditing.
Push core: `push_to_remote` is split so a private function takes full ref names (`refs/heads/<b>` or `refs/tags/<t>`); the branch entry point keeps its exact behaviour.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (split the push core, `list_advertised_refs`, `trace_remote_operation`) |
| `crates/core/src/tools/git_tags.rs` | modify (`tool_remote_push_tag`, `publish_tag`, `short7`) |
| `crates/core/src/tools/git.rs` | modify (`#[cfg(test)]` registry entry, helper `call_publish_tag`, tests) |
| `crates/core/src/mcp/server.rs` | modify (`GitRemotePushTagArgs`, `git_remote_push_tag`, count) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify |
| `crates/core/src/tools/contract_golden.rs` | modify (count) |
| `crates/agent/src/mcp.rs` | modify (count) |

### Existing patterns
Existing pattern, the remote pipeline: `tool_remote_push` at `crates/core/src/tools/git.rs:1494` -> `remote_push` at `:3222` (`require_remote`, `resolve_clone_credential`, `early_remote_failure`) -> `push_branch_inner` at `:3416` (`write_lock`, `on_git_thread`, `hydrate`, `with_remote_deadline`) -> `push_to_remote` at `crates/core/src/git/remote.rs:516`.

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-033: Publish one tag only
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_033_*`
- **Category:** journey (happy). **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** local `v1.0` (`a1b2c3d`) and `v2.0` (`c3d4e5f`); the remote has neither. **When** Alice publishes `v1.0`. **Then** the remote has `v1.0` at `a1b2c3d`. **And** the remote has no `v2.0`.
- **Priority:** Critical

### SPEC-0052/E2E-034: Publish an unknown tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_034_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** no local `v9.9`. **When** Alice publishes `v9.9`. **Then** the refusal is `Tag 'v9.9' does not exist.` **And** the remote is untouched.
- **Priority:** High

### SPEC-0052/E2E-035: Publish an identical tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_035_*`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-012
- **Given** the remote and the local project both have `v1.0` at `a1b2c3d`. **When** Alice publishes `v1.0`. **Then** the result says `up to date`. **And** the remote is unchanged and no audit entry was added.
- **Priority:** High

### SPEC-0052/E2E-038: Remote unavailable, rate limited or credential expired
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_038_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** local `v1.0` exists. **When** Alice publishes while the remote is down, then while it rate limits, then with an expired credential. **Then** each is refused with the refusal publishing a branch gives for that cause. **And** the remote has no partial tag and the local tag is unchanged.
- **Priority:** High

### SPEC-0052/E2E-039: Connection lost while publishing
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_039_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** local `v2.0` exists. **When** the connection drops during the publish before the tag reaches the remote, then Alice publishes again. **Then** the first attempt, which dropped before the tag reached the remote, gives the usual connection refusal; the second succeeds. **And** the remote never holds a partial tag and one audit entry exists.
- **Priority:** Medium

### SPEC-0052/E2E-040: Non member cannot publish; the audit names the publisher
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_040_*`
- **Category:** security. **Scenario:** SC-004. **Requirements:** FR-NEW-014, FR-NEW-012, FR-NEW-007
- **Given** Carol is not a member; `v1.0` exists locally. **When** Carol publishes `v1.0`, then Alice publishes it. **Then** Carol receives the usual refusal and the remote is untouched. **And** exactly one audit entry exists, by Alice, reading `published tag 'v1.0' at <full id of a1b2c3d>`.
- **Priority:** Critical

### SPEC-0052/E2E-055: Publish with no remote
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_055_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-018
- **Given** `proj-local` tracks no remote and has `v1.0`. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': this project tracks no remote.` **And** no audit entry was added.
- **Priority:** High

### SPEC-0052/E2E-071: Tag on a commit the remote lacks
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_071_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** `v1.0` is at `a1b2c3d`, a commit on local branch `dev` never published. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': commit a1b2c3d is not on the remote. Publish its branch first.` **And** the remote has no `v1.0` and no new commit.
- **Priority:** High

### SPEC-0052/E2E-072: Tag on a published commit
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_072_*`
- **Category:** journey (happy). **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** `dev` was published and `v1.0` is at its tip `b2c3d4e`. **When** Alice publishes `v1.0`. **Then** the remote has `v1.0` at `b2c3d4e`. **And** the remote gained no other reference.
- **Priority:** Medium

### SPEC-0052/E2E-073: Commit reachable from another remote branch
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_073_*`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** `v1.0` is at `c3d4e5f`, which belongs to the history of remote branch `release` but not of the tracked branch. **When** Alice publishes `v1.0`. **Then** the remote has `v1.0` at `c3d4e5f`.
- **Priority:** Medium

### SPEC-0052/E2E-074: Non member outranks every other publish refusal
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_074_*`
- **Category:** security. **Scenario:** SC-004. **Requirements:** FR-NEW-020, FR-NEW-014
- **Given** `proj-local` tracks no remote; Carol is not a member. **When** Carol publishes `v9.9`. **Then** she receives the usual refusal, not `Tag 'v9.9' does not exist.` **And** nothing about the project is revealed.
- **Priority:** Medium

### SPEC-0052/E2E-075: Unknown tag outranks no remote
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_075_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-020, FR-NEW-007
- **Given** `proj-local` tracks no remote and has no `v9.9`. **When** Alice publishes `v9.9`. **Then** the refusal is `Tag 'v9.9' does not exist.` **And** it does not mention the remote.
- **Priority:** Medium

### SPEC-0052/E2E-077: Connection lost after the remote accepted the tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_077_*`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-012
- **Given** local `v2.0` exists and the connection drops after the remote accepted the tag but before Alice saw the answer. **When** Alice sees the connection refusal, then publishes again. **Then** the second publish says `up to date`. **And** no audit entry exists for either attempt.
- **Priority:** Medium

### SPEC-0052/E2E-084: Commit only on an unfetched remote tip
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_084_*`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-019
- **Given** the remote `main` advanced to `d4e5f6a`, which Alice has not fetched, and her `v1.0` is at `e5f6a7b`, an ancestor of `d4e5f6a` on the remote but absent from Alice's history. **When** Alice publishes `v1.0`. **Then** the refusal is `Cannot publish tag 'v1.0': commit e5f6a7b is not on the remote. Publish its branch first.` **And** the remote has no `v1.0`.
- **Priority:** Medium

### SPEC-0052/E2E-085: Remote refuses the tag name
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_085_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007
- **Given** Alice created the local tag `a//b` (accepted by the naming rules) and the remote rejects that name with the reason `invalid ref name`. **When** Alice publishes `a//b`. **Then** the refusal is `Remote refused tag 'a//b': invalid ref name`. **And** the remote has no `a//b`, the local tag is unchanged and no audit entry was added.
- **Priority:** Medium

### 9.4 Modified tests

## Constraints
- The branch push must stay byte identical: the existing push tests are the guard.
- Publishing never sends commits and never fetches (DEC-020).
- Do not add `overwrite` yet.
- Use the live tool count, not a remembered one.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0089: publish one tag to the remote`, body `Spec: SPEC-0052` and the requirement ids.
