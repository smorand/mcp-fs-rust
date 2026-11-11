# US-0232: cuts before the judgment, the right password and the accepted limit

> Spec: SPEC-0090 (sdlc/specs/SPEC-0090-tell-a-wrong-password-on-rar4-encrypted-names/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0231
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Pin the answers the judgment must not change: an archive cut before the first protected block's start is whole is cut whatever the password; a possible start cut short is cut, the right password's case and the accepted limit of a wrong password that happens to read as a block; with the right password, extraction, later damage and later cuts answer as before; three hundred generated wrong passwords give WRONG at least 291 times and nothing else than WRONG or CUT.

## Requirements covered
- SPEC-0090/FR-NEW-003: IF a version 4 RAR archive declares encrypted file names in its whole leading part AND a password that is not empty is given AND the archive ends before the start of the first protected block is whole THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, whatever the password.
- SPEC-0090/FR-NEW-004: IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is a possible block start AND the archive ends before that block, at the size its start declares, is whole THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`.
- SPEC-0090/FR-NEW-005: IF the first protected block of a version 4 RAR archive with encrypted file names is whole and passes its integrity check with the given password THEN THE archive extraction SHALL extract or refuse the archive exactly as before this increment.
- SPEC-0090/FR-NEW-001 (partial: the bulk of wrong passwords, E2E-020): IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is whole AND it is not a possible block start THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`, whether the rest of the archive is complete, cut or damaged.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0090_001 e2e_0090_002 e2e_0090_010 e2e_0090_011 e2e_0090_012 e2e_0090_013 e2e_0090_016 e2e_0090_017 e2e_0090_018 e2e_0090_019 e2e_0090_020`: 11 tests pass. Real assertions: extracted file contents, code and message of each refusal, nothing written, the WRONG count over 300 passwords.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0090):** the password is judged in `walk_rar4_encrypted` on the first protected block only, before the rest of the walk, with the key already derived for it: start not possible -> WRONG; possible and the block (its declared size rounded up to whole AES blocks) runs past the end -> CUT (today's `rest.get(..padded)`); whole and CRC16 failing -> WRONG; passing -> the existing loop goes on for the later blocks, where a CRC16 failure still returns `Ok(())` for UnRAR to answer and a shortfall stays CUT. The start's 24 bytes missing stays CUT (today's `get(..RAR4_SALT)` and `get(..AES_BLOCK)`). **Obliged by:** SPEC-0090/FR-NEW-001 to FR-NEW-005, FR-MOD-001, FR-MOD-002. **Rationale:** the only bytes that can judge a version 4 password are the first protected block's; judging there puts WRONG before the cut and every later refusal.
- **DEC-002 (SPEC-0090):** `possible_start` is the spec's rule verbatim: kinds `0x74` to `0x7b`; declared size at least 32 (`SIZEOF_FILEHEAD3`) for file `0x74` and service `0x7a`, at least 7 (`SIZEOF_SHORTBLOCKHEAD`) for the six others; `RAR4_FILE_MIN: usize = 32` added next to `RAR4_BASE_HEADER` (`archive_rar.rs:142`). **Obliged by:** SPEC-0090/FR-NEW-001, FR-NEW-004 (definitions of section 6). **Rationale:** normative rule, same answer in every implementation.
- **DEC-005 (SPEC-0090):** E2E-020 runs the 300 passwords through `ensure_rar_complete(HP4, Some(pw))` split across `std::thread::available_parallelism()` scoped threads (one RAR 3 key derivation each, about 0.5 s in the debug build), mapping each result to WRONG, CUT or other, and runs `wrong0`, `wrong135`, `wrong299` and `nope` through the typed engine call with "nothing written" checked; it asserts every answer is WRONG or CUT, at least 291 WRONG, `wrong135` CUT. **Obliged by:** SPEC-0090/FR-NEW-001, FR-NEW-004 (E2E-020), spec 14.3 run time. **Rationale:** the spec allows the completeness check for the bulk provided four go through the tool; the walk is the whole judgment and writes nothing.
- **DEC-006 (SPEC-0090):** fixtures are slices and bit flips of the committed archives at run time, through local helpers in `archive_rar_password_e2e.rs` (`refused(stem, bytes, password, check)`, `wrong`, `cut`, `corrupt_not_cut`, modelled on `archive_rar_cut_e2e.rs:24-56`, which stay private there). **Obliged by:** SPEC-0090 3.3. **Rationale:** no archive is made at test time.
### Contracts
No production change. E2E-020's bulk calls `crate::tools::archive_rar::ensure_rar_complete(HP4, Some(&format!("wrong{i}")))` (`pub(super)` in `tools::archive_rar`, `archive_rar.rs:228`, hence visible from the sibling `tools::archive_rar_password_e2e`; no visibility change), split over `std::thread::scope` threads, one per `std::thread::available_parallelism()`, each mapping its results to WRONG, CUT or other.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar_password_e2e.rs` | modify |

Read only (imported, never modified): `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/archive_rar.rs`.
### Existing patterns
US-0231's helpers; `archive_rar_cut_e2e.rs:577-624` (SPEC-0087 E2E-034, right password cuts) for the cut loops.
### Fixtures
Local helpers in `archive_rar_password_e2e.rs` (DEC-006), modelled on `archive_rar_cut_e2e.rs:24-56`: `const HP4`, `HP4LONG`, `V4_SECRET`, `HIDDEN` (`include_bytes!` of the committed archives), `const PASSWORD: &str = "s3cret-Ünï"`, `const LONG: &str = "CorrectHorseBatteryStapleÜnï0123456789ab"`; `fn flip(bytes: &[u8], k: usize) -> Vec<u8>` (bit 0 of byte k); `fn wrong(v: &Result<Value>)` (`ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`); `fn cut(v)` (`ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: unexpected end of archive`); `fn corrupt_not_cut(v)` (`ERR_INVALID_ARGUMENT`, starts with `archive is corrupt or not a valid rar file:`, not CUT); `async fn refused(stem, bytes, password, check)`: seeds `/uploads/{stem}.rar` in a fresh `archive_testkit::fixture()`, notes `quota::used_bytes` and `f.state.safety.session_remaining(OWNER, MOUNT)`, extracts with no destination and `overwrite` false, runs `check`, then asserts "nothing written": `/uploads/{stem}` absent, both figures unchanged, `extraction_audit_lines` 0.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0090/E2E-001: Right password extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-005, FR-NEW-008; **Priority:** Critical
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** `/uploads/hp4/a.txt` holds `hello`, `/uploads/hp4/b.txt` holds `goodbye`; the answer reports 2 files and 12 bytes; one audit line is recorded
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_001_right_password_extracts`
- Green on arrival: the right password extracts today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-002: A 40 character password extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/hp4long.rar`
- **When** the member extracts it with `CorrectHorseBatteryStapleÜnï0123456789ab`
- **Then** `/uploads/hp4long/a.txt` holds `hello`; 1 file, 5 bytes
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_002_a_40_character_password_extracts`
- Green on arrival: the long password extracts today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-010: One byte short of the start
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/uploads/hp4t.rar`, the first 43 bytes of `hp4.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_010_one_byte_short_of_the_start`
- Green on arrival: 43 bytes miss the start (today's `get(..AES_BLOCK)`); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-011: Leading part alone, or one byte more
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/hp4l.rar`, the first 20 bytes of `hp4.rar`, then the first 21 bytes
- **When** the member extracts each with `nope`, then with `s3cret-Ünï`
- **Then** all four refusals are CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_011_leading_part_alone_or_one_byte_more`
- Green on arrival: 20 and 21 bytes miss the start; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-012: Right password, cut before the start is whole
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** the first 30 bytes, then the first 43 bytes of `hp4.rar`, each stored as `/uploads/hp4r.rar`
- **When** the member extracts each with `s3cret-Ünï`
- **Then** each refusal is CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_012_right_password_cut_before_the_start_i`
- Green on arrival: the right password on a short start is CUT; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-013: Right password, cut inside the first protected block
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** the first 44, 50, then 91 bytes of `hp4.rar`, each stored as `/uploads/hp4r.rar`
- **When** the member extracts each with `s3cret-Ünï`
- **Then** each refusal is CUT, not WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_013_right_password_cut_inside_the_first_p`
- Green on arrival: a possible start running past the end stays CUT (US-0231); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-016: Right password, a later block damaged
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** `/uploads/hp4b.rar`, `hp4.rar` with byte 140 changed (inside `b.txt`'s block)
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is CORRUPT, not WRONG; nothing written, `/uploads/hp4b/a.txt` does not exist
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_016_right_password_a_later_block_damaged`
- Green on arrival: later blocks keep today's handling; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-017: Right password, cut after the first file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_017_right_password_cut_after_the_first_fi`
- Green on arrival: a cut after the first block is CUT; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-018: Right password, cut inside the second file's data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/hp4data.rar`, the first 190 bytes of `hp4.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_018_right_password_cut_inside_the_second`
- Green on arrival: a cut inside the second file's data is CUT; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-019: A wrong password that reads as a possible block (accepted limit)
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `wrong135`
- **Then** the refusal is CUT; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_019_a_wrong_password_that_reads_as_a_poss`
- Green on arrival: `wrong135` reads a possible file block of 28 935 bytes (spec P-2); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-020: Three hundred wrong passwords
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-004; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with each of `wrong0` to `wrong299`
- **Then** every refusal is WRONG or CUT, none is a success or another message; at least 291 are WRONG; `wrong135` is CUT; nothing written after any of them
- Level / Driver / Location: functional e2e / `ensure_rar_complete` for 296 passwords on parallel scoped threads, typed engine call for `nope`, `wrong0`, `wrong135`, `wrong299` (DEC-005) / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_020_three_hundred_wrong_passwords`
- Green on arrival: US-0231 answers WRONG for every start that is not possible; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-001 | compute the first block CRC16 over `plain[..size]` instead of `plain[2..size]`: the right password reads WRONG, the test fails |
| E2E-002 | as E2E-001 |
| E2E-010 | decrypt a short start padded with zeros instead of refusing: `nope` reads WRONG, the test fails |
| E2E-011 | as E2E-010 |
| E2E-012 | as E2E-010: the right password on a zero padded start reads WRONG, the test fails |
| E2E-013 | answer WRONG when a possible start runs past the end: the test fails |
| E2E-016 | apply the CRC16 WRONG rule to every block, not only the first: the damaged `b.txt` block reads WRONG, the test fails |
| E2E-017 | return `Ok(())` once the first block passes (stop walking): UnRAR answers the 108 byte cut otherwise than CUT, the test fails |
| E2E-018 | as E2E-017 |
| E2E-019 | as E2E-013 |
| E2E-020 | accept every kind as a possible start (`_ => true` in `possible_start`): far fewer than 291 WRONG, the test fails |

## Constraints
- Files not to touch: `crates/core/tests/fixtures/rar/*` (no archive made or committed), `crates/core/src/tools/archive_testkit.rs`, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- No production change in this story
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] Green on arrival tests (E2E-001, E2E-002, E2E-010, E2E-011, E2E-012, E2E-013, E2E-016, E2E-017, E2E-018, E2E-019, E2E-020): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0080_` and `e2e_0087_` test other than the renamed one, the key derivation pins `rar3_key_decrypts_the_first_header_of_hp4` and `rar3_key_with_a_long_password_decrypts_hp4long`, SPEC-0080 E2E-065, the REST `assert_cut_refused` tests: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_ e2e_0087_ e2e_0090_`
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0232: cuts before the judgment, the right password and the accepted limit` on its story branch `story/US-0232-cuts-before-the-judgment-the-right-passw`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0090-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0090` and the requirement ids (SPEC-0090/FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-001).
