# US-0087: receive remote tags on pull, also when the merge pauses

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0086
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Make `git.remote_pull` report and keep tags exactly as fetch does, import them before any merge decision so a pull that pauses on a conflict still delivers them, and bring none when the exchange fails.

## Requirements covered
- SPEC-0052/FR-NEW-009 (partial: pull path): WHEN a fetch or a pull succeeds, THE Remote exchange context SHALL bring in the remote tags that point into the fetched branch's history, and report which were added.
- SPEC-0052/FR-NEW-010 (partial: pull path): IF a remote tag has the name of a local tag at a different commit, THEN THE Remote exchange context SHALL keep the local tag and report the tag as `kept local`.
- SPEC-0052/FR-NEW-017: IF a pull completes its exchange with the remote but pauses on a merge conflict, THEN THE Remote exchange context SHALL still bring in the tags of FR-NEW-009 and report them as added or kept local.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_054 e2e_tag_063 e2e_tag_064` green; a pull paused on a conflict leaves the remote tag listed in `git.tags` and in the pull result.

## Design excerpt
### Decisions
- DEC-012 (SPEC-0052): A reconcile step in `git_tags.rs`, `reconcile_incoming_tags(entry, repo, incoming) -> (added, kept_local)`, runs right after `import_from_repo` in fetch, pull and clone: for each incoming tag, absent locally means `set_ref` and added; identical (DEC-017) means skip; different means keep the local tag and list it as kept local. Results gain `tags_added` and `tags_kept_local` (arrays of names, sorted). The incoming temporary refs are deleted from the on-disk repo. Obliged by: FR-NEW-009, FR-NEW-010, FR-NEW-011. Alternatives: letting libgit2 write refs (it cannot see the db's local tags, since only objects are hydrated, `tools/git.rs:1898`).
- DEC-013 (SPEC-0052): In pull, reconcile runs immediately after the fetch import and before the merge decision, so a pull that pauses with `status: "conflict"` has already delivered its tags; a failed exchange returns before reconcile. Obliged by: FR-NEW-017. Alternatives: reconcile after the merge (loses the tags on a pause).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
`git.remote_pull` result gains `tags_added` and `tags_kept_local`, same shape as fetch, present also when the result carries `status: "conflict"`.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (pull path calls the reconcile step right after its fetch import and before the merge decision; tests) |
| `crates/core/src/tools/git_tags.rs` | modify only if the reconcile signature needs a small adjustment |

### Existing patterns
Existing pattern: `fetch_from_remote` at `crates/core/src/git/remote.rs:703` (lists advertised refs, then `update_tips` callback, `download_tags(AutotagOption::None)` at `:747`), consumed by `fetch_branch_inner` in `crates/core/src/tools/git.rs` (~3545: `write_lock`, `hydrate`, `fetch_from_remote`, `objects.import_from_repo`, `set_ref` per updated ref). Test helpers (private to the `tests` module): `seed_bare_remote` (`:9373`), `advance_bare_remote` (`:9428`), `tag_bare_remote` (`:10594`), `call_fetch_branch`, `call_pull_branch` (`:10987`), `call_clone_and_import` (`:8399`), `call_push_branch` (`:9599`).

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-054: Pull paused on a conflict still receives tags
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_054_*`
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-017, FR-NEW-009
- **Given** the remote `main` has `v1.0` (`c3d4e5f`) in its history and a change that conflicts with Alice's local change. **When** Alice pulls `main`. **Then** the pull ends paused on the conflict. **And** `v1.0` exists locally at `c3d4e5f` and the result lists it as added.
- **Priority:** High

### SPEC-0052/E2E-063: Failed pull brings no tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_063_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-017, FR-NEW-009
- **Given** the remote has a new tag `v3.0` in history. **When** Alice pulls while the remote is down, then with an expired credential. **Then** each gives the usual pull refusal. **And** `v3.0` does not exist locally.
- **Priority:** High

### SPEC-0052/E2E-064: Paused pull keeps a conflicting local tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_064_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-017, FR-NEW-010
- **Given** local `v1.0` at `a1b2c3d`; remote `v1.0` at `c3d4e5f`; the pull pauses on a conflict. **When** Alice pulls `main`. **Then** local `v1.0` stays at `a1b2c3d`. **And** the result lists `v1.0` as kept local.
- **Priority:** High

## Constraints
- Do not change merge behaviour: the same results, conflict states and errors as before.
- A failed exchange returns before any tag is reconciled.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0087: receive remote tags on pull, also when the merge pauses`, body `Spec: SPEC-0052` and the requirement ids.
