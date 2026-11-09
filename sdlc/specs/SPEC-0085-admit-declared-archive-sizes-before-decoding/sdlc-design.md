# SPEC-0085: Admit declared archive sizes before decoding, design

> Spec: sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: internal finding
> Designed on: 2026-11-09
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| zip listing and decoding | `zip` (`ZipArchive::by_index_raw` for the listing, `by_index`/`by_index_decrypt` to decode) | `zip = "9"` (`Cargo.toml:77`), locked 9.0.5 | DEC-001 |
| 7z listing and decoding | `sevenz_rust2::Archive::read` (listing from `archive.files`), `ArchiveReader::for_each_entries` (decoding) | `sevenz-rust2 = "0.20"` (`Cargo.toml:92`), locked 0.20.2 | DEC-001, DEC-006 |
| RAR listing and decoding | `unrar` `open_for_listing` then `open_for_processing` over one temp file | `unrar = "0.5.8"` (`Cargo.toml:95`), feature `rar` | DEC-007 |
| Capped reader | `archive_single::read_capped` (shared since SPEC-0084 DEC-001) | n/a (in tree) | DEC-005 |
| Exact arithmetic | `u128`/`i128` sums and `i64::try_from` in `quota.rs`, `safety.rs`, `archive.rs` | std | DEC-002, DEC-003 |

No new dependency. Versions from the manifests and `Cargo.lock`.

Skills loaded: `rust` (edition 2024, no `unwrap` on fallible production paths, `spawn_blocking` for decoding, clippy `--all-features -D warnings`, fmt check).

## 2. Approaches

### Chosen: list, check the declared figures, then decode under per entry caps
Split the multi format path of `extract_archive` (`crates/core/src/tools/archive.rs:202-262`) into three steps for zip, 7z and RAR: (1) a listing in `spawn_blocking` that reads headers only and applies every header refusal of FR-NEW-009 item 5; (2) an async check of the declared total and the declared net growth against the two limits (`quota::check_declared`), charging nothing and holding no lock across decoding; (3) the decode in `spawn_blocking`, each entry read through `read_capped` at its declared size, then destination, collision and the unchanged final admission (`admit_paths`). Pros: zero decoded bytes for a refused archive; the final admission still covers concurrent writes (spec D-4); one place computes the figures. Cons: zip and 7z are opened twice (headers only the first time; cheap); RAR keeps its temp file across the two blocking steps. Affected scope: `tools/archive.rs`, `tools/archive_rar.rs`, `quota.rs`, `safety.rs`.

### Rejected: move `admit_paths` before decoding
Taking the admission (and its lock) before decoding would hold the volume's admission lock across a long decode and charge the session for a call that may still fail as corrupt. Rejected: spec D-4 keeps the early check a second check, and SPEC-0020 E2E-051 needs the final admission (`quota_e2e.rs:2085-2120`).

### Rejected: a global decoded byte budget only
Counting decoded bytes against the limits during decoding (as SPEC-0084 does for tar) decodes up to the limit before refusing; FR-NEW-004 requires zero decoded bytes for an archive whose declared sizes already exceed a limit.

## 3. Architecture

```
extract_archive (archive.rs), zip / 7z / RAR
  items 1 to 4 unchanged (membership at the door, path, folder, RAR name and support)
  planned destination (SPEC-0084 DEC-003), read bytes
  spawn_blocking: list_archive(format, bytes, password) -> Listing          item 5 (header refusals)
  quota::check_declared(client, safety, person, mount, &targets, total,
                        count_stored)                                       items 6, 7 (charges nothing)
  spawn_blocking: decode_listed(listing, bytes, password, &mut held)        item 8 (per entry cap)
  hook: LAST_HIGH_WATER = held (cfg(test), async thread)
  destination refusal (unchanged place), collision, admit_paths (sizes via i64::try_from)   items 9 to 11
  writes, finish_extraction
```

