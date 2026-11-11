# US-0222: refuse PDF forms nested past 64, loops included

> Spec: SPEC-0092 (sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0221
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
A PDF whose forms draw each other in a loop, or chain more than 64 deep, ends the whole server today: the reader draws each form inside the previous one with no limit and overflows its thread's stack. After this story the SPEC-0089 walk counts the depth on its own drawing stack and refuses the draw that reaches depth 65 before the form is measured, so the reader never recurses past 64. The story also builds what every later exploit test of the spec stands on: the child process harness that turns an abort into a failed assertion, and the bystander project `orbit`.

## Requirements covered
- SPEC-0092/FR-NEW-001: IF a page of a PDF draws a form at depth 65 THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> draws PDF forms nested more than 64 deep`.
- SPEC-0092/FR-NEW-005 (partial: a form depth refusal leaves nothing and is logged once): THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-003.
- SPEC-0092/FR-NEW-006 (partial: PDF forms): THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- SPEC-0092/FR-NEW-007 (partial: nested forms within the limit, a form drawn side by side): THE document text extraction SHALL answer a PDF whose nestings are within FR-NEW-001 to FR-NEW-003 with the same text, page markers, page count, title, note, companion, preview, charge, audit line and refusals as before, and SHALL answer from an up to date companion without reading the PDF when no refresh is asked.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0092_sec_001 e2e_0092_sec_002 e2e_0092_sec_003 e2e_0092_sec_004 e2e_0092_001 e2e_0092_005 e2e_0092_008 e2e_0092_012 form_depth_is_checked`: 9 tests pass. Real assertions: the child process exit status (a normal exit, not signal 6), the refusal code and message, `nothing_left` (the `/docs` listing, the artifact lists, the audit trail, a 52 428 800 byte write), `leo@example.com` reading `hello` through `McpServer::fs_read`, the preview text of the forms that stay within the limit.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0092):** the three PDF nestings are refused by the SPEC-0089 walk before the reader runs (section 2, chosen approach). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-004, FR-NEW-006, spec 3.3, 14.3. **Rationale:** the walk already reproduces the reader's draws and font loads in drawing order, iteratively; three checks there bound every recursion the reader performs; safe Rust, no dependency change.
- **DEC-002 (SPEC-0092):** the limits are fixed `pub(crate) const` in `pdf_bound.rs`: `FORM_DEPTH_LIMIT` 64, `CLIMB_LIMIT` 256, `MAP_NESTING_LIMIT` 64; no configuration, no test seam. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, spec D-4 to D-7. **Rationale:** the messages carry the numbers literally; a scaled limit proves nothing about the shipped one (SPEC-0089 DEC-002).
- **DEC-003 (SPEC-0092):** form depth is the drawing stack's length at a `Do`: the page content is index 0, so drawing a form from the top of a stack of length L makes it depth L; the walk refuses when L > `FORM_DEPTH_LIMIT` (`page <N> draws PDF forms nested more than 64 deep`, `<N>` = `budget.page_no`). The check runs after the name and the XObject lookup succeed (a missing XObject still breaks, the reader panics there today) and before `page.decode(id)`, so a refused form is never measured (spec D-9). The skip of a form already on the stack (`pdf_bound.rs:1550-1552`) is removed: a self draw is walked like the reader runs it and reaches depth 65. Every pushed XObject counts, whatever its `Subtype`, since the reader runs every drawn XObject as content (`lib.rs:1860-1868`). A form drawn side by side is popped between draws and adds no depth. **Obliged by:** SPEC-0092/FR-NEW-001, FR-NEW-004. **Rationale:** the walk's stack is exactly the reader's recursion; checking before the measure keeps the order of FR-NEW-004.
- **DEC-006 (SPEC-0092):** the three refusals are `ToolError::invalid_argument` with the reason only; `prepare_extraction` adds `could not extract <path>: ` as for every reader error, and the refusal leaves no trace by the existing path (it returns before `land_conversion`, logged once at INFO by `logging::log_tool_failure`). **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-003, FR-NEW-005. **Rationale:** SPEC-0088/0089 convention, no new code path.
- **DEC-008 (SPEC-0092):** `in_child(module, name, body)` (in `nesting_testkit.rs`): when the environment variable `MCPFS_CHILD_CASE` equals `name`, it runs `body` on a fresh `tokio` multi thread runtime of 2 workers with `thread_stack_size(2 * 1024 * 1024)` (the production default: `#[tokio::main]` sets no stack size, so std's 2 MiB applies to workers and blocking threads alike), then prints `MCPFS_CHILD_CASE_PASSED <name>` on stdout. Otherwise it spawns `std::env::current_exe()` with `--exact <module><name> --nocapture --test-threads 1`, the variable set and `RUST_MIN_STACK` removed, waits at most 120 s (killing the child past it), and asserts the exit status is success and stdout holds the marker; a failed assertion message carries the exit code or the signal (`std::os::unix::process::ExitStatusExt::signal`) and the child's last 4 096 bytes of stderr. The body's own assertions run in the child; a panic there fails the child, hence the parent. **Obliged by:** SPEC-0092/FR-NEW-006, spec D-14, 14.3 testability. **Rationale:** an abort on any thread ends the whole process, so only another process can observe it; re running the test binary for one case is the in tree pattern.
- **DEC-009 (SPEC-0092):** a test runs in a child exactly when its document or source aborts the process on today's code: E2E-015, 016, 017, 021, 022, 023, 024, 027, 028 and every E2E-SEC test; the others run in process (their documents are within the measured thresholds, so RED shows as a plain assertion). **Obliged by:** SPEC-0092/FR-NEW-006 (spec D-14). **Rationale:** RED evidence must be an assertion, not a crashed runner; child runs cost about one second each.
- **DEC-010 (SPEC-0092):** "the server is still up" is `still_up(&f, pid_before)`: within 1 s after the answer, `McpServer::new(state, "leo@example.com").fs_read` of `/notes/hello.txt` in `orbit` returns `hello`, and `std::process::id()` equals `pid_before` read before the call. E2E-SEC-001 and E2E-SEC-011 also run `bystander_loop` (a task reading every 50 ms until told to stop, each read asserted `hello` within 1 s) while the attacked call runs. `nesting_fixture()` is `archive_testkit::office_fixture()` plus `orbit` seeded with `f.seed_project("orbit", "leo@example.com")` and the note. **Obliged by:** SPEC-0092/FR-NEW-006, spec 9 header. **Rationale:** reads through the real handler, from another project and member.
- **DEC-011 (SPEC-0092):** SPEC-0092 adds its own `nesting_testkit.rs` and `nesting_e2e.rs`; the SPEC-0088/0089 kits are imported read only (`pdf_testkit::{PdfWriter, PdfObj, plain, pages_writer, bomb, PDF_BOMB}`, `archive_testkit::{office_fixture, extract_text, seed, read, nothing_left, heavy, OWNER, MOUNT}`). `chain.pdf` (about 1 700 000 bytes) does not pass through `pdf_testkit::small()`. **Obliged by:** SPEC-0092/FR-NEW-001 to FR-NEW-009 (test layout); SPEC-0091/FR-NEW-003 (E2E-017 unmodified suites). **Rationale:** the next spec in the queue asserts those files unchanged.
- **DEC-015 (SPEC-0092):** timing bounds are asserted inside the child around the attacked call with `Instant`: E2E-SEC-004 under 2 s (refused at depth 65 after a load of 10 010 objects), E2E-SEC-011 under 10 s. **Obliged by:** SPEC-0092 7.1 (E2E-SEC-004, E2E-SEC-011). **Rationale:** the spec's figures, measured where the call runs.
### Data model
No new type in production: the existing `stack: Vec<Drawing>` of `walk` (`pdf_bound.rs:1490-1491`) is the depth (its length minus one is the depth of the top). The `stack.iter().any(|d| d.form == Some(id))` skip (`:1550-1552`) is deleted.
### Contracts
- `pub(crate) const FORM_DEPTH_LIMIT: usize = 64;` in `crates/core/src/docs/pdf_bound.rs`.
- In `walk`, `"Do"` arm, order: name (else `break`), XObject lookup (else `break`), then `if stack.len() > FORM_DEPTH_LIMIT { return Err(ToolError::invalid_argument(format!("page {} draws PDF forms nested more than {FORM_DEPTH_LIMIT} deep", budget.page_no))); }`, then `page.decode(id, budget)?`, then the object (else `break`), the parse (else `break`), the push. Answer seen by the member: `ERR_INVALID_ARGUMENT`, `could not extract <path>: page <N> draws PDF forms nested more than 64 deep` (prefix added by `prepare_extraction`, unchanged).
- Test only, `crates/core/src/tools/nesting_testkit.rs` (`#[cfg(test)] pub(crate) mod nesting_testkit;`):
  - `pub(crate) const CHILD_ENV: &str = "MCPFS_CHILD_CASE";`
  - `pub(crate) fn in_child<F, Fut>(module: &str, name: &str, body: F) where F: FnOnce() -> Fut, Fut: std::future::Future<Output = ()>`: in the child (variable equal to `name`) builds `tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_stack_size(2 * 1024 * 1024).enable_all()`, `block_on(body())`, prints `MCPFS_CHILD_CASE_PASSED {name}`; in the parent spawns `std::env::current_exe()` with `["--exact", &format!("{module}{name}"), "--nocapture", "--test-threads", "1"]`, `env(CHILD_ENV, name)`, `env_remove("RUST_MIN_STACK")`, waits at most 120 s (polling `try_wait`, killing past it), asserts `status.success()` and the marker in stdout; the failure message names the exit code or the signal (`ExitStatusExt::signal`) and the last 4 096 bytes of stderr.
  - `pub(crate) async fn nesting_fixture() -> Fixture`: `archive_testkit::office_fixture()` (project `atlas`, `mia@example.com`, empty `/docs`), plus `f.seed_project("orbit", "leo@example.com")` and `/notes/hello.txt` reading `hello` written through the `orbit` volume client.
  - `pub(crate) const BYSTANDER: &str = "leo@example.com";`
  - `pub(crate) async fn still_up(f: &Fixture, pid_before: u32)`: asserts `std::process::id() == pid_before`, then within 1 s (`tokio::time::timeout`) `McpServer::new(f.state.clone(), BYSTANDER.into()).fs_read(..)` of `/notes/hello.txt` in `orbit` answers a success whose content is `hello`.
  - `pub(crate) async fn bystander_loop(state: Arc<AppState>, stop: tokio::sync::watch::Receiver<bool>) -> usize`: reads as `still_up` does every 50 ms until `stop` turns true, each read asserted within 1 s; returns the number of reads (at least 1 asserted by the caller).
  - PDF builders (fixtures below), all returning `Vec<u8>` built with `pdf_testkit::PdfWriter` and `PdfObj`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify |
| `crates/core/src/tools/nesting_testkit.rs` | create |
| `crates/core/src/tools/nesting_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] pub(crate) mod nesting_testkit;`, `#[cfg(test)] mod nesting_e2e;`) |

