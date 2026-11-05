# US-0165: extract a bare bzip2 file next to the archive

> Spec: SPEC-0083 (sdlc/specs/SPEC-0083-extract-bz2-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0160
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A member can extract `/uploads/data.csv.bz2` and get `/uploads/data.csv` next to it, with the destination option, the output name rules and the same answer as for gzip.

## Requirements covered
- SPEC-0083/FR-NEW-001: WHEN a member asks to extract a file whose name ends in `.bz2`, in any letter case, and does not end in a tar suffix, THE archive extraction SHALL write one file named like the archive without its `.bz2` suffix, in the destination folder, and answer the destination, 1 file written, the folders created and the bytes written, and record one audit line.
- SPEC-0083/FR-NEW-002: WHEN a member gives a destination THE archive extraction SHALL write the output file in that folder, create the folder when it does not exist, and count it in the folders created.
- SPEC-0083/FR-NEW-003: IF the archive name without its `.bz2` suffix is empty, `.` or `..` THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `cannot derive an output file name from '<name>'`, and write nothing.
- SPEC-0083/FR-MOD-001: THE archive extraction SHALL treat a bare `.bz2` as a supported extension.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0083_001 e2e_0083_002 e2e_0083_003 e2e_0083_023 e2e_0083_024 e2e_0083_025 e2e_0083_038 e2e_0083_039 e2e_0083_043 e2e_0083_044 e2e_0083_045 e2e_0083_046 e2e_0083_047 e2e_0083_051 e2e_0083_052`: all 15 tests pass. The stored `/uploads/data.csv` equals the 16 byte fixture, read back from the store.

## Design excerpt
### Decisions
- **DEC-001:** bzip2 reuses the single file flow and rules of SPEC-0081 (`extract_single`, `read_capped`, caps, admission, order); its stories depend on the SPEC-0081 stories that build each rule. **Obliged by:** SPEC-0083/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine for gzip, bzip2 and xz. **Alternatives considered:** a parallel flow. **Source:** SPEC-0081 DEC-001, DEC-005, DEC-007 (existing design binds). **Evidence:** `sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-design.md`.
- **DEC-003:** the table entry `(".bz2", Single(Bzip2))` follows the tar entries and `.gz`. **Obliged by:** SPEC-0083/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** ordered `ends_with` matching, compound before plain. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `archive.rs:40-69`.
- **DEC-004:** US-0165 wires `.bz2` with `BzDecoder` (single stream) and the bytes read through the SPEC-0081 flow; US-0166 switches to `MultiBzDecoder`, which is what turns the multi stream and trailing bytes tests green; US-0167 and US-0168 add tests that characterize rules already built by SPEC-0081 (collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** the rules are shared code, so their bzip2 tests are green on arrival; proving they bite needs a mutation (break the rule, see the test fail). **Alternatives considered:** one story of 53 tests. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
Table entry `(".bz2", ArchiveFormat::Single(SingleFormat::Bzip2))` after the tar entries and `.gz`; `SingleFormat::Bzip2` with `label() == "bz2"`; `decoder(Bzip2, bytes)` returns `bzip2::read::BzDecoder` (single stream; US-0166 switches it). `detect_format_maps_every_suffix` gains rows for `.bz2` and `.BZ2` (suffix length 4).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
`SingleFormat`, `decoder`, `extract_single` from US-0160 (`archive_single.rs`); `SUFFIXES` (`archive.rs:40-50`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0083/E2E-001: bzip2 file decompresses next to the archive
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** a member and `/uploads/data.csv.bz2`, the bzip2 compression of the 16 byte text `id,name`, newline, `1,Alice`, newline; nothing at `/uploads/data.csv`
- **When** the member extracts `/uploads/data.csv.bz2`
- **Then** the answer is destination `/uploads`, 1 file written, 0 folders created, 16 bytes written
- **And** `/uploads/data.csv` reads the 16 bytes; `/uploads/data.csv.bz2` is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_001_bzip2_file_decompresses_next_to_the`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-002: Upper case suffix
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** the same content stored as `/uploads/DATA.CSV.BZ2`
- **When** the member extracts it
- **Then** `/uploads/DATA.CSV` exists with the 16 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_002_upper_case_suffix`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-003: Explicit existing destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.bz2` and the existing empty folder `/out`
- **When** the member extracts it to the destination `/out`
- **Then** the answer is destination `/out`, 1 file written, 0 folders created; `/out/data.csv` exists and `/uploads/data.csv` does not
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_003_explicit_existing_destination`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-023: File named `.bz2`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.bz2`, a valid bzip2 file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `cannot derive an output file name from '.bz2'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_023_file_named_bz2`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-024: File named `..bz2`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/..bz2`, a valid bzip2 file
- **When** the member extracts it
- **Then** the refusal is as E2E-023 with `'..bz2'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_024_file_named_bz2`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-025: Upper case `.BZ2` alone
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.BZ2`, a valid bzip2 file
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.BZ2'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_025_upper_case_bz2_alone`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-038: Another extension is still refused
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-MOD-001, FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.lz` holding `anything`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.lz'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_038_another_extension_is_still_refused`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-039: bzip2 of an empty file
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/empty.txt.bz2`, the bzip2 compression of a 0 byte file
- **When** the member extracts it
- **Then** the answer is 1 file written, 0 bytes written; `/uploads/empty.txt` exists with 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_039_bzip2_of_an_empty_file`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-043: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/rapport été.txt.bz2`
- **When** the member extracts it
- **Then** `/uploads/rapport été.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_043_accents_and_spaces`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-044: Several dots and no dot
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/a.b.c.bz2` and `/uploads/README.bz2`
- **When** the member extracts each
- **Then** `/uploads/a.b.c` and `/uploads/README` exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_044_several_dots_and_no_dot`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-045: Archive directly under the project root
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/a.bz2`, the bzip2 compression of `hello`
- **When** the member extracts it
- **Then** the answer is destination `/`, 1 file written, 0 folders created, 5 bytes written; `/a` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_045_archive_directly_under_the_project_r`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-046: Missing destination is created
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2` and no folder `/new/dir`
- **When** the member extracts it to the destination `/new/dir`
- **Then** the answer is destination `/new/dir`, 1 file written, 1 folder created; `/new/dir/data.csv` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_046_missing_destination_is_created`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-047: Trailing slash on the destination
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2` and the existing folder `/out`
- **When** the member extracts it to the destination `/out/`
- **Then** the answer is destination `/out` and `/out/data.csv` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_047_trailing_slash_on_the_destination`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-051: Success is audited
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** the member extracts it
- **Then** the audit trail has exactly one extraction line for `/uploads/data.csv.bz2` with the destination, 1 file written and 16 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_051_success_is_audited`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-052: The HTTP API gives the same answer
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/data.csv.bz2`
- **Then** the answer is that of E2E-001, and a second call with `overwrite` false carries the error of E2E-009
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0083_052_the_http_api_gives_the_same_answer`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none (`bzip2 = "0.6"` is already a dependency)
- Scope boundary: several streams, trailing bytes, password, tar rules, order, limits belong to US-0166 to US-0168

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0165: extract a bare bzip2 file next to the archive`, body `Spec: SPEC-0083` and the requirement ids.
