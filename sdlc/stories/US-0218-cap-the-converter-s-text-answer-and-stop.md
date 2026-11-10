# US-0218: cap the converter's text answer and stop it

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0217
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A local converter that answers more Markdown on its standard output than the converter limit is stopped as soon as the answer passes the limit: the server reads at most one chunk past it, kills and reaps the program, removes its working folder and refuses with today's bundle message, without waiting for the converter or the timeout. Documentize leaves nothing; an upload or binary write with the conversion flag keeps the document and reports the refusal. Every other conversion outcome is unchanged.

## Requirements covered
- SPEC-0089/FR-NEW-010 (partial: the standard output in text mode): IF the converter's answer passes the converter limit THEN THE document conversion SHALL stop reading it, end the converter's work, remove its working folder, and refuse with `ERR_INTERNAL_ERROR` and the message `document service output is over the doc_service.max_input_bytes limit of <cap>`.
- SPEC-0089/FR-NEW-011 (partial: the answer held in text mode): THE document conversion SHALL hold, during one conversion, at most the converter limit plus 65 536 bytes of the converter's answer, at most 73 728 bytes of its diagnostic output, and at most 65 536 bytes of its standard output in bundle mode.
- SPEC-0089/FR-NEW-012: IF several refusals apply to one conversion THEN THE document conversion SHALL report the first met in this order: (1) membership; (2) not a file (documentize); (3) converter not configured or extension not accepted (`ERR_NOT_SUPPORTED`, today's messages); (4) the document over the converter limit (today's message); (5) the SPEC-0041 Office check; (6) the companion exists and overwrite is not asked (documentize, `ERR_NO_CLOBBER`); (7) during the conversion, whichever comes first: the answer passing the converter limit (FR-NEW-010), the timeout; then, once the converter ended within the limit, its failure exit, a web failure status, an empty answer or a bundle without its Markdown, with today's messages.
- SPEC-0089/FR-NEW-013: THE document conversion SHALL NOT write or change a companion, store an artifact, charge the session write allowance for a companion, or record an audit line for a companion, for a conversion refused under FR-NEW-010.
- SPEC-0089/FR-NEW-014 (partial: the assistant tool's binary write, the unchanged text mode outcomes): THE product SHALL answer a PDF extraction or a conversion through the HTTP API with the same code, message and effects as through the assistant tool, and SHALL answer a conversion within the converter limit with the same companion, artifacts, report, charge, audit line and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_003 e2e_0089_033 e2e_0089_034 e2e_0089_040 e2e_0089_042 e2e_0089_044 e2e_0089_045 e2e_0089_046 e2e_0089_047 e2e_0089_048 e2e_0089_049 e2e_0089_sec_013`: 12 tests pass. Real assertions: the `CONVERTER_HELD` records (answer at most 1 114 112), the stand in's pid gone (`kill -0` fails) and its working folder absent after the call, the listing of `/docs`, the artifact list, the audit trail, a following 52 428 800 byte write, the stand in's start marker, the captured log lines. The 22 tests of `docs/service.rs` still pass.

## Design excerpt
### Decisions
- **DEC-015 (SPEC-0089), this story's part:** the pipes are read concurrently with the wait, inside the existing timeout (`crates/core/src/docs/service.rs:262-279`): text mode stdout through `read_capped_async(stdout, max_input_bytes)` (65 536 byte reads; a read that would bring the buffer past the cap returns `Capped::Over` with the buffer unextended, so at most the cap plus one chunk is held); stderr still read whole in this story (its tail ring is US-0219); bundle mode stdout unchanged in this story (drained in US-0219). The reads and the wait run under `tokio::try_join!`, so `Over` returns at once; then `child.kill().await` (kill and reap, as the timeout path, `service.rs:287-295`) and `over_cap(cap)`, before `dir` drops. Order: cap or timeout first, whichever comes first; then exit status, empty answer, bundle. Text mode markdown is `String::from_utf8_lossy` of the capped bytes as today.
- **DEC-017 (SPEC-0089):** converter hook, test only: each `convert` (both modes, every outcome) pushes `ConverterHeld { answer, diagnostic, bundle_stdout }` onto the thread local `CONVERTER_HELD`; `answer` is the largest answer buffer length (text stdout or web body), `diagnostic` the largest diagnostic buffer length, `bundle_stdout` the largest chunk read and discarded. One record per call lets E2E-046 read two concurrent calls.
- **DEC-019 (SPEC-0089), this story's part:** stand ins in `converter_bound_e2e.rs`: `stand_in(home, body) -> PathBuf` (an executable `sh` script that first writes `started`, `$$` and `$(pwd)` to a marker file outside the sandbox, then runs `body`; floods end in `exec perl -e '...'` so the flooding process is the one killed); `cli(command, cap, timeout) -> CliDocService`; `converter_fixture()` (project `atlas`, member `mia@example.com`, session write allowance 1 000 000 000, `/docs/deck.pptx` holding `PK`). `perl` ships with macOS and every Debian based image.
- **DEC-025 (SPEC-0089):** the error level log line of a documentize refusal comes from `to_call_result` calling `log_tool_failure` (SPEC-0088 DEC-012); no new log call.
- **DEC-028 (SPEC-0089):** E2E-003's stand in answers `# Deck` followed by 1 994 letters `d` (2 000 bytes in total): the spec's Given says "followed by 1 990 bytes" (1 996 in total) while its Then asserts 2 000 bytes written; the asserted figure is kept and the fixture sized to it (recorded in the spec's `design-issues.md`, non blocking).
- **DEC-026 (SPEC-0089):** the converter tests run the real `CliDocService` over stand ins, through `core::fs_ops::documentize`; `max_input_bytes` 1 048 576, timeout 30 seconds (E2E-044: 1).
### Data model
`enum Capped { Over, Io(std::io::Error) }`; `#[cfg(test)] ConverterHeld { answer: u64, diagnostic: u64, bundle_stdout: u64 }` and `CONVERTER_HELD: RefCell<Vec<ConverterHeld>>`.
### Contracts
`crates/core/src/docs/service.rs`: `const CHUNK: usize = 65_536;`, `async fn read_capped_async(reader: impl AsyncRead + Unpin, cap: u64) -> std::result::Result<Vec<u8>, Capped>`, `fn over_cap(cap: u64) -> ToolError` returning `ToolError::internal(format!("document service output is over the doc_service.max_input_bytes limit of {cap}"))`, used by `read_bundle_blocking` (`:515-517`, `:522-525`) and `parse_api_bundle` (`:574-579`) too, same text. Every other message unchanged: `document service exited with <exit code n>: <tail>`, `doc service timed out after <s>s`, `document service exited with exit code 0 but produced no output: <tail>`, `'<path>' is <n> bytes, over the doc_service.max_input_bytes limit of <cap>`, `'<md>' exists (pass overwrite=true)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/service.rs` | modify |
| `crates/core/src/tools/converter_bound_e2e.rs` | create |
| `crates/core/src/tools/mod.rs` | modify (`#[cfg(test)] mod converter_bound_e2e;`) |
### Existing patterns
`read_capped` (`crates/core/src/tools/archive_single.rs:92-108`, the synchronous model); the CLI stand in scripts and the kill probe (`docs/service.rs:771-779` `script`, `:845-883` `cli_timeout_kills_the_child_and_removes_the_tempdir`); a server with a converter (`tools/artifacts.rs:663-680` `with_doc_as`, `:2124-2136` a real `CliDocService`); log capture (`tools/office_bound_e2e.rs:943-979`); `archive_testkit::{MOUNT, OWNER, SECOND, add_second_member, seed, read, exists, office_fixture_allowance, heavy}`.
### Fixtures
SEC-013 and E2E-046, E2E-048, E2E-049: `exec perl -e 'my $s = "a" x 65536; print $s for 1..16384'` (1 073 741 824 letters `a`). E2E-033: `printf '#'; exec perl -e 'print "d" x 1048575'`; E2E-034: 1 048 576 letters `d` after `#`. E2E-003: `printf '# Deck'; exec perl -e 'print "d" x 1994'` (2 000 bytes in total, DEC-028). E2E-042: `echo 'the real cause' >&2; exit 3`. E2E-044: `exec sleep 30`. E2E-047: `exit 0`. E2E-045: `/docs/big.pptx` of 1 048 577 bytes (`PK` then zeros). E2E-040: `McpServer::fs_write_bytes(WriteBytesArgs { path: "/docs/deck.pptx", base64: "UEs=", documentation: true, .. })`. E2E-046: a second project `orbit` owned by `noa@example.com`, each with `/docs/deck.pptx`, two `McpServer::fs_documentize` calls joined. Take `heavy()` first in SEC-013, E2E-034, E2E-040, E2E-046 and every flood test.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-013: Text answer flood
- **Covers:** occurrence 14
- **Red before the fix because:** `And the answer held never exceeds 1 114 112` fails: today the 1 073 741 824 bytes are read whole before the empty answer check.
- **Given** `/docs/bomb.docx` (`PK`) and a text mode stand in converter answering 1 073 741 824 letters `a`, the answer of a converter unpacking a zip bomb
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`, before the converter has written its whole answer
- **And** the answer held never exceeds 1 114 112; the operating system lists no process of the stand in converter (`kill -0 <pid>` fails); the server's temporary folder holds no working folder of the call (the folder the stand in wrote does not exist); nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::documentize` with a real `CliDocService`, after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_sec_013_text_answer_flood`
- Red before the change because: step 1 (hook around today's whole reads): `answer` is 1 073 741 824 and the call ends on another error (the companion write is over the allowance); the red run holds about 2 GB (the lossy copy), run it alone under `heavy()`
- Cleanup: the stand in's home folder is a `tempfile::TempDir`

### SPEC-0089/E2E-003: Text mode conversion within the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** `/docs/deck.pptx` and a text mode stand in converter answering `# Deck` followed by `d` letters, 2 000 bytes in total
- **When** the member asks for its companion
- **Then** the answer names `/docs/deck.md` with 2 000 bytes written, not overwritten
- **And** `/docs/deck.md` holds that answer; the audit trail has one line for `/docs/deck.md`
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::documentize` / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_003_text_mode_conversion_within_the_limit`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-033: Text answer of exactly the converter limit
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** a text mode stand in converter answering `#` then 1 048 575 letters `d`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` is written with 1 048 576 bytes
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_033_text_answer_of_exactly_the_converter`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-034: Text answer one byte over the converter limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010, FR-NEW-011, FR-NEW-013; **Priority:** Critical
- **Given** the stand in converter of E2E-033 answering one more letter
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_034_text_answer_one_byte_over_the_convert`
- Red before the change because: today the companion is written with 1 048 577 bytes
- Cleanup: none

### SPEC-0089/E2E-040: Binary write with conversion, answer over the limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-013, FR-NEW-014; **Priority:** High
- **Given** the stand in converter of E2E-034
- **When** the member writes `/docs/deck.pptx` through the assistant tool's binary write with the conversion flag
- **Then** the write succeeds and its documentation report carries `ERR_INTERNAL_ERROR` and `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** `/docs/deck.md` does not exist
- Level / Driver / Location: functional e2e / assistant tool method `McpServer::fs_write_bytes` (server built as `tools/artifacts.rs:668-680` does) / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_040_binary_write_with_conversion_answer_o`
- Red before the change because: today the report carries `md_path` and `/docs/deck.md` exists
- Cleanup: none

### SPEC-0089/E2E-042: Converter failure within the limits, unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012, FR-NEW-014; **Priority:** Medium
- **Given** a text mode stand in converter writing `the real cause` to its diagnostic output and ending with exit code 3
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service exited with exit code 3: the real cause`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_042_converter_failure_within_the_limits_u`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-044: Timeout, unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012, FR-NEW-014; **Priority:** Medium
- **Given** timeout 1 second and a text mode stand in converter that waits 30 seconds before answering
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `doc service timed out after 1s`; nothing left (no process, no working folder)
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_044_timeout_unchanged`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-045: Document over the converter limit refused before conversion
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/docs/big.pptx` of 1 048 577 bytes and the stand in converter of E2E-034
- **When** the member asks for its companion
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `'/docs/big.pptx' is 1048577 bytes, over the doc_service.max_input_bytes limit of 1048576`
- **And** the stand in converter was never started (no marker)
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_045_document_over_the_converter_limit_ref`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-046: Two converter floods at once
- **Category:** side effect; **Scenario:** SC-006; **Requirements:** FR-NEW-011, FR-NEW-013; **Priority:** Medium
- **Given** members `mia@example.com` and `noa@example.com`, each with `/docs/deck.pptx` in their own project, and the stand in converter of E2E-SEC-013
- **When** both ask for the companion at the same time
- **Then** both are refused with the message of E2E-034
- **And** each call's answer held never exceeds 1 114 112 (two `CONVERTER_HELD` records); the server log has exactly one error level line per call naming `ERR_INTERNAL_ERROR` and that message; no converter process and no working folder remain
- Level / Driver / Location: functional e2e / assistant tool method `McpServer::fs_documentize`, two calls joined, log capture (plain `#[test]` on a current thread runtime under `crate::logging::capture::lock_for_test`) / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_046_two_converter_floods_at_once`
- Red before the change because: after step 1 each record's `answer` is 1 073 741 824 and the error lines name the allowance, not the cap; the red run holds about 4 GB: run it alone, record it, or record the red of SEC-013 and this test's failure on the message with the flood cut to its first 16 MB through a temporary edit of the stand in, reverted (state which in the commit body)
- Cleanup: none

### SPEC-0089/E2E-047: Empty answer, unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012, FR-NEW-014; **Priority:** Medium
- **Given** a text mode stand in converter that answers nothing and ends with exit code 0
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `document service exited with exit code 0 but produced no output`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_047_empty_answer_unchanged`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-048: Not a member, converter flood
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/docs/deck.pptx`, the stand in converter of E2E-SEC-013 and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its companion (`f.state.authorize(MOUNT, "eve@example.com")` first, as the doors do)
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- **And** the stand in converter was never started
- Level / Driver / Location: functional e2e / `AppState::authorize` / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_048_not_a_member_converter_flood`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-049: Companion exists without overwrite, converter flood
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012, FR-NEW-013; **Priority:** Medium
- **Given** `/docs/deck.pptx`, `/docs/deck.md` holding `old`, and the stand in converter of E2E-SEC-013
- **When** the member asks for its companion without overwrite
- **Then** the refusal is `ERR_NO_CLOBBER` with `'/docs/deck.md' exists (pass overwrite=true)`
- **And** `/docs/deck.md` still reads `old`; the stand in converter was never started
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_049_companion_exists_without_overwrite_co`
- Green on arrival; mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-003 | in `read_capped_async`, drop the last byte read: 1 999 bytes written, the test fails |
| E2E-033 | refuse when the buffer would reach the cap (`>=` for `>`): the exact limit is refused, the test fails |
| E2E-042 | build the failure message without the stderr tail: the message lacks `the real cause`, the test fails |
| E2E-044 | skip `child.kill().await` on the timeout path: the `sleep` process is still alive after the call, the test fails |
| E2E-045 | in `documentize`, check the input size after `write_companion`: the stand in starts, the test fails |
| E2E-047 | skip the empty answer check in text mode: an empty companion is written, the test fails |
| E2E-048 | make `AppState::authorize` admit any person: the test's `unwrap_err` panics, the test fails |
| E2E-049 | in `write_companion`, convert before the no clobber check: the stand in starts, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/core/fs_ops.rs` (the refusal travels the existing error paths), the 22 tests of `docs/service.rs`
- Dependencies not to add: none (tokio `io-util` and `process` are already enabled, `Cargo.toml:14`)
- No `unsafe` (no process group kill: BL-0152); kill and reap before the `TempDir` drops
- No dash as punctuation in code, comments or docs
- Scope boundary: stderr tail ring and bundle stdout drain are US-0219; the web answer US-0220; the HTTP door tests US-0221

## Definition of done
- [ ] RED evidence recorded for E2E-SEC-013, E2E-034, E2E-040, E2E-046 (step 1: stand ins, the test module and the hook around today's reads, no cap; under `heavy()`, one at a time), before the production code (step 2: `read_capped_async`, `try_join!`, kill on the cap, `over_cap`).
- [ ] Green on arrival tests (E2E-003, E2E-033, E2E-042, E2E-044, E2E-045, E2E-047, E2E-048, E2E-049): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::service tools::document tools::artifacts api::dataplane` pass (the bundle cap tests keep their message).
- [ ] Docs updated: none in this story.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; failure modes listed in the commit body (a child that ignores SIGKILL cannot exist; grandchildren survive, BL-0152).
- [ ] One commit `US-0218: cap the converter's text answer and stop it` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013, FR-NEW-014).
