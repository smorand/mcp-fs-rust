# US-0162: gzip password, collision and destination rules

> Spec: SPEC-0081 (sdlc/specs/SPEC-0081-extract-gz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0161
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A password for a bare gzip file is refused before the file is read. The destination is checked (outside the project, a file, under a file) and the output path is checked (a file needs `overwrite`, a folder is never replaced), all before any decoding. Tar suffixes stay tar.

## Requirements covered
- SPEC-0081/FR-NEW-005: IF the output path already holds a file AND `overwrite` is false THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' already exists; pass overwrite=true to replace it`, and write nothing. IF the output path holds a folder THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` and the message `'<path>' is a folder and cannot be replaced`, with or without `overwrite`, and write nothing.
- SPEC-0081/FR-NEW-013: IF the destination is an existing file, or has a file among its ancestors, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'<path>' is a file, not a folder`, naming the file found, and write nothing.
- SPEC-0081/FR-NEW-009: IF a password is given, empty or not, for a `.gz` file THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`, before reading the file.
- SPEC-0081/FR-NEW-010: THE archive extraction SHALL extract a file ending in `.tar.gz`, `.tgz` or the same suffixes in any letter case as a tar archive, exactly as before this increment.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0081_006 e2e_0081_011 e2e_0081_024 e2e_0081_028 e2e_0081_029 e2e_0081_034 e2e_0081_039 e2e_0081_040 e2e_0081_044 e2e_0081_054 e2e_0081_055 e2e_0081_056`: all 12 tests pass. After a refused call the store holds no new file and no new folder, read back with `client.exists`.

## Design excerpt
### Decisions
- **DEC-007:** order inside `extract_single`: password (already in the caller), output name, destination normalize, destination is a file, collision, caps, capped decode, admission, write. No byte is written and no folder is created before the decode succeeds. **Obliged by:** SPEC-0081/FR-NEW-011, FR-NEW-005, FR-NEW-013. **Rationale:** it is the spec's list, items 4 to 9. **Alternatives considered:** none, the spec fixes the order. **Source:** spec. **Evidence:** spec Section 6, FR-NEW-011.
- **DEC-008:** the destination check walks the destination and each ancestor from the root with `client.meta.get`; the first node that exists and is not a folder yields `'<that path>' is a file, not a folder`. The collision check reads the output path node: a folder gives the folder message with or without `overwrite`; a file gives the exists message unless `overwrite`. **Obliged by:** SPEC-0081/FR-NEW-005, FR-NEW-013. **Rationale:** `client.exists` cannot tell a file from a folder. **Alternatives considered:** `makedirs` and map its error. **Source:** design. **Evidence:** `archive.rs:130-141` uses `meta.get` and `is_dir`.
- **DEC-010:** the password check in `extract_archive` is widened from `Tar(_)` to `Tar(_) | Single(_)`; message unchanged. **Obliged by:** SPEC-0081/FR-NEW-009. **Rationale:** the spec wants the same refusal before the file is read. **Alternatives considered:** a second check in `extract_single` (too late, the file was already read). **Source:** existing code. **Evidence:** `archive.rs:146-150`.
- **DEC-012:** the stories are cut by capability so each one turns its own tests red to green: US-0160 builds the flow with `GzDecoder`, an unbounded read and a strict rule (anything at the output path is refused), US-0161 switches to `MultiGzDecoder`, US-0162 adds the password, destination and collision rules before the decode, US-0163 adds the caps, US-0164 pins the order of refusals with mutation evidence, writes the documentation and closes. Tests that characterize behavior already built (membership, missing path, folder, tar suffixes) are recorded green on arrival with that reason. **Obliged by:** n/a (no code impact): story planning. **Rationale:** vertical stories with real red evidence. **Alternatives considered:** one story of 57 tests. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
`extract_archive`: the password check `matches!(format, ArchiveFormat::Tar(_))` (`archive.rs:146`) becomes `Tar(_) | Single(_)`. `extract_single` order: output name, destination normalize (`path escapes the volume root: <destination>`), `check_destination` (ancestors then destination, first non folder node gives `'<path>' is a file, not a folder`), `check_collision` (folder: `ERR_NO_CLOBBER` `'<path>' is a folder and cannot be replaced` always; file: `ERR_NO_CLOBBER` `'<path>' already exists; pass overwrite=true to replace it` unless `overwrite`), then decode. The temporary rule of US-0160 (refuse anything that exists) is replaced.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
`client.meta.get(path)` returning `Option<Node>` with `is_dir()` (`archive.rs:130-141`); the existing no clobber message (`archive.rs:172-181`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0081/E2E-006: Collision
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/data.csv.gz` and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' already exists; pass overwrite=true to replace it`
- **And** `/uploads/data.csv` still reads `old`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_006_collision`
- Green on arrival: US-0160 already refuses an existing output path with the same message
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-011: Password given
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/data.csv.gz`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`; `/uploads/data.csv` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_011_password_given`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-024: Overwrite replaces
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` (16 bytes decoded), an existing `/uploads/data.csv` of 40 bytes, and a project storage limit with 5 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds, `/uploads/data.csv` holds the 16 bytes and the remaining storage is 29 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_024_overwrite_replaces`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-028: `.tar.gz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/uploads/pack.tar.gz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack`, 1 file written, 1 folder created; `/uploads/pack/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_028_tar_gz_is_still_a_tar`
- Green on arrival: the table order of US-0160 keeps `.tar.gz` a tar
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-029: `.tgz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.tgz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack` and `/uploads/pack/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_029_tgz_is_still_a_tar`
- Green on arrival: same reason
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-034: A folder bears the output name
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Low
- **Given** `/uploads/data.csv.gz` and a folder `/uploads/data.csv`
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/data.csv' is a folder and cannot be replaced`; the folder is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_034_a_folder_bears_the_output_name`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-039: Empty password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.gz`
- **When** the member extracts it with an empty password
- **Then** the refusal is as E2E-011
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_039_empty_password`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-040: Password through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.gz`
- **When** a client asks the HTTP API to extract it with the password `s3cret`
- **Then** the error code and message are those of E2E-011
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0081_040_password_through_the_http_api`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-044: `.TAR.GZ` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/PACK.TAR.GZ` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/PACK` and `/uploads/PACK/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_044_tar_gz_is_still_a_tar`
- Green on arrival: same reason
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-054: Destination is a file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013, FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`; `/uploads/notes.txt` is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_054_destination_is_a_file`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-055: A file among the ancestors of the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` and a file `/uploads/notes.txt`
- **When** the member extracts it to the destination `/uploads/notes.txt/sub`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/notes.txt' is a file, not a folder`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_055_a_file_among_the_ancestors_of_the_de`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-056: Destination is a file, through the HTTP API
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** the situation of E2E-054
- **When** a client asks the HTTP API to extract it to the destination `/uploads/notes.txt`
- **Then** the error code and message are those of E2E-054
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0081_056_destination_is_a_file_through_the_ht`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-006 | drop the existence check: the test fails |
| E2E-028 | move the `.gz` entry before `.tar.gz` in `SUFFIXES`: the test fails |
| E2E-029 | map `.tgz` to a Single format in the table: the test fails |
| E2E-044 | make `detect_format` case sensitive: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Both checks run before the decode; no folder is created before the decode succeeds
- Scope boundary: caps, final order pins, docs belong to US-0163 and US-0164

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-006, E2E-028, E2E-029, E2E-044): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0162: gzip password, collision and destination rules`, body `Spec: SPEC-0081` and the requirement ids.
