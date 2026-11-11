# SPEC-0091: Keep requests answered while documents are extracted, design

> Spec: sdlc/specs/SPEC-0091-keep-requests-answered-while-documents-are-extracted/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-11
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Decoding off the request threads | `tokio::task::spawn_blocking` | tokio 1.54.1 (`Cargo.lock:5973`), features `rt-multi-thread` (`Cargo.toml:14`) | DEC-002 |
| Ceiling | `tokio::sync::Semaphore` (fair: permits are granted in request order), `acquire_owned` | same | DEC-001, DEC-003 |
| Number of request processing threads | `tokio::runtime::Handle::current().metrics().num_workers()` (stable API, `tokio-1.54.1/src/runtime/metrics/runtime.rs:48`) | same | DEC-001 |
| Probes in tests | the real router of `crate::app::build_with_state` (`/health`), the MCP methods `McpServer::fs_list_dir`, `fs_read`, `fs_extract_text` | in tree | DEC-008 |

Dependency statement: no new crate, no feature change, no `unsafe`.

Skills loaded: `rust` (CPU bound work through `spawn_blocking`, never block an async worker; `tokio::sync`
primitives; no `std::sync::Mutex` held across `.await`; typed `ToolError`; clippy and fmt gates).

## 2. Approaches

### Chosen: the six readers run on the blocking pool, behind a ceiling of places

`Extractor::extract` (`crates/core/src/docs/extract.rs:151`) dispatches the six built in readers as plain
synchronous calls (`:166-171`). They move into one `spawn_blocking` closure that owns the document bytes and
the place it took from a per server `DecodePlaces` (a fair semaphore of `num_workers()` permits). The request
side awaits the join handle; the OCR arm, text decode and every refusal before decoding stay where they are.
Pros: request threads only await; today's ceiling is kept (as many decodings as request processing threads);
the owned permit inside the closure frees the place exactly when the decoding ends, abandoned or not. Cons:
one copy of the document bytes (`Vec` moved into the closure, already owned by `prepare_extraction`,
`extract.rs:376`); the test only figures written on the decoding thread must be carried back (DEC-004).

### Rejected: `tokio::task::block_in_place` (DEC-002)

Keeps the decoding on the worker but hands its other tasks to a new worker. Panics on a current thread
runtime (the whole test suite's `#[tokio::test]`, DI-3) and still occupies a request processing thread.

### Rejected: a dedicated decoding thread pool (DEC-002)

A pool of N threads with a channel: duplicates what the blocking pool and a semaphore give, and needs its own
shutdown. Nothing in the spec asks for it.

### Rejected: no ceiling (DEC-001)

The blocking pool grows to 512 threads; memory held by concurrent decodings could exceed today's (spec D-5).

## 3. Architecture

```
fs.extract_text (mcp/server.rs:2402)          POST /api/fs/{mount}/extract-text (api/dataplane.rs:1487)
        \  membership, normalized path                 /
         +---- core::fs_ops::extract_document(client, safety, ...) (fs_ops.rs:1720)
                 prepare_extraction(client, ocr, safety.decode_places(), ...)       (extract.rs:330)
                   not a file -> ERR_NOT_FOUND                     (no place)
                   up to date companion -> Cached                  (no place)
                   data = client.read_bytes(path)
                   Extractor::with_places(ocr, places).extract_owned(data, path, ...)
                     ".pdf" .docx .pptx(...) .xlsx(...) .html .csv:
                        permit = places.acquire().await            (FIFO wait, droppable)   [US-0228]
                        spawn_blocking(move || { _permit; clear LAST_*; reader(&data);
                                                 take LAST_* })   -> (outcome, figures)     [US-0228]
                        join: Ok -> restore LAST_* on this thread, insert per path (as today)
                              panic -> "the document reader stopped unexpectedly"           [US-0229]
                     images -> OCR (async, no place); text -> decode (no place)
                   reader error -> "could not extract <path>: <reason>" (ERR_INVALID_ARGUMENT, unchanged)
                   stop -> "could not extract <path>: the document reader stopped unexpectedly"
                           (ERR_INTERNAL_ERROR)                                             [US-0229]
```

`DecodePlaces` is owned by `SafetyManager` (`crates/core/src/safety.rs:31-40`), the one per server object the
engine functions already receive (`fs_ops::extract_document` takes `safety`), which avoids changing
`extract_document`'s signature (called unchanged by `archive_testkit::extract_text`, a SPEC-0088 file that
E2E-017 requires untouched).

