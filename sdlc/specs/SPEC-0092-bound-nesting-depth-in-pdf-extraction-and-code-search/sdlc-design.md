# SPEC-0092: Bound nesting depth in PDF extraction and code search, design

> Spec: sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: internal finding
> Designed on: 2026-11-11
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| PDF text pass | the SPEC-0089 walk in `crates/core/src/docs/pdf_bound.rs` (`walk` `:1455-1570`, `inherited_resources` `:1349-1362`, `font_streams` `:1573-1595`), then the unmodified reader `pdf_extract::output_doc_page` | `pdf-extract = "0.12"` locked 0.12.1, lopdf 0.42.0, adobe-cmap-parser 0.4.1 (`Cargo.lock`) | DEC-001 |
| Code search scan | `tree_sitter::Node::named_children` with an explicit stack, in `crates/core/src/docs/symbols.rs` | `tree-sitter = "0.26"` (`Cargo.toml:148`) | DEC-007 |
| Child process harness | `std::process::Command` on `std::env::current_exe()`, the pattern of `crates/core/src/storage/blob/s3.rs:282-310` (`run_child`); `tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_stack_size(2 MiB)` | std, tokio 1.54.1 (`Cargo.lock:5973`) | DEC-008 |
| Test fixtures | hand written PDF bytes through `pdf_testkit::PdfWriter`, `plain`, `pages_writer`, `bomb`, `object_stream_nulls` (read only) | in tree (SPEC-0089) | DEC-011 |

Dependency statement: no new crate, no feature change, no `unsafe`, no vendored or patched library.

Skills loaded: `rust` (edition 2024, no `unwrap` or `expect` on fallible production paths, typed `ToolError`,
clippy `--all-targets --all-features -D warnings`, `cargo fmt --check`, functional e2e through the real entry
points, no `unsafe`).

## 2. Approaches

The key fact (spec 14.1 rows 3 to 5, 14.3): the three PDF recursions live in third party code with no depth
option (`pdf-extract lib.rs:1860-1868` `Do` calls `process_stream`, `:2352-2362` `get_inherited`,
`adobe-cmap-parser lib.rs:61-66`, `:112-146`). A stack overflow aborts the process; `catch_unwind`
(`crates/core/src/docs/extract.rs:519`) cannot contain it.

### Chosen: refuse in the SPEC-0089 walk, before the reader runs

The walk already runs every page before `output_doc_page`, iteratively, in drawing order, and already measures
every form, character map and code map the reader will decode (SPEC-0089 DEC-007). It gains three checks: a
depth counter on its explicit drawing stack (the self draw skip at `pdf_bound.rs:1550-1552` goes), a bounded
climb mirroring `get_inherited` for the resources and the page size, and an iterative nesting scan of every
character map and code map once measured. The reader then only meets what the walk accepted, and every
accepted nesting sits far under the measured abort thresholds (64 forms against 200 fine, 256 page groups
against 3 000 fine, 64 map levels against 65 fine and 1 000 abort; spec 14.3). Code search: the two recursive
visits of `symbols.rs` become one explicit stack each, same pre order.
Pros: safe Rust, no dependency change, one place for the order of refusals, the walk's measured parity with the
reader is already tested by SPEC-0089. Cons: parity is now load bearing: where the walk stops (`break`), the
reader must stop too (DEC-013 tests each stop). Scope: `pdf_bound.rs`, `symbols.rs`.

### Rejected: patch, vendor or replace the reader (DEC-001)

A vendored `pdf-extract` with a depth argument, or another PDF text library. Violates spec 3.3 (no new third
party component without necessity), and a vendored copy diverges from upstream fixes.

### Rejected: a larger stack (DEC-001)

A bigger runtime worker or blocking thread stack only moves the 400 forms and 10 000 page groups thresholds;
a loop never ends (spec 14.5 row 1: 64 MiB still aborts).

### Rejected: one child process per extraction (DEC-001)

Containment by process isolation: a new architecture (process pool, transport of bytes and figures), costly
per call, and outside this spec's scope (SPEC-0091 owns where extraction runs).

### Rejected for code search: a depth cap (DEC-007)

A cap would refuse or drop definitions, which FR-NEW-008 and spec D-10 forbid.

## 3. Architecture

