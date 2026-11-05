# SPEC-0083: Extract a bare bzip2 file, design

> Spec: sdlc/specs/SPEC-0083-extract-bz2-single-file/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-05
> Target tier: 2 (default, no `--tier`, no constitution tier, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Bzip2 reader | `bzip2::read::MultiBzDecoder` (crate already a dependency, pure Rust `libbz2-rs-sys` backend) | `bzip2 = "0.6"` (`Cargo.toml:85`), locked 0.6.1 | DEC-002 |
| Everything else | as SPEC-0081's design (module `archive_single`, caps, admission) | see SPEC-0081 design | DEC-001 |

Skills loaded: rust (same effect as SPEC-0081's design). No dependency added.

## 2. Approaches

### Chosen: add `SingleFormat::Bzip2` to the single file flow of SPEC-0081
The flow, the order of refusals, the caps and the answer are built by SPEC-0081's stories. This spec adds a table entry, a reader and the tests that pin bzip2 behavior. Pros: about 15 production lines, one implementation of every rule. Cons: this spec depends on SPEC-0081's stories landing first (DEC-001). Affected scope: `tools/archive.rs` (table entry), `tools/archive_single.rs` (variant and reader).

### Rejected: a separate flow for bzip2
Duplicates 200 lines of rules that must stay identical for gzip, bzip2 and xz. Rejected (DEC-001): the AGENTS.md convention is one engine per operation.

## 3. Architecture

Unchanged from SPEC-0081's design. `SingleFormat::Bzip2` selects the reader in `archive_single::decoder(format, bytes) -> Box<dyn Read + '_>`: `MultiGzDecoder` for `Gzip`, `MultiBzDecoder` for `Bzip2`. The corrupt message label is `"bz2"`.

## 4. Data model

None.

## 5. Contracts and interfaces

- Table: `(".bz2", ArchiveFormat::Single(SingleFormat::Bzip2))` after the tar entries (`.tar.bz2`, `.tb2`, at `archive.rs:43-44`) and after `.gz`.
- `SingleFormat::label()` returns `"bz2"`; the corrupt message is `archive is corrupt or not a valid bz2 file: <detail>`.
- Every other message, parameter and answer key: as SPEC-0081's design, section 5. `TOOL_CONTRACT.txt` and the golden file do not change.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive.rs` | modify: table entry, `detect_format_maps_every_suffix` rows | FR-NEW-001, FR-MOD-001 | US-0165 |
| `crates/core/src/tools/archive_single.rs` | modify: variant, reader, tests | FR-NEW-001 to FR-NEW-012 | US-0165 to US-0168 |
| `crates/core/src/api/dataplane.rs` | modify: HTTP door tests (answer, password, destination is a file) | FR-NEW-001, FR-NEW-009, FR-NEW-011 | US-0165, US-0166, US-0167 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md` | modify: formats list | closing story | US-0168 |

Affected existing tests: none modified. SPEC-0083 has no modified test (the bare `.bz2` refusal is not pinned today). Affected documentation: as above.

## 7. Compatibility and risks

- `.tar.bz2` and `.tb2` keep their position before `.bz2`; signal E2E-006 to E2E-008.
- The tar path keeps `BzDecoder` (single stream, ignores trailing bytes, `archive.rs:643-648`); the bare file uses `MultiBzDecoder`. The two behaviors differ on purpose and are pinned by the spec (FR-NEW-010 and FR-NEW-008).
- Risk, dependency order: US-0165 to US-0168 need SPEC-0081's US-0160 (flow), US-0161 to US-0164 (rules). They are queued after them.
- No dependency added. Rollback: revert the commits.

## 9. Test strategy

Same drivers and locations as SPEC-0081's design (section 9), test names `e2e_0083_<nnn>_<slug>`, filter `e2e_0083_`. Fixtures are generated in the test with `bzip2::write::BzEncoder`; streams are concatenated byte vectors; a damaged checksum flips one byte of the compressed data; the bomb is `BzEncoder` over 10 000 000 zero bytes (49 bytes, measured). The reader streams, so the memory observable is asserted on `CappedRead.high_water <= cap + 65 536` exactly as in SPEC-0081.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-002 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-003 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-004 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-005 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-006 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-007 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-008 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-009 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-010 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-011 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-012 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-013 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-014 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-015 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-016 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-017 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-018 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-019 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-020 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-021 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-022 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0166 |
| E2E-023 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-024 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-025 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-026 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-027 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-028 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-029 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-030 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-031 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0167 |
| E2E-032 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-033 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-034 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-035 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-036 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-037 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-038 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-039 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-040 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-041 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-042 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-043 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-044 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-045 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-046 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-047 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-048 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0166 |
| E2E-049 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |
| E2E-050 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0167 |
| E2E-051 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0165 |
| E2E-052 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0165 |
| E2E-053 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0168 |

Test data: all generated.

## 10. Decisions log

- **DEC-001:** bzip2 reuses the single file flow and rules of SPEC-0081 (`extract_single`, `read_capped`, caps, admission, order); its stories depend on the SPEC-0081 stories that build each rule. **Obliged by:** SPEC-0083/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine for gzip, bzip2 and xz. **Alternatives considered:** a parallel flow. **Source:** SPEC-0081 DEC-001, DEC-005, DEC-007 (existing design binds). **Evidence:** `sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-design.md`.
- **DEC-002:** the reader is `bzip2::read::MultiBzDecoder` over a `Cursor` inside `spawn_blocking`, read errors mapped to `corrupt("bz2", e)`. **Obliged by:** SPEC-0083/FR-NEW-007, FR-NEW-008. **Rationale:** spike on bzip2 0.6.1: two streams give `hello world`; `JUNK` and 4 zero bytes after a stream fail with `bzip2: bz2 header missing`; a 0 byte file and a truncated file fail with `decompression not finished but EOF reached`; a flipped byte fails with `bzip2: invalid data`; an empty stream gives an empty output (`bufread.rs:181-218`). **Alternatives considered:** `BzDecoder` (single stream, ignores trailing bytes, which would accept the bytes FR-NEW-008 refuses). **Source:** spike run 2026-11-05. **Evidence:** `bzip2-0.6.1/src/bufread.rs:181-218`, `:234-244`.
- **DEC-003:** the table entry `(".bz2", Single(Bzip2))` follows the tar entries and `.gz`. **Obliged by:** SPEC-0083/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** ordered `ends_with` matching, compound before plain. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `archive.rs:40-69`.
- **DEC-004:** US-0165 wires `.bz2` with `BzDecoder` (single stream) and the bytes read through the SPEC-0081 flow; US-0166 switches to `MultiBzDecoder`, which is what turns the multi stream and trailing bytes tests green; US-0167 and US-0168 add tests that characterize rules already built by SPEC-0081 (collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** the rules are shared code, so their bzip2 tests are green on arrival; proving they bite needs a mutation (break the rule, see the test fail). **Alternatives considered:** one story of 53 tests. **Source:** design. **Evidence:** n/a.
- **DEC-005:** documentation as DEC-013 of SPEC-0081, formats list gains `.bz2`. **Obliged by:** n/a (no code impact): project convention. **Rationale:** AGENTS.md. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `AGENTS.md:130`.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0165 | extract a bare bzip2 file next to the archive | P1 | SPEC-0083/FR-NEW-001, SPEC-0083/FR-NEW-002, SPEC-0083/FR-NEW-003, SPEC-0083/FR-MOD-001 | US-0160 | 2 | no |
| US-0166 | decode several bzip2 streams and refuse corrupt input | P1 | SPEC-0083/FR-NEW-007, SPEC-0083/FR-NEW-008, SPEC-0083/FR-NEW-009, SPEC-0083/FR-NEW-010 | US-0165, US-0162 | 2 | no |
| US-0167 | check the bzip2 collision, destination and order rules | P1 | SPEC-0083/FR-NEW-004, SPEC-0083/FR-NEW-011, SPEC-0083/FR-NEW-012 | US-0166, US-0164 | 2 | no |
| US-0168 | check the bzip2 storage and session limits and document the format | P1 | SPEC-0083/FR-NEW-005, SPEC-0083/FR-NEW-006 | US-0167, US-0163 | 2 | yes |

