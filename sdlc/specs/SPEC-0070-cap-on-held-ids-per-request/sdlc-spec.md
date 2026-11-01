# Cap on held ids per request: functional specification

> Id: SPEC-0070
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one observable behavior (what a request that states a very large number of held ids gets), one area (the git service), 3 requirements, no persisted change.
> Status: designed
> Generated on: 2026-11-01
> Project: existing
> From backlog: BL-0117
> Depends on: SPEC-0067
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary
A fetch request states the commits the client holds. Today the service reads every one of them, however many, so a single request can cost work proportional to its size and only the web framework's default size limit for a request body bounds it. After this change the service reads at most 5 000 held ids per request, in the order they appear, and treats the rest as not stated. A real client states a few hundred at most and sees no difference; a client that states more still gets a correct answer, only less incremental.

## 2. Current State (MANDATORY, every depth)
### 2.1 What users can do today
- The service reads every `have` line of a request, with no limit on their number (row 1). Each held id costs a lookup in the project's objects and, for a commit, a line in the answer (row 2).
- A request with 20 000 held ids is read in full and costs work proportional to 20 000 (row 1).
- A real git client resends the commits in common and a bounded batch of new ones in each round, a few hundred lines at most (project owner's recommendation, accepted; not measured here).
### 2.2 Specs governing this area
- SPEC-0067 (implemented): FR-NEW-001 to 004 define how held ids are read and answered; none bounds their number. BL-0117 recorded the gap. This spec adds a bound and modifies no requirement.
### 2.3 What is verified today
- The SPEC-0067 tests send requests with a handful of held ids, and one with 12; none sends thousands (row 3). Files and command: Section 14.1.

## 3. Scope
### 3.1 In scope
- The number of `have` lines read per request: 5 000, counted in order of appearance in the whole request.
- What the client sees when the limit is exceeded: the lines beyond it are treated as not stated, with no error.
- The time a very large request takes.
### 3.2 Out of scope, and why
- Refusing an oversized request with a message: not chosen, because the fetch has to keep working (owner decision, recommendation accepted).
- A limit on the size of the request in bytes: the framework's default applies and is not changed.
- A limit on the number of wanted lines: not requested (BL-0117 is about held ids).
### 3.3 Constraints imposed by the context
- The limit is 5 000 held ids per request (project owner); it is not configurable in this increment.
- The answer lines are git's, as in SPEC-0067 Section 3.3. For requests within the limit the answer is the reference answer; beyond it the answer is the one given for the request cut to its first 5 000 held ids.

## 5. Usage scenarios
### SC-001: A request within the limit
**Actor:** Developer
**Preconditions:** project `demo` with commits `c1` to `c6`; the request states 5 000 held ids or fewer.
**Flow:**
1. The client states what it holds → the service reads every held id and answers as in SPEC-0067.
**Postconditions:** nothing changes for the developer.

### SC-002: A request over the limit
**Actor:** A client or a script
**Preconditions:** as above; the request states more than 5 000 held ids.
**Flow:**
1. The client states more than 5 000 held ids → the service reads the first 5 000, ignores the rest, and answers as if the rest had not been stated.
**Postconditions:** no error; the answer is correct and can carry more objects than needed; the time stays under 1 second.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: Held ids beyond the limit are not read
> WHEN a request states more than 5 000 held ids, THE git service SHALL read the first 5 000 in the order they appear and SHALL treat the others as not stated, without reporting an error.
- **Business rules:** the limit counts lines, not distinct ids: a commit stated 6 000 times is acknowledged 5 000 times; ids malformed or of objects that are not commits count too; the order is the order of appearance in the whole request.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: Requests within the limit are read in full
> WHEN a request states 5 000 held ids or fewer, THE git service SHALL read all of them and answer as before this change.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified
None.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)
- NFR-001 [EARS-E]: WHEN a request states 20 000 held ids, THE git service SHALL answer in under 1 second.
  - Business rules: measured on a project of 6 commits in a development build, the way SPEC-0066 and SPEC-0067 measure their one second bounds.

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. Each test runs against a project served by the git service, through a real git client or a request written by hand with the lines a git client sends. Where the limit is not reached the answer is compared with git's own server; beyond it git's own server has no limit, so the answer is spelled out. Unknown ids are well formed ids the project never held.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-002 | Critical |
| E2E-002 | New | failure | SC-002 | FR-NEW-001 | Critical |
| E2E-003 | New | failure | SC-002 | FR-NEW-001, NFR-001 | High |
| E2E-004 | New | edge | SC-002 | FR-NEW-001 | Medium |
| E2E-005 | New | journey | SC-001 | FR-NEW-002 | High |
| E2E-006 | New | edge | SC-001 | FR-NEW-002 | Medium |
| E2E-007 | New | failure | SC-002 | FR-NEW-001, NFR-001 | High |
| E2E-008 | New | failure | SC-002 | FR-NEW-001, NFR-001 | Medium |

Coverage: happy 2, failure 4, edge 2, side effect 0 (the project is checked in each test), state transition 0 (the service keeps nothing between requests). Happy to failure ratio 2:4 (beats 1:1).

