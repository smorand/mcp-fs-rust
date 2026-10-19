# US-0049: refuse inconsistent documents on text extraction and conversion

> Spec: SPEC-0041 (sdlc/specs/SPEC-0041-refuse-inconsistent-zips/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0048
> min_tier: 1
> Files touched: 4
> Closes spec: yes

## Objective
A malformed `.docx`/`.xlsx`/`.xlsm`/`.pptx`/`.pptm`/`.potx`/`.ppsx` today yields text (or stored text) as one reader sees it, and is handed to the external converter unchecked. After this story `fs.extract_text` refuses it even when stored text exists, and conversion (`fs.documentize`, conversion on write and upload) refuses it before calling the converter, using the helper US-0048 created.

## Requirements covered
- SPEC-0041/FR-NEW-002: IF any part of a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` document disagrees with the document's directory on its name, alternate name, compression, encryption flag, AES encryption settings, trailing size record flag, last modified time, checksum, compressed size or uncompressed size, or two parts occupy overlapping bytes of the archive, or two parts share a name, THEN the product SHALL refuse the text extraction and return no text.
- SPEC-0041/FR-NEW-005: IF a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` document sent to the document service is malformed in the sense of FR-NEW-002 THEN the product SHALL refuse the conversion without calling the converter and without writing a companion.
- SPEC-0041/FR-NEW-004 (partial: documents): The product SHALL extract every well formed archive and document exactly as before this change.

## Independent Test
`make check` green; E2E-021 calls `fs_ops::documentize` on a fixture volume holding `/docs/bad.docx` (local name differs from central) with a counting `StubDocService`, asserts `ERR_INVALID_ARGUMENT` with prefix `could not convert /docs/bad.docx: not a valid OOXML package: header inconsistency detected`, `stub.calls() == 0`, and `client.exists("/docs/bad.docx.md")` false.

## Design excerpt
### Decisions
- **DEC-002:** One helper module `docs/zip_check.rs` returns the reason string (zip's Display with `invalid Zip archive: ` stripped, any other error text kept as is); each caller adds its own prefix. **Obliged by:** FR-NEW-001, FR-NEW-002, FR-NEW-005 user visible texts. **Rationale:** one implementation (AGENTS.md "never reimplement"), three message shapes. **Alternatives considered:** returning `ToolError` from the helper (would hard code one prefix). **Source:** design. **Evidence:** `zip-9.0.5/src/result.rs:70`.
- **DEC-004:** Text extraction checks in `open_zip` (fresh path, wrapped by `extract.rs:358-362`) and, for `OOXML_EXTS`, in the cached branch of `prepare_extraction` before returning `Prepared::Cached`, reading the source bytes and returning `ToolError::invalid_argument("could not extract {path}: not a valid OOXML package: {reason}")`. A source that does not open as zip on the cached path keeps today's cached answer. **Obliged by:** FR-NEW-002, E2E-013. **Rationale:** the stored text must not bypass the check; reusing the existing wrapper keeps one message shape. **Alternatives considered:** invalidating the cache at write time (misses companions stored before this change). **Source:** design. **Evidence:** `docs/extract.rs:336-351`, `:539-542`.
- **DEC-005:** Conversion checks with `zip_check::ooxml_header_inconsistency(norm, data)` (the seven extensions, whatever `doc_service.extensions` says) after `ensure_documentable` and `ensure_input_size`: in `documentize` returning `ToolError::invalid_argument("could not convert {norm}: not a valid OOXML package: {reason}")`; in `write_bytes_documented` inside the `Some(service)` arm, after the source write, producing the same error in the `documentation.error` report. **Obliged by:** FR-NEW-005 and its business rules. **Rationale:** keeps the existing refusals first and the upload stored. **Alternatives considered:** checking before the source write (would refuse the upload, contradicting EXC-003b). **Source:** spec constraint. **Evidence:** `core/fs_ops.rs:648-672`, `:704-711`, `config.rs:851-854`.
- **DEC-006:** Acceptance tests run at the tool function level on a real fixture volume, as the existing archive and extraction tests do. **Obliged by:** FR-NEW-001..005 tests. **Rationale:** MCP and REST are thin adapters over these functions (AGENTS.md); no message transformation happens above them. **Alternatives considered:** HTTP level tests (no added signal). **Source:** existing code `tools/archive.rs:723`, `core/fs_ops.rs:3683`.
- **DEC-007:** Test archives are produced by `zip::ZipWriter` then patched by `#[cfg(test)] zip_check::craft`: `rename_local(bytes, from, to)` (splices the local header name, fixes its length field, shifts every later local header offset in the central directory and the central directory offset in the end record), `set_local_mtime`, `set_local_compressed_size`, `overlap(bytes)` (adds a second central record pointing at the first entry, fixing the end record counts and sizes). Identical raw names reuse `zip_with_raw_names` (`archive.rs` tests). **Obliged by:** E2E-002..006, 008..013, 016, 018, 019, 021..024, SEC-001, SEC-002. **Rationale:** one crafting implementation shared by both stories. **Alternatives considered:** checked in binary fixtures (opaque, unreviewable). **Source:** design. **Evidence:** existing patcher `archive.rs:1655`.
- **DEC-008:** `StubDocService` (test only) gains a `calls()` counter. **Obliged by:** FR-NEW-005 ("without calling the converter"), E2E-020..024. **Rationale:** the observable for "converter never called". **Alternatives considered:** a new fake. **Source:** existing code `docs/service.rs:679`.
### Data model
None.
### Contracts
Consumes from US-0048 (`crates/core/src/docs/zip_check.rs`): `OOXML_EXTS`, `header_inconsistency`, `ooxml_header_inconsistency`, `craft::*`.
Messages, all `ERR_INVALID_ARGUMENT`:
- text, fresh: `open_zip` (`docs/extract.rs:539`) returns `not a valid OOXML package: <reason>`; existing wrapper `extract.rs:358-362` yields `could not extract <path>: not a valid OOXML package: <reason>`.
- text, cached branch (`extract.rs:336-351`), extension in `OOXML_EXTS`: read source bytes, on Some(reason) return `could not extract <path>: not a valid OOXML package: <reason>`.
- `documentize` (`core/fs_ops.rs:688`), after `ensure_input_size` (`:707`): `could not convert <norm>: not a valid OOXML package: <reason>`.
- `write_bytes_documented` (`fs_ops.rs:635`), inside `Some(service)` before `write_companion` (`:666`): the same error lands as `{"error": {"code": "ERR_INVALID_ARGUMENT", "message": ...}}` in `documentation`; the source write stays.
- `StubDocService` (`docs/service.rs:679`, test only): `pub fn calls(&self) -> usize`, incremented in `convert`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify: `open_zip`, cached branch, routing at `:166-168` may use `OOXML_EXTS`; tests |
| `crates/core/src/core/fs_ops.rs` | modify: `documentize`, `write_bytes_documented`; tests |
| `crates/core/src/docs/service.rs` | modify: test only call counter |
| `.agent_docs/tools.md` | modify: `fs.extract_text` and `fs.documentize` (`:204-211`) refusal notes |
### Existing patterns
- OOXML test builder `make_zip(parts)` (`docs/extract.rs:1505`); doc service tests with `StubDocService::ok(MD)` (`core/fs_ops.rs:3683-3745`, `documentize_writes_the_companion_and_honours_no_clobber` `:3863`, `convert_deck` `:4145`); `with_max_input_bytes` and extension lists on `StubDocService` (`service.rs:670`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

"The message of E2E-008, carrying its own path" = code `ERR_INVALID_ARGUMENT`, prefix `could not extract <path>: not a valid OOXML package: header inconsistency detected`. Conversion refusals: prefix `could not convert <path>: not a valid OOXML package: header inconsistency detected`. "Converter never called" = `stub.calls() == 0`; E2E-020 asserts `stub.calls() == 1`. E2E-013: write the companion `/docs/stale.docx.md` (`hello`) with an mtime not older than the source so the cached branch is taken, then assert the refusal and that the companion is left as is.

### SPEC-0041/E2E-007: well formed document text unchanged
- **Category:** journey · **Scenario:** SC-002 · **Requirements:** FR-NEW-004
- **Given** `/docs/memo.docx`, a well formed Word document whose body is `hello`
- **When** Olivia extracts its text
- **Then** the returned text contains `hello`
- **Priority:** Critical
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-008: document whose main part is named differently is refused
- **Category:** failure · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/bad.docx`, the document of E2E-007 with its main text part named differently in that part's own header
- **When** Olivia extracts its text
- **Then** refused with a message starting `could not extract /docs/bad.docx: not a valid OOXML package: header inconsistency detected`; no text is returned
- **Priority:** Critical
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-009: document with two parts of the same name is refused
- **Category:** failure · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/dup.docx`, the document of E2E-007 with its main text part present twice under the same name
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **Priority:** High
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-010: a malformed part the extraction does not read still refuses
- **Category:** edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/props.docx`, the document of E2E-007 plus a properties part named differently in its own header
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-012: macro enabled workbook with a malformed part is refused
- **Category:** edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/bad.xlsm`, a workbook whose worksheet part is named differently in its own header
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-013: previously stored text does not bypass the check
- **Category:** failure · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/stale.docx`, malformed as in E2E-008, whose text `hello` was extracted and stored before this change
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path; no text is returned
- **Priority:** High
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-020: well formed document converts as today
- **Category:** journey · **Scenario:** SC-003 · **Requirements:** FR-NEW-005, FR-NEW-004
- **Given** the document service enabled and `/docs/memo.docx` of E2E-007
- **When** Olivia converts it on demand
- **Then** the companion `/docs/memo.docx.md` exists and the converter was called once
- **Priority:** High
- Level / Driver / Location: functional e2e / `fs_ops::documentize` or `write_bytes_documented` with counting `StubDocService` / `crates/core/src/core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-021: malformed document refused on demand, converter not called
- **Category:** failure, side effect · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled and `/docs/bad.docx` of E2E-008
- **When** Olivia converts it on demand
- **Then** refused with code `ERR_INVALID_ARGUMENT` and a message starting `could not convert /docs/bad.docx: not a valid OOXML package: header inconsistency detected`
- **And** the converter was never called, and no `/docs/bad.docx.md` exists
- **Priority:** Critical
- Level / Driver / Location: functional e2e / `fs_ops::documentize` or `write_bytes_documented` with counting `StubDocService` / `crates/core/src/core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-022: malformed deck uploaded with conversion: file stored, conversion refused
- **Category:** edge, side effect · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled and a `.pptx` deck whose slide part is named differently in its own header
- **When** Olivia uploads it as `/docs/deck.pptx` with the conversion option
- **Then** `/docs/deck.pptx` is stored with the uploaded bytes; the report's `documentation.error.code` is `ERR_INVALID_ARGUMENT` and its `documentation.error.message` starts with `could not convert /docs/deck.pptx: not a valid OOXML package: header inconsistency detected`
- **And** the converter was never called, and no `/docs/deck.pptx.md` exists
- **Priority:** High
- Level / Driver / Location: functional e2e / `fs_ops::documentize` or `write_bytes_documented` with counting `StubDocService` / `crates/core/src/core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-023: operator added extension still checked
- **Category:** edge · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled with its accepted list set to `.xlsm`, and `/docs/bad.xlsm` of E2E-012
- **When** Olivia converts it on demand
- **Then** refused with code `ERR_INVALID_ARGUMENT` and a message starting `could not convert /docs/bad.xlsm: not a valid OOXML package: header inconsistency detected`; the converter was never called; no `/docs/bad.xlsm.md` exists
- **Priority:** Medium
- Level / Driver / Location: functional e2e / `fs_ops::documentize` or `write_bytes_documented` with counting `StubDocService` / `crates/core/src/core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-024: document with two parts of the same name refused on demand
- **Category:** failure · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled and `/docs/dup.docx` of E2E-009
- **When** Olivia converts it on demand
- **Then** refused with a message starting `could not convert /docs/dup.docx: not a valid OOXML package: header inconsistency detected`; the converter was never called; no `/docs/dup.docx.md` exists
- **Priority:** High
- Level / Driver / Location: functional e2e / `fs_ops::documentize` or `write_bytes_documented` with counting `StubDocService` / `crates/core/src/core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

### SPEC-0041/E2E-SEC-002: a disguised part cannot change the text a document yields
- **Covers:** class sweep occurrence #2 (Section 14.2)
- **Red before the fix because:** today the text `hello` is returned and the refusal assertion fails.
- **Given** `/docs/smuggle.docx`, the document of E2E-007 whose main part is named `word/other.xml` in its own header
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **And** the document's stored companion text, read separately, is absent (no extraction result was kept)
- Level / Driver / Location: functional e2e / `fs_ops::extract_text` on fixture volume / `crates/core/src/docs/extract.rs` or `core/fs_ops.rs` tests
- Cleanup: fixture volume is per test, dropped at end

E2E-SEC-002 red before the fix because: today the text `hello` is returned, so `unwrap_err` panics.

## Constraints
- Files not to touch: `tools/archive.rs` (US-0048), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`.
- Dependencies not to add: any.
- Patterns to avoid: checking before the source write in `write_bytes_documented`; gating the conversion check on `doc_service.extensions`; reimplementing the check in `api/dataplane.rs`.
- Scope boundary: nothing else in SPEC-0041 remains after this story.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation RED for E2E-007 and E2E-020, already green).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing docx/pptx/xlsx extraction test, every doc service test in `fs_ops.rs` and `api/dataplane.rs` (`:2849` converter failure keeps `ERR_INTERNAL_ERROR`).
- [ ] Docs updated: `.agent_docs/tools.md`.
- [ ] Self review: full.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E- of SPEC-0041, including US-0048's), and every requirement of SPEC-0041 cited by at least one story with status done or landed.
- [ ] One commit `US-0049: refuse inconsistent documents on text extraction and conversion`, body `Spec: SPEC-0041` and the requirement ids.
