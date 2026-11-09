# US-0201: decrypt RAR5 headers to see a cut with the right password

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0200
> min_tier: 1
> Files touched: 8
> Closes spec: no

## Objective
A RAR version 5 archive with encrypted file names, opened with the right password, is refused as corrupt when it is cut, even between two files, before its end block or before a link entry, where today it extracts the files before the cut. Our code decrypts the headers (AES 256 CBC, PBKDF2 HMAC SHA-256) with RustCrypto crates already in `Cargo.lock`, and answers a wrong password from the archive's check value. Tier 1: a key derivation and a binary header format.

## Requirements covered
- SPEC-0087/FR-NEW-001 (partial: RAR5 headers encrypted, read with the right password): THE archive extraction SHALL treat a RAR archive as complete when, read from its RAR marker, every block after the marker is wholly present, header and the data it declares, up to and including the end of archive block.
- SPEC-0087/FR-NEW-005 (partial: the check value mismatch answers the wrong password): IF a version 5 RAR archive declares encrypted file names in its whole leading part AND the given password is wrong or empty THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`, whether the rest of the archive is complete or not.
- SPEC-0087/FR-NEW-007 (partial: a cut before a link entry): IF a truncated RAR archive meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password (FR-NEW-004), a wrong one (FR-NEW-005) or, for version 4, an empty one (FR-NEW-009); (5) the truncation (FR-MOD-001); (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.
- SPEC-0087/FR-MOD-001 (partial: RAR5 with encrypted file names and the right password): IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_004 e2e_0087_028 e2e_0087_029 e2e_0087_030 e2e_0087_037 rar5_check_value_of_hidden_rar_matches e2e_0080_058`: all pass. Real assertion: `rar5_kdf` over the salt and count read from `hidden.rar` reproduces the check value stored in that archive (made by RAR 7.23), and the decrypted first header's CRC32 matches.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0087):** encrypted headers. The leading part (RAR4: marker and main header; RAR5: marker and encryption header) must be whole, else `TRUNCATED`. With `password` `None` or `Some("")` the walk returns `Ok` after the leading part and UnRAR answers (password needed, or incorrect for the empty password, `archive_rar.rs:162-169`, `:69-81`). With a non empty password: RAR5 derives the keys once per call and, when the check value flag is set and its checksum is valid, a mismatch returns `password_required_message(password)`; each header is decrypted (RAR5: 16 byte IV then the ciphertext; RAR4: 8 byte salt then the ciphertext, keys cached per salt within the call), its CRC checked, and a CRC mismatch returns `Ok` (UnRAR then gives today's answer: corrupt for RAR4, BL-0136; incorrect for RAR5); a shortfall anywhere returns `TRUNCATED`. **Obliged by:** SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007 item 4, FR-NEW-009, FR-MOD-001. **Rationale:** the password questions keep their place before the truncation; answering the wrong password ourselves only where RAR5 offers a check value matches what UnRAR says today (BadPassword from 62 bytes, P-3); deferring on a CRC mismatch keeps every wrong password answer byte for byte. **Alternatives considered:** deferring every password question to UnRAR after the walk (the walk cannot run without the key, so a cut archive with a wrong password would read as TRUNCATED, against FR-NEW-005). **Source:** design. **Evidence:** spec probes P-2 and P-3; `unrar_sys-0.5.8/vendor/unrar/crypt5.cpp`, `crypt3.cpp`.
- **DEC-004 (SPEC-0087):** RAR5 crypto: `hmac` 0.13 with `sha2` 0.11 (renamed `sha2_v011`) for PBKDF2 HMAC SHA-256, one loop producing the key (count), the hash key value (count + 16, unused) and the check value (count + 32), as UnRAR's `pbkdf2` does; `lg2_count` above 24 (UnRAR's `CRYPT5_KDF_LG2_COUNT_MAX`) is corrupt; `aes` 0.8 and `cbc` 0.1 (`cbc::Decryptor<aes::Aes256>`, `decrypt_padded_mut::<NoPadding>`) for AES 256 CBC. All four are optional dependencies enabled by the `rar` feature, already in `Cargo.lock` (section 1). **Obliged by:** SPEC-0087/FR-MOD-001 (encrypted file names with the right password), FR-NEW-005. **Rationale:** the crates are the RustCrypto ones the tree already compiles (zip, 7z, S3 signing); a hand written HMAC or AES is avoided. `pbkdf2` 0.13 is not used because it would run the derivation three times for the three values. **Alternatives considered:** `pbkdf2` 0.13; `sha2` 0.10 with a hand written HMAC. **Source:** design. **Evidence:** `Cargo.lock:31-32`, `:1276-1277`, `:3073-3074`, `:5281-5282`; `cargo tree --all-features -i hmac@0.13.0` (zip, aws-sdk-s3, sqlx-postgres).
- **DEC-006 (SPEC-0087):** fixtures: `two4.rar`, `big4.rar`, `dir4.rar`, `solid4.rar`, `extras4.rar`, `hp4.rar` (RAR 6.24) and `hp5two.rar` (RAR 7.23) are the archives the spec measured, copied from `/tmp/bl131/` (sizes 123, 3 116, 152, 133, 745, 220, 382 bytes, checked on 2026-11-09 with `wc -c`) into `crates/core/tests/fixtures/rar/` with a README row each (commands of spec 14.3); `hp4long.rar` is made once with `rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt` (RAR 6.24) for DEC-005's write back path. When `/tmp/bl131` is gone, remake each archive with its README command and re measure every offset the story's tests use (spec 14.3: RAR stamps times, sizes stay). **Obliged by:** SPEC-0087/FR-NEW-001 (spec 3.3, 9.0 archive table). **Rationale:** the offsets of spec 9.0 are those of these exact files. **Alternatives considered:** committing every cut (one archive covers every cut at run time). **Source:** spec 14.3, existing fixtures README. **Evidence:** `ls /tmp/bl131`, `crates/core/tests/fixtures/rar/README.md`.
- **DEC-009 (SPEC-0087):** internal tests: `non_rar5_input_is_left_to_the_library` becomes `partial_markers_are_left_to_the_library` (`b""` and `b"Rar!\x1a\x07\x01"` still `Ok`) plus `rar4_marker_alone_is_truncated` (`b"Rar!\x1a\x07\x00"` refused, message `archive is corrupt or not a valid rar file: unexpected end of archive`); `every_strict_prefix_of_a_rar5_archive_is_corrupt`, `huge_data_size_is_corrupt_not_a_panic` and the end block test at `:770` only switch to `ensure_rar_complete(.., None)`. Design tests (not spec tests): `rar5_check_value_of_hidden_rar_matches` and `rar3_key_decrypts_the_first_header_of_hp4` (header CRC matches after decryption), `rar3_key_with_a_long_password_decrypts_hp4long`. **Obliged by:** SPEC-0087/FR-MOD-001, FR-NEW-001 (spec 14.3 "internal tests that pin the old limitation"). **Rationale:** the old test pins the limitation this spec removes; the design tests pin each key derivation on a real archive before the acceptance tests depend on it. **Alternatives considered:** published test vectors (none exist for the RAR 3 derivation). **Source:** spec 14.3. **Evidence:** `crates/core/src/tools/archive_rar.rs:740-800`.
- **DEC-011 (SPEC-0087):** no `unsafe`, no `unrar_sys` call. **Obliged by:** SPEC-0087/FR-MOD-001 (spec 14.3 constraint, SPEC-0080 DEC-006). **Rationale:** `unsafe_code` is forbidden workspace wide. **Alternatives considered:** section 2. **Source:** constitution of the workspace lints. **Evidence:** spec 14.3.
### Data model
None.
### Contracts
Manifests: `Cargo.toml` `[workspace.dependencies]` gains `aes = "0.8"`, `cbc = "0.1"`, `hmac = "0.13"`, `sha2_v011 = { package = "sha2", version = "0.11" }` (all in `Cargo.lock` today: 0.8.4, 0.1.2, 0.13.0, 0.11.0); `crates/core/Cargo.toml` gains them with `optional = true` and `rar = ["dep:unrar", "dep:aes", "dep:cbc", "dep:hmac", "dep:sha2_v011"]`; `Cargo.lock` changes only in `mcp-fs-core`'s dependency list (check with `git diff Cargo.lock`: no `[[package]]` added). New module `crates/core/src/tools/archive_rar_crypt.rs` (`#[cfg(feature = "rar")] pub(crate) mod archive_rar_crypt;` in `tools/mod.rs`): `pub(crate) fn rar5_kdf(password: &str, salt: &[u8; 16], lg2_count: u8) -> ([u8; 32], [u8; 8])` (PBKDF2 HMAC SHA-256 over the UTF-8 password, one 32 byte block: `U1 = HMAC(pw, salt || 00 00 00 01)`, `F ^= U` for `2^lg2_count - 1` more iterations gives the key, 16 more the hash key value, 16 more the check value; `check[i % 8] ^= value[i]` for `i` in 0..32), `pub(crate) fn aes256_cbc_decrypt(key: &[u8; 32], iv: &[u8; 16], buf: &mut [u8])` (`cbc::Decryptor::<aes::Aes256>::new(key.into(), iv.into()).decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(buf)`, `buf.len()` a multiple of 16). `archive_rar.rs` RAR5 walk after the encryption header (type 4: `version` vint 0, `flags` vint, `lg2_count` u8 at most 24 else corrupt, `salt` 16 bytes, when `flags & 0x1` `check` 8 bytes and `sum` 4 bytes = first 4 bytes of SHA-256 of `check`): with a non empty password derive once; if the check value is present, its sum valid and the derived check differs, return `Err(password_required_message(password))`; then each header is a 16 byte IV plus ciphertext: decrypt the first 16 bytes to read CRC32 and the `HeadSize` vint, then the rest up to `ceil16(4 + vint length + HeadSize)`; a CRC32 mismatch (over the bytes after the CRC field) returns `Ok(())`; then skip the data area (`HFL_DATA`); type 5 returns `Ok(())`; any shortfall `TRUNCATED`. New fixture `hp5two.rar` (382 bytes, `rar a -ma5 -ep -hps3cret-Ünï hp5two.rar a.txt b.txt`, RAR 7.23), README row.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify |
| `crates/core/Cargo.toml` | modify |
| `crates/core/src/tools/archive_rar_crypt.rs` | create |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify |
| `crates/core/tests/fixtures/rar/hp5two.rar` | create |
| `crates/core/tests/fixtures/rar/README.md` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Fixtures load with `include_bytes!("../../tests/fixtures/rar/<name>.rar")` as the SPEC-0080 tests do (`crates/core/src/tools/archive_rar.rs` tests); a cut is a slice taken at run time (`&TWO4[..67]`), never a committed file. Password of every encrypted fixture: `s3cret-Ünï`. Today's walk to extend: `ensure_rar5_complete` and `read_vint` (`crates/core/src/tools/archive_rar.rs:98-160`); the password mapping it must not change: `listing_error` (`:162-169`) and the entry read mapping (`:62-81`). The every length loop model: `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`). The log and audit leak check model: `assert_never_recorded` (`:922-938`). Offsets of spec section 9.0 are measured on the archives of `/tmp/bl131`; an archive remade from its README command needs its offsets re measured. The RAR5 encryption header of `hp5two.rar` starts at offset 8: `a1 33 f2 5a` (CRC32), `21` (size 33), `04` (type), `00` (flags), `00` (version), `01` (check value present), `0f` (lg2_count 15), then the 16 byte salt, the 8 byte check value and its 4 byte sum. UnRAR's reference: `unrar_sys-0.5.8/vendor/unrar/crypt5.cpp` (`pbkdf2`, `SetKey50`), `arcread.cpp` (`ReadHeader50`). RustCrypto usage already in the tree: `aes-gcm` in `git/oauth/` (cipher), `cbc` through `sevenz-rust2`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-004: Version 5 with encrypted file names, two files, right password
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/hp5two.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the answer is 2 files, 12 bytes, with the content of E2E-003 under `/uploads/hp5two`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_004_version_5_with_encrypted_file_names`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-028: Version 5, encrypted names, end block missing, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hid222.rar`, the first 222 bytes of `hidden.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written (today `/uploads/hid222/note.txt` is written and the call succeeds)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_028_version_5_encrypted_names_end_block`
- Red before the change because: today the walk stops at the encryption header (`archive_rar.rs:129`) and the cut before the end block extracts `note.txt` (P-2)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-029: Version 5, encrypted names, cut inside data, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not the wrong password message; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_029_version_5_encrypted_names_cut_inside`
- Red before the change because: today the cut inside data with the right password reads as `incorrect password for this archive; ...` (P-2: read `BadData`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-030: Version 5, encrypted names, cut between files, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001; **Priority:** High
- **Given** `/uploads/hp5half.rar`, the first 206 bytes of `hp5two.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_030_version_5_encrypted_names_cut_betwee`
- Red before the change because: today the cut between the two files extracts `a.txt` and succeeds (P-2)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-037: A cut wins over a link entry
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/hl2cut.rar`, the first 334 bytes of `hl2.rar` (the link `l` is listed, the end block is missing)
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not `ERR_NOT_SUPPORTED` naming `l`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_037_a_cut_wins_over_a_link_entry`
- Red before the change because: today the listing reaches the link `l` and refuses it with `ERR_NOT_SUPPORTED` (P-2: 334 bytes `Ok [a.txt, l]`)
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-004 | make the decrypted walk return `TRUNCATED` after the first decrypted header: the complete `hp5two.rar` is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: anything outside `aes`, `cbc`, `hmac`, `sha2` 0.11, which are already in `Cargo.lock`; `cargo deny check` must stay green
- No dash as punctuation in code, comments or docs
- Fixture provenance (DEC-006): copy the archive from `/tmp/bl131/<name>.rar` (the file the spec measured; check its size with `wc -c` against the spec 9.0 table); when it is gone, remake it with the README command using the RARLAB trial build (`/tmp/rarbin/v6/rar/rar` is RAR 6.24, `/tmp/rarbin/rar/rar` is RAR 7.23, else download them from rarlab.com) and re measure every offset this story's tests use
- No `unsafe`, no `unrar_sys`, no hand written AES, SHA-256 or HMAC (DEC-004, DEC-011)
- The password is never logged, traced or put in an error text (the walk runs inside `spawn_blocking`, where the per thread log capture cannot see it: keep it out by construction)
- Scope boundary: RAR4 decryption is US-0202; the password order tests are US-0203

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-028, E2E-029, E2E-030, E2E-037), before the production code.
- [ ] Green on arrival tests (E2E-004): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract; `e2e_0080_058_truncated_archive_with_encrypted_fil` (`cuth.rar` with `nope`) now answers through the check value and still gets the wrong password message
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0201: decrypt RAR5 headers to see a cut with the right password`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-001, FR-NEW-005, FR-NEW-007, FR-MOD-001).
