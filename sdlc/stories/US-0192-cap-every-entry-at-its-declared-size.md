# US-0192: cap every entry at its declared size

> Spec: SPEC-0085 (sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0191
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Whatever the decoding library does, no zip or 7z entry is held past its declared size plus one 64 KiB chunk, and a size lie is refused with the one size lie message; huge zip64 declarations are refused before decoding; the tar family and bare files are untouched.

## Requirements covered
- SPEC-0085/FR-NEW-007 (partial: zip and 7z): IF an entry of a `.zip`, `.7z` or `.rar` archive decodes to more bytes than it declares THEN THE archive extraction SHALL stop decoding it, refuse with `ERR_INVALID_ARGUMENT` and the message `archive entry '<path>' decompressed to more bytes than declared <D>`, and write nothing.
- SPEC-0085/FR-NEW-008: THE archive extraction SHALL hold, during one extraction of a `.zip`, `.7z` or `.rar` archive, at most the declared total plus 65 536 bytes of decoded entry data.
- SPEC-0085/FR-NEW-010: THE archive extraction SHALL answer a `.zip`, `.7z` or `.rar` archive within the limits with the same destination, counts and audit line as before, SHALL charge the session write allowance the declared total on success as before, and SHALL extract or refuse plain `.tar`, compressed tar archives and bare `.gz`, `.bz2` and `.xz` files exactly as before this increment.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0085_017 e2e_0085_018 e2e_0085_019 e2e_0085_021 e2e_0085_023 e2e_0085_024 e2e_0085_037 e2e_0085_038 e2e_0085_040`: 9 tests pass. Real assertion: the hook set by the real decoder reads at most 65 546 for the 10 byte liar and exactly 10 for the honest 10 byte entry.

## Design excerpt
### Decisions
- **DEC-004 (SPEC-0085):** decoded bytes held: the decode step keeps a running total of decoded entry bytes and its maximum in a `&mut u64` it returns with its result (success or error); the async side sets `archive_single::LAST_HIGH_WATER` to it under `cfg(test)`; a call refused before decoding never sets it, so `held()` reads 0. **Obliged by:** SPEC-0085/FR-NEW-004, FR-NEW-008, NFR 7.1. **Rationale:** the one hook of SPEC-0084 DEC-006, set on the async thread (the blocking thread differs). **Alternatives considered:** an `Arc<AtomicU64>`. **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:56-61`.
- **DEC-005 (SPEC-0085):** every zip and 7z entry is read through `read_capped(reader, declared)`; `CapExceeded` maps to `declared_size_exceeded(rel_path, declared)`; zip's own `larger than its declared` error keeps its mapping (`zip_read_error`, `archive.rs:410-424`), now to the same function; RAR keeps UnRAR's own stop at the declared size and the `data.len() != declared` check (`archive_rar.rs:59-87`). **Obliged by:** SPEC-0085/FR-NEW-007, FR-NEW-008. **Rationale:** "a decoder that ignores a declared size does not lift the bound": the cap holds whatever the library does; at most the declared size plus one 65 536 byte chunk per entry, and the sum of declared sizes for the call. **Alternatives considered:** trusting the libraries' own bounds (zip `crc32.rs:43-50`, 7z `BoundedReader`): true today, not guaranteed. **Source:** design. **Evidence:** `zip-9.0.5/src/crc32.rs:43-50`, `sevenz-rust2-0.20.2/src/reader.rs:1660-1666`.
- **DEC-009 (SPEC-0085):** fixtures generated in the tests. Bombs: zip `build_zip(&[("zeros.bin", &vec![0; 10_000_000])], None)` (deflate, at most 20 000 bytes), 7z `build_sevenz_ext` (LZMA2), RAR `rar5_store(&[RarEntry::file("zeros.bin", &vec![0; 10_000_000])])` (store method, 10 MB, decoded size equal to its stored size); liars and declared sizes: a test helper `zip_declaring(entries: &[(&str, u64, &[u8])]) -> Vec<u8>` writes local headers, deflated data (`flate2::write::DeflateEncoder`), CRC32 (`flate2::Crc`) of the real content, a central directory and an end record; for a declared size of `0xFFFF_FFFF` or more both headers carry `0xFFFF_FFFF` and a zip64 extra field (id `0x0001`, uncompressed size then compressed size, 16 bytes) and version needed 45; a helper `sevenz_declaring(name, declared, data) -> Vec<u8>` assembles a 7z with one Copy coder folder (pack size = data length, unpack size = `declared`), no CRC digest, a plain (not encoded) header with the file name, and the start header CRC (`flate2::Crc`); RAR liars `RarEntry::declaring` (`archive_rar_fixtures.rs:87`). Remaining storage: `limit_mb_remaining(f, n)` sets a 1 048 576 byte limit and writes `/filler.bin` so exactly `n` bytes remain (the archive counts as stored); `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB (spec 9 preamble). **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-005, FR-NEW-007 (spec section 9: fixtures by the design), spec 14.3 (zip64 and 7z payloads). **Rationale:** no library writes a lying header; both builders are short and only test code. **Alternatives considered:** committed fixture files; patching a `zip` crate archive in place (the zip64 field does not exist in a small archive). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:1104-1160`, `:2207-2250`, `crates/core/src/tools/archive_rar_fixtures.rs:55-105`.
- **DEC-013 (SPEC-0085):** tests whose rule an earlier story already built are green on arrival and each is proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182. **Source:** design. **Evidence:** `git log` of those stories.
### Data model
None.
### Contracts
`archive.rs`: `pub(crate) fn declared_size_exceeded(rel_path: &str, declared: u64) -> ToolError` (`archive entry '<path>' decompressed to more bytes than declared <D>`); `zip_read_error` and RAR's `oversized` call it; zip and 7z entries are read with `archive_single::read_capped(reader, declared)`, `ReadError::CapExceeded` mapped to `declared_size_exceeded`, `ReadError::Io` to the existing corrupt or password mapping; `ensure_decoded_size_matches` stays for the tar family.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_testkit.rs` | modify |
| `crates/core/src/tools/archive_declared_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Remaining storage with exact messages: `limit_mb_remaining(f, n)` sets a limit of 1 048 576 bytes (`f.state.admin.set_quota(MOUNT, Some(1_048_576))`) and writes `/filler.bin` so that exactly `n` bytes remain (the archive counts as stored), hence `<U>` = 1 048 576 minus `n`; `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB. Archive builders: `build_zip` (`crates/core/src/tools/archive.rs:1104-1121`), `build_sevenz_ext` (`:2207-2250`), `rar5_store` and `RarEntry` (`crates/core/src/tools/archive_rar_fixtures.rs:55-105`). `zip_declaring` (US-0188) builds the liars: `zip_declaring(&[("big.bin", 10, &vec![7u8; 1_000_000])])`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0085/E2E-017: zip64 size against the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/big64.zip` whose only entry `huge.bin` declares 5 000 000 000 bytes in its zip64 record and holds the 5 bytes `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 5000000000 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/big64` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_017_zip64_size_against_the_storage_limit`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-018: zip64 size against the default allowance
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** the default session allowance and `/uploads/big64.zip` of E2E-017
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_018_zip64_size_against_the_default_allow`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-019: zip entry holding more than it declares
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/liar.zip` whose entry `big.bin` declares 10 bytes and holds 1 000 000 bytes of the byte 7
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'big.bin' decompressed to more bytes than declared 10`
- **And** the decoded bytes held never exceed 65 546; `/uploads/liar` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_019_zip_entry_holding_more_than_it_decla`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-021: zip entry declared 0 holding data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/zero.zip` whose entry `a.txt` declares 0 bytes and holds `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'a.txt' decompressed to more bytes than declared 0`; `/uploads/zero` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_021_zip_entry_declared_0_holding_data`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-023: Entry holding exactly its declared size
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/exact.zip` whose entry `exact.bin` declares 10 bytes and holds 10 bytes of the byte 7
- **When** the member extracts it
- **Then** the extraction succeeds with 10 bytes written; the decoded bytes held are 10
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_023_entry_holding_exactly_its_declared_s`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-024: The whole call is bounded by the declared total
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-NEW-008; **Priority:** High
- **Given** `/uploads/mixed.zip` holding `a.bin`, 600 000 bytes declared and held, then `b.bin`, declaring 10 bytes and holding 5 000 000 bytes of the byte 7
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'b.bin' decompressed to more bytes than declared 10`
- **And** the decoded bytes held never exceed 665 546; `/uploads/mixed` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_024_the_whole_call_is_bounded_by_the_dec`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-037: Plain tar over the storage limit unchanged
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** 4 bytes of remaining storage and `/uploads/pack.tar` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`; `/uploads/pack` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_037_plain_tar_over_the_storage_limit_unc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-038: Tar family and bare files unchanged
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/t1/pack.tar`, `/t2/pack.tar.gz` and `/t3/a.txt.gz`, the first two holding `a.txt` (`hello`), the third being `hello` compressed
- **When** the member extracts each
- **Then** `/t1/pack/a.txt`, `/t2/pack/a.txt` and `/t3/a.txt` read `hello`, each answer reporting 1 file and 5 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_038_tar_family_and_bare_files_unchanged`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-040: Solid 7z within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-008; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/solid.7z` holding `a.txt` (`hello`), `b.txt` (`world`) and `c.txt` (`again`) in one compressed block
- **When** the member extracts it
- **Then** the extraction succeeds: 3 files, 15 bytes written; the decoded bytes held never exceed 15; the remaining storage is 985 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_040_solid_7z_within_the_limits`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-017 | run `check_declared` after the decode: `huge.bin` (5 bytes) is decoded, `held()` is 5, not 0, the test fails |
| E2E-018 | the E2E-017 mutation: `held()` is 5, not 0, the test fails |
| E2E-019 | map `ReadError::CapExceeded` and zip's own size error to `corrupt("zip", ..)`: the message changes, the test fails |
| E2E-021 | the E2E-019 mutation: the message changes, the test fails |
| E2E-023 | cap each entry at `declared - 1` (saturating): the 10 byte entry is refused as a size lie, the test fails |
| E2E-024 | the E2E-019 mutation: the message for `b.bin` changes, the test fails |
| E2E-037 | in `quota::check_and_charge` skip the project check: the plain tar extracts against 4 bytes, the test fails |
| E2E-038 | make `decode_tar` return no entries for a plain `.tar`: `/t1/pack/a.txt` is missing, the test fails |
| E2E-040 | return `Ok(false)` from the `for_each_entries` closure after the first file: 1 file is written instead of 3, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: no change to the RAR decode bound (UnRAR already stops at the declared size, DEC-005)

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-017, E2E-018, E2E-019, E2E-021, E2E-023, E2E-024, E2E-037, E2E-038, E2E-040): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive tools::quota_e2e` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted`, `spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing`, `spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted`, `spec_0020_e2e_051_an_extraction_racing_a_write` (`quota_e2e.rs`), `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs`), every `e2e_0084_` test; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0192: cap every entry at its declared size`, body `Spec: SPEC-0085` and the requirement ids (SPEC-0085/FR-NEW-007, FR-NEW-008, FR-NEW-010).