```
fs.extract_text (mcp/server.rs:2402)        POST /api/fs/{mount}/extract-text (api/dataplane.rs:171)
        \  membership, path normalized                 /
         +---- core::fs_ops::extract_document --------+
                 prepare_extraction (docs/extract.rs:330) -> Extractor::extract -> ".pdf" -> pdf(data)
                   catch_unwind(pdf_bound::extract(data))           (unchanged)
                     load (SPEC-0089)
                     for page n in order:
                       walk(page n):
                         climb Resources  (<= 256 levels, else refuse)          [US-0223]
                         climb MediaBox   (<= 256 levels, else refuse)          [US-0223]
                         decode page content (SPEC-0089 measure)
                         drawing stack, iterative:
                           Tf: font_streams -> decode (stream limit, page total)
                               -> ToUnicode / Type0 Encoding: nesting scan <= 64 [US-0224]
                           Do: lookup -> depth = stack.len(); > 64 refuse       [US-0222]
                               -> decode -> parse -> push (no self skip)
                       output_doc_page (reader meets only what the walk accepted)
                 refusal -> "could not extract <path>: <reason>"   (extract.rs:378-387, unchanged)

fs.find_definition / fs.find_references (mcp/server.rs:2034, :2051)   GET find-definition / find-references
         \------- core::fs_ops::find_definitions / find_references (fs_ops.rs:1666-1704) -------/
                    docs::symbols::find_definitions / find_references
                      walk (explicit stack, pre order)                         [US-0225]
                      name_of -> walk_descendants (explicit stack, first identifier) [US-0225]
```

Bounded contexts: "document text extraction" owns the three refusals in `pdf_bound.rs`; "code search" owns
`symbols.rs`; "project storage" is untouched (a refusal returns before `land_conversion`, as in SPEC-0089).

## 4. Data model

Nothing persisted. In memory, per walk: the existing `stack: Vec<Drawing>` (its length is the form depth plus
one); per climb: a level counter (no `seen` set any more); per character map: a scanner state `{ depth: u32,
in_string: u32, escape: bool, in_hex: bool, in_comment: bool, max: u32 }`. Code search: `Vec<Node>` stacks.

## 5. Contracts and interfaces

- `pub(crate) const FORM_DEPTH_LIMIT: usize = 64`, `CLIMB_LIMIT: usize = 256`, `MAP_NESTING_LIMIT: u32 = 64` in
  `pdf_bound.rs` (DEC-002).
- Refusals (the `could not extract <path>: ` prefix added by `prepare_extraction`, `extract.rs:378-387`,
  unchanged), all `ToolError::invalid_argument`:
  - `page <N> draws PDF forms nested more than 64 deep` (FR-NEW-001, DEC-003)
  - `page <N> inherits from more than 256 levels of the PDF page tree` (FR-NEW-002, DEC-004)
  - `a PDF character map is nested more than 64 deep` (FR-NEW-003, DEC-005)
- `fn climb<'d>(doc: &'d Document, page: &'d Dictionary, key: &[u8], usable: fn(&Object) -> bool, page_no: u32)
  -> Result<Option<&'d Object>>` replaces `inherited_resources` (DEC-004).
- `pub(crate) fn map_nesting(bytes: &[u8]) -> u32` (the deepest level, scanning stops once it passes
  `MAP_NESTING_LIMIT`), DEC-005.
- `symbols.rs`: `walk` and `walk_descendants` keep their signatures and visit order (DEC-007).
- Test only (`#[cfg(test)]`, `crates/core/src/tools/nesting_testkit.rs`):
  `pub(crate) fn in_child<F, Fut>(module: &str, name: &str, body: F) where F: FnOnce() -> Fut, Fut: Future<Output = ()>`
  (DEC-008); `pub(crate) async fn nesting_fixture() -> Fixture` (`atlas` with `mia@example.com` and an empty
  `/docs`, `orbit` with `leo@example.com` and `/notes/hello.txt` reading `hello`, DEC-010);
  `pub(crate) async fn still_up(f: &Fixture, pid_before: u32)`; `pub(crate) async fn bystander_loop(f: &Fixture,
  stop: tokio::sync::watch::Receiver<bool>) -> usize`; the PDF builders of section 9.