### 9.2 New tests
#### E2E-001: exactly the limit, the held commit last
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating 4 999 well formed ids the project never held, then `c4`, as 5 000 held ids in all, ending the negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c4> common`, `ACK <c4>`, then exactly the 6 objects of `c5` and `c6`, and equals the answer of git's own server
- **And** no error is reported; the project is unchanged
- **Priority:** Critical

#### E2E-002: one over the limit, the held commit last
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating 5 000 well formed ids the project never held, then `c4`, as 5 001 held ids in all, ending the negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK` followed by all 18 objects reachable from `c6`, as if `c4` had not been stated
- **And** no error is reported; no line names `c4`; the project is unchanged
- **Priority:** Critical

#### E2E-003: 20 000 unknown held ids
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, NFR-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating 20 000 well formed ids the project never held, ending the negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK` followed by all 18 objects reachable from `c6`, and arrives in under 1 second
- **And** no error is reported; the project is unchanged
- **Priority:** High

#### E2E-004: the held commit repeated 6 000 times
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating `c4` 6 000 times as held, ending the negotiation
- **When** the request is sent
- **Then** the answer holds exactly 5 000 lines `ACK <c4> common`, then the line `ACK <c4>`, then exactly the 6 objects of `c5` and `c6`
- **And** no error is reported; the project is unchanged
- **Priority:** Medium

#### E2E-005: a real fetch is unaffected
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a copy made by a real git client that then made 40 local commits; the project gains `c7`
- **When** the developer runs `git fetch`
- **Then** the copy holds `c7` on `origin/main` and keeps its 40 local commits; the integrity check reports no problem
- **And** the objects received are exactly those of `c7`; the project is unchanged
- **Priority:** High

#### E2E-006: a small request is read in full
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating `c3`, `c4` and an id the project never held as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer equals the answer of git's own server for the same request, line for line (the held ids are `c3`, `c4` and the unknown id, in that order)
- **And** no error is reported; the project is unchanged
- **Priority:** Medium

#### E2E-007: the held commit repeated 20 000 times, round not ended
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, NFR-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating `c4` 20 000 times as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is 5 000 lines `ACK <c4> common`, then `ACK <c4> ready`, then `NAK`, and arrives in under 1 second
- **And** no object is sent; no error is reported; the project is unchanged
- **Priority:** High

#### E2E-008: 20 000 ids of a folder listing
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, NFR-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` stating the id of the top folder listing of `c4` 20 000 times as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK` followed by all 18 objects reachable from `c6`, and arrives in under 1 second
- **And** no error is reported; the project is unchanged
- **Priority:** Medium

### 9.4 Modified tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-002 | E2E-001, 005 | none: a request within the limit has no refusal | E2E-006 |
| SC-002 | FR-NEW-001, NFR-001 | none: reading less than asked has no successful outcome to show | E2E-002, 003, 007, 008 | E2E-004 |

Tests per requirement (each at least 3): FR-NEW-001 (002, 003, 004, 007, 008); FR-NEW-002 (001, 005, 006); NFR-001 (003, 007, 008).

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Held id | an id a client states as already held | negotiation |
| Unknown id | a well formed id that names no object of the project | negotiation |
| Limit | the largest number of held ids read from one request, 5 000 | negotiation |
| Not stated | read as if the client had not written it | negotiation |
| Reference answer | as defined in SPEC-0067 | tests |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline checklist, depth S) |

**Amendments applied:** none. Checklist read back from disk: no forced choice (limits, order, defaults and messages stated); every line spelled; the option and answer lines are the contract of SPEC-0067 Section 3.3 (git compatibility), no other technical name outside Section 14; every test concrete; each requirement has 3 or more tests, failure 4 against happy 2; EARS clean, no forbidden modal in a requirement; FEAT that changes what a client observes; claims cited, lines re-read after drafting. Tests written by the requirements author (depth S).
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)
### 14.1 Current State evidence
| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | every `have` line is kept, no count is checked | `crates/core/src/git/http/negotiate.rs:46-47` (`req.haves.push`), the loop at `:33-60` |
| 2 | each held id costs an object lookup, a commit a line | `crates/core/src/git/http/negotiate.rs:221-266` (`acknowledge`) |
| 3 | tests send few held ids | `e2e_011_twelve_unknown_held_commits` in `crates/core/tests/git_negotiation_e2e.rs` (12 ids, the largest list) |
| 4 | the only bound is the framework's default request body limit (2 MiB, ASSUMED from the framework's documentation; `rg -n DefaultBodyLimit crates/core/src` finds no override for the fetch route) on the fetch route; the push route sets its own | `crates/core/src/git/http/mod.rs:99-105` (the fetch route has no body limit layer, the push route has `DefaultBodyLimit::max(max_pack)` at `:103`); not changed here |
Test command: `cargo test -p mcp-fs-core --test git_negotiation_e2e`.

### 14.2 Class sweep
Not applicable (`Security: n/a`).

### 14.3 Design inputs
- The cap belongs where the lines are read (`parse` in `negotiate.rs`), so that `acknowledge` and the `ready` rule see only the first 5 000. Signal: E2E-002, 004 fail until it does.
- `Request.haves` is read after the whole request is parsed; a request of 20 000 lines is parsed in full before the cap applies unless `parse` stops storing at 5 000. Parsing is linear and cheap; the cost the cap bounds is the object lookups. Signal: E2E-003, 007, 008 timing.
- The hand written request builder of the harness (`request`) takes a slice of ids: a 20 000 id list is built by the test.
- 20 000 lines of about 50 bytes are about 1 MB, under the framework's default body limit, so E2E-003, 007 and 008 reach the service; a request over that limit is refused before the service reads it (unchanged). NFR-001 is measured on a small project; the 20 000 object projects of SPEC-0067 need no additional case.
