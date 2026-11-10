# US-0219: diagnostic and bundle mode outputs held small

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0218
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
A local converter that floods its diagnostic output, or its standard output while it answers by a bundle, no longer makes the server hold that flood: only the last 8 192 bytes of the diagnostic output are kept (the part quoted on failure), and standard output in bundle mode is discarded as it comes. The bundle rule and its message are unchanged.

## Requirements covered
- SPEC-0089/FR-NEW-011 (partial: the diagnostic output and the standard output in bundle mode): THE document conversion SHALL hold, during one conversion, at most the converter limit plus 65 536 bytes of the converter's answer, at most 73 728 bytes of its diagnostic output, and at most 65 536 bytes of its standard output in bundle mode.
- SPEC-0089/FR-NEW-010 (partial: the bundle's files together, unchanged rule): IF the converter's answer passes the converter limit THEN THE document conversion SHALL stop reading it, end the converter's work, remove its working folder, and refuse with `ERR_INTERNAL_ERROR` and the message `document service output is over the doc_service.max_input_bytes limit of <cap>`.
- SPEC-0089/FR-NEW-014 (partial: bundle mode within the limit, the failure tail): THE product SHALL answer a PDF extraction or a conversion through the HTTP API with the same code, message and effects as through the assistant tool, and SHALL answer a conversion within the converter limit with the same companion, artifacts, report, charge, audit line and failure messages as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0089_004 e2e_0089_037 e2e_0089_038 e2e_0089_sec_014 e2e_0089_sec_017`: 5 tests pass. Real assertions: the `CONVERTER_HELD` records (`diagnostic` at most 73 728, `bundle_stdout` at most 65 536), the failure message ending with `the real cause` and at most 8 320 bytes long, the companion and the image artifact of a bundle. `cli_failure_carries_the_exit_code_and_only_the_stderr_tail` and the two bundle cap tests of `docs/service.rs` still pass.

## Design excerpt
### Decisions
- **DEC-015 (SPEC-0089), this story's part:** stderr is read through `read_tail` (read a 65 536 byte chunk, append it, then keep only the last `STDERR_CAP` 8 192 bytes: at most 8 192 + 65 536 = 73 728 held); in bundle mode stdout is read through `drain` (one chunk buffer, nothing kept, not counted against the converter limit). Both run in the `try_join!` of US-0218, inside the timeout. The failure message is built from the kept tail with `tail_of` as today (`crates/core/src/docs/service.rs:592-596`).
- **DEC-017 (SPEC-0089):** `ConverterHeld.diagnostic` is the largest tail buffer length, `bundle_stdout` the largest chunk read and discarded.
### Contracts
`async fn read_tail(reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>>`; `async fn drain(reader: impl AsyncRead + Unpin) -> std::io::Result<()>`. Messages unchanged: `document service exited with exit code <n>: <tail>`; bundle `document service output is over the doc_service.max_input_bytes limit of <cap>` (`over_cap`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/service.rs` | modify |
| `crates/core/src/tools/converter_bound_e2e.rs` | modify |
### Existing patterns
US-0218's `read_capped_async`, `stand_in`, `cli`, `converter_fixture`; the bundle stand in of `docs/service.rs:923-958` (`cli_outdir_mode_reads_the_bundle_the_converter_wrote`) and its `{outdir}` command (`crate::config::OUTDIR_PLACEHOLDER`); artifact listing `crate::tools::artifacts::list_images(&f.state, MOUNT, doc, None)`.
### Fixtures
E2E-SEC-014: `exec perl -e 'my $s = "n" x 65536; print STDERR $s for 1..16384; print STDERR "the real cause"; exit 3'`. E2E-SEC-017 (bundle mode, command `[stand_in, {outdir}, {document}]`): `printf '# Deck' > "$1/document.md"; exec perl -e 'my $s = "p" x 65536; print $s for 1..16384'`. E2E-004: `document.md` holding `# Deck\n\n![Chart](figures/f1.png)\n`, `figures/f1.png` 500 bytes, `artifacts.json` `{"pictures": {"figures/f1.png": {"caption": "Chart"}}}`. E2E-037: `document.md` `# Deck` (6 bytes) and `big.bin` of 1 048 570 bytes; E2E-038 `big.bin` of 1 048 571 bytes. Take `heavy()` first in SEC-014 and SEC-017.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0089/E2E-SEC-014: Diagnostic output flood
- **Covers:** occurrence 15
- **Red before the fix because:** `And the diagnostic held never exceeds 73 728` fails: today the 1 073 741 824 bytes of diagnostic output are read whole and only their tail kept.
- **Given** a text mode stand in converter writing 1 073 741 824 letters `n`, then `the real cause`, to its diagnostic output and ending with exit code 3
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `document service exited with exit code 3: ` and ending with `the real cause`, of at most 8 320 bytes
- **And** the diagnostic held never exceeds 73 728; nothing left
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::documentize` with a real `CliDocService` / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_sec_014_diagnostic_output_flood`
- Red before the change because: step 1 (hook recording the whole stderr buffer of US-0218's code): `diagnostic` is 1 073 741 838
- Cleanup: none

### SPEC-0089/E2E-SEC-017: Standard output flood in bundle mode
- **Covers:** occurrence 18
- **Red before the fix because:** `And the bundle mode output held never exceeds 65 536` fails: today the 1 073 741 824 bytes of standard output are read whole and then ignored.
- **Given** a bundle mode stand in converter writing 1 073 741 824 letters `p` to its standard output and a valid bundle with `document.md` holding `# Deck`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the conversion succeeds and `/docs/deck.md` holds `# Deck`
- **And** the bundle mode output held never exceeds 65 536
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_sec_017_standard_output_flood_in_bundle_m`
- Red before the change because: after step 1 `bundle_stdout` is 1 073 741 824
- Cleanup: none

### SPEC-0089/E2E-004: Bundle mode conversion within the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** a bundle mode stand in converter writing `document.md` with `# Deck` and `![Chart](figures/f1.png)`, a 500 byte picture `figures/f1.png` and a manifest captioning it `Chart`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` holds the Markdown, and one image artifact captioned `Chart` is listed for `/docs/deck.pptx`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_004_bundle_mode_conversion_within_the_lim`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-037: Bundle of exactly the converter limit
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-010, FR-NEW-014; **Priority:** High
- **Given** a bundle mode stand in converter writing `document.md` with `# Deck` (6 bytes) and `big.bin` of 1 048 570 bytes
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` holds `# Deck`
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_037_bundle_of_exactly_the_converter_limit`
- Green on arrival; mutation below
- Cleanup: none

### SPEC-0089/E2E-038: Bundle one byte over the converter limit, unchanged
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** the converter of E2E-037 with `big.bin` of 1 048 571 bytes
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`, as today; nothing left
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/converter_bound_e2e.rs
- Function: `e2e_0089_038_bundle_one_byte_over_the_converter_li`
- Green on arrival; mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-004 | ignore `artifacts.json` in `read_bundle_blocking`: the image is listed with an empty caption, the test fails |
| E2E-037 | in `read_bundle_blocking`, refuse a file whose length reaches the remaining cap (`>=` for `>`): the bundle is refused, the test fails |
| E2E-038 | remove both size checks of `read_bundle_blocking`: the bundle is accepted, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/core/fs_ops.rs`, the 22 tests of `docs/service.rs`
- Dependencies not to add: none
- Bundle file reading is unchanged (BL-0150 owns a file growing after its length is taken)
- No dash as punctuation in code, comments or docs
- Scope boundary: the web answer is US-0220

## Definition of done
- [ ] RED evidence recorded for E2E-SEC-014 and E2E-SEC-017 (step 1: tests and the hook figures on US-0218's code, under `heavy()`), before the production code (step 2: `read_tail` and `drain`).
- [ ] Green on arrival tests (E2E-004, E2E-037, E2E-038): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit; main only receives the whole spec.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib docs::service tools::artifacts` and every `e2e_0089_` test pass.
- [ ] Docs updated: none in this story.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; failure modes listed in the commit body.
- [ ] One commit `US-0219: diagnostic and bundle mode outputs held small` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-010, FR-NEW-011, FR-NEW-014).
