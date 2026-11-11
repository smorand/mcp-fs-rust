# US-0234: wrong version 4 passwords through the HTTP door and documentation

> Spec: SPEC-0090 (sdlc/specs/SPEC-0090-tell-a-wrong-password-on-rar4-encrypted-names/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0233
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Prove the HTTP API answers a wrong version 4 password, a damaged first block and cuts exactly as the assistant tool does, and close the spec: the documentation states that the password is judged on the first protected block and that the old gap is closed.

## Requirements covered
- SPEC-0090/FR-NEW-007: THE archive extraction SHALL give, through the HTTP API, the same code and message as through the assistant tool for every answer of this specification.
- SPEC-0090/FR-NEW-002 (partial: through the HTTP API): IF a version 4 RAR archive declares encrypted file names AND a password that is not empty is given AND the start of the first protected block is a possible block start AND the whole block is present AND it fails its integrity check THEN THE archive extraction SHALL refuse with `ERR_PASSWORD_REQUIRED` and `incorrect password for this archive; call again with the correct 'password' parameter`.
- SPEC-0090/FR-NEW-003 (partial: through the HTTP API): IF a version 4 RAR archive declares encrypted file names in its whole leading part AND a password that is not empty is given AND the archive ends before the start of the first protected block is whole THEN THE archive extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and `archive is corrupt or not a valid rar file: unexpected end of archive`, whatever the password.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0090_032 e2e_0090_033 e2e_0090_034`: 3 tests pass. Real assertions: HTTP status and body, nothing written. Then the closing checks below.

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0090):** WRONG is `password_required_message(Some(password))` and CUT is `corrupt("rar", TRUNCATED)`, the existing constructors, so both doors and the logging stay as they are. **Obliged by:** SPEC-0090/FR-NEW-001 to FR-NEW-004, FR-NEW-006, FR-NEW-007. **Rationale:** one wording per answer.
- **DEC-007 (SPEC-0090):** the closing story rewrites `.agent_docs/tools.md:231-232` (the BL-0136 gap closed), documents the first block judgment in `AGENTS.md` (`archive_rar.rs` sentence) and `README.md`, and adds the `e2e_0090_` filter to `.agent_docs/testing.md`. The doc comment of `walk_rar4_encrypted` is rewritten in US-0231 with the code. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** house closing pattern.
### Contracts
`POST /api/fs/{mount}/extract-archive` `{"path": "/uploads/hp4.rar", "password": "nope"}`: status of `ERR_PASSWORD_REQUIRED` as today's REST mapping gives it, body `error` `ERR_PASSWORD_REQUIRED`, detail `ERR_PASSWORD_REQUIRED: incorrect password for this archive; call again with the correct 'password' parameter`; CUT: status 400, `ERR_INVALID_ARGUMENT`, the truncation detail.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (tests `e2e_0090_032`, `033`, `034`, `#[cfg(feature = "rar")]`) |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify (`:231-232`) |
| `.agent_docs/testing.md` | modify |
### Existing patterns
`assert_cut_refused` and `e2e_0087_053_the_http_api_refuses_a_version_4_enc` (`crates/core/src/api/dataplane.rs:3675-3712`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0090/E2E-032: HTTP API, wrong password
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/uploads/hp4.rar`
- **When** the member extracts it through the HTTP API with `nope`
- **Then** the refusal is WRONG, the same code and message as E2E-003; nothing written
- Level / Driver / Location: functional e2e / HTTP door / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0090_032_http_api_wrong_password`
- Green on arrival: US-0231 changed the engine both doors share; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-033: HTTP API, damaged first block and a cut, right password
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007, FR-NEW-002; **Priority:** High
- **Given** `/uploads/hp4d.rar` of E2E-014 and `/uploads/hp4half.rar` of E2E-007
- **When** the member extracts each through the HTTP API with `s3cret-Ünï`
- **Then** the first refusal is WRONG, the second CUT, as through the assistant tool; nothing written
- Level / Driver / Location: functional e2e / HTTP door / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0090_033_http_api_damaged_first_block_and_a_cu`
- Green on arrival: as E2E-032; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0090/E2E-034: HTTP API, cut before the start
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-007, FR-NEW-003; **Priority:** Medium
- **Given** `/uploads/hp4t.rar` of E2E-010
- **When** the member extracts it through the HTTP API with `nope`
- **Then** the refusal is CUT; nothing written
- Level / Driver / Location: functional e2e / HTTP door / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0090_034_http_api_cut_before_the_start`
- Green on arrival: a short start is CUT at both doors; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-032 | in the REST error mapping, map `ERR_PASSWORD_REQUIRED` to `ERR_INVALID_ARGUMENT`: the error differs, the test fails |
| E2E-033 | as E2E-032 |
| E2E-034 | as US-0232 E2E-010 (zero padded start): WRONG instead of CUT, the test fails |

### Closing checks (spec converge)
- Every acceptance test of SPEC-0090 green, run by `make test` with the `rar` feature (`./test.sh` runs it): 36 `E2E-` and 2 `E2E-MOD-` tests (`rg -c 'fn e2e_0090_' crates/core/src` totals 38) and `rar4_first_encrypted_header_judges_the_password`.
- Requirement coverage audit: `comm -23 <(rg -o 'FR-(NEW|MOD)-[0-9]{3}' sdlc/specs/SPEC-0090-*/sdlc-spec.md | sort -u) <(rg -o 'SPEC-0090/FR-(NEW|MOD)-[0-9]{3}' sdlc/stories/US-023[1-4]-*.md -N --no-filename | sed 's#.*/##' | sort -u)` prints nothing, and US-0231 to US-0233 are `done` or `landed` in `sdlc/stories/_queue.md`.

## Constraints
- Files not to touch: every production file, the committed archives, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Scope boundary: damaged encrypted data with the right password (spec 3.2, D-7) is a side finding, never filed

## Definition of done
- [ ] Green on arrival tests (E2E-032, E2E-033, E2E-034): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` clean; `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_ e2e_0087_ e2e_0090_`
- [ ] Docs updated: `README.md` (RAR passwords: a wrong version 4 password with encrypted names is told as such), `AGENTS.md` (`archive_rar.rs` sentence: the first protected block judges a version 4 password), `.agent_docs/tools.md` (`:231-232`, the BL-0136 gap closed, the accepted limit), `.agent_docs/testing.md` (filter `e2e_0090_`, the parallel E2E-020), each entry dated (DEC-007).
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E- and E2E-SEC- of SPEC-0090), and every requirement of SPEC-0090 cited by at least one story with status done or landed (the audit above).
- [ ] One commit `US-0234: wrong version 4 passwords through the HTTP door and documentation` on its story branch `story/US-0234-wrong-version-4-passwords-through-the-ht`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0090-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0090` and the requirement ids (SPEC-0090/FR-NEW-007, FR-NEW-002, FR-NEW-003).
