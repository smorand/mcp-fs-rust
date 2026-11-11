# US-0231: judge a version 4 password on the first protected block

> Spec: SPEC-0090 (sdlc/specs/SPEC-0090-tell-a-wrong-password-on-rar4-encrypted-names/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0230
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A member who mistypes the password of a RAR version 4 archive with encrypted file names is told today that the archive is cut short or damaged. After this story the completeness walk judges the password on the archive's first protected block: when its start does not read as a block, or the whole block fails its check, the member is told the password is wrong, cut archive or not. The two existing tests that pinned the old answer change with the code.

## Requirements covered
- SPEC-0090/FR-NEW-001: IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is whole AND it is not a possible block start THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`, whether the rest of the archive is complete, cut or damaged.
- SPEC-0090/FR-NEW-002: IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is a possible block start AND the whole block is present AND it fails its integrity check THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`.
- SPEC-0090/FR-MOD-001: IF a version 4 RAR archive with encrypted file names whose leading part is whole is given a password that is not empty and not the right one THEN THE archive extraction SHALL answer as FR-NEW-001 to FR-NEW-004 decide, cut or not.
- SPEC-0090/FR-NEW-008 (partial: no password and the empty password on the damaged cut (E2E-MOD-002)): THE archive extraction SHALL keep today's answers for no password and for the empty password on a version 4 archive with encrypted file names, for every password on a version 4 archive with readable file names, and for every version 5 archive.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0090_003 e2e_0090_004 e2e_0090_005 e2e_0090_006 e2e_0090_007 e2e_0090_008 e2e_0090_009 e2e_0090_014 e2e_0090_015 e2e_0090_035 e2e_0090_mod_001 e2e_0090_mod_002 rar4_first_encrypted_header_judges_the_password`: 13 tests pass. Real assertions: code and message of each refusal, the destination folder absent, `quota::used_bytes` and `session_remaining` unchanged, no audit line.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0090):** the password is judged in `walk_rar4_encrypted` on the first protected block only, before the rest of the walk, with the key already derived for it: start not possible -> WRONG; possible and the block (its declared size rounded up to whole AES blocks) runs past the end -> CUT (today's `rest.get(..padded)`); whole and CRC16 failing -> WRONG; passing -> the existing loop goes on for the later blocks, where a CRC16 failure still returns `Ok(())` for UnRAR to answer and a shortfall stays CUT. The start's 24 bytes missing stays CUT (today's `get(..RAR4_SALT)` and `get(..AES_BLOCK)`). **Obliged by:** SPEC-0090/FR-NEW-001 to FR-NEW-005, FR-MOD-001, FR-MOD-002. **Rationale:** the only bytes that can judge a version 4 password are the first protected block's; judging there puts WRONG before the cut and every later refusal.
- **DEC-002 (SPEC-0090):** `possible_start` is the spec's rule verbatim: kinds `0x74` to `0x7b`; declared size at least 32 (`SIZEOF_FILEHEAD3`) for file `0x74` and service `0x7a`, at least 7 (`SIZEOF_SHORTBLOCKHEAD`) for the six others; `RAR4_FILE_MIN: usize = 32` added next to `RAR4_BASE_HEADER` (`archive_rar.rs:142`). **Obliged by:** SPEC-0090/FR-NEW-001, FR-NEW-004 (definitions of section 6). **Rationale:** normative rule, same answer in every implementation.
- **DEC-003 (SPEC-0090):** WRONG is `password_required_message(Some(password))` and CUT is `corrupt("rar", TRUNCATED)`, the existing constructors, so both doors and the logging stay as they are. **Obliged by:** SPEC-0090/FR-NEW-001 to FR-NEW-004, FR-NEW-006, FR-NEW-007. **Rationale:** one wording per answer.
- **DEC-004 (SPEC-0090):** the two existing tests the spec changes are changed in the same story as the code: the SPEC-0087 test is renamed `e2e_0090_mod_001_version_4_encrypted_names_wrong_p` in place (it keeps its file, its four calls, now `password_incorrect`), and the internal check is renamed `rar4_first_encrypted_header_judges_the_password` with its `garbled` line asserting the WRONG error; E2E-MOD-002 also gets a tool level test `e2e_0090_mod_002_a_damaged_first_block_with_the_ri`. **Obliged by:** SPEC-0090/FR-MOD-001, FR-NEW-002, FR-NEW-008. **Rationale:** main stays green after the one behaviour change; the spec's filter finds the modified test.
- **DEC-006 (SPEC-0090):** fixtures are slices and bit flips of the committed archives at run time, through local helpers in `archive_rar_password_e2e.rs` (`refused(stem, bytes, password, check)`, `wrong`, `cut`, `corrupt_not_cut`, modelled on `archive_rar_cut_e2e.rs:24-56`, which stay private there). **Obliged by:** SPEC-0090 3.3. **Rationale:** no archive is made at test time.
### Data model
`walk_rar4_encrypted` keeps a `first: bool`, true until the first protected block is judged.
### Contracts
- `const RAR4_FILE_MIN: usize = 32;` next to `RAR4_BASE_HEADER` (`crates/core/src/tools/archive_rar.rs:142`).
- `fn possible_start(first: &[u8; AES_BLOCK]) -> bool { let kind = first[2]; let size = usize::from(u16::from_le_bytes([first[5], first[6]])); match kind { RAR4_FILE | RAR4_SERVICE => size >= RAR4_FILE_MIN, 0x75..=0x79 | RAR4_END => size >= RAR4_BASE_HEADER, _ => false } }`.
- In `walk_rar4_encrypted`, for the first block only: after decrypting `first`, `if !possible_start(&first) { return Err(password_required_message(Some(password))); }`; the `rest.get(..padded)` shortfall stays CUT; after the full decrypt, `if !crc_matches { return Err(password_required_message(Some(password))); }`; then `first = false`. For later blocks the code is today's (`!crc_matches` returns `Ok(())`).
- Answers: WRONG `ERR_PASSWORD_REQUIRED` `incorrect password for this archive; call again with the correct 'password' parameter`; CUT `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid rar file: unexpected end of archive`.
- The doc comment of `walk_rar4_encrypted` (`:278-286`) is rewritten: the first protected block judges the password (no check value in version 4), later CRC failures are UnRAR's; the BL-0136 sentence goes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify (walk, `possible_start`, test renamed `rar4_first_encrypted_header_judges_the_password`) |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify (`e2e_0087_027_version_4_encrypted_names_cut_wrong` renamed `e2e_0090_mod_001_version_4_encrypted_names_wrong_p`, expects WRONG) |
| `crates/core/src/tools/archive_rar_password_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] mod archive_rar_password_e2e;`) |

