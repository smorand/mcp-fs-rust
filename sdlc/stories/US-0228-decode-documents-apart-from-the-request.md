# US-0228: decode documents apart from the request threads, behind today's ceiling

> Spec: SPEC-0091 (sdlc/specs/SPEC-0091-keep-requests-answered-while-documents-are-extracted/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0227
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
While a large document decodes, every other request scheduled on the same request processing thread waits for it: 5.5 seconds behind the large PDF. After this story the six built in readers run on the blocking pool, so request threads only await; at most as many documents decode at once as there are request processing threads, waiting ones start in arrival order, and every extraction answer stays byte identical to the answer recorded on the code before this story.

## Requirements covered
- SPEC-0091/FR-NEW-001: WHILE one or more documents are being decoded for text extraction THE server SHALL add at most 250 ms to the answer time of every request that decodes no document, compared with the same request when no document is decoding.
- SPEC-0091/FR-NEW-002 (partial: the ceiling, arrival order, refusals before decoding and cached answers take no place, a refusal frees its place): The document text extraction SHALL decode at most as many documents at once as the server has request processing threads, and SHALL start a waiting extraction as soon as a decoding ends, in arrival order.
- SPEC-0091/FR-NEW-003 (partial: answers, companions, artifacts, audit lines and charges as recorded; refusals during probes): The document text extraction SHALL answer, for the same document, options and member, exactly what it answers today: text, preview, character count, truncation, notes, companion, table and image artifacts, audit line, write charge, and every refusal code and message.
- SPEC-0091/FR-NEW-006 (partial: the assistant tool door): The server SHALL apply FR-NEW-001 to FR-NEW-005 alike to extractions asked through the assistant tool and through the HTTP API.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0091_ --test-threads=1` (the 15 tests of this story) and the SPEC-0088 and SPEC-0089 suites unmodified. Real assertions: each probe's measured answer time under 250 ms, `most_at_once()` read from the server's places, the answer order, the answers, companions, table lists and remaining allowances equal to the literals recorded on US-0227's code.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0091):** the ceiling is a fair `tokio::sync::Semaphore` held by `DecodePlaces`, created at the first acquire with `Handle::current().metrics().num_workers().max(1)` permits: the runtime's request processing threads (honouring `TOKIO_WORKER_THREADS`), 1 on a current thread test runtime. **Obliged by:** SPEC-0091/FR-NEW-002 (ceiling = request processing threads, arrival order), DI-3, DI-4. **Rationale:** tokio's semaphore grants permits in request order, which is the arrival order the spec asks; the count is read from the runtime that serves the requests, so a 1 or 2 worker test runtime gives 1 or 2 places with no configuration.
- **DEC-002 (SPEC-0091):** the six readers (`.pdf`, `.docx`, the four PowerPoint suffixes, `.xlsx`, `.xlsm`, `.html`, `.htm`, `.csv`, as dispatched at `extract.rs:165-171`) run inside one `tokio::task::spawn_blocking` closure that owns the bytes (`Vec<u8>`) and the permit; images, fenced and text suffixes and the audio and video refusal stay on the request side and take no place. **Obliged by:** SPEC-0091/FR-NEW-001, FR-NEW-002, spec 3.3. **Rationale:** the blocking pool exists on every runtime flavour; the request thread only awaits.
- **DEC-003 (SPEC-0091):** the permit is acquired on the request side before `spawn_blocking` and moved into the closure, dropped when the closure returns: a waiting extraction whose future is dropped never starts, a started one keeps its place until its decoding ends, success, refusal or panic alike. **Obliged by:** SPEC-0091/FR-NEW-002, FR-NEW-005. **Rationale:** dropping the future cancels the acquire; the closure owns the release.
- **DEC-004 (SPEC-0091):** the test only figures are carried back as SPEC-0084 DEC-006 does: the closure clears `LAST_OFFICE_HELD` and `LAST_PDF_HELD` on its own thread first, runs the reader, then takes both and returns them with the outcome; after the join the request side sets them on its own thread, and the existing code (`extract.rs:180-190`) moves them into `OFFICE_HELD` and `PDF_HELD` keyed by path, with no `.await` between. `LAST_OFFICE_HELD` becomes `pub(crate)` for that. The log lines stay emitted by the request side (`mcp/server.rs:57`, `api/dataplane.rs:264`), never by the decoding thread (DI-2). **Obliged by:** SPEC-0091/FR-NEW-003 (E2E-017), spec 3.3, DI-1, DI-2. **Rationale:** the SPEC-0088 and SPEC-0089 suites read the figures on the thread that awaited the call; a pooled blocking thread must not leak a stale figure to the next decoding.
- **DEC-005 (SPEC-0091):** `DecodePlaces` lives in `SafetyManager`, created in `SafetyManager::new`; `fs_ops` passes `safety.decode_places()` to `prepare_extraction`. **Obliged by:** SPEC-0091/FR-NEW-002, FR-NEW-003 (E2E-017: `extract_document`'s signature, used by `archive_testkit.rs`, must not change). **Rationale:** the manager is the composition wide object the engine already receives (its `limits` field is the precedent, `safety.rs:34-39`); per server state means parallel tests never share places.
- **DEC-008 (SPEC-0091):** the probes: the operator's health check is `GET /health` on the router built with the state; noa's listing is `McpServer::fs_list_dir` of `/notes`; noa's read is `McpServer::fs_read` of `/notes/todo.txt`. A probe loop runs as a spawned task sending one probe every 100 ms from 50 ms after the extraction started until a `watch` channel says stop, recording each answer time. "Abandon" is aborting the task that awaits `McpServer::fs_extract_text` (the transport drops that future when a client cancels), and, at the HTTP door, closing the client socket of a served router (hyper drops the handler future when it reads the end of the connection). **Obliged by:** SPEC-0091/FR-NEW-001, FR-NEW-005, FR-NEW-006. **Rationale:** real handlers, no stand in.
- **DEC-009 (SPEC-0091):** "recorded answers" are captured on the code before this spec: the first story writes the tests E2E-016, E2E-024 and E2E-025 with their expectations printed by a first run on its base commit (US-0227's code), pasted as literals into `offload_e2e.rs`, and committed with that story; later stories never edit them. A field that differs between two record runs on the base code would be excluded and named in the story (none expected: the extraction answer holds no time). **Obliged by:** SPEC-0091/FR-NEW-003 (recorded answer, glossary). **Rationale:** byte for byte against the old code, with no committed fixture file to regenerate.
### Data model
- `pub(crate) struct DecodePlaces { semaphore: std::sync::OnceLock<Arc<tokio::sync::Semaphore>>, #[cfg(test)] probe: std::sync::Mutex<PlacesProbe> }` in `crates/core/src/docs/extract.rs`.
- `#[cfg(test)] struct PlacesProbe { running: usize, most: usize, started: Vec<String> }` (US-0229 adds `faults`).
- `#[cfg(test)] struct Figures { office: Option<OfficeHeld>, pdf: Option<PdfHeld> }`.
### Contracts
- `SafetyManager` gains `decode_places: DecodePlaces` (built in `SafetyManager::new`) and `pub(crate) fn decode_places(&self) -> &DecodePlaces`.
- `DecodePlaces::acquire(&self) -> tokio::sync::OwnedSemaphorePermit` (async): the semaphore is built at the first call with `tokio::runtime::Handle::current().metrics().num_workers().max(1)` permits; a closed semaphore cannot happen (never closed), so the `acquire_owned` error is mapped to `ToolError::internal("decoding places closed")`.
- Test only: `DecodePlaces::most_at_once(&self) -> usize`, `DecodePlaces::started(&self) -> Vec<String>`; the closure increments `running` (updating `most`) and pushes its path to `started` when it begins, decrements `running` when it ends.
- `prepare_extraction(client, ocr, places: &DecodePlaces, path, max_chars, preview_chars, ocr_enabled, refresh)` and the docs level `extract_text(client, ocr, places, ...)` take the places; `core::fs_ops::extract_document` passes `safety.decode_places()` (its own signature unchanged).
- `Extractor::with_places(ocr, places)`; `Extractor::new(ocr)` keeps no ceiling (unit tests); `Extractor::extract(&self, data: &[u8], ..)` unchanged in signature, copies `data` and calls `extract_owned(&self, data: Vec<u8>, ..)`.
- In `extract_owned`, for the six reader suffixes: `let permit = places.acquire().await` (when places are set), then `tokio::task::spawn_blocking(move || { let _permit = permit; clear LAST_OFFICE_HELD and LAST_PDF_HELD; let outcome = reader(&data); (outcome, take both figures) })`; the join error of a panic is resumed on the request side (`std::panic::resume_unwind(e.into_panic())`, today's behaviour until US-0229); after the join, set `LAST_OFFICE_HELD` and `LAST_PDF_HELD` on this thread and let the existing lines move them into the maps keyed by path, with no `.await` in between. `LAST_OFFICE_HELD` becomes `pub(crate)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/safety.rs` | modify |
| `crates/core/src/core/fs_ops.rs` | modify (one argument) |
| `crates/core/src/tools/offload_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] pub(crate) mod offload_e2e;`) |

Read only (imported, never modified): `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/pdf_testkit.rs`, `crates/core/src/tools/tool_surface_e2e.rs (its `setup` is the model, not imported)`.
### Existing patterns
The dispatch and the figure move (`crates/core/src/docs/extract.rs:151-195`); SPEC-0084's carry back (`crates/core/src/tools/archive_single.rs:56-61`, `:294-306`, DEC-006 there); `spawn_blocking` around a reader (`crates/core/src/tools/archive.rs:213-229`); the served app of `crates/core/src/tools/tool_surface_e2e.rs:37-57` (`setup`, `build_with_state`, `create_project`), rebuilt in `offload_e2e.rs`; `McpServer::fs_extract_text` (`crates/core/src/mcp/server.rs:2402`), `fs_list_dir` (`:2070`), `fs_read` (`:1567`); the health route (`crates/core/src/app.rs:133`) through `tower::ServiceExt::oneshot`.
### Fixtures
`offload_e2e.rs` helpers: `served() -> (Router, Arc<AppState>, TempDir)` (keys, config dirs as `tool_surface_e2e::setup`; `atlas` owned by `mia@example.com`, members `noa@example.com`, `ola@example.com`; `/notes/todo.txt` = `buy milk`; `/docs`); `extract(state, person, path) -> Result<Value>` through `McpServer::fs_extract_text` (default options); `health(router) -> Duration`; `probe_loop(kind, stop) -> JoinHandle<Vec<Duration>>` (one probe every 100 ms from 50 ms after start; asserts each answer); `places(state) -> &DecodePlaces`. Documents: the large PDF `pdf_testkit::large_content_pdf(pdf_testkit::STREAM_LIMIT)`; the bomb `pdf_testkit::content_bomb_pdf()`; the large Word document `letters_docx(104_857_600)` and the workbook `repeated_text(b'd', 140)`, copied from `office_bound_e2e.rs:199-209`, `:569-585` with `archive_testkit::{deflate_run, RawPart, zip_raw}`; `/docs/memo.docx` = `archive_testkit::office_zip` of the memo body of `office_bound_e2e.rs:997-999`; `/docs/book.xlsx` (sheet `Data`, A1 `cell`), `/docs/deck.pptx` (slide `Intro`), `/docs/report.pdf` (`pdf_testkit::report_pdf`), `/docs/page.html`, `/docs/data.csv`, `/docs/notes.txt`, `/docs/scan.png` (8 byte PNG signature, OCR disabled) as the spec's E2E-016 states. Every timing test takes `archive_testkit::heavy()` first.
Recorded answers (DEC-009): step 1 runs E2E-016, E2E-024 and E2E-025 on US-0227's code with the expectations printed (`dbg!` of the answer and companion), and the printed values become the literals `RECORDED_*` of `offload_e2e.rs`, committed with this story.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0091/E2E-001: Health check during a large PDF extraction
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-006; **Priority:** Critical
- **Given** the large PDF; one health check answered in time before anything starts
- **When** mia asks for its text and, from 50 ms after, the operator sends a health check every 100 ms until mia's answer arrives
- **Then** every health check is answered healthy in time (today: the first waits about 5.5 seconds)
- **And** mia's answer reports truncated text whose preview after the page heading is only the letter `b`, as in SPEC-0089 E2E-007
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs (probe: `GET /health`)
- Function: `e2e_0091_001_health_check_during_a_large_pdf_extra`
- Red before the change because: on US-0227 code the first health check waits about 5.5 s behind the decoding on the one worker
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-002: Folder listing during a large Word extraction
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Critical
- **Given** the large Word document
- **When** mia asks for its text and noa lists `/notes` every 100 ms from 50 ms after until mia's answer arrives
- **Then** every listing shows `todo.txt` and is answered in time
- **And** mia's answer reports truncated text made of the letter `b`
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs (probe: `McpServer::fs_list_dir` as noa)
- Function: `e2e_0091_002_folder_listing_during_a_large_word_ex`
- Red before the change because: on US-0227 code the listings wait about 0.9 s
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-003: Every place taken, one more waiting, probes still answered
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Critical
- **Given** a server with two request processing threads; three copies of the large PDF, `/docs/a.pdf`, `/docs/b.pdf`, `/docs/c.pdf`
- **When** mia, noa and ola ask for one each at the same moment, and noa reads `/notes/todo.txt` every 100 ms from 50 ms after until all three answers arrive
- **Then** every read returns `buy milk` in time
- **And** the three answers are identical apart from their paths; most at once is 2
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs (probe: `McpServer::fs_read` as noa)
- Function: `e2e_0091_003_every_place_taken_one_more_waiting_pr`
- Red before the change because: on US-0227 code both workers decode and the reads wait seconds
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-004: A PDF refused by its limit while probes run
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** `/docs/bomb.pdf`, the document of SPEC-0089 E2E-SEC-001
- **When** mia asks for its text while the operator sends a health check every 100 ms from 50 ms after
- **Then** mia is refused `ERR_INVALID_ARGUMENT` with `could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** every health check is answered in time; nothing left
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs (probe: `GET /health`)
- Function: `e2e_0091_004_a_pdf_refused_by_its_limit_while_prob`
- Red before the change because: on US-0227 code the health checks wait while the bomb decodes to the limit
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-005: An Excel workbook refused by the content limit while probes run
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** `/docs/long.xlsx`, the workbook of SPEC-0088 E2E-021 (140 rows)
- **When** mia asks for its text while noa lists `/notes` every 100 ms from 50 ms after
- **Then** mia is refused `ERR_INVALID_ARGUMENT` with `could not extract /docs/long.xlsx: the extracted content exceeds 268435456 bytes`
- **And** every listing shows `todo.txt` in time; nothing left
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs (probe: `McpServer::fs_list_dir` as noa)
- Function: `e2e_0091_005_an_excel_workbook_refused_by_the_cont`
- Red before the change because: on US-0227 code the listings wait while the workbook decodes
- Note: DI-8: step 1 measures the decoding time of `repeated_text(b'd', 140)` on US-0227 code; if under 250 ms (test green before the change), raise the rows to a count that refuses with the same message and decodes longer, and say so in the commit body.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-006: Three extractions, two places
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Critical
- **Given** a server with two request processing threads; `/docs/a.pdf`, `/docs/b.pdf`, `/docs/c.pdf` as in E2E-003
- **When** mia, noa and ola ask for one each at the same moment
- **Then** all three succeed with the answer of E2E-001 for their own path
- **And** most at once is 2
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_006_three_extractions_two_places`
- Green on arrival: after step 1 adds the probe counter, two workers already cap the decodings at 2 on US-0227 code; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-007: One place, arrival order
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/docs/a.pdf` and `/docs/b.pdf` as in E2E-003; `/docs/memo.docx` whose text is `# Results` then `Quarterly results`
- **When** mia asks for `/docs/a.pdf`, then 50 ms later noa asks for `/docs/memo.docx`, then 50 ms later ola asks for `/docs/b.pdf`
- **Then** the answers arrive in the order a, memo, b; memo's preview is `# Results\n\nQuarterly results`
- **And** most at once is 1; a health check sent 100 ms after ola's request is answered in time
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs (probe: `GET /health` 100 ms after ola)
- Function: `e2e_0091_007_one_place_arrival_order`
- Red before the change because: on US-0227 code the health check waits for the decodings
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-008: A refused decoding frees its place
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** a server with two request processing threads; `/docs/m.pdf` and `/docs/n.pdf`, two copies of `/docs/bomb.pdf` (E2E-004), and `/docs/memo.docx` (E2E-007)
- **When** mia and noa ask for m and n at the same moment, and ola asks for memo 50 ms later
- **Then** mia and noa are refused with the message of E2E-004 for their paths; ola gets memo's preview `# Results\n\nQuarterly results`
- **And** most at once is 2; nothing left for m and n
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_008_a_refused_decoding_frees_its_place`
- Green on arrival: a refused decoding returns its worker today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-016: Every format answers as recorded
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Critical
- **Given** `/docs/memo.docx` (E2E-007), `/docs/book.xlsx` (one sheet `Data`, cell A1 `cell`), `/docs/deck.pptx` (one slide `Intro`), `/docs/report.pdf` (E2E-009, no fault), `/docs/page.html` (`<h1>Title</h1><p>Body é</p>`), `/docs/data.csv` (`a,b` then `1,2`), `/docs/notes.txt` (`line one`), `/docs/scan.png` with OCR disabled
- **When** mia asks for the text of each
- **Then** each answer equals its recorded answer byte for byte
- **And** each companion written equals its recorded companion byte for byte
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test]` (current thread) / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_016_every_format_answers_as_recorded`
- Green on arrival: recorded on US-0227 code by construction; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-018: An up to date companion answers during a full ceiling
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** a server with two request processing threads; `/docs/memo.docx` already extracted, its companion up to date; `/docs/a.pdf` and `/docs/b.pdf` decoding for noa and ola
- **When** mia asks for memo's text 50 ms after they started
- **Then** mia's answer arrives in time, marked as served from the companion, preview `# Results\n\nQuarterly results`
- **And** most at once stays 2
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_018_an_up_to_date_companion_answers_durin`
- Red before the change because: on US-0227 code both workers decode, so the cached answer waits seconds
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-021: Not a member, during a full ceiling
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** a server with two request processing threads; `/docs/a.pdf` and `/docs/b.pdf` decoding for mia and noa; `eve@example.com` is not a member of `atlas`
- **When** eve asks for the text of `/docs/memo.docx` 50 ms after they started
- **Then** eve is refused in time with `ERR_FORBIDDEN` and a message containing `'eve@example.com' is not a member of 'atlas'`
- **And** most at once stays 2
- Level / Driver / Location: functional e2e / `McpServer::new(state, "eve@example.com").fs_extract_text` (the real handler) on the served app, two workers / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_021_not_a_member_during_a_full_ceiling`
- Red before the change because: on US-0227 code the refusal waits for a worker
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-022: Missing path, during a full ceiling
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** as E2E-021
- **When** ola asks for the text of `/docs/none.pdf`, which does not exist
- **Then** ola is refused in time with `ERR_NOT_FOUND` and `not a file: /docs/none.pdf`
- **And** most at once stays 2
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_022_missing_path_during_a_full_ceiling`
- Red before the change because: on US-0227 code the refusal waits for a worker
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-023: The extraction's own traces are unchanged by the probes
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** E2E-001 just ran
- **When** mia lists `/docs` and reads the audit trail
- **Then** `/docs/large.md` exists and equals the recorded companion of the large PDF byte for byte
- **And** the audit trail holds exactly one extraction line for `/docs/large.pdf`, and noa's and the operator's probes added no line
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, probes as spawned tasks, `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_023_the_extraction_s_own_traces_are_uncha`
- Green on arrival: the companion and the audit line are recorded on US-0227 code; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-024: Three extractions, three companions, three charges
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** Medium
- **Given** E2E-006 just ran
- **When** each member lists `/docs` and reads the audit trail
- **Then** `/docs/a.md`, `/docs/b.md`, `/docs/c.md` exist, each equal to the recorded companion of the large PDF
- **And** one audit line per document, each in its requester's name; each member's write allowance was charged exactly the recorded charge of one extraction
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` / crates/core/src/tools/offload_e2e.rs
- Function: `e2e_0091_024_three_extractions_three_companions_th`
- Green on arrival: recorded on US-0227 code; then the mutation below
- Note: runs E2E-006's scenario then reads the listing, the audit trail and each member's `session_remaining`.
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0091/E2E-025: Companion, artifacts and charge as recorded
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/docs/book.xlsx` of E2E-016, extracted by mia
- **When** mia lists the table artifacts of `/docs/book.xlsx` and checks her remaining write allowance
- **Then** the table list equals the recorded one (one table, sheet `Data`)
- **And** the remaining allowance equals the recorded remaining allowance
- Level / Driver / Location: functional e2e / `McpServer::fs_extract_text` on the served app, `#[tokio::test]` (current thread) / crates/core/src/tools/offload_e2e.rs and `McpServer::fs_list_tables`
- Function: `e2e_0091_025_companion_artifacts_and_charge_as_rec`
- Green on arrival: recorded on US-0227 code; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-006 | build the semaphore with `num_workers() + 1` permits: three decode at once, `most_at_once()` is 3, the test fails |
| E2E-008 | hold the permit in the request future and leak it on a refusal (`std::mem::forget`): memo never gets a place and the test hangs (run the mutation under `timeout 120`) |
| E2E-016 | in `html`, drop the `# ` heading marker: the HTML answer differs from its record, the test fails |
| E2E-023 | record the extraction audit line twice in `core::fs_ops::extract_document`: two lines, the test fails |
| E2E-024 | charge the companion twice in `core::fs_ops::land_conversion`: the remaining allowance differs from its record, the test fails |
| E2E-025 | name the first table of a workbook `Sheet` instead of its sheet name in `xlsx`: the table list differs, the test fails |

## Constraints
- Files not to touch (E2E-017): `crates/core/src/tools/office_bound_e2e.rs`, `crates/core/src/tools/pdf_bound_e2e.rs`, `crates/core/src/tools/archive_testkit.rs`, `crates/core/src/tools/pdf_testkit.rs`, the `#[cfg(test)] mod tests` of `crates/core/src/docs/extract.rs`; also `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Never hold a `std::sync::Mutex` guard across an `.await`; the probe mutex is locked only inside the closure or after the join
- The permit is acquired before `spawn_blocking` and moved into the closure (DEC-003); never acquire inside the closure
- No dash as punctuation in code, comments or docs
- Scope boundary: the unexpected stop answer and the abandon tests are US-0229; the HTTP door and docs US-0230

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-007, E2E-018, E2E-021, E2E-022 (step 1: the recorded literals, `served`, probes, the probe counter around the reader on US-0227 code, then the tests; the probe tests and E2E-018, E2E-021, E2E-022 fail by their time assertions; step 2: places and `spawn_blocking`).
- [ ] Green on arrival tests (E2E-006, E2E-008, E2E-016, E2E-023, E2E-024, E2E-025): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 48 `e2e_0088_` tests, the 50 `e2e_0089_` PDF tests and the 51 extraction unit tests of `docs::extract`, unmodified (`env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0088_ e2e_0089_ docs::extract --test-threads=1`), with every decoded bytes and content held figure they assert; every `e2e_0092_` test; every `api::dataplane` extraction test
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0228: decode documents apart from the request threads, behind today's ceiling` on its story branch `story/US-0228-decode-documents-apart-from-the-request`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0091-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0091` and the requirement ids (SPEC-0091/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-006).
