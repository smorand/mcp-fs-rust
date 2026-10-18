# Pin the docx envelope and the zip declared size message: technical debt design

> Id: SPEC-0037
> Nature: DEBT
> Depth: S
> Depth evidence: one debt item, 2 files (`crates/core/src/docs/docx.rs`, `crates/core/src/tools/archive.rs`), no public contract change, no persisted change
> Status: specified
> Generated on: 2026-10-18
> Behavior change: none
> From backlog: BL-0064
> Depends on: SPEC-0036 (implemented)
> Security: n/a (test hardening; the zip advisory lot is SPEC-0036)
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: Two behaviors SPEC-0036 relies on are pinned loosely
- **Current structure:** (a) SPEC-0036/DT-038 asserts `document.contains(&build_body(md, None))` (`crates/core/src/docs/docx.rs:407`), so a change to the envelope `render_markdown_to_docx` writes around the body (`docx.rs:37-44`, the `format!` with `<w:document>`, `<w:body>`, `<w:sectPr>`) goes unnoticed. (b) `zip_read_error` (`crates/core/src/tools/archive.rs:325-334`) maps zip 9's declared size refusal by matching the text `larger than its declared`; the only test, `e2e_new_026` (`archive.rs:1666-1676`), asserts `contains("mismatch.txt")` and `contains("declared 1")`, not the message.
- **Cost paid today:** a docx envelope regression and a reworded FR-NEW-020 message both pass the suite.
- **Target structure:** the envelope is built by one function used by production and by the test; two new tests assert exact equality.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/docs/docx.rs` | extract the `format!` at `:39-42` into `fn document_xml(body: &str) -> String`, called by `render_markdown_to_docx`; new test DT-040 | 1 production site, 1 new test |
| `crates/core/src/tools/archive.rs` | new test DT-041 next to `e2e_new_026`, reusing `build_zip_stored` (`:1632`) and `patch_zip_declared_size` (`:1655`) | 1 new test |

**Non source occurrences:** none (no doc, CI or config names these).
**Contracts touched:** none; `render_markdown_to_docx` keeps its signature (internal, called from `core/fs_ops.rs:1791`).
**Guardrail coverage:** docx tests in `docx.rs` (tests module), archive tests in `archive.rs` (tests module from `:679`). **Test command:** `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
**Total:** 2 files, 1 production refactor, 2 new tests.

## 3. Class sweep
n/a (not a security lot).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: One envelope builder
> `crates/core/src/docs/docx.rs` SHALL build the `word/document.xml` text through `fn document_xml(body: &str) -> String`, the only place the envelope literal appears, and `render_markdown_to_docx` SHALL call it.
- **Old form:** inline `format!(r#"<?xml ...><w:document ...>{body}<w:sectPr>...</w:document>"#)` in `render_markdown_to_docx`
- **New form:** `let document = document_xml(&body);`
- **Casing convention:** Rust snake_case

#### DR-002 [EARS-U]: Exact docx document pinned
> A test SHALL assert that `word/document.xml` of `render_markdown_to_docx("# Hello\n\nbody\n", None)` equals `document_xml(&build_body("# Hello\n\nbody\n", None))` byte for byte.

#### DR-003 [EARS-U]: Exact declared size message pinned
> A test SHALL assert that extracting a zip whose single stored entry `mismatch.txt` holds `0123456789` with a declared uncompressed size of 1 fails with code `ERR_INVALID_ARGUMENT` and message exactly `archive entry 'mismatch.txt' decompressed to more bytes than declared 1`.

### 4.2 Invariance
#### DR-004 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change, and `tool-contract-golden.json` SHALL be unchanged.

### 4.3 Compatibility
None (no public contract touched).

## 5. Compatibility plan
None.

