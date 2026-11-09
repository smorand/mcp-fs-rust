# US-0185: order the bound among the tar refusals and re extract near the limit

> Spec: SPEC-0084 (sdlc/specs/SPEC-0084-tar-family-bounded-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0184
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The decoding bound takes its place in the order of refusals (after the password, before a collision or a link entry, and against corruption whichever comes first), files already stored in the destination count toward it so a re extraction near the limit still succeeds, and an entry declaring more than the archive holds stays a corrupt refusal.

## Requirements covered
- SPEC-0084/FR-NEW-002 (partial: the stored files under the destination, for `overwrite` true and false, and files elsewhere not counted): THE archive extraction SHALL set, before decoding a compressed tar archive, a decoding bound equal to the lower of the project side and the session side, plus a framing allowance of 1 048 576 bytes.
- SPEC-0084/FR-NEW-005: IF several refusals apply to the extraction of a compressed tar archive THEN THE archive extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path does not exist (`ERR_NOT_FOUND`, `'<path>' not found`); (3) the path is a folder (`ERR_INVALID_ARGUMENT`, `'<path>' is a directory, not an archive file`); (4) a password is given (`ERR_INVALID_ARGUMENT`, `password is not applicable to this archive format`); (5) corrupt compressed data (`ERR_INVALID_ARGUMENT`, a message starting `archive is corrupt or not a valid <format> file:`) or the decoding bound (FR-NEW-003), whichever decoding meets first; (6) the entry checks of today, in archive order (an unsupported entry type, an entry path leaving the destination, an unreadable or truncated entry); (7) the destination refusal of today; (8) collision (`ERR_NO_CLOBBER`, `'<path>' already exists; pass overwrite=true to replace it`); (9) the project storage limit, then the session write allowance, on the files' sizes.
- SPEC-0084/FR-NEW-010: IF an entry header of a compressed tar archive declares more bytes than the decompressed archive holds after it THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and a message starting `archive is corrupt or not a valid <format> file:`, and write nothing.
- SPEC-0084/FR-MOD-001 (partial: the bound before collision and entry type checks): IF a compressed tar archive decompresses past the decoding bound THEN THE archive extraction SHALL refuse it at that point, before its entries are checked, before its collisions are checked and before the limits are checked on the files' sizes.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0084_016 e2e_0084_017 e2e_0084_018 e2e_0084_019 e2e_0084_020 e2e_0084_022 e2e_0084_023 e2e_0084_024 e2e_0084_025 e2e_0084_043 e2e_0084_044 e2e_0084_045`: 12 tests pass. Real assertions: `/uploads/bomb/zeros.bin` still reads `old` after E2E-016, `quota::used_bytes` shows 100 000 bytes still remaining after the 3 000 000 byte re extraction of E2E-022.

## Design excerpt
### Decisions
- **DEC-002 (SPEC-0084):** the project side is `quota::project_remaining_for(client, safety, stored)` where `stored = quota::stored_under(client, dest)` (sum of `size` over the file nodes of `meta.subtree(dest)`), for `overwrite` true and false; `stored = 0` when the planned destination is `None` (refused). No project limit: `None`, the session side alone sets the cap. The session side is `safety.session_remaining(person, mount_id)`. **Obliged by:** SPEC-0084/FR-NEW-002 (and its business rule on a refused destination). **Rationale:** `meta.subtree` already selects a folder and everything below it with the escaped `LIKE` plus exact match (`SubtreeMatch`, AGENTS.md conventions), so no new SQL; `project_remaining_for` already adds a replaced size and floors at 0. **Alternatives considered:** `folder_usage` (direct items only, needs a cap); a new SUM query (new SQL across two dialects for no gain). **Source:** existing code. **Evidence:** `crates/core/src/quota.rs:165-178`, `crates/core/src/storage/meta.rs:1868`, `crates/core/src/safety.rs:151-154`.
- **DEC-005 (SPEC-0084):** the bounded tar decode runs in one `spawn_blocking` task that decompresses with `read_capped(tar_decoder(c, &bytes)?, bound.limit)` then parses with the existing entry loop; it returns either the entries and the high water, or `CapExceeded { high_water }`, or a `ToolError`. On `CapExceeded` the async side sets the hook and returns `caps.refusal(state, &client).await?`. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-005 (item 5: corrupt or bound, whichever decoding meets first). **Rationale:** the order "corrupt or bound, whichever comes first" falls out of one sequential read; nothing is written, charged or audited before `admit_paths` and `finish_extraction`, which a refusal never reaches (FR-NEW-004). **Alternatives considered:** two tasks (decompress, then parse): one more hop for nothing. **Source:** design. **Evidence:** `crates/core/src/tools/archive_single.rs:254-277`.
- **DEC-012 (SPEC-0084):** E2E-024 carries one derived assertion beyond its spec text: the decompressed bytes held stay at or below 1 148 576 (the bound: 100 000 + 1 048 576). **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-002. **Rationale:** its spec assertions (`ERR_PROJECT_QUOTA_EXCEEDED`, nothing written) are also produced by the later admission on declared sizes, so without it no mutation of the bound is observable (US-0181 and US-0172 commit notes: a listed mutation must really kill the test). **Alternatives considered:** recording E2E-024 as a guard with no mutation. **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:247-262`.
- **DEC-013 (SPEC-0084):** stories US-0185 and US-0186 hold tests that turn green with an earlier story's code (green on arrival); each is proven by a mutation reasoned to change that test's observable outcome, listed in the story. **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182. **Alternatives considered:** fewer, larger stories over the tier 2 budget. **Source:** design. **Evidence:** `git log` of US-0180 to US-0182.
### Data model
None.
### Contracts
No production contract change (US-0184 built it). Testkit additions: `tar_with_symlink(link_name, target, then: &[(&str, &[u8])]) -> Vec<u8>` (GNU symlink header first, as `build_tar_with_special_entry`, `archive.rs:1315`), `tar_lying(name, declared: u64, content: &[u8]) -> Vec<u8>` (one GNU header declaring `declared`, then `content`, nothing after), `abc_bytes(n)` (`abc` repeated to `n` bytes). E2E-024 also asserts `held_at_most(1_148_576)` (DEC-012).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_testkit.rs` | modify |
| `crates/core/src/tools/archive_tar_bound_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0084/E2E-016: The bound wins over a collision
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-002, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage, `/uploads/bomb.tar.gz` of E2E-008, and an existing `/uploads/bomb/zeros.bin` containing `old`
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_NO_CLOBBER`
- **And** `/uploads/bomb/zeros.bin` still reads `old`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_016_the_bound_wins_over_a_collision`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-017: The bound wins over an unsupported entry
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` whose decompressed archive holds a symbolic link `link` pointing to `/etc/passwd`, then `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_NOT_SUPPORTED`; `/uploads/bomb` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_017_the_bound_wins_over_an_unsupported_e`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-018: Password wins over the bound
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` of E2E-008
- **When** the member extracts it with the password `s3cret`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with the message `password is not applicable to this archive format`
- **And** the decompressed bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_018_password_wins_over_the_bound`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-019: Corruption met first wins over the bound
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` of E2E-008 with its first 2 bytes (the gzip signature) set to zero
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.gz file:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_019_corruption_met_first_wins_over_the_b`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-020: The bound met first wins over a later corruption
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz` of E2E-008 with its last 8 bytes (the gzip trailer) inverted
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not the corrupt refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_020_the_bound_met_first_wins_over_a_late`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-022: Re extract with overwrite near the limit
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** Critical
- **Given** 100 000 bytes of remaining storage, `/uploads/pack.tar.gz` holding `big.bin` (3 000 000 bytes of the text `abc` repeated), and `/uploads/pack/big.bin` already holding the same 3 000 000 bytes
- **When** the member extracts it with `overwrite` true
- **Then** the extraction succeeds with 1 file and 3 000 000 bytes written
- **And** the remaining storage is still 100 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_022_re_extract_with_overwrite_near_the_l`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-023: Same situation without overwrite
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005, FR-NEW-002; **Priority:** High
- **Given** the situation of E2E-022
- **When** the member extracts it with `overwrite` false
- **Then** the refusal is `ERR_NO_CLOBBER` with the message `'/uploads/pack/big.bin' already exists; pass overwrite=true to replace it`, not a limit refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_023_same_situation_without_overwrite`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-024: Files stored elsewhere do not count
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** the archive of E2E-022, the same 3 000 000 bytes stored at `/elsewhere/big.bin`, nothing under `/uploads/pack`, and 100 000 bytes of remaining storage
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`; `/uploads/pack` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_024_files_stored_elsewhere_do_not_count`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-025: Explicit destination holding the files
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** the archive of E2E-022, `/out/big.bin` holding the same 3 000 000 bytes, nothing under `/uploads/pack`, and 100 000 bytes of remaining storage
- **When** the member extracts it to the destination `/out` with `overwrite` true
- **Then** the extraction succeeds; `/out/big.bin` holds the 3 000 000 bytes; the remaining storage is still 100 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_025_explicit_destination_holding_the_fil`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-043: `.tar.gz` entry declaring more than it holds
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/lie.tar.gz` whose only entry `a.txt` declares 1 000 000 000 bytes and is followed by the 5 bytes `hello`, then the end of the archive
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.gz file:`, not a limit refusal
- **And** `/uploads/lie` does not exist; the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_043_tar_gz_entry_declaring_more_than_it`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-044: `.tar.bz2` entry declaring more than it holds
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/lie.tar.bz2` built as in E2E-043
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.bz2 file:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_044_tar_bz2_entry_declaring_more_than_it`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-045: `.tar.xz` entry declaring more than it holds
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** `/uploads/lie.tar.xz` built as in E2E-043
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid tar.xz file:`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_045_tar_xz_entry_declaring_more_than_it`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-016 | remove the bound (call `read_capped` with `u64::MAX`): the bomb decodes whole, the collision scan refuses with `ERR_NO_CLOBBER`, the test fails |
| E2E-017 | remove the bound as for E2E-016: the entry loop reaches the symlink and refuses with `ERR_NOT_SUPPORTED`, the test fails |
| E2E-018 | drop the tar password check (`archive.rs:166-173`): the bound refusal comes instead, and `held()` is no longer 0, the test fails |
| E2E-019 | map `ReadError::Io` to `caps.refusal(..)` instead of `corrupt`: the damaged signature answers `ERR_PROJECT_QUOTA_EXCEEDED`, the test fails |
| E2E-020 | remove the bound as for E2E-016: `GzDecoder` reaches the inverted trailer and the refusal is the corrupt one, the test fails |
| E2E-022 | make `stored_under` return 0: the bound falls to 1 148 576 bytes, below the 3 000 000 byte entry, the call is refused, the test fails |
| E2E-023 | count the stored files only when `overwrite` is true: the bound falls to 1 148 576 bytes and the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` instead of `ERR_NO_CLOBBER`, the test fails |
| E2E-024 | call `stored_under(client, "/")` instead of the destination: `/elsewhere/big.bin` counts, the bound rises to 4 148 576, about 3 000 000 decompressed bytes are held and `held_at_most(1_148_576)` fails |
| E2E-025 | compute the stored files from the default destination (`/uploads/pack`) instead of the given `/out`: the bound falls to 1 148 576 bytes, the call is refused, the test fails |
| E2E-043 | in the entry loop of `decode_tar`, stop (`break`) at the first error of the entries iterator instead of returning it: `a.txt` (5 bytes, declared 1 000 000 000) reaches the admission, which refuses with `ERR_PROJECT_QUOTA_EXCEEDED`, the test fails |
| E2E-044 | the E2E-043 mutation: the admission refusal replaces the corrupt one, the test fails |
| E2E-045 | the E2E-043 mutation: the admission refusal replaces the corrupt one, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- The memory hook is the `cfg(test)` thread local `archive_single::LAST_HIGH_WATER`; set it on the async thread after `spawn_blocking` returns (the blocking thread is another thread), and keep tests on the default single threaded `#[tokio::test]`
- No dash as punctuation in code, comments or docs
- Production files not to touch: none expected; if a test fails for a reason the design did not foresee, stop and report it rather than change the order of refusals
- Scope boundary: no change to the destination refusal's place (SPEC-0086 moves it)

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-016, E2E-017, E2E-018, E2E-019, E2E-020, E2E-022, E2E-023, E2E-024, E2E-025, E2E-043, E2E-044, E2E-045): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` (includes `tools::archive_single`) passes unchanged, the bare format suites `e2e_0081_`, `e2e_0082_`, `e2e_0083_` and `e2e_new_` included; `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0185: order the bound among the tar refusals and re extract near the limit`, body `Spec: SPEC-0084` and the requirement ids (SPEC-0084/FR-NEW-002, FR-NEW-005, FR-NEW-010, FR-MOD-001).
