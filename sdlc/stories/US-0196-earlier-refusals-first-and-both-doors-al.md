# US-0196: earlier refusals first and both doors alike

> Spec: SPEC-0086 (sdlc/specs/SPEC-0086-refuse-destination-outside-the-project/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0195
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The refusals decided on the archive's name (unsupported extension, a password on a tar archive, a multi volume RAR name) still come before the destination, and the HTTP API gives the same code and message as the assistant tool for climbing, inside and NUL destinations.

## Requirements covered
- SPEC-0086/FR-MOD-002 (partial: items 4 to 6 before the destination): IF several refusals apply to the extraction of a zip, 7z, RAR, plain tar or compressed tar archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) the extension is not supported, including `.rar` on a server without RAR support (`ERR_NOT_SUPPORTED`, `unsupported archive extension: '<extension as typed>'`); (5) a password is given for a plain or compressed tar archive (`ERR_INVALID_ARGUMENT`, `password is not applicable to this archive format`); (6) a multi volume RAR name (`ERR_NOT_SUPPORTED`, `multi volume RAR archives are not supported: extract a single file archive`); (7) the destination holds a NUL character (FR-NEW-004); (8) the destination is outside the project (FR-MOD-001); (9) every later refusal in the order its own specification gives, the destination refusal of that order removed.
- SPEC-0086/FR-NEW-005: THE archive extraction SHALL report, through the HTTP API, the same refusal code and the same message as through the assistant tool for every destination of this specification.
- SPEC-0086/FR-NEW-002 (partial: through the HTTP door): WHEN a member extracts an archive with a destination that is not outside the project THE archive extraction SHALL write the entries under the folder that destination resolves to from the project root, and SHALL answer and audit that resolved folder as the destination.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0086_027 e2e_0086_028 e2e_0086_031 e2e_0086_032 e2e_0086_034` and `cargo test -p mcp-fs-core --lib --features rar -- e2e_0086_029`: 6 tests pass. Real assertion: the HTTP status and JSON error body (`code`, `message`) and the volume read back.

## Design excerpt
### Decisions
- **DEC-002 (SPEC-0086):** for zip, 7z, RAR, plain and compressed tar the destination is resolved right after the multi volume check and before `client.read_bytes` (`archive.rs:178-205`); `planned_destination` (SPEC-0084 DEC-003) is removed and the bound and the declared check use the resolved destination with its stored files always counted; the bare formats keep their place (`archive_single.rs:241-248`). **Obliged by:** SPEC-0086/FR-MOD-002, FR-NEW-003, FR-NEW-004. **Rationale:** after this move a refused destination never reaches decoding, so "stored files count for nothing when refused" (SPEC-0084 FR-NEW-002, SPEC-0085 FR-NEW-001) holds trivially. **Alternatives considered:** keeping the helper (dead branch). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:143-227`.
- **DEC-005 (SPEC-0086):** tests in `crates/core/src/tools/archive_destination_e2e.rs`; RAR ones (E2E-004, E2E-017, E2E-029) in a nested `#[cfg(feature = "rar")] mod archive_rar` (SPEC-0085 DEC-011, `test.sh:7`); E2E-004's legs that need no RAR are not split out (the spec writes one test). **Obliged by:** n/a (no code impact): test layout. **Rationale:** as SPEC-0085 DEC-011. **Source:** SPEC-0085 DEC-011. **Evidence:** `test.sh:7`.
- **DEC-007 (SPEC-0086):** tests whose rule an earlier story built are green on arrival, each proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0180 to US-0182. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
No production contract change. HTTP: `POST /api/fs/{mount_id}/extract-archive` with `{"path": ..., "destination": ...}`; `ERR_PATH_OUT_OF_BOUNDS` maps to its HTTP status through `errors.rs` (unchanged); the JSON error body carries `code` and `message` as the other archive HTTP tests read them.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_destination_e2e.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify (tests) |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). The climb walk to mirror: `ensure_entry_path_safe` (`crates/core/src/tools/archive.rs:455-476`) and `tools::export::ensure_no_escape` (`crates/core/src/tools/export.rs:119-126`). `PosixPath::normpath` (`crates/core/src/util/posix.rs:48-55`). "Nothing is written" reads `exists` on `/x`, `/uploads/pack` and every folder the test names, `quota::used_bytes`, `allowance(f)`, and `extraction_audit_lines`. HTTP archive tests: `crates/core/src/api/dataplane.rs:3297-3470` (`Harness`, `u("extract-archive")`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0086/E2E-027: An unsupported extension wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/pack.lz` holding 5 bytes
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.lz'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_027_an_unsupported_extension_wins_over_t`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-028: A password on a tar archive wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/pack.tar.gz`
- **When** the member extracts it with password `x` and destination `../../x`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_028_a_password_on_a_tar_archive_wins_ove`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-029: A multi volume RAR name wins over the destination
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-002; **Priority:** Medium
- **Given** a server with RAR support and `/uploads/pack.part1.rar`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `multi volume RAR archives are not supported: extract a single file archive`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0086_029_a_multi_volume_rar_name_wins_over_th`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-031: HTTP API, same refusal
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.zip`
- **When** the member extracts it through the HTTP API with destination `../../x`
- **Then** the answer is a refusal with code `ERR_PATH_OUT_OF_BOUNDS` and message `path escapes the volume root: ../../x`, the same as through the assistant tool
- **And** nothing is written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0086_031_http_api_same_refusal`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-032: HTTP API, a climb that stays inside
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-005, FR-NEW-002; **Priority:** High
- **Given** `/uploads/pack.7z`
- **When** the member extracts it through the HTTP API with destination `/a/../b`
- **Then** the answer gives destination `/b`, 1 file written, 5 bytes written; `/b/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0086_032_http_api_a_climb_that_stays_inside`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-034: HTTP API, absolute climb and NUL on a bare file
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-MOD-001, FR-NEW-004; **Priority:** High
- **Given** `/uploads/a.txt.gz` holding `hello` compressed
- **When** the member extracts it through the HTTP API with destination `/../x`, then with the destination made of `out`, a NUL character, then `x`
- **Then** the refusals are `ERR_PATH_OUT_OF_BOUNDS` with `path escapes the volume root: /../x`, then `ERR_PATH_OUT_OF_BOUNDS` with `path contains a NUL byte`; nothing is written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0086_034_http_api_absolute_climb_and_nul_on_a`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-027 | resolve the destination before `detect_format`: `ERR_PATH_OUT_OF_BOUNDS` replaces `ERR_NOT_SUPPORTED`, the test fails |
| E2E-028 | resolve the destination before the tar password check: `ERR_PATH_OUT_OF_BOUNDS` replaces the password refusal, the test fails |
| E2E-029 | resolve the destination before the multi volume check: `ERR_PATH_OUT_OF_BOUNDS` replaces `ERR_NOT_SUPPORTED`, the test fails |
| E2E-031 | make `resolve_destination` return `state.safety.normalize_path(raw)`'s answer (the old clamp): the call writes `/x/a.txt`, the test fails |
| E2E-032 | refuse any `..` segment in `resolve_destination`: `/a/../b` is refused, the test fails |
| E2E-034 | walk only relative destinations (skip the walk when `raw` starts with `/`): `/../x` is clamped and accepted, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Production files not to touch: none expected

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-027, E2E-028, E2E-029, E2E-031, E2E-032, E2E-034): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_0080_061_destination_outside_the_project` (`archive_rar.rs:1283-1301`), the `../../x` tests of SPEC-0081 E2E-035, SPEC-0082 E2E-022 and SPEC-0083 E2E-032 (`archive_single.rs:1096-1097`, `:1533-1534`, `:2271-2272`), the path pinning tests `safety.rs:263-265` and `util/posix.rs:115-118`, every `e2e_0084_` and `e2e_0085_` test except the one this spec replaces; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0196: earlier refusals first and both doors alike`, body `Spec: SPEC-0086` and the requirement ids (SPEC-0086/FR-MOD-002, FR-NEW-005, FR-NEW-002).
