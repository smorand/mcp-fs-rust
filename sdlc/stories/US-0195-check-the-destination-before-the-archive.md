# US-0195: check the destination before the archive is read

> Spec: SPEC-0086 (sdlc/specs/SPEC-0086-refuse-destination-outside-the-project/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0194
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
For zip, 7z, RAR and the tar family, a destination outside the project or holding a NUL character is refused right after the checks on the archive's name, before the archive is read: the member learns the real mistake first and nothing is decoded. SPEC-0085's test where the declared limit beat a refused destination changes accordingly.

## Requirements covered
- SPEC-0086/FR-MOD-002 (partial: the move before reading and the refusals after it): IF several refusals apply to the extraction of a zip, 7z, RAR, plain tar or compressed tar archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) the extension is not supported, including `.rar` on a server without RAR support (`ERR_NOT_SUPPORTED`, `unsupported archive extension: '<extension as typed>'`); (5) a password is given for a plain or compressed tar archive (`ERR_INVALID_ARGUMENT`, `password is not applicable to this archive format`); (6) a multi volume RAR name (`ERR_NOT_SUPPORTED`, `multi volume RAR archives are not supported: extract a single file archive`); (7) the destination holds a NUL character (FR-NEW-004); (8) the destination is outside the project (FR-MOD-001); (9) every later refusal in the order its own specification gives, the destination refusal of that order removed.
- SPEC-0086/FR-NEW-003 (partial: nothing read for a refused destination): THE archive extraction SHALL NOT read the archive's content, write a file, create a folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused under FR-MOD-001 or FR-NEW-004.
- SPEC-0086/FR-NEW-004: IF the destination holds a NUL character THEN THE archive extraction SHALL refuse with `ERR_PATH_OUT_OF_BOUNDS` and the message `path contains a NUL byte`, at the place FR-MOD-002 gives to the destination refusals.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0086_019 e2e_0086_020 e2e_0086_021 e2e_0086_022 e2e_0086_023 e2e_0086_024 e2e_0086_025 e2e_0086_026 e2e_0086_030 e2e_0086_036` and `cargo test -p mcp-fs-core --lib --features rar -- e2e_0086_mod_001`: 11 tests pass. Real assertion: `allowance(f)` still 1 000 and `held()` 0 for the tar bomb of E2E-020.

## Design excerpt
### Decisions
- **DEC-002 (SPEC-0086):** for zip, 7z, RAR, plain and compressed tar the destination is resolved right after the multi volume check and before `client.read_bytes` (`archive.rs:178-205`); `planned_destination` (SPEC-0084 DEC-003) is removed and the bound and the declared check use the resolved destination with its stored files always counted; the bare formats keep their place (`archive_single.rs:241-248`). **Obliged by:** SPEC-0086/FR-MOD-002, FR-NEW-003, FR-NEW-004. **Rationale:** after this move a refused destination never reaches decoding, so "stored files count for nothing when refused" (SPEC-0084 FR-NEW-002, SPEC-0085 FR-NEW-001) holds trivially. **Alternatives considered:** keeping the helper (dead branch). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:143-227`.
- **DEC-004 (SPEC-0086):** US-0194 applies the strict rule at today's place (after decoding for the multi formats), US-0195 moves it before reading: the order tests of US-0195 are red against US-0194 and green after, and SPEC-0085 E2E-028 changes in the story that changes its outcome. **Obliged by:** n/a (no code impact): story planning. **Rationale:** each story ships red evidence for the behavior it adds. **Alternatives considered:** one story (16 plus tests, over the tier 2 budget). **Source:** design. **Evidence:** n/a.
- **DEC-005 (SPEC-0086):** tests in `crates/core/src/tools/archive_destination_e2e.rs`; RAR ones (E2E-004, E2E-017, E2E-029) in a nested `#[cfg(feature = "rar")] mod archive_rar` (SPEC-0085 DEC-011, `test.sh:7`); E2E-004's legs that need no RAR are not split out (the spec writes one test). **Obliged by:** n/a (no code impact): test layout. **Rationale:** as SPEC-0085 DEC-011. **Source:** SPEC-0085 DEC-011. **Evidence:** `test.sh:7`.
### Data model
None.
### Contracts
`archive.rs` `extract_archive`: right after the multi volume check (`:176-185`) and before `client.read_bytes` (`:205`): `let dest = match destination { Some(d) => archive_single::resolve_destination(d)?, None => normalized[..normalized.len() - suffix_len].to_string() };` for every non single format; the later destination computation and `planned_destination` are removed; the bound (SPEC-0084) and `check_declared` (SPEC-0085) receive `&dest` with stored files always counted. Existing test replaced in `archive_declared_e2e.rs` (nested `archive_rar`): `e2e_0085_028_the_limit_wins_over_a_refused_destin` is removed and `e2e_0086_mod_001_the_destination_wins_over_the_declar` added in its place (same RAR bomb and 1 000 bytes remaining, new expectation `ERR_PATH_OUT_OF_BOUNDS` `path escapes the volume root: ../../x`, neither `/x` nor `/uploads/bomb` exists).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_destination_e2e.rs` | modify |
| `crates/core/src/tools/archive_declared_e2e.rs` | modify (one test replaced) |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). The climb walk to mirror: `ensure_entry_path_safe` (`crates/core/src/tools/archive.rs:455-476`) and `tools::export::ensure_no_escape` (`crates/core/src/tools/export.rs:119-126`). `PosixPath::normpath` (`crates/core/src/util/posix.rs:48-55`). "Nothing is written" reads `exists` on `/x`, `/uploads/pack` and every folder the test names, `quota::used_bytes`, `allowance(f)`, and `extraction_audit_lines`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0086/E2E-019: The destination wins over a damaged archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/broken.zip` holding the 9 bytes `not a zip`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not a message starting `archive is corrupt or not a valid zip file:`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_019_the_destination_wins_over_a_damaged`
- Red before the change because: the zip is read first: the refusal is `archive is corrupt or not a valid zip file: ...`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-020: The destination wins over the limits
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002, FR-NEW-003; **Priority:** High
- **Given** a session allowance of 1 000 bytes and `/uploads/bomb.tar.gz` holding `zeros.bin`, 10 000 000 zero bytes
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_WRITE_QUOTA_EXCEEDED`; nothing is written; the member's used session allowance is still 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_020_the_destination_wins_over_the_limits`
- Red before the change because: the tar bound (SPEC-0084) refuses first with `session write quota of 1000 bytes exceeded`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-021: The destination wins over a link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/link.zip` holding a symbolic link entry `l` pointing to `/etc/passwd`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_NOT_SUPPORTED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_021_the_destination_wins_over_a_link_ent`
- Red before the change because: the zip listing refuses the link first with `ERR_NOT_SUPPORTED`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-022: The destination wins over an escaping entry name
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `/uploads/evil.tar` holding an entry named `../../etc/passwd`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not a message starting `archive entry path escapes the destination:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_022_the_destination_wins_over_an_escapin`
- Red before the change because: the tar entry check refuses `../../etc/passwd` first
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-023: The destination wins over a missing password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/secret.7z` encrypted with the password `s3cret`
- **When** the member extracts it with no password and destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_PASSWORD_REQUIRED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_023_the_destination_wins_over_a_missing`
- Red before the change because: the 7z missing password gate refuses first with `ERR_PASSWORD_REQUIRED`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-024: Membership wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** High
- **Given** `eve@example.com`, not a member of `atlas`, and `/uploads/pack.zip`
- **When** eve extracts it with destination `../../x`
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'eve@example.com' is not a member of 'atlas'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_024_membership_wins_over_the_destination`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-025: A missing path wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** nothing at `/uploads/none.zip`
- **When** the member extracts `/uploads/none.zip` with destination `../../x`
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/none.zip' not found`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_025_a_missing_path_wins_over_the_destina`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-026: A folder path wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** a folder `/uploads/dir.zip`
- **When** the member extracts `/uploads/dir.zip` with destination `../../x`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.zip' is a directory, not an archive file`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_026_a_folder_path_wins_over_the_destinat`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-030: A NUL character is refused before the archive is read
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-MOD-002, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/broken.zip` of E2E-019
- **When** the member extracts it with the destination made of `out`, a NUL character, then `x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path contains a NUL byte`, not the corrupt refusal; nothing is written; `/out` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_030_a_nul_character_is_refused_before_th`
- Red before the change because: the NUL refusal of `normalize_path` comes after the zip is read, so the corrupt refusal wins today
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-036: A NUL character in an otherwise valid call
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with the destination made of `out`, a NUL character, then `x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path contains a NUL byte`; nothing is written; `/out` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_036_a_nul_character_in_an_otherwise_vali`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-MOD-001: The destination wins over the declared limit, RAR (was SPEC-0085 E2E-028)
- **Validated before:** with 1 000 bytes of remaining storage and the RAR bomb, destination `../../x` answered `ERR_PROJECT_QUOTA_EXCEEDED`.
- **Validates now:** the destination refusal comes first (FR-MOD-002 item 8).
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002, FR-NEW-003; **Priority:** High
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar` holding `zeros.bin`, 10 000 000 zero bytes
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`, not `ERR_PROJECT_QUOTA_EXCEEDED`; neither `/x` nor `/uploads/bomb` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `archive_rar`, replaces `e2e_0085_028_the_limit_wins_over_a_refused_destin`)
- Function: `e2e_0086_mod_001_the_destination_wins_over_the_declar`
- Red before the change because: the declared check of SPEC-0085 runs before the destination refusal: the answer is `ERR_PROJECT_QUOTA_EXCEEDED` (the old test asserted exactly that)
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-024 | make `state.authorize` accept a non member: the destination refusal comes instead of `ERR_FORBIDDEN`, the test fails |
| E2E-025 | resolve the destination before the node lookup (`archive.rs:154-160`): `ERR_PATH_OUT_OF_BOUNDS` replaces `ERR_NOT_FOUND`, the test fails |
| E2E-026 | resolve the destination before the folder check: `ERR_PATH_OUT_OF_BOUNDS` replaces the directory refusal, the test fails |
| E2E-036 | drop the NUL check from `resolve_destination`: the destination resolves to a path holding a NUL character and the answer is no longer `path contains a NUL byte`, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Bare `.gz`, `.bz2`, `.xz` keep their order (destination after the output name, `archive_single.rs:241-248`)
- The removed test is exactly `e2e_0085_028_the_limit_wins_over_a_refused_destin`; no other existing test changes

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-019, E2E-020, E2E-021, E2E-022, E2E-023, E2E-030, E2E-MOD-001), before the production code.
- [ ] Green on arrival tests (E2E-024, E2E-025, E2E-026, E2E-036): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_0080_061_destination_outside_the_project` (`archive_rar.rs:1283-1301`), the `../../x` tests of SPEC-0081 E2E-035, SPEC-0082 E2E-022 and SPEC-0083 E2E-032 (`archive_single.rs:1096-1097`, `:1533-1534`, `:2271-2272`), the path pinning tests `safety.rs:263-265` and `util/posix.rs:115-118`, every `e2e_0084_` and `e2e_0085_` test except the one this spec replaces; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0195: check the destination before the archive is read`, body `Spec: SPEC-0086` and the requirement ids (SPEC-0086/FR-MOD-002, FR-NEW-003, FR-NEW-004).
