# US-0173: refuse multi volume names, corrupt files, a missing path and a folder

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0172
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A `name.partN.rar` file is refused before it is read with a clear message; a truncated, fake or empty `.rar` is the corrupt refusal; a missing path and a folder get the shared refusals for `.rar` too.

## Requirements covered
- SPEC-0080/FR-NEW-010: IF the server has RAR support AND the file name, ignoring letter case, ends in `.part` followed by one or more digits followed by `.rar` THEN THE archive extraction SHALL refuse with `ERR_NOT_SUPPORTED` and the message `multi volume RAR archives are not supported: extract a single file archive`, and write nothing.
- SPEC-0080/FR-NEW-011: IF a file named `.rar` is empty, is not a RAR archive or is truncated THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid rar file:`, and write nothing.
- SPEC-0080/FR-NEW-019: IF the path does not exist THEN THE archive extraction SHALL refuse with `ERR_NOT_FOUND` and the message `'<path>' not found`, and write nothing.
- SPEC-0080/FR-NEW-021: IF the path is a folder THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'<path>' is a directory, not an archive file`, and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_013 e2e_0080_014 e2e_0080_016 e2e_0080_023 e2e_0080_024 e2e_0080_025 e2e_0080_026 e2e_0080_027 e2e_0080_056 e2e_0080_071 e2e_0080_074 e2e_0080_075 e2e_0080_076`: all 13 tests pass. After a refused multi volume call the file was not read: the test asserts no temp file is left and no entry is written.

## Design excerpt
### Decisions
- **DEC-002:** `.rar` detection is a cfg'd table entry after the tar entries; `is_multi_volume_name` runs after format detection and before the file is read, only when the feature is on; with the feature off `.part1.rar` falls to the unsupported extension message. **Obliged by:** SPEC-0080/FR-NEW-010, FR-NEW-012, FR-NEW-015. **Rationale:** FR-NEW-012 includes multi volume names. **Alternatives considered:** check in `decode_rar` (too late, the file is read). **Source:** spec. **Evidence:** `archive.rs:40-69`, `:146-150`.
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
### Data model
None.
### Contracts
`is_multi_volume_name(file_name) -> bool` (design section 5); in `extract_archive` after `detect_format` and before the file is read: `if matches!(format, ArchiveFormat::Rar) && is_multi_volume_name(file_name) { return Err(ToolError::not_supported("multi volume RAR archives are not supported: extract a single file archive")); }`. Corrupt: `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid rar file: <detail>`. Missing path `'<path>' not found`, folder `'<path>' is a directory, not an archive file` (existing, `archive.rs:130-141`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
The password and detect_format sequence (`archive.rs:143-151`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-013: Multi volume name
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** a server with RAR support and `/uploads/big.part1.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `multi volume RAR archives are not supported: extract a single file archive`; nothing is written
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_013_multi_volume_name`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-014: Truncated RAR
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-011
- **Given** `/uploads/broken.rar`, the first half of a valid RAR
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing is written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_014_truncated_rar`
- Green on arrival: the corrupt mapping exists since US-0170
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-016: Missing archive
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-019
- **Given** no file at `/uploads/ghost.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.rar' not found`
- **Priority:** Low
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_016_missing_archive`
- Green on arrival: shared check unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-023: `.part02.rar` is multi volume
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** a server with RAR support and `/uploads/big.part02.rar`
- **When** the member extracts it
- **Then** the refusal and message are those of E2E-013
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_023_part02_rar_is_multi_volume`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-024: `part1x` is not a multi volume name
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** `/uploads/report.part1x.rar` and `/uploads/big.part.rar`, two valid RARs each holding `a.txt` (5 bytes)
- **When** the member extracts each
- **Then** the destinations are `/uploads/report.part1x` and `/uploads/big.part` and 1 file is written each time
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_024_part1x_is_not_a_multi_volume_name`
- Green on arrival: a name that only looks multi volume is an ordinary RAR since US-0170
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-025: Upper case multi volume name
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-010
- **Given** a server with RAR support and `/uploads/BIG.PART3.RAR`
- **When** the member extracts it
- **Then** the refusal and message are those of E2E-013
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_025_upper_case_multi_volume_name`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-026: A text file named `.rar`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-011
- **Given** `/uploads/fake.rar` containing the text `not an archive`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing is written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_026_a_text_file_named_rar`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-027: A 0 byte `.rar`
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-011
- **Given** `/uploads/empty.rar` with 0 bytes
- **When** the member extracts it
- **Then** the refusal is as E2E-026; `/uploads/empty` does not exist
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_027_a_0_byte_rar`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-056: Path is a folder
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-021
- **Given** a folder `/uploads/dir.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.rar' is a directory, not an archive file`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_056_path_is_a_folder`
- Green on arrival: shared check unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-071: Missing archive through the HTTP API
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-019; **Priority:** Low
- **Given** no file at `/uploads/ghost.rar`
- **When** a client asks the HTTP API to extract it
- **Then** the error code is `ERR_NOT_FOUND` with the message `'/uploads/ghost.rar' not found`
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_071_missing_archive_through_the_http_api`
- Green on arrival: shared check unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-074: Missing archive with an explicit destination
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-019; **Priority:** Low
- **Given** no file at `/uploads/ghost.rar`
- **When** the member extracts it to the destination `/out`
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.rar' not found`; `/out` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_074_missing_archive_with_an_explicit_des`
- Green on arrival: shared check unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-075: Folder named with an upper case suffix
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-021; **Priority:** Medium
- **Given** a folder `/uploads/DIR.RAR`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/DIR.RAR' is a directory, not an archive file`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_075_folder_named_with_an_upper_case_suff`
- Green on arrival: shared check unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-076: Folder through the HTTP API
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-021; **Priority:** Medium
- **Given** a folder `/uploads/dir.rar`
- **When** a client asks the HTTP API to extract it
- **Then** the error code and message are those of E2E-056
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_076_folder_through_the_http_api`
- Green on arrival: shared check unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-014 | map a header error to `ERR_INTERNAL_ERROR`: the test fails |
| E2E-016 | regression only, no mutation |
| E2E-024 | let `is_multi_volume_name` match `part1x`: the test fails |
| E2E-026 | same |
| E2E-027 | same |
| E2E-056 | regression only |
| E2E-071 | regression only |
| E2E-074 | regression only |
| E2E-075 | regression only |
| E2E-076 | regression only |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Scope boundary: passwords, versions, order belong to later stories

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-014, E2E-016, E2E-024, E2E-026, E2E-027, E2E-056, E2E-071, E2E-074, E2E-075, E2E-076): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0173: refuse multi volume names, corrupt files, a missing path and a folder`, body `Spec: SPEC-0080` and the requirement ids.
