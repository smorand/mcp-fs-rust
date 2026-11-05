# US-0166: decode several bzip2 streams and refuse corrupt input

> Spec: SPEC-0083 (sdlc/specs/SPEC-0083-extract-bz2-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0165, US-0162
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A bzip2 file made of several streams is decoded whole; bytes after a valid stream, junk or zero bytes, are the corrupt refusal; a password is refused; the tar suffixes stay tar.

## Requirements covered
- SPEC-0083/FR-NEW-007: WHEN a bzip2 file holds several bzip2 streams back to back THE archive extraction SHALL decode them all and write their concatenation as the one output file.
- SPEC-0083/FR-NEW-008: IF the file is empty, is truncated, is not bzip2, has a damaged checksum, or has bytes after a valid stream that do not form another bzip2 stream, trailing zero bytes included, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid bz2 file:`, and write nothing.
- SPEC-0083/FR-NEW-009: IF a password is given, empty or not, for a `.bz2` file THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`, before reading the file.
- SPEC-0083/FR-NEW-010: THE archive extraction SHALL extract a file ending in `.tar.bz2` or `.tb2`, in any letter case, as a tar archive exactly as before this increment, and SHALL keep refusing every other extension that is not supported, such as `.lz`, with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '<extension as typed>'`, and SHALL leave the collision, destination and order of refusals rules of zip, 7z and tar archives unchanged.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0083_004 e2e_0083_005 e2e_0083_006 e2e_0083_007 e2e_0083_008 e2e_0083_014 e2e_0083_015 e2e_0083_016 e2e_0083_017 e2e_0083_018 e2e_0083_019 e2e_0083_020 e2e_0083_021 e2e_0083_022 e2e_0083_048`: all 15 tests pass. The stored output after a multi stream call is the concatenation of the streams, read back from the store.

## Design excerpt
### Decisions
- **DEC-002:** the reader is `bzip2::read::MultiBzDecoder` over a `Cursor` inside `spawn_blocking`, read errors mapped to `corrupt("bz2", e)`. **Obliged by:** SPEC-0083/FR-NEW-007, FR-NEW-008. **Rationale:** spike on bzip2 0.6.1: two streams give `hello world`; `JUNK` and 4 zero bytes after a stream fail with `bzip2: bz2 header missing`; a 0 byte file and a truncated file fail with `decompression not finished but EOF reached`; a flipped byte fails with `bzip2: invalid data`; an empty stream gives an empty output (`bufread.rs:181-218`). **Alternatives considered:** `BzDecoder` (single stream, ignores trailing bytes, which would accept the bytes FR-NEW-008 refuses). **Source:** spike run 2026-11-05. **Evidence:** `bzip2-0.6.1/src/bufread.rs:181-218`, `:234-244`.
- **DEC-004:** US-0165 wires `.bz2` with `BzDecoder` (single stream) and the bytes read through the SPEC-0081 flow; US-0166 switches to `MultiBzDecoder`, which is what turns the multi stream and trailing bytes tests green; US-0167 and US-0168 add tests that characterize rules already built by SPEC-0081 (collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** the rules are shared code, so their bzip2 tests are green on arrival; proving they bite needs a mutation (break the rule, see the test fail). **Alternatives considered:** one story of 53 tests. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
`decoder(Bzip2, bytes)` returns `bzip2::read::MultiBzDecoder`. Read errors keep mapping to `corrupt("bz2", e)`: `archive is corrupt or not a valid bz2 file: <detail>`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
Spike results (bzip2 0.6.1): two streams give `hello world`; `JUNK` or 4 zero bytes after a stream fail with `bzip2: bz2 header missing`; a 0 byte file or a truncated file fails with `decompression not finished but EOF reached`; a flipped byte fails with `bzip2: invalid data`; an empty stream gives an empty output. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0083/E2E-004: Two bzip2 streams back to back
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/two.txt.bz2` made of two bzip2 streams, the first holding `hello ` and the second `world`
- **When** the member extracts it
- **Then** `/uploads/two.txt` reads `hello world` and the answer reports 11 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_004_two_bzip2_streams_back_to_back`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-005: Three bzip2 streams
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/three.txt.bz2` made of the streams `a`, `b`, `c`
- **When** the member extracts it
- **Then** `/uploads/three.txt` reads `abc` and 3 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_005_three_bzip2_streams`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-006: `.tar.bz2` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/uploads/pack.tar.bz2` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack`, 1 file written, 1 folder created; `/uploads/pack/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_006_tar_bz2_is_still_a_tar`
- Green on arrival: the table order keeps `.tar.bz2` a tar
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-007: `.tb2` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.tb2` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack` and `/uploads/pack/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_007_tb2_is_still_a_tar`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-008: `.TAR.BZ2` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/PACK.TAR.BZ2` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/PACK` and `/uploads/PACK/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_008_tar_bz2_is_still_a_tar`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-014: Truncated file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/cut.bz2`, the first half of a valid bzip2 file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid bz2 file:`; `/uploads/cut` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_014_truncated_file`
- Green on arrival: the corrupt mapping is shared since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-015: Text file named `.bz2`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/fake.bz2` containing the text `not bzip2`
- **When** the member extracts it
- **Then** the refusal is as E2E-014; `/uploads/fake` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_015_text_file_named_bz2`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-016: Bytes after a valid stream
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/tail.bz2`, a valid bzip2 file followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-014; `/uploads/tail` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_016_bytes_after_a_valid_stream`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-017: Zero bytes after a valid stream
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/pad.bz2`, a valid bzip2 file followed by 4 zero bytes
- **When** the member extracts it
- **Then** the refusal is as E2E-014; `/uploads/pad` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_017_zero_bytes_after_a_valid_stream`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-018: Damaged checksum
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/bad.bz2`, the content of `/uploads/data.csv.bz2` with one byte of its compressed data changed
- **When** the member extracts it
- **Then** the refusal is as E2E-014; `/uploads/bad` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_018_damaged_checksum`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-019: A 0 byte file named `zero.bz2`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** a 0 byte file `/uploads/zero.bz2`
- **When** the member extracts it
- **Then** the refusal is as E2E-014; `/uploads/zero` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_019_a_0_byte_file_named_zero_bz2`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-020: Password given
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/data.csv.bz2`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`; `/uploads/data.csv` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_020_password_given`
- Green on arrival: the password check covers Single formats since US-0162
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-021: Empty password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2`
- **When** the member extracts it with an empty password
- **Then** the refusal is as E2E-020
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_021_empty_password`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-022: Password through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2`
- **When** a client asks the HTTP API to extract it with the password `s3cret`
- **Then** the error code and message are those of E2E-020
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0083_022_password_through_the_http_api`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-048: An empty bzip2 stream between two others
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/hole.txt.bz2` made of the streams `a`, an empty stream, `b`
- **When** the member extracts it
- **Then** `/uploads/hole.txt` reads `ab`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_048_an_empty_bzip2_stream_between_two_ot`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-006 | put `.bz2` before `.tar.bz2`: the test fails |
| E2E-007 | map `.tb2` to Single: the test fails |
| E2E-008 | make `detect_format` case sensitive: the test fails |
| E2E-014 | map a read error to `ERR_INTERNAL_ERROR`: the test fails |
| E2E-015 | same |
| E2E-018 | same |
| E2E-019 | same |
| E2E-020 | drop `Single(_)` from the password check: the test fails |
| E2E-021 | same |
| E2E-022 | same |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `archive.rs`
- Dependencies not to add: none
- The tar path keeps `BzDecoder`; do not touch `decode_tar`
- Scope boundary: collisions, destination, order, limits belong to US-0167 and US-0168

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-006, E2E-007, E2E-008, E2E-014, E2E-015, E2E-018, E2E-019, E2E-020, E2E-021, E2E-022): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0166: decode several bzip2 streams and refuse corrupt input`, body `Spec: SPEC-0083` and the requirement ids.
