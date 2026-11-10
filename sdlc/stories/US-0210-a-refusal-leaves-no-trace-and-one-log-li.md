# US-0210: a refusal leaves no trace and one log line

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0209
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
A refused extraction leaves the project exactly as it was (stale or up to date companion kept byte for byte, no artifact, nothing charged, no audit line), whatever the suffix's letter case, on repeat and with a refresh, and the operator finds one informational log line per refused call. Three concurrent calls stay within their own bound.

## Requirements covered
- SPEC-0088/FR-NEW-007: THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-002 to FR-NEW-005.
- SPEC-0088/FR-NEW-002 (partial: upper case suffix and the four other suffixes): IF a part about to be read declares more than 104 857 600 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: document part '<part>' is <D> bytes once decoded, over the limit of 104857600 bytes`, before decoding that part.
- SPEC-0088/FR-NEW-006 (partial: two calls at once each within the bound): THE document text extraction SHALL hold, during one call, at most 104 857 600 plus 65 536 bytes of any one decoded part, at most 268 435 456 plus 65 536 bytes of decoded part data in total, and at most 373 293 056 bytes of extracted content.
- SPEC-0088/FR-NEW-011 (partial: a normal document beside two refused ones): THE document text extraction SHALL answer a Word, Excel or PowerPoint document within every limit with the same text, metadata, tables, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading any part when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_030 e2e_0088_031 e2e_0088_032 e2e_0088_033 e2e_0088_034 e2e_0088_035 e2e_0088_036 rest_refusal_logs_one_info_line`, then `cargo test -p mcp-fs-core --lib logging tools::archive`: every test passes. Real assertions: the companion bytes, the artifact lists, the audit trail, a following 52 428 800 byte write, and the captured log events with their level.

