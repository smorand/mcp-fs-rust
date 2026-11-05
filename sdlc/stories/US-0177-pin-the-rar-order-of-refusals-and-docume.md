# US-0177: pin the RAR order of refusals, check membership and document the format

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0172, US-0173, US-0174, US-0176
> min_tier: 2
> Files touched: 5
> Closes spec: yes
> Blocked: user provided fixtures missing today (see Constraints)

## Objective
The 12 step order of refusals of FR-NEW-015 and the membership rule are pinned by tests, each proven to fail when the order is broken; the documentation describes the `rar` feature, its licence note and the fixtures. This closes SPEC-0080: its full acceptance set runs green.

## Requirements covered
- SPEC-0080/FR-NEW-015: IF several refusals apply to the same RAR archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (FR-NEW-018); (2) the path is missing (FR-NEW-019) or is a folder (FR-NEW-021); (3) multi volume name (FR-NEW-010) or, on a server without RAR support, FR-NEW-012; (4) the file names are encrypted and the password is missing or wrong (FR-NEW-002, FR-NEW-003, FR-NEW-020), including when the archive is also truncated; (5) corrupt (FR-NEW-011); (6) the first offending entry in archive order, a link or device entry (FR-NEW-006) or an escaping name (FR-NEW-007); (7) the data is encrypted and the password is missing or wrong (FR-NEW-002, FR-NEW-003, FR-NEW-016, FR-NEW-020); (8) decoded larger than declared (FR-NEW-008); (9) a `destination` that lies outside the project (`ERR_PATH_OUT_OF_BOUNDS`, message `path escapes the volume root: <destination>`); (10) collision (FR-NEW-009); (11) project storage limit (FR-NEW-014); (12) session write allowance (FR-NEW-022).
- SPEC-0080/FR-NEW-018: IF the caller is not a member of the project, including a platform administrator without membership, THEN THE archive extraction SHALL refuse with `ERR_FORBIDDEN` and the message `'<person>' is not a member of '<project>'`, and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_047 e2e_0080_048 e2e_0080_049 e2e_0080_053 e2e_0080_054 e2e_0080_055 e2e_0080_057 e2e_0080_058 e2e_0080_059 e2e_0080_060 e2e_0080_061 e2e_0080_066 e2e_0080_067 e2e_0080_068 e2e_0080_081`: all 15 tests pass. Every refused call leaves the store unchanged.

## Design excerpt
### Decisions
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-009:** test data: a test only RAR5 store builder (`archive_rar_fixtures.rs`: signature, main header, file headers with vint fields and CRC32, directory, symlink and hard link redirection records) produces the plain cases; encrypted, RAR4 and solid cases need real fixtures made with `rar` and committed under `crates/core/tests/fixtures/rar/` with a `README` line naming the command that made each. **Obliged by:** SPEC-0080 tests (spec Section 9 says fixtures come from the design). **Rationale:** `rar` is not installed, `7z` cannot create RAR; implementing RAR5 AES in the builder is out of proportion. **Alternatives considered:** the `unrar` crate's own `data/*.rar` (content and passwords differ from the spec's concrete data); a builder with encryption. **Source:** user answer (default taken 2026-11-05). **Evidence:** `which rar` returned nothing.
- **DEC-010:** story order: US-0169 (wording, independent), US-0170 (feature and plain extraction), US-0171 (unsafe entries, collision), US-0172 (limits), US-0173 (multi volume, corrupt, missing path), US-0174 (versions, solid, user fixtures), US-0175 and US-0176 (passwords, user fixtures), US-0177 (order, membership, docs, closes). **Obliged by:** n/a (no code impact): story planning. **Rationale:** the closing story needs every other fixture, so it is last. **Alternatives considered:** passwords before limits. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
No new code contract. Documentation: `README.md:294-295` formats gain `.rar` (opt in feature `rar`, UnRAR licence note); `AGENTS.md:130-131` and the Cargo features list gain `rar`; `.agent_docs/tools.md:178-183` gains the RAR rules; `.agent_docs/testing.md` gains the fixture provenance (which fixtures are built, which come from `rar`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
The order list of FR-NEW-015. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-047: Link entry wins over a missing password when names are readable
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-006
- **Given** `/uploads/both.rar`, encrypted with readable file names, holding `a.txt` and a symbolic link `l`
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_NOT_SUPPORTED` naming `l`, not `ERR_PASSWORD_REQUIRED`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_047_link_entry_wins_over_a_missing_passw`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-048: Corrupt wins over a wrong password when file names are readable
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-011
- **Given** `/uploads/cut.rar`, a truncated RAR with encrypted data and readable file names
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_048_corrupt_wins_over_a_wrong_password_w`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-049: Escaping name wins over a collision
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-007
- **Given** `/uploads/two.rar` holding `a.txt` and `../x.txt`, and `/uploads/two/a.txt` already existing
- **When** the member extracts it with overwrite false
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`, not `ERR_NO_CLOBBER`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_049_escaping_name_wins_over_a_collision`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-053: Non member
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-018
- **Given** `/uploads/report.rar` in project `alpha`, and `bob@example.com` who is not a member of `alpha`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`; nothing is written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_053_non_member`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-054: Platform administrator without membership
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-018
- **Given** the same archive and `root@example.com`, a platform administrator who is not a member of `alpha`
- **When** `root@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'root@example.com' is not a member of 'alpha'`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_054_platform_administrator_without_membe`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-055: Member of another project
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-018
- **Given** `carol@example.com`, a member of project `beta` only, and the archive in `alpha`
- **When** `carol@example.com` extracts it asking for project `alpha`
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'carol@example.com' is not a member of 'alpha'`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_055_member_of_another_project`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-057: Non member asking for a missing archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-018; **Priority:** High
- **Given** project `alpha` without `/uploads/ghost.rar`, and `bob@example.com` who is not a member of `alpha`
- **When** `bob@example.com` extracts `/uploads/ghost.rar`
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`, not `ERR_NOT_FOUND`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_057_non_member_asking_for_a_missing_arch`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-058: Truncated archive with encrypted file names, wrong password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-003; **Priority:** High
- **Given** `/uploads/cuth.rar`, a truncated RAR with encrypted file names, password `s3cret-Ünï`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_058_truncated_archive_with_encrypted_fil`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-059: Encrypted file names, right password, link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-006, FR-NEW-004; **Priority:** High
- **Given** `/uploads/hl2.rar` with encrypted file names, password `s3cret-Ünï`, holding `a.txt` and a symbolic link `l`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is `ERR_NOT_SUPPORTED` naming `l`; `/uploads/hl2` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_059_encrypted_file_names_right_password`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-060: Truncated archive with readable names, no password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-011; **Priority:** High
- **Given** `/uploads/cut2.rar`, a truncated RAR with encrypted data and readable file names
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_060_truncated_archive_with_readable_name`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-061: Destination outside the project
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015; **Priority:** Medium
- **Given** `/uploads/report.rar` as in E2E-001
- **When** the member extracts it with the destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_061_destination_outside_the_project`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-066: Missing password wins over an oversized entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/encbomb.rar`, encrypted data with readable names, entry `big.bin` declaring 10 bytes and decoding to 1 000 000
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-005
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_066_missing_password_wins_over_an_oversi`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-067: Oversized entry wins over a collision
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/bomb.rar` as in E2E-012 and an existing file `/uploads/bomb/big.bin`
- **When** the member extracts it with overwrite false
- **Then** the refusal is `ERR_INVALID_ARGUMENT` naming `big.bin`, not `ERR_NO_CLOBBER`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_067_oversized_entry_wins_over_a_collisio`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-068: Collision wins over the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-009; **Priority:** Medium
- **Given** the situation of E2E-011 (5 bytes remaining, 12 declared) and `/uploads/report/data/a.txt` already existing
- **When** the member extracts it with overwrite false
- **Then** the refusal is `ERR_NO_CLOBBER`, not `ERR_PROJECT_QUOTA_EXCEEDED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_068_collision_wins_over_the_storage_limi`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-081: The project refusal comes first
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-015, FR-NEW-022, FR-NEW-014; **Priority:** Medium
- **Given** a project storage limit with 5 bytes of remaining storage, a session write allowance with 5 bytes remaining, and `/uploads/report.rar` declaring 12 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_081_the_project_refusal_comes_first`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-047 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-048 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-049 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-053 | remove `f.state.authorize` from the helper: the test fails |
| E2E-054 | remove `f.state.authorize` from the helper: the test fails |
| E2E-055 | remove `f.state.authorize` from the helper: the test fails |
| E2E-057 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-058 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-059 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-060 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-061 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-066 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-067 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-068 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |
| E2E-081 | swap the two checks the test names (for example move the entry rules after the password rule): the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Documentation without dashes as punctuation
- BLOCKED while US-0174 to US-0176 are blocked (their fixtures)
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-047, E2E-048, E2E-049, E2E-053, E2E-054, E2E-055, E2E-057, E2E-058, E2E-059, E2E-060, E2E-061, E2E-066, E2E-067, E2E-068, E2E-081): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md:294-295`, `AGENTS.md:130-131`, `.agent_docs/tools.md:178-183`, `.agent_docs/testing.md` (fixture provenance).
- [ ] Fixtures present before the story starts; otherwise stop and report BLOCKED with the missing file names.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0080 is green (`cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_`), and every requirement of SPEC-0080 is cited by at least one story with status done or landed.
- [ ] One commit `US-0177: pin the RAR order of refusals, check membership and document the format`, body `Spec: SPEC-0080` and the requirement ids.
