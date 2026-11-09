# US-0199: complete RAR archives unchanged, folders and solid cuts refused

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0198
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
Complete RAR archives keep extracting exactly as before (two files, an empty archive, bytes after the end block or before the marker), a cut after a folder or inside a solid archive is refused, and the refusals that come before a cut (multi volume name, membership, a server without RAR support) still do; a destination inside the project changes nothing.

## Requirements covered
- SPEC-0087/FR-NEW-001 (partial: folders, solid archives, empty archive, bytes before and after): THE archive extraction SHALL treat a RAR archive as complete when, read from its RAR marker, every block after the marker is wholly present, header and the data it declares, up to and including the end of archive block.
- SPEC-0087/FR-NEW-007 (partial: items 1 to 3 before the cut): IF a truncated RAR archive meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password (FR-NEW-004), a wrong one (FR-NEW-005) or, for version 4, an empty one (FR-NEW-009); (5) the truncation (FR-MOD-001); (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.
- SPEC-0087/FR-NEW-009 (partial: complete archives unchanged): THE archive extraction SHALL extract or refuse every complete RAR archive exactly as before this increment, and SHALL keep, for a version 4 archive with encrypted file names whose leading part is whole, cut or not, today's answers to a password that is not the right one: a wrong password is refused as corrupt (a message starting `archive is corrupt or not a valid rar file:`), an empty password with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`.
- SPEC-0087/FR-MOD-001 (partial: folders and solid): IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_001 e2e_0087_005 e2e_0087_006 e2e_0087_007 e2e_0087_017 e2e_0087_018 e2e_0087_041 e2e_0087_042 e2e_0087_054` (9 pass) and `cargo test -p mcp-fs-core --lib -- e2e_0087_043` in the default build (1 passes). Real assertion: `/uploads/two4/a.txt` and `b.txt` read back `hello` and `goodbye`.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0087):** `ensure_rar_complete(bytes, password)` replaces `ensure_rar5_complete` at the start of the RAR listing: it finds the marker as UnRAR does (the first 7 bytes, else the first `0x52` byte from offset 7 within the next `0x200000 - 16` bytes whose bytes form a signature; RAR 1.4 signature or no signature or a RAR5 signature without its 8th byte: return `Ok` and let UnRAR refuse), then walks the version's blocks; any block whose header or declared data runs past the end, or an archive ending after a whole block that is not the end of archive block, is refused with `corrupt("rar", TRUNCATED)`; bytes after the end block are ignored. **Obliged by:** SPEC-0087/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-MOD-001. **Rationale:** the safe API cannot tell an end block from an end of file (`unrar-0.5.8/src/open_archive.rs:452`, `dll.cpp:222-231`) and carries no packed size (`open_archive.rs:582-590`); mirroring UnRAR's marker search keeps the walk and the reader on the same archive (E2E-047). **Alternatives considered:** section 2. **Source:** spec 14.3 and existing code. **Evidence:** `crates/core/src/tools/archive_rar.rs:98-160`, `unrar_sys-0.5.8/vendor/unrar/archive.cpp:98-175`.
- **DEC-006 (SPEC-0087):** fixtures: `two4.rar`, `big4.rar`, `dir4.rar`, `solid4.rar`, `extras4.rar`, `hp4.rar` (RAR 6.24) and `hp5two.rar` (RAR 7.23) are the archives the spec measured, copied from `/tmp/bl131/` (sizes 123, 3 116, 152, 133, 745, 220, 382 bytes, checked on 2026-11-09 with `wc -c`) into `crates/core/tests/fixtures/rar/` with a README row each (commands of spec 14.3); `hp4long.rar` is made once with `rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt` (RAR 6.24) for DEC-005's write back path. When `/tmp/bl131` is gone, remake each archive with its README command and re measure every offset the story's tests use (spec 14.3: RAR stamps times, sizes stay). **Obliged by:** SPEC-0087/FR-NEW-001 (spec 3.3, 9.0 archive table). **Rationale:** the offsets of spec 9.0 are those of these exact files. **Alternatives considered:** committing every cut (one archive covers every cut at run time). **Source:** spec 14.3, existing fixtures README. **Evidence:** `ls /tmp/bl131`, `crates/core/tests/fixtures/rar/README.md`.
- **DEC-008 (SPEC-0087):** tests live in `crates/core/src/tools/archive_rar_cut_e2e.rs` (`#[cfg(test)] mod archive_rar_cut_e2e;`), each `#[cfg(feature = "rar")]` except E2E-043 (`#[cfg(not(feature = "rar"))]`, run by the default `cargo test --workspace`); the module path contains `archive_rar` for `test.sh:7`; the HTTP tests sit in a nested `#[cfg(feature = "rar")] mod archive_rar` of `api/dataplane.rs` tests. Helpers from `archive_testkit.rs` plus a module local `assert_never_recorded` twin (`archive_rar.rs:922-938`). **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0085 DEC-011. **Source:** existing code. **Evidence:** `test.sh:7`, `crates/core/src/tools/archive.rs:905-933`.
- **DEC-012 (SPEC-0087):** stories US-0199, US-0200 and US-0203 add tests whose rule an earlier story built; each test green on arrival is proven by a mutation reasoned to change its observable outcome (listed per story); the two crypto stories are `min_tier` 1. **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182; decision density of a binary format plus two key derivations. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
No production contract change. New fixtures `dir4.rar` (152 bytes, `rar a -ma4 dir4.rar d a.txt` with empty folder `d/e`) and `solid4.rar` (133 bytes, `rar a -ma4 -ep -s solid4.rar a.txt b.txt`), README rows. E2E-043 is `#[cfg(not(feature = "rar"))]` and runs in the default build; E2E-005 builds `empty4.rar` at run time as `[&V4[..20], &V4[V4.len() - 7..]].concat()`.
### Paths
| Path | Action |
|---|---|
| `crates/core/tests/fixtures/rar/dir4.rar` | create |
| `crates/core/tests/fixtures/rar/solid4.rar` | create |
| `crates/core/tests/fixtures/rar/README.md` | modify |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Fixtures load with `include_bytes!("../../tests/fixtures/rar/<name>.rar")` as the SPEC-0080 tests do (`crates/core/src/tools/archive_rar.rs` tests); a cut is a slice taken at run time (`&TWO4[..67]`), never a committed file. Password of every encrypted fixture: `s3cret-Ünï`. Today's walk to extend: `ensure_rar5_complete` and `read_vint` (`crates/core/src/tools/archive_rar.rs:98-160`); the password mapping it must not change: `listing_error` (`:162-169`) and the entry read mapping (`:62-81`). The every length loop model: `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`). The log and audit leak check model: `assert_never_recorded` (`:922-938`). Offsets of spec section 9.0 are measured on the archives of `/tmp/bl131`; an archive remade from its README command needs its offsets re measured. The server without RAR support test model: `e2e_0080_015_server_without_rar_support` (`crates/core/src/tools/archive.rs:929-933`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-001: Complete version 4 archive with two files
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** Critical
- **Given** `/uploads/two4.rar`
- **When** the member extracts it
- **Then** the answer is 2 files, 12 bytes; `/uploads/two4/a.txt` holds `hello`, `/uploads/two4/b.txt` holds `goodbye`
- **And** one audit line names `/uploads/two4.rar`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_001_complete_version_4_archive_with_two`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-005: Complete empty version 4 archive
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/empty4.rar`: the first 20 bytes of `v4.rar` followed by its last 7 bytes (marker, main header, end of archive block)
- **When** the member extracts it
- **Then** the answer is 0 files, 0 bytes, 1 folder created; `/uploads/empty4` exists and is empty
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_005_complete_empty_version_4_archive`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-006: Bytes after the end of archive block
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/tail4.rar` = `v4.rar` followed by the 13 bytes `trailing junk`, and `/uploads/tail5.rar` = `v5.rar` followed by the same bytes
- **When** the member extracts each
- **Then** each answer is 1 file, 5 bytes; `/uploads/tail4/a.txt` and `/uploads/tail5/a.txt` hold `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_006_bytes_after_the_end_of_archive_block`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-007: Bytes before the marker, complete archive
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/sfx4.rar` = 100 zero bytes followed by `v4.rar`, and `/uploads/sfx5.rar` = 100 zero bytes followed by `v5.rar`
- **When** the member extracts each
- **Then** each answer is 1 file, 5 bytes; `/uploads/sfx4/a.txt` and `/uploads/sfx5/a.txt` hold `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_007_bytes_before_the_marker_complete_arc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-017: Version 4 cut after a folder
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001; **Priority:** High
- **Given** `/uploads/dircut.rar`, the first 107 bytes of `dir4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written (today `a.txt` and `d/e` are written and the call succeeds)
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_017_version_4_cut_after_a_folder`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-018: Solid version 4 cut between files
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001; **Priority:** High
- **Given** `/uploads/solidcut.rar`, the first 79 bytes of `solid4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_018_solid_version_4_cut_between_files`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-041: A multi volume name wins over a cut
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/big.part1.rar`, the first 67 bytes of `two4.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `multi volume RAR archives are not supported: extract a single file archive`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_041_a_multi_volume_name_wins_over_a_cut`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-042: Membership wins over a cut
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/half4.rar` of E2E-008 in project `atlas`, and `bob@example.com`, not a member of `atlas`
- **When** `bob@example.com` extracts it
- **Then** the refusal is `ERR_FORBIDDEN` with `'bob@example.com' is not a member of 'atlas'`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_042_membership_wins_over_a_cut`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-043: Server without RAR support
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Low
- **Given** a server delivered without RAR support and `/uploads/half4.rar` of E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `unsupported archive extension: '.rar'`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs (`#[cfg(not(feature = "rar"))]`)
- Function: `e2e_0087_043_server_without_rar_support`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-054: A cut with a destination inside the project
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007, FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/half4.rar` of E2E-008
- **When** the member extracts it with destination `out`
- **Then** the refusal is that of E2E-008; `/out` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_054_a_cut_with_a_destination_inside_the`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-001 | make `decode_rar_listed` return no entries: the answer reports 0 files, the test fails |
| E2E-005 | require at least one file block before the end block in the RAR4 walk: `empty4.rar` is refused, the test fails |
| E2E-006 | refuse any byte after the end block: both trailing junk archives are refused, the test fails |
| E2E-007 | look for the marker at offset 0 only and return `TRUNCATED` when it is not there: both prefixed archives are refused, the test fails |
| E2E-017 | accept an archive that ends right after a whole block (return `Ok` at the end of input): the cut after folder `d/e` extracts, the test fails |
| E2E-018 | the E2E-017 mutation: the solid cut extracts `a.txt`, the test fails |
| E2E-041 | move the multi volume name check after the listing: the cut is refused as corrupt first, the test fails |
| E2E-042 | make `state.authorize` accept a non member: the corrupt refusal comes instead of `ERR_FORBIDDEN`, the test fails |
| E2E-043 | in a build without the feature map `.rar` to `ArchiveFormat::Zip` in `SUFFIXES`: the refusal becomes the corrupt zip one, the test fails |
| E2E-054 | create the destination folder before the listing (call `client.makedirs(&dest, true)` right after resolving it): `/out` exists after the refusal, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Fixture provenance (DEC-006): copy the archive from `/tmp/bl131/<name>.rar` (the file the spec measured; check its size with `wc -c` against the spec 9.0 table); when it is gone, remake it with the README command using the RARLAB trial build (`/tmp/rarbin/v6/rar/rar` is RAR 6.24, `/tmp/rarbin/rar/rar` is RAR 7.23, else download them from rarlab.com) and re measure every offset this story's tests use
- Production files not to touch: none expected

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-001, E2E-005, E2E-006, E2E-007, E2E-017, E2E-018, E2E-041, E2E-042, E2E-043, E2E-054): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0199: complete RAR archives unchanged, folders and solid cuts refused`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-001, FR-NEW-007, FR-NEW-009, FR-MOD-001).
