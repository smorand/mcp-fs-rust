# US-0136: answer the end of a negotiation like git does

> Spec: SPEC-0067 (sdlc/specs/SPEC-0067-incremental-fetch-negotiation/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0135
> min_tier: 1
> Files touched: 4
> Closes spec: no

## Objective
A fetch ends with the closing line and only the objects the client lacks. This story reads the whole request (haves after the wanted lines, the end marker), acknowledges held commits, hides git's edge commits and sends the missing objects. It is the story with the most decisions: min_tier 1.

## Requirements covered
- SPEC-0067/FR-NEW-001: WHEN a client states wanted and held commits without ending the negotiation, THE git service SHALL answer with the acknowledgement lines of the reference answer, in the same order, and no object data.
- SPEC-0067/FR-NEW-002: WHEN the client ends the negotiation, THE git service SHALL answer with the acknowledgement lines and the closing line of the reference answer, followed by the same set of objects as the reference answer.
- SPEC-0067/FR-NEW-004: IF a held id is not a well formed object id or is the id of no object of the project, THEN THE git service SHALL treat it as not stated and SHALL NOT report an error.
- SPEC-0067/FR-NEW-005: THE git service SHALL NOT change any data of the project while answering a clone or a fetch.
- SPEC-0067/FR-NEW-006: THE git service SHALL apply to a clone and to a fetch the same access rules as before this change.
- SPEC-0067/FR-NEW-007: IF a held id is a well formed id of an object the project holds and that object is not a commit, THEN THE git service SHALL NOT acknowledge it and SHALL treat it as not stated, without reporting an error.
- SPEC-0067/NFR-001: WHEN a client asks for a commit it holds on a project of 20 000 objects, THE git service SHALL answer in under 1 second and send no object (baseline: 1.6 to 1.8 seconds before SPEC-0066, development build; not re-measured since).

## Independent Test
`cargo test -p mcp-fs-core --test git_negotiation_e2e` is green for the story's tests; E2E-002 asserts the transfer holds exactly 6 objects against the reference program's set.

## Design excerpt
### Decisions
- DEC-001: implement the negotiation in a new module over libgit2 (revwalk `hide`, packbuilder). Obliged by: SPEC-0067/FR-NEW-001, FR-NEW-002. Rationale: reuses what the service already uses to build packs. Alternatives considered: DEC-002, DEC-003. Source: design. Evidence: `crates/core/src/git/http/mod.rs:460-494`.
- DEC-003: keep libgit2's packbuilder for object selection, hide exactly git's edge commits (the uninteresting parents of included commits), not every held commit. Obliged by: SPEC-0067/FR-NEW-002. Rationale: git marks the trees of edge commits only (`mark_edges_uninteresting`); libgit2's `insert_walk` marks the trees of the commits hidden (`pack-objects.c:1684-1697` in libgit2-sys 0.18.8), so hiding only edges reproduces git. Alternatives considered: hide every held commit (differs when two held commits are on one line, E2E-007). Source: design, auditor finding. Evidence: spec 14.3.
- DEC-004: acknowledgements follow git's `get_common_commits` (stateless, `multi_ack_detailed`): one `ACK <id> common` per have the project holds as a commit, repeats included; at the end of the haves, without `done`: `ACK <last> ready` when at least one held commit, no unknown well formed have, and every wanted commit reaches a held commit, then `NAK`; with `done`: `ACK <last>` or `NAK`, then the objects. Obliged by: SPEC-0067/FR-NEW-001, FR-NEW-011. Rationale: verified on 2026-10-31 against git 2.50.1: duplicates are acknowledged twice; an unknown have suppresses `ready`; a want that does no
- DEC-005: declared differences are applied while reading the haves: a have that is not 40 hexadecimal characters is skipped; a well formed id of an object that is not a commit is skipped; neither acknowledges nor suppresses `ready`; a well formed id the project does not hold suppresses `ready` and is not acknowledged (as the reference). Obliged by: SPEC-0067/FR-NEW-004, FR-NEW-007. Rationale: matches "treat as not stated" for the two cases the spec declares, and the reference for the rest. Alternatives considered: treat unknown well formed ids as skipped too (rejected: differs from the refere
### Data model
None.
### Contracts
`negotiate::parse`, `negotiate::answer` and the two reader methods of Section 5 of the design (pasted here): `PktReader::at_end(&self) -> bool`, `PktReader::last_was_cut(&self) -> bool`; `pub(super) struct Request {{ wants, haves, have_lines, done, option_line }}`; `pub(super) fn parse(body: &[u8]) -> Request`; `pub(super) fn answer(repo: &git2::Repository, ref_values: &[String], req: &Request) -> Result<Vec<u8>>`. `handle_upload_pack` keeps `(entry: &GitRepoEntry, body: &[u8]) -> Result<Vec<u8>>`. Wire lines: `ACK <id> common`, `ACK <id>`, `NAK`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/negotiate.rs` | create |
| `crates/core/src/git/http/mod.rs` | modify |
| `crates/core/src/git/http/pktline.rs` | modify |
| `crates/core/tests/git_negotiation_e2e.rs` | modify |
### Existing patterns
`crates/core/src/git/http/mod.rs:398-418` (parser to keep compatible), `:421-449` (handler), `:460-494` (`build_pack`: revwalk, `insert_walk`, `empty_pack` when the pack is empty), `:312` (`concrete_refs`, the ref values for DEC-006).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> Reference answer = the answer of `git upload-pack --stateless-rpc` on a bare copy of the same project data, using only the options the service offers (`multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta`); compare acknowledgement lines line by line and objects as a set.

### SPEC-0067/E2E-002: fetch of two new commits
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** a copy of `demo` at `c4`; the project then gains `c5` (adds `c.txt`) and `c6` (edits `a.txt`)
- **When** the developer runs `git fetch`
- **Then** the copy holds 6 commits, `origin/main` is at `c6`, the integrity check reports no problem
- **And** the objects sent are exactly the 6 of `c5` and `c6` (for each: the commit, its folder listing, the changed file), the same set as the reference answer's
- **Priority:** Critical
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-003: fetch with nothing new
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-005
- **Given** a copy of `demo` at `c4` and the project still at `c4`
- **When** the developer runs `git fetch`; then a request wanting `c4`, stating `c4` as held, then ending the negotiation is sent
- **Then** git reports nothing to fetch and the copy is unchanged; the answer to the request equals the reference answer and carries 0 objects
- **And** the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-004: the copy holds commits the project lacks
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** a copy at `c4` with 3 local commits `l1`, `l2`, `l3` on top; the project gains `c5` (adds `d.txt`)
- **When** the developer runs `git fetch`
- **Then** the copy holds `c5` on `origin/main` and still holds `l1` to `l3`
- **And** the objects sent are exactly the 3 of `c5`, the same set as the reference answer's
- **Priority:** Critical
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-005: forty local commits, two in common
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** a copy with 40 local commits of which only `c3` and `c4` are on the project; the project is at `c6`
- **When** the developer runs `git fetch`
- **Then** the copy holds `c5` and `c6` and keeps its 40 commits; the integrity check reports no problem
- **And** every exchange equals the reference exchange, line by line; the objects sent are exactly those of `c5` and `c6`
- **Priority:** Critical
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-006: merge history with one side held
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** `main` at merge commit `m1` joining `c4` and side commit `s1` (forks from `c3`, adds `side.txt`); a copy that holds `c4` only
- **When** the developer runs `git fetch`
- **Then** the copy holds `s1` and `m1`
- **And** the objects sent are the same set as the reference answer's and include no object that belongs to the contents of `c4`
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-007: a file deleted long ago and added back
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** `c1` adds `old.txt` (content `old`), `c2` deletes it, `c3` adds `other.txt`; a copy holds `c3`; the project gains `c4`, which adds `old.txt` with content `old` again
- **When** the developer runs `git fetch`
- **Then** the copy holds `c4` and the integrity check reports no problem
- **And** the objects sent are the same set as the reference answer's, whatever it is (this case is where git and a simpler walk differ)
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-011: twelve unknown held commits
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004
- **Given** a request wanting `c6` that states 12 well formed commit ids the project never held, then ends the negotiation
- **When** the request is sent
- **Then** the answer equals the reference answer: the closing line and every object reachable from `c6`
- **And** none of the 12 ids appears in an acknowledgement line
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-014: a held id that is not a valid id
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-004
- **Given** a request wanting `c6` that states one held id which is not a valid commit id (the text `not-an-id`), then ends the negotiation
- **When** the request is sent
- **Then** the answer is as if that id had not been stated, with no error
- **And** the id appears in no line of the answer
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-021: anonymous caller allowed when anonymous reading is on
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** journey; **Scenario:** SC-006; **Requirements:** FR-NEW-006
- **Given** anonymous reading on, a copy at `c4` made without identity, the project at `c6`
- **When** the caller runs `git fetch`
- **Then** the copy holds `c5` and `c6`
- **And** the objects sent are exactly the 6 of `c5` and `c6`
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-029: held id of a file version
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007
- **Given** `demo` with `c1` to `c4`, and `a.txt` in `c4` with content `a`; a request wanting `c4`, stating the id of that `a.txt` as held, then ending the negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id and no error is reported
- **And** the objects sent are every object reachable from `c4`; the project is unchanged
- **Priority:** High
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-032: held id with only ten characters
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-004
- **Given** a request wanting `c6`, stating the first 10 characters of the id of `c4` as held, then ending the negotiation
- **When** the request is sent
- **Then** the answer is as if that id had not been stated, with no error
- **And** the id appears in no line of the answer
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-033: held id of a folder listing
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007
- **Given** a request wanting `c4`, stating the id of the top folder listing of `c4` as held, then ending the negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id and no error is reported
- **And** the objects sent are every object reachable from `c4`
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-034: held id of an annotated tag
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007
- **Given** annotated tag `v1` on `c4`; a request wanting `c6`, stating the id of the tag `v1` as held, then ending the negotiation
- **When** the request is sent
- **Then** no acknowledgement line contains that id and no error is reported
- **And** the project is unchanged
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

### SPEC-0067/E2E-038: up to date request with unknown ids besides the held tip
- Level / Driver / Location: functional e2e / real `git` client or a hand written request sent over HTTP to an in-process server; the reference answer from `git upload-pack --stateless-rpc` on a bare copy of the same project data / `crates/core/tests/git_negotiation_e2e.rs`
- **Category:** edge; **Scenario:** SC-003; **Requirements:** NFR-001, FR-NEW-004
- **Given** a project of 20 000 objects with tip `c20000`; a request wanting `c20000` and stating `c20000` and two well formed ids the project never held, then ending the negotiation
- **When** the request is sent
- **Then** the answer arrives in under 1 second and equals the reference answer, with 0 objects
- **And** the unknown ids appear in no line
- **Priority:** Medium
- Cleanup: temp directories and the in-process server are dropped with the test.

## Constraints
- Files not to touch: `crates/core/src/git/http/mod.rs` route and access code (`:96-260`), `crates/core/src/git/http/mod.rs` receive-pack code, any `spec.md`.
- Dependencies not to add: none (git2, axum, tokio, reqwest are already in the manifest).
- Patterns to avoid: running the `git` program in production code (DEC-002); dashes as punctuation in code, comments or docs.
- Scope boundary: behaviors of later stories (see Depends on and the story list in the design) stay as they are today.

## Definition of done
- [ ] RED evidence recorded. Write the story's tests first and record them failing for the stated reason before the production code (a failing assertion, not a compile error).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing tests of `crates/core/src/git/http/mod.rs` (`parses_wants_and_haves`, `parses_an_empty_upload_pack_request`, `upload_pack_without_wants_answers_a_bare_flush`, the `handle_upload_pack` round trips) and the three real clones of `crates/core/tests/full_stack_e2e.rs` keep passing unchanged.
- [ ] Docs updated: none in this story (US-0141).
- [ ] Self review: full (complexity of the critical path stated: the answer is O(included commits + objects); no nested loop over wants and haves beyond O(W x H) reachability checks, bounded by the number of wanted ids).

- [ ] One commit `US-0136: answer the end of a negotiation like git does`, body `Spec: SPEC-0067` and the requirement ids.
