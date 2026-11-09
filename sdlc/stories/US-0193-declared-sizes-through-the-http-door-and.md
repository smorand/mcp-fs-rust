# US-0193: declared sizes through the HTTP door and documentation

> Spec: SPEC-0085 (sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0192
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
A client of the HTTP API gets the same answer and audit line as through the assistant tool for a zip within the limits, and the documentation describes the declared size check before decoding and the new order of refusals. This closes SPEC-0085.

## Requirements covered
- SPEC-0085/FR-NEW-010 (partial: the HTTP answer and the audit line): THE archive extraction SHALL answer a `.zip`, `.7z` or `.rar` archive within the limits with the same destination, counts and audit line as before, SHALL charge the session write allowance the declared total on success as before, and SHALL extract or refuse plain `.tar`, compressed tar archives and bare `.gz`, `.bz2` and `.xz` files exactly as before this increment.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0085_` and `cargo test -p mcp-fs-core --lib --features rar -- e2e_0085_`: every test of SPEC-0085 passes. Real assertion: the JSON body of the HTTP door and the audit trail.

## Design excerpt
### Decisions
- **DEC-012 (SPEC-0085):** documentation in the closing story: `README.md` (limits checked on declared sizes before decoding; size lies stop at the declared size plus 64 KiB), `AGENTS.md` (`archive.rs` entry), `.agent_docs/tools.md` (order of refusals per format, a dated SPEC-0085 paragraph), `.agent_docs/testing.md` (filter `e2e_0085_`, nested `archive_rar` rule, the two builders). **Obliged by:** n/a (no code impact): project convention. **Rationale:** `README.md:310-313` describes the old "decoded and checked before the first write" rule. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `README.md:306-320`, `.agent_docs/tools.md:184-217`.
### Data model
None.
### Contracts
HTTP: `POST /api/fs/{mount_id}/extract-archive` with `{"path": "/uploads/pack.zip"}` answers `{"destination": "/uploads/pack", "files_written": 1, "dirs_created": 1, "bytes_written": 5}`. Documentation: `README.md:306-320` (declared sizes checked against both limits before any entry is decoded; a size lie stops at the declared size plus 64 KiB; sizes at or past 2^63 refused at the limit); `AGENTS.md:134-137` (`archive.rs` entry: list, check, decode); `.agent_docs/tools.md:184-217` (per format order of refusals; dated `(2026-11-09, SPEC-0085.)` paragraph); `.agent_docs/testing.md` (dated line: filters `e2e_0085_`, the nested `archive_rar` module rule for `test.sh`, `zip_declaring` and `sevenz_declaring`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (test) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
HTTP tests of the archive route (`crates/core/src/api/dataplane.rs:3297-3470`); the dated paragraph style of `.agent_docs/tools.md:220-252` and `.agent_docs/testing.md:432-435`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0085/E2E-005: HTTP API and audit line unchanged
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the situation of E2E-001
- **When** a client asks the HTTP API to extract `/uploads/pack.zip`
- **Then** the answer is that of E2E-001
- **And** the audit trail has exactly one extraction line for `/uploads/pack.zip` naming destination `/uploads/pack`, 1 file written and 5 bytes written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0085_005_http_api_and_audit_line_unchanged`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-005 | in `finish_extraction` skip `record_audit`: no extraction line is recorded, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Documentation describes what US-0188 to US-0192 built, nothing more

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-005): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive tools::quota_e2e` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_new_020_archive_total_declared_size_exceeds_remaining_quota`, `e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds` (`archive.rs:1785-1830`), `spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted`, `spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing`, `spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted`, `spec_0020_e2e_051_an_extraction_racing_a_write` (`quota_e2e.rs`), `e2e_0080_066_missing_password_wins_over_an_oversi`, `e2e_0080_067_oversized_entry_wins_over_a_collisio`, `e2e_0080_081_the_project_refusal_comes_first` (`archive_rar.rs`), every `e2e_0084_` test; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` as listed in Contracts.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0085 is green (`cargo test -p mcp-fs-core --lib -- e2e_0085_ && cargo test -p mcp-fs-core --lib --features rar -- e2e_0085_`), and every requirement of SPEC-0085 is cited by at least one story with status done or landed.
- [ ] One commit `US-0193: declared sizes through the HTTP door and documentation`, body `Spec: SPEC-0085` and the requirement ids (SPEC-0085/FR-NEW-010).