Read only (imported, never modified): `crates/core/src/tools/archive_testkit.rs`.
### Existing patterns
`walk_rar4_encrypted` (`crates/core/src/tools/archive_rar.rs:287-335`), `password_required_message` (`crates/core/src/tools/archive.rs:1005-1015`), `assert_refused_and_nothing_written` (`crates/core/src/tools/archive_rar_cut_e2e.rs:33-54`), the internal check `rar4_encrypted_headers_left_to_the_library` (`archive_rar.rs:1750-1767`).
### Fixtures
Local helpers in `archive_rar_password_e2e.rs` (DEC-006), modelled on `archive_rar_cut_e2e.rs:24-56`: `const HP4`, `HP4LONG`, `V4_SECRET`, `HIDDEN` (`include_bytes!` of the committed archives), `const PASSWORD: &str = "s3cret-Ünï"`, `const LONG: &str = "CorrectHorseBatteryStapleÜnï0123456789ab"`; `fn flip(bytes: &[u8], k: usize) -> Vec<u8>` (bit 0 of byte k); `fn wrong(v: &Result<Value>)` (`ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`); `fn cut(v)` (`ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: unexpected end of archive`); `fn corrupt_not_cut(v)` (`ERR_INVALID_ARGUMENT`, starts with `archive is corrupt or not a valid rar file:`, not CUT); `async fn refused(stem, bytes, password, check)`: seeds `/uploads/{stem}.rar` in a fresh `archive_testkit::fixture()`, notes `quota::used_bytes` and `f.state.safety.session_remaining(OWNER, MOUNT)`, extracts with no destination and `overwrite` false, runs `check`, then asserts "nothing written": `/uploads/{stem}` absent, both figures unchanged, `extraction_audit_lines` 0.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0090/E2E-003: Wrong password on the whole archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG (today CUT); nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_003_wrong_password_on_the_whole_archive`
- Red before the change because: on US-0230 code `nope` reads the start as a size past the end: CUT
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-004: Wrong password that today reads as a damaged header
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `w785`
- **Then** the refusal is WRONG (today `archive is corrupt or not a valid rar file: File header damaged`); nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_004_wrong_password_that_today_reads_as_a`
- Red before the change because: on US-0230 code `w785` passes the walk with a failing CRC16 and UnRAR answers `File header damaged`
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-005: Password one letter off
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Üni`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_005_password_one_letter_off`
- Red before the change because: on US-0230 code CUT (spec P-1)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-006: Another archive's password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4long.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_006_another_archive_s_password`
- Red before the change because: on US-0230 code CUT (spec P-1)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-007: Wrong password on a cut archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001, FR-MOD-002; **Priority:** Critical
- **Given** `/uploads/hp4half.rar`, the first 108 bytes of `hp4.rar`
- **When** the member extracts it with `nope`, then with `w785`
- **Then** each refusal is WRONG, not CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_007_wrong_password_on_a_cut_archive`
- Red before the change because: on US-0230 code `nope` CUT, `w785` `File header damaged`
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-008: Wrong password, archive cut inside the first protected block
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4in.rar`, the first 50 bytes of `hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_008_wrong_password_archive_cut_inside_the`
- Red before the change because: on US-0230 code CUT
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-009: Start exactly whole
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** `/uploads/hp4s.rar`, the first 44 bytes of `hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_009_start_exactly_whole`
- Red before the change because: on US-0230 code CUT (the start is whole but the size runs past 44 bytes)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-014: Right password, first protected block damaged
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-MOD-001; **Priority:** Critical
- **Given** `/uploads/hp4d.rar`, `hp4.rar` with byte 50 changed
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is WRONG (today `archive is corrupt or not a valid rar file: File header damaged`); nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_014_right_password_first_protected_block`
- Red before the change because: on US-0230 code the failing CRC16 is left to UnRAR: `File header damaged`
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-015: Wrong password on the damaged archive
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/hp4d.rar` of E2E-014
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_015_wrong_password_on_the_damaged_archive`
- Red before the change because: on US-0230 code CUT
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-035: Right password, first block of another archive damaged
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/uploads/hp4longd.rar`, `hp4long.rar` with byte 50 changed
- **When** the member extracts it with `CorrectHorseBatteryStapleÜnï0123456789ab`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_035_right_password_first_block_of_another`
- Red before the change because: on US-0230 code `File header damaged`
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-MOD-001: Version 4, encrypted names, wrong password (was SPEC-0087 E2E-027)
- **Validated before:** with `nope` and `w785`, on the first 108 bytes of `hp4.rar` and on the whole archive, a refusal starting `archive is corrupt or not a valid rar file:`.
- **Validates now:** the same four calls answer WRONG.
- **Given** `/uploads/hp4half.rar` (first 108 bytes) and `/uploads/hp4.rar`
- **When** the member extracts each with `nope`, then with `w785`
- **Then** each of the four refusals is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_cut_e2e.rs (the SPEC-0087 test renamed in place)
- Function: `e2e_0090_mod_001_version_4_encrypted_names_wrong_p`
- Red before the change because: after the rename and the new expectation (step 1), US-0230 code answers CUT and `File header damaged`
- Note: modifies the existing `e2e_0087_027_version_4_encrypted_names_cut_wrong` (`archive_rar_cut_e2e.rs:540-549`): renamed, `corrupt_any` replaced by a WRONG check, its comment about `w785` and BL-0136 removed.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-MOD-002: A damaged first block with the right password (was the internal check that leaves it to the reading library, 14.1 row 8)
- **Validated before:** the first 108 bytes of `hp4.rar` with byte 50 changed, right password, pass the completeness check and are left to the reading library; the same cut with no password or the empty password pass it too.
- **Validates now:** that damaged cut, right password: WRONG. With no password and with the empty password: unchanged (needed, and WRONG as today).
- **Given** the first 108 bytes of `hp4.rar` with byte 50 changed, as `/uploads/hp4dc.rar`
- **When** the member extracts it with `s3cret-Ünï`, then with no password, then with the empty password
- **Then** WRONG; then `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; then WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs; plus the internal check in `archive_rar.rs` tests
- Function: `e2e_0090_mod_002_a_damaged_first_block_with_the_ri`
- Red before the change because: on US-0230 code the damaged cut with the right password is CUT through the tool and `ensure_rar_complete` returns `Ok`
- Note: modifies the existing `rar4_encrypted_headers_left_to_the_library` (`archive_rar.rs:1750-1767`): renamed `rar4_first_encrypted_header_judges_the_password`; its `garbled` line now asserts `ensure_rar_complete(&garbled, Some(PASSWORD))` is the WRONG error; its 108 byte cut (CUT), `None` and `Some("")` assertions stay unchanged.
- Cleanup: none (temporary directories dropped with the fixture)

