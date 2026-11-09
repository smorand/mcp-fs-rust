# US-0200: every block kind counts: stored data, recovery record, comment

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0199
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A RAR4 archive cut inside a stored file's data, inside a recovery record or a comment block, at any length, is refused as corrupt; a cut archive never replaces a colliding file, is never charged and leaves no trace even when repeated; it is reported before the storage limit.

## Requirements covered
- SPEC-0087/FR-NEW-001 (partial: recovery record and comment blocks, every length): THE archive extraction SHALL treat a RAR archive as complete when, read from its RAR marker, every block after the marker is wholly present, header and the data it declares, up to and including the end of archive block.
- SPEC-0087/FR-NEW-006: THE archive extraction SHALL NOT write a file, create a folder, charge the remaining storage or the session write allowance, record an audit line, or show the password in a message, a log line or an audit line, for a call refused under FR-MOD-001.
- SPEC-0087/FR-NEW-007 (partial: before collision and the storage limit): IF a truncated RAR archive meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password (FR-NEW-004), a wrong one (FR-NEW-005) or, for version 4, an empty one (FR-NEW-009); (5) the truncation (FR-MOD-001); (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.
- SPEC-0087/FR-MOD-001 (partial: stored data and every length of the plain RAR4 archives): IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_002 e2e_0087_010 e2e_0087_016 e2e_0087_038 e2e_0087_039 e2e_0087_040 e2e_0087_045 e2e_0087_046`: 8 tests pass. Real assertion: `/uploads/half4/a.txt` still holds `old` after the refused `overwrite` call (E2E-039).

## Design excerpt
### Decisions
- **DEC-002 (SPEC-0087):** the RAR4 walk reads the 7 byte base header of each block (CRC16, type, flags, size, size at least 7), the data size from `ADD_SIZE` when flag `0x8000` is set (plus the high 32 bits at header offset 32 for types `0x74` and `0x7A` when flag `0x0100` is set), stops at type `0x7B`, and treats every other type generically (comment, recovery record, service, sub blocks); a RAR4 header whose size is below 7, or a RAR5 vint that overflows, is refused as `TRUNCATED` or with the existing vint messages. Header CRCs of clear headers are not checked (UnRAR checks them); CRCs of decrypted headers are (DEC-003), with `flate2::Crc` (CRC16 = low 16 bits of the CRC32 of the header after its CRC field; RAR5 CRC32 of the header after its CRC field). **Obliged by:** SPEC-0087/FR-NEW-001 (every kind of block), FR-NEW-002. **Rationale:** the generic `LONG_BLOCK` rule covers the recovery record and comment of `extras4.rar` (P-1: a cut inside the recovery record is accepted today) without a table of block types. **Alternatives considered:** a per type table (misses unknown service blocks). **Source:** spec 14.3 "RAR4 block walk". **Evidence:** `unrar_sys-0.5.8/vendor/unrar/arcread.cpp`, spec probes P-1 and P-5.
- **DEC-006 (SPEC-0087):** fixtures: `two4.rar`, `big4.rar`, `dir4.rar`, `solid4.rar`, `extras4.rar`, `hp4.rar` (RAR 6.24) and `hp5two.rar` (RAR 7.23) are the archives the spec measured, copied from `/tmp/bl131/` (sizes 123, 3 116, 152, 133, 745, 220, 382 bytes, checked on 2026-11-09 with `wc -c`) into `crates/core/tests/fixtures/rar/` with a README row each (commands of spec 14.3); `hp4long.rar` is made once with `rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt` (RAR 6.24) for DEC-005's write back path. When `/tmp/bl131` is gone, remake each archive with its README command and re measure every offset the story's tests use (spec 14.3: RAR stamps times, sizes stay). **Obliged by:** SPEC-0087/FR-NEW-001 (spec 3.3, 9.0 archive table). **Rationale:** the offsets of spec 9.0 are those of these exact files. **Alternatives considered:** committing every cut (one archive covers every cut at run time). **Source:** spec 14.3, existing fixtures README. **Evidence:** `ls /tmp/bl131`, `crates/core/tests/fixtures/rar/README.md`.
- **DEC-012 (SPEC-0087):** stories US-0199, US-0200 and US-0203 add tests whose rule an earlier story built; each test green on arrival is proven by a mutation reasoned to change its observable outcome (listed per story); the two crypto stories are `min_tier` 1. **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182; decision density of a binary format plus two key derivations. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
No production contract change. New fixtures `big4.rar` (3 116 bytes, `rar a -ma4 -ep -m0 big4.rar r.bin a.txt`, `r.bin` = 3 000 bytes of `x`) and `extras4.rar` (745 bytes, `rar a -ma4 -ep -rr5% -zc.txt -k extras4.rar a.txt b.txt`, `c.txt` = `a comment`), README rows.
### Paths
| Path | Action |
|---|---|
| `crates/core/tests/fixtures/rar/big4.rar` | create |
| `crates/core/tests/fixtures/rar/extras4.rar` | create |
| `crates/core/tests/fixtures/rar/README.md` | modify |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Fixtures load with `include_bytes!("../../tests/fixtures/rar/<name>.rar")` as the SPEC-0080 tests do (`crates/core/src/tools/archive_rar.rs` tests); a cut is a slice taken at run time (`&TWO4[..67]`), never a committed file. Password of every encrypted fixture: `s3cret-Ünï`. Today's walk to extend: `ensure_rar5_complete` and `read_vint` (`crates/core/src/tools/archive_rar.rs:98-160`); the password mapping it must not change: `listing_error` (`:162-169`) and the entry read mapping (`:62-81`). The every length loop model: `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`). The log and audit leak check model: `assert_never_recorded` (`:922-938`). Offsets of spec section 9.0 are measured on the archives of `/tmp/bl131`; an archive remade from its README command needs its offsets re measured.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-002: Recovery record, comment and lock
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-001; **Priority:** High
- **Given** `/uploads/extras4.rar`
- **When** the member extracts it
- **Then** the answer is 2 files, 12 bytes, with the content of E2E-001 under `/uploads/extras4`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_002_recovery_record_comment_and_lock`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-010: Version 4 cut inside a file's data
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003; **Priority:** Critical
- **Given** `/uploads/big4cut.rar`, the first 1 562 bytes of `big4.rar`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008, not `archive entry 'r.bin' decompressed to more bytes than declared 3000`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_010_version_4_cut_inside_a_file_s_data`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-016: Every length of the plain version 4 archives
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-003, FR-NEW-001; **Priority:** Critical
- **Given** for each of `v4.rar`, `two4.rar`, `dir4.rar`, `solid4.rar` and `extras4.rar`, and each length from 0 to the archive's size minus one, the archive cut to that length stored as `/uploads/p.rar`
- **When** the member extracts each one
- **Then** from 7 bytes on the refusal is exactly that of E2E-008; below 7 bytes it is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; in every case `/uploads/p` does not exist
- **And** each whole archive extracts with every file it holds
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_016_every_length_of_the_plain_version_4`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-038: A cut wins over a collision
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007, FR-NEW-006; **Priority:** High
- **Given** `/uploads/half4.rar` of E2E-008 and `/uploads/half4/a.txt` holding `old`
- **When** the member extracts it
- **Then** the refusal is that of E2E-008, not `ERR_NO_CLOBBER`; `/uploads/half4/a.txt` still holds `old`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_038_a_cut_wins_over_a_collision`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-039: A cut with `overwrite` replaces nothing
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-006, FR-NEW-007; **Priority:** Critical
- **Given** `/uploads/half4.rar` of E2E-008 and `/uploads/half4/a.txt` holding `old`
- **When** the member extracts it with `overwrite` true
- **Then** the refusal is that of E2E-008; `/uploads/half4/a.txt` still holds `old` (today it is replaced by `hello` and the call succeeds); the remaining storage and session write allowance are unchanged; no audit line
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_039_a_cut_with_overwrite_replaces_nothin`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-040: A cut wins over the storage limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/big4cut.rar` of E2E-010 (it declares 3 005 bytes)
- **When** the member extracts it
- **Then** the refusal is that of E2E-008, not `ERR_PROJECT_QUOTA_EXCEEDED`; the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_040_a_cut_wins_over_the_storage_limit`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-045: A refused cut leaves no trace
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Critical
- **Given** a storage limit with 10 000 bytes remaining, a session write allowance of 10 000 bytes, and `/uploads/half4.rar` of E2E-008
- **When** the member extracts it
- **Then** the refusal is that of E2E-008; the remaining storage is 10 000 bytes, the session allowance still accepts a 10 000 byte write; `/uploads/half4` does not exist; the audit trail holds no extraction line
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_045_a_refused_cut_leaves_no_trace`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-046: Repeating a refused cut
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/half4.rar` of E2E-008
- **When** the member extracts it twice
- **Then** both refusals are that of E2E-008; nothing written after either
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_046_repeating_a_refused_cut`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-002 | refuse any RAR4 block type other than main, file and end in the walk: `extras4.rar` is refused, the test fails |
| E2E-010 | make `ensure_rar_complete` return `Ok(())` at once (no walk): the cut answers `archive entry 'r.bin' decompressed to more bytes than declared 3000`, the test fails |
| E2E-016 | the E2E-010 mutation (no walk): the cuts on a block boundary and inside the recovery record of `extras4.rar` extract (P-1), the test fails |
| E2E-038 | the E2E-010 mutation: the listing reaches the collision and answers `ERR_NO_CLOBBER`, the test fails |
| E2E-039 | the E2E-010 mutation: the call succeeds and replaces `old` with `hello`, the test fails |
| E2E-040 | the E2E-010 mutation: the declared check of SPEC-0085 refuses 3 005 bytes against 1 000 with `ERR_PROJECT_QUOTA_EXCEEDED`, the test fails |
| E2E-045 | the E2E-010 mutation: the call succeeds and charges the storage and the allowance, the test fails |
| E2E-046 | the E2E-010 mutation: both calls succeed, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Fixture provenance (DEC-006): copy the archive from `/tmp/bl131/<name>.rar` (the file the spec measured; check its size with `wc -c` against the spec 9.0 table); when it is gone, remake it with the README command using the RARLAB trial build (`/tmp/rarbin/v6/rar/rar` is RAR 6.24, `/tmp/rarbin/rar/rar` is RAR 7.23, else download them from rarlab.com) and re measure every offset this story's tests use
- Production files not to touch: none expected; if E2E-016 shows a block kind the walk misreads, fix the walk under DEC-002 and record it in the commit body

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-002, E2E-010, E2E-016, E2E-038, E2E-039, E2E-040, E2E-045, E2E-046): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0200: every block kind counts: stored data, recovery record, comment`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-001, FR-NEW-006, FR-NEW-007, FR-MOD-001).
