# US-0161: decode several gzip members and refuse corrupt input

> Spec: SPEC-0081 (sdlc/specs/SPEC-0081-extract-gz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0160
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
A gzip file made of several members is decoded whole, and any byte after a valid stream (junk or zero bytes) is the corrupt refusal. Header names, comments and extra fields never influence the output.

## Requirements covered
- SPEC-0081/FR-NEW-004: THE archive extraction SHALL NOT use any name, comment or extra field stored inside the gzip file to name or place the output.
- SPEC-0081/FR-NEW-007: WHEN a gzip file holds several members back to back THE archive extraction SHALL decode them all and write their concatenation as the one output file.
- SPEC-0081/FR-NEW-008: IF the file is empty, is truncated, is not gzip, or has bytes that are not gzip after a valid stream THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid gz file:`, and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0081_005 e2e_0081_008 e2e_0081_009 e2e_0081_010 e2e_0081_018 e2e_0081_033 e2e_0081_037 e2e_0081_038 e2e_0081_042 e2e_0081_043 e2e_0081_046`: all 11 tests pass. The stored file after the call is the concatenation of the members (`hello world`), read back from the store.

## Design excerpt
### Decisions
- **DEC-004:** gzip is read with `flate2::read::MultiGzDecoder` over a `Cursor` of the file bytes inside `spawn_blocking`; any read error maps to `corrupt("gz", e)` (`archive.rs:350-354`); header name, comment and extra fields are never read by our code. **Obliged by:** SPEC-0081/FR-NEW-004, FR-NEW-007, FR-NEW-008. **Rationale:** spike on flate2 1.x: two members decode to `hello world`; `JUNK` after a member and 4 zero bytes both fail with `unexpected end of file`; an empty member gives an empty output; a 0 byte file and a text file fail; a truncated file fails with `incomplete deflate stream`. **Alternatives considered:** `GzDecoder` (stops after the first member, ignores trailing bytes). **Source:** spike run 2026-11-05 on the crates in `Cargo.lock`. **Evidence:** `Cargo.toml:82`.
- **DEC-012:** the stories are cut by capability so each one turns its own tests red to green: US-0160 builds the flow with `GzDecoder`, an unbounded read and a strict rule (anything at the output path is refused), US-0161 switches to `MultiGzDecoder`, US-0162 adds the password, destination and collision rules before the decode, US-0163 adds the caps, US-0164 pins the order of refusals with mutation evidence, writes the documentation and closes. Tests that characterize behavior already built (membership, missing path, folder, tar suffixes) are recorded green on arrival with that reason. **Obliged by:** n/a (no code impact): story planning. **Rationale:** vertical stories with real red evidence. **Alternatives considered:** one story of 57 tests. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
`decoder(Gzip, bytes)` returns `flate2::read::MultiGzDecoder` instead of `GzDecoder`. No new signature.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
### Existing patterns
`decoder` and `corrupt("gz", e)` from US-0160. Spike results (flate2 1.x): two members give `hello world`; `JUNK` or 4 zero bytes after a member fail with `unexpected end of file`; an empty member gives an empty output; a truncated stream fails with `incomplete deflate stream`. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0081/E2E-005: Two gzip members back to back
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/two.txt.gz` made of two gzip members, the first holding `hello ` and the second `world`
- **When** the member extracts it
- **Then** `/uploads/two.txt` reads `hello world` and the answer reports 11 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_005_two_gzip_members_back_to_back`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-008: Truncated file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/cut.gz`, the first half of a valid gzip file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid gz file:`; `/uploads/cut` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_008_truncated_file`
- Green on arrival: the corrupt mapping exists since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-009: Text file named `.gz`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/fake.gz` containing the text `not gzip`
- **When** the member extracts it
- **Then** the refusal is as E2E-008; `/uploads/fake` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_009_text_file_named_gz`
- Green on arrival: the corrupt mapping exists since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-010: Bytes after a valid stream
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/tail.gz`, a valid gzip file followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-008; `/uploads/tail` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_010_bytes_after_a_valid_stream`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-018: Name stored inside the file is ignored
- **Category:** edge, security; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** Critical
- **Given** `/uploads/data.csv.gz` whose gzip header stores the file name `../../etc/passwd`
- **When** the member extracts it
- **Then** `/uploads/data.csv` exists with the content
- **And** no `/etc/passwd` and no `/uploads/etc` exist in the project
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_018_name_stored_inside_the_file_is_ignor`
- Green on arrival: the header fields are never read since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-033: A refused extraction creates nothing
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/cut.gz` truncated and no folder `/fresh`
- **When** the member extracts it to the destination `/fresh`
- **Then** the refusal is as E2E-008 and `/fresh` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_033_a_refused_extraction_creates_nothing`
- Green on arrival: the destination is created only after the decode since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-037: Three gzip members
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/three.txt.gz` made of the members `a`, `b`, `c`
- **When** the member extracts it
- **Then** `/uploads/three.txt` reads `abc` and 3 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_037_three_gzip_members`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-038: An empty gzip member between two others
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/gap.txt.gz` made of the members `a`, an empty member, `b`
- **When** the member extracts it
- **Then** `/uploads/gap.txt` reads `ab`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_038_an_empty_gzip_member_between_two_oth`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-042: A different stored name
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` whose gzip header stores the file name `other.txt`
- **When** the member extracts it
- **Then** `/uploads/data.csv` exists and `/uploads/other.txt` does not
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_042_a_different_stored_name`
- Green on arrival: the header fields are never read since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-043: Comment and extra fields in the header
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/data.csv.gz` whose header holds a comment `from alice` and an extra field
- **When** the member extracts it
- **Then** `/uploads/data.csv` holds the content and nothing else is created in `/uploads`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_043_comment_and_extra_fields_in_the_head`
- Green on arrival: the header fields are never read since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-046: Empty file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** a 0 byte file `/uploads/zero.gz`
- **When** the member extracts it
- **Then** the refusal is as E2E-008; `/uploads/zero` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_046_empty_file`
- Green on arrival: an empty file is the corrupt refusal since US-0160
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-008 | map a read error to `ERR_INTERNAL_ERROR`: the test fails |
| E2E-009 | same mutation |
| E2E-018 | make the decoder return the stored name as the output name: the test fails |
| E2E-033 | create the destination before the decode: the test fails |
| E2E-042 | same mutation: the test fails |
| E2E-043 | make the decoder fail on a header comment: the test fails |
| E2E-046 | same as E2E-008 |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `archive.rs`
- Dependencies not to add: none
- Scope boundary: password, collisions, caps, order belong to later stories

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-008, E2E-009, E2E-018, E2E-033, E2E-042, E2E-043, E2E-046): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0161: decode several gzip members and refuse corrupt input`, body `Spec: SPEC-0081` and the requirement ids.