## 4. Data model

Nothing persisted. In memory:

- `pub(crate) struct DecodePlaces { semaphore: OnceLock<Arc<Semaphore>>, #[cfg(test)] probe: std::sync::Mutex<PlacesProbe> }`
  in `extract.rs`; the semaphore is created at the first acquire with `num_workers()` permits (at least 1).
- `#[cfg(test)] struct PlacesProbe { running: usize, most: usize, started: Vec<String>, faults: HashSet<String> }`:
  decodings running now, the most at once, the paths whose decoding started (in order), the paths with a test
  fault.
- `#[cfg(test)] struct Figures { office: Option<OfficeHeld>, pdf: Option<PdfHeld> }`, returned by the closure.

## 5. Contracts and interfaces

- `pub(crate) fn SafetyManager::decode_places(&self) -> &DecodePlaces`.
- `pub async fn prepare_extraction(client, ocr, places: &DecodePlaces, path, max_chars, preview_chars,
  ocr_enabled, refresh)`; `pub async fn extract_text(client, ocr, places: &DecodePlaces, ...)` (the docs
  level helper; no production caller, `rg` finds none).
- `Extractor::new(ocr)` (unit tests, no ceiling) and `Extractor::with_places(ocr, places)` (production);
  `Extractor::extract(&self, data: &[u8], ...)` keeps its signature (the 51 unit tests call it unmodified) and
  copies `data` once; `Extractor::extract_owned(&self, data: Vec<u8>, ...)` is what `prepare_extraction` calls.
- `DecodePlaces::acquire(&self) -> OwnedSemaphorePermit` (async); test only: `most_at_once()`, `started()`,
  `inject_fault(path)`, `clear_faults()`.
- Unexpected stop: `ToolError::internal(format!("could not extract {path}: the document reader stopped
  unexpectedly"))`, through the existing error paths: MCP `CallToolResult::error` and one ERROR log line
  (`mcp/server.rs:46-62`, `logging.rs:79-85`), REST status 500 with `{"error": "ERR_INTERNAL_ERROR", "detail":
  "ERR_INTERNAL_ERROR: could not extract ..."}` (`api/dataplane.rs:259-266`).
- Every other answer, limit, message and code: unchanged (FR-NEW-003).

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/docs/extract.rs` | modify: `DecodePlaces`; place around the six readers; `spawn_blocking` and the figures carried back; stop mapping and test fault | FR-NEW-001 to FR-NEW-005 | US-0228, US-0229 |
| `crates/core/src/safety.rs` | modify: `decode_places` field and accessor | FR-NEW-002 | US-0228 |
| `crates/core/src/core/fs_ops.rs` | modify: pass `safety.decode_places()` to `prepare_extraction` | FR-NEW-002 | US-0228 |
| `crates/core/src/tools/offload_e2e.rs` | create: the `e2e_0091_*` tests, the served app helper, probes, recorded answers | every requirement | US-0228 to US-0229 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] pub(crate) mod offload_e2e;` | test layout | US-0228 |
| `crates/core/src/api/dataplane.rs` | modify (tests only): `e2e_0091_011`, `019`, `020`, `027` | FR-NEW-006 | US-0230 |
| `.agent_docs/architecture.md` | modify: where extraction decodes, the ceiling, the stop | DEC-010 | US-0229 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing (DEC-010) | US-0230 |

Not modified (E2E-017): `tools/office_bound_e2e.rs`, `tools/pdf_bound_e2e.rs`, `tools/archive_testkit.rs`,
`tools/pdf_testkit.rs`, the tests of `docs/extract.rs` (they call `Extractor::new(&null).extract(&data, ..)`,
`extract.rs:1658-1689`, unchanged signature). They are imported read only.

