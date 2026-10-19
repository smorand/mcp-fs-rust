# SPEC-0041: Refuse zip archives whose headers disagree, design

> Spec: sdlc/specs/SPEC-0041-refuse-inconsistent-zips/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: internal finding (hardening: archives whose directory and entry headers disagree let two readers see different contents)
> Designed on: 2026-10-19
> Target tier: 2 (default; no `--tier`, no constitution field)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| zip reader | `zip` (already in the manifest, `aes-crypto` on, `Cargo.toml:87`) | 9.0.5 (Cargo.lock) | DEC-001 |
| Language | Rust 2024, Rust 1.88 | manifest | n/a |
No dependency added or removed. Skills loaded: none beyond the project's `AGENTS.md`; gate is `make check` (fmt, clippy `--all-features -D warnings`, audit, tests).

## 2. Approaches
### Chosen: one whole archive check after opening, shared by the three readers
Call zip's `ZipArchive::validate_header_consistency` (`zip-9.0.5/src/read/zip_archive.rs:754-768`) once per archive, through one helper, at each of the three entry points (archive extraction, OOXML text extraction, conversion). Pros: covers every field the spec lists (duplicate names `:755`, per entry identity and sizes `read/mod.rs:356-471`, overlap `:763`) in one call; reads headers only, never entry content, so it runs before any decryption (FR-NEW-003, E2E-018). Cons: a second pass over the local headers of each archive (cheap: headers only). Scope: `docs/zip_check.rs` (new), `tools/archive.rs`, `docs/extract.rs`, `core/fs_ops.rs`.
### Rejected: per read check (`ZipReadOptions::enforce_header_consistency`, `read/mod.rs:324`)
Checks only entries actually read: misses parts the OOXML reader skips (E2E-010), duplicate names zip already folded (E2E-004) and overlap (E2E-019). Rejected by DEC-001.

## 3. Architecture
```
tools/archive.rs decode_zip ──┐
docs/extract.rs open_zip ─────┼──> docs/zip_check.rs::header_inconsistency(&mut ZipArchive<R>) -> Option<String>
docs/extract.rs prepare_extraction (cached branch) ─┐      ^
core/fs_ops.rs documentize / write_bytes_documented ┴─> zip_check::ooxml_header_inconsistency(path, &[u8]) -> Option<String>
```
One bounded context (project files). The helper is the single implementation; each caller only formats its own message prefix (DEC-002).

## 4. Data model
No change. Nothing persisted.

## 5. Contracts and interfaces
New module `crates/core/src/docs/zip_check.rs`, `pub(crate) mod zip_check;` in `docs/mod.rs`:
```rust
/// The seven zip based document extensions (FR-NEW-002, FR-NEW-005).
pub(crate) const OOXML_EXTS: &[&str] = &[".docx", ".xlsx", ".xlsm", ".pptx", ".pptm", ".potx", ".ppsx"];
/// Some(reason) when zip's whole archive check fails; reason is zip's text with the
/// leading "invalid Zip archive: " removed, e.g. "header inconsistency detected: duplicate file names in central directory".
pub(crate) fn header_inconsistency<R: std::io::Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Option<String>;
/// Some(reason) when `path` has one of OOXML_EXTS, the bytes open as a zip, and the
/// check fails. A zip that does not open returns None: that case keeps its existing handling.
pub(crate) fn ooxml_header_inconsistency(path: &str, data: &[u8]) -> Option<String>;
```
Exact user visible messages, all `ERR_INVALID_ARGUMENT`:
| Site | Message |
|---|---|
| archive extraction | `archive is corrupt or not a valid zip file: <reason>` via existing `corrupt("zip", reason)` (`tools/archive.rs:350`) |
| text extraction, fresh | `open_zip` (`docs/extract.rs:539`) returns `not a valid OOXML package: <reason>`; the existing wrapper (`extract.rs:358-362`) makes it `could not extract <path>: not a valid OOXML package: <reason>` |
| text extraction, cached | the same full text, built in the cached branch (`extract.rs:336-351`) |
| conversion, on demand | `documentize` returns `could not convert <path>: not a valid OOXML package: <reason>` |
| conversion, on write/upload | `documentation` report `{"error": {"code": "ERR_INVALID_ARGUMENT", "message": "could not convert <path>: not a valid OOXML package: <reason>"}}` (shape of `fs_ops.rs:671`) |
Tool contract and `tool-contract-golden.json` unchanged (no parameter, no description changes).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/docs/zip_check.rs` | create: helper, `OOXML_EXTS`, `#[cfg(test)] pub(crate) mod craft` byte patchers | FR-NEW-001, FR-NEW-002 | US-0048 |
| `crates/core/src/docs/mod.rs` | modify: declare module | FR-NEW-001 | US-0048 |
| `crates/core/src/tools/archive.rs` | modify: `decode_zip` check after password gate; tests; DT-039 b rewritten | FR-NEW-001, FR-NEW-003, FR-MOD-001 | US-0048 |
| `README.md`, `.agent_docs/tools.md` | modify: extraction refusal, duplicate names | FR-NEW-001, FR-MOD-001 | US-0048 |
| `crates/core/src/docs/extract.rs` | modify: `open_zip` check, cached branch check, use `OOXML_EXTS`; tests | FR-NEW-002 | US-0049 |
| `crates/core/src/core/fs_ops.rs` | modify: check in `documentize` and `write_bytes_documented`; tests | FR-NEW-005 | US-0049 |
| `crates/core/src/docs/service.rs` | modify: `StubDocService` test only call counter | FR-NEW-005 (E2E-020..024 "converter never called") | US-0049 |
| `.agent_docs/tools.md` | modify: `fs.extract_text` and `fs.documentize` refusal | FR-NEW-002, FR-NEW-005 | US-0049 |
Affected existing tests: `dt_039_b_identical_raw_names_keep_the_last_entry` (`archive.rs:2494`) becomes E2E-MOD-001 (modify, declared by the spec). Kept unchanged and must stay green: `dt_039_a`, `dt_039_c` (DR-009 message and order), `dt_041_declared_size_lie_message_is_exact` and `e2e_new_026` (their patcher `archive.rs:1655` patches both headers alike, so the check passes), every password test (`archive.rs:2046` builders), every docx/pptx/xlsx extraction test built by `make_zip` (`extract.rs:1505`, consistent by construction).

