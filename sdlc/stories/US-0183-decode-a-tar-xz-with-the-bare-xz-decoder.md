# US-0183: decode a .tar.xz with the bare xz decoder

> Spec: SPEC-0084 (sdlc/specs/SPEC-0084-tar-family-bounded-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
A member whose `.tar.xz` or `.txz` was made of several xz streams, with zero padding, a branch filter or a SHA-256 check gets it extracted, where today it is refused as corrupt; a malformed one is refused as `tar.xz` corrupt. The tar family now uses the one xz decoder of the bare `.xz` (SPEC-0082). The decoding bound comes in US-0184.

## Requirements covered
- SPEC-0084/FR-NEW-006: WHEN a `.tar.xz` or `.txz` file holds several xz streams back to back THE archive extraction SHALL decode them all and read their concatenation as the one tar archive.
- SPEC-0084/FR-NEW-007: IF zero bytes follow an xz stream of a `.tar.xz` or `.txz` file, between streams or at the end, AND their count is a multiple of 4 THEN THE archive extraction SHALL ignore them.
- SPEC-0084/FR-NEW-008: WHEN a `.tar.xz` or `.txz` file uses the delta filter, a branch conversion filter that xz defines (x86, PowerPC, IA-64, ARM, ARM-Thumb, SPARC, ARM64, RISC-V) or any integrity check type xz defines (none, CRC32, CRC64, SHA-256) THE archive extraction SHALL decode it.
- SPEC-0084/FR-NEW-009: IF a `.tar.xz` or `.txz` file is empty, is truncated, is not xz, is in the legacy `.lzma` format, has a damaged checksum or index, declares an integrity check type xz does not define, uses a filter the decoder does not know, or has bytes after a stream that FR-NEW-007 does not accept as padding, THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid tar.xz file:`, and write nothing.
- SPEC-0084/FR-MOD-002 (partial: the decoder switch; the dependency removal is US-0186): THE archive extraction SHALL decode a `.tar.xz` or `.txz` file with the rules of a bare `.xz` file (FR-NEW-006 to FR-NEW-009).

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0084_026 e2e_0084_027 e2e_0084_028 e2e_0084_029 e2e_0084_030 e2e_0084_031 e2e_0084_032 e2e_0084_033 e2e_0084_034 e2e_0084_035 e2e_0084_036 e2e_0084_037 e2e_0084_038`: 13 tests pass. Real assertion: the extracted `/uploads/pack/a.txt` read back from the volume holds `hello` for the several stream, padded, x86 and SHA-256 archives, which `lzma-rs` refuses today.

## Design excerpt
### Decisions
- **DEC-004 (SPEC-0084):** decoders: `.tar.gz`/`.tgz` `flate2::read::GzDecoder`, `.tar.bz2`/`.tb2` `bzip2::read::BzDecoder` (single member and single stream, as today), `.tar.xz`/`.txz` `archive_single::decoder(SingleFormat::Xz, bytes)` (liblzma explicit stream decoder with `CONCATENATED | TELL_UNSUPPORTED_CHECK`, SPEC-0082 DEC-003); a plain `.tar` is not decompressed and gets no bound. Read errors map to `corrupt(<format>, e)`; `ReadError::Init` maps to `ToolError::internal("cannot start the tar.xz decoder: <e>")`. **Obliged by:** SPEC-0084/FR-MOD-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-011 (spec 3.2: several gzip members or bzip2 streams stay out). **Rationale:** the bare xz decoder already implements every `.tar.xz` rule of the spec (measured in SPEC-0082 DEC-003: several streams, padding of a multiple of 4, BCJ, SHA-256 accepted; `.lzma`, unknown check, junk refused). **Alternatives considered:** `MultiGzDecoder`/`MultiBzDecoder` for the tar family (a behavior change the spec excludes); keeping `lzma-rs` (refuses padding, BCJ, SHA-256, several streams: the defect of BL-0132). **Source:** spec constraint and existing code. **Evidence:** `crates/core/src/tools/archive_single.rs:64-86`, `crates/core/src/tools/archive.rs:727-749`, `liblzma-0.4.9/src/stream.rs:1320`.
- **DEC-008 (SPEC-0084):** test layout: the `e2e_0084_*` tests live in a new test module `crates/core/src/tools/archive_tar_bound_e2e.rs` (`#[cfg(test)] mod archive_tar_bound_e2e;` in `tools/mod.rs`, precedent `quota_e2e.rs`, `diff_files_e2e.rs`); helpers shared by SPEC-0084 to SPEC-0087 live in `crates/core/src/tools/archive_testkit.rs` (`#[cfg(test)] pub(crate) mod archive_testkit;`): `MOUNT = "atlas"`, `OWNER = "mia@example.com"`, `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (authorizes then calls `extract_archive`), `read`, `exists`, `limit_remaining(f, n)` (exact bytes), `remaining_storage`, `allowance`, `held_reset`, `held`, `held_some`, `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal`, `corrupt_refusal`. The HTTP test stays with its harness in `api/dataplane.rs`. **Obliged by:** n/a (no code impact): test layout. **Rationale:** `archive.rs` (2 976 lines) and `archive_single.rs` (2 429 lines) already hold their own suites; four specs add about 190 tests sharing the same helpers. **Alternatives considered:** appending to `archive.rs` tests; one helper copy per module. **Source:** existing code. **Evidence:** `crates/core/src/tools/mod.rs:39-91`, `crates/core/src/tools/archive_single.rs:786-866`.
- **DEC-009 (SPEC-0084):** fixtures are generated in the tests: tar bytes with `tar::Builder` and GNU headers (`archive.rs:1297-1311` pattern), gzip `flate2::write::GzEncoder`, bzip2 `bzip2::write::BzEncoder`, xz `liblzma::write::XzEncoder::new(_, 6)` (CRC64), x86 and SHA-256 and CRC32 variants through `liblzma::stream::Stream::new_stream_encoder` with `Filters` (SPEC-0082 DEC-007 recipe), legacy `.lzma` through `Stream::new_lzma_encoder(&LzmaOptions::new_preset(6)?)`, the reserved check type 3 by patching the stream flags byte at offset 7 and `len - 3` and recomputing both CRC32 with `flate2::Crc` (SPEC-0082 DEC-007 recipe); several streams are concatenations; the bomb is a tar holding `zeros.bin`, 10 000 000 zero bytes, declared as such; the exact size archive is the tar bytes of `a.txt` (2 048 bytes: header, one data block, two end blocks) extended with zero bytes to exactly 1 049 576 (E2E-014) or 1 049 577 (E2E-015) before compression; the lying entry is one GNU header declaring 1 000 000 000 followed by the 5 bytes `hello` and nothing else. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-006 to FR-NEW-010 (spec section 9: fixtures built by the design). **Rationale:** no committed binary, no external tool; every recipe already proven in SPEC-0082. **Alternatives considered:** committed fixture files. **Source:** existing code and SPEC-0082 DEC-007. **Evidence:** `crates/core/src/tools/archive_single.rs:1700-1720`, `:2319-2322`.
### Data model
None.
### Contracts
`archive_single::decoder(format: SingleFormat, bytes: &[u8]) -> std::io::Result<Box<dyn Read + '_>>` becomes `pub(crate)` (body unchanged). In `decode_tar` (`archive.rs:720-793`) the `TarCompression::Xz` arm becomes: `archive_single::decoder(SingleFormat::Xz, bytes).map_err(|e| ToolError::internal(format!("cannot start the tar.xz decoder: {e}")))?` then `read_to_end` mapped with `corrupt("tar.xz", e)`. The gzip and bzip2 arms do not change. Corrupt message: `archive is corrupt or not a valid tar.xz file: <detail>`. New files: `crates/core/src/tools/archive_testkit.rs` (`#[cfg(test)] pub(crate) mod archive_testkit;`) and `crates/core/src/tools/archive_tar_bound_e2e.rs` (`#[cfg(test)] mod archive_tar_bound_e2e;`), both declared in `crates/core/src/tools/mod.rs` next to `quota_e2e`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/tools/archive_testkit.rs` | create |
| `crates/core/src/tools/archive_tar_bound_e2e.rs` | create |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0084/E2E-026: Two xz streams
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-006, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` made of two xz streams, the first holding the first 512 bytes of a tar with `a.txt` (`hello`), the second holding the rest
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello` and the answer reports 1 file, 5 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_026_two_xz_streams`
- Red before the change because: `lzma-rs` reads one stream and fails on the second (`lzma-rs-0.3.0/src/decode/xz.rs:86-90`): today the refusal is `archive is corrupt or not a valid tar.xz file: ...`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-027: x86 branch filter
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-008, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` compressed with the x86 branch filter before the main compression, holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_027_x86_branch_filter`
- Red before the change because: `lzma-rs` knows only LZMA2 and refuses the x86 filter id
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-028: SHA-256 integrity check
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-008, FR-MOD-002; **Priority:** High
- **Given** `/uploads/pack.tar.xz` made with a SHA-256 integrity check, holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_028_sha_256_integrity_check`
- Red before the change because: `lzma-rs` refuses the SHA-256 check (`lzma-rs-0.3.0/src/decode/xz.rs:326-329`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-029: `.txz` of two streams with a CRC32 check
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-006, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/pack.txz` made of two streams with a CRC32 check, together holding a tar with `a.txt` (`hello`)
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_029_txz_of_two_streams_with_a_crc32_chec`
- Red before the change because: the second stream is refused by `lzma-rs`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-030: 4 zero bytes after the stream
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-003 followed by 4 zero bytes
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_030_4_zero_bytes_after_the_stream`
- Red before the change because: `lzma-rs` refuses any byte after the stream
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-031: 4 zero bytes between two streams
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-026 with 4 zero bytes between its two streams
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_031_4_zero_bytes_between_two_streams`
- Red before the change because: padding between streams is refused by `lzma-rs`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-032: Empty stream between two streams
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-026 with an empty xz stream inserted between its two streams
- **When** the member extracts it
- **Then** `/uploads/pack/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_032_empty_stream_between_two_streams`
- Red before the change because: the inserted empty stream is a second stream, refused by `lzma-rs`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-033: 3 zero bytes after the stream
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-003 followed by 3 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`; `/uploads/pack` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_033_3_zero_bytes_after_the_stream`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-034: Other bytes after the stream
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/pack.tar.xz` of E2E-003 followed by the 4 bytes `JUNK`
- **When** the member extracts it
- **Then** the refusal is as E2E-033
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_034_other_bytes_after_the_stream`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-035: Legacy `.lzma` file named `.tar.xz`
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` holding the tar of E2E-003 compressed in the legacy `.lzma` format
- **When** the member extracts it
- **Then** the refusal is as E2E-033
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_035_legacy_lzma_file_named_tar_xz`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-036: Integrity check type xz does not define
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz` of E2E-003 whose stream header and footer declare the reserved check type 3, with valid header and footer checksums
- **When** the member extracts it
- **Then** the refusal is as E2E-033
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_036_integrity_check_type_xz_does_not_def`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-037: Truncated and empty files
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/cut.tar.xz`, the first half of `/uploads/pack.tar.xz` of E2E-003, and `/uploads/zero.tar.xz`, a 0 byte file
- **When** the member extracts each
- **Then** each refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`; neither `/uploads/cut` nor `/uploads/zero` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_037_truncated_and_empty_files`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-038: Damaged compressed byte
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/uploads/pack.tar.xz` of E2E-003 with one byte of its compressed data changed
- **When** the member extracts it
- **Then** the refusal is as E2E-033
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_038_damaged_compressed_byte`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-033 | drop `CONCATENATED` from the flags of the xz stream decoder in `archive_single::decoder`: liblzma then stops after the first stream and ignores the 3 trailing zero bytes (measured in SPEC-0082 DEC-008), the call succeeds, the test fails |
| E2E-034 | drop `CONCATENATED` as for E2E-033: `JUNK` after the stream is ignored, the call succeeds, the test fails |
| E2E-035 | build the decoder with `liblzma::stream::Stream::new_auto_decoder(u64::MAX, CONCATENATED)` instead of `new_stream_decoder`: the legacy `.lzma` data decodes (SPEC-0082 DEC-003), the call succeeds, the test fails |
| E2E-036 | drop `TELL_UNSUPPORTED_CHECK` from the decoder flags: liblzma decodes the stream without verifying the unknown check (SPEC-0082 DEC-003 measured it for type 2), the call succeeds, the test fails |
| E2E-037 | in the xz arm map a read error to an empty output (`.unwrap_or_default()` instead of `corrupt`): the empty tar parses to zero entries and the call succeeds, the test fails |
| E2E-038 | the E2E-037 mutation: the damaged stream decodes to an empty output, the call succeeds, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- The memory hook is the `cfg(test)` thread local `archive_single::LAST_HIGH_WATER`; set it on the async thread after `spawn_blocking` returns (the blocking thread is another thread), and keep tests on the default single threaded `#[tokio::test]`
- No dash as punctuation in code, comments or docs
- Scope boundary: no decoding bound yet (US-0184); `lzma-rs` stays a dependency until US-0186 (its fixtures still compile); nothing about plain `.tar`, gzip or bzip2 changes

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-026, E2E-027, E2E-028, E2E-029, E2E-030, E2E-031, E2E-032), before the production code.
- [ ] Green on arrival tests (E2E-033, E2E-034, E2E-035, E2E-036, E2E-037, E2E-038): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` (includes `tools::archive_single`) passes unchanged, the bare format suites `e2e_0081_`, `e2e_0082_`, `e2e_0083_` and `e2e_new_` included; `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes; no change to the tool contract; `e2e_0082_005_tar_xz_is_still_a_tar`, `e2e_0082_006_txz_is_still_a_tar`, `e2e_0082_047_tar_xz_is_still_a_tar` and `e2e_new_002_six_formats_extract_default_destination` stay green with their `lzma-rs` made fixtures (liblzma reads them)
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0183: decode a .tar.xz with the bare xz decoder`, body `Spec: SPEC-0084` and the requirement ids (SPEC-0084/FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-MOD-002).
