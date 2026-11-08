# US-0180: xz password, output name, tar rows and the HTTP answer

> Spec: SPEC-0082 (sdlc/specs/SPEC-0082-extract-xz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0179
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The rules shared with gzip and bzip2 are pinned for xz by their own tests: a password is refused before the file is read, a name with no stem is refused, `.tar.xz`, `.txz` and `.TAR.XZ` stay tar archives, `.lz` stays unsupported, and the HTTP door answers what the engine answers.

## Requirements covered
- SPEC-0082/FR-NEW-001 (partial: the HTTP door, E2E-037): WHEN a member asks to extract a file whose name ends in `.xz`, in any letter case, and does not end in a tar suffix, THE archive extraction SHALL write one file named like the archive without its `.xz` suffix, in the destination folder, and answer the destination, 1 file written, the folders created and the bytes written, and record one audit line.
- SPEC-0082/FR-NEW-003: IF the archive name without its `.xz` suffix is empty, `.` or `..` THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `cannot derive an output file name from '<name>'`, and write nothing.
- SPEC-0082/FR-NEW-009: IF a password is given, empty or not, for an `.xz` file THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `password is not applicable to this archive format`, before reading the file.
- SPEC-0082/FR-NEW-010: THE archive extraction SHALL extract a file ending in `.tar.xz` or `.txz`, in any letter case, as a tar archive exactly as before this increment, and SHALL keep refusing every other extension that is not supported, such as `.lz`, with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '<extension as typed>'`; the collision, destination and order of refusals rules of zip, 7z and tar archives are unchanged by this increment.
- SPEC-0082/FR-MOD-001 (partial: E2E-048, other extensions still refused): THE archive extraction SHALL treat a bare `.xz` as a supported extension.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0082_016 e2e_0082_045 e2e_0082_046 e2e_0082_017 e2e_0082_041 e2e_0082_042 e2e_0082_005 e2e_0082_006 e2e_0082_047 e2e_0082_048 e2e_0082_037`: all 11 tests pass. After the tar tests the store holds `/uploads/pack/a.txt` (5 bytes), read back from the store.

## Design excerpt
### Decisions
- **DEC-001:** xz reuses the single file flow and rules of SPEC-0081 and SPEC-0083 (`extract_single`, `read_capped`, `caps`, `check_destination`, `check_collision`, admission, order). **Obliged by:** SPEC-0082/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine per operation (AGENTS.md conventions); the spec's rules are those of the gzip increment word for word. **Alternatives considered:** a parallel xz flow. **Source:** existing code and SPEC-0083 DEC-001. **Evidence:** `crates/core/src/tools/archive_single.rs:212-277`, `sdlc/specs/SPEC-0083-extract-bz2-single-file/sdlc-design.md`.
- **DEC-005:** table row `(".xz", ArchiveFormat::Single(SingleFormat::Xz))` after `(".bz2", ...)`; `label()` is `"xz"`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** `detect_format` takes the first `ends_with` match in table order (`archive.rs:68-79`), so `.tar.xz` and `.txz` (rows `:49-50`) win over the plain `.xz`. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `archive.rs:44-62`.
- **DEC-008:** US-0178 wires `.xz` with `Stream::new_stream_decoder(u64::MAX, TELL_UNSUPPORTED_CHECK)` (one stream); US-0179 adds `CONCATENATED`, which turns the several streams, junk and odd padding tests from red to green; US-0180 to US-0182 add tests that characterize rules already built by SPEC-0081 (password, name, tar rows, collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** measured: without `CONCATENATED` two streams give `hello ` and `JUNK` or 3 zero bytes after a stream are ignored, so US-0179's red evidence is real; the shared rules are green on arrival for xz, so proving they bite needs a mutation. **Alternatives considered:** one story of 58 tests; the full flags in US-0178 (US-0179 would then be all green). **Source:** design. **Evidence:** spike.
- **DEC-009:** the tar family keeps `lzma-rs`; its decode path (`archive.rs:742-746`) is not touched. **Obliged by:** SPEC-0082/FR-NEW-010, spec section 3.2. **Rationale:** the spec pins the tar behavior as unchanged; the move is BL-0132. **Alternatives considered:** moving tar xz to liblzma now. **Source:** spec constraint. **Evidence:** `sdlc/backlog/BL-0132-move-the-tar-xz-decoder-to-liblzma.md`.
### Data model
None.
### Contracts
No code contract: the password refusal is `archive.rs:169` (every `Single(_)` format), the name rule is `output_name` (`archive_single.rs:93-101`), the tar rows are `archive.rs:49-50`. Only tests are added, plus a code change if a test proves xz differs.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
### Existing patterns
Password and tar models: `password_refused`, `tar_bz2_is_a_tar` and `e2e_0083_020` to `e2e_0083_008` (`archive_single.rs:1361-1420`); tar xz fixture: the tar bytes of `tar_bz2_is_a_tar` (`archive_single.rs:1381-1392`) compressed with `lzma_rs::xz_compress` as `archive.rs:1984-1991`. Name model: `refuses_bz2_name` (`archive_single.rs:1135-1165`). Fixture helper added by US-0178, next to `bzip2()` at `archive_single.rs:1087`: `fn xz(content: &[u8]) -> Vec<u8>` with `liblzma::write::XzEncoder::new(Vec::new(), 6)`, `write_all`, `finish` (DEC-007). Test helpers: `fixture()`, `seed`, `extract`, `extract_with`, `read`, `exists` in the tests module of `crates/core/src/tools/archive_single.rs` (`:297-319`, `:613-629`); the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_0083_052_the_http_api_gives_the_same_answer` at `crates/core/src/api/dataplane.rs:3539`. Password through HTTP: `e2e_0083_022_password_through_the_http_api` at `dataplane.rs:3585`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0082/E2E-016: Password given
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/data.csv.xz`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`; `/uploads/data.csv` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_016_password_given`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-045: Empty password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.xz`
- **When** the member extracts it with an empty password
- **Then** the refusal is as E2E-016
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_045_empty_password`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-046: Password through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/data.csv.xz`
- **When** a client asks the HTTP API to extract it with the password `s3cret`
- **Then** the error code and message are those of E2E-016
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0082_046_password_through_the_http_api`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-017: File named `.xz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.xz`, a valid xz file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `cannot derive an output file name from '.xz'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_017_file_named_xz`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-041: File named `..xz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/..xz`, a valid xz file
- **When** the member extracts it
- **Then** the refusal is as E2E-017 with `'..xz'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_041_file_named_xz`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-042: Upper case `.XZ` alone
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/.XZ`, a valid xz file
- **When** the member extracts it
- **Then** the message is `cannot derive an output file name from '.XZ'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_042_upper_case_xz_alone`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-005: `.tar.xz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/uploads/pack.tar.xz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack`, 1 file written, 1 folder created; `/uploads/pack/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_005_tar_xz_is_still_a_tar`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-006: `.txz` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.txz` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/pack` and `/uploads/pack/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_006_txz_is_still_a_tar`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-047: `.TAR.XZ` is still a tar
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/PACK.TAR.XZ` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** the destination is `/uploads/PACK` and `/uploads/PACK/a.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_047_tar_xz_is_still_a_tar`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-048: Another extension is still refused
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-MOD-001, FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/pack.lz` holding `anything`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.lz'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_048_another_extension_is_still_refused`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-037: The HTTP API gives the same answer
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/data.csv.xz`
- **Then** the answer is that of E2E-001, and a second call with `overwrite` false carries the error of E2E-007
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0082_037_the_http_api_gives_the_same_answer`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-016 | remove `ArchiveFormat::Single(_)` from the password check at `archive.rs:169`: the test fails |
| E2E-045 | remove `ArchiveFormat::Single(_)` from the password check at `archive.rs:169`: the test fails |
| E2E-046 | remove `ArchiveFormat::Single(_)` from the password check at `archive.rs:169`: the test fails |
| E2E-017 | let `output_name` accept an empty stem: the test fails |
| E2E-041 | let `output_name` accept a `.` stem: the test fails |
| E2E-042 | let `output_name` accept an empty stem: the test fails |
| E2E-005 | move the `.xz` row before `.tar.xz` in `SUFFIXES`: the test fails |
| E2E-006 | move the `.xz` row before `.txz` in `SUFFIXES`: the test fails |
| E2E-047 | move the `.xz` row before `.tar.xz` in `SUFFIXES`: the test fails |
| E2E-048 | add a `(".lz", ArchiveFormat::Single(SingleFormat::Xz))` row: the test fails |
| E2E-037 | remove the `.xz` row from `SUFFIXES`: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `archive.rs` unless a test proves the rule wrong for xz
- Dependencies not to add: none
- If a test fails on arrival the story fixes the shared rule for all formats and says so in its commit body
- Scope boundary: collision, destination and order belong to US-0181; limits to US-0182

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-016, E2E-045, E2E-046, E2E-017, E2E-041, E2E-042, E2E-005, E2E-006, E2E-047, E2E-048, E2E-037): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0180: xz password, output name, tar rows and the HTTP answer`, body `Spec: SPEC-0082` and the requirement ids.
