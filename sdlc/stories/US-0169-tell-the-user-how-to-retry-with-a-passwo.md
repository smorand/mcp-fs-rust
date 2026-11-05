# US-0169: tell the user how to retry with a password for zip and 7z

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
The two password refusals of zip and 7z end with the instruction to call again with the `password` parameter, so the user knows what to type. One shared wording will serve RAR as well.

## Requirements covered
- SPEC-0080/FR-MOD-002: IF a zip or 7z archive needs a password and none is given THEN THE archive extraction SHALL refuse with the FR-NEW-002 message.
- SPEC-0080/FR-MOD-003: IF the password given for a zip or 7z archive is wrong THEN THE archive extraction SHALL refuse with the FR-NEW-003 message.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_045 e2e_0080_046 e2e_0080_mod_002 e2e_0080_mod_003 e2e_0080_mod_004 e2e_0080_mod_005`: all 6 tests pass. The tests read the message of the error returned by a real encrypted zip and 7z built by the existing helpers (`build_zip`, `build_zipcrypto`, the 7z builders).

## Design excerpt
### Decisions
- **DEC-007:** zip and 7z get the new password wording by extending `password_required_message` (`archive.rs:615-620`) and routing the three literals (`:441`, `:489`, `:531`) through it; the existing tests that pin the old text (`:1034`, `:1047`, `:2158`, `:2174`) and SPEC-0041's story tests are updated in the same story. **Obliged by:** SPEC-0080/FR-MOD-002, FR-MOD-003. **Rationale:** one wording for three formats. **Alternatives considered:** reword RAR only (leaves zip and 7z less helpful, rejected in the spec). **Source:** spec. **Evidence:** `archive.rs:441`, `:489`, `:531`, `:615-620`.
### Data model
None.
### Contracts
`password_required_message(password: Option<&str>) -> ToolError` (`archive.rs:615-620`): `None` gives `password required to extract this archive; call again with the 'password' parameter`; `Some(_)` gives `incorrect password for this archive; call again with the correct 'password' parameter`; code `ERR_PASSWORD_REQUIRED`. The literals at `archive.rs:441`, `:489`, `:531` call it. Existing assertions to update: `:1034`, `:1047` (prefix checks stay valid), `:2158` and `:2174` (exact equality becomes the new text), and the SPEC-0041 story US-0048 tests that quote the old text.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
### Existing patterns
`password_required_message` (`archive.rs:615-620`), `ToolError::password_required` (`errors.rs:98`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-045: Zip missing password through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-002
- **Given** an encrypted `/uploads/pack.zip`
- **When** a client asks the HTTP API to extract it with no password
- **Then** the error code is `ERR_PASSWORD_REQUIRED` and the message is `password required to extract this archive; call again with the 'password' parameter`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_045_zip_missing_password_through_the_htt`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-046: 7z wrong password through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-003
- **Given** an encrypted `/uploads/pack.7z`
- **When** a client asks the HTTP API to extract it with the password `nope`
- **Then** the error code is `ERR_PASSWORD_REQUIRED` and the message is `incorrect password for this archive; call again with the correct 'password' parameter`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0080_046_7z_wrong_password_through_the_http_a`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-MOD-002: 7z missing password message (was the 7z test pinning `password required to extract this archive`)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Validated before:** exact message `password required to extract this archive`.
- **Validates now:** exact message of FR-NEW-002.
- **Given** an encrypted 7z; **When** extracted with no password; **Then** `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`.
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive.rs tests (existing test updated)
- Function: `e2e_0080_mod_002_7z_missing_password_message_was_the`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-MOD-003: zip missing password message (was SPEC-0018 E2E-NEW-006)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Validated before:** message contains `password required`.
- **Validates now:** exact message of FR-NEW-002.
- **Given** an encrypted zip; **When** extracted with no password; **Then** the exact message of FR-NEW-002.
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive.rs tests (existing test updated)
- Function: `e2e_0080_mod_003_zip_missing_password_message_was_spe`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-MOD-004: zip wrong password message (was SPEC-0018 E2E-NEW-057)
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-003; **Priority:** Medium
- **Validated before:** message contains `incorrect password`.
- **Validates now:** exact message of FR-NEW-003.
- **Given** an encrypted zip; **When** extracted with `nope`; **Then** the exact message of FR-NEW-003.
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive.rs tests (existing test updated)
- Function: `e2e_0080_mod_004_zip_wrong_password_message_was_spec`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-MOD-005: 7z wrong password message
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-003; **Priority:** Medium
- **Validated before:** exact message `incorrect password for this archive`.
- **Validates now:** exact message of FR-NEW-003.
- **Given** an encrypted 7z; **When** extracted with `nope`; **Then** `ERR_PASSWORD_REQUIRED` with `incorrect password for this archive; call again with the correct 'password' parameter`.
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive.rs tests (existing test updated)
- Function: `e2e_0080_mod_005_7z_wrong_password_message`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- Keep the prefixes `password required` and `incorrect password`
- Scope boundary: RAR belongs to US-0170 and later

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Existing tests updated in this story: `archive.rs:2158`, `:2174` (exact text), `:1034`, `:1047` (assert the full new text), and the tests quoting the old text found by `rg -n 'password required to extract this archive|incorrect password for this archive' crates sdlc/stories`.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0169: tell the user how to retry with a password for zip and 7z`, body `Spec: SPEC-0080` and the requirement ids.
