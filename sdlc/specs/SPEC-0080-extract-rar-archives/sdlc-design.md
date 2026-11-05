# SPEC-0080: Extract RAR archives, design

> Spec: sdlc/specs/SPEC-0080-extract-rar-archives/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-05
> Target tier: 2 (default, no `--tier`, no constitution tier, no `.spec.json`); US-0170 is `min_tier` 1 (decision density: new dependency, C++ build, feature flag)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| RAR reader | crate `unrar`, optional, wrapper of RARLAB's UnRAR C++ library (built from source by `unrar_sys` with `cc`) | `unrar = "0.5.8"` (crates.io, MIT OR Apache-2.0; `unrar_sys` 0.5.8 same licence field) | DEC-001 |
| Cargo feature | `rar` on `mcp-fs-core`, forwarded by `mcp-fs`, not in `default` | `crates/core/Cargo.toml:15-23` pattern | DEC-001 |
| Temp files | `tempfile` (already a dependency) | `tempfile = "3"` (`Cargo.toml:123`) | DEC-003 |
| Fixtures | test only builder for RAR5 store archives, plus committed binary fixtures made with the RARLAB `rar` tool | none | DEC-009 |

Version source: crates.io API on 2026-11-05 (`unrar` max stable 0.5.8, updated 2025-02-19; `unrar-ng` 0.7.7 considered and rejected, DEC-001). The context7 documentation for `/muja/unrar.rs` gives the API used: `Archive::new`, `Archive::with_password`, `open_for_listing`, `open_for_processing`, `read_header`, `entry()`, `read()`, `skip()`, `FileHeader { filename, unpacked_size, file_attr, is_directory(), is_file(), is_encrypted() }`.
Skills loaded: rust. Effect: `make check` is the gate and compiles `--all-features`, so the `rar` feature (and its C++ build) is compiled and tested on every check; logic in `crates/core`.

## 2. Approaches

### Chosen: an optional `rar` decoder arm in the existing archive pipeline
RAR becomes `ArchiveFormat::Rar` (compiled only with the feature) and a `decode_rar` arm of `decode_archive`. The pipeline already does what the spec asks after decoding: destination normalize, collision scan, admission (project then session), write, audit. Pros: every shared rule stays one implementation; the order of refusals of FR-NEW-015 is the pipeline's order (decode with its checks, then destination, collision, admission). Cons: the file is decoded fully into memory before the admission, as for zip and 7z today (DEC-008). Affected scope: `archive.rs`, new `archive_rar.rs`, three manifests.

### Rejected: shell out to an external `unrar` binary
Adds a runtime dependency and weakens the sandbox; rejected (DEC-001, the user chose an opt in build feature).

### Rejected: `unrar-ng` 0.7.7
Same C library, edition 2024 and newer, extraction callbacks; rejected (DEC-001): the entry API we need is identical, `unrar` is the better known crate, and nothing here needs callbacks.

### Rejected: a RAR reader in safe pure Rust
None exists with RAR5 and encryption. Rejected (DEC-001).

## 3. Architecture

```
extract_archive
  normalize path -> node lookup (missing, folder) -> detect_format (".rar" only with feature rar)
  multi volume name?  -> ERR_NOT_SUPPORTED (before reading the file)
  read bytes -> spawn_blocking(decode_archive -> decode_rar)
       decode_rar: bytes -> temp file in a per call temp dir
         list headers (password if given): encrypted names, corrupt
         entry rules in archive order: link or device, escaping name
         password rule: any entry encrypted and no password
         read each file entry (solid archives are read in order): wrong password, size mismatch
       -> Vec<DecodedEntry>
  destination normalize -> collision scan -> admit_paths (project, then session) -> writes -> audit
```

`archive_rar.rs` holds `decode_rar`, `is_multi_volume_name`, `entry_kind`, and the error mapping. `archive.rs` gains the variant, the cfg'd table entry, the `decode_archive` arm, and `password_required_message` with the new wording. Bounded contexts: Archive extraction; Project storage (unchanged).

## 4. Data model

None.

## 5. Contracts and interfaces

