# US-0170: extract a RAR archive when the server has RAR support

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 7
> Closes spec: no

## Objective
Add the opt in `rar` feature and the RAR reader so a plain `.rar` extracts like any other format, and keep today's refusal for a server built without the feature. Decision density is high (new dependency with a C++ build, a feature flag, a test only archive builder), hence `min_tier` 1.

## Requirements covered
- SPEC-0080/FR-NEW-001: WHEN a member asks to extract a file whose name ends in `.rar`, in any letter case, THE archive extraction SHALL extract it by the rules that apply to every other supported format: default destination is the archive path without its suffix, `destination` and `overwrite` are honoured, the answer carries the destination, files written, folders created and bytes written, and one audit line is recorded.
- SPEC-0080/FR-NEW-012: IF the server is delivered without RAR support AND the file name, ignoring letter case, ends in `.rar` THEN THE archive extraction SHALL refuse with `ERR_NOT_SUPPORTED` and the message `unsupported archive extension: '<extension as typed>'`, and write nothing; this includes multi volume names.
- SPEC-0080/FR-MOD-001: IF the server has RAR support THEN THE archive extraction SHALL treat `.rar` as a supported extension.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_001 e2e_0080_002 e2e_0080_015 e2e_0080_017 e2e_0080_018 e2e_0080_028 e2e_0080_029 e2e_0080_039 e2e_0080_040 e2e_0080_mod_001`: all 5 tests pass. Without the feature run `cargo test -p mcp-fs-core --lib -- e2e_0080_015 e2e_0080_028 e2e_0080_029 e2e_0080_039 e2e_0080_040`: the 5 tests marked not(feature) pass. The test only archive builder is itself checked by reading its output back with the real `unrar` reader (E2E-001), which is not a mock of the story's output.

## Design excerpt
### Decisions
- **DEC-001:** RAR support is the `unrar` crate 0.5.8 behind an opt in Cargo feature `rar`, off by default, forwarded by `mcp-fs`; a build without it keeps today's refusal. **Obliged by:** SPEC-0080/FR-NEW-012, FR-MOD-001, spec 3.3. **Rationale:** the only RAR reader is a wrapper of RARLAB's UnRAR with a non free licence; the user chose an opt in feature. **Alternatives considered:** external `unrar` binary; `unrar-ng` 0.7.7; always on; decline the format. **Source:** user answer (2026-11-04, opt in feature) and spec. **Evidence:** crates.io API `unrar` 0.5.8 `MIT OR Apache-2.0`; context7 `/muja/unrar.rs`.
- **DEC-002:** `.rar` detection is a cfg'd table entry after the tar entries; `is_multi_volume_name` runs after format detection and before the file is read, only when the feature is on; with the feature off `.part1.rar` falls to the unsupported extension message. **Obliged by:** SPEC-0080/FR-NEW-010, FR-NEW-012, FR-NEW-015. **Rationale:** FR-NEW-012 includes multi volume names. **Alternatives considered:** check in `decode_rar` (too late, the file is read). **Source:** spec. **Evidence:** `archive.rs:40-69`, `:146-150`.
- **DEC-003:** the archive bytes are written to a file in `tempfile::tempdir()` per call because the crate opens a path; the temp dir is removed when the call returns; everything runs in `spawn_blocking`. **Obliged by:** SPEC-0080/FR-NEW-001. **Rationale:** `Archive::new(path)` is the only constructor (`unrar-0.5.8/src/archive.rs:49-72`). **Alternatives considered:** a named pipe or `memfd` (platform specific, unsafe). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:154-160` (decode already in `spawn_blocking`).
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-009:** test data: a test only RAR5 store builder (`archive_rar_fixtures.rs`: signature, main header, file headers with vint fields and CRC32, directory, symlink and hard link redirection records) produces the plain cases; encrypted, RAR4 and solid cases need real fixtures made with `rar` and committed under `crates/core/tests/fixtures/rar/` with a `README` line naming the command that made each. **Obliged by:** SPEC-0080 tests (spec Section 9 says fixtures come from the design). **Rationale:** `rar` is not installed, `7z` cannot create RAR; implementing RAR5 AES in the builder is out of proportion. **Alternatives considered:** the `unrar` crate's own `data/*.rar` (content and passwords differ from the spec's concrete data); a builder with encryption. **Source:** user answer (default taken 2026-11-05). **Evidence:** `which rar` returned nothing.
### Data model
None.
### Contracts
Features: `mcp-fs-core`: `rar = ["dep:unrar"]`; `mcp-fs`: `rar = ["mcp-fs-core/rar"]`; workspace `unrar = "0.5.8"`. `#[cfg(feature = "rar")] ArchiveFormat::Rar`; table entry `(".rar", Rar)` with the same cfg after the tar entries; `decode_archive` arm calls `archive_rar::decode_rar(bytes, password)`; `decode_rar` writes the bytes to a file in `tempfile::tempdir()`, opens it with `unrar::Archive::new` (or `with_password`), lists then reads each file entry, returns `Vec<DecodedEntry>`; corrupt input maps to `corrupt("rar", e)`. Without the feature nothing changes and `.rar` gives `unsupported archive extension: '.rar'` (as typed). Builder (test only): `fn rar5_store(entries: &[RarEntry]) -> Vec<u8>` with `RarEntry { name, kind (File, Dir, Symlink(target), HardLink(target)), data }`.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify |
| `crates/core/Cargo.toml` | modify |
| `crates/mcp-fs/Cargo.toml` | modify |
| `crates/core/src/tools/archive.rs` | modify |
| `crates/core/src/tools/archive_rar.rs` | create |
| `crates/core/src/tools/archive_rar_fixtures.rs` | create |
| `crates/core/src/tools/mod.rs` | modify |
### Existing patterns
`decode_sevenz` (`archive.rs:504-`) as the model of a decoder arm returning `Vec<DecodedEntry>` (`archive.rs:313-318`); `build_zip` (`archive.rs:915`) as the model of a test archive builder. unrar usage from the crate docs: `Archive::new(path).open_for_processing()`, `while let Some(h) = archive.read_header()? { let e = h.entry(); ... archive = if e.is_file() { let (data, next) = h.read()?; ... next } else { h.skip()? } }`. Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-001: Plain RAR extracts to the default destination
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** a member of a project holding `/uploads/report.rar` with folder `data`, `data/a.txt` containing `hello` and `data/b.txt` containing `goodbye`, and nothing at `/uploads/report`
- **When** the member extracts `/uploads/report.rar`
- **Then** the answer is destination `/uploads/report`, 2 files written, 2 folders created, 12 bytes written
- **And** `/uploads/report/data/a.txt` reads `hello` and `/uploads/report/data/b.txt` reads `goodbye`; the audit trail has one extraction line for `/uploads/report.rar`
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_001_plain_rar_extracts_to_the_default_de`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-002: Upper case suffix is detected
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** the same archive stored as `/uploads/REPORT.RAR`
- **When** the member extracts it
- **Then** the destination is `/uploads/REPORT` and 2 files are written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_002_upper_case_suffix_is_detected`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-015: Server without RAR support
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012, FR-MOD-001
- **Given** a server delivered without RAR support and `/uploads/report.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.rar'`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]`
- Function: `e2e_0080_015_server_without_rar_support`
- Green on arrival: without the feature the refusal already exists (run it with `cargo test -p mcp-fs-core --lib` without `--features rar`)
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-017: Empty RAR
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** `/uploads/none.rar` holding no entry
- **When** the member extracts it
- **Then** the answer is 0 files written, 1 folder created, 0 bytes written; `/uploads/none` exists and is empty
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_017_empty_rar`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-018: Accents and spaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** `/uploads/noms.rar` holding `été 2026/compte rendu.txt` containing `ok`
- **When** the member extracts it
- **Then** `/uploads/noms/été 2026/compte rendu.txt` reads `ok`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_018_accents_and_spaces`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-028: Extension shown as typed
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/report.RAR`
- **When** the member extracts it
- **Then** the message is `unsupported archive extension: '.RAR'`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]`
- Function: `e2e_0080_028_extension_shown_as_typed`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-029: Other formats unchanged without RAR support
- **Category:** happy; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/pack.zip` holding `a.txt` (5 bytes)
- **When** the member extracts it
- **Then** 1 file is written at `/uploads/pack/a.txt`
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]`
- Function: `e2e_0080_029_other_formats_unchanged_without_rar`
- Green on arrival: other formats are unchanged without the feature
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-039: Multi volume name without RAR support
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/big.part1.rar`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.rar'`; nothing is written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]`
- Function: `e2e_0080_039_multi_volume_name_without_rar_suppor`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-040: Upper case multi volume name without RAR support
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012
- **Given** a server without RAR support and `/uploads/big.PART2.RAR`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` with the message `unsupported archive extension: '.RAR'`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]`
- Function: `e2e_0080_040_upper_case_multi_volume_name_without`
- Green on arrival: same
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-MOD-001: `.rar` support (was SPEC-0018 E2E-NEW-022)
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-MOD-001; **Priority:** High
- **Validated before:** a `.rar` is refused with `ERR_NOT_SUPPORTED` naming `.rar`.
- **Validates now:** with RAR support, `/uploads/data.rar` holding a valid archive extracts; without RAR support the refusal of before still holds.
- **Given** `/uploads/data.rar`; **When** extracted on each kind of server; **Then** extraction succeeds with RAR support and `ERR_NOT_SUPPORTED` containing `.rar` without.
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive.rs tests (existing test updated)
- Function: `e2e_0080_mod_001_rar_support_was_spec_0018_e2e_new_02`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-015 | add the `.rar` table entry without the cfg: the test fails in a build without the feature |
| E2E-028 | same |
| E2E-029 | same |
| E2E-039 | same |
| E2E-040 | same |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies: add `unrar = "0.5.8"` only, optional; no `unsafe`
- The feature is off by default; `default = []` stays
- Verify `cargo deny check licenses` and `cargo tree -p mcp-fs-core` with and without `--features rar`: the dependency appears only with the feature; if `cargo deny` fails on `unrar_sys`, add the narrowest exception in `deny.toml` with a comment (the spec 3.3 constraint) and say so in the commit body
- Scope boundary: entry rules, limits, passwords, versions, multi volume, order belong to US-0171 and later

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Green on arrival tests (E2E-015, E2E-028, E2E-029, E2E-039, E2E-040): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0170: extract a RAR archive when the server has RAR support`, body `Spec: SPEC-0080` and the requirement ids.