## 6. Decisions
- **DEC-001:** Extract the envelope into `document_xml` rather than copying the literal into the test. **Alternatives considered:** duplicate the `format!` in the test (rejected: two copies drift, and the test would pin its own copy); keep `contains` (rejected: the cost in DBT-001). **Rationale:** one literal, compared through its own builder plus the body. **Implemented by:** DR-001, DR-002. **Evidence:** `docx.rs:37-44`.
- **DEC-002:** New tests, no edit to SPEC-0036/DT-038 or `e2e_new_026`. **Alternatives considered:** tighten them in place (rejected: a DEBT never edits an existing test without a declared exception, and adding is enough). **Rationale:** the unmodified suite stays the criterion. **Implemented by:** DR-002, DR-003. **Evidence:** `docx.rs:407`, `archive.rs:1673-1675`.
- **DEC-003:** Keep `zip_read_error`'s text match; DT-041 is its tripwire. **Alternatives considered:** treat every `InvalidData` from `read_to_end` as a size lie (rejected: would relabel genuine corruption, a behavior change). **Rationale:** a rewording upstream now fails one named test. **Implemented by:** DR-003. **Evidence:** `archive.rs:325-334`.

## 7. Declared breaks
None.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None (no public contract).

### 8.3 Structural tests
#### DT-040: docx document equals its envelope around the body
- **Validates:** DR-001, DR-002
- **Location:** `crates/core/src/docs/docx.rs` tests
- **Given** markdown `"# Hello\n\nbody\n"` / **When** `render_markdown_to_docx(md, None)` and `word/document.xml` is read back with `zip::ZipArchive` / **Then** it equals `document_xml(&build_body(md, None))`; and the text of `crates/core/src/docs/docx.rs` contains the envelope opening `<w:document xmlns:w=` exactly once (inside `document_xml`).
- **Red before:** does not compile until `document_xml` exists (structural RED); mutation RED after: deleting `<w:sectPr>...</w:sectPr>` from the bytes `pack` receives (not from `document_xml`) fails equality while SPEC-0036/DT-038 stays green.

#### DT-041: exact declared size message
- **Validates:** DR-003
- **Location:** `crates/core/src/tools/archive.rs` tests
- **Given** `build_zip_stored("mismatch.txt", b"0123456789")` patched with `patch_zip_declared_size(&mut bytes, 1)` at `/uploads/mismatch.zip` / **When** extracted / **Then** `e.code == ERR_INVALID_ARGUMENT` and `e.message == "archive entry 'mismatch.txt' decompressed to more bytes than declared 1"`; **And** `fs_ops::list_dir("/uploads")` (second channel, pattern at `archive.rs:2367`) lists only `mismatch.zip`, no `mismatch` directory.
- **Red before:** guardrail (behavior exists); mutation RED: changing the `zip_read_error` text match to a non matching string makes the message the generic `archive is corrupt or not a valid zip file: ...` and the test fails.

#### DT-042: guardrail
- **Validates:** DR-004
- 8.1 green; `git diff main -- tool-contract-golden.json` empty.

### 8.4 Existing tests to modify
None.

## 9. Implementation order
1. Extract `document_xml` (DR-001), write DT-040 (structural RED until extraction exists, then mutation RED), write DT-041 with its mutation RED.
2. 8.1 green. One story.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline checklist, depth S): 1 no forced choice (exact strings, exact inputs); 2 names spelled (`document_xml`, both forms); 3 n/a; 4 tests fully specified; 5 n/a DEBT; 6 no open question; 7 EARS clean, no forbidden modal; 8 DEBT, no behavior change, nothing smuggled; 9 citations opened (`docx.rs:37-44`, `:407`, `archive.rs:325-334`, `:1632`, `:1655`, `:1666-1676`, `:2367`); 10 no missing capability; 11 within budget; 12 G13: test hardening, not an advisory upgrade, Security n/a correct; 14 perimeter 2 files, no existing test modified |

**Amendments applied:** DT-040 structural assertion reworded to the envelope opening (count verified: one occurrence at `docx.rs:41`); DT-041 second channel cited.