- Features: `mcp-fs-core`: `rar = ["dep:unrar"]`; `mcp-fs`: `rar = ["mcp-fs-core/rar"]`; workspace: `unrar = { version = "0.5.8" }` (no default features are declared by the crate). A build without the feature has no `unrar` in `cargo tree`.
- `#[cfg(feature = "rar")] ArchiveFormat::Rar`; table entry `(".rar", Rar)` with the same cfg, after the tar entries.
- `fn is_multi_volume_name(file_name: &str) -> bool`: lower case, ends with `.rar`, and the part before it ends with `.part` followed by one or more ASCII digits. True for `big.part1.rar`, `big.part01.rar`, `BIG.PART3.RAR`; false for `big.part.rar`, `report.part1x.rar`.
- `fn decode_rar(bytes: &[u8], password: Option<&str>) -> Result<Vec<DecodedEntry>>` (sync, called inside `spawn_blocking`).
- `fn password_required_message(password: Option<&str>) -> ToolError` (exists at `archive.rs:615-620`): `None` gives `password required to extract this archive; call again with the 'password' parameter`; `Some(_)` gives `incorrect password for this archive; call again with the correct 'password' parameter`; the literals at `archive.rs:441`, `:489`, `:531` go through it.
- Error contract (exact): refusal codes and messages of the spec FR-NEW-002 to FR-NEW-022: `ERR_PASSWORD_REQUIRED` (the two messages above), `ERR_NOT_SUPPORTED` `archive entry '<name>' is a <symlink|hard link|device>, which is not supported` (the existing tar and zip wording, `archive.rs:421-423`, `:671-673`), `ERR_PATH_OUT_OF_BOUNDS` (`ensure_entry_path_safe`, `archive.rs:359-385`), `ERR_INVALID_ARGUMENT` `archive entry '<name>' ...` for a size mismatch, `ERR_NO_CLOBBER`, `ERR_PROJECT_QUOTA_EXCEEDED`, `ERR_WRITE_QUOTA_EXCEEDED` (existing), `ERR_NOT_SUPPORTED` `multi volume RAR archives are not supported: extract a single file archive`, `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid rar file: <detail>`.
- Config keys: none.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `Cargo.toml` | modify: `unrar` workspace dependency | FR-MOD-001, spec 3.3 | US-0170 |
| `crates/core/Cargo.toml` | modify: optional dependency and `rar` feature | FR-NEW-012 | US-0170 |
| `crates/mcp-fs/Cargo.toml` | modify: forward the feature | FR-NEW-012 | US-0170 |
| `crates/core/src/tools/archive.rs` | modify: wording, `Rar` variant, cfg table entry, `decode_archive` arm, multi volume check | FR-NEW-001, FR-NEW-010, FR-MOD-001, FR-MOD-002, FR-MOD-003 | US-0169, US-0170, US-0173 |
| `crates/core/src/tools/archive_rar.rs` | create: `decode_rar`, `is_multi_volume_name`, `entry_kind`, tests | FR-NEW-001 to FR-NEW-022 | US-0170 to US-0177 |
| `crates/core/src/tools/archive_rar_fixtures.rs` | create (`#[cfg(test)]`): RAR5 store builder | FR-NEW-001, FR-NEW-006, FR-NEW-007 | US-0170, US-0171 |
| `crates/core/tests/fixtures/rar/*.rar` | create: binary fixtures made with `rar` (encrypted, RAR4, solid) | FR-NEW-002 to FR-NEW-005, FR-NEW-016, FR-NEW-020 | US-0174 to US-0176 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(feature = "rar")] mod archive_rar;` | FR-NEW-001 | US-0170 |
| `crates/core/src/api/dataplane.rs` | modify: HTTP door tests | FR-NEW-002, FR-NEW-019 and others | US-0175, US-0173 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify: formats, the `rar` feature and its licence note, fixture provenance | closing story | US-0177 |

Affected existing tests: `e2e_new_022_rar_extension_is_not_supported` (`archive.rs:796-804`) is modified (it keeps its assertion for a build without the feature and gains a feature branch); `e2e_new_006` and `e2e_new_057` zip tests (`archive.rs:1028-1047`) and the 7z tests at `:2158`, `:2174` get the new wording (E2E-MOD-002 to E2E-MOD-005); every other existing test is kept. SPEC-0041 FR-NEW-003 and its story US-0048 quote the old missing password text; their tests are updated with the same change and listed in US-0169.
Affected documentation: as above.

## 7. Compatibility and risks

- Consumers: with the feature off nothing observable changes for `.rar` (FR-NEW-012). The two password messages gain a suffix for zip and 7z (declared by FR-MOD-002 and FR-MOD-003); the prefixes `password required` and `incorrect password` are kept.
- Licence: the UnRAR source licence is non free (it forbids using it to recreate the RAR compression algorithm). `cargo deny` reads crate metadata only: `unrar` and `unrar_sys` declare `MIT OR Apache-2.0`, so no `deny.toml` exception is expected; this corrects the spec's design input that assumed one. Signal: `cargo deny check licenses` in US-0170. The licence of the bundled source is recorded in `README.md`, `AGENTS.md` and a comment at the dependency.
- Build: `--all-features` compiles C++; developer machines need a C++ compiler (present).
- Risk, entry kinds: `FileHeader` exposes `file_attr` but not the RAR5 redirection record (`unrar-0.5.8/src/open_archive.rs:582-640`; `unrar_sys::HeaderDataEx::redir_type` is not surfaced). A symlink is recognized by Unix mode bits in `file_attr` (`0o120000`) or the Windows reparse attribute (`0x400`); a RAR5 hard link carries neither. This project contains no `unsafe`; this design adds none. If the symlink or hard link fixtures show the safe API cannot see them, US-0171 stops with BLOCKED_ON_SPEC and a `design-issues.md` entry (DEC-006).
- Risk, declared size: the C library extracts exactly the declared size, so a lying header surfaces as a read error, not as an oversize entry; it is mapped to `ERR_INVALID_ARGUMENT` naming the entry (DEC-005). Signal: E2E-012, E2E-036, E2E-037 with the builder.
- Memory: an entry is read whole before the admission, as for zip and 7z; recorded as backlog BL-0125 (not required by this spec).
- Fixtures: `rar` is not installed; the encrypted, RAR4 and solid fixtures are user provided (DEC-009); US-0174, US-0175, US-0176 and US-0177 are flagged.
- Rollback: revert; the feature is off by default.

## 9. Test strategy

Tests: `crates/core/src/tools/archive_rar.rs` (`#[cfg(test)] mod tests`, `#[cfg(feature = "rar")]` where the test needs the reader) with `Fixture` as in SPEC-0081's design; test names `e2e_0080_<nnn>_<slug>`, filter `e2e_0080_`. Tests of a build without the feature (E2E-015 and its siblings) are `#[cfg(not(feature = "rar"))]` and run by `cargo test -p mcp-fs-core --lib` without `--features rar`; `make check` runs `--all-features` for the others. Wording tests of zip and 7z (E2E-MOD) stay in `archive.rs` tests. HTTP door tests in `dataplane.rs` with `Harness`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0170 |
| E2E-002 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0170 |
| E2E-003 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-004 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-005 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-006 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-007 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-008 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-009 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-010 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-011 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-012 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-013 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-014 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-015 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]` | US-0170 |
| E2E-016 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-017 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0170 |
| E2E-018 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0170 |
| E2E-019 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-020 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-021 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-022 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0175 |
| E2E-023 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-024 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-025 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-026 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-027 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-028 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]` | US-0170 |
| E2E-029 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]` | US-0170 |
| E2E-030 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0175 |
| E2E-031 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0176 |
| E2E-032 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-033 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-034 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-035 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-036 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-037 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-038 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-039 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]` | US-0170 |
| E2E-040 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(not(feature = "rar"))]` | US-0170 |
| E2E-041 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-042 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0176 |
| E2E-043 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0176 |
| E2E-044 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0171 |
| E2E-045 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0169 |
| E2E-046 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0169 |
| E2E-047 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-048 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-049 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-050 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0176 |
| E2E-051 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0176 |
| E2E-052 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0176 |
| E2E-053 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-054 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-055 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-056 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-057 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-058 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-059 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-060 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-061 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-062 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0174 |
| E2E-063 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0174 |
| E2E-064 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0174 |
| E2E-065 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0174 |
| E2E-066 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-067 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-068 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-069 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0176 |
| E2E-070 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0176 |
| E2E-071 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0173 |
| E2E-072 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0176 |
| E2E-073 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-074 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-075 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0173 |
| E2E-076 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0173 |
| E2E-077 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-078 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-079 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-080 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0172 |
| E2E-081 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]` | US-0177 |
| E2E-MOD-001 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive.rs tests (existing test updated) | US-0170 |
| E2E-MOD-002 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive.rs tests (existing test updated) | US-0169 |
| E2E-MOD-003 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive.rs tests (existing test updated) | US-0169 |
| E2E-MOD-004 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive.rs tests (existing test updated) | US-0169 |
| E2E-MOD-005 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive.rs tests (existing test updated) | US-0169 |

