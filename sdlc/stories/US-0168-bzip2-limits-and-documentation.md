# US-0168: check the bzip2 storage and session limits and document the format

> Spec: SPEC-0083 (sdlc/specs/SPEC-0083-extract-bz2-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0167, US-0163
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
A bzip2 decompression bomb is stopped at the lower of the two limits, with the same refusals as gzip, and the documentation lists the bare bzip2 format. This closes SPEC-0083.

## Requirements covered
- SPEC-0083/FR-NEW-005: IF the project has a storage limit AND the net growth of the project's stored bytes caused by the output exceeds the remaining storage THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- SPEC-0083/FR-NEW-006: IF the output exceeds the member's remaining session write allowance THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0083_011 e2e_0083_012 e2e_0083_013 e2e_0083_040 e2e_0083_041 e2e_0083_042 e2e_0083_049 e2e_0083_053`: all 8 tests pass. The bomb test asserts `high_water <= cap + 65 536`; the bzip2 reader streams, so the bound is real.

## Design excerpt
### Decisions
- **DEC-001:** bzip2 reuses the single file flow and rules of SPEC-0081 (`extract_single`, `read_capped`, caps, admission, order); its stories depend on the SPEC-0081 stories that build each rule. **Obliged by:** SPEC-0083/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine for gzip, bzip2 and xz. **Alternatives considered:** a parallel flow. **Source:** SPEC-0081 DEC-001, DEC-005, DEC-007 (existing design binds). **Evidence:** `sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-design.md`.
- **DEC-005:** documentation as DEC-013 of SPEC-0081, formats list gains `.bz2`. **Obliged by:** n/a (no code impact): project convention. **Rationale:** AGENTS.md. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `AGENTS.md:130`.
### Data model
None.
### Contracts
No code contract: `read_capped`, `caps` and `admit_paths` are SPEC-0081's. Documentation: `README.md:294-295` formats gain `.bz2`; `AGENTS.md:130-131` and `.agent_docs/tools.md:178-183` mention bare `.bz2` next to `.gz`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
### Existing patterns
SPEC-0081 story US-0163 tests (`e2e_0081_` bomb and limit tests) as the model. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0083/E2E-011: Output over the project storage limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/data.csv.bz2` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/data.csv` does not exist and the remaining storage is still 10 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_011_output_over_the_project_storage_limi`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-012: Decompression bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Critical
- **Given** a project storage limit with 1 000 bytes of remaining storage and `/uploads/bomb.bz2`, a bzip2 file of under 1 KB that decompresses to 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, and the answer arrives without the server holding the 10 000 000 bytes (the server's memory use during the call stays below 1 000 000 bytes above its level before the call)
- **And** `/uploads/bomb` does not exist and the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_012_decompression_bomb_against_the_stora`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-013: Decompression bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project without a storage limit, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.bz2` as in E2E-012
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`, reached without the server holding the 10 000 000 bytes (memory as in E2E-012)
- **And** `/uploads/bomb` does not exist and the allowance is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_013_decompression_bomb_against_the_sessi`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-040: Output exactly equal to the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and `/uploads/data.csv.bz2` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the remaining storage is 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_040_output_exactly_equal_to_the_remainin`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-041: Overwrite counts net growth
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/data.csv.bz2` (16 bytes decoded), an existing `/uploads/data.csv` of 40 bytes, and a project storage limit with 5 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds, `/uploads/data.csv` holds the 16 bytes and the remaining storage is 29 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_041_overwrite_counts_net_growth`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-042: The allowance is charged the full output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, an existing `/uploads/data.csv` of 40 bytes and `/uploads/data.csv.bz2` decompressing to 16 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds and the allowance has 4 bytes remaining (16 charged, nothing netted)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_042_the_allowance_is_charged_the_full_ou`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-049: Both limits at the same byte
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006, FR-NEW-012; **Priority:** Medium
- **Given** a project storage limit with 1 000 bytes of remaining storage, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.bz2` as in E2E-012
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_049_both_limits_at_the_same_byte`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0083/E2E-053: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a session write allowance with 10 bytes remaining and `/uploads/data.csv.bz2` decompressing to 16 bytes
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 7 bytes remaining
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0083_053_a_refused_extraction_charges_nothing`
- Green on arrival: the rule is built by earlier stories; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-011 | accept a net growth one byte over, or drop the project cap: the test fails |
| E2E-012 | accept a net growth one byte over, or drop the project cap: the test fails |
| E2E-013 | drop the session cap or charge nothing: the test fails |
| E2E-040 | accept a net growth one byte over, or drop the project cap: the test fails |
| E2E-041 | accept a net growth one byte over, or drop the project cap: the test fails |
| E2E-042 | drop the session cap or charge nothing: the test fails |
| E2E-049 | accept a net growth one byte over, or drop the project cap: the test fails |
| E2E-053 | drop the session cap or charge nothing: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Documentation without dashes as punctuation
- Scope boundary: nothing else

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-011, E2E-012, E2E-013, E2E-040, E2E-041, E2E-042, E2E-049, E2E-053): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md:294-295`, `AGENTS.md:130-131`, `.agent_docs/tools.md:178-183`.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0083 is green (`cargo test -p mcp-fs-core --lib -- e2e_0083_`), and every requirement of SPEC-0083 is cited by at least one story with status done or landed.
- [ ] One commit `US-0168: check the bzip2 storage and session limits and document the format`, body `Spec: SPEC-0083` and the requirement ids.
