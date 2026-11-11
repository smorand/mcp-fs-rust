# US-0226: nesting refusals in order, without trace, walk and reader in step

> Spec: SPEC-0092 (sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0225
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Pin what the three refusals of US-0222 to US-0224 must keep: their place among the SPEC-0089 refusals (membership, the load, the pages in order), no companion, artifact, charge or audit line, one informational log line, the stored companion untouched on refresh, an upper case suffix, three attacks at once. And prove the walk stops exactly where the reader would: at each of the walk's eight stops, a self drawing form placed after the defect must still end in the reader's contained failure, never in an abort.

## Requirements covered
- SPEC-0092/FR-NEW-004 (partial: membership, the load and the page order before the nesting refusals): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in the order of SPEC-0089 FR-NEW-007, item 4 read as: pages in page order; for each page, first FR-NEW-002 for the resources, then FR-NEW-002 for the page size, then each decode and draw in drawing order, where a draw checks FR-NEW-001 before its stream is measured and a character map or code map checks SPEC-0089 FR-NEW-002, then SPEC-0089 FR-NEW-004, then FR-NEW-003.
- SPEC-0092/FR-NEW-005 (partial: repeat, refresh, the operator trail): THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-003.
- SPEC-0092/FR-NEW-006 (partial: concurrent attacks; the walk stops in step with the reader): THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- SPEC-0092/FR-NEW-007 (partial: answer from an up to date companion without reading the PDF): THE document text extraction SHALL answer a PDF whose nestings are within FR-NEW-001 to FR-NEW-003 with the same text, page markers, page count, title, note, companion, preview, charge, audit line and refusals as before, and SHALL answer from an up to date companion without reading the PDF when no refresh is asked.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0092_011 e2e_0092_016 e2e_0092_018 e2e_0092_019 e2e_0092_020 e2e_0092_023 e2e_0092_024 e2e_0092_027 e2e_0092_028 parity_`: 17 tests pass (9 spec tests, 8 parity tests). Real assertions: exact refusal messages and codes, the stored companion bytes, the audit trail, the captured log events, the child exit status of each parity case.

## Design excerpt
### Decisions
- **DEC-006 (SPEC-0092):** the three refusals are `ToolError::invalid_argument` with the reason only; `prepare_extraction` adds `could not extract <path>: ` as for every reader error, and the refusal leaves no trace by the existing path (it returns before `land_conversion`, logged once at INFO by `logging::log_tool_failure`). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-005. **Rationale:** SPEC-0088/0089 convention, no new code path.
- **DEC-009 (SPEC-0092):** a test runs in a child exactly when its document or source aborts the process on today's code: E2E-015, 016, 017, 021, 022, 023, 024, 027, 028 and every E2E-SEC test; the others run in process (their documents are within the measured thresholds, so RED shows as a plain assertion). **Obliged by:** SPEC-0092/FR-NEW-006 (spec D-14). **Rationale:** RED evidence must be an assertion, not a crashed runner; child runs cost about one second each.
- **DEC-010 (SPEC-0092):** "the server is still up" is `still_up(&f, pid_before)`: within 1 s after the answer, `McpServer::new(state, "leo@example.com").fs_read` of `/notes/hello.txt` in `orbit` returns `hello`, and `std::process::id()` equals `pid_before` read before the call. E2E-SEC-001 and E2E-SEC-011 also run `bystander_loop` (a task reading every 50 ms until told to stop, each read asserted `hello` within 1 s) while the attacked call runs. `nesting_fixture()` is `archive_testkit::office_fixture()` plus `orbit` seeded with `f.seed_project("orbit", "leo@example.com")` and the note. **Obliged by:** SPEC-0092/FR-NEW-006, spec 9 header. **Rationale:** reads through the real handler, from another project and member.
- **DEC-013 (SPEC-0092):** walk and reader parity at every stop: one child test per `break` of the walk (Tf without a name; Tf on a font missing from the resources; `cs` without a name; `cs` on a colour space missing from the resources; `Do` without a name; `Do` on an XObject missing from the resources; `Do` on an object that is not a stream; `Do` on a form whose content cannot be parsed), each page content putting the defect first and the self drawing form of E2E-SEC-001 after it. Each asserts the child exits normally and the answer is `could not extract /docs/parity.pdf: pdf parser failed on this document` (the reader panics at the defect, contained). A red parity test is a story failure to report (the reader would continue where the walk stops), never patched silently. **Obliged by:** SPEC-0092/FR-NEW-006, FR-NEW-007, spec 14.3 parity. **Rationale:** the walk counts only what it reaches.
- **DEC-014 (SPEC-0092):** the closing story documents the limits (`README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md`); `.agent_docs/architecture.md` is updated in US-0226; the tool contract is unchanged (no description names the limits). **Obliged by:** n/a (no code impact): project documentation rule (`AGENTS.md`). **Rationale:** house closing pattern (SPEC-0089 DEC-023).
### Contracts
No production change expected. If a parity test is red on arrival (the reader continues where the walk stops), that is a story failure: stop and report it with the case; do not change the walk's stop to a refusal.
- `.agent_docs/architecture.md`: a dated paragraph in the PDF extraction section: the walk's three nesting refusals (64 forms, 256 page groups, 64 map levels), checked before the reader runs, in the order of FR-NEW-004; the code search scan uses an explicit stack.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/nesting_e2e.rs` | modify |
| `crates/core/src/tools/nesting_testkit.rs` | modify (order and parity builders) |
| `.agent_docs/architecture.md` | modify |

Read only (imported, never modified): `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/archive_testkit.rs`.
### Existing patterns
SPEC-0088 E2E-035 for the log capture (`crates/core/src/tools/office_bound_e2e.rs:939-948`: `crate::logging::capture::lock_for_test()`, a current thread runtime built in the test, `capture::clear()`, `capture::events()`), here inside `in_child`; `McpServer::fs_extract_text` with `ExtractTextArgs` (`crates/core/src/mcp/server.rs:2402`); SPEC-0089's `object_stream_nulls` (`crates/core/src/tools/pdf_testkit.rs:181`) and `bomb` (`:81`).
### Fixtures
- `both_pdf()` (E2E-019): `loop_pdf`'s objects plus object streams holding 2 000 000 `null` objects (`PdfWriter::object_stream_nulls`), stored in under 1 000 000 bytes.
- `two_pages_bomb_then_loop_pdf()` (E2E-020): `pages_writer(vec![bomb(""), plain("/A Do")], "/XObject << /A 10 0 R >>")` with object 10 the self drawing form.
- `/docs/LOOP.PDF` (E2E-016): the bytes of `loop_pdf()`.
- Parity builders, `parity_pdf(defect: &str, resources: &str)`: page content `<defect> /A Do`, `/A` the self drawing form, one per stop: `Tf` with no operand (`12 Tf`); `/F9 12 Tf` with no `/F9` in the fonts; `cs` with no operand (`cs`); `/CS9 cs` with no `/CS9` colour space; `Do` with no operand (`Do`); `/X9 Do` with no `/X9` in the XObjects; `/N Do` with `/XObject << /N 12 0 R >>` and object 12 a dictionary, not a stream; `/G Do` with object 13 a form whose content is `(unclosed` (not parsable). Each stored as `/docs/parity.pdf`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0092/E2E-011: Up to date companion without refresh
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/loop.pdf` of E2E-SEC-001 and an up to date `/docs/loop.md` reading `kept`
- **When** the member asks for its text without refresh
- **Then** the answer is cached and its preview reads `kept` (reading the PDF refuses the call, E2E-SEC-001)
- **And** the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_011_up_to_date_companion_without_refresh`
- Green on arrival: the companion answers without reading the PDF today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-016: Upper case suffix
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** the bytes of `/docs/loop.pdf` stored as `/docs/LOOP.PDF`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/LOOP.PDF: page 1 draws PDF forms nested more than 64 deep`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs inside `in_child`
- Function: `e2e_0092_016_upper_case_suffix`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: US-0222 refuses the loop whatever the suffix case; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-018: Not a member, self drawing form
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/loop.pdf` and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its text
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- Level / Driver / Location: functional e2e / `McpServer::new(state, "eve@example.com").fs_extract_text` (the real handler) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_018_not_a_member_self_drawing_form`
- Green on arrival: membership is checked first today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-019: Load refusal before a form loop
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/both.pdf`, `/docs/loop.pdf` with object streams holding 2 000 000 more objects written `null`, stored in under 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/both.pdf: the PDF holds more than 1000000 objects`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_019_load_refusal_before_a_form_loop`
- Green on arrival: the load refuses the object count before any page (SPEC-0089); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-020: Page 1 stream limit before a page 2 form loop
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/two.pdf` whose page 1 content is a PDF bomb stream (SPEC-0089: under 300 000 stored bytes inflating to 200 000 000 spaces) and whose page 2 draws the self drawing form of E2E-SEC-001
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/two.pdf: a PDF stream decodes to more than 104857600 bytes`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_020_page_1_stream_limit_before_a_page_2_f`
- Green on arrival: page 1 is refused by its stream before page 2 is walked; then the mutation below
- Note: takes `archive_testkit::heavy()` first
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-023: Refusal repeated
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/docs/loop.pdf`
- **When** the member asks for its text twice
- **Then** both answers are the same refusal
- **And** nothing left; the audit trail has no line for `/docs/loop.md`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs inside `in_child`
- Function: `e2e_0092_023_refusal_repeated`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: a refusal returns before `land_conversion` (DEC-006); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-024: Refresh with an existing companion
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-005, FR-NEW-007; **Priority:** High
- **Given** `/docs/loop.pdf` and an existing `/docs/loop.md` reading `kept`
- **When** the member asks for its text with refresh
- **Then** the refusal is the form depth message for page 1 of `/docs/loop.pdf`
- **And** `/docs/loop.md` still reads `kept`; nothing else left; the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs with `refresh` true inside `in_child`
- Function: `e2e_0092_024_refresh_with_an_existing_companion`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: a refusal never touches the stored companion; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-027: Operator trail of a refusal
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/docs/loop.pdf`
- **When** the member asks for its text
- **Then** the server log holds exactly one informational line naming `ERR_INVALID_ARGUMENT` and `could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep`
- **And** the audit trail holds no line for the call
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` under `crate::logging::capture::lock_for_test` on a current thread runtime, inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_027_operator_trail_of_a_refusal`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: `logging::log_tool_failure` logs one INFO line per refused call; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-028: Two attacks and a normal extraction at once
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/docs/loop.pdf`, `/docs/pair.pdf` of E2E-SEC-003 and `/docs/forms.pdf` of E2E-001
- **When** three calls of the member, one per document, start at the same time
- **Then** the first two are refused with the form depth message for page 1 of their document; the third succeeds as in E2E-001
- **And** the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs, three calls joined with `tokio::join!`, inside `in_child`
- Function: `e2e_0092_028_two_attacks_and_a_normal_extraction_a`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: each call is bounded by US-0222; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Design tests (guard a decision, not in the spec)
- Eight parity tests, `parity_tf_without_name`, `parity_tf_missing_font`, `parity_cs_without_name`, `parity_cs_missing_space`, `parity_do_without_name`, `parity_do_missing_xobject`, `parity_do_not_a_stream`, `parity_do_unparsable_form` (nesting_e2e.rs, engine call inside `in_child`): each asserts the answer is `ERR_INVALID_ARGUMENT` with `could not extract /docs/parity.pdf: pdf parser failed on this document` and the child exits normally. Green on arrival; each killed by the mutation "replace that stop's `break` by `continue`" (the walk then reaches the self drawing form and answers the form depth message instead). Record the eight mutation runs.

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-011 | treat every `.pdf` companion as stale (`refresh` forced for PDFs in `prepare_extraction`): the refusal comes instead of the cached text, the test fails |
| E2E-016 | compare the `.pdf` suffix case sensitively in `Extractor::extract`: `LOOP.PDF` is decoded as text and succeeds, the test fails |
| E2E-018 | in `McpServer::fs_extract_text`, call the engine before `authorize`: eve gets the form depth refusal, the test fails |
| E2E-019 | skip the object limit check of the load: page 1 is walked and the form depth refusal comes, the test fails |
| E2E-020 | walk the pages in reverse order: page 2's loop is refused first, the test fails |
| E2E-023 | on a reader refusal in `core::fs_ops::extract_document`, write an empty companion: `nothing_left` fails, the test fails |
| E2E-024 | on refresh, delete the stored companion before extracting: `/docs/loop.md` is gone, the test fails |
| E2E-027 | add `tracing::info!` of the refusal inside `pdf_bound::extract`: two lines, the test fails |
| E2E-028 | restore the skip of a form already on the drawing stack: the reader recurses, the child ends by signal 6, the test fails |

## Constraints
- Files not to touch: `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/office_bound_e2e.rs` (SPEC-0091 E2E-017 asserts them unchanged), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Production files not to touch: `crates/core/src/docs/pdf_bound.rs` (a red parity test is reported, not patched), `crates/core/src/docs/symbols.rs`
- No dash as punctuation in code, comments or docs
- Scope boundary: the HTTP door and the closing docs are US-0227

## Definition of done
- [ ] Green on arrival tests (E2E-011, E2E-016, E2E-018, E2E-019, E2E-020, E2E-023, E2E-024, E2E-027, E2E-028): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0089_` and `e2e_0088_` test, the 19 tests of `docs::pdf_bound`, `pdf_extracts_text_pages_and_metadata`, `pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion`, every earlier `e2e_0092_` test, the 22 `docs::symbols` tests
- [ ] Docs updated: `.agent_docs/architecture.md` (DEC-014), entry dated.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0226: nesting refusals in order, without trace, walk and reader in step` on its story branch `story/US-0226-nesting-refusals-in-order-without-trace`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0092-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0092` and the requirement ids (SPEC-0092/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007).