No tool contract change (DEC-014): no description mentions a nesting limit.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify: depth check on `Do`, self skip removed; `climb` for Resources and MediaBox; `map_nesting` after a map is measured | FR-NEW-001 to FR-NEW-004 | US-0222, US-0223, US-0224 |
| `crates/core/src/docs/symbols.rs` | modify: `walk`, `walk_descendants` iterative | FR-NEW-006, FR-NEW-008 | US-0225 |
| `crates/core/src/tools/nesting_testkit.rs` | create: `in_child`, `nesting_fixture`, `still_up`, `bystander_loop`, PDF and source builders | test harness (DEC-008, DEC-010, DEC-011) | US-0222, US-0223, US-0224, US-0226 |
| `crates/core/src/tools/nesting_e2e.rs` | create: the `e2e_0092_*` engine and child tests | every requirement | US-0222 to US-0226 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] pub(crate) mod nesting_testkit; #[cfg(test)] mod nesting_e2e;` | test layout | US-0222 |
| `crates/core/src/api/dataplane.rs` | modify (tests only): `e2e_0092_002`, `004`, `015`, `026`, `sec_013` | FR-NEW-009 | US-0225, US-0227 |
| `.agent_docs/architecture.md` | modify: the walk's three nesting refusals, the code search scan | DEC-014 | US-0226 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-014) | US-0227 |

`crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`,
`crates/core/src/tools/archive_testkit.rs` and `crates/core/src/tools/office_bound_e2e.rs` are read, never
modified (DEC-011): SPEC-0091 E2E-017 asserts that the SPEC-0088 and SPEC-0089 suites run unmodified.

Affected existing tests, kept unchanged and green: the 19 tests of `docs/pdf_bound.rs`, the 50 of
`tools/pdf_bound_e2e.rs` (forms drawn by the page, `forms_pdf` `pdf_testkit.rs:830`, `big_forms_pdf` `:815`,
`form_total_pdf` `:800`), the 22 of `docs/symbols.rs`, the 13 of `tools/search.rs`, the find tests of
`api/dataplane.rs:2151`. Any other existing test that changes is a story failure: stop and report it.

Affected documentation: `README.md` (document extraction paragraph), `AGENTS.md` (`pdf_bound.rs` limits line,
new nesting limits; `nesting_e2e` filter in key commands), `.agent_docs/tools.md` (`fs.extract_text` nesting
refusals; `fs.find_definition`, `fs.find_references` any nesting), `.agent_docs/testing.md` (`e2e_0092_`
filter, the child harness, `MCPFS_CHILD_CASE`), `.agent_docs/architecture.md`.

## 7. Compatibility and risks

- Declared changes (spec): a PDF past a nesting limit is refused; a deeply nested source is answered.
- Accepted by the spec: a chain of 3 000 page groups, extracted today, is refused (D-5); a character map of 65
  levels, read today, is refused (D-7).
- Undeclared, documented: a draw of an image XObject also counts one level of depth (DEC-003), since the reader
  runs every drawn XObject as content; an image at depth 65 is refused although it is not a form. No natural
  document draws 64 levels deep.
- Parity risk: a walk stop where the reader would not panic would let the reader reach an uncounted nesting.
  The eight stops are tested in a child (DEC-013).
- Ordering between stories (they land on main one by one, repo practice): US-0222 removes the self draw skip
  in the same commit that adds the depth refusal, so no intermediate main lets the walk recurse; every later
  story adds a refusal without loosening an earlier one.
- Rollback: revert the story commits; no data or contract change.

## 8. Security design

- Threat surface: a stored PDF (forms and their resources, the page tree's `Parent` links, every `ToUnicode`
  and Type0 `Encoding` stream) and a stored source file are untrusted member bytes. Sinks: the reader's
  recursive `process_stream`, `get_inherited`, the cmap parser's `call(value)` and `nested_literal_string`, and
  `symbols.rs`'s `walk` and `walk_descendants`, each one stack frame per level on a 2 MiB thread.
- Occurrences of the class sweep (spec 14.2), each fixed in this lot:

| # | Fix site | Mechanism | DEC | Exploit tests | Story |
|---|---|---|---|---|---|
| 1 | `pdf_bound.rs` `walk`, `"Do"` arm (`:1542-1565`; self skip `:1550-1552` removed) | depth = drawing stack length, refused at 65 before the form is measured | DEC-003 | E2E-SEC-001 to E2E-SEC-004 | US-0222 |
| 2 | `pdf_bound.rs` `inherited_resources` (`:1349-1362`) replaced by `climb`, called for `Resources` then `MediaBox` before the page content is decoded | bounded climb, a loop passes 256 | DEC-004 | E2E-SEC-005, E2E-SEC-006 | US-0223 |
| 3 | `pdf_bound.rs` `walk`, `"Tf"` arm (`:1504-1517`), `ToUnicode` stream | `map_nesting` on the reader's bytes once measured | DEC-005 | E2E-SEC-007 to E2E-SEC-009 | US-0224 |
| 4 | same arm, Type0 `Encoding` stream (`font_streams` `:1576`) | same scan | DEC-005 | E2E-SEC-010 | US-0224 |
| 5 | `symbols.rs` `walk` (`:207-213`) | explicit stack, same pre order | DEC-007 | E2E-SEC-011, E2E-SEC-013 | US-0225 |
| 6 | `symbols.rs` `walk_descendants` (`:240-247`) via `name_of` (`:225`) | explicit stack, first identifier, stop at the first | DEC-007 | E2E-SEC-012 | US-0225 |

- Exploit channel: each exploit test runs its attacked call in a child process of the test binary (DEC-008), so
  the pre fix abort is observed as a failed assertion (`child ended by signal 6`), never as a test runner
  crash. Inside the child, the absence of effect is read through other channels: `archive_testkit::nothing_left`
  (the `/docs` listing, the artifact tables, the audit trail, a 52 428 800 byte write that must succeed), the
  bystander `leo@example.com` reading `/notes/hello.txt` of `orbit` through `McpServer::fs_read` within 1 s, and
  the process id equal before and after the call (DEC-010). Each payload is written literally as the spec's
  Given states it.
- Red before the fix: every exploit test is red because the child aborts (spec 14.5 rows 1 to 21).
- Secrets: none touched; the refusal names the page number only, never document content.

## 9. Test strategy

Functional e2e through the typed engine call (`archive_testkit::extract_text`, which authorizes then calls
`core::fs_ops::extract_document`; `core::fs_ops::find_definitions` and `find_references` after
`f.state.authorize`), membership tests through the real MCP methods (`McpServer::fs_extract_text`,
`McpServer::fs_find_definition`), the HTTP door through the `Harness` of `api/dataplane.rs` tests (project
`proj`, member `owner@test.com` standing for `atlas` and `mia@example.com`, as SPEC-0089). Names
`e2e_0092_<id>_<title in snake case>`, cut at 50 characters, trailing underscore dropped; filter `e2e_0092_`.
"Child" means the test body runs through `in_child` (DEC-009).

| Test | Level | Driver | Location | Child | Story |
|---|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0222 |
| E2E-002 | functional e2e | HTTP door | api/dataplane.rs tests | no | US-0227 |
| E2E-003 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0225 |
| E2E-004 | functional e2e | HTTP door and `McpServer::fs_find_references` | api/dataplane.rs tests | no | US-0225 |
| E2E-005 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0222 |
| E2E-006 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0223 |
| E2E-007 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0224 |
| E2E-008 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0222 |
| E2E-009 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0223 |
| E2E-010 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0225 |
| E2E-011 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0226 |
| E2E-012 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0222 |
| E2E-013 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0223 |
| E2E-014 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0224 |
| E2E-015 | functional e2e | HTTP door | api/dataplane.rs tests | yes | US-0227 |
| E2E-016 | functional e2e | typed engine call | nesting_e2e.rs | yes | US-0226 |
| E2E-017 | functional e2e | typed engine call | nesting_e2e.rs | yes | US-0223 |
| E2E-018 | functional e2e | `McpServer::fs_extract_text` as eve | nesting_e2e.rs | no | US-0226 |
| E2E-019 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0226 |
| E2E-020 | functional e2e | typed engine call | nesting_e2e.rs | no | US-0226 |
| E2E-021 | functional e2e | typed engine call | nesting_e2e.rs | yes | US-0223 |
| E2E-022 | functional e2e | typed engine call | nesting_e2e.rs | yes | US-0224 |
| E2E-023 | functional e2e | typed engine call | nesting_e2e.rs | yes | US-0226 |
| E2E-024 | functional e2e | typed engine call | nesting_e2e.rs | yes | US-0226 |
| E2E-025 | functional e2e | `McpServer::fs_find_definition` as eve | nesting_e2e.rs | no | US-0225 |
| E2E-026 | functional e2e | HTTP door and `McpServer::fs_find_references` | api/dataplane.rs tests | no | US-0225 |
| E2E-027 | functional e2e | `McpServer::fs_extract_text` under the log capture | nesting_e2e.rs | yes | US-0226 |
| E2E-028 | functional e2e | typed engine call, three calls joined | nesting_e2e.rs | yes | US-0226 |
| E2E-SEC-001 to E2E-SEC-004 | exploit | typed engine call | nesting_e2e.rs | yes | US-0222 |
| E2E-SEC-005, E2E-SEC-006 | exploit | typed engine call | nesting_e2e.rs | yes | US-0223 |
| E2E-SEC-007 to E2E-SEC-010 | exploit | typed engine call | nesting_e2e.rs | yes | US-0224 |
| E2E-SEC-011, E2E-SEC-012 | exploit | typed engine call | nesting_e2e.rs | yes | US-0225 |
| E2E-SEC-013 | exploit | HTTP door | api/dataplane.rs tests | yes | US-0225 |