## 7. Compatibility and risks
- Declared break: identical names were "last wins" (SPEC-0036 DR-008), now refused (FR-MOD-001).
- Risk: archives from tools that write differing timestamps in the two headers are now refused (FR-NEW-001 lists time explicitly; accepted by the spec). Streaming zips with sizes after the data pass (zip allows zero local CRC/sizes with a data descriptor, `read/mod.rs:356`; E2E-017 guards it).
- Cached text extraction now reads the source bytes for the seven extensions before answering (DEC-004): one blob read per cached call.
- Rollback: revert the two story commits; no data written.

## 8. Security design
Threat: an untrusted zip whose central directory and local headers disagree (CWE-436). Sinks: files written by extraction, text returned or stored, bytes handed to the external converter.
| Occurrence (spec 14.2) | Fix site | Mechanism | DEC |
|---|---|---|---|
| #1 archive extraction | `tools/archive.rs` `decode_zip`, between the password gate (`:440`) and the decode loop (`:446`) | whole archive check, refuse | DEC-001, DEC-003 |
| #2 document text | `docs/extract.rs:539` `open_zip` and cached branch `:336` | whole archive check, refuse | DEC-001, DEC-004 |
| #4 conversion | `core/fs_ops.rs` `documentize` after `ensure_input_size` (`:707`), `write_bytes_documented` before `write_companion` (`:666`) | whole archive check, refuse before the converter | DEC-005 |
Exploit tests: E2E-SEC-001 (`archive.rs` tests) payload local name `../../evil.txt` behind central `a.txt`; rejection asserted on the error, absence read through a separate channel: `fs_ops::glob`/listing of the whole project for `*.txt` and `root_has_out`. Red before the fix because extraction writes `/out/a.txt`. E2E-SEC-002 (`extract.rs` tests) payload local name `word/other.xml` behind central `word/document.xml`; absence read by `client.exists("/docs/smuggle.docx.md")` false. Red before the fix because the text `hello` is returned. No secret involved; passwords stay unlogged (existing rule).

## 9. Test strategy
Level "functional e2e" in this repo means the tool level function on a real fixture volume (the pattern of the existing SPEC-0015/0036 tests, `archive.rs` `extract_to_out`, `extract.rs` tests, `fs_ops.rs:3683` doc service tests): same function the MCP and REST adapters call (DEC-006).
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001..006, 011, 014..019, MOD-001, SEC-001 | functional e2e | `extract_to_out` / `extract` fixture | `crates/core/src/tools/archive.rs` tests | US-0048 |
| E2E-007..010, 012, 013, SEC-002 | functional e2e | `fs_ops::extract_text` on fixture volume | `crates/core/src/docs/extract.rs` tests (or `fs_ops.rs` tests where the existing extract_text fixture lives) | US-0049 |
| E2E-020..024 | functional e2e | `fs_ops::documentize` / `write_bytes_documented` with counting `StubDocService` | `crates/core/src/core/fs_ops.rs` tests | US-0049 |
Test data: generated. Malformed archives are well formed zips from `zip::ZipWriter` then byte patched by `zip_check::craft` (DEC-007).

