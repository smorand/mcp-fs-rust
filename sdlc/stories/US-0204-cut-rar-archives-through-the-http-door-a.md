# US-0204: cut RAR archives through the HTTP door and documentation

> Spec: SPEC-0087 (sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0203
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
A client of the HTTP API gets the same corrupt refusal as the assistant tool for a cut RAR4 archive, a cut archive with encrypted file names and a cut archive with encrypted data, and the documentation says truncation is detected for both RAR versions. This closes SPEC-0087.

## Requirements covered
- SPEC-0087/FR-NEW-008: THE archive extraction SHALL give, through the HTTP API, the same code and message as through the assistant tool for every refusal of this specification.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_` and `cargo test -p mcp-fs-core --lib -- e2e_0087_043`: every test of SPEC-0087 passes. Real assertion: the HTTP status and JSON error body.

## Design excerpt
### Decisions
- **DEC-008 (SPEC-0087):** tests live in `crates/core/src/tools/archive_rar_cut_e2e.rs` (`#[cfg(test)] mod archive_rar_cut_e2e;`), each `#[cfg(feature = "rar")]` except E2E-043 (`#[cfg(not(feature = "rar"))]`, run by the default `cargo test --workspace`); the module path contains `archive_rar` for `test.sh:7`; the HTTP tests sit in a nested `#[cfg(feature = "rar")] mod archive_rar` of `api/dataplane.rs` tests. Helpers from `archive_testkit.rs` plus a module local `assert_never_recorded` twin (`archive_rar.rs:922-938`). **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0085 DEC-011. **Source:** existing code. **Evidence:** `test.sh:7`, `crates/core/src/tools/archive.rs:905-933`.
- **DEC-010 (SPEC-0087):** documentation in the closing story: `.agent_docs/tools.md` (truncation detected for both versions, encrypted file names included; the RAR4 limitation sentence at `:208-210` removed), `README.md`, `AGENTS.md` (`archive_rar_crypt.rs` entry, RAR header decryption with in tree RustCrypto crates), `.agent_docs/testing.md` (filter `e2e_0087_`, reduced loop, fixture provenance). **Obliged by:** n/a (no code impact): project convention. **Rationale:** `.agent_docs/tools.md:209` states the limitation this spec removes. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** as cited.
### Data model
None.
### Contracts
HTTP: `POST /api/fs/{mount_id}/extract-archive` with `{"path": ..., "password": ...}`; the refusal body carries `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`. The three tests sit in a nested `#[cfg(feature = "rar")] mod archive_rar` inside the tests of `api/dataplane.rs` (so `test.sh:7` runs them). Documentation: `.agent_docs/tools.md:196-218` (truncation detected for RAR4 and RAR5, encrypted file names included; remove "Truncation is detected for RAR 5 only" at `:209`; dated `(2026-11-09, SPEC-0087.)` paragraph), `README.md` RAR sentence, `AGENTS.md` (`archive_rar.rs` walk, new `archive_rar_crypt.rs`: RAR header decryption with RustCrypto `aes`, `cbc`, `hmac`, `sha2` 0.11, `sha1`), `.agent_docs/testing.md` (dated line: filter `e2e_0087_`, the reduced loop of E2E-034, fixture provenance `/tmp/bl131` and RAR 6.24 / 7.23).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (tests) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
HTTP RAR tests: `e2e_0080_022_the_http_api_gives_the_same_refusal`, `e2e_0080_070_empty_password_through_the_http_api` (`crates/core/src/api/dataplane.rs:3390-3470`); fixtures through `include_bytes!("../../tests/fixtures/rar/two4.rar")` relative to `api/`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0087/E2E-044: The HTTP API refuses a version 4 cut alike
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/half4.rar` of E2E-008
- **When** the member extracts it through the HTTP API
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, as E2E-008; nothing written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0087_044_the_http_api_refuses_a_version_4_cut`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-052: The HTTP API refuses a version 5 encrypted names cut alike
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/uploads/hid222.rar` of E2E-028
- **When** the member extracts it through the HTTP API with `s3cret-Ünï`
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`; nothing written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0087_052_the_http_api_refuses_a_version_5_enc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0087/E2E-053: The HTTP API refuses a version 4 encrypted data cut alike
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/uploads/v4cut.rar` of E2E-019
- **When** the member extracts it through the HTTP API with `s3cret-Ünï`
- **Then** the refusal is that of E2E-052; nothing written
- Level / Driver / Location: functional e2e / HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` / crates/core/src/api/dataplane.rs tests (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0087_053_the_http_api_refuses_a_version_4_enc`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-044 | make `ensure_rar_complete` return `Ok(())` at once: the HTTP call succeeds, the test fails |
| E2E-052 | the E2E-044 mutation: `hid222.rar` extracts through the HTTP door, the test fails |
| E2E-053 | the E2E-044 mutation: the answer becomes the wrong password message, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Documentation describes what US-0198 to US-0203 built, nothing more

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-044, E2E-052, E2E-053): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` passes with SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 unchanged (`archive_rar.rs`), every `e2e_0085_` and `e2e_0086_` RAR test passes, and `cargo test --workspace` (default build) passes; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` as listed in Contracts.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0087 is green (`cargo test -p mcp-fs-core --lib --features rar -- e2e_0087_ && cargo test -p mcp-fs-core --lib -- e2e_0087_043`), and every requirement of SPEC-0087 is cited by at least one story with status done or landed.
- [ ] One commit `US-0204: cut RAR archives through the HTTP door and documentation`, body `Spec: SPEC-0087` and the requirement ids (SPEC-0087/FR-NEW-008).
