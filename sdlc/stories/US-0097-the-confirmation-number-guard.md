# US-0097: the confirmation number guard

> Spec: SPEC-0054 (sdlc/specs/SPEC-0054-publish-all-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0096
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
Make the stated number a real guard: a malformed value gets `The number to confirm must be a whole number, zero or more.`, a number that differs from the count of tags to publish gets the wrong number refusal with the fresh list, and a project with nothing to publish answers `No tag to publish.` Zero is a number and leading zeros are accepted.

## Requirements covered
- SPEC-0054/FR-NEW-003: IF the number a member states differs from the count of tags to publish at that moment, THEN THE Remote exchange context SHALL refuse, show the fresh list and leave the remote untouched.
- SPEC-0054/FR-NEW-004 (partial: nothing to publish with no tag due to the number rules; skipped outcomes arrive in US-0098): IF no local tag is to publish and the member states no number or the number 0, THEN THE Remote exchange context SHALL answer `No tag to publish.` with the list of FR-NEW-001 and send nothing.
- SPEC-0054/FR-NEW-015 (partial: member, remote, then the number; the connection position is checked in US-0099): The Remote exchange context SHALL give refusals in this order: non member, remote lookup, connection and credential, then a number that is not a count (FR-NEW-019) or a wrong number (FR-NEW-003).
- SPEC-0054/FR-NEW-019: IF the stated number is not a whole number of zero or more, THEN THE Remote exchange context SHALL refuse with `The number to confirm must be a whole number, zero or more.` and send nothing.

## Independent Test
`cargo test -p mcp-fs-core --lib publish_all` green; stating `1` while 2 tags are due leaves the remote byte identical (`bare_refs` equal before and after) and returns the refusal whose first line is `Tags to publish: 2, not 1. Review the list, then confirm again.`

## Design excerpt
### Decisions
- DEC-005 (SPEC-0054): Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then `parse_confirm` (not a count), then the count comparison (wrong number), then the push. Codes in Section 5. **Obliged by:** SPEC-0054/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-015, FR-NEW-019. **Alternatives considered:** parsing the number before the connection (rejected: FR-NEW-015 puts the connection first, E2E-050). **Source:** design. **Evidence:** `git_tags.rs:467-505`.
- DEC-008 (SPEC-0054): The count guard: `confirm_count` is `Option<serde_json::Value>`. Absent or null is a preview. A string of 1 to 9 ASCII digits or a JSON integer between 0 and 999999999 is a number. Anything else (empty string, sign, space, decimal, exponent, other script, longer than 9 digits, boolean, array, object, negative or fractional number) is the number refusal. A number different from the count of tags to publish is the wrong number refusal, and the count of 0 with a stated 0 is the "No tag to publish." answer. **Obliged by:** SPEC-0054/FR-NEW-003, FR-NEW-004, FR-NEW-019. **Rationale:** a typed integer parameter would let the transport reject `two` with its own message; a raw value lets the refusal text be the spec's. **Alternatives considered:** `Option<u32>` (transport message, FR-NEW-019 broken); `Option<String>` only (an MCP client sending the integer 2 would fail deserialization). **Source:** design. **Evidence:** `mcp/server.rs:1121-1136` (the args pattern).
- DEC-009 (SPEC-0054): Output rendering: one `render(tags)` function builds `lines` and the multi line error message from the same list, so the preview, the report and the wrong number refusal cannot drift (FR-NEW-001, 002, 003 share the wording). The refusal line is always first. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004. **Alternatives considered:** three formatters. **Source:** design.
- DEC-007 (SPEC-0054): The whole action holds the repository `write_lock`, preview included, so the count shown and the count checked come from one consistent view and two confirmations serialize; the second finds nothing to publish and gets the wrong number refusal with `Tags to publish: 0, not N` (FR-NEW-013). `reject_if_operation_in_progress` is NOT called (FR-NEW-014). The span is `trace_remote_operation("git.remote_push_tags", ...)`. **Obliged by:** SPEC-0054/FR-NEW-013, FR-NEW-014. **Alternatives considered:** no lock for the preview (a preview racing a confirm could show a stale count; harmless but untestable); taking the lock only for the send (the count check would race). **Source:** existing code. **Evidence:** `git_tags.rs:560`.
- DEC-015 (SPEC-0054): No existing test is modified; the 59 tests are all new. The assertions that change are counts, registry lengths and parity docs, as for every new tool. **Obliged by:** SPEC-0054/FR-NEW-016. **Alternatives considered:** none. **Source:** design.

### Data model
No schema change; no database write beyond what the single publish already does.

### Contracts
**`git.remote_push_tags`** (new in US-0096): parameters `mount_id` (string, required), `remote` (string, default `origin`, empty means `origin`), `confirm_count` (optional string or integer; absent or null is a preview). Result `{mount_id, remote, to_publish, published, sent, tags: [{name, outcome, commit, remote_commit, reason}], lines, message, auth}` with `outcome` one of `to publish`, `published`, `up to date`, `differs`, `commit not on remote`, `refused by remote`; `lines` hold the exact texts of the spec in byte order of the name. Refusals and codes: no remote `ERR_NOT_FOUND` `Cannot publish tags: this project tracks no remote.`; unknown remote `ERR_NOT_FOUND` `Cannot publish tags: this project does not track a remote named '<r>'.`; not a count `ERR_INVALID_ARGUMENT` `The number to confirm must be a whole number, zero or more.`; wrong number `ERR_NO_CLOBBER` `Tags to publish: <M>, not <N>. Review the list, then confirm again.` followed by one line per tag; connection, credential and timeout texts are the existing ones. Audit: action `git.tag_publish`, path `/`, `published tag '<n>' at <full 40>` per tag sent.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (`parse_confirm`, count check, `render` of the refusal) |
| `crates/core/src/tools/git.rs` | modify (14 tests) |

### Existing patterns
`parse_confirm`, `classify_tags` and `render` from US-0096; test helpers `call_publish_all_tags`, `bare_refs`, `publish_audit` in `crates/core/src/tools/git.rs`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Test functions are named `e2e_tag_<number>_publish_all_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_publish_all_tags` with a `file://` bare remote (the registered tool for the non member and credential cases). Fixtures: project `proj-a`, remote `origin` at the host `github.ibm.com`, members Alice and Bob, commits `a1b2c3d`, `b2c3d4e`, `c3d4e5f` on the remote unless stated (use the real commit ids the fixtures produce).

### SPEC-0054/E2E-009: Number too low
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_009_publish_all_*`
- **Given** two tags due (`v1.0`, `v1.1`). **When** Alice confirms stating 1. **Then** she is refused with `Tags to publish: 2, not 1. Review the list, then confirm again.` followed by the list. **And** the remote has neither tag and the audit trail has no new entry.
- **Priority:** Critical

### SPEC-0054/E2E-010: Number too high
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_010_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating 5. **Then** she is refused with `Tags to publish: 2, not 5. Review the list, then confirm again.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

### SPEC-0054/E2E-011: Zero is a number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_011_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating 0. **Then** she is refused with `Tags to publish: 2, not 0. Review the list, then confirm again.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

### SPEC-0054/E2E-012: The count changed after the preview
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_012_publish_all_*`
- **Given** Alice previewed 2 tags due (`v1.0`, `v1.1`). **When** Bob publishes `v1.1` alone, then Alice confirms stating 2. **Then** Alice is refused with `Tags to publish: 1, not 2. Review the list, then confirm again.` and the list shows `v1.0 to publish`, `v1.1 up to date`. **And** the remote holds `v1.1` (Bob's) and not `v1.0`.
- **Priority:** High

### SPEC-0054/E2E-024: A project with no tag
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_024_publish_all_*`
- **Given** a project with no tag. **When** Alice previews. **Then** the answer is exactly the one line `No tag to publish.` **And** nothing is recorded in the audit trail.
- **Priority:** Medium

### SPEC-0054/E2E-025: Everything is up to date
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_025_publish_all_*`
- **Given** the local tags `v1.0` and `v1.1` held identically by the remote. **When** Alice previews, then confirms stating 0. **Then** both answers are exactly `v1.0 up to date`, `v1.1 up to date`, `No tag to publish.` **And** the audit trail is unchanged.
- **Priority:** Medium

### SPEC-0054/E2E-028: No remote beats a wrong number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_028_publish_all_*`
- **Given** a project with no remote and 2 tags. **When** Alice confirms stating 7. **Then** the answer is `Cannot publish tags: this project tracks no remote.`
- **Priority:** Medium

### SPEC-0054/E2E-038: A negative number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_038_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating `-1`. **Then** the answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

### SPEC-0054/E2E-039: Non member beats no remote
- Level / Driver / Location: functional e2e / registered tool, `e.call` / `crates/core/src/tools/git.rs`, `e2e_tag_039_publish_all_*`
- **Given** Mallory is not a member of `proj-a`, which tracks no remote. **When** Mallory confirms stating 7. **Then** the answer is `'mallory@example.com' is not a member of 'proj-a'` and not the no remote text.
- **Priority:** Medium

### SPEC-0054/E2E-042: A new tag appears after the preview
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_042_publish_all_*`
- **Given** Alice previewed `Tags to publish: 2` (`v1.0`, `v1.1`). **When** Bob creates `v1.2` at `c3d4e5f` locally in the same project and Alice confirms stating 2. **Then** Alice is refused with `Tags to publish: 3, not 2. Review the list, then confirm again.` and the list shows `v1.0 to publish`, `v1.1 to publish`, `v1.2 to publish`. **And** the remote holds none of them.
- **Priority:** High

### SPEC-0054/E2E-049: A decimal number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_049_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating `2.5`. **Then** the answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged.
- **Priority:** Medium

### SPEC-0054/E2E-052: Leading zeros are a number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_052_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating `02`. **Then** the two tags are sent and the report has two `published` lines. **And** stating `0000000002` (ten characters) gives `The number to confirm must be a whole number, zero or more.`
- **Priority:** Medium

### SPEC-0054/E2E-053: Signs and spaces are not a number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_053_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating `+2`, then ` 2` (a space then 2). **Then** each answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged.
- **Priority:** Medium

### SPEC-0054/E2E-059: An empty number is not a number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_059_publish_all_*`
- **Given** two tags due. **When** Alice confirms stating an empty text. **Then** the answer is `The number to confirm must be a whole number, zero or more.` **And** the remote and the audit trail are unchanged; leaving the number out entirely gives the preview of E2E-001's shape.
- **Priority:** Medium

## Constraints
- Files not to touch: `crates/core/src/git/remote.rs`, `crates/core/src/mcp/server.rs`, contract files, anything under `sdlc/specs/`.
- Dependencies not to add: none.
- Patterns to avoid: a typed integer parameter (the transport would answer `two` with its own message, DEC-008).
- Scope boundary: skipped and refused tags (US-0098); connection failures and concurrency (US-0099).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered by an earlier story may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every US-0096 test and every existing git test passes unmodified.
- [ ] Docs updated: none beyond code comments.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0097: the confirmation number guard`, body `Spec: SPEC-0054` and the requirement ids.