## 10. Decisions log
- **DEC-001:** Use `ZipArchive::validate_header_consistency` once per archive, not the per read option. **Obliged by:** SPEC-0041/FR-NEW-001, FR-NEW-002, FR-MOD-001. **Rationale:** covers skipped parts, folded duplicates and overlap; reads headers only. **Alternatives considered:** per read `enforce_header_consistency`; hand written comparison. **Source:** design. **Evidence:** `zip-9.0.5/src/read/zip_archive.rs:754-768`, `read/mod.rs:324`, `:356-471`.
- **DEC-002:** One helper module `docs/zip_check.rs` returns the reason string (zip's Display with `invalid Zip archive: ` stripped, any other error text kept as is); each caller adds its own prefix. **Obliged by:** FR-NEW-001, FR-NEW-002, FR-NEW-005 user visible texts. **Rationale:** one implementation (AGENTS.md "never reimplement"), three message shapes. **Alternatives considered:** returning `ToolError` from the helper (would hard code one prefix). **Source:** design. **Evidence:** `zip-9.0.5/src/result.rs:70`.
- **DEC-003:** In `decode_zip`, the check runs after the pre scan (symlink, path, DR-009, `archive.rs:417-427`) and after the missing password answer (`:440-442`), before the decode loop. **Obliged by:** FR-NEW-001 business rules, FR-NEW-003, E2E-006, E2E-015, E2E-018. **Rationale:** exactly the order the spec fixes; no decryption needed. **Alternatives considered:** check right after opening (would answer header error before the password and before DR-009). **Source:** spec constraint. **Evidence:** `tools/archive.rs:401-446`.
- **DEC-004:** Text extraction checks in `open_zip` (fresh path, wrapped by `extract.rs:358-362`) and, for `OOXML_EXTS`, in the cached branch of `prepare_extraction` before returning `Prepared::Cached`, reading the source bytes and returning `ToolError::invalid_argument("could not extract {path}: not a valid OOXML package: {reason}")`. A source that does not open as zip on the cached path keeps today's cached answer. **Obliged by:** FR-NEW-002, E2E-013. **Rationale:** the stored text must not bypass the check; reusing the existing wrapper keeps one message shape. **Alternatives considered:** invalidating the cache at write time (misses companions stored before this change). **Source:** design. **Evidence:** `docs/extract.rs:336-351`, `:539-542`.
- **DEC-005:** Conversion checks with `zip_check::ooxml_header_inconsistency(norm, data)` (the seven extensions, whatever `doc_service.extensions` says) after `ensure_documentable` and `ensure_input_size`: in `documentize` returning `ToolError::invalid_argument("could not convert {norm}: not a valid OOXML package: {reason}")`; in `write_bytes_documented` inside the `Some(service)` arm, after the source write, producing the same error in the `documentation.error` report. **Obliged by:** FR-NEW-005 and its business rules. **Rationale:** keeps the existing refusals first and the upload stored. **Alternatives considered:** checking before the source write (would refuse the upload, contradicting EXC-003b). **Source:** spec constraint. **Evidence:** `core/fs_ops.rs:648-672`, `:704-711`, `config.rs:851-854`.
- **DEC-006:** Acceptance tests run at the tool function level on a real fixture volume, as the existing archive and extraction tests do. **Obliged by:** FR-NEW-001..005 tests. **Rationale:** MCP and REST are thin adapters over these functions (AGENTS.md); no message transformation happens above them. **Alternatives considered:** HTTP level tests (no added signal). **Source:** existing code `tools/archive.rs:723`, `core/fs_ops.rs:3683`.
- **DEC-007:** Test archives are produced by `zip::ZipWriter` then patched by `#[cfg(test)] zip_check::craft`: `rename_local(bytes, from, to)` (splices the local header name, fixes its length field, shifts every later local header offset in the central directory and the central directory offset in the end record), `set_local_mtime`, `set_local_compressed_size`, `overlap(bytes)` (adds a second central record pointing at the first entry, fixing the end record counts and sizes). Identical raw names reuse `zip_with_raw_names` (`archive.rs` tests). **Obliged by:** E2E-002..006, 008..013, 016, 018, 019, 021..024, SEC-001, SEC-002. **Rationale:** one crafting implementation shared by both stories. **Alternatives considered:** checked in binary fixtures (opaque, unreviewable). **Source:** design. **Evidence:** existing patcher `archive.rs:1655`.
- **DEC-008:** `StubDocService` (test only) gains a `calls()` counter. **Obliged by:** FR-NEW-005 ("without calling the converter"), E2E-020..024. **Rationale:** the observable for "converter never called". **Alternatives considered:** a new fake. **Source:** existing code `docs/service.rs:679`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0048 | refuse inconsistent zips on archive extraction | P1 | FR-NEW-001, FR-NEW-003, FR-MOD-001, FR-NEW-004 (partial: archives) | main | 1 | no |
| US-0049 | refuse inconsistent documents on text extraction and conversion | P1 | FR-NEW-002, FR-NEW-005, FR-NEW-004 (partial: documents) | US-0048 | 1 | yes |
