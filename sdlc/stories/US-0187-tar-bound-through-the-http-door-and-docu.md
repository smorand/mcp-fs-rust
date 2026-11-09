# US-0187: tar bound through the HTTP door and documentation

> Spec: SPEC-0084 (sdlc/specs/SPEC-0084-tar-family-bounded-decoding/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0186
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
A client of the HTTP API extracting a `.tar.xz` gets the same answer and audit line as through the assistant tool, and the documentation describes the bound on compressed tar archives and the single xz decoder. This closes SPEC-0084.

## Requirements covered
- SPEC-0084/FR-NEW-011 (partial: the HTTP answer and the audit line): THE archive extraction SHALL extract plain `.tar`, `.zip`, `.7z`, RAR, bare `.gz`, `.bz2` and `.xz` files exactly as before this increment, SHALL answer a compressed tar archive within the limits with the same destination, counts and audit line as before, SHALL check the two limits on the files' sizes as before, and SHALL keep refusing `.tbz2` and every other unsupported extension with `ERR_NOT_SUPPORTED` and `unsupported archive extension: '<extension as typed>'`.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0084_`: all 46 tests of SPEC-0084 pass. Real assertion: the HTTP door's JSON body and the audit trail read through `state.safety.audit`.

## Design excerpt
### Decisions
- **DEC-011 (SPEC-0084):** documentation, in the closing story: `README.md` archive paragraph (`.tar.xz` decoded by liblzma, compressed tar decoding stops at the bound), `AGENTS.md` stack line (`lzma-rs` removed) and `archive.rs` entry, `.agent_docs/tools.md` archive section (order of refusals for compressed tar, a dated `(2026-11-09, SPEC-0084.)` paragraph), `.agent_docs/testing.md` (filter `e2e_0084_`, `archive_testkit.rs`, the hook). **Obliged by:** n/a (no code impact): project convention (AGENTS.md "keep README.md and AGENTS.md up to date"). **Rationale:** `README.md:295` says the tar family is "all decoded in pure Rust", false once `.tar.xz` uses liblzma. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `README.md:294-305`, `AGENTS.md:37`, `.agent_docs/tools.md:181-217`.
### Data model
None.
### Contracts
HTTP: `POST /api/fs/{mount_id}/extract-archive` with `{"path": "/uploads/pack.tar.xz"}`, answer `{"destination": "/uploads/pack", "files_written": 1, "dirs_created": 1, "bytes_written": 5}` (route `api/dataplane.rs:176`, handler `:876-902`). Documentation: `README.md:294-305` (`.tar.xz`/`.txz` decoded by liblzma like a bare `.xz`, no longer "pure Rust"; decoding of a compressed tar stops at the lower limit plus 1 MiB of tar bookkeeping); `AGENTS.md:37` stack line (`liblzma` for every xz, `lzma-rs` gone) and the `archive.rs` entry (`:134-137`); `.agent_docs/tools.md:181-217` (order of refusals for a compressed tar: membership, path, folder, password, corrupt or bound whichever first, entry checks, destination, collision, limits; a dated `(2026-11-09, SPEC-0084.)` paragraph); `.agent_docs/testing.md` (a dated line: filter `e2e_0084_`, `archive_testkit.rs` shared by SPEC-0084 to SPEC-0087, `held_some` versus `held`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (test) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
HTTP tests of the archive route: `e2e_new_031_extract_archive_happy_path` and the `e2e_0080_` HTTP tests (`crates/core/src/api/dataplane.rs:3297-3470`, `Harness`, `u("extract-archive")`, `seed` through `h.state.stores.client(MOUNT)`); the dated paragraph style of `.agent_docs/tools.md:220-252` and `.agent_docs/testing.md:432-435`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0084/E2E-007: Audit line and HTTP API unchanged
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** the situation of E2E-003 for `/uploads/pack.tar.xz`
- **When** a client asks the HTTP API to extract it
- **Then** the answer is that of E2E-003
- **And** the audit trail has exactly one extraction line for `/uploads/pack.tar.xz` naming destination `/uploads/pack`, 1 file written and 5 bytes written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0084_007_audit_line_and_http_api_unchanged`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-007 | in `finish_extraction` skip `record_audit` (`archive.rs:322-328`): no extraction line is recorded, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Documentation describes what US-0183 to US-0186 built, nothing more

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-007): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` (includes `tools::archive_single`) passes unchanged, the bare format suites `e2e_0081_`, `e2e_0082_`, `e2e_0083_` and `e2e_new_` included; `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` as listed in Contracts.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0084 is green (`cargo test -p mcp-fs-core --lib -- e2e_0084_`), and every requirement of SPEC-0084 is cited by at least one story with status done or landed.
- [ ] One commit `US-0187: tar bound through the HTTP door and documentation`, body `Spec: SPEC-0084` and the requirement ids (SPEC-0084/FR-NEW-011).
