# US-0190: net growth with stored files and the order of refusals

> Spec: SPEC-0085 (sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0189
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The early check counts files already stored at the target paths, whatever `overwrite` says, so a re extraction near the limit succeeds and a collision keeps its refusal when the growth fits; the limits are reported after every header refusal and before anything decoding finds.

## Requirements covered
- SPEC-0085/FR-NEW-001 (partial: stored files at the target paths, explicit destination, files elsewhere): WHEN a member extracts a `.zip`, `.7z` or `.rar` archive, in any letter case, THE archive extraction SHALL compute, from the archive's headers and before decoding any entry, the declared total and the net growth, and check them against the two limits.
- SPEC-0085/FR-NEW-009 (partial: zip and 7z): IF several refusals apply to the extraction of a `.zip`, `.7z` or `.rar` archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) RAR only: a multi volume name, or a server without RAR support (SPEC-0080 FR-NEW-010, FR-NEW-012); (5) the refusals read from the headers, in this order per format: zip: unreadable archive, then per entry in archive order a link, an escaping name or a duplicate name, then a missing password, then a local header disagreeing with the directory; 7z: an encrypted header without the right password or an unreadable header, then a missing password, then per entry in archive order an escaping name then a link or special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then a missing password for encrypted data; (6) the project storage limit on the declared net growth (FR-NEW-002, FR-NEW-005); (7) the session write allowance on the declared total (FR-NEW-003, FR-NEW-005); (8) what decoding finds, in archive order: a wrong password (`ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`), damaged data (`ERR_INVALID_ARGUMENT`, a message starting `archive is corrupt or not a valid <format> file:`), an entry past its declared size (FR-NEW-007); (9) the destination refusal of today; (10) collision (`ERR_NO_CLOBBER`, `'<path>' already exists; pass overwrite=true to replace it`); (11) the project storage limit then the session write allowance at the moment of writing, with the same figures, charged on success.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0085_014 e2e_0085_015 e2e_0085_016 e2e_0085_025 e2e_0085_026 e2e_0085_027 e2e_0085_031 e2e_0085_032 e2e_0085_035 e2e_0085_042 e2e_0085_043`: 11 tests pass. Real assertions: `/uploads/bomb/zeros.bin` still reads `old` (E2E-025), the remaining storage still 100 000 after a 3 000 000 byte re extraction (E2E-014).

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0085):** zip, 7z and RAR go through list, then `check_declared`, then decode, each blocking step in `spawn_blocking`; the listing applies every header refusal of FR-NEW-009 item 5 in the spec's per format order (zip: unreadable, per entry link / escaping / duplicate, missing password, local header disagreement; 7z: encrypted or unreadable header, missing password, per entry escaping then special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then missing password for encrypted data); the decode step reopens zip and 7z from the same bytes. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-004, FR-NEW-009, FR-MOD-001. **Rationale:** headers are all the early check needs (`by_index_raw` gives `size()` with no password, `archive.rs:506-527`; `Archive::read` gives `archive.files[i].size`, `archive.rs:597`; RAR's listing gives `unpacked_size`, `archive_rar.rs:36-41`), so nothing is decoded before the limits. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** as cited.
- **DEC-002 (SPEC-0085):** `quota::check_declared` computes the declared total (`u128`) and the declared net growth (`i128`, last occurrence per target path, minus the stored file size, whatever `overwrite` says, 0 for every path when `count_stored` is false), treats any size, total or growth above `i64::MAX` as too large, and refuses: project limit and (too large or `used + growth > limit` with `growth > 0`) → `project_quota_exceeded(limit, used, min(growth, i64::MAX))`, `i64::MAX` when too large; else (no limit and too large) or `total > session_remaining` → the session refusal; equality accepted; nothing charged; the admission lock is taken only inside `quota::used_bytes`. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-005, FR-NEW-006. **Rationale:** the exact figures of today's admission (`quota.rs:136-159`) computed without wrap and without charging; project first and the tie to the project as FR-NEW-003 says. **Alternatives considered:** reusing `admit_paths` with a dry run flag (it takes the lock and charges; a flag doubles its contract). **Source:** design. **Evidence:** `crates/core/src/quota.rs:96-159`, `crates/core/src/safety.rs:136-154`.
- **DEC-008 (SPEC-0085):** target paths for the early check are joined to the planned destination of SPEC-0084 DEC-003 (`join_entry_destination`, `archive.rs:369-372`); when it is `None` (the destination will be refused), `count_stored` is false and the paths are joined to `/` only to keep last occurrence semantics. **Obliged by:** SPEC-0085/FR-NEW-001 business rule, spec D-6. **Rationale:** one destination resolution for the two bounds; SPEC-0086 later replaces it by the real destination known before reading. **Alternatives considered:** a second resolution. **Source:** SPEC-0084 DEC-003. **Evidence:** `crates/core/src/tools/archive.rs:213-227`.
- **DEC-013 (SPEC-0085):** tests whose rule an earlier story already built are green on arrival and each is proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182. **Source:** design. **Evidence:** `git log` of those stories.
### Data model
None.
### Contracts
No production contract change (US-0189 built it). Testkit addition: `limit_whole_mb_remaining(f, n)` (spec 9 preamble: when the stored files exceed 1 MB, the limit is the next whole megabyte and only the message prefix is asserted), `abc_bytes(n)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_testkit.rs` | modify |
| `crates/core/src/tools/archive_declared_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Remaining storage with exact messages: `limit_mb_remaining(f, n)` sets a limit of 1 048 576 bytes (`f.state.admin.set_quota(MOUNT, Some(1_048_576))`) and writes `/filler.bin` so that exactly `n` bytes remain (the archive counts as stored), hence `<U>` = 1 048 576 minus `n`; `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB. Archive builders: `build_zip` (`crates/core/src/tools/archive.rs:1104-1121`), `build_sevenz_ext` (`:2207-2250`), `rar5_store` and `RarEntry` (`crates/core/src/tools/archive_rar_fixtures.rs:55-105`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0085/E2E-014: Re extract with overwrite near the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Critical
- **Given** 100 000 bytes of remaining storage (limit large enough for the files below), `/uploads/pack.zip` holding `big.bin` (3 000 000 bytes of the text `abc` repeated), and `/uploads/pack/big.bin` already holding the same 3 000 000 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds with 1 file and 3 000 000 bytes written
- **And** the remaining storage is still 100 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_014_re_extract_with_overwrite_near_the_l`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-015: Same situation without overwrite
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** High
- **Given** the situation of E2E-014
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/pack/big.bin' already exists; pass overwrite=true to replace it`, not a limit refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_015_same_situation_without_overwrite`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-016: Files stored elsewhere do not count
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** the archive of E2E-014, the same 3 000 000 bytes stored at `/elsewhere/big.bin`, nothing under `/uploads/pack`, 100 000 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `3000000 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/pack` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_016_files_stored_elsewhere_do_not_count`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-025: The limit wins over a collision
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage, `/uploads/bomb.zip` of E2E-006 and an existing `/uploads/bomb/zeros.bin` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9999997 bytes requested`, not `ERR_NO_CLOBBER`
- **And** `/uploads/bomb/zeros.bin` still reads `old`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_025_the_limit_wins_over_a_collision`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-026: The limit wins over a wrong password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/vault.zip`, a bomb whose entry is encrypted with the password `s3cret`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_PASSWORD_REQUIRED`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_026_the_limit_wins_over_a_wrong_password`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-027: The limit wins over damaged data
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.zip` of E2E-006 with the first 16 bytes of its compressed data inverted, headers intact
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not a message starting `archive is corrupt or not a valid zip file:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_027_the_limit_wins_over_damaged_data`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-031: A missing password wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/vault.7z`, a bomb encrypted with `s3cret`, header in clear
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_031_a_missing_password_wins_over_the_lim`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-032: An unreadable archive wins over everything after it
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/cut.zip`, the first 100 bytes of `/uploads/bomb.zip` of E2E-006
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid zip file:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_032_an_unreadable_archive_wins_over_ever`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-035: Membership wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage, `/uploads/bomb.zip` of E2E-006, and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_035_membership_wins_over_the_limit`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-042: Replacement one byte over
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** 100 000 bytes of remaining storage, `/uploads/pack/big.bin` holding 3 000 000 bytes, and `/uploads/pack.zip` holding `big.bin` of 3 100 001 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `100001 bytes requested`; the decoded bytes held are 0; `/uploads/pack/big.bin` is unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_042_replacement_one_byte_over`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-043: Explicit destination holding the files
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** the archive of E2E-014, `/out/big.bin` holding the same 3 000 000 bytes, nothing under `/uploads/pack`, 100 000 bytes of remaining storage
- **When** the member extracts it to the destination `/out` with `overwrite` true
- **Then** the extraction succeeds; the remaining storage is still 100 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_043_explicit_destination_holding_the_fil`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-014 | call `check_declared` with `count_stored = false`: the growth is 3 000 000 against 100 000, the call is refused, the test fails |
| E2E-015 | subtract the stored size only when `overwrite` is true: the growth is 3 000 000 and the limit refusal replaces `ERR_NO_CLOBBER`, the test fails |
| E2E-016 | subtract the stored files of the whole project (`stored_under(client, "/")`) from the growth: the early check passes, the zip is decoded, the final admission refuses with the same message, but `held()` is 3 000 000, not 0, the test fails |
| E2E-025 | run the no clobber scan over the listed entries before `check_declared`: the refusal is `ERR_NO_CLOBBER`, the test fails |
| E2E-026 | run `check_declared` after the decode: the decode with `nope` fails first with `ERR_PASSWORD_REQUIRED`, the test fails |
| E2E-027 | run `check_declared` after the decode: the damaged data fails first as corrupt, the test fails |
| E2E-031 | move the 7z missing password gate after `check_declared`: the limit refusal comes first, the test fails |
| E2E-032 | treat a zip whose directory cannot be read as an empty listing (`unwrap_or_default` on `ZipArchive::new`) and skip the decode: the call answers 0 files, the test fails |
| E2E-035 | make the membership check accept any person (`state.authorize` returns `Ok` for a non member): the limit refusal comes instead of `ERR_FORBIDDEN`, the test fails |
| E2E-042 | call `check_declared` with `count_stored = false`: the message ends `3100001 bytes requested`, not `100001`, the test fails |
| E2E-043 | join the target paths to the default destination (`/uploads/pack`) instead of `/out`: nothing is stored there, the growth is 3 000 000, the call is refused, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Production files not to touch: none expected; a test failing for an unforeseen reason is reported, not patched around
- Scope boundary: RAR is US-0191

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-014, E2E-015, E2E-016, E2E-025, E2E-026, E2E-027, E2E-031, E2E-032, E2E-035, E2E-042, E2E-043): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive tools::quota_e2e` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted`, `spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing`, `spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted`, `spec_0020_e2e_051_an_extraction_racing_a_write` (`quota_e2e.rs`), `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs`), every `e2e_0084_` test; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0190: net growth with stored files and the order of refusals`, body `Spec: SPEC-0085` and the requirement ids (SPEC-0085/FR-NEW-001, FR-NEW-009).
