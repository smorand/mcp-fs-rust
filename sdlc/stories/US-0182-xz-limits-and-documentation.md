# US-0182: check the xz storage and session limits and document the format

> Spec: SPEC-0082 (sdlc/specs/SPEC-0082-extract-xz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0181
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
An xz decompression bomb is stopped mid block at the lower of the two limits, with the same refusals as gzip and bzip2, and the documentation lists the bare xz format. This closes SPEC-0082.

## Requirements covered
- SPEC-0082/FR-NEW-005: IF the project has a storage limit AND the net growth of the project's stored bytes caused by the output exceeds the remaining storage THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- SPEC-0082/FR-NEW-006: IF the output exceeds the member's remaining session write allowance THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0082_008 e2e_0082_009 e2e_0082_010 e2e_0082_029 e2e_0082_030 e2e_0082_031 e2e_0082_038 e2e_0082_058`: all 8 tests pass. The bomb tests assert `high_water <= cap + 65 536` and `< 1 000 000`, both through `LAST_HIGH_WATER` after the whole extraction and through a direct `read_capped(decoder(SingleFormat::Xz, &xz_bomb()).unwrap(), cap)`; liblzma streams, so the bound is real (spike: 1 000 byte cap, `high_water` 0, RSS delta at most 144 KB).

## Design excerpt
### Decisions
- **DEC-001:** xz reuses the single file flow and rules of SPEC-0081 and SPEC-0083 (`extract_single`, `read_capped`, `caps`, `check_destination`, `check_collision`, admission, order). **Obliged by:** SPEC-0082/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine per operation (AGENTS.md conventions); the spec's rules are those of the gzip increment word for word. **Alternatives considered:** a parallel xz flow. **Source:** existing code and SPEC-0083 DEC-001. **Evidence:** `crates/core/src/tools/archive_single.rs:212-277`, `sdlc/specs/SPEC-0083-extract-bz2-single-file/sdlc-design.md`.
- **DEC-006:** no decoder memory limit; the bomb tests measure memory with the existing hooks: `LAST_HIGH_WATER` after the whole extraction (`assert_extraction_bounded`, `archive_single.rs:837-842`) and a direct `read_capped(decoder(Xz, &bomb)?, cap)` (`assert_bomb_bounded` pattern, `:822-833`), both asserting at most `cap + 65 536` bytes of output held and under 1 000 000. Process RSS is not asserted in a test: reading it needs `unsafe` (forbidden workspace wide) or spawning `ps`, which races with the parallel test threads. **Obliged by:** SPEC-0082/FR-NEW-005, FR-NEW-006 (E2E-009, E2E-010), NFR 7.2. **Rationale:** spike, release build, RSS read with `ps` before and during a capped read (cap 1 000 bytes) of 10 000 000 zero bytes: xz preset 6 file (8 MiB dictionary, 1 592 bytes) RSS delta 144 KB; liblzma encoder with 64 KiB, 8 MiB and 64 MiB dictionaries (1 584 bytes each) RSS delta 16 KB, 16 KB, 48 KB; the first `read` returns 65 536 bytes and the cap stops at `high_water` 0. liblzma touches dictionary pages only as it writes them, so memory follows the bytes decoded, which the cap bounds. A memory limit would refuse legitimate files (xz `-9` needs 65 MiB) for no spec requirement. **Alternatives considered:** `memlimit` of 64 or 256 MiB; an RSS assertion through `ps`. **Source:** spike. **Evidence:** spike output quoted in this entry (throwaway crate, not committed), `archive_single.rs:51-57`, `:822-842`.
- **DEC-007:** fixtures are generated in the test with liblzma's encoder: plain files `liblzma::write::XzEncoder::new(Vec::new(), 6)`; E2E-015 a `Stream::new_stream_encoder(&filters, Check::Crc64)` with `Filters::new()` then `.x86()` then `.lzma2(&LzmaOptions::new_preset(6)?)`; E2E-056 the same with `Check::Sha256` and only `.lzma2`; several streams are concatenated byte vectors; padding is appended zero bytes; the damaged checksum flips the byte at offset 24 of the CSV fixture (all 72 flips measured as refused); the bomb is preset 6 over 10 000 000 zero bytes (1 584 bytes, one block, 8 MiB dictionary); tar fixtures keep `lzma_rs::xz_compress` (`archive.rs:1991`). The design test for an unknown check encodes `hello` with `Check::Crc32`, sets the stream flags check byte to 0x02 at offset 7 and at `len - 3`, and recomputes the header CRC32 (bytes 6 to 7, stored at 8 to 11) and the footer CRC32 (bytes `len - 8` to `len - 3`, stored at `len - 12` to `len - 9`) with `flate2::Crc`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-007, FR-NEW-008, FR-NEW-013 (fixtures "provided by the design", spec section 9). **Rationale:** no test depends on an installed `xz` binary (present here, 5.8.4, but not guaranteed on another machine); `flate2` is already a dependency. **Alternatives considered:** committed fixture files made with `xz`; `crc32fast` (not a direct dependency). **Source:** spike. **Evidence:** spike output; `Cargo.toml:82`.
- **DEC-010:** documentation as SPEC-0083 DEC-005: README formats list and single file paragraph gain `.xz` and say it is decoded by liblzma (C), not in pure Rust; AGENTS.md's `archive_single.rs` entry and stack line name xz and liblzma; `.agent_docs/tools.md` formats list and a dated SPEC-0082 paragraph (several streams, padding multiple of 4, BCJ and SHA-256 decode, `.lzma` refused). **Obliged by:** n/a (no code impact): project convention (AGENTS.md "keep README.md and AGENTS.md up to date"). **Rationale:** the README claims "all decoded in pure Rust" for the archive formats it lists, which a bare `.xz` would make wrong if listed there. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `README.md:294-303`, `AGENTS.md:137-140`, `.agent_docs/tools.md:180-183`, `:220-240`.
### Data model
None.
### Contracts
No code contract: `read_capped`, `caps` and `admit_paths` are SPEC-0081's. Test helper: `fn xz_bomb() -> Vec<u8>`, `xz(&vec![0u8; BOMB_LEN])` (preset 6, 1 584 bytes, one block, 8 MiB dictionary); `assert_bomb_bounded` (`archive_single.rs:822`) takes the format as a parameter, or an `assert_xz_bomb_bounded` twin is added. Documentation: `README.md:294-303` formats gain `.xz` with the note that a bare `.xz` is decoded by liblzma (C), not in pure Rust, several streams and zero padding of a multiple of 4 accepted; `AGENTS.md:137-140` names bare xz `.xz` (SPEC-0082) in the `archive_single.rs` entry, and the Stack line gains `liblzma`; `.agent_docs/tools.md:181` lists `.xz`, and a dated `(2026-11-08, SPEC-0082.)` paragraph after the SPEC-0083 one records: same flow, several streams, padding multiple of 4, BCJ filters and SHA-256 decode, `.lzma` data refused as corrupt, tar xz unchanged (BL-0132).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
### Existing patterns
SPEC-0083 story US-0168 tests (`e2e_0083_011` to `e2e_0083_053`, `archive_single.rs:1581-1672`) and their helpers `fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `write_small`, `project_refusal`, `session_refusal`, `assert_extraction_bounded` (`:767-842`). Test helpers: `fixture()`, `seed`, `extract`, `extract_with`, `read`, `exists` in the tests module of `crates/core/src/tools/archive_single.rs` (`:297-319`, `:613-629`); the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0082/E2E-008: Output over the project storage limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/data.csv` does not exist and the remaining storage is still 10 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_008_output_over_the_project_storage_limi`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-009: Decompression bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** a project storage limit with 1 000 bytes of remaining storage and `/uploads/bomb.xz`, an xz file of about 1 KB that decompresses to 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, and the answer arrives without the server holding the 10 000 000 bytes (the server's memory use during the call stays below 1 000 000 bytes above its level before the call)
- **And** `/uploads/bomb` does not exist and the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_009_decompression_bomb_against_the_stora`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-010: Decompression bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project without a storage limit, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.xz` as in E2E-009
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`, reached without the server holding the 10 000 000 bytes (memory as in E2E-009)
- **And** `/uploads/bomb` does not exist and the allowance is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_010_decompression_bomb_against_the_sessi`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-029: Output exactly equal to the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the remaining storage is 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_029_output_exactly_equal_to_the_remainin`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-030: Overwrite counts net growth
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` (16 bytes decoded), an existing `/uploads/data.csv` of 40 bytes, and a project storage limit with 5 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds, `/uploads/data.csv` holds the 16 bytes and the remaining storage is 29 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_030_overwrite_counts_net_growth`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-031: The allowance is charged the full output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, an existing `/uploads/data.csv` of 40 bytes and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds and the allowance has 4 bytes remaining (16 charged, nothing netted)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_031_the_allowance_is_charged_the_full_ou`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-038: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a session write allowance with 10 bytes remaining and `/uploads/data.csv.xz` decompressing to 16 bytes
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 7 bytes remaining
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_038_a_refused_extraction_charges_nothing`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-058: Both limits at the same byte
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006, FR-NEW-012; **Priority:** Medium
- **Given** a project storage limit with 1 000 bytes of remaining storage, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.xz` as in E2E-009
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_058_both_limits_at_the_same_byte`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-008 | accept a net growth one byte over, or drop the project cap: the test fails |
| E2E-009 | read the decoder to its end before comparing with the cap in `read_capped`: the `high_water` assertion fails |
| E2E-010 | read the decoder to its end before comparing with the cap in `read_capped`: the `high_water` assertion fails |
| E2E-029 | refuse a net growth equal to the remaining storage (`>=` for `>` in `read_capped`): the test fails |
| E2E-030 | count the gross output instead of the net growth in `caps`: the test fails |
| E2E-031 | drop the session cap or charge nothing: the test fails |
| E2E-038 | drop the session cap or charge nothing: the test fails |
| E2E-058 | give the tie to the session (`p < session` for `p <= session` in `caps`): the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Documentation without dashes as punctuation
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-008, E2E-009, E2E-010, E2E-029, E2E-030, E2E-031, E2E-038, E2E-058): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md:294-303`, `AGENTS.md:137-140` and the Stack line, `.agent_docs/tools.md:181` and the new SPEC-0082 paragraph.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0082 is green (`cargo test -p mcp-fs-core --lib -- e2e_0082_ xz_0082_`), and every requirement of SPEC-0082 is cited by at least one story with status done or landed.
- [ ] One commit `US-0182: check the xz storage and session limits and document the format`, body `Spec: SPEC-0082` and the requirement ids.
