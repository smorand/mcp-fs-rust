# SPEC-0081: Extract a bare gzip file, design

> Spec: sdlc/specs/SPEC-0081-extract-gz-single-file/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-05
> Target tier: 2 (default, no `--tier`, no constitution tier, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024, rustc 1.99.1 locally, `rust-version = "1.94.1"` | `Cargo.toml:7-8` | none, existing |
| Gzip reader | `flate2::read::MultiGzDecoder` (crate already a dependency, miniz_oxide backend) | `flate2 = "1"` (`Cargo.toml:82`) | DEC-004 |
| Async runtime | tokio, decode in `spawn_blocking` | existing | DEC-004 |
| Temp files, MCP, REST | not used | none | none |

Skills loaded: rust. Effect: `make` targets (`make check` is the gate: `format-check`, `lint` with `--all-features -D warnings`, `typecheck`, `security`, `test`), logic in `crates/core`, `thiserror` style errors through the existing `ToolError`, no `unwrap` on fallible paths in production code, tests as `#[cfg(test)] mod tests`. The project does not forbid `unsafe` by lint but contains none; this design adds none.

## 2. Approaches

### Chosen: a dedicated single file flow beside the archive flow
`fs.extract_archive` keeps one entry point. After the shared checks (path, missing, folder, format by suffix, password) a bare gzip file leaves for a new module `tools/archive_single.rs` that owns the single file rules: output name, destination, collision, bounded decode, admission, write. Pros: the order of refusals of FR-NEW-011 is a sequence of statements in one function; the archive flow (zip, 7z, tar) is not touched except two lines; SPEC-0083 and a later xz spec add a reader and nothing else. Cons: about 25 lines of tail (audit, tracing, answer) are shared, so they are extracted into one helper used by both flows. Affected scope: `tools/archive.rs`, `tools/archive_single.rs`, `tools/mod.rs`, `safety.rs`, `quota.rs`.

### Rejected: treat a gzip file as an archive of one entry inside `decode_archive`
It reuses the whole pipeline (pre scan, collision scan, admission, write) and needs about 40 new lines. Rejected (DEC-001): that pipeline decodes before it checks the destination and the collision, and decodes with no size bound, which contradicts FR-NEW-011 (cheap refusals before decoding) and FR-NEW-006 (bomb protection). Adapting it means rewriting its order for every format, which FR-NEW-010 forbids.

### Rejected: a sibling tool `fs.decompress`
A new tool changes the 116 tool contract and its golden file, and splits the two doors. Rejected (DEC-001): the spec says extraction through the existing tool and the existing HTTP route.

## 3. Architecture

```
fs.extract_archive (MCP)  POST /api/fs/{mount}/extract-archive (REST)
              \                /
         tools::archive::extract_archive
           normalize path, node lookup (missing, folder), detect_format, password check
              | ArchiveFormat::Single(SingleFormat::Gzip)
              v
         tools::archive_single::extract_single
           name -> destination -> destination is a file -> collision
           -> caps -> bounded decode -> admit_paths -> write -> audit and answer
```

Components: `archive.rs` keeps the detection table and the zip, 7z, tar decoders. `archive_single.rs` holds `SingleFormat`, `output_name`, `default_destination`, `check_destination`, `check_collision`, `caps`, `read_capped`, `extract_single`. `safety.rs` gains `session_remaining`. Bounded contexts: Archive extraction (both files), Project storage (`quota.rs`, `safety.rs`).

## 4. Data model

None. No persisted concept, no schema change.

## 5. Contracts and interfaces

- Tool and route: unchanged name, parameters and answer keys `destination`, `files_written`, `dirs_created`, `bytes_written`. `TOOL_CONTRACT.txt` and `tool-contract-golden.json` do not change.
- `detect_format(path) -> Result<(ArchiveFormat, usize)>`: new variant `ArchiveFormat::Single(SingleFormat)`; table entry `(".gz", Single(Gzip))` placed after every tar entry and after `.7z`.
- `pub(crate) enum SingleFormat { Gzip }` (`Bzip2` is added by SPEC-0083). `fn label(self) -> &'static str` returns `"gz"` for the corrupt message.
- `pub(crate) async fn extract_single(state: &AppState, mount_id: &str, person: &str, client: &VolumeClient, normalized: &str, suffix_len: usize, destination: Option<&str>, overwrite: bool, format: SingleFormat, archive_bytes: Vec<u8>) -> Result<Value>`.
- `fn output_name(file_name: &str, suffix_len: usize) -> Result<String>`: strips the suffix; empty, `.` and `..` give `ERR_INVALID_ARGUMENT`, `cannot derive an output file name from '<file_name>'` (file name as typed).
- `fn decoder<'a>(format: SingleFormat, bytes: &'a [u8]) -> Box<dyn Read + 'a>`: `GzDecoder` while US-0160 stands, `MultiGzDecoder` from US-0161.
- `fn read_capped(reader: impl Read, cap: u64) -> Result<CappedRead, CapExceeded>` with `CappedRead { bytes: Vec<u8>, high_water: usize }`, chunks of 65 536 bytes. `high_water` is the largest `Vec` length held at any time and is at most `cap + 65 536`.
- `SafetyManager::session_remaining(&self, person: &str, project: &str) -> i64` (`write_quota_bytes - bytes_written`, never negative).
- Error contract (exact, from the spec): `ERR_INVALID_ARGUMENT` `password is not applicable to this archive format`; `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid gz file: <detail>`; `ERR_NO_CLOBBER` `'<path>' already exists; pass overwrite=true to replace it` and `'<path>' is a folder and cannot be replaced`; `ERR_INVALID_ARGUMENT` `'<path>' is a file, not a folder`; `ERR_PATH_OUT_OF_BOUNDS` `path escapes the volume root: <destination>`; `ERR_PROJECT_QUOTA_EXCEEDED` (message from `ToolError::project_quota_exceeded`, starts `project storage quota of`); `ERR_WRITE_QUOTA_EXCEEDED` `session write quota of <N> bytes exceeded`.
- Config keys: none added.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive_single.rs` | create | FR-NEW-001 to FR-NEW-013 | US-0160 to US-0164 |
| `crates/core/src/tools/archive.rs` | modify: variant, table entry, dispatch, password check, shared tail helper, tests of `detect_format`, modified test E2E-NEW-034 | FR-NEW-001, FR-NEW-009, FR-MOD-001 | US-0160, US-0162 |
| `crates/core/src/tools/mod.rs` | modify: `pub(crate) mod archive_single;` next to `archive` (`:31`) | FR-NEW-001 | US-0160 |
| `crates/core/src/api/dataplane.rs` | modify: one HTTP door test in the existing extract block (`:3297-3345`) | FR-NEW-001 | US-0160 |
| `crates/core/src/safety.rs` | modify: `session_remaining` | FR-NEW-012 | US-0163 |
| `crates/core/src/quota.rs` | modify: `project_remaining_for(client, safety, replaced_size) -> Option<i64>` | FR-NEW-006 | US-0163 |
| `README.md` (`:294-295`), `AGENTS.md` (`:130-131`), `.agent_docs/tools.md` (`:178-183`) | modify: formats list, `archive_single.rs` entry | closing story | US-0164 |

Affected existing tests: `e2e_new_034_bare_gz_extension_is_not_supported` (`archive.rs:806-814`) is modified (E2E-MOD-001); `detect_format_maps_every_suffix` (`:870-890`) gains `.gz` rows; every other existing test is kept unmodified.
Affected documentation: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, and a one line gotcha in `.agent_docs/testing.md` about the capped reader hook.

## 7. Compatibility and risks

- Consumers: the tool and route answer the same keys; a `.gz` call that was `ERR_NOT_SUPPORTED` now succeeds or fails with a specific error. `.tar.gz` and `.tgz` keep their table position before `.gz` (`archive.rs:40-50` order), checked by E2E-028 and E2E-029.
- Risk, ordering: `.gz` must follow `.tar.gz` and `.tgz` in `SUFFIXES`, or a `.tar.gz` would be read as a bare gzip file. Signal: E2E-028.
- Risk, race: the caps are computed before decoding and the admission runs after it, so a concurrent writer can use the room in between; the admission then refuses with the same codes and nothing is written (DEC-009). No rollback is needed.
- Memory: a compressed file is read whole (`client.read_bytes`), as today; the decoded output is bounded by the lower cap plus one chunk (DEC-005). The tar family stays unbounded (BL-0124).
- No dependency added or removed. Rollback: revert the commits; no data format changes.

## 9. Test strategy

Tests live in `crates/core/src/tools/archive_single.rs` (`#[cfg(test)] mod tests`) using `Fixture` from `crate::tools::admin::test_support` exactly as `archive.rs` tests do (`:721-735`), driving the typed function `crate::tools::archive::extract_archive` (the function both doors call). The HTTP door tests live in `crates/core/src/api/dataplane.rs` tests with `Harness` (`:3297-3345`). Test function names: `e2e_0081_<nnn>_<slug>`, so `cargo test -p mcp-fs-core --lib e2e_0081_` runs the spec's suite. Gzip fixtures are generated in the test with `flate2::write::GzEncoder` (and a hand assembled header for stored names, comments and extra fields); the bomb is `GzEncoder` over 10 000 000 zero bytes (9 737 bytes, measured). The memory observable of E2E-017 and E2E-048 style tests is asserted on `CappedRead.high_water` and on the refusal: `high_water <= cap + 65 536`, far below 1 000 000.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-002 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-003 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-004 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-005 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-006 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-007 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-008 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-009 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-010 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-011 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-012 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-013 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-014 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-015 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-016 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-017 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-018 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-019 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-020 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-021 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-022 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-023 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-024 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-025 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-026 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-027 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-028 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-029 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-030 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-031 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0160 |
| E2E-032 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-033 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-034 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-035 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-036 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-037 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-038 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-039 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-040 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0162 |
| E2E-041 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0160 |
| E2E-042 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-043 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-044 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-045 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0164 |
| E2E-046 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0161 |
| E2E-047 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-048 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-049 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-050 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-051 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-052 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-053 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0163 |
| E2E-054 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-055 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0162 |
| E2E-056 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0162 |
| E2E-MOD-001 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive.rs tests (existing test updated) | US-0160 |

Test data: all generated, no external file.

## 10. Decisions log

- **DEC-001:** a bare gzip file is handled by a new module `tools/archive_single.rs` reached from `extract_archive` after the shared checks; no new tool, no new route. **Obliged by:** SPEC-0081/FR-NEW-001, FR-NEW-010, FR-NEW-011. **Rationale:** the order of refusals and the bound on decoding cannot be expressed in the zip, 7z, tar pipeline without changing it for every format. **Alternatives considered:** one entry archive inside `decode_archive`; sibling tool. **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:119-215` (decode precedes collision and admission).
- **DEC-002:** format by suffix table entry `(".gz", ArchiveFormat::Single(SingleFormat::Gzip))` placed after all tar entries and `.7z`; `detect_format` signature unchanged. **Obliged by:** SPEC-0081/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** matching is by ordered `ends_with` (`archive.rs:56-69`), compound before plain. **Alternatives considered:** a second table; content sniffing. **Source:** existing code `archive.rs:40-69`. **Evidence:** `archive.rs:40-50`.
- **DEC-003:** output name is the archive file name minus `suffix_len` bytes; default destination is the parent folder of the archive (`/` for `/a.gz`); the destination option goes through `state.safety.normalize_path` (which strips a trailing slash and yields `path escapes the volume root: <destination>`); the destination is created only after a successful decode and counted once in `dirs_created`. **Obliged by:** SPEC-0081/FR-NEW-001, FR-NEW-002, FR-NEW-003. **Rationale:** the messages in the spec match `safety.rs:99-111` verbatim. **Alternatives considered:** the tar rule (a folder named like the archive). **Source:** spec. **Evidence:** `crates/core/src/safety.rs:99-111`, `archive.rs:166-169`.
- **DEC-004:** gzip is read with `flate2::read::MultiGzDecoder` over a `Cursor` of the file bytes inside `spawn_blocking`; any read error maps to `corrupt("gz", e)` (`archive.rs:350-354`); header name, comment and extra fields are never read by our code. **Obliged by:** SPEC-0081/FR-NEW-004, FR-NEW-007, FR-NEW-008. **Rationale:** spike on flate2 1.x: two members decode to `hello world`; `JUNK` after a member and 4 zero bytes both fail with `unexpected end of file`; an empty member gives an empty output; a 0 byte file and a text file fail; a truncated file fails with `incomplete deflate stream`. **Alternatives considered:** `GzDecoder` (stops after the first member, ignores trailing bytes). **Source:** spike run 2026-11-05 on the crates in `Cargo.lock`. **Evidence:** `Cargo.toml:82`.
- **DEC-005:** decoding is `read_capped(reader, cap)`: read 65 536 byte chunks into one `Vec`, stop with `CapExceeded` as soon as the length exceeds `cap`; a length equal to `cap` is accepted. `cap = min(project_cap, session_cap)` where `project_cap = remaining storage + size of the file replaced under overwrite` (only when the project has a limit) and `session_cap = SafetyManager::session_remaining`; on `CapExceeded` the refusal is the project one when `project_cap <= session_cap` (tie goes to the project), else the session one. **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** `quota::check_and_charge` refuses when `used + delta > limit` (`quota.rs:96-113`), which is `new <= limit - used + old`. **Alternatives considered:** decode fully then check (unbounded memory); two passes. **Source:** design. **Evidence:** `crates/core/src/quota.rs:96-113`, `safety.rs:72-81`, `safety.rs:136-147`.
- **DEC-006:** `SafetyManager::session_remaining(person, project) -> i64` is added (read only, `with_session`); `quota.rs` gains `project_remaining_for` reading `safety.project_limit` and `client.meta.used_bytes()`. **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** both numbers are private to those modules today. **Alternatives considered:** make `with_session` public. **Source:** existing code. **Evidence:** `safety.rs:82-92`, `quota.rs:96-113`.
- **DEC-007:** order inside `extract_single`: password (already in the caller), output name, destination normalize, destination is a file, collision, caps, capped decode, admission, write. No byte is written and no folder is created before the decode succeeds. **Obliged by:** SPEC-0081/FR-NEW-011, FR-NEW-005, FR-NEW-013. **Rationale:** it is the spec's list, items 4 to 9. **Alternatives considered:** none, the spec fixes the order. **Source:** spec. **Evidence:** spec Section 6, FR-NEW-011.
- **DEC-008:** the destination check walks the destination and each ancestor from the root with `client.meta.get`; the first node that exists and is not a folder yields `'<that path>' is a file, not a folder`. The collision check reads the output path node: a folder gives the folder message with or without `overwrite`; a file gives the exists message unless `overwrite`. **Obliged by:** SPEC-0081/FR-NEW-005, FR-NEW-013. **Rationale:** `client.exists` cannot tell a file from a folder. **Alternatives considered:** `makedirs` and map its error. **Source:** design. **Evidence:** `archive.rs:130-141` uses `meta.get` and `is_dir`.
- **DEC-009:** after a successful capped decode the write goes through `quota::admit_paths(client, safety, person, mount_id, len, &[(out, len)])` and the returned `Admission` lives until the write lands, as the archive flow does (`archive.rs:183-205`). **Obliged by:** SPEC-0081/FR-NEW-006, FR-NEW-012. **Rationale:** project check on net growth first, session charged gross only after it passed, one admission lock. **Alternatives considered:** a second admission code path. **Source:** existing code. **Evidence:** `quota.rs:136-158`.
- **DEC-010:** the password check in `extract_archive` is widened from `Tar(_)` to `Tar(_) | Single(_)`; message unchanged. **Obliged by:** SPEC-0081/FR-NEW-009. **Rationale:** the spec wants the same refusal before the file is read. **Alternatives considered:** a second check in `extract_single` (too late, the file was already read). **Source:** existing code. **Evidence:** `archive.rs:146-150`.
- **DEC-011:** the answer, the audit line and the tracing event are produced by one helper `finish_extraction(state, person, mount_id, normalized, dest, files, bytes, dirs)` extracted from the tail of `extract_archive` (`archive.rs:242-275`) and used by both flows; no behavior change for existing formats. **Obliged by:** SPEC-0081/FR-NEW-001. **Rationale:** one implementation of the answer shape. **Alternatives considered:** copy the 30 lines. **Source:** AGENTS.md convention (one engine, no copies). **Evidence:** `archive.rs:242-275`.
- **DEC-012:** the stories are cut by capability so each one turns its own tests red to green: US-0160 builds the flow with `GzDecoder`, an unbounded read and a strict rule (anything at the output path is refused), US-0161 switches to `MultiGzDecoder`, US-0162 adds the password, destination and collision rules before the decode, US-0163 adds the caps, US-0164 pins the order of refusals with mutation evidence, writes the documentation and closes. Tests that characterize behavior already built (membership, missing path, folder, tar suffixes) are recorded green on arrival with that reason. **Obliged by:** n/a (no code impact): story planning. **Rationale:** vertical stories with real red evidence. **Alternatives considered:** one story of 57 tests. **Source:** design. **Evidence:** n/a.
- **DEC-013:** documentation: formats list in `README.md:294-295`, structure entry in `AGENTS.md:130-131`, `.agent_docs/tools.md:178-183`; no tool contract change. **Obliged by:** n/a (no code impact): project convention (keep README and AGENTS.md up to date). **Rationale:** AGENTS.md rule. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `AGENTS.md:130`.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0160 | extract a bare gzip file next to the archive | P1 | SPEC-0081/FR-NEW-001, SPEC-0081/FR-NEW-002, SPEC-0081/FR-NEW-003, SPEC-0081/FR-MOD-001 | main | 2 | no |
| US-0161 | decode several gzip members and refuse corrupt input | P1 | SPEC-0081/FR-NEW-004, SPEC-0081/FR-NEW-007, SPEC-0081/FR-NEW-008 | US-0160 | 2 | no |
| US-0162 | gzip password, collision and destination rules | P1 | SPEC-0081/FR-NEW-005, SPEC-0081/FR-NEW-013, SPEC-0081/FR-NEW-009, SPEC-0081/FR-NEW-010 | US-0161 | 2 | no |
| US-0163 | stop gzip decoding at the storage and session limits | P1 | SPEC-0081/FR-NEW-006, SPEC-0081/FR-NEW-012 | US-0162 | 2 | no |
| US-0164 | pin the gzip order of refusals and document it | P1 | SPEC-0081/FR-NEW-011 | US-0163 | 2 | yes |

