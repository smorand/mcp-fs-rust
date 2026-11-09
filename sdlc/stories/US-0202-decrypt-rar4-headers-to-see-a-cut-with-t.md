# US-0202: decrypt RAR4 headers to see a cut with the right password

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0201
> min_tier: 1
> Files touched: 7
> Closes spec: no

## Objective
A RAR version 4 archive with encrypted file names, opened with the right password, is refused as corrupt when cut between files, inside data or right after its main header; our code derives the RAR 3 key from SHA-1 exactly as RAR does, including its quirk for long passwords, and decrypts the headers with AES 128 CBC. Tier 1: a nonstandard key derivation pinned on real archives.

## Requirements covered
- SPEC-0087/FR-NEW-001 (partial: RAR4 headers encrypted, read with the right password): THE archive extraction SHALL treat a RAR archive as complete when, read from its RAR marker, every block after the marker is wholly present, header and the data it declares, up to and including the end of archive block.
- SPEC-0087/FR-NEW-006 (partial: the password stays out of a cut refusal): THE archive extraction SHALL NOT write a file, create a folder, charge the remaining storage or the session write allowance, record an audit line, or show the password in a message, a log line or an audit line, for a call refused under FR-MOD-001.
- SPEC-0087/FR-MOD-001 (partial: RAR4 with encrypted file names and the right password): IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_003 e2e_0087_022 e2e_0087_023 e2e_0087_024 e2e_0087_048 rar3_key_decrypts_the_first_header_of_hp4 rar3_key_with_a_long_password_decrypts_hp4long`: all pass. Real assertion: the first encrypted header of `hp4.rar` (RAR 6.24) decrypted with `rar3_kdf("s3cret-Ünï", salt)` has a matching CRC16, and the same holds for `hp4long.rar` with its 40 character password.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0087):** encrypted headers. The leading part (RAR4: marker and main header; RAR5: marker and encryption header) must be whole, else `TRUNCATED`. With `password` `None` or `Some("")` the walk returns `Ok` after the leading part and UnRAR answers (password needed, or incorrect for the empty password, `archive_rar.rs:162-169`, `:69-81`). With a non empty password: RAR5 derives the keys once per call and, when the check value flag is set and its checksum is valid, a mismatch returns `password_required_message(password)`; each header is decrypted (RAR5: 16 byte IV then the ciphertext; RAR4: 8 byte salt then the ciphertext, keys cached per salt within the call), its CRC checked, and a CRC mismatch returns `Ok` (UnRAR then gives today's answer: corrupt for RAR4, BL-0136; incorrect for RAR5); a shortfall anywhere returns `TRUNCATED`. **Obliged by:** SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007 item 4, FR-NEW-009, FR-MOD-001. **Rationale:** the password questions keep their place before the truncation; answering the wrong password ourselves only where RAR5 offers a check value matches what UnRAR says today (BadPassword from 62 bytes, P-3); deferring on a CRC mismatch keeps every wrong password answer byte for byte. **Alternatives considered:** deferring every password question to UnRAR after the walk (the walk cannot run without the key, so a cut archive with a wrong password would read as TRUNCATED, against FR-NEW-005). **Source:** design. **Evidence:** spec probes P-2 and P-3; `unrar_sys-0.5.8/vendor/unrar/crypt5.cpp`, `crypt3.cpp`.
- **DEC-005 (SPEC-0087):** RAR4 crypto: the RAR 3 key derivation of `crypt3.cpp` (`SetKey30`) with `sha1` 0.10's `compress` (feature `compress` enabled on the existing direct dependency; no new package): a small SHA-1 context (state, byte count, 64 byte buffer) reproduces `sha1_process_rar29`, which writes the 16 expanded schedule words back into the password buffer after each block processed straight from it (`sha1.cpp:146-168`; only passwords whose UTF-16 form plus the salt spans a whole block reach that path, so `s3cret-Ünï` does not, and `hp4long.rar` pins it); `aes` 0.8 and `cbc` 0.1 (`cbc::Decryptor<aes::Aes128>`) for AES 128 CBC. **Obliged by:** SPEC-0087/FR-MOD-001 (version 4 with encrypted file names and the right password, any password). **Rationale:** without the write back, a long password derives a different key than RAR and every cut would go undetected for it. **Alternatives considered:** the plain `Sha1` digest (wrong keys for long passwords); handing long passwords to UnRAR (spec FR-MOD-001 has no length exception). **Source:** UnRAR source. **Evidence:** `unrar_sys-0.5.8/vendor/unrar/sha1.cpp:146-168`, `crypt3.cpp` (`SetKey30`), `sha1-0.10.7/src/lib.rs:80-81`.
- **DEC-006 (SPEC-0087):** fixtures: `two4.rar`, `big4.rar`, `dir4.rar`, `solid4.rar`, `extras4.rar`, `hp4.rar` (RAR 6.24) and `hp5two.rar` (RAR 7.23) are the archives the spec measured, copied from `/tmp/bl131/` (sizes 123, 3 116, 152, 133, 745, 220, 382 bytes, checked on 2026-11-09 with `wc -c`) into `crates/core/tests/fixtures/rar/` with a README row each (commands of spec 14.3); `hp4long.rar` is made once with `rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt` (RAR 6.24) for DEC-005's write back path. When `/tmp/bl131` is gone, remake each archive with its README command and re measure every offset the story's tests use (spec 14.3: RAR stamps times, sizes stay). **Obliged by:** SPEC-0087/FR-NEW-001 (spec 3.3, 9.0 archive table). **Rationale:** the offsets of spec 9.0 are those of these exact files. **Alternatives considered:** committing every cut (one archive covers every cut at run time). **Source:** spec 14.3, existing fixtures README. **Evidence:** `ls /tmp/bl131`, `crates/core/tests/fixtures/rar/README.md`.
- **DEC-009 (SPEC-0087):** internal tests: `non_rar5_input_is_left_to_the_library` becomes `partial_markers_are_left_to_the_library` (`b""` and `b"Rar!\x1a\x07\x01"` still `Ok`) plus `rar4_marker_alone_is_truncated` (`b"Rar!\x1a\x07\x00"` refused, message `archive is corrupt or not a valid rar file: unexpected end of archive`); `every_strict_prefix_of_a_rar5_archive_is_corrupt`, `huge_data_size_is_corrupt_not_a_panic` and the end block test at `:770` only switch to `ensure_rar_complete(.., None)`. Design tests (not spec tests): `rar5_check_value_of_hidden_rar_matches` and `rar3_key_decrypts_the_first_header_of_hp4` (header CRC matches after decryption), `rar3_key_with_a_long_password_decrypts_hp4long`. **Obliged by:** SPEC-0087/FR-MOD-001, FR-NEW-001 (spec 14.3 "internal tests that pin the old limitation"). **Rationale:** the old test pins the limitation this spec removes; the design tests pin each key derivation on a real archive before the acceptance tests depend on it. **Alternatives considered:** published test vectors (none exist for the RAR 3 derivation). **Source:** spec 14.3. **Evidence:** `crates/core/src/tools/archive_rar.rs:740-800`.
### Data model
None.
### Contracts
`Cargo.toml`: `sha1 = { version = "0.10", features = ["compress"] }` (feature of an existing direct dependency; `Cargo.lock` unchanged). `archive_rar_crypt.rs`: `pub(crate) fn rar3_kdf(password: &str, salt: &[u8; 8]) -> ([u8; 16], [u8; 16])` returning (key, IV): `raw` = the password encoded as UTF-16 little endian followed by the salt; a SHA-1 context (state `[u32; 5]` initialised to the SHA-1 constants, 64 bit byte count, 64 byte buffer) driven by `sha1::compress`; for `i` in `0..0x40000`: process `raw` with the RAR 2.9 variant (`sha1.cpp:146-168`: when a whole 64 byte block is processed straight from `raw`, compute the 80 word schedule of that block, `w[t] = rotl1(w[t-3] ^ w[t-8] ^ w[t-14] ^ w[t-16])`, and write `w[64..80]` back into those 64 bytes of `raw` as little endian words), then process the 3 bytes `i`, `i >> 8`, `i >> 16` normally; when `i % 0x4000 == 0` finish a copy of the context and set `iv[i / 0x4000]` to the last byte of its digest; at the end finish the context: `key[k * 4 + j] = (digest_word[k] >> (8 * j)) as u8` for `k`, `j` in 0..4; `pub(crate) fn aes128_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], buf: &mut [u8])`. `archive_rar.rs` RAR4 walk after a main header flagged `0x0080`, with a non empty password: each block is an 8 byte salt then `ceil16(HEAD_SIZE)` bytes of AES 128 CBC ciphertext (key and IV from `rar3_kdf`, cached per salt within the call); decrypt the first 16 bytes to read the base header, then the rest; CRC16 mismatch (low 16 bits of the CRC32 of the header after its CRC field) returns `Ok(())`; then skip `ADD_SIZE` (plus the high part) of data; type `0x7B` returns `Ok(())`; any shortfall `TRUNCATED`. New fixtures `hp4.rar` (220 bytes, `rar a -ma4 -ep -hps3cret-Ünï hp4.rar a.txt b.txt`) and `hp4long.rar` (`rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt`), both RAR 6.24, README rows.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify |
| `crates/core/src/tools/archive_rar_crypt.rs` | modify |
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify |
| `crates/core/tests/fixtures/rar/hp4.rar` | create |
| `crates/core/tests/fixtures/rar/hp4long.rar` | create |
| `crates/core/tests/fixtures/rar/README.md` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Fixtures load with `include_bytes!("../../tests/fixtures/rar/<name>.rar")` as the SPEC-0080 tests do (`crates/core/src/tools/archive_rar.rs` tests); a cut is a slice taken at run time (`&TWO4[..67]`), never a committed file. Password of every encrypted fixture: `s3cret-Ünï`. Today's walk to extend: `ensure_rar5_complete` and `read_vint` (`crates/core/src/tools/archive_rar.rs:98-160`); the password mapping it must not change: `listing_error` (`:162-169`) and the entry read mapping (`:62-81`). The every length loop model: `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`). The log and audit leak check model: `assert_never_recorded` (`:922-938`). Offsets of spec section 9.0 are measured on the archives of `/tmp/bl131`; an archive remade from its README command needs its offsets re measured. `hp4.rar` starts `52 61 72 21 1a 07 00` (marker), `ce 99 73 80 00 0d 00` plus 6 reserved bytes (main header, flags `0x0080`, size 13), then at offset 20 the salt `f3 68 27 2b 78 ec e5 69` of the first encrypted header. UnRAR's reference: `unrar_sys-0.5.8/vendor/unrar/crypt3.cpp` (`SetKey30`), `sha1.cpp:146-168` (`sha1_process_rar29`), `arcread.cpp` (`ReadHeader15`, salt before each header).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-003: Version 4 with encrypted file names, right password
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the answer is 2 files, 12 bytes; `/uploads/hp4/a.txt` holds `hello`, `/uploads/hp4/b.txt` holds `goodbye`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_003_version_4_with_encrypted_file_names`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-022: Version 4, encrypted names, cut between files, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4half.rar`, the first 108 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written (today `/uploads/hp4half/a.txt` is written and the call succeeds)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_022_version_4_encrypted_names_cut_betwee`
- Red before the change because: today the 108 byte cut extracts `a.txt` and succeeds (P-2)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-023: Version 4, encrypted names, cut inside data, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/hp4data.rar`, the first 190 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008, not the wrong password message; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_023_version_4_encrypted_names_cut_inside`
- Red before the change because: today the cut inside `b.txt`'s data with the right password answers `incorrect password for this archive; ...` (P-2: read `BadData`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-024: Version 4, encrypted names, leading part only, right password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4lead.rar`, the first 20 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; `/uploads/hp4lead` does not exist (today an empty folder is created)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_024_version_4_encrypted_names_leading_pa`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-048: The password stays out of a cut refusal
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4data.rar` of E2E-023
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; the text `s3cret-Ünï` appears in no message, log line or audit line written during the call
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_048_the_password_stays_out_of_a_cut_refu`
- Red before the change because: today the answer is the wrong password message (P-2), not the corrupt one; the leak half of the test holds today and stays
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-003 | make the RAR4 decrypted walk return `TRUNCATED` after its first header: the complete `hp4.rar` is refused, the test fails |
| E2E-024 | drop the US-0198 rule "non empty password and nothing after the main header: `TRUNCATED`": the 20 byte archive extracts nothing and succeeds with an empty folder, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none; only the `compress` feature of the existing `sha1` 0.10
- No dash as punctuation in code, comments or docs
- Fixture provenance (DEC-006): copy the archive from `/tmp/bl131/<name>.rar` (the file the spec measured; check its size with `wc -c` against the spec 9.0 table); when it is gone, remake it with the README command using the RARLAB trial build (`/tmp/rarbin/v6/rar/rar` is RAR 6.24, `/tmp/rarbin/rar/rar` is RAR 7.23, else download them from rarlab.com) and re measure every offset this story's tests use
- No `unsafe`, no `unrar_sys`, no hand written AES or SHA-1 compression (the schedule write back reuses `sha1::compress` for the state update)
- The password is never logged, traced or put in an error text
- Scope boundary: a wrong password on RAR4 encrypted file names stays corrupt (BL-0136): a CRC mismatch hands the archive to UnRAR

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-022, E2E-023, E2E-048), before the production code.
- [ ] Green on arrival tests (E2E-003, E2E-024): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract; `e2e_0080_065` (RAR4 wrong password on encrypted data) unchanged
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0202: decrypt RAR4 headers to see a cut with the right password`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-001, FR-NEW-006, FR-MOD-001).
