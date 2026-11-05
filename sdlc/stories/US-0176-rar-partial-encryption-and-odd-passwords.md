# US-0176: handle partly encrypted RAR archives and empty or ignored passwords

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0175
> min_tier: 2
> Files touched: 3
> Closes spec: no
> Blocked: user provided fixtures missing today (see Constraints)

## Objective
A RAR with one encrypted entry among plain ones needs the password for the whole archive and writes nothing without it; an empty password for an encrypted archive is the wrong password refusal; a password for a plain archive is ignored.

## Requirements covered
- SPEC-0080/FR-NEW-016: IF at least one entry of a RAR archive is encrypted AND no password is given THEN THE archive extraction SHALL apply FR-NEW-002 to the whole archive and write nothing, not even the plain entries.
- SPEC-0080/FR-NEW-017: WHEN a member gives a password, empty or not, for a RAR archive in which no entry is encrypted THE archive extraction SHALL ignore it and extract as FR-NEW-001 describes.
- SPEC-0080/FR-NEW-020: IF an empty password is given for an encrypted RAR archive THEN THE archive extraction SHALL refuse as FR-NEW-003 describes.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_031 e2e_0080_042 e2e_0080_043 e2e_0080_050 e2e_0080_051 e2e_0080_052 e2e_0080_069 e2e_0080_070 e2e_0080_072`: all 9 tests pass. Nothing is written on refusal, read back with `client.exists`.

## Design excerpt
### Decisions
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-009:** test data: a test only RAR5 store builder (`archive_rar_fixtures.rs`: signature, main header, file headers with vint fields and CRC32, directory, symlink and hard link redirection records) produces the plain cases; encrypted, RAR4 and solid cases need real fixtures made with `rar` and committed under `crates/core/tests/fixtures/rar/` with a `README` line naming the command that made each. **Obliged by:** SPEC-0080 tests (spec Section 9 says fixtures come from the design). **Rationale:** `rar` is not installed, `7z` cannot create RAR; implementing RAR5 AES in the builder is out of proportion. **Alternatives considered:** the `unrar` crate's own `data/*.rar` (content and passwords differ from the spec's concrete data); a builder with encryption. **Source:** user answer (default taken 2026-11-05). **Evidence:** `which rar` returned nothing.
### Data model
None.
### Contracts
No new code contract beyond DEC-004: `Some("")` counts as a given password. Fixture (user provided): `part.rar` (`a.txt` plain, `b.txt` encrypted with `s3cret-Ünï`, made with `rar a part.rar a.txt` then `rar a -ps3cret-Ünï part.rar b.txt`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
| `crates/core/tests/fixtures/rar/part.rar` | create |
### Existing patterns
US-0175 tests. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do. HTTP door tests: `Harness::new()`, `h.post(&u("extract-archive"), json!({...}))` as `e2e_new_031_extract_archive_happy_path` at `crates/core/src/api/dataplane.rs:3339`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-031: Password on a plain RAR is ignored
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-017
- **Given** the plain `/uploads/report.rar` of E2E-001
- **When** the member extracts it with the password `whatever`
- **Then** the answer is that of E2E-001
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_031_password_on_a_plain_rar_is_ignored`
- Green on arrival: a password for a plain archive is ignored by the reader from US-0170
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-042: Empty password on a plain RAR is ignored
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-017
- **Given** the plain `/uploads/report.rar` of E2E-001
- **When** the member extracts it with an empty password
- **Then** the answer is that of E2E-001
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_042_empty_password_on_a_plain_rar_is_ign`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-043: Ignored password through the HTTP API
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-017
- **Given** the plain `/uploads/report.rar` of E2E-001
- **When** a client asks the HTTP API to extract it with the password `whatever`
- **Then** the client receives the answer of E2E-001
- **Priority:** Medium
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_043_ignored_password_through_the_http_ap`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-050: One encrypted entry among plain ones, no password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-016
- **Given** `/uploads/part.rar` with `a.txt` plain and `b.txt` encrypted with `s3cret-Ünï`
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-005; `/uploads/part` does not exist, so `a.txt` was not written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_050_one_encrypted_entry_among_plain_ones`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-051: Empty password on an encrypted RAR
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-020
- **Given** `/uploads/secret.rar`
- **When** the member extracts it with an empty password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_051_empty_password_on_an_encrypted_rar`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-052: Partly encrypted RAR with the right password
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-016
- **Given** `/uploads/part.rar` as in E2E-050
- **When** the member extracts it with `s3cret-Ünï`
- **Then** 2 files are written and `/uploads/part/a.txt` and `/uploads/part/b.txt` exist
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_052_partly_encrypted_rar_with_the_right`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-069: Empty password on encrypted file names
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-020, FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/hidden.rar`
- **When** the member extracts it with an empty password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_069_empty_password_on_encrypted_file_nam`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-070: Empty password through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-020; **Priority:** Medium
- **Given** `/uploads/secret.rar`
- **When** a client asks the HTTP API to extract it with an empty password
- **Then** the error code and message are those of E2E-007
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_070_empty_password_through_the_http_api`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-072: Partly encrypted RAR, no password, through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-016; **Priority:** Medium
- **Given** `/uploads/part.rar` as in E2E-050
- **When** a client asks the HTTP API to extract it with no password
- **Then** the error code and message are those of E2E-005; `/uploads/part` does not exist
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_072_partly_encrypted_rar_no_password_thr`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-031 | pass the password to `Archive::with_password` and fail on an empty list of encrypted entries: the test fails |
| E2E-042 | same |
| E2E-043 | same |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- BLOCKED until `part.rar` exists (data: user provided, missing today)
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-031, E2E-042, E2E-043): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Fixtures present before the story starts; otherwise stop and report BLOCKED with the missing file names.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0176: handle partly encrypted RAR archives and empty or ignored passwords`, body `Spec: SPEC-0080` and the requirement ids.
