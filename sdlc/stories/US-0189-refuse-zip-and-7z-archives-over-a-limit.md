# US-0189: refuse zip and 7z archives over a limit before decoding

> Spec: SPEC-0085 (sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0188
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A zip or 7z archive whose declared sizes exceed the remaining storage or the session allowance is refused before a single entry is decoded, with the exact figures of the final admission. 7z entries with an escaping name, a link or a special type are refused before decoding too, as zip and RAR already do.

## Requirements covered
- SPEC-0085/FR-NEW-001 (partial: zip and 7z): WHEN a member extracts a `.zip`, `.7z` or `.rar` archive, in any letter case, THE archive extraction SHALL compute, from the archive's headers and before decoding any entry, the declared total and the net growth, and check them against the two limits.
- SPEC-0085/FR-NEW-002 (partial: zip and 7z): IF the project has a storage limit AND the declared net growth exceeds the remaining storage THEN THE archive extraction SHALL refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and the message `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested`.
- SPEC-0085/FR-NEW-003 (partial: zip and 7z): IF FR-NEW-002 did not refuse AND the declared total exceeds the member's remaining session write allowance THEN THE archive extraction SHALL refuse with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`.
- SPEC-0085/FR-NEW-004 (partial: zip and 7z): THE archive extraction SHALL NOT decode any entry, write any file, create any folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused under FR-NEW-002, FR-NEW-003 or FR-NEW-005.
- SPEC-0085/FR-NEW-005 (partial: 7z: E2E-SEC-005): IF one declared size, the declared total or the declared net growth exceeds 9 223 372 036 854 775 807 bytes THEN THE archive extraction SHALL refuse as FR-NEW-002 states when the project has a storage limit, with `<R>` reported as `9223372036854775807`, and as FR-NEW-003 states otherwise.
- SPEC-0085/FR-NEW-008 (partial: the decoded bytes held hook and 0 for a refused call): THE archive extraction SHALL hold, during one extraction of a `.zip`, `.7z` or `.rar` archive, at most the declared total plus 65 536 bytes of decoded entry data.
- SPEC-0085/FR-MOD-001 (partial: zip and 7z): IF the declared net growth or the declared total of a `.zip`, `.7z` or `.rar` archive exceeds its limit THEN THE archive extraction SHALL refuse before decoding any entry, before the destination check and before the collision check.
- SPEC-0085/FR-MOD-002: IF any entry of a `.7z` archive has an escaping name, a link type or another special type THEN THE archive extraction SHALL refuse it before decoding any entry.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0085_001 e2e_0085_003 e2e_0085_006 e2e_0085_007 e2e_0085_009 e2e_0085_010 e2e_0085_011 e2e_0085_012 e2e_0085_029 e2e_0085_033 e2e_0085_034 e2e_0085_039 e2e_0085_041 e2e_0085_sec_005`: 14 tests pass. Real assertion: `held()` reads 0 (no decoder ran) for every refusal before decoding, while `quota::used_bytes` and `session_remaining` are unchanged.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0085):** zip, 7z and RAR go through list, then `check_declared`, then decode, each blocking step in `spawn_blocking`; the listing applies every header refusal of FR-NEW-009 item 5 in the spec's per format order (zip: unreadable, per entry link / escaping / duplicate, missing password, local header disagreement; 7z: encrypted or unreadable header, missing password, per entry escaping then special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then missing password for encrypted data); the decode step reopens zip and 7z from the same bytes. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-004, FR-NEW-009, FR-MOD-001. **Rationale:** headers are all the early check needs (`by_index_raw` gives `size()` with no password, `archive.rs:506-527`; `Archive::read` gives `archive.files[i].size`, `archive.rs:597`; RAR's listing gives `unpacked_size`, `archive_rar.rs:36-41`), so nothing is decoded before the limits. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** as cited.
- **DEC-002 (SPEC-0085):** `quota::check_declared` computes the declared total (`u128`) and the declared net growth (`i128`, last occurrence per target path, minus the stored file size, whatever `overwrite` says, 0 for every path when `count_stored` is false), treats any size, total or growth above `i64::MAX` as too large, and refuses: project limit and (too large or `used + growth > limit` with `growth > 0`) → `project_quota_exceeded(limit, used, min(growth, i64::MAX))`, `i64::MAX` when too large; else (no limit and too large) or `total > session_remaining` → the session refusal; equality accepted; nothing charged; the admission lock is taken only inside `quota::used_bytes`. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-005, FR-NEW-006. **Rationale:** the exact figures of today's admission (`quota.rs:136-159`) computed without wrap and without charging; project first and the tie to the project as FR-NEW-003 says. **Alternatives considered:** reusing `admit_paths` with a dry run flag (it takes the lock and charges; a flag doubles its contract). **Source:** design. **Evidence:** `crates/core/src/quota.rs:96-159`, `crates/core/src/safety.rs:136-154`.
- **DEC-004 (SPEC-0085):** decoded bytes held: the decode step keeps a running total of decoded entry bytes and its maximum in a `&mut u64` it returns with its result (success or error); the async side sets `archive_single::LAST_HIGH_WATER` to it under `cfg(test)`; a call refused before decoding never sets it, so `held()` reads 0. **Obliged by:** SPEC-0085/FR-NEW-004, FR-NEW-008, NFR 7.1. **Rationale:** the one hook of SPEC-0084 DEC-006, set on the async thread (the blocking thread differs). **Alternatives considered:** an `Arc<AtomicU64>`. **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:56-61`.
- **DEC-006 (SPEC-0085):** the 7z entry checks (escaping name, then link or special type, in archive order) move from inside `for_each_entries` (`archive.rs:637-647`) to the listing over `archive.files` after `Archive::read` and the missing password gate. **Obliged by:** SPEC-0085/FR-MOD-002, FR-NEW-009 item 5. **Rationale:** `archive.files` holds every entry with its attributes before any block is decoded; the same `ensure_entry_path_safe` and `sevenz_special_type` run on it. **Alternatives considered:** none: the spec moves them. **Source:** spec. **Evidence:** `crates/core/src/tools/archive.rs:593-683`.
- **DEC-008 (SPEC-0085):** target paths for the early check are joined to the planned destination of SPEC-0084 DEC-003 (`join_entry_destination`, `archive.rs:369-372`); when it is `None` (the destination will be refused), `count_stored` is false and the paths are joined to `/` only to keep last occurrence semantics. **Obliged by:** SPEC-0085/FR-NEW-001 business rule, spec D-6. **Rationale:** one destination resolution for the two bounds; SPEC-0086 later replaces it by the real destination known before reading. **Alternatives considered:** a second resolution. **Source:** SPEC-0084 DEC-003. **Evidence:** `crates/core/src/tools/archive.rs:213-227`.
### Data model
In memory only: `ListedEntry { rel_path: String, is_dir: bool, declared: u64 }`; `Listing { entries: Vec<ListedEntry>, any_encrypted: bool }`.
### Contracts
`quota.rs`: `pub(crate) async fn check_declared(client: &VolumeClient, safety: &SafetyManager, person: &str, mount_id: &str, files: &[(String, u64)], count_stored: bool) -> Result<()>` (DEC-002: total `u128`, growth `i128` over the last occurrence per path minus the stored file size, too large above `i64::MAX`, project refusal `project_quota_exceeded(limit, used, min(growth, i64::MAX))` first, then the session refusal; equality accepted; charges nothing). `archive.rs`: `fn list_zip(bytes, password) -> Result<Listing>` and `fn list_sevenz(bytes, password) -> Result<Listing>` (header refusals of FR-NEW-009 item 5 in the spec's order; the 7z entry checks run over `archive.files` after `Archive::read` and the missing password gate), `fn decode_zip_listed(..)`, `fn decode_sevenz_listed(..)` (reopen from the bytes; a running total and maximum of decoded entry bytes in `held: &mut u64`); in `extract_archive` zip and 7z go: planned destination, read bytes, `spawn_blocking` listing, `check_declared(.., count_stored = planned.is_some())`, `spawn_blocking` decode, hook (`LAST_HIGH_WATER`, `cfg(test)`), then destination, collision, `admit_paths` unchanged. RAR keeps its current path until US-0191. Testkit additions: `sevenz_declaring(name: &str, declared: u64, data: &[u8]) -> Vec<u8>` (DEC-009: Copy coder, unpack size `declared`, no CRC digest, plain header, start header CRC), `zip_bomb()`, `sevenz_bomb()`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/quota.rs` | modify |
| `crates/core/src/tools/archive_testkit.rs` | modify |
| `crates/core/src/tools/archive_declared_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). Remaining storage with exact messages: `limit_mb_remaining(f, n)` sets a limit of 1 048 576 bytes (`f.state.admin.set_quota(MOUNT, Some(1_048_576))`) and writes `/filler.bin` so that exactly `n` bytes remain (the archive counts as stored), hence `<U>` = 1 048 576 minus `n`; `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB. Archive builders: `build_zip` (`crates/core/src/tools/archive.rs:1104-1121`), `build_sevenz_ext` (`:2207-2250`), `rar5_store` and `RarEntry` (`crates/core/src/tools/archive_rar_fixtures.rs:55-105`). Today's zip and 7z decoders to split: `decode_zip` (`archive.rs:490-591`), `decode_sevenz` (`:593-683`); the figures to mirror: `admit_paths` (`quota.rs:136-159`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0085/E2E-001: zip within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-010; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/pack.zip` holding `a.txt` (`hello`); nothing at `/uploads/pack`
- **When** the member extracts `/uploads/pack.zip`
- **Then** the answer is destination `/uploads/pack`, 1 file written, 1 folder created, 5 bytes written
- **And** `/uploads/pack/a.txt` reads `hello`; the remaining storage is 995 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_001_zip_within_the_limits`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-003: Net growth equal to the remaining storage
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 5 bytes of remaining storage and `/uploads/pack.zip` of E2E-001
- **When** the member extracts it
- **Then** the extraction succeeds with 5 bytes written; the remaining storage is 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_003_net_growth_equal_to_the_remaining_st`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-006: zip bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001; **Priority:** Critical
- **Covers:** class sweep occurrence 4 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the limit is checked.
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.zip`, a bomb of at most 20 000 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 10000000 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes; the audit trail has no extraction line for `/uploads/bomb.zip`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_006_zip_bomb_against_the_storage_limit`
- Red before the change because: after step 1 (hook wired, no early check) `the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the admission refuses (the message itself already matches)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-007: 7z bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-004; **Priority:** Critical
- **Covers:** class sweep occurrence 4 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today every entry is decoded, 10 000 000 bytes, before the limit is checked.
- **Given** session allowance 1 000 and `/uploads/bomb.7z`, a bomb
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`
- **And** the decoded bytes held are 0; `/uploads/bomb` does not exist; a following write of a 1 000 byte file `/uploads/small.bin` succeeds
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_007_7z_bomb_against_the_session_allowanc`
- Red before the change because: after step 1, `held()` is 10 000 000, not 0
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-009: Tie between the two limits
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** 1 000 bytes of remaining storage, session allowance 1 000, and `/uploads/bomb.zip` of E2E-006
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both are unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_009_tie_between_the_two_limits`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-010: The session allowance is lower
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** a storage limit of 3 MB with 2 000 000 bytes remaining, session allowance 1 000, and `/uploads/mid.zip` holding `a.bin`, 1 500 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 1000 bytes exceeded`
- **And** the decoded bytes held are 0; the remaining storage is still 2 000 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_010_the_session_allowance_is_lower`
- Red before the change because: after step 1, `held()` is 1 500, not 0 (the session refusal comes from the admission after decoding)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-011: One byte over the remaining storage
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/over.zip` holding `a.bin`, 1 001 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 1001 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/over` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_011_one_byte_over_the_remaining_storage`
- Red before the change because: after step 1, `held()` is 1 001, not 0
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-012: Entries that each fit but not together
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/three.zip` holding `a.bin`, `b.bin` and `c.bin`, 400 bytes each
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with the message `project storage quota of 1 MB exceeded: 1047576 bytes used, 1200 bytes requested`
- **And** the decoded bytes held are 0; `/uploads/three` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_012_entries_that_each_fit_but_not_togeth`
- Red before the change because: after step 1, `held()` is 1 200, not 0
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-029: A 7z link entry wins over the limit
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.7z` holding `zeros.bin` of a bomb, then a symbolic link entry `link`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `archive entry 'link' is a symlink, which is not supported`
- **And** the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_029_a_7z_link_entry_wins_over_the_limit`
- Red before the change because: after step 1, `held()` is 10 000 000: today `zeros.bin` is decoded before the link entry is met inside `for_each_entries` (`archive.rs:637-647`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-033: A 7z escaping entry wins over earlier damaged data
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/mix.7z` holding `a.txt` (`hello`) with its compressed data damaged, then an entry named `../evil.txt` (`x`), the two in separate blocks
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS` with `archive entry path escapes the destination: '../evil.txt'`, not the corrupt refusal
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_033_a_7z_escaping_entry_wins_over_earlie`
- Red before the change because: today the damaged `a.txt` is decoded before `../evil.txt` is checked, so the refusal is the corrupt one
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-034: A 7z special entry wins over a wrong password
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009, FR-MOD-002; **Priority:** Medium
- **Given** `/uploads/vault.7z` encrypted with `s3cret`, header in clear, holding `a.txt` (`hello`) then a symbolic link entry `link`
- **When** the member extracts it with the password `nope`
- **Then** the refusal is `ERR_NOT_SUPPORTED` with `archive entry 'link' is a symlink, which is not supported`, not `ERR_PASSWORD_REQUIRED`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_034_a_7z_special_entry_wins_over_a_wrong`
- Red before the change because: today `a.txt` is decoded with `nope` before the link is checked, so the refusal is `ERR_PASSWORD_REQUIRED`
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-039: A repeated refusal stays a refusal and changes nothing
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** Medium
- **Given** the situation of E2E-006
- **When** the member extracts `/uploads/bomb.zip` twice
- **Then** both refusals carry the message of E2E-006
- **And** the remaining storage is still 1 000 bytes; extracting `/uploads/pack.zip` of E2E-001 right after succeeds and leaves 995 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_039_a_repeated_refusal_stays_a_refusal_a`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-041: Solid 7z bomb counted by its entries
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/solid.7z` holding `a.bin`, `b.bin` and `c.bin`, 4 000 000 zero bytes each, in one compressed block
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `12000000 bytes requested`; the decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_041_solid_7z_bomb_counted_by_its_entries`
- Red before the change because: after step 1, `held()` is 12 000 000, not 0
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0085/E2E-SEC-005: 7z size of 2^63
- **Covers:** occurrence 1, 7z
- **Red before the fix because:** `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: today the call writes `free.bin` (Section 14.3 asks the design to confirm the fixture reproduces it).
- **Given** 1 000 bytes of remaining storage and `/uploads/free.7z` whose entry `free.bin` declares `9223372036854775808` bytes with no checksum and holds `hello`
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message ending `9223372036854775807 bytes requested`
- **And** the operator's view of the project's storage still reads 1 047 576 bytes used; `/uploads/free` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_declared_e2e.rs
- Function: `e2e_0085_sec_005_7z_size_of_2_63`
- Red before the change because: `Then the refusal is ERR_PROJECT_QUOTA_EXCEEDED` fails: today the call writes `free.bin` (spec), or the 7z reader refuses the 2^63 size as a damaged block; in that second case record the refusal as the control and keep the test as a guard (spec 14.3)
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-001 | make `decode_zip_listed` return every entry with empty bytes: `/uploads/pack/a.txt` is written empty, the test fails |
| E2E-003 | in `check_declared` refuse a growth equal to the remaining storage (`>=` for `>`): the call is refused before decoding, the test fails |
| E2E-009 | check the session before the project in `check_declared`: the refusal is `ERR_WRITE_QUOTA_EXCEEDED`, the test fails |
| E2E-039 | run the writes before `check_declared` (move the decode and write pass up): the first call writes `zeros.bin`, the remaining storage changes, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- The memory hook is the `cfg(test)` thread local `archive_single::LAST_HIGH_WATER`; set it on the async thread after `spawn_blocking` returns (the blocking thread is another thread), and keep tests on the default single threaded `#[tokio::test]`
- No dash as punctuation in code, comments or docs
- The early check never holds the admission lock across decoding and never charges; the final `admit_paths` stays where it is (`archive.rs:247-262`)
- Scope boundary: RAR is US-0191; the per entry cap is US-0192; the destination refusal keeps its place after decoding

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-006, E2E-007, E2E-010, E2E-011, E2E-012, E2E-029, E2E-033, E2E-034, E2E-041, E2E-SEC-005), before the production code (step 1: wire the decoded bytes hook of DEC-004 into the zip and 7z decode and run the tests: E2E-006, 007, 010, 011, 012, 029, 041 are red on their `held()` assertion, 033 and 034 on their code; step 2: the listing and `check_declared`).
- [ ] Green on arrival tests (E2E-001, E2E-003, E2E-009, E2E-039): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive tools::quota_e2e` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted`, `spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing`, `spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted`, `spec_0020_e2e_051_an_extraction_racing_a_write` (`quota_e2e.rs`), `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs`), every `e2e_0084_` test; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0189: refuse zip and 7z archives over a limit before decoding`, body `Spec: SPEC-0085` and the requirement ids (SPEC-0085/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-008, FR-MOD-001, FR-MOD-002).
