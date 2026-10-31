# US-0135: build the oracle harness and the baseline guards

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
The negotiation work is verified against git's own server. This story builds the harness that makes that possible (an in-process server, real `git` clients, a request builder, the reference program, an answer parser that compares acknowledgement lines and the object set) and records, as tests that pass today, the behaviors that must not change: clone, wants only, empty project, unknown or malformed wanted ids, empty request, access rules, reads that change nothing.

## Requirements covered
- SPEC-0067/FR-NEW-003: WHEN the request contains no `have` line, THE git service SHALL send every object reachable from the wanted commits, whether or not the client has ended the negotiation.
- SPEC-0067/FR-NEW-005: THE git service SHALL NOT change any data of the project while answering a clone or a fetch.
- SPEC-0067/FR-NEW-006: THE git service SHALL apply to a clone and to a fetch the same access rules as before this change.
- SPEC-0067/FR-NEW-008: IF a wanted id is malformed, is the id of no object of the project, or is the id of an object that no branch or tag reaches, THEN THE git service SHALL send no data for it and SHALL NOT report an error.
- SPEC-0067/FR-NEW-010: IF a request is empty, THEN THE git service SHALL answer as it does today for a request that wants nothing, with no error. IF a request ends in the middle of a line, THEN THE git service SHALL discard the cut line and everything after it and answer the lines before it by every other requirement, with no error. IF a request, once cut, contains no wanted line, or contains none at all, THEN THE git service SHALL answer as it does today for a request that wants nothing, with no error. IF a request carries a shallow, deepen or filter line and contains no wanted line, THEN THE git service SHALL answer as it does today for a request that wants nothing (a bare flush, no `NAK`), this requirement taking precedence over FR-NEW-009.

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e` is green; breaking the answer for a wants only request (for example answering a bare flush) turns E2E-012 red, which proves the harness compares real answers.

## Design excerpt
### Decisions
- DEC-007: the tests compare with the reference program and with real clients over an in-process server; the `git` program is a test dependency only. Obliged by: SPEC-0067/FR-NEW-001, FR-NEW-002 and all E2E (spec 3.3: "equals the reference answer"). Rationale: the contract is the reference; enumerating cases by hand is what five audits showed does not converge. Alternatives considered: golden bytes recorded once (rejected: hides drift with new git versions). Source: spec constraint. Evidence: `crates/core/tests/full_stack_e2e.rs:94` (`require_git_cli` pattern), `app::build` at `crates/core/src
- DEC-008: the wire e2e tests live in one new integration test file with a server booted by `app::build` on SQLite and local blobs (default config), anonymous reading switched per test; projects are created and filled through the real `git push` with a minted admin token. Obliged by: SPEC-0067/FR-NEW-005, FR-NEW-006 and all E2E. Rationale: crosses the same wire as users; no crate private helper. Alternatives considered: extend `full_stack_e2e` (rejected: needs PostgreSQL and MinIO, `#[ignore]`). Source: design. Evidence: `crates/core/tests/full_stack_e2e.rs:369-371`.
### Data model
None.
### Contracts
Test only. Helpers to create in the test file: `struct Server` (boots `app::build` on `127.0.0.1:0` with SQLite and local blobs, `git.enabled = true`, `git.anonymous_read` per test, keys as in `full_stack_e2e`), `fn push_history(...)`, `fn request(wants, haves, done, caps) -> Vec<u8>` (pkt-lines), `fn post_upload_pack(server, project, &[u8]) -> Vec<u8>`, `fn reference_answer(bare: &Path, &[u8]) -> Vec<u8>` (runs `git upload-pack --stateless-rpc`), `struct Answer {{ ack_lines: Vec<String>, objects: BTreeSet<String> }}` and `Answer::parse(&[u8])` (reads pkt-lines until the pack, demultiplexes side-band channel 1, indexes the pack into a temp repository with git2 and lists its objects), `fn assert_same_answer(ours, reference)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/tests/git_negotiation_e2e.rs` | create |
### Existing patterns
`crates/core/tests/full_stack_e2e.rs:298-320` (`git`, `git_ok` run the `git` program with `http.extraHeader=Authorization: Bearer <token>`), `:369-372` (boot the router on an ephemeral port), `:374` (`keys::mint_token_from_file`), `:161-250` (`establish_session`, `call_tool` for `admin.create_project`), `:94` (`require_git_cli`). Pkt-line helpers: `crates/core/src/git/http/pktline.rs` (`encode`, `flush`, `PktReader`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

### SPEC-0067/E2E-001: clone of a four commit project
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-003, FR-NEW-005
- **Given** project `demo` with commits `c1` to `c4` on `main` (`a.txt` added in `c1`, `b.txt` in `c3`) and a developer who is a member
- **When** the developer runs `git clone` on `demo`
- **Then** the copy has 4 commits with `main` at `c4` and the integrity check reports no problem
- **And** the objects sent are the same set as the reference answer's; the project is unchanged
- **Priority:** Critical
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-012: wants and nothing else
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003
- **Given** a request wanting `c6`, with no held id and no end of negotiation
- **When** the request is sent
- **Then** the answer carries every object reachable from `c6`, as before this change
- **And** no error is reported
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-013: empty project
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003
- **Given** a project with no commit
- **When** the developer runs `git clone`
- **Then** git prints `You appear to have cloned an empty repository.` and the copy has no commit
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-015: wanted object unknown to the project
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008
- **Given** a request wanting an object id that is well formed and not held by the project
- **When** the negotiation ends
- **Then** the answer is the line `NAK` with zero objects and no error, exactly as before this change
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-016: wanted object that is not an id
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008
- **Given** a request wanting the text `xyz`
- **When** the negotiation ends
- **Then** the answer is the line `NAK` with zero objects and no error
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-017: empty request
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-010
- **Given** a request with no content
- **When** the request is sent
- **Then** the answer is the one given today to a request that wants nothing: no object, no error
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-019: a non member is refused
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-006
- **Given** a caller with an identity who is not a member of `demo`
- **When** the caller runs `git fetch` on `demo`
- **Then** access is refused exactly as before this change
- **And** no object is sent
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-020: anonymous caller refused when anonymous reading is off
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-006
- **Given** anonymous reading off and a caller with no identity
- **When** the caller runs `git clone` on `demo`
- **Then** access is refused exactly as before this change
- **And** no object is sent
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-022: reads change nothing
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** side effect; **Scenario:** SC-001 to SC-004; **Requirements:** FR-NEW-005
- **Given** project `demo` at `c4`, with its branches, tags, audit entries and trash listed
- **When** a clone, then a fetch with nothing new, then a fetch with 2 new commits are done
- **Then** branches and tags are identical apart from the 2 commits the test added, and the trash is unchanged
- **And** the audit log has no new entry from the three reads
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

## Constraints
- Files not to touch: `crates/core/src/git/http/mod.rs` route and access code (`:96-260`), `crates/core/src/git/http/mod.rs` receive-pack code, any `spec.md`.
- Dependencies not to add: none (git2, axum, tokio, reqwest are already in the manifest).
- Patterns to avoid: running the `git` program in production code (DEC-002); dashes as punctuation in code, comments or docs.
- Scope boundary: behaviors of later stories (see Depends on and the story list in the design) stay as they are today.

## Definition of done
- [ ] RED evidence recorded. These tests describe behavior that holds today, so the RED evidence is a mutation per test group: make `handle_upload_pack` answer a bare flush for a wants only request (E2E-012 red), return `NAK` without a pack (E2E-001 red), and skip the membership check (E2E-019 red); revert each mutation.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing tests of `crates/core/src/git/http/mod.rs` (`parses_wants_and_haves`, `parses_an_empty_upload_pack_request`, `upload_pack_without_wants_answers_a_bare_flush`, the `handle_upload_pack` round trips) and the three real clones of `crates/core/tests/full_stack_e2e.rs` keep passing unchanged.
- [ ] Docs updated: none in this story (US-0141).
- [ ] Self review: full (complexity of the critical path stated: the answer is O(included commits + objects); no nested loop over wants and haves beyond O(W x H) reachability checks, bounded by the number of wanted ids).

- [ ] One commit `US-0135: build the oracle harness and the baseline guards`, body `Spec: SPEC-0067` and the requirement ids.
