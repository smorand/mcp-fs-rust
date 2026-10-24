# US-0091: documentation, round trip test and spec close

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0090, US-0085, US-0087, US-0088
> min_tier: 1
> Files touched: 7
> Closes spec: yes

## Objective
Close SPEC-0052: prove the create, publish, receive round trip, update the documentation (tool counts 111 to 114, tool reference, git guide, divergence register row D15), and run the requirement coverage audit.

## Requirements covered
- SPEC-0052/FR-NEW-002 (partial: round trip): WHEN a member creates a tag with a message, THE Versioning context SHALL create an annotated tag recording the message exactly as given, the member as tagger and the creation time.
- SPEC-0052/FR-NEW-007 (partial: round trip): WHEN a member publishes a named tag, THE Remote exchange context SHALL send that tag and no other.
- SPEC-0052/FR-NEW-009 (partial: round trip): WHEN a fetch or a pull succeeds, THE Remote exchange context SHALL bring in the remote tags that point into the fetched branch's history, and report which were added.
- SPEC-0052/FR-NEW-012 (partial: round trip): WHEN a tag is created, overwritten, deleted or published, THE Audit context SHALL record the member and one line of text.

## Independent Test
`make check` green; `rg -n '114' AGENTS.md` shows the new tool count and `rg -n '111' AGENTS.md .agent_docs/tools.md README.md` finds no stale count.

## Design excerpt
### Decisions
- DEC-021 (SPEC-0052): Contract changes are deliberate and reviewed: three new `#[tool]` methods in `mcp/server.rs`, three matching `#[cfg(test)]` registry entries, the `message`, `overwrite`, `expected_remote_commit` parameters with their frozen descriptions copied from `TOOL_CONTRACT.txt`, the `git.tags` description, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the 111 constants raised stepwise to 112, 113, 114 (`tools/contract_golden.rs:146,:253,:309,:323`, `mcp/server.rs:4294,4315,4316`, `crates/agent/src/mcp.rs:319`). Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: none (the golden test fails otherwise).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
Documentation only plus the round trip test. Tool counts: 111 becomes 114; `git.*` family 39 becomes 42. `git.tags` row of `.agent_docs/tools.md:312` gets the new keys; add the three tools. `.agent_docs/git-divergences.md` row D15 is marked closed by SPEC-0052 with its remaining out of scope parts (BL-0075, BL-0076).

### Paths
| Path | Action |
|---|---|
| `AGENTS.md` | modify (5 count hits, family list) |
| `.agent_docs/architecture.md` | modify (1 count hit) |
| `.agent_docs/testing.md` | modify (1 count hit) |
| `.agent_docs/tools.md` | modify (3 count hits, `git.tags` row, three new tool rows) |
| `.agent_docs/git.md` | modify (tags section: create, delete, publish, receive, audit lines) |
| `.agent_docs/git-divergences.md` | modify (D15 closed) |
| `crates/core/src/tools/git.rs` | modify (test `e2e_tag_051` only) |

### Existing patterns
Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-051: Create, publish, receive round trip
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_051_*`
- **Category:** cross scenario. **Scenario:** SC-001, SC-004, SC-005. **Requirements:** FR-NEW-002, 007, 009, 012
- **Given** Alice and Bob each have a clone of the project. **When** Alice creates annotated `release-2026` with message `RC`, publishes it, and Bob fetches. **Then** Bob's list shows `release-2026`, `annotated`, message `RC`, same commit. **And** the audit shows Alice's creation and publication.
- **Priority:** High

## Constraints
- No behaviour change in this story except the added test.
- Verify every old count is gone with `rg`, do not edit from memory.
- Dashes are not punctuation anywhere (project rule).
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: AGENTS.md, README.md, .agent_docs/*, see Paths.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] Closes spec: the spec's full acceptance set green (every E2E-001..085 and E2E-MOD-001, E2E-MOD-002), and every requirement of SPEC-0052 cited by at least one story with status done or landed.
- [ ] One commit `US-0091: documentation, round trip test and spec close`, body `Spec: SPEC-0052` and the requirement ids.
