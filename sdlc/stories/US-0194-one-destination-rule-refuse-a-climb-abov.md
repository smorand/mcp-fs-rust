# US-0194: one destination rule: refuse a climb above the root for every format

> Spec: SPEC-0086 (sdlc/specs/SPEC-0086-refuse-destination-outside-the-project/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0193
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A destination that climbs above the project root, relative (`../../x`) or absolute (`/../x`, `/a/../../b`), is refused with `path escapes the volume root: <destination>` by every format, where zip, 7z and the tar family write at the root today and RAR and bare files clamp the absolute form; a folder named `..out` is accepted. The refusal keeps its place in each format's order here; US-0195 moves it before the archive is read.

## Requirements covered
- SPEC-0086/FR-NEW-001: THE archive extraction SHALL treat a destination as outside the project when, reading its segments separated by `/` from left to right and starting at the project root (with or without a leading `/`), a `..` segment is met while at the project root.
- SPEC-0086/FR-NEW-003 (partial: a refused climb writes, creates, charges and audits nothing): THE archive extraction SHALL NOT read the archive's content, write a file, create a folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused under FR-MOD-001 or FR-NEW-004.
- SPEC-0086/FR-MOD-001: IF the destination is outside the project THEN THE archive extraction SHALL refuse with `ERR_PATH_OUT_OF_BOUNDS` and the message `path escapes the volume root: <destination>`, `<destination>` being the destination exactly as the member typed it, for every archive format.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0086_009 e2e_0086_010 e2e_0086_011 e2e_0086_012 e2e_0086_013 e2e_0086_014 e2e_0086_015 e2e_0086_016 e2e_0086_018 e2e_0086_035` and `cargo test -p mcp-fs-core --lib --features rar -- e2e_0086_017`: 11 tests pass. Real assertion: `/x/a.txt` absent and `/x/a.txt` still holding `old` (E2E-035) when read back from the volume.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0086):** one function `archive_single::resolve_destination(raw: &str) -> Result<String>`: NUL refused first (`path contains a NUL byte`), then a segment walk on `/` with a depth counter (`""` and `"."` stay, `".."` at depth 0 refused with `path escapes the volume root: <raw>`, `".."` otherwise up, any other segment down, dots inside a name included), then `PosixPath::normpath(&format!("/{raw}"))`; it never calls `SafetyManager::normalize_path`. **Obliged by:** SPEC-0086/FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001. **Rationale:** the walk is the repository's existing climb rule (`export.rs:119-126`, `archive.rs:455-476`); judging while walking refuses `a/../../x` and `/a/../../b` even though a clamp would land inside (spec D-6); `normpath` of a non climbing path stays under `/`, and skipping `normalize_path` drops its `starts_with("/..")` prefix test that refuses `..out` (`safety.rs:105`). Splitting on `/` only keeps a backslash a plain character (spec 3.2). **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/tools/export.rs:119-126`, `crates/core/src/safety.rs:99-111`, `crates/core/src/util/posix.rs:48-55`.
- **DEC-003 (SPEC-0086):** `SafetyManager::normalize_path` and every other path option stay as they are. **Obliged by:** SPEC-0086/FR-NEW-006. **Rationale:** BL-0135 owns them. **Alternatives considered:** section 2. **Source:** spec. **Evidence:** `crates/core/src/safety.rs:263-265`, `crates/core/src/util/posix.rs:115-118`.
- **DEC-004 (SPEC-0086):** US-0194 applies the strict rule at today's place (after decoding for the multi formats), US-0195 moves it before reading: the order tests of US-0195 are red against US-0194 and green after, and SPEC-0085 E2E-028 changes in the story that changes its outcome. **Obliged by:** n/a (no code impact): story planning. **Rationale:** each story ships red evidence for the behavior it adds. **Alternatives considered:** one story (16 plus tests, over the tier 2 budget). **Source:** design. **Evidence:** n/a.
- **DEC-005 (SPEC-0086):** tests in `crates/core/src/tools/archive_destination_e2e.rs`; RAR ones (E2E-004, E2E-017, E2E-029) in a nested `#[cfg(feature = "rar")] mod archive_rar` (SPEC-0085 DEC-011, `test.sh:7`); E2E-004's legs that need no RAR are not split out (the spec writes one test). **Obliged by:** n/a (no code impact): test layout. **Rationale:** as SPEC-0085 DEC-011. **Source:** SPEC-0085 DEC-011. **Evidence:** `test.sh:7`.
### Data model
None.
### Contracts
`archive_single.rs`: `pub(crate) fn resolve_destination(raw: &str) -> Result<String>` replaces `normalize_destination(state, dest)` (`:119-130`): NUL refused with `ToolError::path_out_of_bounds("path contains a NUL byte")`; walk `raw.split('/')` with a depth counter (`""`, `"."` stay; `".."` at depth 0 refused with `ToolError::path_out_of_bounds(format!("path escapes the volume root: {raw}"))`; `".."` otherwise up; any other segment down); return `PosixPath::normpath(&format!("/{raw}"))`. Callers: `extract_single` (`archive_single.rs:244`) and every multi format in `extract_archive` (`archive.rs:219-227`: one arm `Some(d) => archive_single::resolve_destination(d)?` for every format, at today's place; the `rar` special case and its comment at `:213-218` go), and `planned_destination` (SPEC-0084 DEC-003) uses it too (`.ok()`). New test module `crates/core/src/tools/archive_destination_e2e.rs` (`#[cfg(test)] mod archive_destination_e2e;`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/tools/archive_destination_e2e.rs` | create |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). The climb walk to mirror: `ensure_entry_path_safe` (`crates/core/src/tools/archive.rs:455-476`) and `tools::export::ensure_no_escape` (`crates/core/src/tools/export.rs:119-126`). `PosixPath::normpath` (`crates/core/src/util/posix.rs:48-55`). "Nothing is written" reads `exists` on `/x`, `/uploads/pack` and every folder the test names, `quota::used_bytes`, `allowance(f)`, and `extraction_audit_lines`. `crates/core/tests/fixtures/rar/v5.rar` is `/uploads/pack.rar` (`a.txt` = `hello`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0086/E2E-009: One step above the root
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `..`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ..`; nothing is written; `/a.txt` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_009_one_step_above_the_root`
- Red before the change because: `normalize_path("..")` turns `/..` into `/` (`util/posix.rs:48-55`), so the zip is extracted at the root and `/a.txt` exists
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-010: Names made of dots and letters are folders
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/pack.tar.xz`
- **When** the member extracts it with destination `..out`, then with destination `out..`
- **Then** the answers give destinations `/..out` and `/out..`; `/..out/a.txt` and `/out../a.txt` read `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_010_names_made_of_dots_and_letters_are_f`
- Red before the change because: `normalize_path` refuses `/..out` on its string prefix test (`safety.rs:105`): today the answer is `path escapes the volume root: ..out`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-011: zip, relative destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`
- **And** nothing is written; `/x/a.txt` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_011_zip_relative_destination_outside_the`
- Red before the change because: the zip destination is clamped: `../../x` becomes `/x` and `/x/a.txt` is written
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-012: 7z, relative destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with destination `../../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_012_7z_relative_destination_outside_the`
- Red before the change because: the 7z destination is clamped to `/x`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-013: plain tar, relative destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.tar`
- **When** the member extracts it with destination `../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../x`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_013_plain_tar_relative_destination_outsi`
- Red before the change because: the tar destination is clamped to `/x`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-014: Climb revealed only after resolving, compressed tar family
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/pack.tgz`, `/uploads/pack.tar.bz2` and `/uploads/pack.TAR.XZ`
- **When** the member extracts each with destination `a/../../x`
- **Then** each refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: a/../../x`; nothing is written; `/a` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_014_climb_revealed_only_after_resolving`
- Red before the change because: `a/../../x` normalizes to `/x` for the tar family
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-015: zip, absolute destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `/../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /../x`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_015_zip_absolute_destination_outside_the`
- Red before the change because: `/../x` normalizes to `/x` for zip
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-016: Bare gzip file, absolute destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** `/uploads/a.txt.gz` holding `hello` compressed
- **When** the member extracts it with destination `/../x`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /../x`; nothing is written; `/x/a.txt` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_016_bare_gzip_file_absolute_destination`
- Red before the change because: `normalize_destination` checks only relative input (`archive_single.rs:124-125`), so `/../x` is clamped to `/x` for a bare gzip file
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-017: RAR, absolute climb through a folder
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** a server with RAR support and `/uploads/pack.rar`
- **When** the member extracts it with destination `/a/../../b`
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /a/../../b`; nothing is written; `/b` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0086_017_rar_absolute_climb_through_a_folder`
- Red before the change because: RAR goes through `normalize_destination`, which clamps an absolute climb: `/a/../../b` becomes `/b`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-018: Bare bzip2 and xz files, absolute destination outside the project
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001; **Priority:** Medium
- **Given** `/uploads/a.txt.bz2` and `/uploads/a.txt.xz`, each holding `hello` compressed
- **When** the member extracts each with destination `/../x`
- **Then** each refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: /../x`; nothing is written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_018_bare_bzip2_and_xz_files_absolute_des`
- Red before the change because: as E2E-016 for bzip2 and xz
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-035: overwrite does not lift the refusal
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/pack.zip` and `/x/a.txt` holding `old`
- **When** the member extracts it with destination `../../x` and `overwrite` true
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with the message `path escapes the volume root: ../../x`; `/x/a.txt` still reads `old`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_035_overwrite_does_not_lift_the_refusal`
- Red before the change because: `../../x` is clamped to `/x` and `overwrite` replaces `/x/a.txt`
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- `SafetyManager::normalize_path` is not modified and not called for the destination (DEC-001, DEC-003)
- Scope boundary: the destination check stays at today's place in each format's order (moved by US-0195)

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-009, E2E-010, E2E-011, E2E-012, E2E-013, E2E-014, E2E-015, E2E-016, E2E-017, E2E-018, E2E-035), before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_0080_061_destination_outside_the_project` (`archive_rar.rs:1283-1301`), the `../../x` tests of SPEC-0081 E2E-035, SPEC-0082 E2E-022 and SPEC-0083 E2E-032 (`archive_single.rs:1096-1097`, `:1533-1534`, `:2271-2272`), the path pinning tests `safety.rs:263-265` and `util/posix.rs:115-118`, every `e2e_0084_` and `e2e_0085_` test except the one this spec replaces; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0194: one destination rule: refuse a climb above the root for every format`, body `Spec: SPEC-0086` and the requirement ids (SPEC-0086/FR-NEW-001, FR-NEW-003, FR-MOD-001).
