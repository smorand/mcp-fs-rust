# US-0100: single publish unchanged, documentation and spec close

> Spec: SPEC-0054 (sdlc/specs/SPEC-0054-publish-all-tags/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0098, US-0099
> min_tier: 2
> Files touched: 6
> Closes spec: yes

## Objective
Close SPEC-0054: prove that publishing one tag behaves exactly as before after a bulk send (including the overwrite confirmation of a tag the bulk skipped), document the new tool, and run the requirement coverage audit and the full acceptance set.

## Requirements covered
- SPEC-0054/FR-NEW-016: The Remote exchange context SHALL NOT change the behavior of publishing one named tag.

## Independent Test
`cargo test -p mcp-fs-core --lib publish_all` green; `rg -n 'git.remote_push_tags' AGENTS.md .agent_docs/tools.md .agent_docs/git.md .agent_docs/git-divergences.md` finds all four docs.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0054): One new tool, `git.remote_push_tags(mount_id, remote="origin", confirm_count)`, next to `git.remote_push_tag` in the `git.remote_*` family. Tool count 115 to 116, `git.*` family 53 to 54. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-016. **Rationale:** FR-NEW-016 forbids changing the single publish; a separate tool keeps its contract exact. **Alternatives considered:** making `tag` optional on `git.remote_push_tag` (an either/or contract and an overwrite rule that bulk forbids); two tools, one to preview and one to send (the preview and the count check must share one classification, and the count guard is one action in the spec). **Source:** design. **Evidence:** `crates/core/src/mcp/server.rs:3530-3538`.
- DEC-015 (SPEC-0054): No existing test is modified; the 59 tests are all new. The assertions that change are counts, registry lengths and parity docs, as for every new tool. **Obliged by:** SPEC-0054/FR-NEW-016. **Alternatives considered:** none. **Source:** design.

### Data model
No schema change; no database write beyond what the single publish already does.

### Contracts
**`git.remote_push_tags`** (new in US-0096): parameters `mount_id` (string, required), `remote` (string, default `origin`, empty means `origin`), `confirm_count` (optional string or integer; absent or null is a preview). Result `{mount_id, remote, to_publish, published, sent, tags: [{name, outcome, commit, remote_commit, reason}], lines, message, auth}` with `outcome` one of `to publish`, `published`, `up to date`, `differs`, `commit not on remote`, `refused by remote`; `lines` hold the exact texts of the spec in byte order of the name. Refusals and codes: no remote `ERR_NOT_FOUND` `Cannot publish tags: this project tracks no remote.`; unknown remote `ERR_NOT_FOUND` `Cannot publish tags: this project does not track a remote named '<r>'.`; not a count `ERR_INVALID_ARGUMENT` `The number to confirm must be a whole number, zero or more.`; wrong number `ERR_NO_CLOBBER` `Tags to publish: <M>, not <N>. Review the list, then confirm again.` followed by one line per tag; connection, credential and timeout texts are the existing ones. Audit: action `git.tag_publish`, path `/`, `published tag '<n>' at <full 40>` per tag sent.

### Paths
| Path | Action |
|---|---|
| `AGENTS.md` | modify (mention the action) |
| `.agent_docs/tools.md` | modify (full row) |
| `.agent_docs/git.md` | modify (bulk publication paragraph: preview, count guard, outcomes, lease, audit, limits) |
| `.agent_docs/git-divergences.md` | modify (D15 note: BL-0076 delivered by SPEC-0054) |
| `crates/core/src/tools/git.rs` | modify (tests only) |

### Existing patterns
the SPEC-0053 closing story US-0095 (`sdlc/stories/US-0095-documentation-round-trip-and-spec-close.md`) and its docs diff; tests `e2e_tag_035_*` and `e2e_tag_036_*` of SPEC-0052 for the single publish shapes.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Test functions are named `e2e_tag_<number>_publish_all_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_publish_all_tags` with a `file://` bare remote (the registered tool for the non member and credential cases). Fixtures: project `proj-a`, remote `origin` at the host `github.ibm.com`, members Alice and Bob, commits `a1b2c3d`, `b2c3d4e`, `c3d4e5f` on the remote unless stated (use the real commit ids the fixtures produce).

### SPEC-0054/E2E-008: Bulk then single publish
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_008_publish_all_*`
- **Given** Alice published all tags with `v1.0` due. **When** she publishes `v1.0` alone with the one tag action. **Then** the answer is `up to date`. **And** no further audit entry exists; publishing a tag `v3.0` alone still sends only `v3.0`, with one new entry.
- **Priority:** High

### SPEC-0054/E2E-044: One tag publish keeps its overwrite rule after a bulk send
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_044_publish_all_*`
- **Given** the state of E2E-013 after the confirmed send (the remote `v1.0` is at `f6a7b8c`, local at `a1b2c3d`). **When** Alice publishes `v1.0` alone without confirming an overwrite. **Then** she is refused with `Remote tag 'v1.0' already exists at a different commit (f6a7b8c). Confirm overwriting to replace it.` **And** the remote `v1.0` is still at `f6a7b8c`.
- **Priority:** High

### SPEC-0054/E2E-048: Overwrite of a skipped tag stays a single tag decision
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_048_publish_all_*`
- **Given** the state of E2E-013 after the confirmed send. **When** Alice publishes `v1.0` alone confirming the overwrite and stating `f6a7b8c`. **Then** the remote `v1.0` is at `a1b2c3d`. **And** the audit trail has one new entry `published tag 'v1.0' at <full id of a1b2c3d>`.
- **Priority:** Medium

## Constraints
- No production code change.
- Verify every documented statement against the code, never from memory.
- Dashes are not punctuation (project rule).
- Files not to touch: anything under `sdlc/specs/` other than the status line at landing; other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered by an earlier story may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing git tag, publish and delete test passes unmodified; `tool_contract_golden_is_current` green.
- [ ] Docs updated: AGENTS.md, .agent_docs/{tools,git,git-divergences}.md.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] Closes spec: the spec's full acceptance set green (E2E-001..059), and every requirement of SPEC-0054 cited by at least one story with status done or landed.
- [ ] One commit `US-0100: single publish unchanged, documentation and spec close`, body `Spec: SPEC-0054` and the requirement ids.