- `quota.rs` owns the declared figures (`check_declared`), next to `admit_paths` whose last occurrence per path and stored size rule it mirrors without the lock and without charging.
- `archive_rar.rs`: `decode_rar` splits into `list_rar` (truncation walk, listing with entry rules, missing password gate) returning a `RarListing` that owns the temp dir, and `decode_rar_listed`.
- The tar family keeps the SPEC-0084 path (its sizes are inside the compressed stream); a plain `.tar` keeps its path.

## 4. Data model

None persisted. In memory: `ListedEntry { rel_path: String, is_dir: bool, declared: u64 }`; `Listing { entries: Vec<ListedEntry>, kind }` where `kind` carries what the decode step needs (`any_encrypted` for zip, nothing for 7z, the `RarListing` temp dir for RAR).

## 5. Contracts and interfaces

- `quota.rs`: `pub(crate) async fn check_declared(client: &VolumeClient, safety: &SafetyManager, person: &str, mount_id: &str, files: &[(String, u64)], count_stored: bool) -> Result<()>` where `files` lists `(target path, declared size)` for every file entry in archive order (folders excluded, repeats kept).
  - total = Σ declared over `files` as `u128`; growth = Σ over distinct paths (last occurrence) of `declared as i128 - stored(path)` where `stored` is the size of the file node at the path (0 for a folder, nothing, or `!count_stored`).
  - too large = any declared size, the total or the growth above `i64::MAX`.
  - with a project limit: too large, or `growth > 0` and `used + growth > limit` → `ToolError::project_quota_exceeded(limit, used, R)` with `R = min(growth, i64::MAX)` (`i64::MAX` when too large), `used` from `quota::used_bytes`; otherwise without a limit and too large → `ERR_WRITE_QUOTA_EXCEEDED` `session write quota of <N> bytes exceeded`; then `total > session_remaining` → the same session refusal. Equal is accepted.
- `quota.rs` `admit_paths` and `check_and_charge`: sums saturate (`saturating_add`, `used.saturating_add(delta) > limit`); `safety.rs` `charge_write`: `s.bytes_written.saturating_add(num_bytes) > quota`, and a negative `num_bytes` never lowers `bytes_written` (`max(0)` on the charge).
- `archive.rs`: declared sizes enter the admission as `i64::try_from(size).unwrap_or(i64::MAX)`; the total is a saturating sum.
- `archive.rs`: `pub(crate) fn declared_size_exceeded(rel_path: &str, declared: u64) -> ToolError` (`archive entry '<path>' decompressed to more bytes than declared <D>`, the text of `archive_rar.rs:219-223` and `archive.rs:410-424`), used by zip, 7z and RAR.
- Messages and codes: unchanged texts (spec FR-NEW-009 items 1 to 11).
- `TOOL_CONTRACT.txt`, `tool-contract-golden.json`: unchanged.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive.rs` | modify: `try_from` sizes, saturating total; zip/7z list, check, decode split; 7z entry checks in the listing; per entry `read_capped`; `declared_size_exceeded`; RAR routed through the split | FR-NEW-001 to FR-NEW-009, FR-MOD-001, FR-MOD-002 | US-0188, US-0189, US-0191, US-0192 |
| `crates/core/src/quota.rs` | modify: saturating `admit_paths`/`check_and_charge`; `check_declared` | FR-NEW-001 to FR-NEW-003, FR-NEW-005, FR-NEW-006 | US-0188, US-0189 |
| `crates/core/src/safety.rs` | modify: saturating `charge_write` | FR-NEW-006 | US-0188 |
| `crates/core/src/tools/archive_rar.rs` | modify: `list_rar`/`decode_rar_listed`; test `e2e_0080_068_collision_wins_over_the_storage_limi` replaced by `e2e_0085_mod_001_the_storage_limit_wins_over_a_collis` | FR-MOD-001, FR-NEW-009 | US-0191 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod archive_declared_e2e;` | test layout | US-0188 |
| `crates/core/src/tools/archive_testkit.rs` | modify: `limit_mb_remaining`, `limit_whole_mb_remaining`, second member, `zip_declaring`, `sevenz_declaring` | fixtures (DEC-009) | US-0188, US-0189, US-0192 |
| `crates/core/src/tools/archive_declared_e2e.rs` | create: the `e2e_0085_*` tests, RAR ones in a nested `mod archive_rar` | every requirement | US-0188 to US-0192 |
| `crates/core/src/api/dataplane.rs` | modify: `e2e_0085_005` | FR-NEW-010 | US-0193 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-012) | US-0193 |

