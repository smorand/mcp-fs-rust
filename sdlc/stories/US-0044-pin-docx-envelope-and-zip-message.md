# US-0044: pin the docx envelope and the zip size message

> Spec: SPEC-0037 (sdlc/specs/SPEC-0037-pin-zip9-test-drift/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
Two behaviors SPEC-0036 relies on are pinned loosely: the docx document is only checked to contain the body, and the zip declared size refusal message is only checked by substring. Extract the docx envelope into one builder and add two exact tests, with no behavior change.

## Requirements covered
- SPEC-0037/DR-001: `crates/core/src/docs/docx.rs` SHALL build the `word/document.xml` text through `fn document_xml(body: &str) -> String`, the only place the envelope literal appears, and `render_markdown_to_docx` SHALL call it.
- SPEC-0037/DR-002: A test SHALL assert that `word/document.xml` of `render_markdown_to_docx("# Hello\n\nbody\n", None)` equals `document_xml(&build_body("# Hello\n\nbody\n", None))` byte for byte.
- SPEC-0037/DR-003: A test SHALL assert that extracting a zip whose single stored entry `mismatch.txt` holds `0123456789` with a declared uncompressed size of 1 fails with code `ERR_INVALID_ARGUMENT` and message exactly `archive entry 'mismatch.txt' decompressed to more bytes than declared 1`.
- SPEC-0037/DR-004: The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change, and `tool-contract-golden.json` SHALL be unchanged.

## Independent Test
`cargo test -p mcp-fs-core --lib docs::docx` and `cargo test -p mcp-fs-core --lib tools::archive` pass DT-040 and DT-041; then the non regression command; `git diff main -- tool-contract-golden.json` is empty.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0037): extract the envelope into `document_xml` rather than copying the literal into the test.
- DEC-002 (SPEC-0037): new tests, no edit to SPEC-0036/DT-038 (`docx.rs:407`) or `e2e_new_026` (`archive.rs:1666-1676`).
- DEC-003 (SPEC-0037): keep `zip_read_error`'s text match (`archive.rs:325-334`); DT-041 is its tripwire.
### Contracts
- New private `fn document_xml(body: &str) -> String` in `docs/docx.rs`, returning exactly the current `format!` output (`docx.rs:39-42`); `render_markdown_to_docx` signature unchanged.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/docx.rs` | extract `document_xml`; add DT-040 in the tests module |
| `crates/core/src/tools/archive.rs` | add DT-041 in the tests module |
### Existing patterns
- Reading `word/document.xml` back from the docx bytes: the SPEC-0036/DT-038 test (`docs/docx.rs:386-408`).
- Declared size archive: `build_zip_stored` (`archive.rs:1632`), `patch_zip_declared_size` (`archive.rs:1655`), `seed_bytes`, `extract` (`archive.rs:723`) as used by `e2e_new_026` (`archive.rs:1666`); listing with `crate::core::fs_ops::list_dir` (`archive.rs:2367`).

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0037/DT-040: docx document equals its envelope around the body
- Level / Location: unit, `crates/core/src/docs/docx.rs` tests
- Given markdown `"# Hello\n\nbody\n"` / When `render_markdown_to_docx(md, None)` and `word/document.xml` is read back with `zip::ZipArchive` / Then it equals `document_xml(&build_body(md, None))`; and the text of `crates/core/src/docs/docx.rs` contains the envelope opening `<w:document xmlns:w=` exactly once.
- RED: does not compile before `document_xml` exists (structural RED); mutation RED after: removing `<w:sectPr>...</w:sectPr>` from the string `render_markdown_to_docx` passes to `pack` (not from `document_xml`) fails equality while SPEC-0036/DT-038 stays green.

### SPEC-0037/DT-041: exact declared size message
- Level / Location: integration, `crates/core/src/tools/archive.rs` tests
- Given `build_zip_stored("mismatch.txt", b"0123456789")` patched with `patch_zip_declared_size(&mut bytes, 1)`, seeded at `/uploads/mismatch.zip` / When extracted / Then `e.code == ERR_INVALID_ARGUMENT` and `e.message == "archive entry 'mismatch.txt' decompressed to more bytes than declared 1"` / And `fs_ops::list_dir("/uploads")` lists only `mismatch.zip`.
- RED: guardrail; mutation RED: changing the `zip_read_error` text match to a non matching string yields `archive is corrupt or not a valid zip file: ...` and fails the test.

### SPEC-0037/DT-042: guardrail
- The non regression command green; `git diff main -- tool-contract-golden.json` empty.

## Constraints
- Files not to touch: every file except the two above; never edit SPEC-0036/DT-038 or `e2e_new_026`.
- Dependencies not to add: any.
- `document_xml` output must be byte identical to today's `format!`.

## Definition of done
- [ ] RED evidence (structural RED for DT-040, mutation RED for DT-040 and DT-041) in `.implement/US-0044-red.log`.
- [ ] Every acceptance test green; `make check` green; main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none (0).
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: DT-040, DT-041, DT-042 green; DR-001..DR-004 cited by this story.
- [ ] One commit `US-0044: pin the docx envelope and the zip size message`, body `Spec: SPEC-0037` and the DR ids.
