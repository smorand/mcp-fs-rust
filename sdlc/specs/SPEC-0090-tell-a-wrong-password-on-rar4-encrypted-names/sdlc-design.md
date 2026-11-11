# SPEC-0090: Tell a wrong password on a RAR version 4 archive with encrypted file names, design

> Spec: sdlc/specs/SPEC-0090-tell-a-wrong-password-on-rar4-encrypted-names/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-11
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| The judgment | `walk_rar4_encrypted` in `crates/core/src/tools/archive_rar.rs:287-335` (SPEC-0087), with `archive_rar_crypt::{rar3_kdf, aes128_cbc_decrypt}` and `flate2::Crc` | in tree; feature `rar` | DEC-001 |
| Messages | `archive::password_required_message(Some(_))` (`crates/core/src/tools/archive.rs:1005-1015`) and `corrupt("rar", TRUNCATED)` (`archive_rar.rs:140`) | in tree | DEC-003 |
| Fixtures | the committed `hp4.rar`, `hp4long.rar`, `v4secret.rar`, `hidden.rar` (`crates/core/tests/fixtures/rar/`), cut or with one bit flipped at run time | in tree | DEC-006 |

Dependency statement: no new crate, no feature change, no `unsafe`; the `rar` feature (`unrar`, `aes`, `cbc`,
`hmac`, `sha2`) is unchanged.

Skills loaded: `rust` (typed `ToolError`, no `unwrap` on fallible production paths, clippy with
`--all-features`, fmt).

## 2. Approaches

### Chosen: judge the password on the first protected block in the completeness walk

`ensure_rar_complete` already decrypts the first protected block's first AES block to read its size
(`archive_rar.rs:304-311`) and checks its CRC16 (`:318-326`). The walk gains a judgment on that first block
only: a start that is not a possible block start is WRONG; a possible start whose block runs past the end
stays CUT; a whole first block failing its CRC16 is WRONG; a first block that passes hands over to today's
walk for the rest. Pros: one function, no extra key derivation (the key is already derived once per call),
runs before UnRAR, so before every later refusal (FR-MOD-002). Cons: none beyond the accepted limit of 3.2.
Scope: `archive_rar.rs`.

### Rejected: judge after UnRAR's `BadData` (DEC-001)

Map UnRAR's `BadData` on encrypted names to WRONG. UnRAR answers `BadData` for damage in any header too
(E2E-016 expects CORRUPT for a later block), and the walk already answers CUT before UnRAR runs for most wrong
passwords (`:311-314`), so the order of refusals would still be wrong.

### Rejected: a stricter possible start (flags, zero padding) (DEC-002)

Would turn `wrong135` into WRONG, changing E2E-019's answer: the possible start rule is normative (spec 14.3).

## 3. Architecture

```
fs.extract_archive (mcp)            POST /api/fs/{mount}/extract-archive (api/dataplane.rs)
     \ membership, path, folder, multi volume, rar support (unchanged)   /
      +---- tools::archive -> spawn_blocking -> archive_rar::list_rar(bytes, password)
              ensure_rar_complete(bytes, password)                         (archive_rar.rs:32)
                RAR4 main header declares encrypted names -> walk_rar4_encrypted(rest, password)
                  none or empty password -> Ok (UnRAR answers, unchanged)
                  first protected block:                                   [US-0231]
                    salt or first AES block missing      -> CUT   (unchanged)
                    start not a possible block start      -> WRONG
                    possible, block runs past the end     -> CUT   (unchanged)
                    whole, CRC16 fails                     -> WRONG
                    whole, CRC16 passes                    -> later blocks as today
                  later blocks: today's walk (CUT, or CRC failure left to UnRAR)
              UnRAR listing, entry rules, password for data, declared sizes, destination, collision,
              limits (unchanged, all after)
```

## 4. Data model

Nothing persisted. The walk keeps one boolean, `first`, true until the first protected block is judged.

## 5. Contracts and interfaces

- `fn walk_rar4_encrypted(rest: &[u8], password: Option<&str>) -> Result<()>`: same signature, new answers for
  the first protected block (DEC-001).
