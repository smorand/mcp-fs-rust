# US-0090: overwrite a remote tag only on explicit confirmation

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0089, US-0084
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
Add `overwrite` and `expected_remote_commit` to `git.remote_push_tag`: refuse a remote tag that differs (kind, commit or message), allow replacement only with the confirmation and the remote tag's current commit, treat a confirmed overwrite of an absent or identical tag as creation or `up_to_date`.

## Requirements covered
- SPEC-0052/FR-NEW-007 (partial: identical comparison): WHEN a member publishes a named tag, THE Remote exchange context SHALL send that tag and no other.
- SPEC-0052/FR-NEW-008: IF the remote already has the tag at a different commit and overwriting is not confirmed, THEN THE Remote exchange context SHALL refuse and leave the remote untouched.
- SPEC-0052/FR-NEW-014 (partial: overwrite): IF the requester is not a member of the project, THEN THE system SHALL refuse to create, delete, publish, list or receive tags with the refusal it gives for any other project action, and reveal no tag name.
- SPEC-0052/FR-NEW-018 (partial: overwrite parts): IF the project tracks no remote when a member publishes a tag, THEN THE Remote exchange context SHALL refuse with `Cannot publish tag 'v1.0': this project tracks no remote.` IF a member confirms an overwrite while the remote has no tag of that name, THEN THE Remote exchange context SHALL publish the tag as a creation. IF a member confirms an overwrite while the remote tag is identical to the local one, THEN THE Remote exchange context SHALL report `up to date` and leave no audit entry. IF a member confirms an overwrite without stating the remote tag's current commit while the remote has the tag at a different commit, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` and leave the remote untouched. IF the commit the member states is not the remote tag's current commit, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.` and leave the remote untouched.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` green; a remote `v1.0` at another commit is untouched without confirmation and moved with it.