## Design excerpt
### Decisions
- **DEC-008 (SPEC-0088):** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>` (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens the package (header check included), calls `read` and, on every outcome, stores `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it, with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given. `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed 65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper bound of what is held at once; `content` is the counter's final value (it only grows). A call that reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0. The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006, NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent calls; no production signature changes. **Alternatives considered:** reusing the single slot `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006. **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
- **DEC-010 (SPEC-0088):** test layout: `crates/core/src/tools/office_bound_e2e.rs` (`#[cfg(test)] mod office_bound_e2e;` in `tools/mod.rs`), every test `#[tokio::test]` (single threaded) through `fs_ops::extract_document` after `f.state.authorize(MOUNT, person)`, except E2E-035 (a plain `#[test]` running a dedicated current thread runtime under `crate::logging::capture::lock_for_test`, calling `McpServer::new(state, person).fs_extract_text(Parameters(..))`, the model being `crates/core/src/tools/archive.rs:1862-1865`) and the three HTTP tests in `api/dataplane.rs`. Names `e2e_0088_<id>_<snake title>` cut at 50 characters. No cargo feature. **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0085 DEC-011 and SPEC-0087 DEC-008 conventions; `./test.sh` runs the default build first (`test.sh:5`). **Alternatives considered:** tests inside `docs/extract.rs` (no access to the project, quota and audit channels). **Source:** existing code. **Evidence:** `test.sh:5-7`, `crates/core/src/tools/mod.rs:70-110`.
- **DEC-012 (SPEC-0088):** the refusal log line: `to_call_result` (`crates/core/src/mcp/server.rs:46-58`) calls `crate::logging::log_tool_failure(tool, &e)` before building the error result; `error_response` (`crates/core/src/api/dataplane.rs:259-264`) calls `crate::logging::log_tool_failure("rest", err)`. The test capture records each event's level as the field `level` (`logging.rs`, test module only). **Obliged by:** SPEC-0088/FR-NEW-007 business rule ("logged once at the informational level"), FR-NEW-010 (same effects at both doors), NFR 7.6. **Rationale:** mismatch found while designing: spec 2.1 row 8 and `.agent_docs/architecture.md:349-357` say a 4xx is logged once at INFO, but `log_tool_failure` has no production caller (`rg -n 'log_tool_failure' crates/core/src` finds only `logging.rs`), so E2E-035 fails today; the requirement is satisfied by wiring the documented classifier at the two doors, not by a log line in the extractor (which would log twice once the doors log). **Alternatives considered:** an `info!` inside `prepare_extraction` (extraction only, other tools stay silent, contradicts the documented split). **Source:** existing code. **Evidence:** as cited.
- **DEC-014 (SPEC-0088):** tests whose rule an earlier story or today's code already holds are green on arrival and each is proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent SPEC-0085 DEC-013. **Source:** design. **Evidence:** US-0189 to US-0193.
- **DEC-015 (SPEC-0088):** documentation: US-0210 updates `.agent_docs/architecture.md` (the error logging split is now wired at both doors; a dated "Office extraction bound" paragraph: the three limits, the size lie, reading order); the closing story US-0211 updates `README.md` (Office extraction limits), `AGENTS.md` (`docs/extract.rs` bullet: `OfficeBudget`, the three limits), `.agent_docs/tools.md` (`:336` gap closed, `fs.extract_text` limits and messages, dated SPEC-0088 paragraph) and `.agent_docs/testing.md` (filter `e2e_0088_`, `heavy()`, `deflate_run`, `bomb_part`, the hook). **Obliged by:** n/a (no code impact): project convention (`AGENTS.md`, spec 14.3 docs list). **Rationale:** `.agent_docs/tools.md:336` names this gap as remaining. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** as cited.
### Data model
None.
### Contracts
`crates/core/src/mcp/server.rs` `to_call_result`: on `Err(e)` call `crate::logging::log_tool_failure(tool, &e)` then build the error result as today. `crates/core/src/api/dataplane.rs` `error_response`: call `crate::logging::log_tool_failure("rest", err)` first. `crates/core/src/logging.rs` `capture` (test module): `on_event` inserts the field `level` (`event.metadata().level().to_string()`, `INFO`, `ERROR`). Unit test `rest_refusal_logs_one_info_line` in the `api/dataplane.rs` tests: under `lock_for_test`, a `POST extract-text` on a missing path leaves exactly one captured event with `level` `INFO` whose `message` contains `ERR_NOT_FOUND`. `.agent_docs/architecture.md`: the error logging split section says where it is wired (`to_call_result`, `error_response`); a dated (2026-11-10, SPEC-0088) paragraph on the Office extraction bound (the three limits, the size lie, the reading order, the hook).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/mcp/server.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify |
| `crates/core/src/logging.rs` | modify (test module only) |
| `crates/core/src/tools/office_bound_e2e.rs` | modify |
| `.agent_docs/architecture.md` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008, extended by US-0205 per DEC-011): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `SECOND` (`noa@example.com`, `add_second_member`), `THIRD` (`ola@example.com`, `add_third_member`), `office_fixture()` (folder `/docs`), `office_fixture_allowance(n)`, `seed(f, path, bytes)`, `read`, `exists`, `extract_text(f, person, path, refresh)` (authorize, then `fs_ops::extract_document(.., 200_000 max chars, 4_000 preview chars, ocr true, refresh)`), `office_zip(&[(name, bytes)])`, `zip_raw(&[RawPart])`, `zip_declaring(&[(name, declared, content)])`, `deflate_run(prefix, letter, n, suffix)`, `bomb_part(name)`, `padded_part(name, xml_head, total)`, `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when the call read no part), `nothing_left(f, doc, companion_before)`, `heavy()` (take it first in every test that builds or decodes a part of 10 MB or more). Test module models: `crates/core/src/tools/archive_declared_e2e.rs` (layout, `write` after a refusal at `:25`), `crates/core/src/core/fs_ops.rs:4895-5110` (SPEC-0041 extraction tests, `ooxml` builder, `assert_refused`). The readers: `docx` (`crates/core/src/docs/extract.rs:714-835`), `pptx` (`:866-943`), `xlsx` (`:985-1127`), `entry_text` (`:563-568`), the wrapper adding `could not extract <path>: ` (`:366-374`). Log capture: `crate::logging::capture` (`crates/core/src/logging.rs:100-215`), `with_capture_lock` and its tests (`crates/core/src/tools/archive.rs:1856-1945`); the classifier `log_tool_failure` (`logging.rs:79-85`) and its tests (`:337-350`); the assistant tool method `fs_extract_text` (`crates/core/src/mcp/server.rs:2396-2422`) and `to_call_result` (`:46-58`); `McpServer::new` in tests (`crates/core/src/tools/case_paths_e2e.rs:65`).
### Fixtures
Payloads: `bomb_part("word/document.xml")` documents; E2E-030 `/docs/report.md` (`old text`) written before `/docs/report.docx`; E2E-034 a bomb sheet `xl/worksheets/sheet1.xml` in `/docs/b.xlsm` and a bomb `ppt/slides/slide1.xml` in `/docs/b.pptm`, `/docs/b.potx`, `/docs/b.ppsx`; E2E-036 `add_second_member` and `add_third_member`, three calls joined with `tokio::join!` in one runtime, each read back through `parts_held(<its path>)`. Take `heavy()` first in every test of this story except the HTTP unit test.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-030: A refusal leaves a stale companion and the allowance untouched
- **Category:** side effect; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/docs/report.md` holding `old text`, then `/docs/report.docx` written after it as a bomb part document
- **When** the member asks for the text of `/docs/report.docx`
- **Then** the refusal is the part limit message for `/docs/report.docx`
- **And** `/docs/report.md` still reads `old text`; no artifact is listed for `/docs/report.docx`; a following write of a 52 428 800 byte file succeeds
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_030_a_refusal_leaves_a_stale_companion_an`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-031: Refresh on a bomb with an up to date companion
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007, FR-NEW-011; **Priority:** High
- **Given** the situation of E2E-005
- **When** the member asks for the text of `/docs/bomb.docx` with refresh
- **Then** the refusal is the part limit message for `/docs/bomb.docx`
- **And** `/docs/bomb.md` still reads `kept text`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_031_refresh_on_a_bomb_with_an_up_to_date`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-032: Repeating a refused call
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/bomb.docx`, a bomb part document
- **When** the member asks for its text twice
- **Then** both answers are the same refusal; nothing left after the second
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_032_repeating_a_refused_call`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-033: Upper case suffix
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/docs/BOMB.DOCX`, a bomb part document
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `/docs/BOMB.DOCX` and `word/document.xml`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_033_upper_case_suffix`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-034: The four other suffixes
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/docs/b.xlsm` with a bomb sheet, `/docs/b.pptm`, `/docs/b.potx` and `/docs/b.ppsx` each with a bomb slide 1
- **When** the member asks for the text of each
- **Then** each is refused with the part limit message naming its path and `xl/worksheets/sheet1.xml` or `ppt/slides/slide1.xml`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_034_the_four_other_suffixes`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-035: What the operator sees for a refusal
- **Category:** side effect; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/bomb.docx`, a bomb part document
- **When** the member asks for its text
- **Then** the server log has exactly one informational line for the call naming `ERR_INVALID_ARGUMENT` and `document part 'word/document.xml'`, and no error level line
- **And** the audit trail has no line for `/docs/bomb.docx` or `/docs/bomb.md`
- Level / Driver / Location: functional e2e / assistant tool method `McpServer::new(state, OWNER).fs_extract_text(Parameters(ExtractTextArgs { .. }))` on a dedicated current thread runtime under `crate::logging::capture::lock_for_test` (model: `crates/core/src/tools/archive.rs:1862-1865`) / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_035_what_the_operator_sees_for_a_refusal`
- Red before the change because: no log line at all: `log_tool_failure` has no production caller (`rg -n 'log_tool_failure' crates/core/src` finds only `logging.rs`); step 1 adds the `level` field to the capture so the assertion reads it
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-036: Two bombs and a normal document at once
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006, FR-NEW-011; **Priority:** Medium
- **Given** members `mia@example.com` and `noa@example.com`, each with a bomb part document `/docs/m.docx` and `/docs/n.docx`, and `ola@example.com` with `/docs/memo.docx` of E2E-001
- **When** the three ask for their texts at the same time
- **Then** the two bombs are refused with the part limit message and the memo succeeds as in E2E-001
- **And** each bomb call's decoded bytes held are 0
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::extract_document` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/office_bound_e2e.rs
- Function: `e2e_0088_036_two_bombs_and_a_normal_document_at_on`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-030 | in `prepare_extraction`, on an extraction error, write an empty companion before returning the refusal: `/docs/report.md` no longer reads `old text`, the test fails |
| E2E-031 | with `refresh`, answer the existing companion when the extraction is refused: the answer is cached, not the refusal, the test fails |
| E2E-032 | in `extract_document`, charge the session with the source size before `prepare_extraction`: after two calls the 52 428 800 byte write is refused, the test fails |
| E2E-033 | drop `to_lowercase` in `extension_of`: `/docs/BOMB.DOCX` is decoded as text and succeeds, the test fails |
| E2E-034 | remove `.pptm` from the PowerPoint arm of `Extractor::extract`: `/docs/b.pptm` is decoded as text and succeeds, the test fails |
| E2E-036 | remove the part limit check from `read_part`: both bombs are decoded and succeed, `parts_held` is 200 000 000, the test fails |

## Constraints
- Files not to touch: `crates/core/src/docs/extract.rs`, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- The log line carries the `ToolError` display only (code and message); never an argument, a token or a password
- A 5xx keeps ERROR level (the classifier is unchanged)
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (E2E-035), before the production code (step 1: the capture's `level` field and the 7 tests plus `rest_refusal_logs_one_info_line`: E2E-035 and the unit test red as stated, the others green on arrival; step 2: the two `log_tool_failure` calls).
- [ ] Green on arrival tests (E2E-030, E2E-031, E2E-032, E2E-033, E2E-034, E2E-036): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `.agent_docs/architecture.md` (DEC-015); the closing story US-0211 carries the rest.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0210: a refusal leaves no trace and one log line` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-007, FR-NEW-002, FR-NEW-006, FR-NEW-011).
