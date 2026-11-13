# US-0243: other formats and replaced files stay as they are, and the documentation

> Spec: SPEC-0094 (sdlc/specs/SPEC-0094-mark-damaged-pdf-page-content/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0242
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
Pin that other formats keep their answers (a cached non PDF answer gets no note) and that a PDF replaced by a clean file loses the note, then document the damage note and close the spec: the whole acceptance set of SPEC-0094 green and every requirement covered.

## Requirements covered
- SPEC-0094/FR-NEW-003 (partial: a document that is not a PDF, and a replaced file): THE text extraction SHALL NOT write a damage note for a PDF in which every page content is complete, nor for any document that is not a PDF.
- SPEC-0094/FR-NEW-005 (partial: a stored text copy of a non PDF stays as it is): WHEN a member extracts the text of a PDF and the answer comes from its stored text copy, THE text extraction SHALL include in that answer a note equal to the note of the extraction that stored the copy: the damage note, or the empty text. The note is present in the answer even when it is empty.
- SPEC-0094/FR-NEW-006 (partial: a replaced PDF is read again): WHEN a member extracts the text of a PDF with the refresh option, THE text extraction SHALL read the PDF again and answer exactly as for a first extraction, damage note included.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0094_009 e2e_0094_013` (2 tests), then the whole set `cargo test -p mcp-fs-core --lib -- e2e_0094_` (21 tests green). Documentation: `rg -c 'SPEC-0094' AGENTS.md .agent_docs/tools.md .agent_docs/testing.md` prints three files each with at least 1. Then the closing checks below.

## Design excerpt
### Decisions
- **DEC-010 (SPEC-0094):** documentation goes in the closing story: `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md`; `README.md` unchanged (checked: `rg -n 'extract_text|damaged' README.md` must have no line to update).
- **DEC-008 (SPEC-0094):** functional e2e through `fs_ops::extract_document` on a `nesting_fixture`, in `tools/damage_e2e.rs`.
### Contracts
No production change. E2E-009 advances the replaced PDF's mtime: after the first extraction wait 1100 ms (real time) before writing the replacement, so `companion mtime >= source mtime` (`extract.rs:625`) is false and the next call extracts afresh; then the refresh call. E2E-013 builds `memo.docx` and `sheet.xlsx` with `archive_testkit::office_zip` (as `office_bound_e2e.rs` does) and calls `extract_text` on each.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/damage_e2e.rs` | modify (two tests) |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |

### Existing patterns
- The `AGENTS.md` paragraph on `docs/pdf_bound.rs` (the line starting `pdf_bound.rs` (PDF text extraction, measured then decoded`) and the command list at the top (`env -u MCPFS_TEST_PG_DSN cargo test ... e2e_0092_ ...`): add one sentence on the damage note (SPEC-0094) and one command line `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0094_ --test-threads=1`.
- `.agent_docs/tools.md:431` (the `fs.extract_text` row) and the paragraph at `:437-456` on `cached: true`: state that a PDF answer carries `note` (the damage note, empty when none), that the note of a cached PDF answer is the one recorded with the conversion (`doc_artifact_sets.note`), that a copy stored before SPEC-0094 answers an empty note until `refresh`, and the exact wording with the 20 page cap.
- `.agent_docs/testing.md:441` (the SPEC-0088 entry shape): add a SPEC-0094 entry: the command, the 21 tests, `tools/damage_e2e.rs`, the `pdf_pages` builder, the mtime wait of E2E-009.
- Each entry dated `2026-11-12`, no dashes as punctuation.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `./test.sh`).

### SPEC-0094/E2E-009: File replaced by a clean one
- **Category:** state transition
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003, FR-NEW-006
- **Given** `report-3p.pdf` extracted once, then replaced by the complete `clean-3p.pdf` content under the same name
- **When** the member extracts it without, then with, the refresh option
- **Then** both answers hold the text `Alpha`, `Beta`, `Gamma` and an empty note
- Level / Driver / Location: functional e2e / `extract_text` helper, replace the file between calls / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_009_a_file_replaced_by_a_clean_one`
- Green on arrival after US-0242 (a replaced file is read again by the mtime rule). Mutation proof: make the cache branch ignore the mtime comparison (`extract.rs:625`) for `.pdf`; the first call after the replacement then answers `cached` true and the test fails.

### SPEC-0094/E2E-013: Other formats are untouched
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003
- **Given** `notes.txt` containing `(unclosed`, `memo.docx` with one paragraph `Hello`, and `sheet.xlsx` with cell A1 holding `42`
- **When** the member extracts the text of each
- **Then** each note is exactly empty, the text of `notes.txt` is `(unclosed`, and no note contains `damaged`
- **And** extracting `memo.docx` a second time gives the answer from its stored text copy exactly as it did before this change
- Level / Driver / Location: functional e2e / `extract_text` helper on `notes.txt`, `memo.docx`, `sheet.xlsx`, then `memo.docx` again / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_013_other_formats_are_untouched`
- Green on arrival. Mutation proof: insert `"note"` into the cached payload for every extension in the cache branch; the second `memo.docx` answer then gains a `note` key and the test fails (the test compares the whole cached answer to the first answer with `cached` set true).

## Constraints
- Files not to touch: every production source file; `TOOL_CONTRACT.txt`, `tool-contract-golden.json`, `README.md`.
- Dependencies not to add: none.
- Patterns to avoid: describing behavior the code does not have; dashes as punctuation in prose.
- Scope boundary: nothing else; any production change found necessary is a defect of US-0241 or US-0242 to fix there.

## Definition of done
- [ ] RED or green on arrival evidence recorded for both tests (mutation proofs run and reverted).
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit.
- [ ] Non regression: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, the golden contract tests unchanged.
- [ ] Docs updated: `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` as listed above.
- [ ] Self review: the documentation states the 20 page cap, the stored copy rule, the refresh rule and the known limit (pages after a reader stop are not listed, DEC-003).
- [ ] Closes spec: the spec's full acceptance set green (all 21 `E2E-` tests, `cargo test -p mcp-fs-core --lib -- e2e_0094_`), and every requirement `FR-NEW-001` to `FR-NEW-009` of SPEC-0094 cited by at least one story with status done or landed.
- [ ] One commit `US-0243: other formats and replaced files stay as they are, and the documentation` on the work branch, body with `Summary:`, `Spec: SPEC-0094` and the requirement ids.
