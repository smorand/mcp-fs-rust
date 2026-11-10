# US-0211: Office limits through the HTTP door and documentation

> Spec: SPEC-0088 (sdlc/specs/SPEC-0088-bound-office-document-part-decoding/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0210
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
A client of the HTTP API gets the same answers and refusals as the assistant tool for Office documents within and past the limits, and the documentation states the three limits, the size lie refusal and how to run the tests. This closes SPEC-0088.

## Requirements covered
- SPEC-0088/FR-NEW-010: THE document text extraction SHALL refuse or answer a call through the HTTP API with the same code and message, and the same effects, as the same call through the assistant tool.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0088_`: every test of SPEC-0088 passes (50). Real assertion: the HTTP status and the JSON body (`error`, `detail`) of the REST route.

## Design excerpt
### Decisions
- **DEC-010 (SPEC-0088):** test layout: `crates/core/src/tools/office_bound_e2e.rs` (`#[cfg(test)] mod office_bound_e2e;` in `tools/mod.rs`), every test `#[tokio::test]` (single threaded) through `fs_ops::extract_document` after `f.state.authorize(MOUNT, person)`, except E2E-035 (a plain `#[test]` running a dedicated current thread runtime under `crate::logging::capture::lock_for_test`, calling `McpServer::new(state, person).fs_extract_text(Parameters(..))`, the model being `crates/core/src/tools/archive.rs:1862-1865`) and the three HTTP tests in `api/dataplane.rs`. Names `e2e_0088_<id>_<snake title>` cut at 50 characters. No cargo feature. **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0085 DEC-011 and SPEC-0087 DEC-008 conventions; `./test.sh` runs the default build first (`test.sh:5`). **Alternatives considered:** tests inside `docs/extract.rs` (no access to the project, quota and audit channels). **Source:** existing code. **Evidence:** `test.sh:5-7`, `crates/core/src/tools/mod.rs:70-110`.
- **DEC-015 (SPEC-0088):** documentation: US-0210 updates `.agent_docs/architecture.md` (the error logging split is now wired at both doors; a dated "Office extraction bound" paragraph: the three limits, the size lie, reading order); the closing story US-0211 updates `README.md` (Office extraction limits), `AGENTS.md` (`docs/extract.rs` bullet: `OfficeBudget`, the three limits), `.agent_docs/tools.md` (`:336` gap closed, `fs.extract_text` limits and messages, dated SPEC-0088 paragraph) and `.agent_docs/testing.md` (filter `e2e_0088_`, `heavy()`, `deflate_run`, `bomb_part`, the hook). **Obliged by:** n/a (no code impact): project convention (`AGENTS.md`, spec 14.3 docs list). **Rationale:** `.agent_docs/tools.md:336` names this gap as remaining. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** as cited.
- **DEC-016 (SPEC-0088):** the tool contract is unchanged: `fs.extract_text`'s description does not mention the limits; they are documented in `.agent_docs/tools.md` and `README.md`. **Obliged by:** SPEC-0088/FR-NEW-011 (the rest unchanged). **Rationale:** spec 14.3 leaves the choice; a contract change would regenerate `tool-contract-golden.json` for no behaviour a caller needs to plan for. **Alternatives considered:** adding the limits to the description. **Source:** design. **Evidence:** `TOOL_CONTRACT.txt`.
### Data model
None.
### Contracts
HTTP: `POST /api/fs/{mount_id}/extract-text` with `{"path": ...}`; a refusal answers status 400 with `error` `ERR_INVALID_ARGUMENT` and `detail` `ERR_INVALID_ARGUMENT: could not extract <path>: <reason>` (`error_response`, `crates/core/src/api/dataplane.rs:259-264`). Documentation per DEC-015: `README.md` (document extraction: the three limits and their refusals), `AGENTS.md` (`docs/` bullet: `OfficeBudget` in `extract.rs`, part, call and content limits), `.agent_docs/tools.md` (`:336` the Office gap is closed; after `:404-410` a dated `(2026-11-10, SPEC-0088.)` paragraph: the four messages, the reading order, parts not read, a lying sheet or slide now refused), `.agent_docs/testing.md` (dated line: filter `e2e_0088_`, `heavy()`, `deflate_run`, `bomb_part`, `zip_raw`, `parts_held` and `content_held`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (tests) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
HTTP extraction tests: `extract_text_returns_a_preview`, `extract_text_on_a_missing_file_is_404` (`crates/core/src/api/dataplane.rs:2386-2405`); the closest spec level model `e2e_0085_005_http_api_and_audit_line_unchanged` (`:3843`). Builders from `crates/core/src/tools/archive_testkit.rs` (`office_zip`, `bomb_part`, `zip_declaring`, `zip_raw`, `heavy`); the hook readers `parts_held` work on the HTTP path too (the route runs the reader on the test's thread). Documentation precedents: the SPEC-0085 and SPEC-0087 paragraphs of `.agent_docs/tools.md` and `.agent_docs/testing.md:436-440`.
### Fixtures
Payloads: E2E-004 the `/docs/memo.docx` of E2E-001; E2E-014 `zip_raw` of a workbook with sheets `Good` (cell `ok`) and `Bad` whose `xl/worksheets/sheet2.xml` declares 100 and holds 500 000 bytes; E2E-029 `bomb_part("word/document.xml")`. Take `heavy()` first in 029. Nothing left is read with the harness's own client (listing of `/docs`, audit of `owner@test.com`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0088/E2E-004: HTTP API within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-010, FR-NEW-011; **Priority:** High
- **Given** the document of E2E-001
- **When** a client asks the HTTP API for its text
- **Then** the answer carries the same companion path, format and preview as E2E-001
- **And** the audit trail has one extraction line for `/docs/memo.md`
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-text"), json!({...}))` (the harness project `proj` and member `owner@test.com` stand for `atlas` and `mia@example.com`) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0088_004_http_api_within_the_limits`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-014: Excel sheet larger than it declares, through the HTTP API
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-010; **Priority:** High
- **Given** `/docs/liar.xlsx` with sheets `Good` (cell `ok`) and `Bad`, whose part `xl/worksheets/sheet2.xml` declares 100 bytes and holds 500 000 bytes
- **When** a client asks the HTTP API for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/liar.xlsx: document part 'xl/worksheets/sheet2.xml' decodes to more bytes than declared 100` (today: success with sheet `Good` only)
- **And** nothing left
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-text"), json!({...}))` (the harness project `proj` and member `owner@test.com` stand for `atlas` and `mia@example.com`) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0088_014_excel_sheet_larger_than_it_declares_t`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0088/E2E-029: The HTTP API refuses a bomb alike
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-010; **Priority:** Critical
- **Given** `/docs/bomb.docx`, a bomb part document
- **When** a client asks the HTTP API for its text
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and the message of E2E-SEC-001
- **And** the decoded bytes held are 0; nothing left
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-text"), json!({...}))` (the harness project `proj` and member `owner@test.com` stand for `atlas` and `mia@example.com`) / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0088_029_the_http_api_refuses_a_bomb_alike`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-004 | in the REST `extract_text` route pass `max_chars` 10: the preview is not the one of E2E-001, the test fails |
| E2E-014 | in the REST route strip the `could not extract <path>: ` prefix from a refusal: the `detail` differs, the test fails |
| E2E-029 | in the REST route map `ERR_INVALID_ARGUMENT` to `ERR_NOT_SUPPORTED`: status and `error` differ, the test fails |

## Constraints
- Files not to touch: `crates/core/src/docs/extract.rs`, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- Each mutation is applied, run, then reverted
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] RED evidence: none: the three tests are green on arrival (DEC-014); baseline run recorded, then each mutation.
- [ ] Green on arrival tests (E2E-004, E2E-014, E2E-029): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::extract tools::document core::fs_ops` pass (the 14 Office reader tests of `docs/extract.rs` unchanged, the SPEC-0041 extraction tests, the 16 `tools/document.rs` tests); every `e2e_0084_`, `e2e_0085_`, `e2e_0086_` test and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; no change to the tool contract.
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` (DEC-015).
- [ ] Closes spec only: the spec's full acceptance set green (`cargo test -p mcp-fs-core --lib -- e2e_0088_`: E2E-001 to E2E-037 and E2E-SEC-001 to E2E-SEC-013, 50 tests), and every requirement of SPEC-0088 (FR-NEW-001 to FR-NEW-011) cited by at least one story with status done or landed (US-0205 to US-0211).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] One commit `US-0211: Office limits through the HTTP door and documentation` on the spec branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0088` and the requirement ids (SPEC-0088/FR-NEW-010).