Test data: plain archives (nested folders, empty, accents, upper case name, symlink, traversal and absolute names, truncated, text, 0 bytes, multi volume names, quota cases) are generated by the store builder; encrypted data, encrypted names, partly encrypted, RAR4, solid and wrong password fixtures are user provided binary files (status: missing today, blocks US-0174 to US-0177).

## 10. Decisions log

- **DEC-001:** RAR support is the `unrar` crate 0.5.8 behind an opt in Cargo feature `rar`, off by default, forwarded by `mcp-fs`; a build without it keeps today's refusal. **Obliged by:** SPEC-0080/FR-NEW-012, FR-MOD-001, spec 3.3. **Rationale:** the only RAR reader is a wrapper of RARLAB's UnRAR with a non free licence; the user chose an opt in feature. **Alternatives considered:** external `unrar` binary; `unrar-ng` 0.7.7; always on; decline the format. **Source:** user answer (2026-11-04, opt in feature) and spec. **Evidence:** crates.io API `unrar` 0.5.8 `MIT OR Apache-2.0`; context7 `/muja/unrar.rs`.
- **DEC-002:** `.rar` detection is a cfg'd table entry after the tar entries; `is_multi_volume_name` runs after format detection and before the file is read, only when the feature is on; with the feature off `.part1.rar` falls to the unsupported extension message. **Obliged by:** SPEC-0080/FR-NEW-010, FR-NEW-012, FR-NEW-015. **Rationale:** FR-NEW-012 includes multi volume names. **Alternatives considered:** check in `decode_rar` (too late, the file is read). **Source:** spec. **Evidence:** `archive.rs:40-69`, `:146-150`.
- **DEC-003:** the archive bytes are written to a file in `tempfile::tempdir()` per call because the crate opens a path; the temp dir is removed when the call returns; everything runs in `spawn_blocking`. **Obliged by:** SPEC-0080/FR-NEW-001. **Rationale:** `Archive::new(path)` is the only constructor (`unrar-0.5.8/src/archive.rs:49-72`). **Alternatives considered:** a named pipe or `memfd` (platform specific, unsafe). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:154-160` (decode already in `spawn_blocking`).
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-005:** a lying declared size (header smaller than the data) is detected as a read error or size mismatch and reported as `ERR_INVALID_ARGUMENT` `archive entry '<name>' decompressed to more bytes than declared <declared>` (same wording as `archive.rs:339-347` where the count is known, the declared size alone where it is not). **Obliged by:** SPEC-0080/FR-NEW-008. **Rationale:** the C library stops at the declared size. **Alternatives considered:** none that stays safe. **Source:** design. **Evidence:** `archive.rs:325-347`.
- **DEC-006:** entry kinds: `FileHeader::is_directory()` is a folder; a Unix symlink is `file_attr & 0o170000 == 0o120000`; a Windows reparse point is `file_attr & 0x400 != 0`; both are refused with the symlink wording. A RAR5 hard link and a device entry cannot be seen through the safe API; US-0171 starts with a spike on the E2E-008 and E2E-032 fixtures; if the spike shows a hard link extracts as an ordinary file, the story ends BLOCKED_ON_SPEC with an entry in `design-issues.md` (options: accept raw FFI via `unrar_sys`, which needs `unsafe`, or amend E2E-032). **Obliged by:** SPEC-0080/FR-NEW-006. **Rationale:** no `unsafe` in this project. **Alternatives considered:** raw `RARReadHeaderEx` via `unrar_sys` (unsafe); `unrar-ng` (same limit). **Source:** design. **Evidence:** `unrar-0.5.8/src/open_archive.rs:582-640`, `unrar-ng-0.7.7/src/open_archive.rs:929-937`, `unrar_sys-0.5.8/src/lib.rs:163-166`.
- **DEC-007:** zip and 7z get the new password wording by extending `password_required_message` (`archive.rs:615-620`) and routing the three literals (`:441`, `:489`, `:531`) through it; the existing tests that pin the old text (`:1034`, `:1047`, `:2158`, `:2174`) and SPEC-0041's story tests are updated in the same story. **Obliged by:** SPEC-0080/FR-MOD-002, FR-MOD-003. **Rationale:** one wording for three formats. **Alternatives considered:** reword RAR only (leaves zip and 7z less helpful, rejected in the spec). **Source:** spec. **Evidence:** `archive.rs:441`, `:489`, `:531`, `:615-620`.
- **DEC-008:** the shared pipeline stays as is: collision scan, `admit_paths` (project net growth first, session gross after), writes. No pre decode admission. **Obliged by:** SPEC-0080/FR-NEW-009, FR-NEW-014, FR-NEW-022. **Rationale:** the refusal outcomes are the same; admitting before decoding is a hardening outside this spec (BL-0125). **Alternatives considered:** pre decode admission on declared sizes. **Source:** existing code. **Evidence:** `archive.rs:183-205`, `quota.rs:96-158`.
- **DEC-009:** test data: a test only RAR5 store builder (`archive_rar_fixtures.rs`: signature, main header, file headers with vint fields and CRC32, directory, symlink and hard link redirection records) produces the plain cases; encrypted, RAR4 and solid cases need real fixtures made with `rar` and committed under `crates/core/tests/fixtures/rar/` with a `README` line naming the command that made each. **Obliged by:** SPEC-0080 tests (spec Section 9 says fixtures come from the design). **Rationale:** `rar` is not installed, `7z` cannot create RAR; implementing RAR5 AES in the builder is out of proportion. **Alternatives considered:** the `unrar` crate's own `data/*.rar` (content and passwords differ from the spec's concrete data); a builder with encryption. **Source:** user answer (default taken 2026-11-05). **Evidence:** `which rar` returned nothing.
- **DEC-010:** story order: US-0169 (wording, independent), US-0170 (feature and plain extraction), US-0171 (unsafe entries, collision), US-0172 (limits), US-0173 (multi volume, corrupt, missing path), US-0174 (versions, solid, user fixtures), US-0175 and US-0176 (passwords, user fixtures), US-0177 (order, membership, docs, closes). **Obliged by:** n/a (no code impact): story planning. **Rationale:** the closing story needs every other fixture, so it is last. **Alternatives considered:** passwords before limits. **Source:** design. **Evidence:** n/a.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0169 | tell the user how to retry with a password for zip and 7z | P1 | SPEC-0080/FR-MOD-002, SPEC-0080/FR-MOD-003 | main | 2 | no |
| US-0170 | extract a RAR archive when the server has RAR support | P1 | SPEC-0080/FR-NEW-001, SPEC-0080/FR-NEW-012, SPEC-0080/FR-MOD-001 | main | 1 | no |
| US-0171 | refuse unsafe RAR entries, oversized entries and collisions | P1 | SPEC-0080/FR-NEW-006, SPEC-0080/FR-NEW-007, SPEC-0080/FR-NEW-008, SPEC-0080/FR-NEW-009 | US-0170 | 2 | no |
| US-0172 | apply the storage limit and the session allowance to RAR archives | P1 | SPEC-0080/FR-NEW-014, SPEC-0080/FR-NEW-022 | US-0171 | 2 | no |
| US-0173 | refuse multi volume names, corrupt files, a missing path and a folder | P1 | SPEC-0080/FR-NEW-010, SPEC-0080/FR-NEW-011, SPEC-0080/FR-NEW-019, SPEC-0080/FR-NEW-021 | US-0172 | 2 | no |
| US-0174 | read RAR4, RAR5 and solid RAR archives | P2 | SPEC-0080/FR-NEW-001, SPEC-0080/FR-NEW-003 | US-0170 | 2 | no |
| US-0175 | ask for the password of an encrypted RAR and accept the right one | P1 | SPEC-0080/FR-NEW-002, SPEC-0080/FR-NEW-003, SPEC-0080/FR-NEW-004, SPEC-0080/FR-NEW-005, SPEC-0080/FR-NEW-013 | US-0170, US-0169 | 2 | no |
| US-0176 | handle partly encrypted RAR archives and empty or ignored passwords | P2 | SPEC-0080/FR-NEW-016, SPEC-0080/FR-NEW-017, SPEC-0080/FR-NEW-020 | US-0175 | 2 | no |
| US-0177 | pin the RAR order of refusals, check membership and document the format | P1 | SPEC-0080/FR-NEW-015, SPEC-0080/FR-NEW-018 | US-0172, US-0173, US-0174, US-0176 | 2 | yes |

