# US-0172: apply the storage limit and the session allowance to RAR archives

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0171
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
A RAR whose declared total exceeds the project's remaining storage or the member's remaining session allowance is refused with the right code and nothing is charged or written; equality is accepted.

## Requirements covered
- SPEC-0080/FR-NEW-014: IF the project has a storage limit AND the net growth of the project's stored bytes caused by the extraction exceeds the remaining storage THEN THE archive extraction SHALL refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- SPEC-0080/FR-NEW-022: IF the declared total of the files to write exceeds the member's remaining session write allowance THEN THE archive extraction SHALL refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_011 e2e_0080_038 e2e_0080_041 e2e_0080_073 e2e_0080_077 e2e_0080_078 e2e_0080_079 e2e_0080_080`: all 8 tests pass. Remaining storage and allowance are read back after each call.

## Design excerpt
### Decisions
- **DEC-008:** the shared pipeline stays as is: collision scan, `admit_paths` (project net growth first, session gross after), writes. No pre decode admission. **Obliged by:** SPEC-0080/FR-NEW-009, FR-NEW-014, FR-NEW-022. **Rationale:** the refusal outcomes are the same; admitting before decoding is a hardening outside this spec (BL-0125). **Alternatives considered:** pre decode admission on declared sizes. **Source:** existing code. **Evidence:** `archive.rs:183-205`, `quota.rs:96-158`.
### Data model
None.
### Contracts
No new code contract: the shared pipeline `quota::admit_paths(client, safety, person, mount_id, total_declared, &entry_targets)` (`archive.rs:183-205`) already checks the project on net growth and charges the session gross after it. This story adds the RAR tests and a code change only if a test shows a gap.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
### Existing patterns
`e2e_new_020_archive_total_declared_size_exceeds_remaining_quota` and `e2e_new_021_...` (`archive.rs:1588`, `:1610`), `quota.rs:96-158`. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-011: Over the project storage limit
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-014
- **Given** a project storage limit with 5 bytes of remaining storage and `/uploads/report.rar` declaring 12 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`; the remaining storage is still 5 bytes and `/uploads/report` does not exist
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_011_over_the_project_storage_limit`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-038: Storage exactly equal to the declared total
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-014
- **Given** a project storage limit with 12 bytes of remaining storage and `/uploads/report.rar` of E2E-001
- **When** the member extracts it
- **Then** 2 files are written and the remaining storage is 0 bytes
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_038_storage_exactly_equal_to_the_declare`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-041: A refused storage limit leaves the project usable
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-014
- **Given** the situation of E2E-011
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the remaining storage is 2 bytes
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_041_a_refused_storage_limit_leaves_the_p`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-073: Overwrite counts net growth against the storage limit
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** a project storage limit with 5 bytes of remaining storage, `/uploads/report.rar` as in E2E-001 (12 declared bytes), and existing `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` totalling 20 bytes
- **When** the member extracts it with overwrite true
- **Then** 2 files are written and the remaining storage is 13 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_073_overwrite_counts_net_growth_against`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-077: Over the session write allowance
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** High
- **Given** a project without a storage limit, a session write allowance with 5 bytes remaining, and `/uploads/report.rar` declaring 12 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`; `/uploads/report` does not exist and the allowance is still 5 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_077_over_the_session_write_allowance`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-078: The allowance is charged the full declared total
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, existing `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` totalling 20 bytes, and `/uploads/report.rar` as in E2E-001 (12 declared bytes)
- **When** the member extracts it with overwrite true
- **Then** 2 files are written and the allowance has 8 bytes remaining (12 charged, nothing netted)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_078_the_allowance_is_charged_the_full_de`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-079: Allowance equal to the declared total
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** Medium
- **Given** a session write allowance with 12 bytes remaining and `/uploads/report.rar` as in E2E-001
- **When** the member extracts it
- **Then** 2 files are written and the allowance has 0 bytes remaining
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_079_allowance_equal_to_the_declared_tota`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-080: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-022; **Priority:** Medium
- **Given** the situation of E2E-077
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 2 bytes remaining
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_080_a_refused_extraction_charges_nothing`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-011 | skip the project check in the admission: the test fails |
| E2E-038 | skip the project check in the admission: the test fails |
| E2E-041 | skip the project check in the admission: the test fails |
| E2E-073 | skip the project check in the admission: the test fails |
| E2E-077 | charge nothing to the session: the test fails |
| E2E-078 | charge nothing to the session: the test fails |
| E2E-079 | charge nothing to the session: the test fails |
| E2E-080 | charge nothing to the session: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-011, E2E-038, E2E-041, E2E-073, E2E-077, E2E-078, E2E-079, E2E-080): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0172: apply the storage limit and the session allowance to RAR archives`, body `Spec: SPEC-0080` and the requirement ids.