Affected existing tests: `e2e_0080_068_collision_wins_over_the_storage_limi` (`archive_rar.rs:1335-1344`) is replaced by E2E-MOD-001 (US-0191). Kept unchanged and named because they sit next to the new rule: `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs:1306-1356`), `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027`, `_028`, `_029`, `_051` (`quota_e2e.rs:1224-1290`, `:2088-2120`: the early check reports the same figures, `refusal(1_030_000, 30_000)` included). Any other existing test that changes is a story failure: stop and report it.

Affected documentation: `README.md:306-320` ("All or nothing: the whole archive is decoded and checked before the first write" becomes: declared sizes are checked against both limits before any entry is decoded), `AGENTS.md:134-137` (`archive.rs` entry: "one fully buffered pre scan" wording), `.agent_docs/tools.md:184-217` (order of refusals for zip, 7z, RAR; a dated SPEC-0085 paragraph), `.agent_docs/testing.md` (filter `e2e_0085_`, the nested `archive_rar` module rule).

## 7. Compatibility and risks

- Declared changes: an over limit zip, 7z or RAR is refused before decoding, before its wrong password, damaged data, refused destination or collision; a 7z link, special or escaping entry is refused before any 7z entry is decoded; an uncountable size is refused at the limit.
- Concurrency: the early check reads `used` and the session counter without holding the admission lock across decoding; the final `admit_paths` re checks under the lock (FR-NEW-009 item 11), so SPEC-0020 E2E-051 stays green.
- Saturation in `admit_paths`, `check_and_charge` and `charge_write` changes nothing for sizes below `i64::MAX`; every other caller (folder copy, git, writes) passes in memory lengths.
- Ordering: US-0188 (exact counting, exploit tests red) lands before the early check so the exploit tests show their red reason (security design, section 8). US-0191 (RAR) after US-0189 because it reuses the split. SPEC-0086 later moves the destination refusal in front of all this (its E2E-MOD-001 rewrites this spec's E2E-028).
- Rollback: revert the story commits.

## 8. Security design

- Threat surface: an uploaded archive is untrusted; its headers (zip central directory and zip64 extra field, 7z header, RAR file headers) carry declared sizes the attacker chooses. Sinks: the signed byte counts of the project usage (`quota.rs:108`, `:156`) and of the session allowance (`safety.rs:139`), and the in memory decode (`archive.rs:555-574`, `:661-662`, `archive_rar.rs:69`).
- Occurrence 1, `crates/core/src/tools/archive.rs:251` (`e.declared_size as i64`): fixed by `i64::try_from(size).unwrap_or(i64::MAX)` (US-0188, DEC-003) and, before decoding, by `check_declared` computing in `u128`/`i128` (US-0189, DEC-002). Exploit tests E2E-SEC-001, E2E-SEC-003 (zip), E2E-SEC-005 (7z).
- Occurrence 2, `archive.rs:252` and `quota.rs:156`, `:108` (unchecked sums): saturating sums (US-0188, DEC-003). Exploit tests E2E-SEC-001, E2E-SEC-004.
- Occurrence 3, `safety.rs:139` (session sum lowered by a negative charge): saturating comparison and no negative charge (US-0188, DEC-003). Exploit test E2E-SEC-002.
- Occurrence 4, the decode before any limit (`archive.rs:555-559`, `:571-574`, `:661-662`, `archive_rar.rs:69`): the listing then `check_declared` before decoding (US-0189 for zip and 7z, US-0191 for RAR, DEC-001, DEC-007). Exploit tests E2E-006, E2E-007 (US-0189), E2E-008 (US-0191).
- Occurrence 5 (`docs/extract.rs:563-568`, Office parts): BL-0134, out of scope (spec D-11). Occurrence 6 (compressed tar): SPEC-0084. Occurrence 7: safe.
- Second channel for the absence of effect: the project usage read through `quota::used_bytes` (what `admin.get_project_quota` reports, "the operator's view"), the session allowance read through a later real write (`core::fs_ops::write_bytes`), the listing of `/uploads/free` by a second member of the project (`seed_member` then `exists`), and the decoded bytes held through `LAST_HIGH_WATER` (0 when no decoder ran). Each exploit test carries its literal payload (`9223372036854775808`, `18446744073709551615`, two times `9223372036854775807`) in a header built by `zip_declaring` or `sevenz_declaring`.
- Red before the fix: E2E-SEC-001 to E2E-SEC-005 are written and run against `main` first (US-0188); the red reason stated by the spec is expected. When the run shows the library itself refuses the payload (spec 14.3: `zip` 9 assumed to accept a short zip64 entry; the 7z reader may refuse a 2^63 size), the story records that refusal as the control and keeps the test as a guard.
- Secrets: none involved; no password reaches a message (unchanged rule, `archive.rs:675-686`).

## 9. Test strategy

Functional e2e through the typed engine call `extract_archive` after `f.state.authorize(MOUNT, person)` (both doors call it), in `crates/core/src/tools/archive_declared_e2e.rs`; tests that need a server with RAR support sit in a nested `#[cfg(feature = "rar")] mod archive_rar` so the second line of `test.sh` (`cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_`, `test.sh:7`) runs them (DEC-011). E2E-005 goes through the HTTP door in `api/dataplane.rs`. E2E-MOD-001 replaces the SPEC-0080 test in `archive_rar.rs`. Names `e2e_0085_<id>_<slug>` (`e2e_0085_sec_001_...`, `e2e_0085_mod_001_...`), filter `e2e_0085_`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-002 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-003 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-004 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-005 | functional e2e | HTTP door | api/dataplane.rs tests | US-0193 |
| E2E-006 | exploit (functional e2e) | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-007 | exploit (functional e2e) | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-008 | exploit (functional e2e) | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-009 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-010 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-011 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-012 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-013 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-014 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-015 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-016 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-017 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-018 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-019 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-020 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-021 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-022 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-023 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-024 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-025 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-026 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-027 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-028 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-029 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-030 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-031 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-032 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-033 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-034 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-035 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-036 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`) | US-0191 |
| E2E-037 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-038 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-039 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-040 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0192 |
| E2E-041 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0189 |
| E2E-042 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-043 | functional e2e | typed engine call | archive_declared_e2e.rs | US-0190 |
| E2E-SEC-001 | exploit | typed engine call | archive_declared_e2e.rs | US-0188 |
| E2E-SEC-002 | exploit | typed engine call | archive_declared_e2e.rs | US-0188 |
| E2E-SEC-003 | exploit | typed engine call | archive_declared_e2e.rs | US-0188 |
| E2E-SEC-004 | exploit | typed engine call | archive_declared_e2e.rs | US-0188 |
| E2E-SEC-005 | exploit | typed engine call | archive_declared_e2e.rs | US-0188 |
| E2E-MOD-001 | functional e2e | typed engine call | crates/core/src/tools/archive_rar.rs tests (replaces `e2e_0080_068_...`) | US-0191 |

Test data: generated (DEC-009). RAR archives with the in tree RAR5 store builder `rar5_store`/`RarEntry` (`archive_rar_fixtures.rs:55-105`); `/uploads/pack.RAR` of E2E-002 is the committed `crates/core/tests/fixtures/rar/v5.rar` (`a.txt` = `hello`).

## 10. Decisions log

- **DEC-001:** zip, 7z and RAR go through list, then `check_declared`, then decode, each blocking step in `spawn_blocking`; the listing applies every header refusal of FR-NEW-009 item 5 in the spec's per format order (zip: unreadable, per entry link / escaping / duplicate, missing password, local header disagreement; 7z: encrypted or unreadable header, missing password, per entry escaping then special type; RAR: SPEC-0080 FR-NEW-015 items 4 to 6, then missing password for encrypted data); the decode step reopens zip and 7z from the same bytes. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-004, FR-NEW-009, FR-MOD-001. **Rationale:** headers are all the early check needs (`by_index_raw` gives `size()` with no password, `archive.rs:506-527`; `Archive::read` gives `archive.files[i].size`, `archive.rs:597`; RAR's listing gives `unpacked_size`, `archive_rar.rs:36-41`), so nothing is decoded before the limits. **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** as cited.
- **DEC-002:** `quota::check_declared` computes the declared total (`u128`) and the declared net growth (`i128`, last occurrence per target path, minus the stored file size, whatever `overwrite` says, 0 for every path when `count_stored` is false), treats any size, total or growth above `i64::MAX` as too large, and refuses: project limit and (too large or `used + growth > limit` with `growth > 0`) → `project_quota_exceeded(limit, used, min(growth, i64::MAX))`, `i64::MAX` when too large; else (no limit and too large) or `total > session_remaining` → the session refusal; equality accepted; nothing charged; the admission lock is taken only inside `quota::used_bytes`. **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-005, FR-NEW-006. **Rationale:** the exact figures of today's admission (`quota.rs:136-159`) computed without wrap and without charging; project first and the tie to the project as FR-NEW-003 says. **Alternatives considered:** reusing `admit_paths` with a dry run flag (it takes the lock and charges; a flag doubles its contract). **Source:** design. **Evidence:** `crates/core/src/quota.rs:96-159`, `crates/core/src/safety.rs:136-154`.
- **DEC-003:** the final admission never sees a negative size: `archive.rs` passes `i64::try_from(declared).unwrap_or(i64::MAX)` and a saturating total; `admit_paths` sums with `saturating_add`; `check_and_charge` compares `used.saturating_add(delta) > limit`; `charge_write` compares `s.bytes_written.saturating_add(num_bytes) > quota` and charges `num_bytes.max(0)`. **Obliged by:** SPEC-0085/FR-NEW-005, FR-NEW-006. **Rationale:** closes occurrences 1 to 3 at their own sites, so the exploit tests turn green in US-0188 before the early check exists, and stay green when it lands (defense in depth). **Alternatives considered:** fixing only the early check (the final admission would stay exploitable by any other caller passing a cast size). **Source:** class sweep, spec 14.2. **Evidence:** `crates/core/src/tools/archive.rs:251-252`, `crates/core/src/quota.rs:108`, `:156`, `crates/core/src/safety.rs:139`.
- **DEC-004:** decoded bytes held: the decode step keeps a running total of decoded entry bytes and its maximum in a `&mut u64` it returns with its result (success or error); the async side sets `archive_single::LAST_HIGH_WATER` to it under `cfg(test)`; a call refused before decoding never sets it, so `held()` reads 0. **Obliged by:** SPEC-0085/FR-NEW-004, FR-NEW-008, NFR 7.1. **Rationale:** the one hook of SPEC-0084 DEC-006, set on the async thread (the blocking thread differs). **Alternatives considered:** an `Arc<AtomicU64>`. **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:56-61`.
- **DEC-005:** every zip and 7z entry is read through `read_capped(reader, declared)`; `CapExceeded` maps to `declared_size_exceeded(rel_path, declared)`; zip's own `larger than its declared` error keeps its mapping (`zip_read_error`, `archive.rs:410-424`), now to the same function; RAR keeps UnRAR's own stop at the declared size and the `data.len() != declared` check (`archive_rar.rs:59-87`). **Obliged by:** SPEC-0085/FR-NEW-007, FR-NEW-008. **Rationale:** "a decoder that ignores a declared size does not lift the bound": the cap holds whatever the library does; at most the declared size plus one 65 536 byte chunk per entry, and the sum of declared sizes for the call. **Alternatives considered:** trusting the libraries' own bounds (zip `crc32.rs:43-50`, 7z `BoundedReader`): true today, not guaranteed. **Source:** design. **Evidence:** `zip-9.0.5/src/crc32.rs:43-50`, `sevenz-rust2-0.20.2/src/reader.rs:1660-1666`.
- **DEC-006:** the 7z entry checks (escaping name, then link or special type, in archive order) move from inside `for_each_entries` (`archive.rs:637-647`) to the listing over `archive.files` after `Archive::read` and the missing password gate. **Obliged by:** SPEC-0085/FR-MOD-002, FR-NEW-009 item 5. **Rationale:** `archive.files` holds every entry with its attributes before any block is decoded; the same `ensure_entry_path_safe` and `sevenz_special_type` run on it. **Alternatives considered:** none: the spec moves them. **Source:** spec. **Evidence:** `crates/core/src/tools/archive.rs:593-683`.
- **DEC-007:** `decode_rar` becomes `list_rar(bytes, password) -> Result<RarListing>` (temp dir and file, completeness walk, listing with entry rules, missing password gate; `RarListing { dir: TempDir, path: PathBuf, entries }`) and `decode_rar_listed(listing, password, held: &mut u64) -> Result<Vec<DecodedEntry>>`; the temp dir drops when the listing drops after decoding or on any refusal. **Obliged by:** SPEC-0085/FR-MOD-001, FR-NEW-009 item 5 and 8. **Rationale:** UnRAR opens a path only (SPEC-0080 DEC-003), so the file is written once and read by both passes. **Alternatives considered:** writing the temp file twice. **Source:** existing code. **Evidence:** `crates/core/src/tools/archive_rar.rs:18-90`.
- **DEC-008:** target paths for the early check are joined to the planned destination of SPEC-0084 DEC-003 (`join_entry_destination`, `archive.rs:369-372`); when it is `None` (the destination will be refused), `count_stored` is false and the paths are joined to `/` only to keep last occurrence semantics. **Obliged by:** SPEC-0085/FR-NEW-001 business rule, spec D-6. **Rationale:** one destination resolution for the two bounds; SPEC-0086 later replaces it by the real destination known before reading. **Alternatives considered:** a second resolution. **Source:** SPEC-0084 DEC-003. **Evidence:** `crates/core/src/tools/archive.rs:213-227`.
- **DEC-009:** fixtures generated in the tests. Bombs: zip `build_zip(&[("zeros.bin", &vec![0; 10_000_000])], None)` (deflate, at most 20 000 bytes), 7z `build_sevenz_ext` (LZMA2), RAR `rar5_store(&[RarEntry::file("zeros.bin", &vec![0; 10_000_000])])` (store method, 10 MB, decoded size equal to its stored size); liars and declared sizes: a test helper `zip_declaring(entries: &[(&str, u64, &[u8])]) -> Vec<u8>` writes local headers, deflated data (`flate2::write::DeflateEncoder`), CRC32 (`flate2::Crc`) of the real content, a central directory and an end record; for a declared size of `0xFFFF_FFFF` or more both headers carry `0xFFFF_FFFF` and a zip64 extra field (id `0x0001`, uncompressed size then compressed size, 16 bytes) and version needed 45; a helper `sevenz_declaring(name, declared, data) -> Vec<u8>` assembles a 7z with one Copy coder folder (pack size = data length, unpack size = `declared`), no CRC digest, a plain (not encoded) header with the file name, and the start header CRC (`flate2::Crc`); RAR liars `RarEntry::declaring` (`archive_rar_fixtures.rs:87`). Remaining storage: `limit_mb_remaining(f, n)` sets a 1 048 576 byte limit and writes `/filler.bin` so exactly `n` bytes remain (the archive counts as stored); `limit_whole_mb_remaining(f, n)` uses the next whole megabyte when the stored files exceed 1 MB (spec 9 preamble). **Obliged by:** SPEC-0085/FR-NEW-001, FR-NEW-005, FR-NEW-007 (spec section 9: fixtures by the design), spec 14.3 (zip64 and 7z payloads). **Rationale:** no library writes a lying header; both builders are short and only test code. **Alternatives considered:** committed fixture files; patching a `zip` crate archive in place (the zip64 field does not exist in a small archive). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:1104-1160`, `:2207-2250`, `crates/core/src/tools/archive_rar_fixtures.rs:55-105`.
- **DEC-010:** story order inside the spec: exact counting with the exploit tests first (US-0188), then the early check for zip and 7z (US-0189), the net growth and the order (US-0190), RAR (US-0191), the per entry cap (US-0192), the HTTP door and docs (US-0193). Global order: SPEC-0084 DEC-010. **Obliged by:** n/a (no code impact): story order. **Rationale:** exploit tests red before their fix; one writer at a time on `archive.rs`. **Alternatives considered:** early check first (the exploit tests would then be green on arrival and lose their red evidence). **Source:** security design section 8. **Evidence:** n/a.
- **DEC-011:** tests needing the `rar` feature sit in a module whose path contains `archive_rar` (a nested `#[cfg(feature = "rar")] mod archive_rar { ... }` in `archive_declared_e2e.rs`; E2E-MOD-001 in `archive_rar.rs`), because `test.sh:7` filters the feature run on `archive_rar` and `e2e_0080_`. **Obliged by:** n/a (no code impact): test runner. **Rationale:** without it `make test` never runs them; editing `test.sh` per spec adds a file to every story that adds one. **Alternatives considered:** extending the `test.sh` filter. **Source:** existing code. **Evidence:** `test.sh:7`.
- **DEC-012:** documentation in the closing story: `README.md` (limits checked on declared sizes before decoding; size lies stop at the declared size plus 64 KiB), `AGENTS.md` (`archive.rs` entry), `.agent_docs/tools.md` (order of refusals per format, a dated SPEC-0085 paragraph), `.agent_docs/testing.md` (filter `e2e_0085_`, nested `archive_rar` rule, the two builders). **Obliged by:** n/a (no code impact): project convention. **Rationale:** `README.md:310-313` describes the old "decoded and checked before the first write" rule. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `README.md:306-320`, `.agent_docs/tools.md:184-217`.
- **DEC-013:** tests whose rule an earlier story already built are green on arrival and each is proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182. **Source:** design. **Evidence:** `git log` of those stories.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0188 | count declared archive sizes exactly, never negative | P1 | SPEC-0085/FR-NEW-005, FR-NEW-006 | US-0187 | 1 | no |
| US-0189 | refuse zip and 7z archives over a limit before decoding | P1 | SPEC-0085/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-008, FR-MOD-001, FR-MOD-002 | US-0188 | 2 | no |
| US-0190 | net growth with stored files and the order of refusals | P1 | SPEC-0085/FR-NEW-001, FR-NEW-009 | US-0189 | 2 | no |
| US-0191 | admit RAR declared sizes before decoding | P1 | SPEC-0085/FR-MOD-001, FR-NEW-007, FR-NEW-009 | US-0190 | 2 | no |
| US-0192 | cap every entry at its declared size | P2 | SPEC-0085/FR-NEW-007, FR-NEW-008, FR-NEW-010 | US-0191 | 2 | no |
| US-0193 | declared sizes through the HTTP door and documentation | P2 | SPEC-0085/FR-NEW-010 | US-0192 | 2 | yes |