- `fn possible_start(first: &[u8; AES_BLOCK]) -> bool`: kind `first[2]` in `0x74..=0x7b`, declared size
  `u16::from_le_bytes([first[5], first[6]])` at least `RAR4_FILE_MIN` (32) for `0x74` and `0x7a`, at least
  `RAR4_BASE_HEADER` (7) for the six others (DEC-002).
- WRONG: `Err(password_required_message(Some(password)))`, code `ERR_PASSWORD_REQUIRED`, message `incorrect
  password for this archive; call again with the correct 'password' parameter`. CUT: `Err(corrupt("rar",
  TRUNCATED))`, `ERR_INVALID_ARGUMENT`, `archive is corrupt or not a valid rar file: unexpected end of archive`.
- Nothing else changes: the empty password and no password still return `Ok(())` from the walk.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify: the first block judgment, `possible_start`, `RAR4_FILE_MIN`; doc comment of `walk_rar4_encrypted` rewritten; test `rar4_encrypted_headers_left_to_the_library` renamed `rar4_first_encrypted_header_judges_the_password`, its `garbled` assertion now expects WRONG (E2E-MOD-002) | FR-NEW-001, FR-NEW-002, FR-MOD-001 | US-0231 |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | modify: `e2e_0087_027_version_4_encrypted_names_cut_wrong` becomes `e2e_0090_mod_001_version_4_encrypted_names_wrong_p`, its four calls expect WRONG, its BL-0136 comment goes (E2E-MOD-001) | FR-MOD-001 | US-0231 |
| `crates/core/src/tools/archive_rar_password_e2e.rs` | create: the `e2e_0090_*` engine tests | FR-NEW-001 to FR-NEW-008, FR-MOD-002 | US-0231, US-0232, US-0233 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod archive_rar_password_e2e;` | test layout | US-0231 |
| `crates/core/src/api/dataplane.rs` | modify (tests only, `rar` feature): `e2e_0090_032`, `033`, `034` | FR-NEW-007 | US-0234 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md` (`:231-232`), `.agent_docs/testing.md` | modify | closing (DEC-007) | US-0234 |

Affected existing tests: modified, exactly two, both in US-0231: `e2e_0087_027_version_4_encrypted_names_cut_wrong`
(`archive_rar_cut_e2e.rs:540-549`) and the `garbled` assertion of `rar4_encrypted_headers_left_to_the_library`
(`archive_rar.rs:1764-1765`; its `None`, `Some("")` and 108 byte cut assertions stay). Kept unchanged and
green: SPEC-0087 E2E-022, E2E-025, E2E-026, E2E-034, E2E-055 (`archive_rar_cut_e2e.rs:445`, `:528-535`,
`:577-624`, `:667-669`), the key derivation pins (`archive_rar.rs:1744-1748`, `:1774-1777`), SPEC-0080 E2E-065,
every other `e2e_0080_` and `e2e_0087_` test, the REST tests `assert_cut_refused` (`api/dataplane.rs:3675-3712`).

## 7. Compatibility and risks

- Declared change: a wrong password on version 4 encrypted names answers WRONG; a damaged first protected block
  with the right password answers WRONG (spec FR-NEW-002, accepted).
- Accepted limit: a wrong password whose start reads as a possible block running past the end stays CUT
  (at most 9 of 300 measured, E2E-019, E2E-020).
- Cost: none; the key is already derived once per call.
- Ordering: one production change, in US-0231, together with the two existing tests it changes, so main is
  green after it; the later stories add tests only (and docs).
- Rollback: revert; no data or contract change.

## 8. Security design

Not applicable: the spec reads `Security: n/a`. The password never reaches a message, a log line or an audit
line (FR-NEW-006, E2E-030): the walk returns the fixed message, and `logging::log_tool_failure` logs the error's
display only (`mcp/server.rs:54-57`).

## 9. Test strategy