## Design excerpt
### Decisions
- DEC-017 (SPEC-0052): One function `tags_identical(repo, a, b)`: both plain with the same commit; both annotated with the same peeled commit and the same message (tagger and time ignored); mixed kinds differ. For a remote annotated tag whose object id differs from the local one, the message is read by a one tag shallow fetch (`depth 1`, refspec `+refs/tags/<n>:refs/mcpfs/probe/<n>`), then the probe ref is deleted. Obliged by: FR-NEW-007, FR-NEW-009, FR-NEW-010. Alternatives: comparing tag object ids (would compare tagger and time, which the spec excludes), asking the user (the remote exposes no message without a transfer).
- DEC-018 (SPEC-0052): Overwrite semantics: without `overwrite`, a differing remote tag is refused with the FR-NEW-008 text; with `overwrite` and no `expected_remote_commit` the same text; with a stated commit (full id or unique prefix of at least 7 characters, compared with the remote tag's peeled commit) that differs, the `has moved` text; with a match, the push is forced with a lease on the advertised oid. Confirmed overwrite on an absent remote tag is a plain creation, on an identical one `up_to_date`. Commits are shown as 7 characters (`short7`), distinct from the 8 character `short` (`tools/git.rs:1866`) that stays elsewhere. Obliged by: FR-NEW-008, FR-NEW-018. Alternatives: reuse `expected_remote_sha` (it leases a branch sha; a tag has both an object and a commit).
- DEC-015 (SPEC-0052): Publishing is the tool `git.remote_push_tag(mount_id, tag, remote="origin", overwrite=false, expected_remote_commit="")`. Flow: `authorize`, local tag exists, `require_remote`, credential resolution (`resolve_clone_credential`), one connection to list the advertised refs, the checks of FR-NEW-019 and FR-NEW-008, then one push of `refs/tags/<n>:refs/tags/<n>` (a `+` prefix only when overwrite is confirmed and the stated commit matched). The push core of `push_to_remote` (`git/remote.rs:516`, hardcoded `refs/heads` refspec at `:601`) is split into a ref name taking private function that both the branch and the tag entry points call; branch behaviour stays byte identical. Obliged by: FR-NEW-007, FR-NEW-008, FR-NEW-018, FR-NEW-019. Alternatives: libgit2 `Remote::push` with both connections merged (the branch path keeps its own negotiation view).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
**`git.remote_push_tag`** gains `overwrite` (bool, default `false`: "Replace a remote tag that differs from the local one; needs expected_remote_commit.") and `expected_remote_commit` (string, default `""`: "Commit the remote tag points at now; at least 7 characters.").
Texts: `Remote tag '<n>' already exists at a different commit (<7 chars>). Confirm overwriting to replace it.` (differs and no confirmation, or confirmation without a stated commit; the commit shown is the remote tag's) and `Remote tag '<n>' has moved to <7 chars> since you last saw it. Fetch, then confirm again.` (stated commit differs).
Rules: a remote tag is identical when `tags_identical` says so (`up_to_date`); a confirmed overwrite of an absent remote tag creates it, of an identical one is `up_to_date`; replacement pushes `+refs/tags/<n>:refs/tags/<n>` with a lease on the advertised oid; audit `published tag '<n>' at <full commit id>` only when something was pushed.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (forced tag push with lease, remote tag probe for the message) |
| `crates/core/src/tools/git_tags.rs` | modify (differ/moved/identical decision) |
| `crates/core/src/mcp/server.rs` | modify (two parameters) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify |
| `crates/core/src/tools/git.rs` | modify (tests) |

### Existing patterns
Existing pattern, the remote pipeline: `tool_remote_push` at `crates/core/src/tools/git.rs:1494` -> `remote_push` at `:3222` (`require_remote`, `resolve_clone_credential`, `early_remote_failure`) -> `push_branch_inner` at `:3416` (`write_lock`, `on_git_thread`, `hydrate`, `with_remote_deadline`) -> `push_to_remote` at `crates/core/src/git/remote.rs:516`.

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-036: Remote tag at another commit, no confirmation
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_036_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-008
- **Given** the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Alice publishes `v1.0` without confirming. **Then** the refusal is `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` **And** the remote `v1.0` is still at `f6a7b8c`.
- **Priority:** Critical

### SPEC-0052/E2E-037: Confirmed overwrite, and a moved remote tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_037_*`
- **Category:** state transition. **Scenario:** SC-004. **Requirements:** FR-NEW-008
- **Given** the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Alice publishes confirming the overwrite and stating `f6a7b8c`. **Then** the remote `v1.0` is at `a1b2c3d`. **And** if the remote tag had first been moved to `0a1b2c3`, the same request is refused and the remote stays at `0a1b2c3`.
- **Priority:** High

### SPEC-0052/E2E-056: Confirmed overwrite where the remote has no such tag, or an identical one
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_056_*`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-018, FR-NEW-007
- **Given** local `v1.0` at `a1b2c3d`; the remote has no `v1.0`. **When** Alice publishes confirming an overwrite and stating `f6a7b8c`, then publishes again the same way. **Then** the first publish creates `v1.0` at `a1b2c3d` on the remote; the second says `up to date`. **And** exactly one audit entry exists.
- **Priority:** Medium

### SPEC-0052/E2E-057: Stated commit differs from the remote tag's commit
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_057_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-018, FR-NEW-008
- **Given** the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Alice publishes confirming an overwrite and stating `0a1b2c3`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.` **And** the remote stays at `f6a7b8c`.
- **Priority:** High

### SPEC-0052/E2E-058: Non member cannot overwrite a remote tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_058_*`
- **Category:** security. **Scenario:** SC-004. **Requirements:** FR-NEW-008, FR-NEW-014
- **Given** Carol is not a member; the remote has `v1.0` at `f6a7b8c`; local `v1.0` is at `a1b2c3d`. **When** Carol publishes `v1.0` confirming an overwrite and stating `f6a7b8c`. **Then** she receives the usual refusal. **And** the remote `v1.0` is still at `f6a7b8c`.
- **Priority:** High

### SPEC-0052/E2E-069: Remote tag differs only by kind
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_069_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-008
- **Given** local annotated `v1.0` (message `A`) at `a1b2c3d`; the remote has plain `v1.0` at `a1b2c3d`. **When** Alice publishes `v1.0` without confirming. **Then** the refusal is `Remote tag 'v1.0' already exists at a different commit (a1b2c3d). Confirm overwriting to replace it.` **And** the remote is untouched.
- **Priority:** High

### SPEC-0052/E2E-083: Confirmed overwrite without a stated commit
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_083_*`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-018, FR-NEW-008
- **Given** the remote `v1.0` is at `f6a7b8c` and local `v1.0` is at `a1b2c3d`. **When** Alice publishes confirming the overwrite and stating no commit. **Then** the refusal is `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` **And** the remote stays at `f6a7b8c`.
- **Priority:** High

## Constraints
- The refusal order is the one of FR-NEW-020; the new checks come after the commit on remote check.
- The probe fetch deletes its temporary ref even on failure.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0090: overwrite a remote tag only on explicit confirmation`, body `Spec: SPEC-0052` and the requirement ids.
