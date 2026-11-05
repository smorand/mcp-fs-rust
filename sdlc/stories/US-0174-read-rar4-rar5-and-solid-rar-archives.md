# US-0174: read RAR4, RAR5 and solid RAR archives

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0170
> min_tier: 2
> Files touched: 5
> Closes spec: no
> Blocked: user provided fixtures missing today (see Constraints)

## Objective
RAR version 4, version 5 and solid archives extract, and a wrong password on a RAR4 archive is the wrong password refusal, not a corrupt one. The fixtures need the RARLAB `rar` tool: this story is blocked until they exist.

## Requirements covered
- SPEC-0080/FR-NEW-001: WHEN a member asks to extract a file whose name ends in `.rar`, in any letter case, THE archive extraction SHALL extract it by the rules that apply to every other supported format: default destination is the archive path without its suffix, `destination` and `overwrite` are honoured, the answer carries the destination, files written, folders created and bytes written, and one audit line is recorded.
- SPEC-0080/FR-NEW-003: IF a RAR archive is encrypted and the given password is wrong THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and the message `incorrect password for this archive; call again with the correct 'password' parameter`, and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_062 e2e_0080_063 e2e_0080_064 e2e_0080_065`: all 4 tests pass. Content is checked by reading the stored files back.

## Design excerpt
### Decisions
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-009:** test data: a test only RAR5 store builder (`archive_rar_fixtures.rs`: signature, main header, file headers with vint fields and CRC32, directory, symlink and hard link redirection records) produces the plain cases; encrypted, RAR4 and solid cases need real fixtures made with `rar` and committed under `crates/core/tests/fixtures/rar/` with a `README` line naming the command that made each. **Obliged by:** SPEC-0080 tests (spec Section 9 says fixtures come from the design). **Rationale:** `rar` is not installed, `7z` cannot create RAR; implementing RAR5 AES in the builder is out of proportion. **Alternatives considered:** the `unrar` crate's own `data/*.rar` (content and passwords differ from the spec's concrete data); a builder with encryption. **Source:** user answer (default taken 2026-11-05). **Evidence:** `which rar` returned nothing.
### Data model
None.
### Contracts
No new code contract. Fixtures (user provided, `rar` not installed here): `v4.rar` (`rar a -ma4 v4.rar a.txt`, `a.txt` = `hello`), `v5.rar` (`rar a -ma5`), `solid.rar` (`rar a -s`, `a.txt` = `hello`, `b.txt` = `goodbye`), `v4secret.rar` (`rar a -ma4 -ps3cret-Ünï v4secret.rar a.txt`); each committed under `crates/core/tests/fixtures/rar/` with its command in a `README` line.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/tests/fixtures/rar/v4.rar` | create |
| `crates/core/tests/fixtures/rar/v5.rar` | create |
| `crates/core/tests/fixtures/rar/solid.rar` | create |
| `crates/core/tests/fixtures/rar/v4secret.rar` | create |
### Existing patterns
Fixture loading with `include_bytes!` from `crates/core/tests/fixtures/` (the directory exists: `schema_sqlite.sql`, `test-remote.pem`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-062: RAR version 4 extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/v4.rar`, created as RAR version 4, holding `a.txt` containing `hello`
- **When** the member extracts it
- **Then** 1 file and 5 bytes are written; `/uploads/v4/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_062_rar_version_4_extracts`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-063: RAR version 5 extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/uploads/v5.rar`, created as RAR version 5, holding `a.txt` containing `hello`
- **When** the member extracts it
- **Then** 1 file and 5 bytes are written; `/uploads/v5/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_063_rar_version_5_extracts`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-064: Solid archive extracts
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** `/uploads/solid.rar`, a solid archive holding `a.txt` containing `hello` and `b.txt` containing `goodbye`
- **When** the member extracts it
- **Then** 2 files and 12 bytes are written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_064_solid_archive_extracts`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-065: Wrong password on a RAR version 4
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/uploads/v4secret.rar`, an encrypted RAR version 4, password `s3cret-Ünï`
- **When** the member extracts it with `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with the message of E2E-007, not a corrupt refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_065_wrong_password_on_a_rar_version_4`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- BLOCKED until the four fixtures exist (data: user provided, missing today); say so in the report instead of faking them
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Fixtures present before the story starts; otherwise stop and report BLOCKED with the missing file names.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0174: read RAR4, RAR5 and solid RAR archives`, body `Spec: SPEC-0080` and the requirement ids.
