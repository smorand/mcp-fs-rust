# US-0227: nesting refusals through the HTTP door and documentation

> Spec: SPEC-0092 (sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0226
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Prove the HTTP API answers nested forms and a self drawing form exactly as the assistant tool does, and close the spec: the documentation states the three nesting limits, the code search guarantee and the child harness, and the full acceptance set and requirement coverage are checked.

## Requirements covered
- SPEC-0092/FR-NEW-001 (partial: through the HTTP API): IF a page of a PDF draws a form at depth 65 THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> draws PDF forms nested more than 64 deep`.
- SPEC-0092/FR-NEW-006 (partial: through the HTTP API): THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- SPEC-0092/FR-NEW-007 (partial: through the HTTP API): THE document text extraction SHALL answer a PDF whose nestings are within FR-NEW-001 to FR-NEW-003 with the same text, page markers, page count, title, note, companion, preview, charge, audit line and refusals as before, and SHALL answer from an up to date companion without reading the PDF when no refresh is asked.
- SPEC-0092/FR-NEW-009 (partial: the PDF text extraction): THE product SHALL answer a PDF text extraction and a definition or reference search through the HTTP API with the same code, message, results and effects as through the assistant tool.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0092_002 e2e_0092_015`: 2 tests pass. Real assertions: the HTTP status and body, the companion and audit line of E2E-002, the child exit status and `nothing_left` of E2E-015. Then the closing checks below.

## Design excerpt
### Decisions
- **DEC-006 (SPEC-0092):** the three refusals are `ToolError::invalid_argument` with the reason only; `prepare_extraction` adds `could not extract <path>: ` as for every reader error, and the refusal leaves no trace by the existing path (it returns before `land_conversion`, logged once at INFO by `logging::log_tool_failure`). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-005. **Rationale:** SPEC-0088/0089 convention, no new code path.
- **DEC-009 (SPEC-0092):** a test runs in a child exactly when its document or source aborts the process on today's code: E2E-015, 016, 017, 021, 022, 023, 024, 027, 028 and every E2E-SEC test; the others run in process (their documents are within the measured thresholds, so RED shows as a plain assertion). **Obliged by:** SPEC-0092/FR-NEW-006 (spec D-14). **Rationale:** RED evidence must be an assertion, not a crashed runner; child runs cost about one second each.
- **DEC-014 (SPEC-0092):** the closing story documents the limits (`README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md`); `.agent_docs/architecture.md` is updated in US-0226; the tool contract is unchanged (no description names the limits). **Obliged by:** n/a (no code impact): project documentation rule (`AGENTS.md`). **Rationale:** house closing pattern (SPEC-0089 DEC-023).
### Contracts
`POST /api/fs/{mount}/extract-text` `{"path": "/docs/forms.pdf"}`: status 200, the payload of E2E-001 (`md_path`, `format`, `pages` 1, preview); `{"path": "/docs/loop.pdf"}`: status 400, `{"error": "ERR_INVALID_ARGUMENT", "detail": "ERR_INVALID_ARGUMENT: could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep"}` (`crates/core/src/api/dataplane.rs:259-266`, unchanged).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (tests `e2e_0092_002`, `e2e_0092_015`) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
`e2e_0089_002_pdf_through_the_http_api` (`crates/core/src/api/dataplane.rs:3961`), `e2e_0088_029` (`:3936`), `h.post(&u("extract-text"), json!(..))`; `nesting_testkit::{forms_pdf, loop_pdf, in_child}` with `module` `"api::dataplane::tests::"`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0092/E2E-002: Nested forms through the HTTP API
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007, FR-NEW-009; **Priority:** High
- **Given** the document of E2E-001
- **When** a client asks the HTTP API for its text
- **Then** the answer carries the same companion path, format, page count and preview as E2E-001
- **And** the audit trail has one extraction line for `/docs/forms.md`
- Level / Driver / Location: functional e2e / HTTP door `POST /api/fs/{mount}/extract-text` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0092_002_nested_forms_through_the_http_api`
- Green on arrival: US-0222 kept nested forms within the limit unchanged; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-015: Self drawing form through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-006, FR-NEW-009; **Priority:** Critical
- **Given** `/docs/loop.pdf` of E2E-SEC-001
- **When** a client asks the HTTP API for its text
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep`, the same as through the assistant tool
- **And** nothing left; the server is still up
- Level / Driver / Location: functional e2e / HTTP door inside `in_child` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0092_015_self_drawing_form_through_the_http_ap`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: US-0222 refuses the loop at both doors; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-002 | skip the extraction audit line in `core::fs_ops::extract_document`: no audit line, the test fails |
| E2E-015 | in the REST error mapping, answer `ERR_INVALID_ARGUMENT` with status 500: the status assertion fails |

### Closing checks (spec converge)
- Every acceptance test of SPEC-0092 green, run by `make test`: the 28 `E2E-` and 13 `E2E-SEC-` tests (`rg -c 'fn e2e_0092_' crates/core/src` totals 41) and the design tests of US-0222, US-0224 and US-0226.
- Requirement coverage audit: `comm -23 <(rg -o 'FR-NEW-[0-9]{3}' sdlc/specs/SPEC-0092-*/sdlc-spec.md | sort -u) <(rg -o 'SPEC-0092/FR-NEW-[0-9]{3}' sdlc/stories/US-022[2-7]-*.md -N --no-filename | sed 's#.*/##' | sort -u)` prints nothing, and every story US-0222 to US-0226 is `done` or `landed` in `sdlc/stories/_queue.md`.

## Constraints
- Files not to touch: every production file (no production change in this story), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, the SPEC-0088 and SPEC-0089 kits
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: the side findings (folder and git trees nested deeply, spec D-12) are reported, never filed in the backlog

## Definition of done
- [ ] Green on arrival tests (E2E-002, E2E-015): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` clean; `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0089_ e2e_0088_ --test-threads=1` green
- [ ] Docs updated: `README.md` (document extraction: the three nesting limits; code search answers any nesting), `AGENTS.md` (`docs/pdf_bound.rs` bullet: form depth 64, page tree climb 256, map nesting 64; `docs/symbols.rs` iterative scan; key command `cargo test -p mcp-fs-core --lib -- e2e_0092_`), `.agent_docs/tools.md` (`fs.extract_text` nesting refusals and messages; `fs.find_definition`, `fs.find_references` any nesting), `.agent_docs/testing.md` (filter `e2e_0092_`, `nesting_testkit::in_child`, `MCPFS_CHILD_CASE`, why exploit tests run in a child), each entry dated (DEC-014).
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E- and E2E-SEC- of SPEC-0092), and every requirement of SPEC-0092 cited by at least one story with status done or landed (the audit above).
- [ ] One commit `US-0227: nesting refusals through the HTTP door and documentation` on its story branch `story/US-0227-nesting-refusals-through-the-http-door-a`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0092-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0092` and the requirement ids (SPEC-0092/FR-NEW-001, FR-NEW-006, FR-NEW-007, FR-NEW-009).
