# US-0056: refuse what cannot be compared

> Spec: SPEC-0045 (sdlc/specs/SPEC-0045-fs-diff-two-files/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0055
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A refused comparison names the project and path at fault with the exact messages of the spec, and the checks run in the fixed order of FR-NEW-015. Adds `VolumeClient::read_text_checked` for the not found and directory cases and switches the orchestrator to it.

## Requirements covered
- SPEC-0045/FR-NEW-006: IF a file does not exist THEN THE product SHALL refuse the comparison and name that file.
- SPEC-0045/FR-NEW-007: IF a path designates a directory THEN THE product SHALL refuse the comparison and name that path.
- SPEC-0045/FR-NEW-014: IF the project of either file has been deleted THEN THE product SHALL refuse the comparison with the same message as any other read in that project.
- SPEC-0045/FR-NEW-015: The product SHALL run these checks in this order and SHALL report only the first problem found: (1) the path of the first file is accepted by the product's path normalization (FR-NEW-017); (2) the same for the second path; (3) the first file's project exists and the caller is a member of it; (4) the same for the second file's project; (5) the first file exists; (6) the first file is not a directory; (7) the first file is within the size limit; (8) the first file is a text file; (9) to (12) the same four checks for the second file.
- SPEC-0045/FR-NEW-017 (refusal and header behavior of path normalization): The product SHALL normalize the path of each file exactly as every other file operation does, and SHALL show the normalized path in refusals and in the diff headers.

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e` : the 14 tests above pass in addition to US-0055 tests. E2E-101 returns exactly `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`; E2E-110 returns `ERR_FORBIDDEN` for Bob and `'alpha:/notes' is a directory` for Alice on the same request. Real storage, no mocks.

## Design excerpt
### Decisions
- (SPEC-0045) DEC-004: Order: normalize path A, normalize path B, authorize and open A, authorize and open B, read A completely, read B completely. **Obliged by:** FR-NEW-015. **Rationale:** matches the spec's twelve checks; `AppState::authorize` already reports a deleted project before membership. **Alternatives considered:** reading both files concurrently (would reorder errors). **Source:** spec. **Evidence:** `mcp/server.rs:178-181`, `storage/admin.rs:392-395`.
- (SPEC-0045) DEC-005: New `VolumeClient::read_text_checked(label, path, max_bytes)`: node lookup (not found), directory, size from `NodeRow.size` before any blob read, then raw bytes checked for a NUL byte and `String::from_utf8`. **Obliged by:** FR-NEW-006, 007, 008, 009. **Rationale:** the existing `read_text` is lossy and unlabelled (`volume.rs:119-122`, `:80`); changing it would alter every read. **Alternatives considered:** reuse `read_text` (cannot detect non text); change `read_text` (behavior change for every tool). **Source:** design. **Evidence:** `storage/volume.rs:76-105`, `storage/traits.rs:19`.
### Data model
None.
### Contracts
```rust
// storage/volume.rs
pub async fn read_text_checked(&self, label: &str, path: &str, max_bytes: u64) -> Result<String>;
```
In this story `max_bytes` is accepted and not yet applied (US-0057 applies size and text checks); pass `u64::MAX` from the orchestrator. Behavior now: node lookup via `self.meta.get(path)`; `None` gives `ToolError::not_found(format!("'{label}' not found"))`; a directory gives `ToolError::invalid_argument(format!("'{label}' is a directory"))`; a file with no `sha256` is the empty string; otherwise `self.blob.get(&sha, 0, None)` then the existing lossy decode (`String::from_utf8_lossy`) for now.
In `tools/diff_files.rs`, the read of A completes (all its checks) before B is touched. `label = format!("{mount_id}:{normalized_path}")`.
Messages (exact): `ERR_NOT_FOUND: '<label>' not found`; `ERR_INVALID_ARGUMENT: '<label>' is a directory`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/volume.rs` | modify |
| `crates/core/src/tools/diff_files.rs` | modify |
| `crates/core/src/tools/diff_files_e2e.rs` | modify |
### Existing patterns
- `crates/core/src/storage/volume.rs:76-105` (`stat`, `read_bytes`): same `self.meta.get`, `ToolError::*` and `self.blob.get(&sha, 0, None)` calls, with the label added to the messages. Do not change `stat`, `read_bytes`, `read_text`.
- Test harness and helpers as in `crates/core/src/tools/quota_e2e.rs:1-70`. Deleted project: `f.admin_store.soft_delete_project("beta").await`; restore: `f.admin_store.undelete_project("beta").await` (see `crates/core/src/mcp/server.rs:4861-4890` for the soft delete form).

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions of all tests (copied from the spec):

Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

### SPEC-0045/E2E-101: First file missing
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-006, 003. **Priority:** High
- **Given** the common data, `alpha:/notes/missing.txt` does not exist
- **When** Alice compares `alpha:/notes/missing.txt` with `alpha:/notes/b.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`, and no diff
- **And** the answer carries no identical indicator and no diff text
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-102: Second file missing
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-006. **Priority:** High
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes/missing.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`, and no diff
- **And** the answer carries no "identical" indicator and no diff text, and the second file is the one named
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-103: A directory
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-007. **Priority:** High
- **Given** the common data
- **When** Alice compares `alpha:/notes` with `alpha:/notes/b.txt`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-109: Deleted project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (inherited: a deleted project is reported unknown by `AppState::authorize`)
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-014. **Priority:** High
- **Given** `beta` has been deleted; Alice was a member
- **When** Alice compares `alpha:/notes/a.txt` with `beta:/cfg/app.txt`
- **Then** `ERR_PROJECT_NOT_FOUND: project 'beta' not found`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-110: Several problems, fixed order
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015. **Priority:** High
- **Given** Bob a member of `alpha` only; `alpha:/notes` a directory; `beta:/cfg/nothing.txt` does not exist
- **When** Bob compares `alpha:/notes` (a directory) with `beta:/cfg/nothing.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'` (access outranks the directory problem)
- **And** when Alice (member of both) makes the same request, the answer is
  `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory` (first file outranks the missing second file)
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-111: Climbing above the top stops at the top
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 017. **Priority:** Medium
- **Given** the common data; `alpha` has no `/etc/passwd`
- **When** Alice compares `alpha:/notes/../../etc/passwd` with `alpha:/notes/b.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/etc/passwd' not found`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-212: Second path is a directory
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007, 015. **Priority:** High
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory`, and no diff
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-217: First file in a deleted project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (inherited, same reason)
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-014. **Priority:** High
- **Given** `beta` deleted
- **When** Alice compares `beta:/cfg/app.txt` with `alpha:/notes/a.txt`
- **Then** `ERR_PROJECT_NOT_FOUND: project 'beta' not found`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-218: Deleted project brought back
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (inherited)
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-014, 004. **Priority:** Medium
- **Given** `beta` deleted, then restored by an administrator
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`
- **Then** she receives the same diff as in E2E-002
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-219: Missing second file in another project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-006. **Priority:** Medium
- **Given** E2E-002 data, `beta:/cfg/missing.txt` does not exist
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/missing.txt`
- **Then** `ERR_NOT_FOUND: 'beta:/cfg/missing.txt' not found`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-222: Climbing above the top, second file, other project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-017. **Priority:** Medium
- **Given** the common data; `beta` has no `/secret.txt`
- **When** Alice compares `alpha:/notes/a.txt` with `beta:/../secret.txt`
- **Then** `ERR_NOT_FOUND: 'beta:/secret.txt' not found`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-226: Path normalization does not outrank access
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (inherited authorization order)
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 017. **Priority:** Medium
- **Given** Bob (member of `alpha` only)
- **When** Bob compares `alpha:/notes/a.txt` with `beta:/../x.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-228: Path with a NUL character
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (inherited `normalize_path` NUL refusal)
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 017. **Priority:** Medium
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with a path made of `/notes/b.txt` followed by a NUL character
- **Then** `ERR_PATH_OUT_OF_BOUNDS: path contains a NUL byte`, and no file is looked up
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-229: Empty path on the tool surface
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-015, 018. **Priority:** Medium
- **Given** the common data
- **When** Alice compares `alpha:/notes/a.txt` with project `alpha` and an empty path, through the tool surface
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/' is a directory`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

## Constraints
- Files not to touch: `core/fs_ops.rs`, `core/diff.rs`, `config.rs`, `api/*`, golden files.
- Dependencies not to add: none.
- Patterns to avoid: changing any existing message; checking both files before reading the first.
- Scope boundary: text and size checks (US-0057), last access (US-0058), REST (US-0059).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, except the tests marked "Expectation on arrival" above, whose green status on arrival is recorded as such.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: Every existing test passes unmodified; `read_text`, `read_bytes`, `stat` are untouched.
- [ ] Docs updated: none in this story.
- [ ] Self review (full): every requirement above is demonstrably met by a named test; no `unwrap()` on a fallible path; no dash as punctuation in code, comments or docs; errors carry the exact texts of this story.
- [ ] One commit `US-0056: refuse what cannot be compared`, body `Spec: SPEC-0045` and the requirement ids.
