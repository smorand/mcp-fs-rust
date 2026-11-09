# US-0203: password questions first, every cut of an encrypted names archive

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0202
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
On an archive with encrypted file names, a missing, wrong or empty password is still answered before the truncation, as today, while every cut read with the right password is refused as corrupt at every length; RAR5 cuts with readable names and a whole archive whose file lies about its size keep their answers.

## Requirements covered
- SPEC-0087/FR-NEW-004: IF a RAR archive declares encrypted file names in its leading part AND that leading part is whole AND no password is given THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `password required to extract this archive; call again with the 'password' parameter`, whether the rest of the archive is complete or not.
- SPEC-0087/FR-NEW-005 (partial: cut archives, empty password): IF a version 5 RAR archive declares encrypted file names in its whole leading part AND the given password is wrong or empty THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`, whether the rest of the archive is complete or not.
- SPEC-0087/FR-NEW-007 (partial: item 4 before item 5): IF a truncated RAR archive meets another refusal THEN THE archive extraction SHALL report the first of this list: (1) not a member; (2) the path is missing or is a folder; (3) a multi volume name or a server without RAR support; (4) encrypted file names with a missing password (FR-NEW-004), a wrong one (FR-NEW-005) or, for version 4, an empty one (FR-NEW-009); (5) the truncation (FR-MOD-001); (6) every later refusal of SPEC-0080 FR-NEW-015: a link or escaping entry, a missing or wrong password for encrypted data, a file larger than declared, the destination, a collision, the project storage limit, the session write allowance.
- SPEC-0087/FR-NEW-009 (partial: RAR4 wrong and empty password, a whole archive with a size lie): THE archive extraction SHALL extract or refuse every complete RAR archive exactly as before this increment, and SHALL keep, for a version 4 archive with encrypted file names whose leading part is whole, cut or not, today's answers to a password that is not the right one: a wrong password is refused as corrupt (a message starting `archive is corrupt or not a valid rar file:`), an empty password with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`.
- SPEC-0087/FR-MOD-001 (partial: every length with the right password): IF a RAR archive holding a whole RAR marker is not complete THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, and write nothing, for version 4 and version 5, plain, with encrypted data or with encrypted file names and the right password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_025 e2e_0087_026 e2e_0087_027 e2e_0087_031 e2e_0087_032 e2e_0087_033 e2e_0087_034 e2e_0087_035 e2e_0087_049 e2e_0087_050 e2e_0087_051 e2e_0087_055`: 12 tests pass. Real assertion: for every length of `hidden.rar`, `hp4.rar` and `hp5two.rar` in E2E-034, `/uploads/p` is absent from the volume after the call.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0087):** encrypted headers. The leading part (RAR4: marker and main header; RAR5: marker and encryption header) must be whole, else `TRUNCATED`. With `password` `None` or `Some("")` the walk returns `Ok` after the leading part and UnRAR answers (password needed, or incorrect for the empty password, `archive_rar.rs:162-169`, `:69-81`). With a non empty password: RAR5 derives the keys once per call and, when the check value flag is set and its checksum is valid, a mismatch returns `password_required_message(password)`; each header is decrypted (RAR5: 16 byte IV then the ciphertext; RAR4: 8 byte salt then the ciphertext, keys cached per salt within the call), its CRC checked, and a CRC mismatch returns `Ok` (UnRAR then gives today's answer: corrupt for RAR4, BL-0136; incorrect for RAR5); a shortfall anywhere returns `TRUNCATED`. **Obliged by:** SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007 item 4, FR-NEW-009, FR-MOD-001. **Rationale:** the password questions keep their place before the truncation; answering the wrong password ourselves only where RAR5 offers a check value matches what UnRAR says today (BadPassword from 62 bytes, P-3); deferring on a CRC mismatch keeps every wrong password answer byte for byte. **Alternatives considered:** deferring every password question to UnRAR after the walk (the walk cannot run without the key, so a cut archive with a wrong password would read as TRUNCATED, against FR-NEW-005). **Source:** design. **Evidence:** spec probes P-2 and P-3; `unrar_sys-0.5.8/vendor/unrar/crypt5.cpp`, `crypt3.cpp`.
- **DEC-007 (SPEC-0087):** E2E-034's right password half runs, for each archive, every length up to the end of the leading part plus 1, every offset spec 9.0 names for that archive and its two neighbours, and the last 8 lengths; its no password half runs every length. Recorded in the test's doc comment. **Obliged by:** SPEC-0087/FR-MOD-001, FR-NEW-003, FR-NEW-004 (E2E-034). **Rationale:** spec 14.3 allows it when the full loop is too slow; every call with a password pays two key derivations (ours and UnRAR's) in an unoptimized debug build. **Alternatives considered:** the full loop (several minutes per run); optimizing dependencies in the dev profile (BL-0137). **Source:** spec 14.3 "Run time". **Evidence:** spec 14.3 probe timings (2.9 s, 7.1 s, 10.4 s in release).
- **DEC-012 (SPEC-0087):** stories US-0199, US-0200 and US-0203 add tests whose rule an earlier story built; each test green on arrival is proven by a mutation reasoned to change its observable outcome (listed per story); the two crypto stories are `min_tier` 1. **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182; decision density of a binary format plus two key derivations. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
No production contract change. E2E-034 (DEC-007): the no password half loops every length from 0 to size minus 1; the right password half loops, per archive, every length from 0 to the end of the leading part plus 1 (21 for `hp4.rar`, 63 for the two others), every offset spec 9.0 names for that archive and its two neighbours (`hidden.rar`: 168, 169, 170, 221, 222, 223; `hp4.rar`: 107, 108, 109, 179, 180, 181, 189, 190, 191, 194, 195, 196; `hp5two.rar`: 205, 206, 207), and the last 8 lengths; the doc comment of the test says so. `cuth.rar` and `cut.rar` are the committed SPEC-0080 fixtures; E2E-049's `bomb.rar` is `rar5_store(&[RarEntry::declaring("big.bin", 10, &vec![7u8; 1_000_000])])`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Fixtures load with `include_bytes!("../../tests/fixtures/rar/<name>.rar")` as the SPEC-0080 tests do (`crates/core/src/tools/archive_rar.rs` tests); a cut is a slice taken at run time (`&TWO4[..67]`), never a committed file. Password of every encrypted fixture: `s3cret-Ünï`. Today's walk to extend: `ensure_rar5_complete` and `read_vint` (`crates/core/src/tools/archive_rar.rs:98-160`); the password mapping it must not change: `listing_error` (`:162-169`) and the entry read mapping (`:62-81`). The every length loop model: `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`). The log and audit leak check model: `assert_never_recorded` (`:922-938`). Offsets of spec section 9.0 are measured on the archives of `/tmp/bl131`; an archive remade from its README command needs its offsets re measured.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-025: Version 4, encrypted names, cut, no password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-007; **Priority:** High
- **Given** `/uploads/hp4half.rar` of E2E-022
- **When** the member extracts it with no password
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with `password required to extract this archive; call again with the 'password' parameter`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_025_version_4_encrypted_names_cut_no_pas`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-026: Version 4, encrypted names, cut inside the leading part, no password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-003; **Priority:** High
- **Given** `/uploads/hp4short.rar`, the first 16 bytes of `hp4.rar`
- **When** the member extracts it with no password
- **Then** the refusal is that of E2E-008, not the password needed message (today it is the password needed message); nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_026_version_4_encrypted_names_cut_inside`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-027: Version 4, encrypted names, cut, wrong password (unchanged, BL-0136)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-009, FR-NEW-007; **Priority:** Medium
- **Given** `/uploads/hp4half.rar` of E2E-022, and `/uploads/hp4.rar` whole
- **When** the member extracts each with `nope`
- **Then** each refusal is `ERR_INVALID_ARGUMENT` with a message starting `archive is corrupt or not a valid rar file:`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_027_version_4_encrypted_names_cut_wrong`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-031: Version 5, encrypted names, cut, wrong password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005, FR-NEW-007; **Priority:** High
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with `nope`
- **Then** the refusal is `ERR_PASSWORD_REQUIRED` with `incorrect password for this archive; call again with the correct 'password' parameter`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_031_version_5_encrypted_names_cut_wrong`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-032: Version 5, encrypted names, cut, no password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-007; **Priority:** High
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with no password
- **Then** the refusal is that of E2E-025; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_032_version_5_encrypted_names_cut_no_pas`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-033: Version 5, cut inside the encryption header, no password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/hid40.rar`, the first 40 bytes of `hidden.rar`
- **When** the member extracts it with no password, then with `nope`
- **Then** both refusals are that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_033_version_5_cut_inside_the_encryption`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-034: Every length of the encrypted names archives, right password
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-003, FR-NEW-004; **Priority:** Critical
- **Given** for each of `hidden.rar`, `hp4.rar` and `hp5two.rar`, and each length from 0 to the archive's size minus one, the archive cut to that length stored as `/uploads/p.rar`
- **When** the member extracts each one with `s3cret-Ünï`
- **Then** from the whole marker on the refusal is exactly that of E2E-008, below it a message starting `archive is corrupt or not a valid rar file:`; `/uploads/p` never exists
- **And** extracted with no password, every length from the whole leading part on (20 bytes for `hp4.rar`, 62 for the two others) gives the refusal of E2E-025, and every shorter length from the whole marker on gives that of E2E-008
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_034_every_length_of_the_encrypted_names`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-035: Version 5, readable names, encrypted data, cut, right password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001, FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/cut.rar`
- **When** the member extracts it with `s3cret-Ünï`
- **Then** the refusal is that of E2E-008; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_035_version_5_readable_names_encrypted_d`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-049: A whole archive whose file is larger than declared (unchanged)
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/uploads/bomb.rar`, a complete version 5 archive whose file `big.bin` declares 10 bytes and holds 1 000 000 bytes (SPEC-0080 E2E-012)
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `archive entry 'big.bin' decompressed to more bytes than declared 10`; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_049_a_whole_archive_whose_file_is_larger`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-050: Version 5, encrypted names, cut, empty password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/uploads/cuth.rar`
- **When** the member extracts it with the empty password
- **Then** the refusal is that of E2E-031; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_050_version_5_encrypted_names_cut_empty`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-051: Version 5, encrypted names, cut between files, wrong password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/uploads/hp5half.rar` of E2E-030
- **When** the member extracts it with `nope`
- **Then** the refusal is that of E2E-031; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_051_version_5_encrypted_names_cut_betwee`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-055: Version 4, encrypted names, cut, empty password (unchanged)
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-009, FR-NEW-007, FR-NEW-004; **Priority:** Medium
- **Given** `/uploads/hp4half.rar` of E2E-022
- **When** the member extracts it with the empty password
- **Then** the refusal is that of E2E-031; nothing written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_rar_cut_e2e.rs
- Function: `e2e_0087_055_version_4_encrypted_names_cut_empty`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-025 | with no password and encrypted headers, return `TRUNCATED` when the leading part is whole (instead of handing over to UnRAR): the answer becomes corrupt, the test fails |
| E2E-026 | return `Ok(())` when the RAR4 main header is cut (hand the archive to UnRAR): 16 bytes answer `password required ...` (P-3: from 14 bytes), the test fails |
| E2E-027 | on a RAR4 decrypted header CRC mismatch return `password_required_message(password)`: the answer becomes `ERR_PASSWORD_REQUIRED`, the test fails |
| E2E-031 | on a RAR5 check value mismatch return `TRUNCATED`: the answer becomes corrupt, the test fails |
| E2E-032 | map `Code::MissingPassword` to `corrupt` in `listing_error`: the answer becomes corrupt, the test fails |
| E2E-033 | return `Ok(())` when the RAR5 encryption header is cut: UnRAR answers with its own detail or a password question, not `unexpected end of archive`, the test fails |
| E2E-034 | hand every archive with encrypted headers to UnRAR whatever the password (skip the decrypted walk): right password cuts extract or answer the wrong password message, the test fails |
| E2E-035 | make `ensure_rar_complete` return `Ok(())` at once: the cut inside encrypted data with the right password answers the wrong password message, the test fails |
| E2E-049 | map the plain entry read error to `corrupt("rar", e)`: the message changes, the test fails |
| E2E-050 | turn an empty password into no password before calling UnRAR: the answer becomes `password required ...`, the test fails |
| E2E-051 | the E2E-031 mutation: the answer becomes corrupt, the test fails |
| E2E-055 | the E2E-050 mutation: the answer becomes `password required ...`, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Production files not to touch: none expected
- E2E-034's loop reduction is the one of DEC-007, written in the test's doc comment; do not widen it to the full loop (debug build run time) and do not narrow it further

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-025, E2E-026, E2E-027, E2E-031, E2E-032, E2E-033, E2E-034, E2E-035, E2E-049, E2E-050, E2E-051, E2E-055): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0203: password questions first, every cut of an encrypted names archive`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007, FR-NEW-009, FR-MOD-001).
