# US-0095: documentation, round trip test and spec close

> Spec: SPEC-0053 (sdlc/specs/SPEC-0053-delete-a-tag-on-the-remote/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0093, US-0094
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Close SPEC-0053: prove the publish, delete remotely, delete locally, republish round trip, document the new action, and run the requirement coverage audit.

## Requirements covered
- SPEC-0053/FR-NEW-001 (partial: round trip): WHEN a member deletes a named tag on the remote and states the commit the remote tag points at now, THE Remote exchange context SHALL remove that tag from the remote and only that tag.
- SPEC-0053/FR-NEW-004 (partial: round trip): IF the remote has no tag of that name, THEN THE Remote exchange context SHALL refuse with `Remote tag 'v9.9' does not exist.`

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_035` green; `rg -n 'git.remote_delete_tag' AGENTS.md .agent_docs/tools.md .agent_docs/git.md` finds the three docs.

## Design excerpt
### Decisions
- DEC-014 (SPEC-0053): Contract changes are deliberate: `#[tool]` method and args struct in `mcp/server.rs` with the frozen descriptions, a `#[cfg(test)]` registry entry in `tools/git.rs`, `TOOL_CONTRACT.txt`, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current` and reviewed, every count site raised by one (`contract_golden.rs`, `mcp/server.rs`, `crates/agent/src/mcp.rs:317-319`, `tools/all.rs`, `tools/tool_surface_e2e.rs`, `ALL_GIT_TOOLS`), and the parity docs (`AGENTS.md`, `.agent_docs/tools.md`, `e2e_new_957`/`958`).

### Data model
No schema change; a remote deletion writes nothing to the database.

### Contracts
Documentation only plus the round trip test. `.agent_docs/git.md` Tags section gains the remote deletion rules (confirmation, refusal order, lease, audit line, limits). `.agent_docs/git-divergences.md` D15 notes remote deletion is delivered by SPEC-0053 (BL-0075). Tool counts are already live from the first story; verify with `rg`.

### Paths
| Path | Action |
|---|---|
| `AGENTS.md` | modify (mention the action) |
| `.agent_docs/tools.md` | modify (full row of the action) |
| `.agent_docs/git.md` | modify (remote deletion paragraph) |
| `.agent_docs/git-divergences.md` | modify (D15 note) |
| `crates/core/src/tools/git.rs` | modify (test `e2e_tag_035` only) |

### Existing patterns
Test helpers (private to the tests module of `crates/core/src/tools/git.rs`): `call_publish_tag`, `publish_as`, `bare_refs`, `publish_audit`, `cloned_remote`, `advance_bare_remote`, `plain_tag_bare_remote`, `bare_ref_sha`, `closing_remote`, `with_git_hosts_lock`; the seam `after_listing` in `git_tags.rs`; the SPEC-0052 tests `e2e_tag_033` to `e2e_tag_085` show the shapes (audit assertions read the audit log like `publish_audit`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_delete_remote_tag` with a `file://` bare remote (the registered tool for non member and expired credential cases).

### SPEC-0053/E2E-035: Publish, delete remotely, delete locally, publish again
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_035_*`
- **Category:** cross scenario. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** Alice has the local tag `v1.0` at `a1b2c3d`. **When** Alice publishes `v1.0`, deletes it on the remote stating `a1b2c3d`, deletes it locally, then Bob creates a local `v1.0` at `e5f6a7b` and publishes it, and Alice deletes that one on the remote stating `e5f6a7b`. **Then** after the remote deletion the local `v1.0` is still there; after the local deletion the remote still has no `v1.0`; the republication succeeds; the last deletion succeeds. **And** each step leaves exactly its own audit entry.
- **Priority:** High

## Constraints
- No production code change.
- Verify every statement against the code, never from memory.
- Dashes are not punctuation (project rule).
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing git test stays green, in particular the tag publish and branch push tests; `tool_contract_golden_is_current` green after review of the regenerated diff.
- [ ] Docs updated: AGENTS.md, .agent_docs/* as listed in Paths.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] Closes spec: the spec's full acceptance set green (E2E-001..052), and every requirement of SPEC-0053 cited by at least one story with status done or landed.
- [ ] One commit `US-0095: documentation, round trip test and spec close`, body `Spec: SPEC-0053` and the requirement ids.
