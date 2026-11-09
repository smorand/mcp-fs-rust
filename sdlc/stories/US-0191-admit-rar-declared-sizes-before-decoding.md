# US-0191: admit RAR declared sizes before decoding

> Spec: SPEC-0085 (sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0190
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A RAR archive goes through the same listing, early check and decode as zip and 7z: an over limit RAR archive is refused before decoding, also when it collides or names a destination that will be refused, and SPEC-0080's test that pinned the collision first changes to the limit first.

## Requirements covered
- SPEC-0085/FR-MOD-001 (partial: RAR): IF the declared net growth or the declared total of a `.zip`, `.7z` or `.rar` archive exceeds its limit THEN THE archive extraction SHALL refuse before decoding any entry, before the destination check and before the collision check.
- SPEC-0085/FR-NEW-007 (partial: RAR: a size lie stops at the declared size): IF an entry of a `.zip`, `.7z` or `.rar` archive decodes to more bytes than it declares THEN THE archive extraction SHALL stop decoding it, refuse with `ERR_INVALID_ARGUMENT` and the message `archive entry '<path>' decompressed to more bytes than declared <D>`, and write nothing.
- SPEC-0085/FR-NEW-009 (partial: RAR): IF several refusals apply to the extraction of a `.zip`, `.7z` or `.rar` archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) RAR only: a multi volume name, or a server without RAR support (SPEC-0080 FR-NEW-010, FR-NEW-012); (5) the refusals read from the headers, in this order per format: zip: unreadable archive, then per entry in archive order a link, an escaping name or a duplicate name, then a missing password, then a local header disagreeing with the directory; 7z: an encrypted header without the right password or an unreadable header, then a missing password, then per entry in archive order an escaping name then a link or special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then a missing password for encrypted data; (6) the project storage limit on the declared net growth (FR-NEW-002, FR-NEW-005); (7) the session write allowance on the declared total (FR-NEW-003, FR-NEW-005); (8) what decoding finds, in archive order: a wrong password (`ERR_PASSWORD_REQUIRED`, `incorrect password for this archive; call again with the correct 'password' parameter`), damaged data (`ERR_INVALID_ARGUMENT`, a message starting `archive is corrupt or not a valid <format> file:`), an entry past its declared size (FR-NEW-007); (9) the destination refusal of today; (10) collision (`ERR_NO_CLOBBER`, `'<path>' already exists; pass overwrite=true to replace it`); (11) the project storage limit then the session write allowance at the moment of writing, with the same figures, charged on success.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0085_002 e2e_0085_004 e2e_0085_008 e2e_0085_013 e2e_0085_020 e2e_0085_022 e2e_0085_028 e2e_0085_030 e2e_0085_036 e2e_0085_mod_001`: 10 tests pass, and `./test.sh` runs them (module paths contain `archive_rar`). Real assertion: `held()` reads 0 for the RAR bomb and `quota::used_bytes` is unchanged.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0085):** zip, 7z and RAR go through list, then `check_declared`, then decode, each blocking step in `spawn_blocking`; the listing applies every header refusal of FR-NEW-009 item 5 in the spec's per format order (zip: unreadable, per entry link / escaping / duplicate, missing password, local header disagreement; 7z: encrypted or unreadable header, missing password, per entry escaping then special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then missing password for encrypted data); the decode step reopens zip and 7z from the same bytes. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-004, FR-NEW-009, FR-MOD-001. **Rationale:** headers are all the early check needs (`by_index_raw` gives `size()` with no password, `archive.rs:506-527`; `Archive::read` gives `archive.files[i].size`, `archive.rs:597`; RAR's listing gives `unpacked_size`, `archive_rar.rs:36-41`), so nothing is decoded before the limits. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** as cited.
- **DEC-004 (SPEC-0085):** decoded bytes held: the decode step keeps a running total of decoded entry bytes and its maximum in a `&mut u64` it returns with its result (success or error); the async side sets `archive_single::LAST_HIGH_WATER` to it under `cfg(test)`; a call refused before decoding never sets it, so `held()` reads 0. **Obliged by:** SPEC-0085/FR-NEW-004, FR-NEW-008, NFR 7.1. **Rationale:** the one hook of SPEC-0084 DEC-006, set on the async thread (the blocking thread differs). **Alternatives considered:** an `Arc<AtomicU64>`. **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:56-61`.
- **DEC-007 (SPEC-0085):** `decode_rar` becomes `list_rar(bytes, password) -> Result<RarListing>` (temp dir and file, completeness walk, listing with entry rules, missing password gate; `RarListing { dir: TempDir, path: PathBuf, entries }`) and `decode_rar_listed(listing, password, held: &mut u64) -> Result<Vec<DecodedEntry>>`; the temp dir drops when the listing drops after decoding or on any refusal. **Obliged by:** SPEC-0085/FR-MOD-001, FR-NEW-009 item 5 and 8. **Rationale:** UnRAR opens a path only (SPEC-0080 DEC-003), so the file is written once and read by both passes. **Alternatives considered:** writing the temp file twice. **Source:** existing code. **Evidence:** `crates/core/src/tools/archive_rar.rs:18-90`.
- **DEC-011 (SPEC-0085):** tests needing the `rar` feature sit in a module whose path contains `archive_rar` (a nested `#[cfg(feature = "rar")] mod archive_rar { ... }` in `archive_declared_e2e.rs`; E2E-MOD-001 in `archive_rar.rs`), because `test.sh:7` filters the feature run on `archive_rar` and `e2e_0080_`. **Obliged by:** n/a (no code impact): test runner. **Rationale:** without it `make test` never runs them; editing `test.sh` per spec adds a file to every story that adds one. **Alternatives considered:** extending the `test.sh` filter. **Source:** existing code. **Evidence:** `test.sh:7`.
### Data model
In memory only: `RarListing { dir: tempfile::TempDir, path: PathBuf, entries: Vec<ListedEntry>, any_encrypted: bool }`.
### Contracts
`archive_rar.rs`: `pub(crate) fn list_rar(bytes: &[u8], password: Option<&str>) -> Result<RarListing>` (temp dir and file, `ensure_rar5_complete`, the listing with `ensure_entry_allowed`, the missing password gate; `archive_rar.rs:18-46` today) and `pub(crate) fn decode_rar_listed(listing: &RarListing, password: Option<&str>, held: &mut u64) -> Result<Vec<DecodedEntry>>` (`:48-90` today); `decode_rar` goes. `archive.rs`: `ArchiveFormat::Rar` joins the zip and 7z route (listing, `check_declared`, decode, hook). Existing test replaced in `archive_rar.rs` tests: `e2e_0080_068_collision_wins_over_the_storage_limi` (`:1334-1344`) is removed and `e2e_0085_mod_001_the_storage_limit_wins_over_a_collis` is added in its place (same fixture `report()`, same `limit_remaining(&f, 5)`, new expectation `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9 bytes requested`, `old` kept, 5 bytes remaining).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify (production and one test) |
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_declared_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Remaining storage with exact messages: `limit_mb_remaining(f, n)` sets a limit of 1 048 576 bytes (`f.state.admin.set_quota(MOUNT, Some(1_048_576))`) and writes `/filler.bin` so that exactly `n` bytes remain (the archive counts as stored), hence `<U>` = 1 048 576 minus `n`; `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB. Archive builders: `build_zip` (`crates/core/src/tools/archive.rs:1104-1121`), `build_sevenz_ext` (`:2207-2250`), `rar5_store` and `RarEntry` (`crates/core/src/tools/archive_rar_fixtures.rs:55-105`). The RAR test helpers to reuse inside `archive_rar.rs`: `report()`, `limit_remaining`, `remaining_storage`, `read` (`archive_rar.rs:224-250`, `:1334-1356`). The committed `crates/core/tests/fixtures/rar/v5.rar` holds `a.txt` = `hello` (E2E-002, `include_bytes!`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0085/E2E-002: 7z and RAR within the limits, upper case suffixes
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** `/u1/PACK.7Z` and, on a server with RAR support, `/u2/pack.RAR`, each holding `a.txt` (`hello`)
- **When** the member extracts each
- **Then** `/u1/PACK/a.txt` and `/u2/pack/a.txt` read `hello`, each answer reporting 1 file and 5 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_002_7z_and_rar_within_the_limits_upper_c`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-004: Declared total equal to the session allowance
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** session allowance 12 and, on a server with RAR support, `/uploads/report.rar` holding folder `data`, `data/a.txt` (`hello`) and `data/b.txt` (`goodbye`)
- **When** the member extracts it
- **Then** the extraction succeeds: 2 files, 12 bytes written
- **And** a following write of a 1 byte file `/uploads/one.bin` is refused with `session write quota of 12 bytes exceeded`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_004_declared_total_equal_to_the_session`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-008: RAR bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-004, FR-MOD-001; **Priority:** Critical
- **Covers:** class sweep occurrence 4 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the limit is checked.
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar`, a bomb
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** the decoded bytes held are 0; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_008_rar_bomb_against_the_storage_limit`
- Red before the change because: after step 1, `held()` is 10 000 000: today every entry is decoded before the admission
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-013: Repeated entry names: net for storage, gross for the allowance
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** Medium
- **Given** a server with RAR support, 1 000 bytes of remaining storage, session allowance 1 000, and `/uploads/dup.rar` holding `a.txt` twice, 600 bytes each
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 1000 bytes exceeded` (gross 1 200), not the project refusal (net 600)
- **And** the decoded bytes held are 0; `/uploads/dup` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_013_repeated_entry_names_net_for_storage`
- Red before the change because: after step 1, `held()` is 1 200, not 0 (the admission already gives the session refusal)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-020: RAR entry holding more than it declares
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** a server with RAR support and `/uploads/liar.rar` built as in E2E-019
- **When** the member extracts it
- **Then** the refusal is as in E2E-019; the decoded bytes held never exceed 65 546
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_020_rar_entry_holding_more_than_it_decla`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-022: RAR entry declared 0 holding data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** a server with RAR support and `/uploads/zero.rar` built as in E2E-021
- **When** the member extracts it
- **Then** the refusal is as in E2E-021
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_022_rar_entry_declared_0_holding_data`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-028: The limit wins over a refused destination
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-001; **Priority:** Medium
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar` of E2E-008
- **When** the member extracts it to the destination `../../x`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_PATH_OUT_OF_BOUNDS`; neither `/x` nor `/uploads/bomb` exists
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_028_the_limit_wins_over_a_refused_destin`
- Red before the change because: today the RAR bomb is decoded, then the destination `../../x` is refused (`ERR_PATH_OUT_OF_BOUNDS`) before the admission
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-030: A RAR escaping name wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** a server with RAR support, 1 000 bytes of remaining storage and `/uploads/bomb.rar` holding `zeros.bin` of a bomb, then an entry named `../../etc/passwd`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with `archive entry path escapes the destination: '../../etc/passwd'`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_030_a_rar_escaping_name_wins_over_the_li`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-036: A collision still wins when the net growth fits
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** High
- **Given** a server with RAR support, 9 bytes of remaining storage, `/uploads/report.rar` of E2E-004 (12 bytes declared) and `/uploads/report/data/a.txt` already holding `old` (3 bytes), so the net growth is 9
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with `'/uploads/report/data/a.txt' already exists; pass overwrite=true to replace it`
- **And** `/uploads/report/data/a.txt` still reads `old`; the remaining storage is still 9 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0085_036_a_collision_still_wins_when_the_net`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-MOD-001: The storage limit wins over a collision (was SPEC-0080 E2E-068, "Collision wins over the storage limit")
- **Validated before:** with 5 bytes remaining, 12 declared and `/uploads/report/data/a.txt` existing, the refusal was `ERR_NO_CLOBBER`.
- **Validates now:** the declared net growth (12 minus the 3 bytes of `old`, so 9) exceeds 5, so the limit is reported first; E2E-036 keeps the collision first when the growth fits.
- **Given** a server with RAR support, 5 bytes of remaining storage, `/uploads/report.rar` of E2E-004 and `/uploads/report/data/a.txt` holding `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9 bytes requested`
- **And** `/uploads/report/data/a.txt` still reads `old`; the remaining storage is still 5 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar.rs tests (replaces `e2e_0080_068_collision_wins_over_the_storage_limi`)
- Function: `e2e_0085_mod_001_the_storage_limit_wins_over_a_collis`
- Red before the change because: today the collision is checked after decoding and before the admission: the refusal is `ERR_NO_CLOBBER` (the old test asserted exactly that)
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-002 | make `decode_rar_listed` return no entries: `/u2/pack/a.txt` is missing, the test fails |
| E2E-004 | pass `session_bytes` 0 to `admit_paths` for RAR: the later 1 byte write is accepted, the test fails |
| E2E-020 | map the plain entry read error to `corrupt("rar", e)` instead of the size lie message: the message changes, the test fails |
| E2E-022 | the E2E-020 mutation: the message changes, the test fails |
| E2E-030 | apply `ensure_entry_allowed` after `check_declared` (collect the listing without the entry rules, check, then refuse): the limit refusal comes first, the test fails |
| E2E-036 | call `check_declared` with `count_stored = false`: the growth is 12 against 9, the limit refusal replaces `ERR_NO_CLOBBER`, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- The memory hook is the `cfg(test)` thread local `archive_single::LAST_HIGH_WATER`; set it on the async thread after `spawn_blocking` returns (the blocking thread is another thread), and keep tests on the default single threaded `#[tokio::test]`
- No dash as punctuation in code, comments or docs
- Tests needing RAR sit in the nested `#[cfg(feature = "rar")] mod archive_rar` of `archive_declared_e2e.rs` (DEC-011), except E2E-MOD-001 which replaces the old test inside `archive_rar.rs`
- The temp dir of a listing is dropped on every return path (refusal included)
- Scope boundary: no change to the RAR completeness walk (SPEC-0087)

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-008, E2E-013, E2E-028, E2E-MOD-001), before the production code (step 1: wire the decoded bytes hook into the RAR decode and run: E2E-008 and E2E-013 are red on `held()`, E2E-028 and E2E-MOD-001 on their code; step 2: the listing split and `check_declared`).
- [ ] Green on arrival tests (E2E-002, E2E-004, E2E-020, E2E-022, E2E-030, E2E-036): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive tools::quota_e2e` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted`, `spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing`, `spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted`, `spec_0020_e2e_051_an_extraction_racing_a_write` (`quota_e2e.rs`), `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs`), every `e2e_0084_` test; no change to the tool contract; the removed test is exactly `e2e_0080_068_collision_wins_over_the_storage_limi`, no other SPEC-0080 test changes
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0191: admit RAR declared sizes before decoding`, body `Spec: SPEC-0085` and the requirement ids (SPEC-0085/FR-MOD-001, FR-NEW-007, FR-NEW-009).
