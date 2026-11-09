# US-0198: refuse a cut RAR4 archive with one message

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0197
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
A RAR version 4 archive cut anywhere after its 7 byte marker, missing only its end of archive block, or reduced to its marker or main header, is refused with `archive is corrupt or not a valid rar file: unexpected end of archive`, where today it extracts the files before the cut or reports a misleading message. The walk finds the marker where UnRAR finds it, so an archive preceded by other bytes is checked too.

## Requirements covered
- SPEC-0087/FR-NEW-001 (partial: RAR4 clear blocks and the marker search; encrypted headers come in US-0201 and US-0202): THE archive extraction SHALL treat a RAR archive as complete when, read from its RAR marker, every block after the marker is wholly present, header and the data it declares, up to and including the end of archive block.
- SPEC-0087/FR-NEW-002: IF a RAR archive ends after a whole block that is not the end of archive block THEN THE archive extraction SHALL refuse it as FR-MOD-001 states, even when every file it lists is whole.
- SPEC-0087/FR-NEW-003: THE archive extraction SHALL word the refusal of an archive that holds a whole RAR marker and is not complete exactly `archive is corrupt or not a valid rar file: unexpected end of archive`, with the code `ERR_INVALID_ARGUMENT`.
- SPEC-0087/FR-NEW-007 (partial: a cut before the encrypted data password question): IF a truncated RAR archive meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password (FR-NEW-004), a wrong one (FR-NEW-005) or, for version 4, an empty one (FR-NEW-009); (5) the truncation (FR-MOD-001); (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.
- SPEC-0087/FR-MOD-001 (partial: RAR4 plain and encrypted data, both versions with bytes before the marker): IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_008 e2e_0087_009 e2e_0087_011 e2e_0087_012 e2e_0087_013 e2e_0087_014 e2e_0087_015 e2e_0087_019 e2e_0087_020 e2e_0087_021 e2e_0087_036 e2e_0087_047 rar4_marker_alone_is_truncated partial_markers_are_left_to_the_library`: all pass; `./test.sh` runs them. Real assertion: `/uploads/half4` absent from the volume and `quota::used_bytes` unchanged after each refusal.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0087):** `ensure_rar_complete(bytes, password)` replaces `ensure_rar5_complete` at the start of the RAR listing: it finds the marker as UnRAR does (the first 7 bytes, else the first `0x52` byte from offset 7 within the next `0x200000 - 16` bytes whose bytes form a signature; RAR 1.4 signature or no signature or a RAR5 signature without its 8th byte: return `Ok` and let UnRAR refuse), then walks the version's blocks; any block whose header or declared data runs past the end, or an archive ending after a whole block that is not the end of archive block, is refused with `corrupt("rar", TRUNCATED)`; bytes after the end block are ignored. **Obliged by:** SPEC-0087/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-MOD-001. **Rationale:** the safe API cannot tell an end block from an end of file (`unrar-0.5.8/src/open_archive.rs:452`, `dll.cpp:222-231`) and carries no packed size (`open_archive.rs:582-590`); mirroring UnRAR's marker search keeps the walk and the reader on the same archive (E2E-047). **Alternatives considered:** section 2. **Source:** spec 14.3 and existing code. **Evidence:** `crates/core/src/tools/archive_rar.rs:98-160`, `unrar_sys-0.5.8/vendor/unrar/archive.cpp:98-175`.
- **DEC-002 (SPEC-0087):** the RAR4 walk reads the 7 byte base header of each block (CRC16, type, flags, size, size at least 7), the data size from `ADD_SIZE` when flag `0x8000` is set (plus the high 32 bits at header offset 32 for types `0x74` and `0x7A` when flag `0x0100` is set), stops at type `0x7B`, and treats every other type generically (comment, recovery record, service, sub blocks); a RAR4 header whose size is below 7, or a RAR5 vint that overflows, is refused as `TRUNCATED` or with the existing vint messages. Header CRCs of clear headers are not checked (UnRAR checks them); CRCs of decrypted headers are (DEC-003), with `flate2::Crc` (CRC16 = low 16 bits of the CRC32 of the header after its CRC field; RAR5 CRC32 of the header after its CRC field). **Obliged by:** SPEC-0087/FR-NEW-001 (every kind of block), FR-NEW-002. **Rationale:** the generic `LONG_BLOCK` rule covers the recovery record and comment of `extras4.rar` (P-1: a cut inside the recovery record is accepted today) without a table of block types. **Alternatives considered:** a per type table (misses unknown service blocks). **Source:** spec 14.3 "RAR4 block walk". **Evidence:** `unrar_sys-0.5.8/vendor/unrar/arcread.cpp`, spec probes P-1 and P-5.
- **DEC-003 (SPEC-0087):** encrypted headers. The leading part (RAR4: marker and main header; RAR5: marker and encryption header) must be whole, else `TRUNCATED`. With `password` `None` or `Some("")` the walk returns `Ok` after the leading part and UnRAR answers (password needed, or incorrect for the empty password, `archive_rar.rs:162-169`, `:69-81`). With a non empty password: RAR5 derives the keys once per call and, when the check value flag is set and its checksum is valid, a mismatch returns `password_required_message(password)`; each header is decrypted (RAR5: 16 byte IV then the ciphertext; RAR4: 8 byte salt then the ciphertext, keys cached per salt within the call), its CRC checked, and a CRC mismatch returns `Ok` (UnRAR then gives today's answer: corrupt for RAR4, BL-0136; incorrect for RAR5); a shortfall anywhere returns `TRUNCATED`. **Obliged by:** SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007 item 4, FR-NEW-009, FR-MOD-001. **Rationale:** the password questions keep their place before the truncation; answering the wrong password ourselves only where RAR5 offers a check value matches what UnRAR says today (BadPassword from 62 bytes, P-3); deferring on a CRC mismatch keeps every wrong password answer byte for byte. **Alternatives considered:** deferring every password question to UnRAR after the walk (the walk cannot run without the key, so a cut archive with a wrong password would read as TRUNCATED, against FR-NEW-005). **Source:** design. **Evidence:** spec probes P-2 and P-3; `unrar_sys-0.5.8/vendor/unrar/crypt5.cpp`, `crypt3.cpp`.
- **DEC-008 (SPEC-0087):** tests live in `crates/core/src/tools/archive_rar_cut_e2e.rs` (`#[cfg(test)] mod archive_rar_cut_e2e;`), each `#[cfg(feature = "rar")]` except E2E-043 (`#[cfg(not(feature = "rar"))]`, run by the default `cargo test --workspace`); the module path contains `archive_rar` for `test.sh:7`; the HTTP tests sit in a nested `#[cfg(feature = "rar")] mod archive_rar` of `api/dataplane.rs` tests. Helpers from `archive_testkit.rs` plus a module local `assert_never_recorded` twin (`archive_rar.rs:922-938`). **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0085 DEC-011. **Source:** existing code. **Evidence:** `test.sh:7`, `crates/core/src/tools/archive.rs:905-933`.
- **DEC-009 (SPEC-0087):** internal tests: `non_rar5_input_is_left_to_the_library` becomes `partial_markers_are_left_to_the_library` (`b""` and `b"Rar!\x1a\x07\x01"` still `Ok`) plus `rar4_marker_alone_is_truncated` (`b"Rar!\x1a\x07\x00"` refused, message `archive is corrupt or not a valid rar file: unexpected end of archive`); `every_strict_prefix_of_a_rar5_archive_is_corrupt`, `huge_data_size_is_corrupt_not_a_panic` and the end block test at `:770` only switch to `ensure_rar_complete(.., None)`. Design tests (not spec tests): `rar5_check_value_of_hidden_rar_matches` and `rar3_key_decrypts_the_first_header_of_hp4` (header CRC matches after decryption), `rar3_key_with_a_long_password_decrypts_hp4long`. **Obliged by:** SPEC-0087/FR-MOD-001, FR-NEW-001 (spec 14.3 "internal tests that pin the old limitation"). **Rationale:** the old test pins the limitation this spec removes; the design tests pin each key derivation on a real archive before the acceptance tests depend on it. **Alternatives considered:** published test vectors (none exist for the RAR 3 derivation). **Source:** spec 14.3. **Evidence:** `crates/core/src/tools/archive_rar.rs:740-800`.
### Data model
None.
### Contracts
`archive_rar.rs`: `fn ensure_rar_complete(bytes: &[u8], password: Option<&str>) -> Result<()>` replaces `ensure_rar5_complete` at its call site (start of `list_rar`, SPEC-0085 DEC-007). Marker search: the first 7 bytes, else the first `0x52` byte from offset 7 within the next `0x200000 - 16` bytes whose following bytes are `Rar!\x1a\x07` then `0x00` (RAR4) or `0x01` plus an 8th byte `0x00` (RAR5); no marker, the RAR 1.4 signature `RE~^`, or a RAR5 signature without its 8th byte: `Ok(())`. RAR4 walk (DEC-002): base header CRC16 u16, type u8, flags u16, size u16 (below 7: `TRUNCATED`); `ADD_SIZE` u32 at offset 7 when `flags & 0x8000`, high part u32 at offset 32 when `flags & 0x0100` on types `0x74` and `0x7A`; end type `0x7B` returns `Ok`; main header `0x73` with `flags & 0x0080` (encrypted headers, DEC-003 for this story): the main header must be whole; then with `password` `None` or `Some("")` return `Ok` (UnRAR answers the password question); with a non empty password and no byte after the main header return `TRUNCATED`; otherwise return `Ok` (the decrypted walk comes in US-0202). RAR5 walk as today, plus the same rule after the encryption header (type 4, whole or `TRUNCATED`): no password or empty: `Ok`; non empty and nothing after it: `TRUNCATED`; otherwise `Ok` (decrypted walk in US-0201). Message: `corrupt("rar", TRUNCATED)`. Internal tests (DEC-009): `non_rar5_input_is_left_to_the_library` is replaced by `partial_markers_are_left_to_the_library` (`ensure_rar_complete(b"", None)` and `ensure_rar_complete(b"Rar!\x1a\x07\x01", None)` are `Ok`) and `rar4_marker_alone_is_truncated` (`ensure_rar_complete(b"Rar!\x1a\x07\x00", None)` fails with code `ERR_INVALID_ARGUMENT` and message `archive is corrupt or not a valid rar file: unexpected end of archive`); `every_strict_prefix_of_a_rar5_archive_is_corrupt`, `huge_data_size_is_corrupt_not_a_panic` and the end block test at `:770` only switch to `ensure_rar_complete(.., None)`. New fixture `two4.rar` (123 bytes) and its README row (`rar a -ma4 -ep two4.rar a.txt b.txt`, RAR 6.24). New test module `crates/core/src/tools/archive_rar_cut_e2e.rs` (`#[cfg(test)] mod archive_rar_cut_e2e;`), RAR tests `#[cfg(feature = "rar")]`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | create |
| `crates/core/tests/fixtures/rar/two4.rar` | create |
| `crates/core/tests/fixtures/rar/README.md` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Fixtures load with `include_bytes!("../../tests/fixtures/rar/<name>.rar")` as the SPEC-0080 tests do (`crates/core/src/tools/archive_rar.rs` tests); a cut is a slice taken at run time (`&TWO4[..67]`), never a committed file. Password of every encrypted fixture: `s3cret-Ünï`. Today's walk to extend: `ensure_rar5_complete` and `read_vint` (`crates/core/src/tools/archive_rar.rs:98-160`); the password mapping it must not change: `listing_error` (`:162-169`) and the entry read mapping (`:62-81`). The every length loop model: `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`). The log and audit leak check model: `assert_never_recorded` (`:922-938`). Offsets of spec section 9.0 are measured on the archives of `/tmp/bl131`; an archive remade from its README command needs its offsets re measured.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-008: Version 4 cut between two files
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/half4.rar`, the first 67 bytes of `two4.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `archive is corrupt or not a valid rar file: unexpected end of archive`; nothing written: `/uploads/half4` does not exist (today `/uploads/half4/a.txt` is written and the call succeeds)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_008_version_4_cut_between_two_files`
- Red before the change because: today the cut at 67 bytes lists and extracts `a.txt` and the call succeeds (spec probe P-1)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-009: Version 4 missing only its end of archive block
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/noend4.rar`, the first 67 bytes of `v4.rar` (every file whole)
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_009_version_4_missing_only_its_end_of_ar`
- Red before the change because: today the first 67 bytes of `v4.rar` extract `a.txt` and succeed
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-011: Version 4 cut inside a header
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** High
- **Given** `/uploads/hdr4.rar`, the first 90 bytes of `two4.rar`
- **When** the member extracts it
- **Then** the refusal is exactly that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_011_version_4_cut_inside_a_header`
- Red before the change because: today UnRAR reports the cut header as damaged data with its own detail, not `unexpected end of archive`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-012: Version 4 marker and main header only
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** High
- **Given** `/uploads/main4.rar`, the first 20 bytes of `v4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; `/uploads/main4` does not exist (today an empty folder is created and the call succeeds)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_012_version_4_marker_and_main_header_onl`
- Red before the change because: today the marker and main header extract nothing and succeed with an empty folder
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-013: Version 4 marker alone
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-MOD-001; **Priority:** High
- **Given** `/uploads/mark4.rar`, the first 7 bytes of `v4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; `/uploads/mark4` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_013_version_4_marker_alone`
- Red before the change because: today the 7 byte marker alone succeeds with an empty folder (P-1: 7 bytes `Ok []`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-014: Cut inside the marker
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/m6.rar`, the first 6 bytes of `v4.rar`, and `/uploads/m7.rar`, the first 7 bytes of `v5.rar`
- **When** the member extracts each
- **Then** each refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_014_cut_inside_the_marker`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-015: One byte short
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** `/uploads/short4.rar`, the first 73 bytes of `v4.rar`
- **When** the member extracts it
- **Then** the refusal is exactly that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_015_one_byte_short`
- Red before the change because: today 73 bytes of `v4.rar` give UnRAR's damaged data detail, not `unexpected end of archive` (P-1: 68 to 73 `BadData`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-019: Version 4, encrypted data, cut, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-007; **Priority:** Critical
- **Given** `/uploads/v4cut.rar`, the first 78 bytes of `v4secret.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not `incorrect password for this archive; call again with the correct 'password' parameter`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_019_version_4_encrypted_data_cut_right_p`
- Red before the change because: today the cut inside encrypted data with the right password reads as `incorrect password for this archive; ...` (P-1)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-020: Version 4, encrypted data, cut, no password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-MOD-001; **Priority:** High
- **Given** `/uploads/v4cut.rar` of E2E-019
- **When** the member extracts it with no password
- **Then** the refusal is that of E2E-008, not `ERR_PASSWORD_REQUIRED`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_020_version_4_encrypted_data_cut_no_pass`
- Red before the change because: today with no password the answer is `password required to extract this archive; ...` (P-1)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-021: Version 4, encrypted data, cut, wrong password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-MOD-001; **Priority:** High
- **Given** `/uploads/v4cut.rar` of E2E-019
- **When** the member extracts it with `nope`
- **Then** the refusal is that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_021_version_4_encrypted_data_cut_wrong_p`
- Red before the change because: today with `nope` the answer is the wrong password message
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-036: Version 5 missing only its end of archive block (unchanged)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/noend5.rar`, the first 68 bytes of `v5.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_036_version_5_missing_only_its_end_of_ar`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-047: Bytes before the marker, cut archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/sfxcut4.rar` = 100 zero bytes followed by the first 67 bytes of `two4.rar`, and `/uploads/sfxcut5.rar` = 100 zero bytes followed by the first 68 bytes of `v5.rar`
- **When** the member extracts each
- **Then** each refusal is that of E2E-008; nothing written (today both succeed)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_047_bytes_before_the_marker_cut_archive`
- Red before the change because: today both prefixed archives succeed: the RAR5 walk uses `strip_prefix` at offset 0 and never runs on them, and RAR4 has no walk (P-4)
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-014 | map every code of `listing_error` to `password_required_message(password)`: the 6 and 7 byte files answer `ERR_PASSWORD_REQUIRED`, the test fails |
| E2E-036 | accept an archive that ends right after a whole RAR5 block (return `Ok` at the end of input instead of `TRUNCATED`): `noend5.rar` extracts, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Fixture provenance (DEC-006): copy the archive from `/tmp/bl131/<name>.rar` (the file the spec measured; check its size with `wc -c` against the spec 9.0 table); when it is gone, remake it with the README command using the RARLAB trial build (`/tmp/rarbin/v6/rar/rar` is RAR 6.24, `/tmp/rarbin/rar/rar` is RAR 7.23, else download them from rarlab.com) and re measure every offset this story's tests use
- No `unsafe`, no `unrar_sys` (DEC-011)
- Scope boundary: no decryption here (US-0201, US-0202); the password questions keep UnRAR's answers

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-008, E2E-009, E2E-011, E2E-012, E2E-013, E2E-015, E2E-019, E2E-020, E2E-021, E2E-047), before the production code.
- [ ] Green on arrival tests (E2E-014, E2E-036): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0198: refuse a cut RAR4 archive with one message`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-007, FR-MOD-001).