Design tests (not spec tests, they guard a decision): `form_depth_is_checked_before_the_form_is_measured`
(DEC-003, US-0222), `map_nesting_counts_lists_dictionaries_and_strings` and
`map_nesting_skips_escapes_hex_strings_and_comments` (unit, `pdf_bound.rs`, DEC-005, US-0224),
`map_nesting_is_checked_after_the_stream_limit` (DEC-005, US-0224), the eight `parity_*` child tests (DEC-013,
US-0226).

Test data: generated at run time by `nesting_testkit.rs` builders; no committed fixture.

## 10. Decisions log

- **DEC-001:** the three PDF nestings are refused by the SPEC-0089 walk before the reader runs (section 2,
  chosen approach). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-004, FR-NEW-006, spec 3.3, 14.3.
  **Rationale:** the walk already reproduces the reader's draws and font loads in drawing order, iteratively;
  three checks there bound every recursion the reader performs; safe Rust, no dependency change. **Alternatives
  considered:** patched or vendored reader; larger stack; process per extraction. **Source:** design, library
  sources. **Evidence:** `pdf_bound.rs:1455-1570`; `pdf-extract-0.12.1/src/lib.rs:1860-1868`, `:2352-2362`,
  `:920-929`, `:989-992`; spec 14.5.
- **DEC-002:** the limits are fixed `pub(crate) const` in `pdf_bound.rs`: `FORM_DEPTH_LIMIT` 64, `CLIMB_LIMIT`
  256, `MAP_NESTING_LIMIT` 64; no configuration, no test seam. **Obliged by:** SPEC-0092/FR-NEW-001 to
  FR-NEW-003, spec D-4 to D-7. **Rationale:** the messages carry the numbers literally; a scaled limit proves
  nothing about the shipped one (SPEC-0089 DEC-002). **Alternatives considered:** configuration keys (D-6
  rejects). **Source:** spec. **Evidence:** spec 6.1.
