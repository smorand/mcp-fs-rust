# US-0184: stop a compressed tar bomb at the decoding bound

> Spec: SPEC-0084 (sdlc/specs/SPEC-0084-tar-family-bounded-decoding/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0183
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
A small `.tar.gz`, `.tar.bz2` or `.tar.xz` that decompresses to megabytes is stopped as soon as the decompressed archive passes the lower of the remaining storage and the session allowance plus 1 048 576 bytes, and refused with that limit's error, writing and charging nothing. Today the whole archive is decompressed in memory first.

## Requirements covered
- SPEC-0084/FR-NEW-001: IF the decompressed archive of a `.tar.gz`, `.tgz`, `.tar.bz2`, `.tb2`, `.tar.xz` or `.txz` file, in any letter case, grows past the decoding bound THEN THE archive extraction SHALL stop decoding at that point and refuse the call as FR-NEW-003 states.
- SPEC-0084/FR-NEW-002 (partial: the bound and both sides; the stored files under the destination are exercised by US-0185): THE archive extraction SHALL set, before decoding a compressed tar archive, a decoding bound equal to the lower of the project side and the session side, plus a framing allowance of 1 048 576 bytes.
- SPEC-0084/FR-NEW-003: IF decoding stops at the decoding bound THEN THE archive extraction SHALL refuse with `ERR_PROJECT_QUOTA_EXCEEDED` and the message `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested` when the project side is lower than or equal to the session side, and otherwise with `ERR_WRITE_QUOTA_EXCEEDED` and the message `session write quota of <N> bytes exceeded`.
- SPEC-0084/FR-NEW-004: THE archive extraction SHALL NOT write any file, create any folder, charge the remaining storage or the session write allowance, or record an audit line, for a call refused at the decoding bound.
- SPEC-0084/FR-MOD-001 (partial: the bound before the entry checks, shown by E2E-008, E2E-010, E2E-046; before collision and links: US-0185): IF a compressed tar archive decompresses past the decoding bound THEN THE archive extraction SHALL refuse it at that point, before its entries are checked, before its collisions are checked and before the limits are checked on the files' sizes.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0084_004 e2e_0084_005 e2e_0084_006 e2e_0084_008 e2e_0084_009 e2e_0084_010 e2e_0084_011 e2e_0084_012 e2e_0084_013 e2e_0084_014 e2e_0084_015 e2e_0084_021 e2e_0084_046`: 13 tests pass. Real assertions: the hook `LAST_HIGH_WATER` set by the real decoder stays at or below 1 049 576 for each 10 000 000 byte bomb, and `quota::used_bytes` and `session_remaining` are unchanged after each refusal.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0084):** the bound reuses the single file machinery: `read_capped` (64 KiB chunks, refuses as soon as the output would pass the cap, equality accepted) and one cap selector `pick_caps(project: Option<u64>, session: u64, framing: u64) -> Caps` that both the single flow (framing 0) and the tar flow (framing `TAR_FRAMING` = 1 048 576) call; `Caps::refusal` reports the project refusal with `requested` = cap + 1 and `used` read at refusal time, or the session refusal. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-002, FR-NEW-003. **Rationale:** one cap rule and one reader for every compressed format; `requested = limit + 1` is exactly FR-NEW-003's `<R>` = bound + 1 once the framing is inside the cap. **Alternatives considered:** a second counting reader in `archive.rs`; streaming the tar parse (rejected in section 2). **Source:** existing code. **Evidence:** `crates/core/src/tools/archive_single.rs:92-108`, `:166-221`.
- **DEC-002 (SPEC-0084):** the project side is `quota::project_remaining_for(client, safety, stored)` where `stored = quota::stored_under(client, dest)` (sum of `size` over the file nodes of `meta.subtree(dest)`), for `overwrite` true and false; `stored = 0` when the planned destination is `None` (refused). No project limit: `None`, the session side alone sets the cap. The session side is `safety.session_remaining(person, mount_id)`. **Obliged by:** SPEC-0084/FR-NEW-002 (and its business rule on a refused destination). **Rationale:** `meta.subtree` already selects a folder and everything below it with the escaped `LIKE` plus exact match (`SubtreeMatch`, AGENTS.md conventions), so no new SQL; `project_remaining_for` already adds a replaced size and floors at 0. **Alternatives considered:** `folder_usage` (direct items only, needs a cap); a new SUM query (new SQL across two dialects for no gain). **Source:** existing code. **Evidence:** `crates/core/src/quota.rs:165-178`, `crates/core/src/storage/meta.rs:1868`, `crates/core/src/safety.rs:151-154`.
- **DEC-003 (SPEC-0084):** `planned_destination(state, format, normalized, suffix_len, destination) -> Option<String>` resolves the destination before reading with today's rules (RAR: `archive_single::normalize_destination`; zip, 7z, tar: `state.safety.normalize_path`; none given: the archive path minus its suffix) and returns `None` instead of an error; it only sizes the bound. The destination refusal keeps its place after the entry checks (FR-NEW-005 item 7), computed as today at `archive.rs:219-227`. **Obliged by:** SPEC-0084/FR-NEW-002, FR-NEW-005. **Rationale:** the bound needs the destination before decoding while its refusal must not move (spec 14.3); SPEC-0086 later moves the refusal and removes this helper (SPEC-0086 DEC-002). **Alternatives considered:** moving the refusal now (changes FR-NEW-005 item 7, an observable order this spec keeps). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:213-227`.
- **DEC-005 (SPEC-0084):** the bounded tar decode runs in one `spawn_blocking` task that decompresses with `read_capped(tar_decoder(c, &bytes)?, bound.limit)` then parses with the existing entry loop; it returns either the entries and the high water, or `CapExceeded { high_water }`, or a `ToolError`. On `CapExceeded` the async side sets the hook and returns `caps.refusal(state, &client).await?`. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-005 (item 5: corrupt or bound, whichever decoding meets first). **Rationale:** the order "corrupt or bound, whichever comes first" falls out of one sequential read; nothing is written, charged or audited before `admit_paths` and `finish_extraction`, which a refusal never reaches (FR-NEW-004). **Alternatives considered:** two tasks (decompress, then parse): one more hop for nothing. **Source:** design. **Evidence:** `crates/core/src/tools/archive_single.rs:254-277`.
- **DEC-006 (SPEC-0084):** "decompressed bytes held" is read through the one existing test hook `archive_single::LAST_HIGH_WATER` (`cfg(test)` thread local), set on the async thread after the blocking task returns, with `high_water` of the capped read (the bytes held when the read stopped, never above the cap) or the decompressed length on success; a call refused before decoding never sets it. Tests reset it before the call and read it with `held()` (0 when unset) or `held_some()` (fails when unset: the decoder never ran). **Obliged by:** SPEC-0084/FR-NEW-001, NFR 7.1 (E2E-008 to E2E-011, E2E-018, E2E-046). **Rationale:** one hook for every format (`.agent_docs/testing.md:432-434`); the tokio test runtime is single threaded, so the thread local belongs to one test; setting it on the async thread avoids the `spawn_blocking` thread mismatch that hides per thread captures (US-0175 commit note). **Alternatives considered:** a second hook in `archive.rs`; process RSS (needs `unsafe` or a `ps` race, SPEC-0082 DEC-006). **Source:** existing code. **Evidence:** `crates/core/src/tools/archive_single.rs:56-61`, `:862-866`.
### Data model
None.
### Contracts
`archive.rs`: `pub(crate) const TAR_FRAMING: u64 = 1_048_576;`; `fn planned_destination(state: &AppState, format: ArchiveFormat, normalized: &str, suffix_len: usize, destination: Option<&str>) -> Option<String>`; `fn tar_decoder<'a>(c: TarCompression, bytes: &'a [u8]) -> std::io::Result<Box<dyn Read + 'a>>` (Gzip `GzDecoder`, Bzip2 `BzDecoder`, Xz `archive_single::decoder(SingleFormat::Xz, ..)`); `decode_tar` takes the decompressed bytes. For `Tar(c)` with `c != None`: `caps = archive_single::pick_caps(project_side, session_side, TAR_FRAMING)` computed before `read_bytes`, then one `spawn_blocking` that runs `read_capped(tar_decoder(c, &bytes)?, caps.limit)` and parses; `ReadError::CapExceeded { high_water }` sets the hook and returns `caps.refusal(state, &client).await?`; `ReadError::Io(e)` is `corrupt(<format>, e)`. `archive_single.rs`: `pub(crate) struct Caps { pub(crate) limit: u64, pub(crate) which: CapKind }`, `pub(crate) enum CapKind`, `pub(crate) fn pick_caps(project: Option<u64>, session: u64, framing: u64) -> Caps` (`Some(p) if p <= session` gives `Project` with `p.saturating_add(framing)`, else `Session` with `session.saturating_add(framing)`), `caps` (single flow) now calls `pick_caps(.., 0)`; `read_capped`, `CappedRead`, `ReadError` become `pub(crate)`. `quota.rs`: `pub(crate) async fn stored_under(client: &VolumeClient, folder: &str) -> Result<i64>` (sum of `size` of the file nodes of `client.meta.subtree(folder)`). Refusals: `ERR_PROJECT_QUOTA_EXCEEDED` `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested` (`<R>` = bound + 1) and `ERR_WRITE_QUOTA_EXCEEDED` `session write quota of <N> bytes exceeded`. Testkit additions: `tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8>`, `gz(bytes)`, `bz2(bytes)`, `xz(bytes)`, `tar_bomb() -> Vec<u8>` (tar of `zeros.bin`, 10 000 000 zero bytes), `held_at_most(max)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_single.rs` | modify |
| `crates/core/src/quota.rs` | modify |
| `crates/core/src/tools/archive_testkit.rs` | modify |
| `crates/core/src/tools/archive_tar_bound_e2e.rs` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). The single flow this story mirrors: `extract_single` (`crates/core/src/tools/archive_single.rs:227-288`): caps before `read_bytes`, `spawn_blocking` around `decoder` plus `read_capped`, the three `ReadError` arms, `LAST_HIGH_WATER` set at `:269` and `:275`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0084/E2E-004: Files equal to the remaining storage
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-011; **Priority:** High
- **Given** 5 bytes of remaining storage and `/uploads/pack.tar.gz` holding `a.txt` (`hello`), whose decompressed archive is larger than 5 bytes because of the tar bookkeeping
- **When** the member extracts it
- **Then** the extraction succeeds with 5 bytes written
- **And** the remaining storage is 0 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_004_files_equal_to_the_remaining_storage`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-005: Files over the remaining storage, archive within the bound
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** 4 bytes of remaining storage and `/uploads/pack.tar.gz` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** `/uploads/pack` does not exist and the remaining storage is still 4 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_005_files_over_the_remaining_storage_arc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-006: Files over the session allowance, archive within the bound
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** a session allowance of 4 bytes and `/uploads/pack.tar.xz` holding `a.txt` (`hello`)
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 4 bytes exceeded`
- **And** `/uploads/pack` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_006_files_over_the_session_allowance_arc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-008: `.tar.gz` bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-MOD-001; **Priority:** Critical
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.gz`, a file of at most 20 000 bytes whose decompressed archive holds `zeros.bin`, 10 000 000 zero bytes, declared as such
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with a message starting `project storage quota of`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_008_tar_gz_bomb_against_the_storage_limi`
- Red before the change because: `held_at_most(1_049_576)` fails: the tar path never sets the hook today (the bomb is decoded whole by `read_to_end`, `archive.rs:727-749`), so `held_some` reports that no capped decode ran
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-009: `.tar.bz2` bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-004; **Priority:** Critical
- **Given** a project without a storage limit, a session allowance of 1 000 bytes, and `/uploads/bomb.tar.bz2` holding `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/bomb` does not exist; a following write of a 1 000 byte file `/uploads/small.bin` succeeds
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_009_tar_bz2_bomb_against_the_session_all`
- Red before the change because: `held_at_most(1_049_576)` fails as in E2E-008 (no capped decode on the tar path)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-010: `.tar.xz` bomb against the storage limit
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** Critical
- **Given** 1 000 bytes of remaining storage and `/uploads/bomb.tar.xz`, a single xz stream holding `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/bomb` does not exist; the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_010_tar_xz_bomb_against_the_storage_limi`
- Red before the change because: `held_at_most(1_049_576)` fails as in E2E-008
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-011: `.TXZ` bomb against the session allowance
- **Category:** failure, security; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** Critical
- **Given** no storage limit, a session allowance of 1 000 bytes and `/uploads/BOMB.TXZ` holding `zeros.bin` as in E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/BOMB` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_011_txz_bomb_against_the_session_allowan`
- Red before the change because: `held_at_most(1_049_576)` fails as in E2E-008
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-012: Tie between the two sides
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** 1 000 bytes of remaining storage, a session allowance of 1 000 bytes, and `/uploads/bomb.tar.gz` of E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not `ERR_WRITE_QUOTA_EXCEEDED`; both limits are unchanged
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_012_tie_between_the_two_sides`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-013: The session side is lower
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** 2 000 000 bytes of remaining storage, a session allowance of 1 000 bytes, and `/uploads/bomb.tar.gz` of E2E-008
- **When** the member extracts it
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 1000 bytes exceeded`; the remaining storage is still 2 000 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_013_the_session_side_is_lower`
- Red before the change because: today the whole bomb is decoded and the admission checks the project first: 10 000 000 declared bytes exceed the 2 000 000 remaining, so the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`, not the expected session refusal
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-014: Decompressed archive exactly at the bound
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** 1 000 bytes of remaining storage, nothing stored under `/uploads/fit`, and `/uploads/fit.tar.gz` whose decompressed archive is exactly 1 049 576 bytes: `a.txt` (`hello`), the end marker, then zero bytes
- **When** the member extracts it
- **Then** the extraction succeeds: 1 file written, 5 bytes written; the remaining storage is 995 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_014_decompressed_archive_exactly_at_the`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-015: One byte over the bound
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** the situation of E2E-014 with `/uploads/fit.tar.gz` replaced by one whose decompressed archive is 1 049 577 bytes (one more zero byte)
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`; `/uploads/fit` does not exist; the remaining storage is still 1 000 bytes
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_015_one_byte_over_the_bound`
- Red before the change because: today only the declared 5 bytes of `a.txt` are admitted, so the 1 049 577 byte archive extracts and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-021: A refusal at the bound is not audited
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** the situation of E2E-008
- **When** the member extracts `/uploads/bomb.tar.gz` and is refused
- **Then** the audit trail has no extraction line for `/uploads/bomb.tar.gz`
- **And** extracting `/uploads/pack.tar.gz` of E2E-001 right after succeeds and leaves 995 bytes of remaining storage
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_021_a_refusal_at_the_bound_is_not_audite`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0084/E2E-046: Bytes after the end marker count
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-MOD-001; **Priority:** High
- **Given** 1 000 bytes of remaining storage and `/uploads/tail.tar.gz` whose decompressed archive holds `a.txt` (`hello`), the end marker, then 10 000 000 zero bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED`
- **And** the decompressed bytes held never exceed 1 049 576; `/uploads/tail` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_tar_bound_e2e.rs
- Function: `e2e_0084_046_bytes_after_the_end_marker_count`
- Red before the change because: today the 10 000 000 bytes after the end marker are decoded and ignored by the parser, `a.txt` (5 bytes) fits the 1 000 remaining, and the call succeeds
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-004 | set `TAR_FRAMING` to 0: the bound becomes 5 bytes, below the 2 048 byte decompressed archive, the call is refused, the test fails |
| E2E-005 | in `quota::check_and_charge` skip the project check (`if false && delta > 0`): the 5 bytes are admitted against 4, the call succeeds, the test fails |
| E2E-006 | in `SafetyManager::charge_write` skip the comparison with the quota: the 5 bytes are charged against 4, the call succeeds, the test fails |
| E2E-012 | in `pick_caps` give the tie to the session (`Some(p) if p < session`): the bound refusal becomes `ERR_WRITE_QUOTA_EXCEEDED`, the test fails |
| E2E-014 | in `read_capped` refuse an output equal to the cap (`>=` for `>`): the archive of exactly 1 049 576 bytes is refused, the test fails |
| E2E-021 | in the bound refusal branch call `state.safety.record_audit(person, mount_id, "extract_archive", &normalized, "refused")` before returning: an extraction line for `/uploads/bomb.tar.gz` appears, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- The memory hook is the `cfg(test)` thread local `archive_single::LAST_HIGH_WATER`; set it on the async thread after `spawn_blocking` returns (the blocking thread is another thread), and keep tests on the default single threaded `#[tokio::test]`
- No dash as punctuation in code, comments or docs
- Scope boundary: plain `.tar` gets no bound (FR-NEW-011); the destination refusal keeps its place after the entry checks (`archive.rs:219-227`); order tests and re extraction are US-0185

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-008, E2E-009, E2E-010, E2E-011, E2E-013, E2E-015, E2E-046), before the production code.
- [ ] Green on arrival tests (E2E-004, E2E-005, E2E-006, E2E-012, E2E-014, E2E-021): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` (includes `tools::archive_single`) passes unchanged, the bare format suites `e2e_0081_`, `e2e_0082_`, `e2e_0083_` and `e2e_new_` included; `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes; no change to the tool contract; the single flow keeps its exact cap rule through `pick_caps(.., 0)` (its 160 tests pass unchanged)
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: none in this story (the closing story of the spec carries them).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0184: stop a compressed tar bomb at the decoding bound`, body `Spec: SPEC-0084` and the requirement ids (SPEC-0084/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-MOD-001).
