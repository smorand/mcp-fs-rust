# US-0188: count declared archive sizes exactly, never negative

> Spec: SPEC-0085 (sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0187
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
An archive entry that declares 2^63 bytes or more, or several sizes whose sum wraps, is refused at the limit instead of being counted as a negative size that writes past the storage limit and lowers the member's used allowance. The fix sits at the admission itself (sizes converted without wrap, sums saturated), so it holds before the early check of US-0189 exists. Tier 1: exploit tests with hand built zip64 headers.

## Requirements covered
- SPEC-0085/FR-NEW-005 (partial: zip: the final admission refuses with `<R>` 9223372036854775807; the 7z leg (E2E-SEC-005) is US-0189): IF one declared size, the declared total or the declared net growth exceeds 9 223 372 036 854 775 807 bytes THEN THE archive extraction SHALL refuse as FR-NEW-002 states when the project has a storage limit, with `<R>` reported as `9223372036854775807`, and as FR-NEW-003 states otherwise.
- SPEC-0085/FR-NEW-006: THE archive extraction SHALL NOT count any declared size as a negative amount, and SHALL NOT lower the member's used session write allowance or the project's stored bytes because of an archive's declared sizes.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0085_sec_001 e2e_0085_sec_002 e2e_0085_sec_003 e2e_0085_sec_004`: 4 tests pass. Real assertions through second channels: `quota::used_bytes` still 1 047 576, a later real `fs_ops::write_bytes` of 1 001 bytes refused and of 1 000 bytes accepted, `/uploads/free/free.bin` absent when a second member (`noa@example.com`, added with `f.state.admin.add_member`) lists it.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0085):** the final admission never sees a negative size: `archive.rs` passes `i64::try_from(declared).unwrap_or(i64::MAX)` and a saturating total; `admit_paths` sums with `saturating_add`; `check_and_charge` compares `used.saturating_add(delta) > limit`; `charge_write` compares `s.bytes_written.saturating_add(num_bytes) > quota` and charges `num_bytes.max(0)`. **Obliged by:** SPEC-0085/FR-NEW-005, FR-NEW-006. **Rationale:** closes occurrences 1 to 3 at their own sites, so the exploit tests turn green in US-0188 before the early check exists, and stay green when it lands (defense in depth). **Alternatives considered:** fixing only the early check (the final admission would stay exploitable by any other caller passing a cast size). **Source:** class sweep, spec 14.2. **Evidence:** `crates/core/src/tools/archive.rs:251-252`, `crates/core/src/quota.rs:108`, `:156`, `crates/core/src/safety.rs:139`.
- **DEC-009 (SPEC-0085):** fixtures generated in the tests. Bombs: zip `build_zip(&[("zeros.bin", &vec![0; 10_000_000])], None)` (deflate, at most 20 000 bytes), 7z `build_sevenz_ext` (LZMA2), RAR `rar5_store(&[RarEntry::file("zeros.bin", &vec![0; 10_000_000])])` (store method, 10 MB, decoded size equal to its stored size); liars and declared sizes: a test helper `zip_declaring(entries: &[(&str, u64, &[u8])]) -> Vec<u8>` writes local headers, deflated data (`flate2::write::DeflateEncoder`), CRC32 (`flate2::Crc`) of the real content, a central directory and an end record; for a declared size of `0xFFFF_FFFF` or more both headers carry `0xFFFF_FFFF` and a zip64 extra field (id `0x0001`, uncompressed size then compressed size, 16 bytes) and version needed 45; a helper `sevenz_declaring(name, declared, data) -> Vec<u8>` assembles a 7z with one Copy coder folder (pack size = data length, unpack size = `declared`), no CRC digest, a plain (not encoded) header with the file name, and the start header CRC (`flate2::Crc`); RAR liars `RarEntry::declaring` (`archive_rar_fixtures.rs:87`). Remaining storage: `limit_mb_remaining(f, n)` sets a 1 048 576 byte limit and writes `/filler.bin` so exactly `n` bytes remain (the archive counts as stored); `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB (spec 9 preamble). **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-005, FR-NEW-007 (spec section 9: fixtures by the design), spec 14.3 (zip64 and 7z payloads). **Rationale:** no library writes a lying header; both builders are short and only test code. **Alternatives considered:** committed fixture files; patching a `zip` crate archive in place (the zip64 field does not exist in a small archive). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:1104-1160`, `:2207-2250`, `crates/core/src/tools/archive_rar_fixtures.rs:55-105`.
- **DEC-010 (SPEC-0085):** story order inside the spec: exact counting with the exploit tests first (US-0188), then the early check for zip and 7z (US-0189), the net growth and the order (US-0190), RAR (US-0191), the per entry cap (US-0192), the HTTP door and docs (US-0193). Global order: SPEC-0084 DEC-010. **Obliged by:** n/a (no code impact): story order. **Rationale:** exploit tests red before their fix; one writer at a time on `archive.rs`. **Alternatives considered:** early check first (the exploit tests would then be green on arrival and lose their red evidence). **Source:** security design section 8. **Evidence:** n/a.
- **DEC-011 (SPEC-0085):** tests needing the `rar` feature sit in a module whose path contains `archive_rar` (a nested `#[cfg(feature = "rar")] mod archive_rar { ... }` in `archive_declared_e2e.rs`; E2E-MOD-001 in `archive_rar.rs`), because `test.sh:7` filters the feature run on `archive_rar` and `e2e_0080_`. **Obliged by:** n/a (no code impact): test runner. **Rationale:** without it `make test` never runs them; editing `test.sh` per spec adds a file to every story that adds one. **Alternatives considered:** extending the `test.sh` filter. **Source:** existing code. **Evidence:** `test.sh:7`.
### Data model
None.
### Contracts
`archive.rs` (`:247-262`): `entry_targets` carry `i64::try_from(e.declared_size).unwrap_or(i64::MAX)`; `total_declared` is a saturating sum (`fold(0i64, i64::saturating_add)`). `quota.rs`: `admit_paths` accumulates `delta = delta.saturating_add(new_size.saturating_sub(previous))`; `check_and_charge` compares `used.saturating_add(delta) > limit`. `safety.rs` `charge_write`: refuses when `s.bytes_written.saturating_add(num_bytes) > quota`, adds `num_bytes.max(0)`. Messages unchanged: `project storage quota of 1 MB exceeded: 1047576 bytes used, 9223372036854775807 bytes requested`, `session write quota of <N> bytes exceeded`. New test module `crates/core/src/tools/archive_declared_e2e.rs` (`#[cfg(test)] mod archive_declared_e2e;` in `tools/mod.rs`). Testkit additions: `limit_mb_remaining`, `add_second_member(f) -> &'static str` (`noa@example.com`), `zip_declaring(entries: &[(&str, u64, &[u8])]) -> Vec<u8>` (DEC-009: deflated data via `flate2::write::DeflateEncoder`, CRC32 of the real content via `flate2::Crc`, local headers, central directory, end record; a declared size of `0xFFFF_FFFF` or more writes `0xFFFF_FFFF` in both headers plus a zip64 extra field `0x0001` holding the uncompressed then compressed size, version needed 45).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/quota.rs` | modify |
| `crates/core/src/safety.rs` | modify |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/tools/archive_testkit.rs` | modify |
| `crates/core/src/tools/archive_declared_e2e.rs` | create |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Remaining storage with exact messages: `limit_mb_remaining(f, n)` sets a limit of 1 048 576 bytes (`f.state.admin.set_quota(MOUNT, Some(1_048_576))`) and writes `/filler.bin` so that exactly `n` bytes remain (the archive counts as stored), hence `<U>` = 1 048 576 minus `n`; `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB. Archive builders: `build_zip` (`crates/core/src/tools/archive.rs:1104-1121`), `build_sevenz_ext` (`:2207-2250`), `rar5_store` and `RarEntry` (`crates/core/src/tools/archive_rar_fixtures.rs:55-105`). Saturating counters already used in the tree: `refund_write` (`safety.rs:161-165`, clamps at 0).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0085/E2E-SEC-001: Size of 2^63 against the storage limit
- **Covers:** class sweep occurrences 1 and 2, zip (Section 14.2)
- **Red before the fix because:** `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: today the call answers 1 file and 10 000 000 bytes written.
- **Given** 1 000 bytes of remaining storage and `/uploads/free.zip` whose entry `free.bin` declares, in its zip64 record, `9223372036854775808` bytes and holds 10 000 000 zero bytes, with a matching checksum
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 9223372036854775807 bytes requested`
- **And** the operator's view of the project's storage still reads 1 047 576 bytes used, and `/uploads/free/free.bin` does not exist when listed by a second member of `atlas`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_sec_001_size_of_2_63_against_the_storage_lim`
- Red before the change because: `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: `archive.rs:251` casts 2^63 to `i64::MIN`, the net change is negative, the project check is skipped (`delta > 0` false) and the call answers 1 file, 10 000 000 bytes written. If the run shows `zip` 9 refusing the short zip64 entry itself (spec 14.3, ASSUMED), record that refusal as the control and keep the test as a guard
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-SEC-002: Size of 2^63 against the session allowance
- **Covers:** occurrences 1 and 3, zip
- **Red before the fix because:** `Then the refusal is ERR_WRITE_QUOTA_EXCEEDED` fails: today the call succeeds, and the later 1 001 byte write succeeds because the used allowance went below zero.
- **Given** a project without a storage limit, session allowance 1 000, and `/uploads/free.zip` of E2E-SEC-001
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 1000 bytes exceeded`
- **And** a following write of a 1 001 byte file `/uploads/w.bin` is refused with `session write quota of 1000 bytes exceeded`, and a write of a 1 000 byte file succeeds
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_sec_002_size_of_2_63_against_the_session_all`
- Red before the change because: `Then the refusal is ERR_WRITE_QUOTA_EXCEEDED` fails: the negative charge passes `charge_write` and lowers `bytes_written`, so the call succeeds and the later 1 001 byte write succeeds too (same control rule as E2E-SEC-001)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-SEC-003: The largest declarable size
- **Covers:** occurrence 1, zip
- **Red before the fix because:** `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: the size counts as minus 1 byte today and the call writes `hello`.
- **Given** 1 000 bytes of remaining storage and `/uploads/max.zip` whose entry `a.txt` declares `18446744073709551615` bytes and holds `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9223372036854775807 bytes requested`
- **And** the operator's view of the project's storage still reads 1 047 576 bytes used
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_sec_003_the_largest_declarable_size`
- Red before the change because: `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: 18446744073709551615 casts to minus 1, the call writes `hello`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-SEC-004: Two sizes whose sum wraps
- **Covers:** occurrence 2, zip
- **Red before the fix because:** `Then the refusal is ERR_WRITE_QUOTA_EXCEEDED` fails: today the sum of the two sizes overflows: the call fails with an internal error or counts a negative total and writes both files.
- **Given** a project without a storage limit and `/uploads/wrap.zip` holding `a.txt` (`a`) and `b.txt` (`b`), each declaring `9223372036854775807` bytes in its zip64 record
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** a following write of a 52 428 800 byte file succeeds, so nothing was charged; `/uploads/wrap` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_sec_004_two_sizes_whose_sum_wraps`
- Red before the change because: `Then the refusal is ERR_WRITE_QUOTA_EXCEEDED` fails: the sum of two 9223372036854775807 sizes overflows at `archive.rs:252` (a panic in the debug test build, an internal error) or counts negative and writes both files
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: no early check yet (US-0189): the archive is still decoded before the admission refuses; nothing about 7z here (E2E-SEC-005 is US-0189)
- Security: each exploit test carries its literal payload in the header built by `zip_declaring` and asserts both the rejection and the absence of effect through the second channels named in the Independent Test

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-SEC-004), before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive tools::quota_e2e` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted`, `spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing`, `spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted`, `spec_0020_e2e_051_an_extraction_racing_a_write` (`quota_e2e.rs`), `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs`), every `e2e_0084_` test; no change to the tool contract; every caller of `admit_paths` and `charge_write` (writes, copies, git) sees identical results for sizes below `i64::MAX`
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0188: count declared archive sizes exactly, never negative`, body `Spec: SPEC-0085` and the requirement ids (SPEC-0085/FR-NEW-005, FR-NEW-006).
