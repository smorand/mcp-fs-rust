# US-0223: refuse a page tree climb past 256 levels

> Spec: SPEC-0092 (sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0222
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A page that takes its resources or its page size from page groups above it makes the reader climb its parent chain one stack frame per level: a chain that loops, or one of 10 000 groups, ends the server. After this story the walk climbs the chain itself, as the reader does, for the resources then the page size, before the page content is decoded, and refuses a climb past 256 levels; a loop simply passes 256.

## Requirements covered
- SPEC-0092/FR-NEW-002: IF finding a page's resources, or finding its page size, climbs more than 256 page groups above the page THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> inherits from more than 256 levels of the PDF page tree`.
- SPEC-0092/FR-NEW-004 (partial: item 4: for each page, the climb for the resources then for the page size before the page decodes and draws): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in the order of SPEC-0089 FR-NEW-007, item 4 read as: pages in page order; for each page, first FR-NEW-002 for the resources, then FR-NEW-002 for the page size, then each decode and draw in drawing order, where a draw checks FR-NEW-001 before its stream is measured and a character map or code map checks SPEC-0089 FR-NEW-002, then SPEC-0089 FR-NEW-004, then FR-NEW-003.
- SPEC-0092/FR-NEW-006 (partial: the page tree): THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- SPEC-0092/FR-NEW-007 (partial: a climb within the limit, a looping chain above a self sufficient page): THE document text extraction SHALL answer a PDF whose nestings are within FR-NEW-001 to FR-NEW-003 with the same text, page markers, page count, title, note, companion, preview, charge, audit line and refusals as before, and SHALL answer from an up to date companion without reading the PDF when no refresh is asked.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0092_sec_005 e2e_0092_sec_006 e2e_0092_006 e2e_0092_009 e2e_0092_013 e2e_0092_017 e2e_0092_021`: 7 tests pass. Real assertions: child exit status, refusal message with page 1, `nothing_left`, the bystander read, the preview `Hello` of the documents within the limit.

## Design excerpt
### Decisions
- **DEC-002 (SPEC-0092):** the limits are fixed `pub(crate) const` in `pdf_bound.rs`: `FORM_DEPTH_LIMIT` 64, `CLIMB_LIMIT` 256, `MAP_NESTING_LIMIT` 64; no configuration, no test seam. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, spec D-4 to D-7. **Rationale:** the messages carry the numbers literally; a scaled limit proves nothing about the shipped one (SPEC-0089 DEC-002).
- **DEC-004 (SPEC-0092):** `climb` mirrors `get_inherited` (`lib.rs:2352-2362`): at level 0 the page, then each `Parent` read as a reference and resolved with `doc.get_dictionary`; a node's value is usable when its key, resolved one step as `maybe_deref` does, is a dictionary (`Resources`) or an array (`MediaBox`, any array: `FromObj for Vec<T>` accepts any array, `lib.rs:211-217`); another kind is passed over and the climb goes on. A usable value at level k <= 256 is the answer; reaching level 257 refuses (`page <N> inherits from more than 256 levels of the PDF page tree`); a missing or non reference `Parent`, or an unresolvable one, ends the climb with no value (today's answer: empty resources, or the reader's `expect("MediaBox")` panic, contained, `pdf parser failed on this document`). No `seen` set: a loop simply climbs past 256. The walk climbs `Resources` then `MediaBox` before decoding the page content (FR-NEW-004 order; the MediaBox value itself is not used by the walk). **Obliged by:** SPEC-0092/FR-NEW-002, FR-NEW-004, FR-NEW-007. **Rationale:** the reader's climb takes the first value it accepts; refusing at the same level the reader would reach keeps parity.
- **DEC-006 (SPEC-0092):** the three refusals are `ToolError::invalid_argument` with the reason only; `prepare_extraction` adds `could not extract <path>: ` as for every reader error, and the refusal leaves no trace by the existing path (it returns before `land_conversion`, logged once at INFO by `logging::log_tool_failure`). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-005. **Rationale:** SPEC-0088/0089 convention, no new code path.
- **DEC-009 (SPEC-0092):** a test runs in a child exactly when its document or source aborts the process on today's code: E2E-015, 016, 017, 021, 022, 023, 024, 027, 028 and every E2E-SEC test; the others run in process (their documents are within the measured thresholds, so RED shows as a plain assertion). **Obliged by:** SPEC-0092/FR-NEW-006 (spec D-14). **Rationale:** RED evidence must be an assertion, not a crashed runner; child runs cost about one second each.
- **DEC-012 (SPEC-0092):** fixtures follow the spec literally (section 9 header): catalog 1, page tree 2, page 3, content 4, font 5 `Helvetica`, forms from object 10, page groups from object 100, maps at object 9, uncompressed streams. Builders: `forms_chain_pdf(n_forms, bottom_text)`, `loop_pdf()`, `inherit_pdf()`, `pair_pdf()`, `repeat_pdf(times)`, `parent_chain_pdf(groups, with_resources_on_page, with_media_box_on_page)`, `parent_loop_pdf(page_resources, page_media_box, content)`, `cmap_pdf(nesting_bytes)`, `type0_pdf(nesting)`, `order_pdf()`, `two_pages_bomb_then_loop_pdf()`, `both_pdf()`, `deep_source(head, open, middle, close, n, tail)`. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-008 (the literal payloads of section 9). **Rationale:** the payload is the attacker's file.
### Contracts
- `pub(crate) const CLIMB_LIMIT: usize = 256;`
- `fn climb<'d>(doc: &'d Document, page: &'d Dictionary, key: &[u8], usable: fn(&Object) -> bool, page_no: u32) -> Result<Option<&'d Object>>`: level 0 is the page; at each level, `node.get(key)` resolved one step (`resolve`, as `maybe_deref`) and accepted when `usable`; otherwise `node.get(b"Parent").and_then(Object::as_reference)` then `doc.get_dictionary(id)`, any failure ending the climb with `Ok(None)`; reaching level 257 returns `Err(ToolError::invalid_argument(format!("page {page_no} inherits from more than {CLIMB_LIMIT} levels of the PDF page tree")))`. No `seen` set.
- `usable` for `Resources`: `Object::as_dict().is_ok()`; for `MediaBox`: `Object::as_array().is_ok()`.
- `walk` starts with `climb(doc, page_dict, b"Resources", ..)` (the result, or the empty dictionary, replaces `inherited_resources`, which is deleted) then `climb(doc, page_dict, b"MediaBox", ..)` (result unused), then decodes the page content as today.
- Member's answer: `ERR_INVALID_ARGUMENT`, `could not extract <path>: page <N> inherits from more than 256 levels of the PDF page tree`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify |
| `crates/core/src/tools/nesting_testkit.rs` | modify (climb builders) |
| `crates/core/src/tools/nesting_e2e.rs` | modify |

Read only (imported, never modified): `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/archive_testkit.rs`.
### Existing patterns
`inherited_resources` (`crates/core/src/docs/pdf_bound.rs:1349-1362`) is the climb being replaced; the reader's `get_inherited` (`pdf-extract-0.12.1/src/lib.rs:2352-2362`, called at `:2410` and `:2413`) is the behaviour mirrored; US-0222's `in_child`, `nesting_fixture`, `still_up`.
### Fixtures
Page 3 replaced (pdf_testkit `pages_writer` then `w.object(3, ..)`), page tree 2 still lists it in `Kids`.
- `parent_chain_pdf(groups: u32)` (E2E-006 with 256, E2E-013 with 257, E2E-SEC-006 with 10 000): page 3 `<< /Type /Page /Parent 100 0 R /Contents 4 0 R >>`, content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`; objects 100 to 100 + groups - 2 each `<< /Type /Pages /Parent n+1 0 R >>`; the last, object 100 + groups - 1, `<< /Type /Pages /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] >>`.
- `parent_loop_pdf(resources: bool, media_box: bool, content: &str)`: page 3 `<< /Type /Page /Parent 100 0 R /Contents 4 0 R` plus `/Resources << /Font << /F1 5 0 R >> >>` when `resources`, `/MediaBox [0 0 612 792]` when `media_box`; object 100 `<< /Type /Pages /Parent 100 0 R >>`. E2E-009: `(true, true, "BT /F1 12 Tf 72 700 Td (Hello) Tj ET")`; E2E-SEC-005: `(false, true, same text)`; E2E-017: `(true, false, same text)`; E2E-021: `(false, false, "/A Do")`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0092/E2E-SEC-005: A page whose parent is itself a loop
- **Covers:** occurrence 2, loop
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 7).
- **Given** `/docs/parentloop.pdf` whose page 3 is `<< /Type /Page /Parent 100 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>` (no resources), object 100 `<< /Type /Pages /Parent 100 0 R >>`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/parentloop.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_005_a_page_whose_parent_is_itself_a_l`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 7)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-006: A parent chain of 10 000 page groups
- **Covers:** occurrence 2, depth without a loop
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 8; a chain of 3 000 is extracted today).
- **Given** `/docs/parents.pdf` as E2E-006 with objects 100 to 10 099, object 10 099 holding the resources and page size
- **When** the member asks for its text
- **Then** the refusal is the page tree message for page 1 of `/docs/parents.pdf`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_006_a_parent_chain_of_10_000_page_gro`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 8)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-006: Resources and page size 256 page groups up
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/tall.pdf` whose page 3 is `<< /Type /Page /Parent 100 0 R /Contents 4 0 R >>` (no resources, no page size), page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`, objects 100 to 355 each `<< /Type /Pages /Parent n+1 0 R >>` except object 355, `<< /Type /Pages /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] >>` (256 page groups)
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Hello`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_006_resources_and_page_size_256_page_grou`
- Green on arrival: a chain of 256 extracts today (3 000 fine); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-009: Looping parent chain above a self sufficient page
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-007; **Priority:** High
- **Given** `/docs/selfloop.pdf` whose page 3 holds its own resources and page size and `/Parent 100 0 R`, object 100 being `<< /Type /Pages /Parent 100 0 R >>`, page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Hello`, as today
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_009_looping_parent_chain_above_a_self_suf`
- Green on arrival: the page holds its own values, so the reader never climbs; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-013: Resources 257 page groups up
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** Critical
- **Given** `/docs/taller.pdf` as E2E-006 with objects 100 to 356, object 356 holding the resources and page size (257 page groups)
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/taller.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- **And** nothing left; the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_013_resources_257_page_groups_up`
- Red before the change because: on US-0222 code 257 page groups extract
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-017: Page size missing along a looping chain
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** High
- **Given** `/docs/nosize.pdf` whose page 3 holds its resources but no page size, and `/Parent 100 0 R`, object 100 being `<< /Type /Pages /Parent 100 0 R >>`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/nosize.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- **And** the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs inside `in_child`
- Function: `e2e_0092_017_page_size_missing_along_a_looping_cha`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6: the reader climbs the loop for the page size forever
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-021: Page tree climb before the page's draws
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** Medium
- **Given** `/docs/climbloop.pdf` whose page 3 has no resources and no page size, `/Parent 100 0 R`, object 100 `<< /Type /Pages /Parent 100 0 R >>`, and whose page content is `/A Do`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/climbloop.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs inside `in_child`
- Function: `e2e_0092_021_page_tree_climb_before_the_page_s_dra`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 on the resources climb
- Note: also pins FR-NEW-004: the climb refusal is reported though the page content then draws `/A`.
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-006 | refuse at level 256 (`>=` for `>`): the test fails |
| E2E-009 | start the climb at the parent (level 1 is the first node read): the loop is climbed and refused, the test fails |

## Constraints
- Files not to touch: `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/office_bound_e2e.rs` (SPEC-0091 E2E-017 asserts them unchanged), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- The climb is a loop with a counter, never recursion and never a `seen` set (a loop must count as passing 256, spec D-8)
- No dash as punctuation in code, comments or docs
- Scope boundary: character maps US-0224; the order tests against the load and the stream limit are US-0226

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-005, E2E-SEC-006, E2E-013, E2E-017, E2E-021 (tests and fixtures first, on the previous story code, each failing for the reason stated).
- [ ] Green on arrival tests (E2E-006, E2E-009): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0089_` and `e2e_0088_` test, the 19 tests of `docs::pdf_bound`, `pdf_extracts_text_pages_and_metadata`, `pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion`, every earlier `e2e_0092_` test
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0223: refuse a page tree climb past 256 levels` on its story branch `story/US-0223-refuse-a-page-tree-climb-past-256-levels`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0092-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0092` and the requirement ids (SPEC-0092/FR-NEW-002, FR-NEW-004, FR-NEW-006, FR-NEW-007).