## Constraints
- Files not to touch: `crates/core/tests/fixtures/rar/*` (no archive made or committed), `crates/core/src/tools/archive_testkit.rs`, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Judge the first protected block only: later blocks keep today's handling (FR-NEW-005)
- Never make the possible start stricter than the spec's rule (flags, padding): E2E-019 depends on it
- No dash as punctuation in code, comments or docs
- Scope boundary: cuts and the right password are pinned in US-0232, order and trace in US-0233, the HTTP door and docs in US-0234

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-003, E2E-004, E2E-005, E2E-006, E2E-007, E2E-008, E2E-009, E2E-014, E2E-015, E2E-035, E2E-MOD-001, E2E-MOD-002 (step 1: the new tests and the two modified tests on US-0230 code, each failing for the reason stated; step 2: the judgment).
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0080_` and `e2e_0087_` test other than the renamed one, the key derivation pins `rar3_key_decrypts_the_first_header_of_hp4` and `rar3_key_with_a_long_password_decrypts_hp4long`, SPEC-0080 E2E-065, the REST `assert_cut_refused` tests: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_ e2e_0087_ e2e_0090_`
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0231: judge a version 4 password on the first protected block` on its story branch `story/US-0231-judge-a-version-4-password-on-the-first`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0090-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0090` and the requirement ids (SPEC-0090/FR-NEW-001, FR-NEW-002, FR-MOD-001, FR-NEW-008).
