# US-0224: refuse character and code maps nested past 64

> Spec: SPEC-0092 (sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0223
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A font's character map, or a composite font's code map, nested a thousand levels ends the server: the reader's map parser goes one stack frame deeper per list, dictionary or bracketed string. After this story the walk scans every map it measures, the way the parser lexes it, and refuses one nested past 64 levels, after its stream limit and page total checks.

## Requirements covered
- SPEC-0092/FR-NEW-003: IF a character map or a composite font's code map that a page selects nests more than 64 levels THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: a PDF character map is nested more than 64 deep`.
- SPEC-0092/FR-NEW-004 (partial: item 4: a character map or code map checks the stream limit, then the page total, then its nesting; a draw before a later map): IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in the order of SPEC-0089 FR-NEW-007, item 4 read as: pages in page order; for each page, first FR-NEW-002 for the resources, then FR-NEW-002 for the page size, then each decode and draw in drawing order, where a draw checks FR-NEW-001 before its stream is measured and a character map or code map checks SPEC-0089 FR-NEW-002, then SPEC-0089 FR-NEW-004, then FR-NEW-003.
- SPEC-0092/FR-NEW-006 (partial: character and code maps): THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- SPEC-0092/FR-NEW-007 (partial: a map within the limit): THE document text extraction SHALL answer a PDF whose nestings are within FR-NEW-001 to FR-NEW-003 with the same text, page markers, page count, title, note, companion, preview, charge, audit line and refusals as before, and SHALL answer from an up to date companion without reading the PDF when no refresh is asked.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0092_sec_007 e2e_0092_sec_008 e2e_0092_sec_009 e2e_0092_sec_010 e2e_0092_007 e2e_0092_014 e2e_0092_022 map_nesting`: 10 tests pass (7 spec tests, 3 design tests). Real assertions: child exit status, refusal message, `nothing_left`, bystander read, preview `Hello` at 64 levels.

## Design excerpt
### Decisions
- **DEC-002 (SPEC-0092):** the limits are fixed `pub(crate) const` in `pdf_bound.rs`: `FORM_DEPTH_LIMIT` 64, `CLIMB_LIMIT` 256, `MAP_NESTING_LIMIT` 64; no configuration, no test seam. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, spec D-4 to D-7. **Rationale:** the messages carry the numbers literally; a scaled limit proves nothing about the shipped one (SPEC-0089 DEC-002).
- **DEC-005 (SPEC-0092):** for each font the walk loads (`"Tf"` arm), after `page.decode(id)` succeeds for a `ToUnicode` stream (any subtype the walk lists in `font_streams`) or a Type0 `Encoding` stream, the walk takes the bytes the reader will parse (`reader_bytes(stream, page.armed.contains(&id))`), records them in the held figure as it does for a form (`budget.hold(size)`), and refuses when `map_nesting(&bytes) > MAP_NESTING_LIMIT` (`a PDF character map is nested more than 64 deep`). `map_nesting` is one iterative pass mirroring the cmap parser's lexing (`adobe-cmap-parser-0.4.1/src/lib.rs:28-150`): outside a string, `[` and `<<` open a level, `]` and `>>` close one, `<` not followed by `<` starts a hexadecimal string skipped to `>`, `%` starts a comment skipped to the end of the line, `(` opens a string level; inside a string, `\` skips the next byte, `(` opens and `)` closes a level, every other byte (brackets included) is text. Levels count the three kinds together, unclosed levels included; the pass stops as soon as the depth passes 64. Order per map: stream limit, then page total (both in `page.decode`), then nesting (FR-NEW-004). FontFile streams are not scanned (not parsed by the cmap parser). **Obliged by:** SPEC-0092/FR-NEW-003, FR-NEW-004. **Rationale:** the parser's recursion follows these levels one for one; lexing like the parser avoids counting brackets inside strings and comments.
- **DEC-006 (SPEC-0092):** the three refusals are `ToolError::invalid_argument` with the reason only; `prepare_extraction` adds `could not extract <path>: ` as for every reader error, and the refusal leaves no trace by the existing path (it returns before `land_conversion`, logged once at INFO by `logging::log_tool_failure`). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-005. **Rationale:** SPEC-0088/0089 convention, no new code path.
- **DEC-012 (SPEC-0092):** fixtures follow the spec literally (section 9 header): catalog 1, page tree 2, page 3, content 4, font 5 `Helvetica`, forms from object 10, page groups from object 100, maps at object 9, uncompressed streams. Builders: `forms_chain_pdf(n_forms, bottom_text)`, `loop_pdf()`, `inherit_pdf()`, `pair_pdf()`, `repeat_pdf(times)`, `parent_chain_pdf(groups, with_resources_on_page, with_media_box_on_page)`, `parent_loop_pdf(page_resources, page_media_box, content)`, `cmap_pdf(nesting_bytes)`, `type0_pdf(nesting)`, `order_pdf()`, `two_pages_bomb_then_loop_pdf()`, `both_pdf()`, `deep_source(head, open, middle, close, n, tail)`. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-008 (the literal payloads of section 9). **Rationale:** the payload is the attacker's file.
### Data model
`MapScan { depth: u32, string: u32, escape: bool, hex: bool, comment: bool, max: u32 }`, local to `map_nesting`.
### Contracts
- `pub(crate) const MAP_NESTING_LIMIT: u32 = 64;`
- `pub(crate) fn map_nesting(bytes: &[u8]) -> u32`: the deepest level reached, returning as soon as it passes `MAP_NESTING_LIMIT`.
- In `walk`, `"Tf"` arm: for each `id` of `font_streams(doc, font)`, `page.decode(id, budget)?` as today; then, when `id` is the font's `ToUnicode` stream or a Type0 font's `Encoding` stream, `let bytes = reader_bytes(stream, page.armed.contains(&id)); budget.hold(size);` and `if map_nesting(&bytes) > MAP_NESTING_LIMIT { return Err(ToolError::invalid_argument(format!("a PDF character map is nested more than {MAP_NESTING_LIMIT} deep"))); }`. `font_streams` returns which of its ids are maps (a second list, or a tagged id), FontFile streams are never scanned.
- Member's answer: `ERR_INVALID_ARGUMENT`, `could not extract <path>: a PDF character map is nested more than 64 deep`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify (scan and its unit tests) |
| `crates/core/src/tools/nesting_testkit.rs` | modify (map builders) |
| `crates/core/src/tools/nesting_e2e.rs` | modify |

Read only (imported, never modified): `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/archive_testkit.rs`.
### Existing patterns
The `"Tf"` arm and `font_streams` (`crates/core/src/docs/pdf_bound.rs:1504-1517`, `:1573-1595`); the form branch's `reader_bytes` then `budget.hold(size)` (`:1553-1560`); the cmap parser's lexing (`adobe-cmap-parser-0.4.1/src/lib.rs:28-30` comments, `:45-66` escapes and nested strings, `:110-146` dictionaries, hexadecimal strings, arrays); SPEC-0089's `cmap_bomb_pdf` (`crates/core/src/tools/pdf_testkit.rs:709-726`) for a ToUnicode stream object.
### Fixtures
- `cmap_pdf(body: &str)`: font 5 `<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 9 0 R >>`, object 9 a plain stream holding `body`, page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`. E2E-007: body `/CIDInit /ProcSet findresource begin 12 dict begin begincmap /X ` + 64 `[` + 64 `]` + ` def endcmap end end`; E2E-014: 65 and 65; E2E-SEC-007: 10 000 and 10 000; E2E-SEC-008: `/CIDInit /ProcSet findresource begin begincmap /X ` + 100 000 `(` + 100 000 `)` + ` def endcmap end`; E2E-SEC-009: the same head + `<< /K ` 100 000 times + `>> ` 100 000 times + ` def endcmap end`.
- `type0_pdf(body: &str)` (E2E-SEC-010): font 5 `<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding 9 0 R /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X >>] >>`, object 9 body `/CIDInit /ProcSet findresource begin begincmap /X ` + 10 000 `[` + 10 000 `]` + ` def endcmap end`.
- `order_pdf()` (E2E-022): page content `/A Do BT /F2 12 Tf (x) Tj ET`, page resources `/XObject << /A 10 0 R >> /Font << /F1 5 0 R /F2 6 0 R >>`, object 10 the self drawing form of `loop_pdf`, font 6 a Type1 font with `/ToUnicode 9 0 R`, object 9 nesting 1 000 `[`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0092/E2E-SEC-007: Character map of 10 000 nested lists
- **Covers:** occurrence 3, lists
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 14; 1 000 already ends it).
- **Given** `/docs/cmapdeep.pdf` as E2E-007 with 10 000 `[` and 10 000 `]`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/cmapdeep.pdf: a PDF character map is nested more than 64 deep`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_007_character_map_of_10_000_nested_li`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 14)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-008: Character map with 100 000 nested bracketed strings
- **Covers:** occurrence 3, strings
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 15).
- **Given** `/docs/cmapparen.pdf` as E2E-007 but object 9 is `/CIDInit /ProcSet findresource begin begincmap /X ` then 100 000 `(`, 100 000 `)`, then ` def endcmap end`
- **When** the member asks for its text
- **Then** the refusal is the character map message for `/docs/cmapparen.pdf`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_008_character_map_with_100_000_nested`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 15)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-009: Character map with 100 000 nested dictionaries
- **Covers:** occurrence 3, dictionaries
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 16).
- **Given** `/docs/cmapdict.pdf` as E2E-007 but the nesting is `<< /K ` written 100 000 times then `>> ` written 100 000 times
- **When** the member asks for its text
- **Then** the refusal is the character map message for `/docs/cmapdict.pdf`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_009_character_map_with_100_000_nested`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 16)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-010: Composite font code map of 10 000 nested lists
- **Covers:** occurrence 4
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 17).
- **Given** `/docs/type0.pdf` whose font 5 is `<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding 9 0 R /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X >>] >>`, object 9 a stream `/CIDInit /ProcSet findresource begin begincmap /X ` then 10 000 `[`, 10 000 `]`, then ` def endcmap end`, page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`
- **When** the member asks for its text
- **Then** the refusal is the character map message for `/docs/type0.pdf`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_010_composite_font_code_map_of_10_000`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 17)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-007: Character map nested exactly 64 deep
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/cmap64.pdf` whose page content is `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`, font 5 `<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 9 0 R >>`, object 9 a stream `/CIDInit /ProcSet findresource begin 12 dict begin begincmap /X ` then 64 `[`, 64 `]`, then ` def endcmap end end`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Hello`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_007_character_map_nested_exactly_64_deep`
- Green on arrival: 64 levels are read today (65 fine); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-014: Character map nested 65 deep
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-005; **Priority:** Critical
- **Given** `/docs/cmap65.pdf` as E2E-007 with 65 `[` and 65 `]`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/cmap65.pdf: a PDF character map is nested more than 64 deep`
- **And** nothing left; the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_014_character_map_nested_65_deep`
- Red before the change because: on US-0223 code 65 levels are read and the extraction succeeds
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-022: Form depth before a later character map on the same page
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-004; **Priority:** Medium
- **Given** `/docs/order.pdf` whose page content is `/A Do BT /F2 12 Tf (x) Tj ET`, `/A` the self drawing form of E2E-SEC-001, `/F2` in the page resources a font whose character map nests 1 000 `[`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/order.pdf: page 1 draws PDF forms nested more than 64 deep`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs inside `in_child`
- Function: `e2e_0092_022_form_depth_before_a_later_character_m`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Green on arrival: on US-0223 code the form depth refusal already comes first (US-0222); this test pins that order now that maps are scanned; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Design tests (guard a decision, not in the spec)
- `map_nesting_counts_lists_dictionaries_and_strings` (unit, `pdf_bound.rs` tests): `map_nesting(b"[[<< /K (()) >>]]")` is 5; 64 `[` gives 64; 65 `[` gives 65; unclosed `(((` gives 3.
- `map_nesting_skips_escapes_hex_strings_and_comments` (unit): `(\( \( [)` is 1 (escaped parentheses and a bracket inside a string are text); `<0A0B> [ ]` is 1 (a hexadecimal string is no level); `% [[[[\n[ ]` is 1 (a comment to the end of the line); `<< >>` is 1.
- `map_nesting_is_checked_after_the_stream_limit` (nesting_e2e.rs, engine call, `heavy()` first): a ToUnicode stream that is `pdf_testkit::zlib_run` of 100 `[` then `PDF_BOMB` spaces (via `flate`), stored as `/docs/mapbomb.pdf`; the refusal is the stream limit message `could not extract /docs/mapbomb.pdf: a PDF stream decodes to more than 104857600 bytes`, not the nesting message. Kills the mutation "scan the map while measuring it".

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-007 | refuse when `map_nesting(..) >= MAP_NESTING_LIMIT`: the test fails |
| E2E-022 | scan the maps of every font in the page resources before walking the page content: the 1 000 level map of `/F2` is refused first, the test fails |

## Constraints
- Files not to touch: `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/office_bound_e2e.rs` (SPEC-0091 E2E-017 asserts them unchanged), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- The scan is one iterative pass over the bytes; never call the cmap parser to judge nesting
- Never scan a FontFile stream; never scan before the stream limit and page total checks
- No dash as punctuation in code, comments or docs
- Scope boundary: code search US-0225; order against the load and other pages US-0226

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-007, E2E-SEC-008, E2E-SEC-009, E2E-SEC-010, E2E-014 (tests and fixtures first, on the previous story code, each failing for the reason stated).
- [ ] Green on arrival tests (E2E-007, E2E-022): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0089_` and `e2e_0088_` test, the 19 tests of `docs::pdf_bound`, `pdf_extracts_text_pages_and_metadata`, `pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion`, every earlier `e2e_0092_` test (SPEC-0089 E2E-SEC-004, the character map bomb, still refused by the stream limit)
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0224: refuse character and code maps nested past 64` on its story branch `story/US-0224-refuse-character-and-code-maps-nested-pa`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0092-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0092` and the requirement ids (SPEC-0092/FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-007).
