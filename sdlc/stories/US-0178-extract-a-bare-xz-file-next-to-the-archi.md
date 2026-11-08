# US-0178: extract a bare xz file next to the archive

> Spec: SPEC-0082 (sdlc/specs/SPEC-0082-extract-xz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
A member can extract `/uploads/data.csv.xz` and get `/uploads/data.csv` next to it, with the destination option and the same answer and audit line as for gzip and bzip2. Files made with an x86 branch filter or a SHA-256 integrity check decode. This story adds `liblzma` as a direct dependency.

## Requirements covered
- SPEC-0082/FR-NEW-001: WHEN a member asks to extract a file whose name ends in `.xz`, in any letter case, and does not end in a tar suffix, THE archive extraction SHALL write one file named like the archive without its `.xz` suffix, in the destination folder, and answer the destination, 1 file written, the folders created and the bytes written, and record one audit line.
- SPEC-0082/FR-NEW-002: WHEN a member gives a destination THE archive extraction SHALL write the output file in that folder, create the folder when it does not exist, and count it in the folders created.
- SPEC-0082/FR-MOD-001: THE archive extraction SHALL treat a bare `.xz` as a supported extension.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0082_001 e2e_0082_002 e2e_0082_003 e2e_0082_015 e2e_0082_026 e2e_0082_034 e2e_0082_035 e2e_0082_036 e2e_0082_039 e2e_0082_040 e2e_0082_056 e2e_0082_057 detect_format_maps_every_suffix`: all 13 tests pass. The stored `/uploads/data.csv` equals the 16 byte fixture, read back from the store; `cargo tree -p mcp-fs-core --depth 1 -e normal` lists `liblzma v0.4.9`; `git diff Cargo.lock` is the one line `"liblzma"` in the `mcp-fs-core` entry.

## Design excerpt
### Decisions
- **DEC-001:** xz reuses the single file flow and rules of SPEC-0081 and SPEC-0083 (`extract_single`, `read_capped`, `caps`, `check_destination`, `check_collision`, admission, order). **Obliged by:** SPEC-0082/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine per operation (AGENTS.md conventions); the spec's rules are those of the gzip increment word for word. **Alternatives considered:** a parallel xz flow. **Source:** existing code and SPEC-0083 DEC-001. **Evidence:** `crates/core/src/tools/archive_single.rs:212-277`, `sdlc/specs/SPEC-0083-extract-bz2-single-file/sdlc-design.md`.
- **DEC-002:** `liblzma` 0.4.9 becomes a direct dependency with its default features: `liblzma = "0.4.9"` in `[workspace.dependencies]`, `liblzma = { workspace = true }` in `crates/core/Cargo.toml`; `lzma-rs` stays for the tar family. **Obliged by:** SPEC-0082/FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-013, and the spec's erratum 1 (a spec constraint: the spec names the library). **Rationale:** the only decoder that streams mid block, decodes concatenated streams and validates padding without custom parsing; already compiled in every build, lockfile delta one line, licences allowed, deny and audit baselines unaffected. Default features keep the current link behavior (system liblzma at least 5.8.0 through pkg-config, else vendored static). **Alternatives considered:** `lzma-rs` plus a hand written splitter (design issue option c); the `static` feature (changes datafusion's build too, no requirement asks for it); `xz2` (older fork, superseded by liblzma). **Source:** spec constraint (erratum 1, design-issues.md resolution 2026-11-05). **Evidence:** context7 `/websites/rs_liblzma_liblzma`; crates.io max stable `liblzma` 0.4.9, `liblzma-sys` 0.4.9; `Cargo.lock:3575-3592`; `cargo tree -p mcp-fs-core -i liblzma -e features`; `liblzma-sys-0.4.9/build.rs:13-40`; `cargo deny check` and `cargo audit` runs of 2026-11-08.
- **DEC-003:** the xz decoder is `liblzma::read::XzDecoder::new_stream(bytes, Stream::new_stream_decoder(u64::MAX, liblzma::stream::CONCATENATED | liblzma::stream::TELL_UNSUPPORTED_CHECK)?)`, read errors mapped to `corrupt("xz", e)`. **Obliged by:** SPEC-0082/FR-NEW-007, FR-NEW-008, FR-NEW-013, and FR-NEW-001 (E2E-015, E2E-056). **Rationale:** spike on liblzma 0.4.9 (2026-11-08, vendored or system liblzma 5.8.4), every input built with liblzma's encoder or `xz` 5.8.4: two streams give `hello world`; streams `a`, empty, `b` give `ab`; 4 or 8 zero bytes after a stream and 4 between two streams are accepted; 3 or 5 zero bytes after, and 3 or 6 between, fail with `lzma data error`; `JUNK` after a stream, a 0 byte file, 4 zero bytes alone, the text `not xz` and a file cut in half fail with `premature eof`; every one of the 72 single byte flips of the 16 byte CSV fixture fails; an x86 BCJ filter and a SHA-256 check decode to `hello`; a `.lzma` file fails with `stream/file format not recognized`; a reserved check type 0x02 fails with `liblzma not built with check support`. `new_stream_decoder` reads xz only (FR-NEW-008 "is not xz"); `TELL_UNSUPPORTED_CHECK` makes an unknown check type an error instead of decoding unverified (measured: without the flag the same file decodes to `hello`). Memory limit `u64::MAX`: see DEC-006. **Alternatives considered:** `new_multi_decoder` (auto decoder, accepts `.lzma`, decodes an unknown check unverified); `XzDecoder::new` (one stream; measured: ignores `JUNK`, 3 zero bytes and a second stream). **Source:** spike. **Evidence:** `liblzma-0.4.9/src/read.rs:149-163`, `src/bufread.rs:185-200`, `src/stream.rs:270-275`, `:379`, `:1320`.
- **DEC-004:** `decoder` returns `std::io::Result<Box<dyn Read + '_>>`; inside the blocking task `decoder(format, &bytes).map_err(ReadError::Init).and_then(|r| read_capped(r, cap))`; `ReadError::Init(e)` maps to `ToolError::internal(format!("cannot start the {label} decoder: {e}"))`. **Obliged by:** SPEC-0082/FR-NEW-008 (a corrupt refusal names a fault of the file, and a decoder that cannot start is a fault of the server). **Rationale:** `Stream::new_stream_decoder` returns `Result` (only memory or option errors); production code may not `unwrap` it (rust skill). **Alternatives considered:** `unwrap`/`expect` as `new_multi_decoder` does internally; mapping to the corrupt refusal. **Source:** design. **Evidence:** `liblzma-0.4.9/src/stream.rs:379`, `archive_single.rs:64-69`, `:239-251`.
- **DEC-005:** table row `(".xz", ArchiveFormat::Single(SingleFormat::Xz))` after `(".bz2", ...)`; `label()` is `"xz"`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** `detect_format` takes the first `ends_with` match in table order (`archive.rs:68-79`), so `.tar.xz` and `.txz` (rows `:49-50`) win over the plain `.xz`. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `archive.rs:44-62`.
- **DEC-007:** fixtures are generated in the test with liblzma's encoder: plain files `liblzma::write::XzEncoder::new(Vec::new(), 6)`; E2E-015 a `Stream::new_stream_encoder(&filters, Check::Crc64)` with `Filters::new()` then `.x86()` then `.lzma2(&LzmaOptions::new_preset(6)?)`; E2E-056 the same with `Check::Sha256` and only `.lzma2`; several streams are concatenated byte vectors; padding is appended zero bytes; the damaged checksum flips the byte at offset 24 of the CSV fixture (all 72 flips measured as refused); the bomb is preset 6 over 10 000 000 zero bytes (1 584 bytes, one block, 8 MiB dictionary); tar fixtures keep `lzma_rs::xz_compress` (`archive.rs:1991`). The design test for an unknown check encodes `hello` with `Check::Crc32`, sets the stream flags check byte to 0x02 at offset 7 and at `len - 3`, and recomputes the header CRC32 (bytes 6 to 7, stored at 8 to 11) and the footer CRC32 (bytes `len - 8` to `len - 3`, stored at `len - 12` to `len - 9`) with `flate2::Crc`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-007, FR-NEW-008, FR-NEW-013 (fixtures "provided by the design", spec section 9). **Rationale:** no test depends on an installed `xz` binary (present here, 5.8.4, but not guaranteed on another machine); `flate2` is already a dependency. **Alternatives considered:** committed fixture files made with `xz`; `crc32fast` (not a direct dependency). **Source:** spike. **Evidence:** spike output; `Cargo.toml:82`.
- **DEC-008:** US-0178 wires `.xz` with `Stream::new_stream_decoder(u64::MAX, TELL_UNSUPPORTED_CHECK)` (one stream); US-0179 adds `CONCATENATED`, which turns the several streams, junk and odd padding tests from red to green; US-0180 to US-0182 add tests that characterize rules already built by SPEC-0081 (password, name, tar rows, collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** measured: without `CONCATENATED` two streams give `hello ` and `JUNK` or 3 zero bytes after a stream are ignored, so US-0179's red evidence is real; the shared rules are green on arrival for xz, so proving they bite needs a mutation. **Alternatives considered:** one story of 58 tests; the full flags in US-0178 (US-0179 would then be all green). **Source:** design. **Evidence:** spike.
- **DEC-009:** the tar family keeps `lzma-rs`; its decode path (`archive.rs:742-746`) is not touched. **Obliged by:** SPEC-0082/FR-NEW-010, spec section 3.2. **Rationale:** the spec pins the tar behavior as unchanged; the move is BL-0132. **Alternatives considered:** moving tar xz to liblzma now. **Source:** spec constraint. **Evidence:** `sdlc/backlog/BL-0132-move-the-tar-xz-decoder-to-liblzma.md`.
### Data model
None.
### Contracts
Manifests: `liblzma = "0.4.9"` in `[workspace.dependencies]` of `Cargo.toml`, after `lzma-rs = "0.3"` (`Cargo.toml:87`); `liblzma = { workspace = true }` in `crates/core/Cargo.toml` after `lzma-rs` (`:72`); default features (no `features = [...]`). Table row `(".xz", ArchiveFormat::Single(SingleFormat::Xz))` after `.bz2` (`archive.rs:61`). `SingleFormat::Xz`, `label() == "xz"`. `fn decoder(format: SingleFormat, bytes: &[u8]) -> std::io::Result<Box<dyn Read + '_>>`: `Gzip` and `Bzip2` arms wrap their reader in `Ok`; `Xz` returns `liblzma::read::XzDecoder::new_stream(bytes, Stream::new_stream_decoder(u64::MAX, liblzma::stream::TELL_UNSUPPORTED_CHECK)?)` (ONE stream in this story: no `CONCATENATED`, DEC-008; US-0179 adds it). New `ReadError::Init(std::io::Error)`; the blocking task runs `decoder(format, &bytes).map_err(ReadError::Init).and_then(|r| read_capped(r, cap))`; `ReadError::Init(e)` maps to `ToolError::internal(format!("cannot start the {label} decoder: {e}"))`. The test helper `assert_bomb_bounded` (`archive_single.rs:822`) unwraps the new `Result`. `detect_format_maps_every_suffix` (`archive.rs:1044`) gains `("/a/x.xz", Single(Xz), 3)` and `("/a/X.CSV.XZ", Single(Xz), 3)` and keeps its `.tar.xz`/`.txz` rows.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify |
| `crates/core/Cargo.toml` | modify |
| `Cargo.lock` | modify (generated, one line) |
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_single.rs` | modify |
### Existing patterns
`SingleFormat`, `decoder`, `extract_single` (`archive_single.rs:17-69`, `:212-277`); `SUFFIXES` (`archive.rs:44-62`). The bzip2 tests `e2e_0083_001` to `e2e_0083_051` (`archive_single.rs:1087-1262`) are the model for every test of this story. Fixture helper to add once (US-0178), next to `bzip2()` at `archive_single.rs:1087`: `fn xz(content: &[u8]) -> Vec<u8>` with `liblzma::write::XzEncoder::new(Vec::new(), 6)`, `write_all`, `finish` (DEC-007). E2E-015 fixture: `let mut f = Filters::new(); f.x86(); f.lzma2(&LzmaOptions::new_preset(6).unwrap());` then `Stream::new_stream_encoder(&f, Check::Crc64)` into `liblzma::write::XzEncoder::new_stream(Vec::new(), stream)`; E2E-056 the same with only `f.lzma2(..)` and `Check::Sha256` (spike: both decode to `hello`). Test helpers: `fixture()`, `seed`, `extract`, `extract_with`, `read`, `exists` in the tests module of `crates/core/src/tools/archive_single.rs` (`:297-319`, `:613-629`); the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0082/E2E-001: xz file decompresses next to the archive
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** a member and `/uploads/data.csv.xz`, the xz compression of the 16 byte text `id,name`, newline, `1,Alice`, newline; nothing at `/uploads/data.csv`
- **When** the member extracts `/uploads/data.csv.xz`
- **Then** the answer is destination `/uploads`, 1 file written, 0 folders created, 16 bytes written
- **And** `/uploads/data.csv` reads the 16 bytes; `/uploads/data.csv.xz` is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_001_xz_file_decompresses_next_to_the_arc`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-002: Upper case suffix
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** the same content stored as `/uploads/DATA.CSV.XZ`
- **When** the member extracts it
- **Then** `/uploads/DATA.CSV` exists with the 16 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_002_upper_case_suffix`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-003: Explicit existing destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/data.csv.xz` and the existing empty folder `/out`
- **When** the member extracts it to the destination `/out`
- **Then** the answer is destination `/out`, 1 file written, 0 folders created; `/out/data.csv` exists and `/uploads/data.csv` does not
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_003_explicit_existing_destination`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-015: A file with an x86 branch filter decodes
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/bcj.xz`, a valid xz file made with an additional x86 branch filter, holding `hello`
- **When** the member extracts it
- **Then** `/uploads/bcj` reads `hello` and 1 file and 5 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_015_a_file_with_an_x86_branch_filter_dec`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-026: xz of an empty file
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/empty.txt.xz`, the xz compression of a 0 byte file
- **When** the member extracts it
- **Then** the answer is 1 file written, 0 bytes written; `/uploads/empty.txt` exists with 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_026_xz_of_an_empty_file`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-034: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/rapport été.txt.xz`
- **When** the member extracts it
- **Then** `/uploads/rapport été.txt` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_034_accents_and_spaces`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-035: Several dots and no dot
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/a.b.c.xz` and `/uploads/README.xz`
- **When** the member extracts each
- **Then** `/uploads/a.b.c` and `/uploads/README` exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_035_several_dots_and_no_dot`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-036: Success is audited
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the situation of E2E-001
- **When** the member extracts it
- **Then** the audit trail has exactly one extraction line for `/uploads/data.csv.xz` with the destination, 1 file written and 16 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_036_success_is_audited`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-039: Missing destination is created
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and no folder `/new/dir`
- **When** the member extracts it to the destination `/new/dir`
- **Then** the answer is destination `/new/dir`, 1 file written, 1 folder created; `/new/dir/data.csv` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_039_missing_destination_is_created`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-040: Trailing slash on the destination
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/data.csv.xz` and the existing folder `/out`
- **When** the member extracts it to the destination `/out/`
- **Then** the answer is destination `/out` and `/out/data.csv` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_040_trailing_slash_on_the_destination`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-056: A file with a SHA-256 integrity check decodes
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/sha.xz`, a valid xz file made with a SHA-256 integrity check, holding `hello`
- **When** the member extracts it
- **Then** `/uploads/sha` reads `hello` and 1 file and 5 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_056_a_file_with_a_sha_256_integrity_chec`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0082/E2E-057: Archive directly under the project root
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/a.xz`, the xz compression of `hello`
- **When** the member extracts it
- **Then** the answer is destination `/`, 1 file written, 0 folders created, 5 bytes written; `/a` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0082_057_archive_directly_under_the_project_r`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `crates/core/src/api/dataplane.rs`; the tar decode path `archive.rs:742-746` (`lzma-rs` stays for `.tar.xz`/`.txz`, DEC-009)
- Dependencies: add `liblzma` 0.4.9 only, default features; no `static`, `parallel` or other feature; do not remove `lzma-rs`
- Do not use `XzDecoder::new_multi_decoder` (auto decoder, accepts `.lzma`, DEC-003)
- Scope boundary: several streams, padding, corrupt input belong to US-0179; password, names, tar rows, HTTP to US-0180; collision, destination, order to US-0181; limits and docs to US-0182

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified (`detect_format_maps_every_suffix` gains two rows; `assert_bomb_bounded` unwraps the decoder `Result`); no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] `make security` (cargo audit + cargo deny check) as green as the 2026-11-08 baseline: deny `advisories ok, bans ok, licenses ok, sources ok`, audit only the 2 allowed warnings (RUSTSEC-2026-0441, RUSTSEC-2026-0442).
- [ ] `git diff Cargo.lock` shows exactly one added line, `"liblzma"` in the `mcp-fs-core` dependency list.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0178: extract a bare xz file next to the archive`, body `Spec: SPEC-0082` and the requirement ids.