- **DEC-003:** form depth is the drawing stack's length at a `Do`: the page content is index 0, so drawing a
  form from the top of a stack of length L makes it depth L; the walk refuses when L > `FORM_DEPTH_LIMIT`
  (`page <N> draws PDF forms nested more than 64 deep`, `<N>` = `budget.page_no`). The check runs after the
  name and the XObject lookup succeed (a missing XObject still breaks, the reader panics there today) and
  before `page.decode(id)`, so a refused form is never measured (spec D-9). The skip of a form already on the
  stack (`pdf_bound.rs:1550-1552`) is removed: a self draw is walked like the reader runs it and reaches
  depth 65. Every pushed XObject counts, whatever its `Subtype`, since the reader runs every drawn XObject as
  content (`lib.rs:1860-1868`). A form drawn side by side is popped between draws and adds no depth.
  **Obliged by:** SPEC-0092/FR-NEW-001, FR-NEW-004. **Rationale:** the walk's stack is exactly the reader's
  recursion; checking before the measure keeps the order of FR-NEW-004. **Alternatives considered:** keep the
  skip and refuse a repeated id (misses a chain of distinct forms, E2E-SEC-004); count only `/Subtype /Form`
  (leaves the reader's image recursion uncounted). **Source:** design. **Evidence:** `pdf_bound.rs:1542-1565`;
  `lib.rs:1860-1868`.
- **DEC-004:** `climb` mirrors `get_inherited` (`lib.rs:2352-2362`): at level 0 the page, then each `Parent`
  read as a reference and resolved with `doc.get_dictionary`; a node's value is usable when its key, resolved
  one step as `maybe_deref` does, is a dictionary (`Resources`) or an array (`MediaBox`, any array:
  `FromObj for Vec<T>` accepts any array, `lib.rs:211-217`); another kind is passed over and the climb goes
  on. A usable value at level k <= 256 is the answer; reaching level 257 refuses (`page <N> inherits from more
  than 256 levels of the PDF page tree`); a missing or non reference `Parent`, or an unresolvable one, ends the
  climb with no value (today's answer: empty resources, or the reader's `expect("MediaBox")` panic, contained,
  `pdf parser failed on this document`). No `seen` set: a loop simply climbs past 256. The walk climbs
  `Resources` then `MediaBox` before decoding the page content (FR-NEW-004 order; the MediaBox value itself is
  not used by the walk). **Obliged by:** SPEC-0092/FR-NEW-002, FR-NEW-004, FR-NEW-007. **Rationale:** the
  reader's climb takes the first value it accepts; refusing at the same level the reader would reach keeps
  parity. **Alternatives considered:** a `seen` set with a separate loop message (spec D-8 forbids a second
  message). **Source:** library source. **Evidence:** `lib.rs:2352-2362`, `:2405-2413`, `:174-180`, `:211-217`;
  `pdf_bound.rs:1349-1362`.
- **DEC-005:** for each font the walk loads (`"Tf"` arm), after `page.decode(id)` succeeds for a `ToUnicode`
  stream (any subtype the walk lists in `font_streams`) or a Type0 `Encoding` stream, the walk takes the bytes
  the reader will parse (`reader_bytes(stream, page.armed.contains(&id))`), records them in the held figure as
  it does for a form (`budget.hold(size)`), and refuses when `map_nesting(&bytes) > MAP_NESTING_LIMIT` (`a PDF
  character map is nested more than 64 deep`). `map_nesting` is one iterative pass mirroring the cmap
  parser's lexing (`adobe-cmap-parser-0.4.1/src/lib.rs:28-150`): outside a string, `[` and `<<` open a level,
  `]` and `>>` close one, `<` not followed by `<` starts a hexadecimal string skipped to `>`, `%` starts a
  comment skipped to the end of the line, `(` opens a string level; inside a string, `\` skips the next byte,
  `(` opens and `)` closes a level, every other byte (brackets included) is text. Levels count the three kinds
  together, unclosed levels included; the pass stops as soon as the depth passes 64. Order per map: stream
  limit, then page total (both in `page.decode`), then nesting (FR-NEW-004). FontFile streams are not scanned
  (not parsed by the cmap parser). **Obliged by:** SPEC-0092/FR-NEW-003, FR-NEW-004. **Rationale:** the
  parser's recursion follows these levels one for one; lexing like the parser avoids counting brackets inside
  strings and comments. **Alternatives considered:** a streaming scan inside the measurer (reports nesting
  before the stream limit, against FR-NEW-004); parsing with the library on a big stack (moves the threshold
  only). **Source:** library source. **Evidence:** `adobe-cmap-parser lib.rs:28-30`, `:45-66`, `:79-90`,
  `:110-146`; `pdf_bound.rs:1504-1517`, `:1573-1595`.
- **DEC-006:** the three refusals are `ToolError::invalid_argument` with the reason only; `prepare_extraction`
  adds `could not extract <path>: ` as for every reader error, and the refusal leaves no trace by the existing
  path (it returns before `land_conversion`, logged once at INFO by `logging::log_tool_failure`). **Obliged by:**
   SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-005. **Rationale:** SPEC-0088/0089 convention, no new code
  path. **Alternatives considered:** a dedicated error code (spec D-11 fixes `ERR_INVALID_ARGUMENT`).
  **Source:** existing code. **Evidence:** `extract.rs:378-387`; `fs_ops.rs:1745-1790`; `logging.rs:79-85`.
- **DEC-007:** `symbols::walk` and `walk_descendants` become explicit stack loops: pop a node, visit it, push
  its named children in reverse so the leftmost is visited next; `walk_descendants` starts with the root's
  named children (the root itself is not visited). `name_of` stops the descendant visit at the first
  identifier found (today it keeps the first and ignores the rest: same result, less work). Pre order and
  results are identical to today's for every tree. **Obliged by:** SPEC-0092/FR-NEW-006, FR-NEW-008.
  **Rationale:** tree-sitter builds and walks 1 000 000 levels without recursion (spec 14.5 row 24); only
  our visit recursed. **Alternatives considered:** `TreeCursor` walk (equivalent, but filtering named nodes
  and the descendant rule are clearer on a stack); a depth cap (changes answers, spec D-10). **Source:**
  design. **Evidence:** `symbols.rs:207-213`, `:218-247`.
- **DEC-008:** `in_child(module, name, body)` (in `nesting_testkit.rs`): when the environment variable
  `MCPFS_CHILD_CASE` equals `name`, it runs `body` on a fresh `tokio` multi thread runtime of 2 workers with
  `thread_stack_size(2 * 1024 * 1024)` (the production default: `#[tokio::main]` sets no stack size, so std's
  2 MiB applies to workers and blocking threads alike), then prints `MCPFS_CHILD_CASE_PASSED <name>` on stdout.
  Otherwise it spawns `std::env::current_exe()` with `--exact <module><name> --nocapture --test-threads 1`, the
  variable set and `RUST_MIN_STACK` removed, waits at most 120 s (killing the child past it), and asserts the
  exit status is success and stdout holds the marker; a failed assertion message carries the exit code or the
  signal (`std::os::unix::process::ExitStatusExt::signal`) and the child's last 4 096 bytes of stderr. The
  body's own assertions run in the child; a panic there fails the child, hence the parent. **Obliged by:**
  SPEC-0092/FR-NEW-006, spec D-14, 14.3 testability. **Rationale:** an abort on any thread ends the whole
  process, so only another process can observe it; re running the test binary for one case is the in tree
  pattern. **Alternatives considered:** a thread with a small stack (an overflow aborts the process anyway);
  spawning the server binary over HTTP (needs a built binary and auth wiring, slower, no gain: the attacked
  code is the same engine). **Source:** existing code. **Evidence:** `storage/blob/s3.rs:282-310`;
  `crates/mcp-fs/src/main.rs:6-9`.
