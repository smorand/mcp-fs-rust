# US-0175: ask for the password of an encrypted RAR and accept the right one

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0170, US-0169
> min_tier: 2
> Files touched: 5
> Closes spec: no
> Blocked: user provided fixtures missing today (see Constraints)

## Objective
An encrypted RAR with no password, a wrong password, or encrypted file names gets the retry message; the right password extracts; the password never reaches a log or an audit line.

## Requirements covered
- SPEC-0080/FR-NEW-002: IF a RAR archive is encrypted and no password is given THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and the message `password required to extract this archive; call again with the 'password' parameter`, and write nothing.
- SPEC-0080/FR-NEW-003: IF a RAR archive is encrypted and the given password is wrong THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and the message `incorrect password for this archive; call again with the correct 'password' parameter`, and write nothing.
- SPEC-0080/FR-NEW-004: IF a RAR archive has its file names encrypted THEN THE archive extraction SHALL treat the password exactly as for an archive with encrypted data: FR-NEW-002, FR-NEW-003 and FR-NEW-005 apply unchanged.
- SPEC-0080/FR-NEW-005: WHEN a member gives the right password for an encrypted RAR archive THE archive extraction SHALL extract it as FR-NEW-001 describes.
- SPEC-0080/FR-NEW-013: THE archive extraction SHALL NOT write the password into any log line or audit line, on success or on failure.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_003 e2e_0080_004 e2e_0080_005 e2e_0080_006 e2e_0080_007 e2e_0080_020 e2e_0080_021 e2e_0080_022 e2e_0080_030`: all 9 tests pass. Extracted content is read back from the store; log lines are read through `crate::logging::capture` as `e2e_new_028` does.

## Design excerpt
### Decisions
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-007:** zip and 7z get the new password wording by extending `password_required_message` (`archive.rs:615-620`) and routing the three literals (`:441`, `:489`, `:531`) through it; the existing tests that pin the old text (`:1034`, `:1047`, `:2158`, `:2174`) and SPEC-0041's story tests are updated in the same story. **Obliged by:** SPEC-0080/FR-MOD-002, FR-MOD-003. **Rationale:** one wording for three formats. **Alternatives considered:** reword RAR only (leaves zip and 7z less helpful, rejected in the spec). **Source:** spec. **Evidence:** `archive.rs:441`, `:489`, `:531`, `:615-620`.
- **DEC-009:** test data: a test only RAR5 store builder (`archive_rar_fixtures.rs`: signature, main header, file headers with vint fields and CRC32, directory, symlink and hard link redirection records) produces the plain cases; encrypted, RAR4 and solid cases need real fixtures made with `rar` and committed under `crates/core/tests/fixtures/rar/` with a `README` line naming the command that made each. **Obliged by:** SPEC-0080 tests (spec Section 9 says fixtures come from the design). **Rationale:** `rar` is not installed, `7z` cannot create RAR; implementing RAR5 AES in the builder is out of proportion. **Alternatives considered:** the `unrar` crate's own `data/*.rar` (content and passwords differ from the spec's concrete data); a builder with encryption. **Source:** user answer (default taken 2026-11-05). **Evidence:** `which rar` returned nothing.
### Data model
None.
### Contracts
`decode_rar` password mapping (design DEC-004): a `MissingPassword` or `BadPassword` at `When::Read` (encrypted names) or any entry `is_encrypted()` with no password gives `password_required_message(None)`; a `BadPassword` or `BadData` on an encrypted entry with a password gives `password_required_message(Some(_))`. Fixtures (user provided): `secret.rar` (`rar a -ps3cret-Ünï secret.rar note.txt`, `note.txt` = `top secret`), `hidden.rar` (`rar a -hps3cret-Ünï hidden.rar note.txt`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
| `crates/core/tests/fixtures/rar/secret.rar` | create |
| `crates/core/tests/fixtures/rar/hidden.rar` | create |
### Existing patterns
`password_required_message` after US-0169; logging tests `e2e_new_028_failing_call_logs_no_literal_password` (`archive.rs:1380`) for the no log assertion. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-003: Right password extracts
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-005
- **Given** `/uploads/secret.rar` encrypted with `s3cret-Ünï`, holding `note.txt` containing `top secret`
- **When** the member extracts it with the password `s3cret-Ünï`
- **Then** the answer reports 1 file written and 10 bytes written; `/uploads/secret/note.txt` reads `top secret`
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_003_right_password_extracts`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-004: Right password on encrypted file names
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-NEW-005
- **Given** `/uploads/hidden.rar` with encrypted file names and password `s3cret-Ünï`, holding `note.txt`
- **When** the member extracts it with that password
- **Then** `/uploads/hidden/note.txt` exists
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_004_right_password_on_encrypted_file_nam`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-005: No password on an encrypted RAR
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** `/uploads/secret.rar` as in E2E-003
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message `password required to extract this archive; call again with the 'password' parameter`
- **And** `/uploads/secret` does not exist
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_005_no_password_on_an_encrypted_rar`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-006: No password on encrypted file names
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-004
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with no password
- **Then** the refusal and message are those of E2E-005; `/uploads/hidden` does not exist
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_006_no_password_on_encrypted_file_names`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-007: Wrong password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003
- **Given** `/uploads/secret.rar`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message `incorrect password for this archive; call again with the correct 'password' parameter`
- **And** `/uploads/secret` does not exist
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_007_wrong_password`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-020: Success is audited without the password
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-013
- **Given** the encrypted archive of E2E-003
- **When** the member extracts it with the password `s3cret-Ünï`
- **Then** the audit trail has exactly one extraction line for `/uploads/secret.rar` with the destination, 1 file written and 10 bytes written
- **And** neither the audit trail nor the logs contain `s3cret-Ünï`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_020_success_is_audited_without_the_passw`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-021: Wrong password is not logged
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-013
- **Given** `/uploads/secret.rar`
- **When** the member extracts it with the password `nope-12345`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` and no log or audit line contains `nope-12345`; there is no extraction audit line
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_021_wrong_password_is_not_logged`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-022: The HTTP API gives the same refusal and the same success
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-005
- **Given** `/uploads/secret.rar`
- **When** a client asks the HTTP API to extract it with no password, then with `s3cret-Ünï`
- **Then** the first answer carries the error code and message of E2E-005; the second carries the answer of E2E-003
- **Priority:** High
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_022_the_http_api_gives_the_same_refusal`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-030: Wrong password on encrypted file names
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-004, FR-NEW-013
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with the password `nope-67890`
- **Then** the refusal and message are those of E2E-007; `/uploads/hidden` does not exist; no log or audit line contains `nope-67890`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_030_wrong_password_on_encrypted_file_nam`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- The password is never logged or audited (FR-NEW-013); do not log `password`
- BLOCKED until `secret.rar` and `hidden.rar` exist (data: user provided, missing today)
- Scope boundary: partial encryption, empty or ignored passwords belong to US-0176

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Fixtures present before the story starts; otherwise stop and report BLOCKED with the missing file names.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0175: ask for the password of an encrypted RAR and accept the right one`, body `Spec: SPEC-0080` and the requirement ids.
