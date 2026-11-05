# US-0164: pin the gzip order of refusals and document it

> Spec: SPEC-0081 (sdlc/specs/SPEC-0081-extract-gz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0163
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
The order of refusals of FR-NEW-011 is pinned by tests, each proven to fail when the order is broken, and the documentation lists the bare gzip format. This closes SPEC-0081: its full acceptance set runs green.

## Requirements covered
- SPEC-0081/FR-NEW-011: IF several refusals apply to the same call THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`); (2) the path does not exist (`ERR_NOT_FOUND`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`); (4) a password is given (FR-NEW-009); (5) the output name is unusable (FR-NEW-003); (6) the destination lies outside the project (`ERR_PATH_OUT_OF_BOUNDS`, message `path escapes the volume root: <destination>`); (7) the destination is a file or sits under a file (FR-NEW-013); (8) collision (FR-NEW-005); (9) corrupt (FR-NEW-008), project storage limit (FR-NEW-006) or session write allowance (FR-NEW-012), whichever decoding meets first, a tie going to the project storage limit.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0081_014 e2e_0081_015 e2e_0081_016 e2e_0081_019 e2e_0081_020 e2e_0081_021 e2e_0081_022 e2e_0081_035 e2e_0081_045`: all 9 tests pass. `rg -n 'bare gzip' README.md AGENTS.md .agent_docs/tools.md` prints one hit per file.

## Design excerpt
### Decisions
- **DEC-007:** order inside `extract_single`: password (already in the caller), output name, destination normalize, destination is a file, collision, caps, capped decode, admission, write. No byte is written and no folder is created before the decode succeeds. **Obliged by:** SPEC-0081/FR-NEW-011, FR-NEW-005, FR-NEW-013. **Rationale:** it is the spec's list, items 4 to 9. **Alternatives considered:** none, the spec fixes the order. **Source:** spec. **Evidence:** spec Section 6, FR-NEW-011.
- **DEC-012:** the stories are cut by capability so each one turns its own tests red to green: US-0160 builds the flow with `GzDecoder`, an unbounded read and a strict rule (anything at the output path is refused), US-0161 switches to `MultiGzDecoder`, US-0162 adds the password, destination and collision rules before the decode, US-0163 adds the caps, US-0164 pins the order of refusals with mutation evidence, writes the documentation and closes. Tests that characterize behavior already built (membership, missing path, folder, tar suffixes) are recorded green on arrival with that reason. **Obliged by:** n/a (no code impact): story planning. **Rationale:** vertical stories with real red evidence. **Alternatives considered:** one story of 57 tests. **Source:** design. **Evidence:** n/a.
- **DEC-013:** documentation: formats list in `README.md:294-295`, structure entry in `AGENTS.md:130-131`, `.agent_docs/tools.md:178-183`; no tool contract change. **Obliged by:** n/a (no code impact): project convention (keep README and AGENTS.md up to date). **Rationale:** AGENTS.md rule. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `AGENTS.md:130`.
### Data model
None.
### Contracts
No new code contract. Documentation: `README.md:294-295` formats sentence gains `.gz` (a bare gzip file decompresses next to the archive); `AGENTS.md:130-131` gains `archive_single.rs` (single file flow, two limits, capped reader); `.agent_docs/tools.md:178-183` gains the bare file rules and the order of refusals; `.agent_docs/testing.md` gains one line about the `e2e_0081_` filter and the capped reader hook.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
The order list of FR-NEW-011; the docs sections named above. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0081/E2E-014: Missing archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Low
- **Given** no file at `/uploads/ghost.gz`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_FOUND` with the message `'/uploads/ghost.gz' not found`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_014_missing_archive`
- Green on arrival: shared checks unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-015: Folder named like an archive
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** a folder `/uploads/dir.gz`
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `'/uploads/dir.gz' is a directory, not an archive file`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_015_folder_named_like_an_archive`
- Green on arrival: shared checks unchanged
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-016: Non member
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/uploads/data.csv.gz` in project `alpha` and `bob@example.com`, who is not a member of `alpha`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with the message `'bob@example.com' is not a member of 'alpha'`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_016_non_member`
- Green on arrival: membership is checked by the door before the engine
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-019: Membership wins over a missing path
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** project `alpha` without `/uploads/ghost.gz` and `bob@example.com`, not a member
- **When** `bob@example.com` extracts `/uploads/ghost.gz`
- **Then** the refusal is `ERR_FORBIDDEN`, not `ERR_NOT_FOUND`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_019_membership_wins_over_a_missing_path`
- Green on arrival: membership is checked by the door before the engine
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-020: Password wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/fake.gz` containing the text `not gzip`
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_020_password_wins_over_a_corrupt_file`
- Green on arrival: password is refused in the caller (US-0162)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-021: Collision wins over a corrupt file
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/cut.gz` truncated and an existing `/uploads/cut` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not the corrupt refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_021_collision_wins_over_a_corrupt_file`
- Green on arrival: the collision check precedes the decode since US-0162
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-022: Collision wins over the storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011, FR-NEW-006; **Priority:** Medium
- **Given** the situation of E2E-007 and an existing `/uploads/data.csv` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER`, not `ERR_PROJECT_QUOTA_EXCEEDED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_022_collision_wins_over_the_storage_limi`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-035: Destination outside the project
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/uploads/data.csv.gz`
- **When** the member extracts it with the destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_035_destination_outside_the_project`
- Green on arrival: the destination is normalized before the decode since US-0162
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-045: Storage limit met before the corruption
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-011, FR-NEW-006; **Priority:** Medium
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/big.gz`, a gzip stream decompressing to 1 000 bytes and then truncated before its end
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, because the limit is met first
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_045_storage_limit_met_before_the_corrupt`
- Green on arrival: the cap stops the decode before the corruption since US-0163
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-014 | regression only, no mutation |
| E2E-015 | regression only, no mutation |
| E2E-016 | regression only: remove the `authorize` call of the test helper and the test fails |
| E2E-019 | same as E2E-016 |
| E2E-020 | move the password check after the decode: the test fails |
| E2E-021 | move `check_collision` after the decode: the test fails |
| E2E-022 | move `check_collision` after the caps: the test fails |
| E2E-035 | normalize the destination after the decode: the test fails |
| E2E-045 | remove the cap: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Documentation without dashes as punctuation (project convention)
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-014, E2E-015, E2E-016, E2E-019, E2E-020, E2E-021, E2E-022, E2E-035, E2E-045): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md:294-295`, `AGENTS.md:130-131`, `.agent_docs/tools.md:178-183`, `.agent_docs/testing.md` (the capped reader hook).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0081 is green (`cargo test -p mcp-fs-core --lib -- e2e_0081_`), and every requirement of SPEC-0081 is cited by at least one story with status done or landed.
- [ ] One commit `US-0164: pin the gzip order of refusals and document it`, body `Spec: SPEC-0081` and the requirement ids.