- **DEC-009:** a test runs in a child exactly when its document or source aborts the process on today's code:
  E2E-015, 016, 017, 021, 022, 023, 024, 027, 028 and every E2E-SEC test; the others run in process (their
  documents are within the measured thresholds, so RED shows as a plain assertion). **Obliged by:**
  SPEC-0092/FR-NEW-006 (spec D-14). **Rationale:** RED evidence must be an assertion, not a crashed runner;
  child runs cost about one second each. **Alternatives considered:** every test in a child (slower, no gain).
  **Source:** spec 14.5. **Evidence:** spec 14.5 rows 1 to 21.
- **DEC-010:** "the server is still up" is `still_up(&f, pid_before)`: within 1 s after the answer,
  `McpServer::new(state, "leo@example.com").fs_read` of `/notes/hello.txt` in `orbit` returns `hello`, and
  `std::process::id()` equals `pid_before` read before the call. E2E-SEC-001 and E2E-SEC-011 also run
  `bystander_loop` (a task reading every 50 ms until told to stop, each read asserted `hello` within 1 s) while
  the attacked call runs. `nesting_fixture()` is `archive_testkit::office_fixture()` plus `orbit` seeded with
  `f.seed_project("orbit", "leo@example.com")` and the note. **Obliged by:** SPEC-0092/FR-NEW-006, spec 9
  header. **Rationale:** reads through the real handler, from another project and member. **Alternatives
  considered:** an engine read (would skip the authorization a bystander goes through). **Source:** design.
  **Evidence:** `tools/archive_testkit.rs:457-463`; `mcp/server.rs:1567`.
- **DEC-011:** SPEC-0092 adds its own `nesting_testkit.rs` and `nesting_e2e.rs`; the SPEC-0088/0089 kits are
  imported read only (`pdf_testkit::{PdfWriter, PdfObj, plain, pages_writer, bomb, PDF_BOMB}`,
  `archive_testkit::{office_fixture, extract_text, seed, read, nothing_left, heavy, OWNER, MOUNT}`).
  `chain.pdf` (about 1 700 000 bytes) does not pass through `pdf_testkit::small()`. **Obliged by:**
  SPEC-0092/FR-NEW-001 to FR-NEW-009 (test layout); SPEC-0091/FR-NEW-003 (E2E-017 unmodified suites).
  **Rationale:** the next spec in the queue asserts those files unchanged. **Alternatives considered:** extend
  `office_fixture` and `pdf_testkit` (spec 14.3 suggests it; it would break SPEC-0091 E2E-017).
  **Source:** design. **Evidence:** SPEC-0091 spec E2E-017.
