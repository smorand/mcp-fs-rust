# US-0179: decode several xz streams and padding, refuse corrupt input

> Spec: SPEC-0082 (sdlc/specs/SPEC-0082-extract-xz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0178
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
An xz file made of several streams is decoded whole; zero padding of a multiple of 4 bytes between or after streams is ignored; any other trailing byte, a truncated, empty, damaged or non xz file is the corrupt refusal.

## Requirements covered
- SPEC-0082/FR-NEW-007: WHEN an xz file holds several xz streams back to back THE archive extraction SHALL decode them all and write their concatenation as the one output file.
- SPEC-0082/FR-NEW-008: IF the file is empty, is truncated, is not xz, has a damaged checksum or index, uses a filter or an integrity check type the xz decoder does not support, or has bytes after a valid stream that FR-NEW-013 does not accept as padding, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid xz file:`, and write nothing.
- SPEC-0082/FR-NEW-013: IF zero bytes follow an xz stream, between streams or at the end of the file, AND their count is a multiple of 4 THEN THE archive extraction SHALL ignore them; IF their count is not a multiple of 4 THEN THE archive extraction SHALL refuse as FR-NEW-008 states.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0082_004 e2e_0082_043 e2e_0082_044 e2e_0082_013 e2e_0082_032 e2e_0082_033 e2e_0082_011 e2e_0082_012 e2e_0082_014 e2e_0082_027 e2e_0082_028 e2e_0082_052 xz_0082_`: all 14 tests pass. The stored output after a multi stream call is the concatenation of the streams, read back from the store.

## Design excerpt
### Decisions
- **DEC-003:** the xz decoder is `liblzma::read::XzDecoder::new_stream(bytes, Stream::new_stream_decoder(u64::MAX, liblzma::stream::CONCATENATED | liblzma::stream::TELL_UNSUPPORTED_CHECK)?)`, read errors mapped to `corrupt("xz", e)`. **Obliged by:** SPEC-0082/FR-NEW-007, FR-NEW-008, FR-NEW-013, and FR-NEW-001 (E2E-015, E2E-056). **Rationale:** spike on liblzma 0.4.9 (2026-11-08, vendored or system liblzma 5.8.4), every input built with liblzma's encoder or `xz` 5.8.4: two streams give `hello world`; streams `a`, empty, `b` give `ab`; 4 or 8 zero bytes after a stream and 4 between two streams are accepted; 3 or 5 zero bytes after, and 3 or 6 between, fail with `lzma data error`; `JUNK` after a stream, a 0 byte file, 4 zero bytes alone, the text `not xz` and a file cut in half fail with `premature eof`; every one of the 72 single byte flips of the 16 byte CSV fixture fails; an x86 BCJ filter and a SHA-256 check decode to `hello`; a `.lzma` file fails with `stream/file format not recognized`; a reserved check type 0x02 fails with `liblzma not built with check support`. `new_stream_decoder` reads xz only (FR-NEW-008 "is not xz"); `TELL_UNSUPPORTED_CHECK` makes an unknown check type an error instead of decoding unverified (measured: without the flag the same file decodes to `hello`). Memory limit `u64::MAX`: see DEC-006. **Alternatives considered:** `new_multi_decoder` (auto decoder, accepts `.lzma`, decodes an unknown check unverified); `XzDecoder::new` (one stream; measured: ignores `JUNK`, 3 zero bytes and a second stream). **Source:** spike. **Evidence:** `liblzma-0.4.9/src/read.rs:149-163`, `src/bufread.rs:185-200`, `src/stream.rs:270-275`, `:379`, `:1320`.
- **DEC-006:** no decoder memory limit; the bomb tests measure memory with the existing hooks: `LAST_HIGH_WATER` after the whole extraction (`assert_extraction_bounded`, `archive_single.rs:837-842`) and a direct `read_capped(decoder(Xz, &bomb)?, cap)` (`assert_bomb_bounded` pattern, `:822-833`), both asserting at most `cap + 65 536` bytes of output held and under 1 000 000. Process RSS is not asserted in a test: reading it needs `unsafe` (forbidden workspace wide) or spawning `ps`, which races with the parallel test threads. **Obliged by:** SPEC-0082/FR-NEW-005, FR-NEW-006 (E2E-009, E2E-010), NFR 7.2. **Rationale:** spike, release build, RSS read with `ps` before and during a capped read (cap 1 000 bytes) of 10 000 000 zero bytes: xz preset 6 file (8 MiB dictionary, 1 592 bytes) RSS delta 144 KB; liblzma encoder with 64 KiB, 8 MiB and 64 MiB dictionaries (1 584 bytes each) RSS delta 16 KB, 16 KB, 48 KB; the first `read` returns 65 536 bytes and the cap stops at `high_water` 0. liblzma touches dictionary pages only as it writes them, so memory follows the bytes decoded, which the cap bounds. A memory limit would refuse legitimate files (xz `-9` needs 65 MiB) for no spec requirement. **Alternatives considered:** `memlimit` of 64 or 256 MiB; an RSS assertion through `ps`. **Source:** spike. **Evidence:** spike output quoted in this entry (throwaway crate, not committed), `archive_single.rs:51-57`, `:822-842`.
- **DEC-007:** fixtures are generated in the test with liblzma's encoder: plain files `liblzma::write::XzEncoder::new(Vec::new(), 6)`; E2E-015 a `Stream::new_stream_encoder(&filters, Check::Crc64)` with `Filters::new()` then `.x86()` then `.lzma2(&LzmaOptions::new_preset(6)?)`; E2E-056 the same with `Check::Sha256` and only `.lzma2`; several streams are concatenated byte vectors; padding is appended zero bytes; the damaged checksum flips the byte at offset 24 of the CSV fixture (all 72 flips measured as refused); the bomb is preset 6 over 10 000 000 zero bytes (1 584 bytes, one block, 8 MiB dictionary); tar fixtures keep `lzma_rs::xz_compress` (`archive.rs:1991`). The design test for an unknown check encodes `hello` with `Check::Crc32`, sets the stream flags check byte to 0x02 at offset 7 and at `len - 3`, and recomputes the header CRC32 (bytes 6 to 7, stored at 8 to 11) and the footer CRC32 (bytes `len - 8` to `len - 3`, stored at `len - 12` to `len - 9`) with `flate2::Crc`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-007, FR-NEW-008, FR-NEW-013 (fixtures "provided by the design", spec section 9). **Rationale:** no test depends on an installed `xz` binary (present here, 5.8.4, but not guaranteed on another machine); `flate2` is already a dependency. **Alternatives considered:** committed fixture files made with `xz`; `crc32fast` (not a direct dependency). **Source:** spike. **Evidence:** spike output; `Cargo.toml:82`.
- **DEC-008:** US-0178 wires `.xz` with `Stream::new_stream_decoder(u64::MAX, TELL_UNSUPPORTED_CHECK)` (one stream); US-0179 adds `CONCATENATED`, which turns the several streams, junk and odd padding tests from red to green; US-0180 to US-0182 add tests that characterize rules already built by SPEC-0081 (password, name, tar rows, collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** measured: without `CONCATENATED` two streams give `hello ` and `JUNK` or 3 zero bytes after a stream are ignored, so US-0179's red evidence is real; the shared rules are green on arrival for xz, so proving they bite needs a mutation. **Alternatives considered:** one story of 58 tests; the full flags in US-0178 (US-0179 would then be all green). **Source:** design. **Evidence:** spike.
### Data model
None.
### Contracts
`decoder(Xz, bytes)` uses `Stream::new_stream_decoder(u64::MAX, liblzma::stream::CONCATENATED | liblzma::stream::TELL_UNSUPPORTED_CHECK)`. Read errors keep mapping to `corrupt("xz", e)`: `archive is corrupt or not a valid xz file: <detail>`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
### Existing patterns
Spike results (liblzma 0.4.9, full flags): two streams give `hello world`; `a`, empty, `b` give `ab`; 4 or 8 zero bytes after, 4 between, accepted; 3 after or 3 between fail with `lzma data error`; `JUNK` after, a 0 byte file, the text `not xz`, the first half of a file fail with `premature eof`; any single byte flip of the CSV fixture fails (offset 24 is used). Without `CONCATENATED` (state after US-0178): two streams give `hello `, `JUNK` and 3 zero bytes after a stream are ignored, which is the RED evidence of E2E-004, E2E-043, E2E-044, E2E-013, E2E-032, E2E-033. Multi stream and corrupt test models: `e2e_0083_004` to `e2e_0083_019` with `bz_streams`, `assert_bz_corrupt`, `refused_bz` (`archive_single.rs:1263-1360`). Test helpers: `fixture()`, `seed`, `extract`, `extract_with`, `read`, `exists` in the tests module of `crates/core/src/tools/archive_single.rs` (`:297-319`, `:613-629`); the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0082/E2E-004: Two xz streams back to back
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/two.txt.xz` made of two xz streams, the first holding `hello ` and the second `world`
- **When** the member extracts it
- **Then** `/uploads/two.txt` reads `hello world` and the answer reports 11 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_004_two_xz_streams_back_to_back`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-043: Three xz streams
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/three.txt.xz` made of the streams `a`, `b`, `c`
- **When** the member extracts it
- **Then** `/uploads/three.txt` reads `abc` and 3 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_043_three_xz_streams`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-044: An empty xz stream between two others
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/hole.txt.xz` made of the streams `a`, an empty stream, `b`
- **When** the member extracts it
- **Then** `/uploads/hole.txt` reads `ab`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_044_an_empty_xz_stream_between_two_other`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-013: Bytes after a valid stream
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/tail.xz`, a valid xz file followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/tail` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_013_bytes_after_a_valid_stream`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-032: Padding of 4 bytes between two streams
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/gap.txt.xz` made of a stream holding `a`, 4 zero bytes, then a stream holding `b`
- **When** the member extracts it
- **Then** `/uploads/gap.txt` reads `ab`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_032_padding_of_4_bytes_between_two_strea`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-033: Padding of 3 bytes is refused
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-013, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/pad3.xz`, a valid xz stream followed by 3 zero bytes
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/pad3` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_033_padding_of_3_bytes_is_refused`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-011: Truncated file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/cut.xz`, the first half of a valid xz file
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid xz file:`; `/uploads/cut` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_011_truncated_file`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-012: Text file named `.xz`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/fake.xz` containing the text `not xz`
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/fake` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_012_text_file_named_xz`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-014: Damaged checksum
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/bad.xz`, a valid xz file of `/uploads/data.csv.xz` with one byte of its compressed data changed
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/bad` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_014_damaged_checksum`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-027: A 0 byte file named `zero.xz`
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** a 0 byte file `/uploads/zero.xz`
- **When** the member extracts it
- **Then** the refusal is as E2E-011; `/uploads/zero` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_027_a_0_byte_file_named_zero_xz`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-028: Zero padding after a stream is accepted
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/pad.txt.xz`, a valid xz stream holding `hello` followed by 4 zero bytes
- **When** the member extracts it
- **Then** `/uploads/pad.txt` reads `hello` and 5 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_028_zero_padding_after_a_stream_is_accep`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-052: Padding of 8 bytes after a stream
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-013; **Priority:** Medium
- **Given** `/uploads/pad8.txt.xz`, a valid xz stream holding `hello` followed by 8 zero bytes
- **When** the member extracts it
- **Then** `/uploads/pad8.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_052_padding_of_8_bytes_after_a_stream`
- Green on arrival: the rule is built by earlier stories or by SPEC-0081; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Design test (DEC-003, not a spec test): a legacy `.lzma` file named `.xz` is corrupt
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` / crates/core/src/tools/archive_single.rs tests
- Given `/uploads/old.xz`, the bytes of `liblzma::write::XzEncoder::new_stream(Vec::new(), Stream::new_lzma_encoder(&LzmaOptions::new_preset(6).unwrap()).unwrap())` over `hello` (the LZMA alone format)
- When the member extracts it
- Then the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid xz file:` (spike detail: `stream/file format not recognized`); `/uploads/old` does not exist
- Function: `xz_0082_lzma_alone_file_is_corrupt`
- Green on arrival (US-0178 already uses `new_stream_decoder`); mutation below
- Cleanup: none

### Design test (DEC-003, not a spec test): an integrity check type liblzma does not know is corrupt
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` / crates/core/src/tools/archive_single.rs tests
- Given `/uploads/chk.xz`: `hello` encoded with `Check::Crc32`, then the check byte set to `0x02` at offset 7 and at `len - 3`, the header CRC32 over bytes 6 to 7 rewritten at 8 to 11 and the footer CRC32 over bytes `len - 8` to `len - 3` rewritten at `len - 12` to `len - 9`, little endian, computed with `flate2::Crc` (DEC-007)
- When the member extracts it
- Then the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid xz file:` (spike detail: `liblzma not built with check support`); `/uploads/chk` does not exist
- Function: `xz_0082_unknown_check_type_is_corrupt`
- Green on arrival (US-0178 sets `TELL_UNSUPPORTED_CHECK`); mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-011 | map `ReadError::Io` to an empty successful read in `extract_single`: the test fails |
| E2E-012 | map `ReadError::Io` to an empty successful read in `extract_single`: the test fails |
| E2E-014 | map `ReadError::Io` to an empty successful read in `extract_single`: the test fails |
| E2E-027 | map `ReadError::Io` to an empty successful read in `extract_single`: the test fails |
| E2E-028 | drop `CONCATENATED` and refuse when the decoder's `total_in()` is below the file length (trailing bytes treated as junk): the test fails |
| E2E-052 | drop `CONCATENATED` and refuse when the decoder's `total_in()` is below the file length: the test fails |
| xz_0082_lzma_alone_file_is_corrupt | replace the `Xz` arm by `XzDecoder::new_multi_decoder(bytes)`: the test fails (spike: decodes `hello`) |
| xz_0082_unknown_check_type_is_corrupt | drop `TELL_UNSUPPORTED_CHECK` from the flags: the test fails (spike: decodes `hello`) |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `archive.rs` (tar xz keeps `lzma-rs`, DEC-009)
- Dependencies not to add: none
- Patterns to avoid: no hand written xz parsing; padding and stream boundaries are liblzma's job (DEC-003)
- Scope boundary: password, names, tar rows belong to US-0180; order and collisions to US-0181; limits to US-0182

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-011, E2E-012, E2E-014, E2E-027, E2E-028, E2E-052, xz_0082_lzma_alone_file_is_corrupt, xz_0082_unknown_check_type_is_corrupt): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0179: decode several xz streams and padding, refuse corrupt input`, body `Spec: SPEC-0082` and the requirement ids.