Functional e2e through the typed engine call `archive_testkit::extract(&f, archive, destination, overwrite,
password)` (authorizes, then runs the tool's engine as both doors do), in
`crates/core/src/tools/archive_rar_password_e2e.rs`, all `#[cfg(feature = "rar")]`; membership through
`archive_testkit::extract` as `bob@example.com` (it authorizes and is refused by the real check); the HTTP door
in `api/dataplane.rs` tests next to `assert_cut_refused`. "Nothing written" is the spec's: the default
destination does not exist, `quota::used_bytes` and `safety.session_remaining` unchanged, no extraction audit
line (`archive_testkit::extraction_audit_lines`). Names `e2e_0090_<id>_<title in snake case>`, cut at 50
characters; filter `e2e_0090_`; run with `--features rar`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001, E2E-002 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0232 |
| E2E-003 to E2E-009 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0231 |
| E2E-010 to E2E-013 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0232 |
| E2E-014, E2E-015 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0231 |
| E2E-016 to E2E-019 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0232 |
| E2E-020 | functional e2e | completeness check for 296 passwords on parallel threads, typed engine call for 4 | archive_rar_password_e2e.rs | US-0232 |
| E2E-021 to E2E-031 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0233 |
| E2E-032 to E2E-034 | functional e2e | HTTP door | api/dataplane.rs tests | US-0234 |
| E2E-035 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0231 |
| E2E-036 | functional e2e | typed engine call | archive_rar_password_e2e.rs | US-0233 |
| E2E-MOD-001 | functional e2e | typed engine call | archive_rar_cut_e2e.rs (renamed in place) | US-0231 |
| E2E-MOD-002 | functional e2e plus the internal check | typed engine call; `ensure_rar_complete` | archive_rar_password_e2e.rs; archive_rar.rs tests | US-0231 |

Test data: the four committed archives; cuts are slices, damage is `bytes[k] ^= 1` at run time (spec 9 common
data). No archive is made at test time.

## 10. Decisions log

- **DEC-001:** the password is judged in `walk_rar4_encrypted` on the first protected block only, before the
  rest of the walk, with the key already derived for it: start not possible -> WRONG; possible and the block
  (its declared size rounded up to whole AES blocks) runs past the end -> CUT (today's `rest.get(..padded)`);
  whole and CRC16 failing -> WRONG; passing -> the existing loop goes on for the later blocks, where a CRC16
  failure still returns `Ok(())` for UnRAR to answer and a shortfall stays CUT. The start's 24 bytes missing
  stays CUT (today's `get(..RAR4_SALT)` and `get(..AES_BLOCK)`). **Obliged by:** SPEC-0090/FR-NEW-001 to
  FR-NEW-005, FR-MOD-001, FR-MOD-002. **Rationale:** the only bytes that can judge a version 4 password are the
  first protected block's; judging there puts WRONG before the cut and every later refusal. **Alternatives
  considered:** mapping UnRAR's `BadData` (section 2). **Source:** design, spec 14.3. **Evidence:**
  `archive_rar.rs:287-335`; `unrar_sys-0.5.8/vendor/unrar/arcread.cpp:516-541`.
- **DEC-002:** `possible_start` is the spec's rule verbatim: kinds `0x74` to `0x7b`; declared size at least 32
  (`SIZEOF_FILEHEAD3`) for file `0x74` and service `0x7a`, at least 7 (`SIZEOF_SHORTBLOCKHEAD`) for the six
  others; `RAR4_FILE_MIN: usize = 32` added next to `RAR4_BASE_HEADER` (`archive_rar.rs:142`). **Obliged by:**
  SPEC-0090/FR-NEW-001, FR-NEW-004 (definitions of section 6). **Rationale:** normative rule, same answer in
  every implementation. **Alternatives considered:** flags or padding checks (change E2E-019). **Source:**
  spec. **Evidence:** `unrar_sys-0.5.8/vendor/unrar/headers.hpp:8-9`, `:81-83`.
- **DEC-003:** WRONG is `password_required_message(Some(password))` and CUT is `corrupt("rar", TRUNCATED)`, the
  existing constructors, so both doors and the logging stay as they are. **Obliged by:**
  SPEC-0090/FR-NEW-001 to FR-NEW-004, FR-NEW-006, FR-NEW-007. **Rationale:** one wording per answer.
  **Alternatives considered:** none. **Source:** existing code. **Evidence:** `archive.rs:1005-1015`;
  `archive_rar.rs:140`.
- **DEC-004:** the two existing tests the spec changes are changed in the same story as the code: the SPEC-0087
  test is renamed `e2e_0090_mod_001_version_4_encrypted_names_wrong_p` in place (it keeps its file, its four
  calls, now `password_incorrect`), and the internal check is renamed
  `rar4_first_encrypted_header_judges_the_password` with its `garbled` line asserting the WRONG error; E2E-MOD-002
  also gets a tool level test `e2e_0090_mod_002_a_damaged_first_block_with_the_ri`. **Obliged by:**
  SPEC-0090/FR-MOD-001, FR-NEW-002, FR-NEW-008. **Rationale:** main stays green after the one behaviour
  change; the spec's filter finds the modified test. **Alternatives considered:** keeping the old name (the
  `e2e_0090_` filter would miss it). **Source:** design. **Evidence:** `archive_rar_cut_e2e.rs:540-549`;
  `archive_rar.rs:1750-1767`.
- **DEC-005:** E2E-020 runs the 300 passwords through `ensure_rar_complete(HP4, Some(pw))` split across
  `std::thread::available_parallelism()` scoped threads (one RAR 3 key derivation each, about 0.5 s in the
  debug build), mapping each result to WRONG, CUT or other, and runs `wrong0`, `wrong135`, `wrong299` and `nope`
  through the typed engine call with "nothing written" checked; it asserts every answer is WRONG or CUT, at
  least 291 WRONG, `wrong135` CUT. **Obliged by:** SPEC-0090/FR-NEW-001, FR-NEW-004 (E2E-020), spec 14.3 run
  time. **Rationale:** the spec allows the completeness check for the bulk provided four go through the tool;
  the walk is the whole judgment and writes nothing. **Alternatives considered:** 300 full extractions (minutes
  in debug). **Source:** spec 14.3. **Evidence:** spec 14.1 P-3.
- **DEC-006:** fixtures are slices and bit flips of the committed archives at run time, through local helpers
  in `archive_rar_password_e2e.rs` (`refused(stem, bytes, password, check)`, `wrong`, `cut`, `corrupt_not_cut`,
  modelled on `archive_rar_cut_e2e.rs:24-56`, which stay private there). **Obliged by:** SPEC-0090 3.3.
  **Rationale:** no archive is made at test time. **Alternatives considered:** making the SPEC-0087 helpers
  `pub(crate)` (one more file touched for four short functions). **Source:** spec. **Evidence:**
  `archive_rar_cut_e2e.rs:24-56`.
- **DEC-007:** the closing story rewrites `.agent_docs/tools.md:231-232` (the BL-0136 gap closed), documents the
  first block judgment in `AGENTS.md` (`archive_rar.rs` sentence) and `README.md`, and adds the `e2e_0090_`
  filter to `.agent_docs/testing.md`. The doc comment of `walk_rar4_encrypted` is rewritten in US-0231 with the
  code. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** house closing
  pattern. **Alternatives considered:** none. **Source:** constitution. **Evidence:** `AGENTS.md`.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0231 | judge a version 4 password on the first protected block | P1 | SPEC-0090/FR-NEW-001, FR-NEW-002, FR-MOD-001, FR-NEW-008 | US-0230 | 2 | no |
| US-0232 | cuts before the judgment, the right password and the accepted limit | P1 | SPEC-0090/FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-001 | US-0231 | 2 | no |
| US-0233 | the wrong password first, nothing left, other answers unchanged | P2 | SPEC-0090/FR-MOD-002, FR-NEW-006, FR-NEW-008, FR-NEW-005 | US-0232 | 2 | no |
| US-0234 | wrong version 4 passwords through the HTTP door and documentation | P2 | SPEC-0090/FR-NEW-007, FR-NEW-002, FR-NEW-003 | US-0233 | 2 | yes |
