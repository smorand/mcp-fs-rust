# US-0160: extract a bare gzip file next to the archive

> Spec: SPEC-0081 (sdlc/specs/SPEC-0081-extract-gz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A member can extract `/uploads/data.csv.gz` and get `/uploads/data.csv` next to it, from the tool and from the HTTP API. The answer gives the destination, 1 file written, the folders created and the bytes written, and one audit line is recorded. This story builds the single file flow with the simplest decoder; later stories harden it.

## Requirements covered
- SPEC-0081/FR-NEW-001: WHEN a member asks to extract a file whose name ends in `.gz`, in any letter case, and does not end in a tar suffix, THE archive extraction SHALL write one file named like the archive without its `.gz` suffix, in the destination folder, and answer the destination, 1 file written, the folders created and the bytes written, and record one audit line.
- SPEC-0081/FR-NEW-002: WHEN a member gives a destination THE archive extraction SHALL write the output file in that folder, create the folder when it does not exist, and count it in the folders created.
- SPEC-0081/FR-NEW-003: IF the archive name without its `.gz` suffix is empty, `.` or `..` THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `cannot derive an output file name from '<name>'`, and write nothing.
- SPEC-0081/FR-MOD-001: THE archive extraction SHALL treat a bare `.gz` as a supported extension.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0081_001 e2e_0081_002 e2e_0081_003 e2e_0081_004 e2e_0081_012 e2e_0081_013 e2e_0081_023 e2e_0081_026 e2e_0081_027 e2e_0081_030 e2e_0081_031 e2e_0081_036 e2e_0081_041 e2e_0081_mod_001`: all 14 tests pass. Reading `/uploads/data.csv` back from the store gives the 16 bytes of the fixture, which is a check on the stored content and not on the story's own return value.

## Design excerpt
### Decisions
- **DEC-001:** a bare gzip file is handled by a new module `tools/archive_single.rs` reached from `extract_archive` after the shared checks; no new tool, no new route. **Obliged by:** SPEC-0081/FR-NEW-001, FR-NEW-010, FR-NEW-011. **Rationale:** the order of refusals and the bound on decoding cannot be expressed in the zip, 7z, tar pipeline without changing it for every format. **Alternatives considered:** one entry archive inside `decode_archive`; sibling tool. **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:119-215` (decode precedes collision and admission).
- **DEC-002:** format by suffix table entry `(".gz", ArchiveFormat::Single(SingleFormat::Gzip))` placed after all tar entries and `.7z`; `detect_format` signature unchanged. **Obliged by:** SPEC-0081/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** matching is by ordered `ends_with` (`archive.rs:56-69`), compound before plain. **Alternatives considered:** a second table; content sniffing. **Source:** existing code `archive.rs:40-69`. **Evidence:** `archive.rs:40-50`.
- **DEC-003:** output name is the archive file name minus `suffix_len` bytes; default destination is the parent folder of the archive (`/` for `/a.gz`); the destination option goes through `state.safety.normalize_path` (which strips a trailing slash and yields `path escapes the volume root: <destination>`); the destination is created only after a successful decode and counted once in `dirs_created`. **Obliged by:** SPEC-0081/FR-NEW-001, FR-NEW-002, FR-NEW-003. **Rationale:** the messages in the spec match `safety.rs:99-111` verbatim. **Alternatives considered:** the tar rule (a folder named like the archive). **Source:** spec. **Evidence:** `crates/core/src/safety.rs:99-111`, `archive.rs:166-169`.
- **DEC-006:** `SafetyManager::session_remaining(person, project) -> i64` is added (read only, `with_session`); `quota.rs` gains `project_remaining_for` reading `safety.project_limit` and `client.meta.used_bytes()`. **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** both numbers are private to those modules today. **Alternatives considered:** make `with_session` public. **Source:** existing code. **Evidence:** `safety.rs:82-92`, `quota.rs:96-113`.
- **DEC-009:** after a successful capped decode the write goes through `quota::admit_paths(client, safety, person, mount_id, len, &[(out, len)])` and the returned `Admission` lives until the write lands, as the archive flow does (`archive.rs:183-205`). **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** project check on net growth first, session charged gross only after it passed, one admission lock. **Alternatives considered:** a second admission code path. **Source:** existing code. **Evidence:** `quota.rs:136-158`.
- **DEC-011:** the answer, the audit line and the tracing event are produced by one helper `finish_extraction(state, person, mount_id, normalized, dest, files, bytes, dirs)` extracted from the tail of `extract_archive` (`archive.rs:242-275`) and used by both flows; no behavior change for existing formats. **Obliged by:** SPEC-0081/FR-NEW-001. **Rationale:** one implementation of the answer shape. **Alternatives considered:** copy the 30 lines. **Source:** AGENTS.md convention (one engine, no copies). **Evidence:** `archive.rs:242-275`.
- **DEC-012:** the stories are cut by capability so each one turns its own tests red to green: US-0160 builds the flow with `GzDecoder`, an unbounded read and a strict rule (anything at the output path is refused), US-0161 switches to `MultiGzDecoder`, US-0162 adds the password, destination and collision rules before the decode, US-0163 adds the caps, US-0164 pins the order of refusals with mutation evidence, writes the documentation and closes. Tests that characterize behavior already built (membership, missing path, folder, tar suffixes) are recorded green on arrival with that reason. **Obliged by:** n/a (no code impact): story planning. **Rationale:** vertical stories with real red evidence. **Alternatives considered:** one story of 57 tests. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
`ArchiveFormat::Single(SingleFormat)`; table entry `(".gz", Single(Gzip))` after every tar entry and `.7z`; `extract_single(...)` as in the design (section 5); `output_name(file_name, suffix_len)`; `read_capped(reader, cap)` exists here with `cap = u64::MAX` (no bound yet) and returns `CappedRead { bytes, high_water }`; `decoder(format, bytes) -> Box<dyn Read + '_>` returns `flate2::read::GzDecoder`; `finish_extraction(...)` extracted from the tail of `extract_archive` (`archive.rs:242-275`) and used by both flows. Answer keys `destination`, `files_written`, `dirs_created`, `bytes_written`. Read errors map to `corrupt("gz", e)` (`archive.rs:350-354`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | create |
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
`detect_format` and `SUFFIXES` (`archive.rs:40-69`); the tail of `extract_archive` (`archive.rs:214-275`: `ensure_parents`, `write_bytes_atomic`, `touch_atime_mtime`, `record_audit`, `tracing::info!`, answer); `quota::admit_paths(client, &state.safety, person, mount_id, session_bytes, &[(path, size)])` (`quota.rs:136-158`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0081/E2E-001: Gzip file decompresses next to the archive
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** a member and `/uploads/data.csv.gz`, the gzip of the 16 byte text `id,name`, newline, `1,Alice`, newline; nothing at `/uploads/data.csv`
- **When** the member extracts `/uploads/data.csv.gz`
- **Then** the answer is destination `/uploads`, 1 file written, 0 folders created, 16 bytes written
- **And** `/uploads/data.csv` reads the 16 bytes; `/uploads/data.csv.gz` is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_001_gzip_file_decompresses_next_to_the_a`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-002: Upper case suffix
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** the same content stored as `/uploads/DATA.CSV.GZ`
- **When** the member extracts it
- **Then** `/uploads/DATA.CSV` exists with the 16 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_002_upper_case_suffix`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-003: Explicit existing destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.gz` and the existing empty folder `/out`
- **When** the member extracts it to the destination `/out`
- **Then** the answer is destination `/out`, 1 file written, 0 folders created; `/out/data.csv` exists and `/uploads/data.csv` does not
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_003_explicit_existing_destination`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-004: Missing destination is created
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.gz` and no folder `/new/dir`
- **When** the member extracts it to the destination `/new/dir`
- **Then** the answer is destination `/new/dir`, 1 file written, 1 folder created; `/new/dir/data.csv` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_004_missing_destination_is_created`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-012: File named `.gz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.gz`, a valid gzip file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `cannot derive an output file name from '.gz'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_012_file_named_gz`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-013: File named `..gz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/..gz`, a valid gzip file
- **When** the member extracts it
- **Then** the refusal is as E2E-012 with `'..gz'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_013_file_named_gz`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-023: Gzip of an empty file
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/empty.txt.gz`, the gzip of a 0 byte file
- **When** the member extracts it
- **Then** the answer is 1 file written, 0 bytes written; `/uploads/empty.txt` exists with 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_023_gzip_of_an_empty_file`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-026: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/rapport été.txt.gz`
- **When** the member extracts it
- **Then** `/uploads/rapport été.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_026_accents_and_spaces`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-027: Several dots and no dot
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/a.b.c.gz` and `/uploads/README.gz`
- **When** the member extracts each
- **Then** `/uploads/a.b.c` and `/uploads/README` exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_027_several_dots_and_no_dot`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-030: Success is audited
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** the member extracts it
- **Then** the audit trail has exactly one extraction line for `/uploads/data.csv.gz` with the destination, 1 file written and 16 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_030_success_is_audited`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-031: The HTTP API gives the same answer
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/data.csv.gz`
- **Then** the answer is that of E2E-001, and a second call with `overwrite` false carries the error of E2E-006
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0081_031_the_http_api_gives_the_same_answer`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-036: Trailing slash on the destination
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` and the existing folder `/out`
- **When** the member extracts it to the destination `/out/`
- **Then** the answer is destination `/out` and `/out/data.csv` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_036_trailing_slash_on_the_destination`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-041: Upper case `.GZ` alone
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.GZ`, a valid gzip file
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.GZ'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_041_upper_case_gz_alone`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-MOD-001: bare `.gz` (was SPEC-0018 E2E-NEW-034)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-008; **Priority:** High
- **Validated before:** the file `/uploads/data.gz` holding the 8 bytes `anything` is refused with `ERR_NOT_SUPPORTED` naming `.gz`.
- **Validates now:** the same file is refused with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid gz file:`, because it is not gzip.
- **Given** `/uploads/data.gz` holding `anything`; **When** the member extracts it; **Then** the corrupt refusal above.
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive.rs tests (existing test updated)
- Function: `e2e_0081_mod_001_bare_gz_was_spec_0018_e2e_new_034`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Use `GzDecoder` and the unbounded `read_capped` here; `MultiGzDecoder` is US-0161, the caps are US-0163 (DEC-012)
- The output path is refused with the `already exists` message when anything exists there, with or without `overwrite`; the overwrite and folder wording belong to US-0162
- Scope boundary: password, destination is a file, collision wording, caps, order of refusals, docs belong to US-0161 to US-0164

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0160: extract a bare gzip file next to the archive`, body `Spec: SPEC-0081` and the requirement ids.
