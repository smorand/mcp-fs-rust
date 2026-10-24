# US-0088: receive remote tags on clone

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0086
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Make `git.remote_clone` import the tags pointing into the cloned branch's history, using the same reconcile step as fetch, and report `tags_added` and `tags_kept_local` in its result.

## Requirements covered
- SPEC-0052/FR-NEW-010 (partial: clone path): IF a remote tag has the name of a local tag at a different commit, THEN THE Remote exchange context SHALL keep the local tag and report the tag as `kept local`.
- SPEC-0052/FR-NEW-011: WHEN a project is cloned, THE Remote exchange context SHALL bring in the tags that point into the cloned branch's history.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_046 e2e_tag_047 e2e_tag_059 e2e_tag_060` green; cloning a bare remote with `v1.0` and annotated `v2.0` lists both in `git.tags`.

## Design excerpt
### Decisions
- DEC-012 (SPEC-0052): A reconcile step in `git_tags.rs`, `reconcile_incoming_tags(entry, repo, incoming) -> (added, kept_local)`, runs right after `import_from_repo` in fetch, pull and clone: for each incoming tag, absent locally means `set_ref` and added; identical (DEC-017) means skip; different means keep the local tag and list it as kept local. Results gain `tags_added` and `tags_kept_local` (arrays of names, sorted). The incoming temporary refs are deleted from the on-disk repo. Obliged by: FR-NEW-009, FR-NEW-010, FR-NEW-011. Alternatives: letting libgit2 write refs (it cannot see the db's local tags, since only objects are hydrated, `tools/git.rs:1898`).
- DEC-014 (SPEC-0052): Clone takes its tags from the temporary clone itself: every `refs/tags/*` of the cloned repo whose peeled target is a commit equal to or an ancestor of the cloned branch tip, passed to the same reconcile step (a re-clone into a populated project is allowed today, `tools/git.rs:3174-3176`, so a conflicting local tag is kept). Obliged by: FR-NEW-011, FR-NEW-010. Alternatives: a second network fetch (clone already holds the data).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
`git.remote_clone` result gains `tags_added` and `tags_kept_local`. Tags are read from the temporary clone: every `refs/tags/*` whose peeled target is a commit equal to or an ancestor of the cloned branch tip.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (clone persistence calls the reconcile step after `import_from_repo`; tests) |
| `crates/core/src/tools/git_tags.rs` | modify (helper to list clone tags if needed) |

### Existing patterns
Existing pattern: `fetch_from_remote` at `crates/core/src/git/remote.rs:703` (lists advertised refs, then `update_tips` callback, `download_tags(AutotagOption::None)` at `:747`), consumed by `fetch_branch_inner` in `crates/core/src/tools/git.rs` (~3545: `write_lock`, `hydrate`, `fetch_from_remote`, `objects.import_from_repo`, `set_ref` per updated ref). Test helpers (private to the `tests` module): `seed_bare_remote` (`:9373`), `advance_bare_remote` (`:9428`), `tag_bare_remote` (`:10594`), `call_fetch_branch`, `call_pull_branch` (`:10987`), `call_clone_and_import` (`:8399`), `call_push_branch` (`:9599`).

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-046: Clone brings in history tags
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_046_*`
- **Category:** journey (happy). **Scenario:** SC-005. **Requirements:** FR-NEW-011
- **Given** the remote `main` has `v1.0` and annotated `v2.0` (message `Release 2.0`) in its history, and `v9.0` only on `experimental`. **When** Alice clones it into `proj-new`. **Then** `proj-new` lists `v1.0` and `v2.0` with the right kind and message. **And** `v9.0` is absent. **And** the clone result lists `v1.0` and `v2.0` as added and nothing as kept local.
- **Priority:** Critical

### SPEC-0052/E2E-047: Clone with no tag, and a failed clone
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_047_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-011
- **Given** remote A has no tag; remote B goes down during the clone. **When** Alice clones A, then B. **Then** A succeeds with an empty tag list; B is refused with the usual clone refusal. **And** B leaves no tag behind.
- **Priority:** High

### SPEC-0052/E2E-059: Clone with many tags
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_059_*`
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-011
- **Given** the remote `main` history holds 500 tags, 100 of them annotated with messages. **When** Alice clones it into `proj-new`. **Then** `proj-new` lists 500 tags, the annotated ones keeping kind, message and tagger. **And** the clone result lists 500 as added.
- **Priority:** Medium

### SPEC-0052/E2E-060: Clone meets a conflicting local tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_060_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-011, FR-NEW-010
- **Given** `proj-new` already has `v1.0` at `a1b2c3d`; the remote `v1.0` is at `c3d4e5f`. **When** Alice clones the remote into `proj-new`. **Then** local `v1.0` stays at `a1b2c3d`. **And** the clone result lists `v1.0` as kept local.
- **Priority:** High

## Constraints
- A failed or refused clone leaves no tag (quota admission and refusal happen before any ref write, `tools/git.rs:3160-3163`).
- Do not change what branch is persisted.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0088: receive remote tags on clone`, body `Spec: SPEC-0052` and the requirement ids.