Affected existing tests, kept unchanged and green: the 48 `e2e_0088_` tests (including the three at once,
`office_bound_e2e.rs:984-1008`, which now wait for the one place of a current thread runtime in turn), the 50
`e2e_0089_` PDF tests, the 51 extraction unit tests, every `api::dataplane` extraction test.

## 7. Compatibility and risks

- Observable change: other requests no longer wait behind a decoding (FR-NEW-001); a reader panic now answers
  `ERR_INTERNAL_ERROR` (FR-NEW-004) instead of ending the request's processing.
- The ceiling equals today's implicit one, so peak memory of concurrent decodings does not grow (spec D-5).
- Cost: one copy of the document bytes per decoding (the reader borrowed them before); spec 7.1 allows 10%.
- A decoding abandoned by its requester runs to its end and keeps its place (FR-NEW-005), as the readers have no
  stopping point.
- Shared ground with SPEC-0092: its child tests run the attacked call on a 2 MiB stack; after US-0228 the reader runs on
  a blocking thread, whose default stack is the same 2 MiB (tokio sets none, std's default applies); SPEC-0092
  bounds every nesting before the reader runs, so its tests stay green.
- Ordering between stories (each lands on main alone, repo practice): the ceiling and the move to the blocking
  pool land in one story (US-0228): a ceiling alone is invisible while the reader still holds a request
  processing thread (that thread is today's ceiling), and the move alone would decode without a ceiling on
  main. A reader panic keeps today's behaviour in US-0228 (`resume_unwind`) until US-0229 maps it.
- Rollback: revert the commits; no data or contract change.

## 8. Security design

Not applicable: the spec reads `Security: n/a` and no `--security` was given. Membership is still checked
before a place is taken (spec 7.2, E2E-021).

## 9. Test strategy

Functional e2e on the served app: `served(workers)` builds a config like `tool_surface_e2e::setup`
(`tools/tool_surface_e2e.rs:37-50`), calls `crate::app::build_with_state`, creates `atlas` owned by
`mia@example.com` with members `noa@example.com` and `ola@example.com`, writes `/notes/todo.txt` (`buy milk`)
and an empty `/docs`. Extractions go through `McpServer::fs_extract_text` (the assistant tool's handler), probes
through the router (`GET /health`) and `McpServer::fs_list_dir`, `fs_read`. Timing tests run on
`#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` (or 2 when the test says so), take
`archive_testkit::heavy()` first, and time each probe with `Instant` from just before it is sent to its answer.
The HTTP door runs in the `api/dataplane.rs` tests (`Harness`); E2E-027 serves the router on a real loopback
socket (`axum::serve`) and closes a raw `TcpStream` after 1 s. Names `e2e_0091_<id>_<title in snake case>`,
cut at 50 characters; filter `e2e_0091_`.

| Test | Level | Driver | Location | Workers | Story |
|---|---|---|---|---|---|
| E2E-001 | functional e2e | MCP method + router `/health` | offload_e2e.rs | 1 | US-0228 |
| E2E-002 | functional e2e | MCP methods | offload_e2e.rs | 1 | US-0228 |
| E2E-003 | functional e2e | MCP methods | offload_e2e.rs | 2 | US-0228 |
| E2E-004 | functional e2e | MCP method + router `/health` | offload_e2e.rs | 1 | US-0228 |
| E2E-005 | functional e2e | MCP methods | offload_e2e.rs | 1 | US-0228 |
| E2E-006 | functional e2e | MCP method | offload_e2e.rs | 2 | US-0228 |
| E2E-007 | functional e2e | MCP method + router `/health` | offload_e2e.rs | 1 | US-0228 |
| E2E-008 | functional e2e | MCP method | offload_e2e.rs | 2 | US-0228 |
| E2E-009 | functional e2e | MCP method | offload_e2e.rs | current thread | US-0229 |
| E2E-010 | functional e2e | MCP method under the log capture, router `/health` | offload_e2e.rs | current thread | US-0229 |
| E2E-011 | functional e2e | HTTP door | api/dataplane.rs tests | current thread | US-0230 |
| E2E-012 | functional e2e | MCP methods | offload_e2e.rs | 2 | US-0229 |
| E2E-013 | functional e2e | MCP method, task aborted | offload_e2e.rs | 1 | US-0229 |
| E2E-014 | functional e2e | MCP methods, task aborted | offload_e2e.rs | 1 | US-0229 |
| E2E-015 | functional e2e | MCP method | offload_e2e.rs | 1 | US-0229 |
| E2E-016 | functional e2e | MCP method, recorded answers | offload_e2e.rs | current thread | US-0228 |
| E2E-017 | functional e2e | the existing suites | office_bound_e2e.rs, pdf_bound_e2e.rs, extract.rs | as they are | US-0230 |
| E2E-018 | functional e2e | MCP method | offload_e2e.rs | 2 | US-0228 |
| E2E-019 | functional e2e | HTTP door | api/dataplane.rs tests | 1 | US-0230 |
| E2E-020 | functional e2e | HTTP door | api/dataplane.rs tests | current thread | US-0230 |
| E2E-021 | functional e2e | MCP method as eve | offload_e2e.rs | 2 | US-0228 |
| E2E-022 | functional e2e | MCP method | offload_e2e.rs | 2 | US-0228 |
| E2E-023 | functional e2e | MCP methods, recorded companion | offload_e2e.rs | 1 | US-0228 |
| E2E-024 | functional e2e | MCP methods, recorded companion and charge | offload_e2e.rs | 2 | US-0228 |
| E2E-025 | functional e2e | MCP methods, recorded tables and allowance | offload_e2e.rs | current thread | US-0228 |
| E2E-026 | functional e2e | MCP method | offload_e2e.rs | current thread | US-0229 |
| E2E-027 | functional e2e | HTTP door on a real socket, closed | api/dataplane.rs tests | 1 | US-0230 |
| E2E-028 | functional e2e | MCP methods, task aborted | offload_e2e.rs | 1 | US-0229 |

Test data: the large PDF is `pdf_testkit::large_content_pdf(STREAM_LIMIT)` as SPEC-0089 E2E-007
(`pdf_bound_e2e.rs:87-90`, read only use); the bomb is SPEC-0089 E2E-SEC-001's `pdf_testkit::content_bomb_pdf`;
`/docs/report.pdf` is `pdf_testkit::report_pdf`; the large Word document and the workbook of E2E-005 are
rebuilt in `offload_e2e.rs` as `letters_docx(PART_LIMIT)` and `repeated_text(b'd', 140)` are built in
`office_bound_e2e.rs:199-209`, `:569-585` (private there, so copied from `archive_testkit::deflate_run`,
`RawPart` and `zip_raw`, never imported by editing that file); the others are small literal documents built by
`archive_testkit::office_zip`. The recorded answers (E2E-016, 023, 024, 025, 026) are literals in
`offload_e2e.rs`, captured on the code before this spec (DEC-009).

## 10. Decisions log

- **DEC-001:** the ceiling is a fair `tokio::sync::Semaphore` held by `DecodePlaces`, created at the first
  acquire with `Handle::current().metrics().num_workers().max(1)` permits: the runtime's request processing
  threads (honouring `TOKIO_WORKER_THREADS`), 1 on a current thread test runtime. **Obliged by:**
  SPEC-0091/FR-NEW-002 (ceiling = request processing threads, arrival order), DI-3, DI-4. **Rationale:**
  tokio's semaphore grants permits in request order, which is the arrival order the spec asks; the count is
  read from the runtime that serves the requests, so a 1 or 2 worker test runtime gives 1 or 2 places with no
  configuration. **Alternatives considered:** a configured count (spec 3.2 rejects); `available_parallelism`
  (ignores `TOKIO_WORKER_THREADS` and the test runtimes). **Source:** library docs. **Evidence:**
  `tokio-1.54.1/src/runtime/metrics/runtime.rs:48`; `crates/mcp-fs/src/main.rs:6-8`.
- **DEC-002:** the six readers (`.pdf`, `.docx`, the four PowerPoint suffixes, `.xlsx`, `.xlsm`, `.html`,
  `.htm`, `.csv`, as dispatched at `extract.rs:165-171`) run inside one `tokio::task::spawn_blocking` closure
  that owns the bytes (`Vec<u8>`) and the permit; images, fenced and text suffixes and the audio and video
  refusal stay on the request side and take no place. **Obliged by:** SPEC-0091/FR-NEW-001, FR-NEW-002,
  spec 3.3. **Rationale:** the blocking pool exists on every runtime flavour; the request thread only awaits.
  **Alternatives considered:** `block_in_place` (panics on a current thread runtime); a private pool.
  **Source:** design, rust skill. **Evidence:** `extract.rs:151-190`.
- **DEC-003:** the permit is acquired on the request side before `spawn_blocking` and moved into the closure,
  dropped when the closure returns: a waiting extraction whose future is dropped never starts, a started one
  keeps its place until its decoding ends, success, refusal or panic alike. **Obliged by:**
  SPEC-0091/FR-NEW-002, FR-NEW-005. **Rationale:** dropping the future cancels the acquire; the closure owns
  the release. **Alternatives considered:** holding the permit in the async future (released on abandon while
  the decoding still runs: E2E-014 fails); acquiring inside the closure with `blocking_acquire` (an abandoned
  waiter would still start: E2E-028 fails). **Source:** design. **Evidence:** spec FR-NEW-005.
- **DEC-004:** the test only figures are carried back as SPEC-0084 DEC-006 does: the closure clears
  `LAST_OFFICE_HELD` and `LAST_PDF_HELD` on its own thread first, runs the reader, then takes both and returns
  them with the outcome; after the join the request side sets them on its own thread, and the existing code
  (`extract.rs:180-190`) moves them into `OFFICE_HELD` and `PDF_HELD` keyed by path, with no `.await` between.
  `LAST_OFFICE_HELD` becomes `pub(crate)` for that. The log lines stay emitted by the request side
  (`mcp/server.rs:57`, `api/dataplane.rs:264`), never by the decoding thread (DI-2). **Obliged by:**
  SPEC-0091/FR-NEW-003 (E2E-017), spec 3.3, DI-1, DI-2. **Rationale:** the SPEC-0088 and SPEC-0089 suites read
  the figures on the thread that awaited the call; a pooled blocking thread must not leak a stale figure to
  the next decoding. **Alternatives considered:** a shared map keyed by path (concurrent tests in other
  threads collide on equal paths). **Source:** existing code. **Evidence:** `extract.rs:575-605`, `:180-190`;
  `tools/archive_single.rs:56-61`, `:294-306`.
- **DEC-005:** `DecodePlaces` lives in `SafetyManager`, created in `SafetyManager::new`; `fs_ops` passes
  `safety.decode_places()` to `prepare_extraction`. **Obliged by:** SPEC-0091/FR-NEW-002, FR-NEW-003 (E2E-017:
  `extract_document`'s signature, used by `archive_testkit.rs`, must not change). **Rationale:** the manager
  is the composition wide object the engine already receives (its `limits` field is the precedent,
  `safety.rs:34-39`); per server state means parallel tests never share places. **Alternatives considered:**
  a process global semaphore (parallel tests would share and distort `most at once`); a new argument to
  `extract_document`. **Source:** existing code. **Evidence:** `safety.rs:31-40`; `fs_ops.rs:1720-1748`;
  `archive_testkit.rs:487-506`.
- **DEC-006:** a panic of the decoding closure (a `JoinError` that `is_panic()`) is answered
  `ToolError::internal("could not extract <path>: the document reader stopped unexpectedly")` by
  `prepare_extraction`, which passes this error through instead of re wrapping it as
  `ERR_INVALID_ARGUMENT`; the closure's place is already free (DEC-003); nothing was written. A
  `JoinError` that is a cancellation (runtime shutdown) is answered the same way. Until US-0229, US-0228 resumes
  the panic on the request side (`std::panic::resume_unwind`), today's behaviour. The PDF reader's own
  `catch_unwind` (`extract.rs:519`) is unchanged: a panic it contains stays `pdf parser failed on this
  document`. **Obliged by:** SPEC-0091/FR-NEW-004, DI-6. **Rationale:** the message is worded once, at the
  place that knows the path. **Alternatives considered:** `catch_unwind` around every reader (changes the
  Office readers' panic behaviour into this refusal anyway, and needs `AssertUnwindSafe` on every reader).
  **Source:** design. **Evidence:** `extract.rs:378-387`; `tools/archive.rs:229`.
- **DEC-007:** the test fault is per server: `DecodePlaces::inject_fault(path)` (test only) makes the decoding
  closure for that path panic before calling the reader; `clear_faults()` removes it (E2E-026). **Obliged by:**
  SPEC-0091/FR-NEW-004 (E2E-009, "the operator's test fault"), DI-6. **Rationale:** the closure runs on a
  pooled thread, so a thread local set by the test is invisible there; a per server set keeps parallel tests
  apart. **Alternatives considered:** a global keyed by path (E2E-016 uses the same path without a fault in
  another test). **Source:** design. **Evidence:** spec E2E-009, E2E-026.
- **DEC-008:** the probes: the operator's health check is `GET /health` on the router built with the state;
  noa's listing is `McpServer::fs_list_dir` of `/notes`; noa's read is `McpServer::fs_read` of
  `/notes/todo.txt`. A probe loop runs as a spawned task sending one probe every 100 ms from 50 ms after the
  extraction started until a `watch` channel says stop, recording each answer time. "Abandon" is aborting the
  task that awaits `McpServer::fs_extract_text` (the transport drops that future when a client cancels), and,
  at the HTTP door, closing the client socket of a served router (hyper drops the handler future when it reads
  the end of the connection). **Obliged by:** SPEC-0091/FR-NEW-001, FR-NEW-005, FR-NEW-006. **Rationale:**
  real handlers, no stand in. **Alternatives considered:** `tokio::time::timeout` around the call (equivalent
  drop, less explicit). **Source:** design. **Evidence:** `app.rs:133`; `mcp/server.rs:1567`, `:2070`, `:2402`.
- **DEC-009:** "recorded answers" are captured on the code before this spec: the first story writes the tests
  E2E-016, E2E-024 and E2E-025 with their expectations printed by a first run on its base commit (US-0227's
  code), pasted as literals into `offload_e2e.rs`, and committed with that story; later stories never edit
  them. A field that differs between two record runs on the base code would be excluded and named in the
  story (none expected: the extraction answer holds no time). **Obliged by:** SPEC-0091/FR-NEW-003 (recorded
  answer, glossary). **Rationale:** byte for byte against the old code, with no committed fixture file to
  regenerate. **Alternatives considered:** a JSON golden file with a rewrite switch (one more file, a switch
  that could silently re record). **Source:** spec glossary. **Evidence:** spec 9 header.
- **DEC-010:** documentation: `.agent_docs/architecture.md` (US-0229) states where decoding runs, the ceiling and
  the stop; the closing story updates `README.md`, `AGENTS.md` (`docs/extract.rs` bullet), `.agent_docs/tools.md`
  (`fs.extract_text`: other requests unaffected, the stop message) and `.agent_docs/testing.md` (filter
  `e2e_0091_`, the multi thread timing tests and `heavy()`). No tool contract change. **Obliged by:** n/a (no
  code impact): project documentation rule. **Rationale:** house closing pattern. **Alternatives considered:**
  none. **Source:** constitution. **Evidence:** `AGENTS.md`.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0228 | decode documents apart from the request threads, behind today's ceiling | P1 | SPEC-0091/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-006 | US-0227 | 2 | no |
| US-0229 | a reader that stops and a requester who gives up | P1 | SPEC-0091/FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005 | US-0228 | 2 | no |
| US-0230 | extraction latency through the HTTP door and documentation | P2 | SPEC-0091/FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006 | US-0229 | 2 | yes |
