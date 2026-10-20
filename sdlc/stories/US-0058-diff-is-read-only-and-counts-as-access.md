# US-0058: a comparison is read only and counts as access

> Spec: SPEC-0045 (sdlc/specs/SPEC-0045-fs-diff-two-files/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0055
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
After a successful comparison both files have a fresh last access date within 2 seconds, a refused comparison updates nothing, and a comparison never counts as having read the files for the read before edit rule. The tests that lock in "nothing is written, nothing is recorded" guard against later regressions.

## Requirements covered
- SPEC-0045/FR-NEW-010: The product SHALL NOT change either file, either project's used storage, or the session write allowance when comparing files.
- SPEC-0045/FR-NEW-011: The product SHALL NOT treat a comparison as having read the files for the read before edit rule.
- SPEC-0045/FR-NEW-012: WHEN a comparison succeeds THE product SHALL update the last access date of both files within 2 seconds of the answer.

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e` : the 7 tests above pass. E2E-006 and E2E-215 observe `VolumeClient::stat(path).atime` newer than before, polled for up to 2 s; E2E-007 observes `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session` from the real `fs_edit` handler after a comparison.

## Design excerpt
### Decisions
- (SPEC-0045) DEC-006: On success only, call `touch_atime` on both files; never call `safety.record_read`. **Obliged by:** FR-NEW-011, FR-NEW-012. **Rationale:** `read_window` does both (`fs_ops.rs:65-66`); the spec wants only the second. **Alternatives considered:** touch before the diff (would touch on a refusal). **Source:** spec. **Evidence:** `core/fs_ops.rs:65-66`.
### Data model
None.
### Contracts
In `tools/diff_files.rs`, after the answer is computed and before returning `Ok`, call `client_a.touch_atime(&path_a)` and `client_b.touch_atime(&path_b)` (both fire and forget, `storage/volume.rs:57-63`). On any error return before reaching them. Never call `state.safety.record_read`.
To observe the session write allowance and storage (E2E-005, E2E-213): `crate::quota::used_bytes` for each project before and after (see `quota.rs`, used by `admin.get_project_quota`), and the file bytes read back with `client.read_bytes`. If no accessor for the remaining session allowance is public, assert on used bytes and file content only and state that in the test comment.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/diff_files.rs` | modify |
| `crates/core/src/tools/diff_files_e2e.rs` | modify |
### Existing patterns
- Last access call pair: `crates/core/src/core/fs_ops.rs:65-66` (`safety.record_read(...)` then `client.touch_atime(path)`): you keep the second line only.
- Edit through the real handler: `crates/core/src/tools/quota_e2e.rs` (`EditArgs`, `server.fs_edit(Parameters(..))`) and error text via `err_text`.

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions of all tests (copied from the spec):

Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

### SPEC-0045/E2E-005: Comparing changes nothing
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (nothing writes)
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** the common data; before: the content of both files, the used storage of `alpha`, and the session
  write allowance Alice has left
- **When** Alice compares `a.txt` with `b.txt` three times
- **Then** both files have the same content, the used storage of `alpha` is the same, and the write allowance
  left is the same as before
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-006: Last access updated
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-012. **Priority:** Medium
- **Given** the common data and last access dates of both files recorded
- **When** Alice compares them, then, separately, compares `a.txt` with `alpha:/notes/missing.txt` (refused)
- **Then** within 2 seconds after the first comparison, as shown by the existing file status operation, both dates are newer; after the refused one `a.txt` keeps the date it
  had after the first
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-007: Comparing is not reading
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (nothing records a read)
- **Category:** side effect. **Scenario:** SC-001. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** the common data and a new session for Alice
- **When** Alice compares `a.txt` with `b.txt`, then tries to replace `two` with `2` in `a.txt`
- **Then** the edit is refused with `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session`
- **And** after Alice reads `a.txt` the same edit succeeds
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-213: Cross project comparison changes nothing
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (nothing writes)
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-010. **Priority:** High
- **Given** E2E-002 data; before: content of both files and used storage of `alpha` and `beta`
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`, and also makes a refused comparison
  against `beta:/cfg/missing.txt`
- **Then** both files, `alpha` used storage and `beta` used storage are exactly as before
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-214: Edit of the second file after a comparison
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (nothing records a read)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-011. **Priority:** High
- **Given** the common data and a new session for Alice
- **When** Alice compares `a.txt` with `b.txt`, then tries to replace `2` with `two` in `b.txt`
- **Then** `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/b.txt' requires reading it first in this session`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-215: Last access in each project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** side effect. **Scenario:** SC-002. **Requirements:** FR-NEW-012. **Priority:** Medium
- **Given** E2E-002 data and the last access dates of both files recorded
- **When** Alice compares them
- **Then** within 2 seconds, as shown by the existing file status operation, the last access date of `alpha:/cfg/app.txt` and of `beta:/cfg/app.txt` are both newer
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-216: Identical files still count as accessed
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-012, 003, 011. **Priority:** Medium
- **Given** E2E-003 data, a new session for Alice and the last access dates recorded
- **When** Alice compares the identical files, then tries to replace `two` with `2` in `a.txt`
- **Then** identical true, and within 2 seconds both last access dates are newer
- **And** the edit is refused with `ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

## Constraints
- Files not to touch: everything except the two paths above.
- Dependencies not to add: none.
- Patterns to avoid: `record_read`; touching last access on a refusal.
- Scope boundary: REST (US-0059), docs (US-0060).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, except the tests marked "Expectation on arrival" above, whose green status on arrival is recorded as such.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: Every existing test passes unmodified.
- [ ] Docs updated: none in this story.
- [ ] Self review (full): every requirement above is demonstrably met by a named test; no `unwrap()` on a fallible path; no dash as punctuation in code, comments or docs; errors carry the exact texts of this story.
- [ ] One commit `US-0058: a comparison is read only and counts as access`, body `Spec: SPEC-0045` and the requirement ids.
