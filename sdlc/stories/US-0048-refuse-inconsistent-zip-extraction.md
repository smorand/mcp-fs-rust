# US-0048: refuse inconsistent zips on archive extraction

> Spec: SPEC-0041 (sdlc/specs/SPEC-0041-refuse-inconsistent-zips/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 5
> Closes spec: no

## Objective
A member extracting a zip whose entry headers disagree with its directory, or which holds two entries of the same name, today gets the archive as the directory describes it (or the last duplicate) silently. After this story `fs.extract_archive` refuses such archives before writing anything, keeping the password and existing refusal order. It also creates the shared check helper and the test crafting helpers US-0049 reuses.

## Requirements covered
- SPEC-0041/FR-NEW-001: IF any entry of a zip archive disagrees with the archive's directory on its name, alternate name, compression, encryption flag, AES encryption settings, trailing size record flag, last modified time, checksum, compressed size or uncompressed size, or two entries occupy overlapping bytes of the archive, THEN the product SHALL refuse the extraction before writing any file.
- SPEC-0041/FR-NEW-003: The product SHALL answer an encrypted archive given without a password with `password required to extract this archive`, whether or not the archive is malformed.
- SPEC-0041/FR-MOD-001: IF two entries of a zip archive carry the same name THEN the product SHALL refuse the extraction before writing any file.
- SPEC-0041/FR-NEW-004 (partial: archives): The product SHALL extract every well formed archive and document exactly as before this change.

## Independent Test
`make check` green; in `crates/core/src/tools/archive.rs` tests, E2E-002 extracts a crafted zip (local name `b.txt`, central `a.txt`) through the real `fs.extract_archive` function on a fixture volume and asserts the error message prefix `archive is corrupt or not a valid zip file: header inconsistency detected` and, through a separate listing of the project root, that no `out` folder exists.

## Design excerpt
### Decisions
- **DEC-001:** Use `ZipArchive::validate_header_consistency` once per archive, not the per read option. **Obliged by:** SPEC-0041/FR-NEW-001, FR-NEW-002, FR-MOD-001. **Rationale:** covers skipped parts, folded duplicates and overlap; reads headers only. **Alternatives considered:** per read `enforce_header_consistency`; hand written comparison. **Source:** design. **Evidence:** `zip-9.0.5/src/read/zip_archive.rs:754-768`, `read/mod.rs:324`, `:356-471`.
- **DEC-002:** One helper module `docs/zip_check.rs` returns the reason string (zip's Display with `invalid Zip archive: ` stripped, any other error text kept as is); each caller adds its own prefix. **Obliged by:** FR-NEW-001, FR-NEW-002, FR-NEW-005 user visible texts. **Rationale:** one implementation (AGENTS.md "never reimplement"), three message shapes. **Alternatives considered:** returning `ToolError` from the helper (would hard code one prefix). **Source:** design. **Evidence:** `zip-9.0.5/src/result.rs:70`.
- **DEC-003:** In `decode_zip`, the check runs after the pre scan (symlink, path, DR-009, `archive.rs:417-427`) and after the missing password answer (`:440-442`), before the decode loop. **Obliged by:** FR-NEW-001 business rules, FR-NEW-003, E2E-006, E2E-015, E2E-018. **Rationale:** exactly the order the spec fixes; no decryption needed. **Alternatives considered:** check right after opening (would answer header error before the password and before DR-009). **Source:** spec constraint. **Evidence:** `tools/archive.rs:401-446`.
- **DEC-006:** Acceptance tests run at the tool function level on a real fixture volume, as the existing archive and extraction tests do. **Obliged by:** FR-NEW-001..005 tests. **Rationale:** MCP and REST are thin adapters over these functions (AGENTS.md); no message transformation happens above them. **Alternatives considered:** HTTP level tests (no added signal). **Source:** existing code `tools/archive.rs:723`, `core/fs_ops.rs:3683`.
- **DEC-007:** Test archives are produced by `zip::ZipWriter` then patched by `#[cfg(test)] zip_check::craft`: `rename_local(bytes, from, to)` (splices the local header name, fixes its length field, shifts every later local header offset in the central directory and the central directory offset in the end record), `set_local_mtime`, `set_local_compressed_size`, `overlap(bytes)` (adds a second central record pointing at the first entry, fixing the end record counts and sizes). Identical raw names reuse `zip_with_raw_names` (`archive.rs` tests). **Obliged by:** E2E-002..006, 008..013, 016, 018, 019, 021..024, SEC-001, SEC-002. **Rationale:** one crafting implementation shared by both stories. **Alternatives considered:** checked in binary fixtures (opaque, unreviewable). **Source:** design. **Evidence:** existing patcher `archive.rs:1655`.
### Data model
None.
### Contracts
New `crates/core/src/docs/zip_check.rs`, declared `pub(crate) mod zip_check;` in `crates/core/src/docs/mod.rs`:
```rust
pub(crate) const OOXML_EXTS: &[&str] = &[".docx", ".xlsx", ".xlsm", ".pptx", ".pptm", ".potx", ".ppsx"];
pub(crate) fn header_inconsistency<R: std::io::Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Option<String>;
pub(crate) fn ooxml_header_inconsistency(path: &str, data: &[u8]) -> Option<String>;
#[cfg(test)] pub(crate) mod craft { rename_local, set_local_mtime, set_local_compressed_size, overlap }
```
`ooxml_header_inconsistency` is created here (unit tested here: None for a non OOXML extension, None for bytes that are not a zip, Some for a crafted docx) though only US-0049 calls it in production.
Message: `corrupt("zip", reason)` (`tools/archive.rs:350`) → `archive is corrupt or not a valid zip file: header inconsistency detected: <zip detail>`; code `ERR_INVALID_ARGUMENT`. Password answers unchanged: `password required to extract this archive` (`archive.rs:441`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/zip_check.rs` | create |
| `crates/core/src/docs/mod.rs` | modify (module declaration) |
| `crates/core/src/tools/archive.rs` | modify: in `decode_zip`, call the check after `if any_encrypted && password.is_none()` (`:440-442`) and before the decode loop; add tests; rewrite `dt_039_b_identical_raw_names_keep_the_last_entry` (`:2494`) as E2E-MOD-001 |
| `README.md` | modify: `fs.extract_archive` paragraph (around `:285`): malformed or duplicate name zips refused |
| `.agent_docs/tools.md` | modify: `fs.extract_archive` note on the refusal |
### Existing patterns
- Test fixture and drivers: `fixture()`, `extract_to_out`, `extract`, `root_has_out`, `read`, `seed_bytes` in `crates/core/src/tools/archive.rs` tests (see `dt_039_c` at `:2508`).
- Builders: `build_zip` (`:905`), `build_zip_stored` (`:1632`), `build_zipcrypto` (`:2046`), `zip_with_raw_names` (identical raw names, used by DT-039 b), byte patcher `patch_zip_declared_size` (`:1655`).
- Streaming archive for E2E-017: `zip::ZipWriter::new_stream`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

All refusals: code `ERR_INVALID_ARGUMENT`, message starting `archive is corrupt or not a valid zip file: header inconsistency detected` ("the message of E2E-002"), except E2E-006 first attempt and E2E-014 (`ERR_PASSWORD_REQUIRED`, `password required to extract this archive`) and E2E-015 (exact `archive is corrupt or not a valid zip file: duplicate entry name 'é.txt'`).

### SPEC-0041/E2E-001: well formed archive extracts unchanged
- **Category:** journey · **Scenario:** SC-001 · **Requirements:** FR-NEW-004
- **Given** Olivia, a member of project `atlas`, and `/in/good.zip` holding `a.txt` (`alpha`) and `b.txt` (`beta`)
- **When** she extracts it into `/out`
- **Then** `/out/a.txt` reads `alpha` and `/out/b.txt` reads `beta`
- **Priority:** Critical
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-002: an entry named differently in its own header is refused
- **Category:** failure, side effect · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/mismatch.zip` whose directory lists `a.txt` (`alpha`) while that entry's own header names it `b.txt`
- **When** Olivia extracts it into `/out`
- **Then** she is refused with a message starting `archive is corrupt or not a valid zip file: header inconsistency detected`
- **And** listing the project root shows no `out` folder
- **Priority:** Critical
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-003: an entry with different sizes or compression in its two headers is refused
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/sizes.zip` whose single entry `a.txt` declares a different compressed size in its own header than in the directory
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** High
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-004: two entries with the same name are refused
- **Category:** failure, side effect · **Scenario:** SC-001 · **Requirements:** FR-MOD-001
- **Given** `/in/dup.zip` holding two entries both named `dup.txt`, contents `first` and `second`
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002
- **And** listing the project root shows no `out` folder
- **Priority:** Critical
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-005: the last of three entries malformed, nothing written
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/late.zip` holding `a.txt`, `b.txt`, `c.txt`, only `c.txt` named `d.txt` in its own header
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; none of `/out/a.txt`, `/out/b.txt`, `/out/c.txt` exists
- **Priority:** High
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-006: encrypted and malformed: password first, then refused
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** `/in/locked.zip`, entry `a.txt` encrypted with password `s3cret`, named `b.txt` in its own header
- **When** Olivia extracts it into `/out` without a password, then again with `s3cret`
- **Then** the first attempt answers `password required to extract this archive`; the second is refused with the message of E2E-002; no `out` folder exists after either
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-011: headers differing only in modification time are refused
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/time.zip` whose single entry `a.txt` carries a different modification time in its own header than in the directory
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-014: encrypted well formed archive without password is asked for it
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-003
- **Given** `/in/pw.zip`, entry `a.txt` (`alpha`) encrypted with `s3cret`, headers consistent
- **When** Olivia extracts it into `/out` without a password
- **Then** answered `password required to extract this archive`; no `out` folder exists
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-015: distinct names reading alike are refused before the password is asked
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-003
- **Given** `/in/pwdup.zip`, encrypted with `s3cret`, holding an entry whose stored name is `é.txt` in UTF-8 flagged as UTF-8, and a second whose stored bytes differ but read as `é.txt` in CP437
- **When** Olivia extracts it into `/out` without a password
- **Then** refused with `archive is corrupt or not a valid zip file: duplicate entry name 'é.txt'`; no `out` folder exists
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-016: three entries with the same name, the distinct sibling not written
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-MOD-001
- **Given** `/in/dup3.zip` holding `keep.txt` (`k`) and `dup.txt` three times
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; `/out/keep.txt` does not exist
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-017: a well formed archive whose sizes follow the data extracts unchanged
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-004
- **Given** `/in/stream.zip`, written in streaming mode so `a.txt` (`alpha`) carries its sizes after its data, with zeros in its own header
- **When** Olivia extracts it into `/out`
- **Then** `/out/a.txt` reads `alpha`
- **Priority:** High
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-018: encrypted malformed archive with a wrong password
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-001, FR-NEW-003
- **Given** `/in/locked.zip` of E2E-006
- **When** Olivia extracts it into `/out` with the password `wrong`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-019: overlapping entries are refused
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/overlap.zip` whose directory lists `a.txt` and `b.txt` both pointing at the same entry header and data, each header consistent with its own directory row
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-MOD-001: identical names refused (was SPEC-0036 DT-039 case b)
- **Validated before:** two entries with identical raw name `é.txt`, the first flagged as UTF-8 (`first`), the second not (`second`), extract one file `/out/é.txt` reading `second`.
- **Validates now:** the same archive is refused with the message of E2E-002 and no `out` folder exists.
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-SEC-001: a disguised entry cannot be smuggled in by archive extraction
- **Covers:** class sweep occurrence #1 (Section 14.2)
- **Red before the fix because:** today `/out/a.txt` is written with `alpha` and the refusal assertion fails.
- **Given** `/in/smuggle.zip` whose directory lists `a.txt` (`alpha`) while the entry's own header names it `../../evil.txt`
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002
- **And** a search of the whole project for `*.txt` finds no file written by this call
- Level / Driver / Location: functional e2e / `extract_to_out` on fixture volume / `crates/core/src/tools/archive.rs` tests
- Cleanup: fixture volume is per test, dropped at end

E2E-SEC-001 red before the fix because: today `/out/a.txt` is written with `alpha`, so `unwrap_err` panics.

## Constraints
- Files not to touch: `docs/extract.rs`, `core/fs_ops.rs`, `docs/service.rs` (US-0049), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`.
- Dependencies not to add: any.
- Patterns to avoid: a second header comparison by hand; checking before the password gate or before the pre scan.
- Scope boundary: document text and conversion belong to US-0049.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (for E2E-001, 014, 015, 017, already green, mutation RED: temporarily break the assertion target and record the failure).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `dt_039_a`, `dt_039_c`, `dt_041_declared_size_lie_message_is_exact`, `e2e_new_026`, every password and tar/7z test unchanged and green.
- [ ] Docs updated: `README.md`, `.agent_docs/tools.md`.
- [ ] Self review: full.
- [ ] One commit `US-0048: refuse inconsistent zips on archive extraction`, body `Spec: SPEC-0041` and the requirement ids.