Read only (imported, never modified): `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/archive_testkit.rs`.
### Existing patterns
`run_child` (`crates/core/src/storage/blob/s3.rs:282-310`) for the child; `refused_on` and `nothing_left` usage in `crates/core/src/tools/pdf_bound_e2e.rs:45-48` and `crates/core/src/tools/archive_testkit.rs:522-560`; `pages_writer` (`crates/core/src/tools/pdf_testkit.rs:285-310`) for the page objects; `McpServer::new(state, person)` and `fs_read` (`crates/core/src/mcp/server.rs:1567`); the walk's drawing stack (`crates/core/src/docs/pdf_bound.rs:1455-1570`).
### Fixtures
Object layout of the spec (section 9 header): catalog 1, page tree 2 (`/Kids [3 0 R]`), page 3 with `/MediaBox [0 0 612 792]`, `/Resources << /Font << /F1 5 0 R >> /XObject << /A 10 0 R >> >>`, content 4, font 5 `<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>`, uncompressed streams (`plain`).
- `forms_pdf()` (E2E-001): page content `BT /F1 12 Tf 72 700 Td (Cover) Tj ET /A Do`; object 10 form `/A` with resources `/XObject << /B 11 0 R >>`, content `/B Do`; 11 `/B` with `/XObject << /C 12 0 R >>`, content `/C Do`; 12 `/C` with `/Font << /F1 5 0 R >>`, content `BT /F1 12 Tf 72 600 Td (Deep text) Tj ET`. Every form `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << ... >> >>`.
- `forms_chain_pdf(forms: u32)` (E2E-005 with 64, E2E-012 with 65, E2E-SEC-004 with 10 000): page content `/A Do`; objects 10 to 10 + forms - 2 each a form with `/Resources << /XObject << /A n+1 0 R >> >>`, content `/A Do`; the last, object 10 + forms - 1, shows `BT /F1 12 Tf 72 600 Td (Bottom) Tj ET` (`Deep` for E2E-SEC-004) with `/Font << /F1 5 0 R >>`. E2E-SEC-004's document is about 1 700 000 bytes and never goes through `pdf_testkit::small`.
- `loop_pdf()` (E2E-SEC-001, 862 bytes in the reproduction): page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET /A Do`; object 10 `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /A 10 0 R >> >> /Length 5 >> stream` `/A Do` `endstream`.
- `inherit_pdf()` (E2E-SEC-002): as `loop_pdf` but object 10 is `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 5 >>` with content `/A Do`.
- `pair_pdf()` (E2E-SEC-003): page draws `/A`; object 10 resources `/XObject << /B 11 0 R >>`, content `/B Do`; object 11 resources `/XObject << /A 10 0 R >>`, content `/A Do`.
- `repeat_pdf(times: usize)` (E2E-008, 50): page content `/A Do` written `times` times separated by a space; object 10 shows `BT /F1 12 Tf 72 600 Td (Tile) Tj ET` with `/Font << /F1 5 0 R >>`.
- `deep_bomb_pdf()` (design test): `forms_chain_pdf(65)` whose last form (object 74) is `pdf_testkit::bomb("")` instead of the text form.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0092/E2E-SEC-001: A form that draws itself
- **Covers:** occurrence 1 (the originating occurrence of BL-0147)
- **Red before the fix because:** `Then the refusal is ...` fails: the process serving the call aborts (signal 6, 14.5 row 1) and no answer arrives; `And the server is still up` fails with it.
- **Given** `/docs/loop.pdf`, 862 bytes, whose page content is `BT /F1 12 Tf 72 700 Td (Hello) Tj ET /A Do`, page resources `/XObject << /A 10 0 R >>`, object 10 `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /A 10 0 R >> >> /Length 5 >> stream` `/A Do` `endstream`
- **When** the member asks for its text, while `leo@example.com` reads `/notes/hello.txt` in a loop
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep`
- **And** every read of `leo@example.com` receives `hello`; nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_001_a_form_that_draws_itself`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: on US-0221 code the child ends by signal 6 (`fatal runtime error: stack overflow, aborting`), so the parent assertion `the child exited normally` fails; no refusal arrives
- Note: `bystander_loop` runs from before the call until its answer; the test asserts at least one read happened.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-002: A form inheriting the resources that name it
- **Covers:** occurrence 1, the form without resources of its own
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 2).
- **Given** `/docs/inherit.pdf` as `/docs/loop.pdf` but object 10 is `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 5 >> stream` `/A Do` `endstream`, using the page's resources
- **When** the member asks for its text
- **Then** the refusal is the form depth message for page 1 of `/docs/inherit.pdf`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_002_a_form_inheriting_the_resources_t`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 2)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-003: Two forms drawing each other
- **Covers:** occurrence 1, through another form
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 3).
- **Given** `/docs/pair.pdf` whose page draws `/A`, object 10 a form with resources `/XObject << /B 11 0 R >>` and content `/B Do`, object 11 a form with resources `/XObject << /A 10 0 R >>` and content `/A Do`
- **When** the member asks for its text
- **Then** the refusal is the form depth message for page 1 of `/docs/pair.pdf`
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_003_two_forms_drawing_each_other`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 3)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-004: A chain of 10 000 distinct forms
- **Covers:** occurrence 1, depth without a loop
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 6; a chain of 400 already ends it, row 5).
- **Given** `/docs/chain.pdf`, about 1 700 000 bytes, whose page draws `/A` (object 10), each object n from 10 to 10 008 a form with resources `/XObject << /A n+1 0 R >>` and content `/A Do`, object 10 009 a form showing `BT /F1 12 Tf 72 600 Td (Deep) Tj ET`
- **When** the member asks for its text
- **Then** the refusal is the form depth message for page 1 of `/docs/chain.pdf`, within 2 seconds
- **And** nothing left; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `archive_testkit::extract_text` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_004_a_chain_of_10_000_distinct_forms`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 6)
- Note: the 2 seconds are measured with `Instant` around the call inside the child (DEC-015).
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-001: PDF with three nested forms
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/docs/forms.pdf` whose page content is `BT /F1 12 Tf 72 700 Td (Cover) Tj ET /A Do`, `/A` a form drawing `/B`, `/B` a form drawing `/C`, `/C` a form showing `BT /F1 12 Tf 72 600 Td (Deep text) Tj ET`, each form naming the next one and `/F1` in its own resources
- **When** the member asks for its text
- **Then** the answer names `/docs/forms.md`, format `pdf`, 1 page, not cached; its preview reads `*[Page 1]*`, `Cover`, `Deep text`, in that order
- **And** `/docs/forms.md` holds that text; the audit trail has one extraction line for `/docs/forms.md`
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_001_pdf_with_three_nested_forms`
- Green on arrival: today three nested forms extract; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-005: Forms nested exactly 64 deep
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/deep64.pdf` whose page content is `/A Do` with `/A` form 10; forms 10 to 72 each `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /A n+1 0 R >> >> >>` drawing `/A Do`, n being its own number; form 73 showing `BT /F1 12 Tf 72 600 Td (Bottom) Tj ET` with `/F1` in its resources (64 forms)
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Bottom`
- **And** the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_005_forms_nested_exactly_64_deep`
- Green on arrival: 64 forms are within the measured threshold today (200 fine); then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-008: One form drawn 50 times side by side
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-007; **Priority:** High
- **Given** `/docs/repeat.pdf` whose page content is `/A Do` written 50 times, `/A` a form showing `BT /F1 12 Tf 72 600 Td (Tile) Tj ET` with `/F1` in its resources
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Tile` 50 times
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_008_one_form_drawn_50_times_side_by_side`
- Green on arrival: today the 50 side by side draws extract; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-012: Forms nested 65 deep
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-005; **Priority:** Critical
- **Given** `/docs/deep65.pdf` as E2E-005 with one form more: forms 10 to 73 draw the next, form 74 shows `Bottom` (65 forms)
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/deep65.pdf: page 1 draws PDF forms nested more than 64 deep`
- **And** nothing left; the server is still up
- Level / Driver / Location: functional e2e / typed engine call `archive_testkit::extract_text(&f, OWNER, path, refresh)` (authorizes, then `core::fs_ops::extract_document`) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_012_forms_nested_65_deep`
- Red before the change because: on US-0221 code 65 forms extract (below the 200 threshold), so the refusal assertion fails with a success
- Cleanup: none (temporary directories dropped with the fixture)

### Design tests (guard a decision, not in the spec)
- `form_depth_is_checked_before_the_form_is_measured` (nesting_e2e.rs, engine call, in process): `deep_bomb_pdf()` stored as `/docs/deepbomb.pdf`; the refusal is `could not extract /docs/deepbomb.pdf: page 1 draws PDF forms nested more than 64 deep`, not the stream limit message, and `pdf_testkit::decoded_held("/docs/deepbomb.pdf")` is below `pdf_testkit::STREAM_LIMIT` (the bomb was never measured). Takes `archive_testkit::heavy()` first. Red on US-0221 code: the walk measures the 65th form and refuses with `a PDF stream decodes to more than 104857600 bytes`. Mutation it kills: check the depth after `page.decode(id, budget)`.

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-001 | set `FORM_DEPTH_LIMIT` to 2: the third form is refused, the test fails |
| E2E-005 | refuse when `stack.len() >= FORM_DEPTH_LIMIT`: depth 64 is refused, the test fails |
| E2E-008 | refuse a form already drawn on this page (a per page set of drawn ids): the second draw is refused, the test fails |

## Constraints
- Files not to touch: `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/office_bound_e2e.rs` (SPEC-0091 E2E-017 asserts them unchanged), `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Never recurse in the walk; never keep a `seen` set of forms (a chain of distinct forms must still be counted)
- Never run an exploit test body in the test runner process: always through `in_child`
- No dash as punctuation in code, comments or docs
- Scope boundary: page tree climb is US-0223, character maps US-0224, code search US-0225, order and parity US-0226, the HTTP door and docs US-0227

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-001, E2E-SEC-002, E2E-SEC-003, E2E-SEC-004, E2E-012 (step 1: harness, builders and tests on US-0221 code; the four exploit tests fail through the parent (signal 6), E2E-012 and the design test fail by assertion; step 2: the depth check and the removal of the skip).
- [ ] Green on arrival tests (E2E-001, E2E-005, E2E-008): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: every `e2e_0089_` and `e2e_0088_` test, the 19 tests of `docs::pdf_bound`, `pdf_extracts_text_pages_and_metadata`, `pdf_utf16_title_is_decoded`, `pdf_end_to_end_writes_the_companion`, every earlier `e2e_0092_` test; `cargo test -p mcp-fs-core --lib -- e2e_0089_ --test-threads=1` green with its held figures unchanged.
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0222: refuse PDF forms nested past 64, loops included` on its story branch `story/US-0222-refuse-pdf-forms-nested-past-64-loops-in`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0092-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0092` and the requirement ids (SPEC-0092/FR-NEW-001, FR-NEW-005, FR-NEW-006, FR-NEW-007).