- **DEC-012:** fixtures follow the spec literally (section 9 header): catalog 1, page tree 2, page 3, content
  4, font 5 `Helvetica`, forms from object 10, page groups from object 100, maps at object 9, uncompressed
  streams. Builders: `forms_chain_pdf(n_forms, bottom_text)`, `loop_pdf()`, `inherit_pdf()`, `pair_pdf()`,
  `repeat_pdf(times)`, `parent_chain_pdf(groups, with_resources_on_page, with_media_box_on_page)`,
  `parent_loop_pdf(page_resources, page_media_box, content)`, `cmap_pdf(nesting_bytes)`, `type0_pdf(nesting)`,
  `order_pdf()`, `two_pages_bomb_then_loop_pdf()`, `both_pdf()`, `deep_source(head, open, middle, close, n, tail)`. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-008 (the literal payloads of section 9).
  **Rationale:** the payload is the attacker's file. **Alternatives considered:**
  committed binary fixtures (none needed). **Source:** spec. **Evidence:** spec 9.2, 9.3.
- **DEC-013:** walk and reader parity at every stop: one child test per `break` of the walk (Tf without a name;
  Tf on a font missing from the resources; `cs` without a name; `cs` on a colour space missing from the
  resources; `Do` without a name; `Do` on an XObject missing from the resources; `Do` on an object that is not
  a stream; `Do` on a form whose content cannot be parsed), each page content putting the defect first and the
  self drawing form of E2E-SEC-001 after it. Each asserts the child exits normally and the answer is
  `could not extract /docs/parity.pdf: pdf parser failed on this document` (the reader panics at the defect,
  contained). A red parity test is a story failure to report (the reader would continue where the walk stops),
  never patched silently. **Obliged by:** SPEC-0092/FR-NEW-006, FR-NEW-007, spec 14.3 parity. **Rationale:**
  the walk counts only what it reaches. **Alternatives considered:** refusing at every stop (changes today's
  answers). **Source:** spec 14.3. **Evidence:** `pdf_bound.rs:1506`, `:1513`, `:1520`, `:1528`, `:1543`,
  `:1547`, `:1553`, `:1557`; `lib.rs:1633-1640`, `:1709-1712`, `:1860-1868`, `:1457`.
- **DEC-014:** the closing story documents the limits (`README.md`, `AGENTS.md`, `.agent_docs/tools.md`,
  `.agent_docs/testing.md`); `.agent_docs/architecture.md` is updated in US-0226; the tool contract is
  unchanged (no description names the limits). **Obliged by:** n/a (no code impact): project documentation
  rule (`AGENTS.md`). **Rationale:** house closing pattern (SPEC-0089 DEC-023). **Alternatives considered:**
  naming the limits in `fs.extract_text`'s description (a contract change for no requirement). **Source:**
  constitution. **Evidence:** `AGENTS.md` documentation index.
- **DEC-015:** timing bounds are asserted inside the child around the attacked call with `Instant`: E2E-SEC-004
  under 2 s (refused at depth 65 after a load of 10 010 objects), E2E-SEC-011 under 10 s. **Obliged by:**
  SPEC-0092 7.1 (E2E-SEC-004, E2E-SEC-011). **Rationale:** the spec's figures, measured where the call runs.
  **Alternatives considered:** timing the whole child (includes process start). **Source:** spec. **Evidence:**
  spec 7.1.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0222 | refuse PDF forms nested past 64, loops included | P1 | SPEC-0092/FR-NEW-001, FR-NEW-005, FR-NEW-006, FR-NEW-007 | US-0221 | 2 | no |
| US-0223 | refuse a page tree climb past 256 levels | P1 | SPEC-0092/FR-NEW-002, FR-NEW-004, FR-NEW-006, FR-NEW-007 | US-0222 | 2 | no |
| US-0224 | refuse character and code maps nested past 64 | P1 | SPEC-0092/FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-007 | US-0223 | 2 | no |
| US-0225 | code search answers source of any nesting | P1 | SPEC-0092/FR-NEW-006, FR-NEW-008, FR-NEW-009 | US-0224 | 2 | no |
| US-0226 | nesting refusals in order, without trace, the walk and the reader in step | P2 | SPEC-0092/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007 | US-0225 | 2 | no |
| US-0227 | nesting refusals through the HTTP door and documentation | P2 | SPEC-0092/FR-NEW-001, FR-NEW-006, FR-NEW-007, FR-NEW-009 | US-0226 | 2 | yes |
