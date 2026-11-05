# US-0163: stop gzip decoding at the storage and session limits

> Spec: SPEC-0081 (sdlc/specs/SPEC-0081-extract-gz-single-file/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0162
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A decompression bomb is stopped as soon as its output exceeds the lower of the project's remaining storage and the member's remaining session allowance. The refusal names the limit that was met first, the project winning a tie, and nothing is written.

## Requirements covered
- SPEC-0081/FR-NEW-006: IF the project has a storage limit AND the net growth of the project's stored bytes caused by the output exceeds the remaining storage THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and a message starting `project storage quota of`, leave the remaining storage unchanged and write nothing.
- SPEC-0081/FR-NEW-012: IF the output exceeds the member's remaining session write allowance THEN THE archive extraction SHALL stop decoding at that point, refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`, charge nothing and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0081_007 e2e_0081_017 e2e_0081_025 e2e_0081_032 e2e_0081_047 e2e_0081_048 e2e_0081_049 e2e_0081_050 e2e_0081_051 e2e_0081_052 e2e_0081_053`: all 11 tests pass. The bomb test asserts `high_water <= cap + 65 536` on the returned `CappedRead`/`CapExceeded` stats, which is a measurement of the decoder, not of the story's own answer.

## Design excerpt
### Decisions
- **DEC-005:** decoding is `read_capped(reader, cap)`: read 65 536 byte chunks into one `Vec`, stop with `CapExceeded` as soon as the length exceeds `cap`; a length equal to `cap` is accepted. `cap = min(project_cap, session_cap)` where `project_cap = remaining storage + size of the file replaced under overwrite` (only when the project has a limit) and `session_cap = SafetyManager::session_remaining`; on `CapExceeded` the refusal is the project one when `project_cap <= session_cap` (tie goes to the project), else the session one. **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** `quota::check_and_charge` refuses when `used + delta > limit` (`quota.rs:96-113`), which is `new <= limit - used + old`. **Alternatives considered:** decode fully then check (unbounded memory); two passes. **Source:** design. **Evidence:** `crates/core/src/quota.rs:96-113`, `safety.rs:72-81`, `safety.rs:136-147`.
- **DEC-006:** `SafetyManager::session_remaining(person, project) -> i64` is added (read only, `with_session`); `quota.rs` gains `project_remaining_for` reading `safety.project_limit` and `client.meta.used_bytes()`. **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** both numbers are private to those modules today. **Alternatives considered:** make `with_session` public. **Source:** existing code. **Evidence:** `safety.rs:82-92`, `quota.rs:96-113`.
- **DEC-009:** after a successful capped decode the write goes through `quota::admit_paths(client, safety, person, mount_id, len, &[(out, len)])` and the returned `Admission` lives until the write lands, as the archive flow does (`archive.rs:183-205`). **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** project check on net growth first, session charged gross only after it passed, one admission lock. **Alternatives considered:** a second admission code path. **Source:** existing code. **Evidence:** `quota.rs:136-158`.
- **DEC-012:** the stories are cut by capability so each one turns its own tests red to green: US-0160 builds the flow with `GzDecoder`, an unbounded read and a strict rule (anything at the output path is refused), US-0161 switches to `MultiGzDecoder`, US-0162 adds the password, destination and collision rules before the decode, US-0163 adds the caps, US-0164 pins the order of refusals with mutation evidence, writes the documentation and closes. Tests that characterize behavior already built (membership, missing path, folder, tar suffixes) are recorded green on arrival with that reason. **Obliged by:** n/a (no code impact): story planning. **Rationale:** vertical stories with real red evidence. **Alternatives considered:** one story of 57 tests. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
`SafetyManager::session_remaining(&self, person: &str, project: &str) -> i64` (never negative). `quota::project_remaining_for(client: &VolumeClient, safety: &SafetyManager, replaced_size: i64) -> Result<Option<i64>>` (`None` when the project has no limit; otherwise `limit - used + replaced_size`, floor 0). `caps(...) -> Caps { limit: u64, which: CapKind }` where `CapKind` is `Project` or `Session`, `Project` when `project_cap <= session_cap`. `read_capped(reader, cap)` now stops with `CapExceeded` once the length exceeds `cap`; equal is accepted; `high_water <= cap + 65 536`. Refusals: `ToolError::project_quota_exceeded(limit, used, requested)` and `ToolError::write_quota_exceeded("session write quota of <N> bytes exceeded")`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/safety.rs` | modify |
| `crates/core/src/quota.rs` | modify |
### Existing patterns
`SafetyManager::project_limit` (`safety.rs:72-81`), `charge_write` (`safety.rs:136-147`), `check_and_charge` (`quota.rs:96-113`), `with_session` (`safety.rs:82-92`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0081/E2E-007: Output over the remaining quota
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project storage limit with 10 bytes of remaining storage and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/data.csv` does not exist and the remaining storage is still 10 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_007_output_over_the_remaining_quota`
- Green on arrival: the admission after the decode (US-0160) already refuses with the project code
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-017: Decompression bomb
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a project storage limit with 1 000 bytes of remaining storage and `/uploads/bomb.gz`, a gzip file of about 10 KB that decompresses to 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, and the answer arrives without the server holding the 10 000 000 bytes (the server's memory use during the call stays below 1 000 000 bytes above its level before the call)
- **And** `/uploads/bomb` does not exist and the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_017_decompression_bomb`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-025: Output exactly equal to the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the remaining storage is 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_025_output_exactly_equal_to_the_remainin`
- Green on arrival: same, equality is accepted by the admission
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-032: A refused extraction leaves the project usable
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** the situation of E2E-007
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the remaining storage is 7 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_032_a_refused_extraction_leaves_the_proj`
- Green on arrival: same, a refused call charges nothing
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-047: One byte over the remaining storage
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a project storage limit with 16 bytes of remaining storage and a gzip file decompressing to 17 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` and the remaining storage is still 16 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_047_one_byte_over_the_remaining_storage`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-048: Bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Critical
- **Given** a project without a storage limit, a session write allowance with 1 000 bytes remaining, and `/uploads/bomb.gz` as in E2E-017
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with a message starting `session write quota of`, reached without the server holding the 10 000 000 bytes (memory as in E2E-017)
- **And** `/uploads/bomb` does not exist and the allowance is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_048_bomb_against_the_session_allowance`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-049: The allowance is charged the full output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a session write allowance with 20 bytes remaining, an existing `/uploads/data.csv` of 40 bytes and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds and the allowance has 4 bytes remaining (16 charged, nothing netted)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_049_the_allowance_is_charged_the_full_ou`
- Green on arrival: the admission charges the gross size
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-050: A refused extraction charges nothing
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a session write allowance with 10 bytes remaining and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the extraction is refused, then the member writes a 3 byte file `/uploads/small.txt`
- **Then** the small write succeeds and the allowance has 7 bytes remaining
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_050_a_refused_extraction_charges_nothing`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-051: Both limits at the same byte
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-012, FR-NEW-006; **Priority:** Medium
- **Given** 10 bytes remaining in the project storage and 10 bytes remaining in the session allowance, and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_051_both_limits_at_the_same_byte`
- Green on arrival: the admission checks the project first
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-052: The lower limit is met first
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** 1 000 bytes remaining in the project storage, 100 bytes remaining in the session allowance, and `/uploads/bomb.gz`
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_052_the_lower_limit_is_met_first`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0081/E2E-053: Allowance equal to the output
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** a session write allowance with 16 bytes remaining and `/uploads/data.csv.gz` decompressing to 16 bytes
- **When** the member extracts it
- **Then** the file is written and the allowance has 0 bytes remaining
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_single.rs tests
- Function: `e2e_0081_053_allowance_equal_to_the_output`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-007 | accept a net growth one byte over: the test fails |
| E2E-025 | refuse at equality: the test fails |
| E2E-032 | charge the session on a refused call: the test fails |
| E2E-047 | accept one byte over: the test fails |
| E2E-049 | net the replaced size in the session charge: the test fails |
| E2E-050 | charge the session before the project check: the test fails |
| E2E-051 | check the session cap first on a tie: the test fails |
| E2E-053 | refuse at equality of the allowance: the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change); `archive.rs`
- Dependencies not to add: none
- The admission after the decode (`admit_paths`) stays as it is
- Scope boundary: order pins and docs belong to US-0164

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-007, E2E-025, E2E-032, E2E-047, E2E-049, E2E-050, E2E-051, E2E-053): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0163: stop gzip decoding at the storage and session limits`, body `Spec: SPEC-0081` and the requirement ids.
