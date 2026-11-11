# US-0233: the wrong password first, nothing left, other answers unchanged

> Spec: SPEC-0090 (sdlc/specs/SPEC-0090-tell-a-wrong-password-on-rar4-encrypted-names/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0232
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Pin the place of the new wrong password answer among the refusals (after membership, before a collision, the storage limit and a new destination), that a refused call leaves nothing, twice, never records the password, never replaces a file with `overwrite` set, and that a retry with the right password extracts; and that no password, the empty password, readable names and version 5 answer as before.

## Requirements covered
- SPEC-0090/FR-MOD-002: IF a refusal under FR-NEW-001 or FR-NEW-002 meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password, a wrong one (version 5, or version 4 under FR-NEW-001 or FR-NEW-002) or an empty one; (5) the truncation; (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.
- SPEC-0090/FR-NEW-006: THE archive extraction SHALL NOT write a file, create a folder, charge the remaining storage or the session write allowance, record an audit line, or show the password in a message, a log line or an audit line, for a call refused under FR-NEW-001 to FR-NEW-004.
- SPEC-0090/FR-NEW-008: THE archive extraction SHALL keep today's answers for no password and for the empty password on a version 4 archive with encrypted file names, for every password on a version 4 archive with readable file names, and for every version 5 archive.
- SPEC-0090/FR-NEW-005 (partial: the retry with the right password): IF the first protected block of a version 4 RAR archive with encrypted file names is whole and passes its integrity check with the given password THEN THE archive extraction SHALL extract or refuse the archive exactly as before this increment.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0090_021 e2e_0090_022 e2e_0090_023 e2e_0090_024 e2e_0090_025 e2e_0090_026 e2e_0090_027 e2e_0090_028 e2e_0090_029 e2e_0090_030 e2e_0090_031 e2e_0090_036`: 12 tests pass. Real assertions: codes and messages, the existing `/uploads/hp4/a.txt` content, `remaining_storage`, `session_remaining`, the audit trail, the captured log events.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0090):** the password is judged in `walk_rar4_encrypted` on the first protected block only, before the rest of the walk, with the key already derived for it: start not possible -> WRONG; possible and the block (its declared size rounded up to whole AES blocks) runs past the end -> CUT (today's `rest.get(..padded)`); whole and CRC16 failing -> WRONG; passing -> the existing loop goes on for the later blocks, where a CRC16 failure still returns `Ok(())` for UnRAR to answer and a shortfall stays CUT. The start's 24 bytes missing stays CUT (today's `get(..RAR4_SALT)` and `get(..AES_BLOCK)`). **Obliged by:** SPEC-0090/FR-NEW-001 to FR-NEW-005, FR-MOD-001, FR-MOD-002. **Rationale:** the only bytes that can judge a version 4 password are the first protected block's; judging there puts WRONG before the cut and every later refusal.
- **DEC-003 (SPEC-0090):** WRONG is `password_required_message(Some(password))` and CUT is `corrupt("rar", TRUNCATED)`, the existing constructors, so both doors and the logging stay as they are. **Obliged by:** SPEC-0090/FR-NEW-001 to FR-NEW-004, FR-NEW-006, FR-NEW-007. **Rationale:** one wording per answer.
- **DEC-006 (SPEC-0090):** fixtures are slices and bit flips of the committed archives at run time, through local helpers in `archive_rar_password_e2e.rs` (`refused(stem, bytes, password, check)`, `wrong`, `cut`, `corrupt_not_cut`, modelled on `archive_rar_cut_e2e.rs:24-56`, which stay private there). **Obliged by:** SPEC-0090 3.3. **Rationale:** no archive is made at test time.
### Contracts
No production change. E2E-027 sets the project's remaining storage to 5 bytes with `archive_testkit::limit_remaining(&f, 5)`; E2E-030 runs the call through `McpServer::fs_extract_archive` (the real handler) under `crate::logging::capture::lock_for_test` on a current thread runtime built in the test, then scans every captured event, the refusal text and `f.state.safety.audit(OWNER, MOUNT)` for `s3cret-Üni`; E2E-025 calls as `bob@example.com` through `archive_testkit::extract` (which runs the real membership check).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar_password_e2e.rs` | modify |

Read only (imported, never modified): `crates/core/src/tools/archive_testkit.rs`.
### Existing patterns
SPEC-0087's order tests (`archive_rar_cut_e2e.rs`), the log capture of SPEC-0088 E2E-035 (`crates/core/src/tools/office_bound_e2e.rs:939-948`), `archive_testkit::{limit_remaining, remaining_storage, allowance, extraction_audit_lines}`.
### Fixtures
Local helpers in `archive_rar_password_e2e.rs` (DEC-006), modelled on `archive_rar_cut_e2e.rs:24-56`: `const HP4`, `HP4LONG`, `V4_SECRET`, `HIDDEN` (`include_bytes!` of the committed archives), `const PASSWORD: &str = "s3cret-Ünï"`, `const LONG: &str = "CorrectHorseBatteryStapleÜnï0123456789ab"`; `fn flip(bytes: &[u8], k: usize) -> Vec<u8>` (bit 0 of byte k); `fn wrong(v: &Result<Value>)` (`ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`); `fn cut(v)` (`ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: unexpected end of archive`); `fn corrupt_not_cut(v)` (`ERR_INVALID_ARGUMENT`, starts with `archive is corrupt or not a valid rar file:`, not CUT); `async fn refused(stem, bytes, password, check)`: seeds `/uploads/{stem}.rar` in a fresh `archive_testkit::fixture()`, notes `quota::used_bytes` and `f.state.safety.session_remaining(OWNER, MOUNT)`, extracts with no destination and `overwrite` false, runs `check`, then asserts "nothing written": `/uploads/{stem}` absent, both figures unchanged, `extraction_audit_lines` 0.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0090/E2E-021: No password, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/hp4.rar`, and `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts each with no password
- **Then** each refusal is `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_021_no_password_unchanged`
- Green on arrival: no password stops the walk before the judgment; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-022: Empty password, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/hp4.rar`, and `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts each with the empty password
- **Then** each refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_022_empty_password_unchanged`
- Green on arrival: the empty password is still UnRAR's answer; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-023: Readable names, encrypted data, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/v4secret.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_023_readable_names_encrypted_data_unchang`
- Green on arrival: readable names never reach the encrypted names walk; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-024: Version 5, unchanged
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** Low
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_024_version_5_unchanged`
- Green on arrival: version 5 has its own check value; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-025: Not a member comes first
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/hp4.rar` in `atlas`; `bob@example.com` is not a member of `atlas`
- **When** `bob@example.com` extracts it with `nope`
- **Then** the refusal is `ERR_FORBIDDEN` with `'bob@example.com' is not a member of 'atlas'`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs as `bob@example.com`
- Function: `e2e_0090_025_not_a_member_comes_first`
- Green on arrival: membership is checked before the archive is read; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-026: Wrong password before a collision
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-MOD-002, FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4.rar` and an existing `/uploads/hp4/a.txt` holding `old`
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG, not the collision; `/uploads/hp4/a.txt` still holds `old`; `/uploads/hp4/b.txt` does not exist
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_026_wrong_password_before_a_collision`
- Green on arrival: the walk runs before the collision check; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-027: Wrong password before the storage limit
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/hp4.rar` and a remaining storage of 5 bytes (below the 12 declared)
- **When** the member extracts it with `nope`
- **Then** the refusal is WRONG, not the storage limit; remaining storage still 5 bytes; nothing written
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_027_wrong_password_before_the_storage_lim`
- Green on arrival: the walk runs before the declared size admission (SPEC-0085); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-028: Wrong password with a new destination
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-MOD-002, FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/hp4.rar`; `/out` does not exist
- **When** the member extracts it with `nope` and the destination `/out`
- **Then** the refusal is WRONG; `/out` does not exist
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs with destination `/out`
- Function: `e2e_0090_028_wrong_password_with_a_new_destination`
- Green on arrival: nothing is created before the listing; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-029: A refused wrong password leaves nothing, twice
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** `/uploads/hp4.rar`; the remaining storage and session write allowance are noted
- **When** the member extracts it with `nope`, twice
- **Then** both refusals are WRONG; `/uploads/hp4` does not exist; the remaining storage and session write allowance are unchanged; no audit line names `/uploads/hp4.rar`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs, called twice
- Function: `e2e_0090_029_a_refused_wrong_password_leaves_nothi`
- Green on arrival: a walk refusal writes nothing; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-030: The wrong password is never recorded
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it with `s3cret-Üni`
- **Then** the refusal is WRONG; `s3cret-Üni` appears in no message, no log line and no audit line
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_archive` under the log capture / crates/core/src/tools/archive_rar_password_e2e.rs
- Function: `e2e_0090_030_the_wrong_password_is_never_recorded`
- Green on arrival: the refusal is a fixed text and the tool logs the error display only; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-031: Overwrite set, refused, nothing replaced
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/uploads/hp4.rar` and `/uploads/hp4/a.txt` holding `old`
- **When** the member extracts it with `nope` and `overwrite` set
- **Then** the refusal is WRONG; `/uploads/hp4/a.txt` still holds `old`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs with `overwrite` true
- Function: `e2e_0090_031_overwrite_set_refused_nothing_replace`
- Green on arrival: a refusal comes before any write; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-036: Retry with the right password after a refusal
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-001; **Priority:** High
- **Given** `/uploads/hp4.rar`, just refused with `s3cret-Üni` (WRONG)
- **When** the member extracts it again with `s3cret-Ünï`
- **Then** `/uploads/hp4/a.txt` holds `hello`, `/uploads/hp4/b.txt` holds `goodbye`; exactly one audit line names `/uploads/hp4.rar`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract(&f, archive, destination, overwrite, password)` (authorizes, then the `fs.extract_archive` engine), `#[cfg(feature = "rar")]` / crates/core/src/tools/archive_rar_password_e2e.rs, then again with the right password
- Function: `e2e_0090_036_retry_with_the_right_password_after_a`
- Green on arrival: a refusal leaves no state; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-021 | judge with the empty string when no password is given: WRONG instead of the needed message, the test fails |
| E2E-022 | map an empty password to `None` before the listing: the needed message instead of WRONG, the test fails |
| E2E-023 | in `listing_error`, map `BadPassword` to the corrupt refusal: `v4secret.rar` with `nope` reads corrupt, the test fails |
| E2E-024 | in `walk_rar5`, skip the check value comparison: the version 5 wrong password is answered otherwise, the test fails |
| E2E-025 | in the extract engine, read and walk the archive before `authorize`: bob gets WRONG, the test fails |
| E2E-026 | run the collision check before `ensure_rar_complete`: the collision refusal comes, the test fails |
| E2E-027 | admit the declared sizes before `ensure_rar_complete`: the storage refusal comes, the test fails |
| E2E-028 | create the destination folder before listing: `/out` exists, the test fails |
| E2E-029 | charge the session before listing (`check_and_charge` moved first): the allowance drops, the test fails |
| E2E-030 | append the password to the WRONG message: the password appears, the test fails |
| E2E-031 | as E2E-026 with the overwrite path first: `a.txt` is replaced or the collision answers, the test fails |
| E2E-036 | cache the WRONG answer per archive path in the walk: the retry is refused, the test fails |

## Constraints
- Files not to touch: `crates/core/tests/fixtures/rar/*` (no archive made or committed), `crates/core/src/tools/archive_testkit.rs`, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- No production change in this story
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] Green on arrival tests (E2E-021, E2E-022, E2E-023, E2E-024, E2E-025, E2E-026, E2E-027, E2E-028, E2E-029, E2E-030, E2E-031, E2E-036): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0080_` and `e2e_0087_` test other than the renamed one, the key derivation pins `rar3_key_decrypts_the_first_header_of_hp4` and `rar3_key_with_a_long_password_decrypts_hp4long`, SPEC-0080 E2E-065, the REST `assert_cut_refused` tests: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_ e2e_0087_ e2e_0090_`
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0233: the wrong password first, nothing left, other answers unchanged` on its story branch `story/US-0233-the-wrong-password-first-nothing-left-ot`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0090-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0090` and the requirement ids (SPEC-0090/FR-MOD-002, FR-NEW-006, FR-NEW-008, FR-NEW-005).
