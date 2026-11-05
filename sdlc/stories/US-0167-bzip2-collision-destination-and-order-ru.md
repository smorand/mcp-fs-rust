# US-0167: check the bzip2 collision, destination and order rules

> Spec: SPEC-0083 (sdlc/specs/SPEC-0083-extract-bz2-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0166, US-0164
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The collision, destination and order of refusals rules, built for gzip, are pinned for bzip2 by their own tests, each proven to fail when its rule is broken.

## Requirements covered
- SPEC-0083/FR-NEW-004: For a bare `.bz2` file, IF the output path already holds a file AND `overwrite` is false THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' already exists; pass overwrite=true to replace it`, and write nothing. IF the output path holds a folder THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' is a folder and cannot be replaced`, with or without `overwrite`, and write nothing.
- SPEC-0083/FR-NEW-011: For a bare `.bz2` file, IF the destination is an existing file, or has a file among its ancestors, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'<path>' is a file, not a folder`, naming the file found, and write nothing.
- SPEC-0083/FR-NEW-012: For a bare `.bz2` file, IF several refusals apply to the same call THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, message `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, message `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, message `'<path>' is a directory, not an archive file`); (4) a password is given (FR-NEW-009); (5) the output name is unusable (FR-NEW-003); (6) the destination lies outside the project (`ERR_PATH_OUT_OF_BOUNDS`, message `path escapes the volume root: <destination>`); (7) the destination is a file or sits under a file (FR-NEW-011); (8) collision (FR-NEW-004); (9) corrupt (FR-NEW-008), project storage limit (FR-NEW-005) or session write allowance (FR-NEW-006), whichever decoding meets first, a tie going to the project storage limit.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0083_009 e2e_0083_010 e2e_0083_026 e2e_0083_027 e2e_0083_028 e2e_0083_029 e2e_0083_030 e2e_0083_031 e2e_0083_032 e2e_0083_033 e2e_0083_034 e2e_0083_035 e2e_0083_036 e2e_0083_037 e2e_0083_050`: all 15 tests pass. After every refused call the store holds no new file and no new folder.

## Design excerpt
### Decisions
- **DEC-001:** bzip2 reuses the single file flow and rules of SPEC-0081 (`extract_single`, `read_capped`, caps, admission, order); its stories depend on the SPEC-0081 stories that build each rule. **Obliged by:** SPEC-0083/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine for gzip, bzip2 and xz. **Alternatives considered:** a parallel flow. **Source:** SPEC-0081 DEC-001, DEC-005, DEC-007 (existing design binds). **Evidence:** `sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-design.md`.
### Data model
None.
### Contracts
No code contract: the rules are SPEC-0081's `check_destination`, `check_collision` and the order of `extract_single`. Only tests are added, plus a code change if a test proves bzip2 differs.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
SPEC-0081 stories US-0162 and US-0164 for the rules; their test helpers in `archive_single.rs`. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0083/E2E-009: Collision
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** `/uploads/data.csv.bz2` and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' already exists; pass overwrite=true to replace it`
- **And** `/uploads/data.csv` still reads `old`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_009_collision`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-010: A folder bears the output name
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** Low
- **Given** `/uploads/data.csv.bz2` and a folder `/uploads/data.csv`
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' is a folder and cannot be replaced`; the folder is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_010_a_folder_bears_the_output_name`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-026: Missing archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Low
- **Given** no file at `/uploads/ghost.bz2`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.bz2' not found`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_026_missing_archive`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-027: Folder named like an archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a folder `/uploads/dir.bz2`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.bz2' is a directory, not an archive file`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_027_folder_named_like_an_archive`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-028: Non member
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** High
- **Given** `/uploads/data.csv.bz2` in project `alpha` and `bob@example.com`, who is not a member of `alpha`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_028_non_member`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-029: Destination is a file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`; `/uploads/notes.txt` is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_029_destination_is_a_file`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-030: A file among the ancestors of the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt/sub`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_030_a_file_among_the_ancestors_of_the_de`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-031: Destination is a file, through the HTTP API
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** the situation of E2E-029
- **When** a client asks the HTTP API to extract it to the destination `/uploads/notes.txt`
- **Then** the error code and message are those of E2E-029
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0083_031_destination_is_a_file_through_the_ht`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-032: Destination outside the project wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/fake.bz2` containing the text `not bzip2`
- **When** the member extracts it with the destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not the corrupt refusal; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_032_destination_outside_the_project_wins`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-033: Password wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/fake.bz2` containing the text `not bzip2`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_033_password_wins_over_a_corrupt_file`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-034: Collision wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/cut.bz2` truncated and an existing `/uploads/cut` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not the corrupt refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_034_collision_wins_over_a_corrupt_file`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-035: Collision wins over the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012, FR-NEW-005; **Priority:** Medium
- **Given** the situation of E2E-011 and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not `ERR_PROJECT_QUOTA_EXCEEDED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_035_collision_wins_over_the_storage_limi`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-036: Destination is a file, wins over a collision
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011, FR-NEW-012; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2`, a file `/uploads/notes.txt` and an existing `/uploads/data.csv`
- **When** the member extracts it to the destination `/uploads/notes.txt` with `overwrite` false
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`, not `ERR_NO_CLOBBER`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_036_destination_is_a_file_wins_over_a_co`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-037: Unusable name wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-012, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.bz2` containing the text `not bzip2`
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.bz2'`, not the corrupt refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_037_unusable_name_wins_over_a_corrupt_fi`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-050: Overwrite replaces a file
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2` and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` true
- **Then** `/uploads/data.csv` reads the 16 bytes of the content
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_050_overwrite_replaces_a_file`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-009 | remove the existence check in `check_collision`: the test fails |
| E2E-010 | remove the existence check in `check_collision`: the test fails |
| E2E-026 | move the named check after the decode: the test fails |
| E2E-027 | move the named check after the decode: the test fails |
| E2E-028 | move the named check after the decode: the test fails |
| E2E-029 | remove `check_destination`: the test fails |
| E2E-030 | remove `check_destination`: the test fails |
| E2E-031 | remove `check_destination`: the test fails |
| E2E-032 | move the named check after the decode: the test fails |
| E2E-033 | move the named check after the decode: the test fails |
| E2E-034 | move the named check after the decode: the test fails |
| E2E-035 | move the named check after the decode: the test fails |
| E2E-036 | remove `check_destination`: the test fails |
| E2E-037 | move the named check after the decode: the test fails |
| E2E-050 | remove the existence check in `check_collision`: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `archive.rs`
- Dependencies not to add: none
- If a test fails on arrival the story fixes the shared rule for all formats and says so in its commit body
- Scope boundary: limits belong to US-0168

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-009, E2E-010, E2E-026, E2E-027, E2E-028, E2E-029, E2E-030, E2E-031, E2E-032, E2E-033, E2E-034, E2E-035, E2E-036, E2E-037, E2E-050): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0167: check the bzip2 collision, destination and order rules`, body `Spec: SPEC-0083` and the requirement ids.
