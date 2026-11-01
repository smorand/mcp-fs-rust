# Unknown held ids and the `ready` line: functional specification

> Id: SPEC-0068
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one observable behavior (what a fetch answer says about a held id the project lacks), one area (the git service), 2 requirements, no persisted change, and no code change expected: the service already answers this way.
> Status: specified
> Generated on: 2026-11-01
> Project: existing
> From backlog: BL-0118
> Depends on: SPEC-0067
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary
SPEC-0067 states that a held id the project does not have "appears in no line" of the answer. Git's own server, which SPEC-0067 makes the contract, says otherwise in one case: when such an id follows held commits that already satisfy every wanted commit, the answer tells the client it has enough with a `ready` line that names that id. This spec records the reference behavior as the contract and corrects the two statements of SPEC-0067 that contradict it. The service already behaves this way; no user sees a change.

## 2. Current State (MANDATORY, every depth)
### 2.1 What users can do today
- A developer fetching from a project states the commits it holds. For each held id the project lacks (well formed, but unknown to the project), the service sends no line saying the project holds it (row 1).
- When the held commits stated so far already satisfy every wanted commit and one more unknown id follows, the service answers `ACK <that id> ready`, once per such id, exactly as git's own server does (rows 1 and 2).
- When an unknown id comes before any held commit, or the held commits do not satisfy the wanted commits, the answer carries no `ready` line (row 1).
- Reference check made on 2026-11-01 with git 2.50.1 (Section 14.1, row 3) shows the same four answers listed in the tests below.

### 2.2 Specs governing this area
- SPEC-0067 (implemented): FR-NEW-001 makes git's answer the contract; FR-NEW-004 says an id the project does not hold is "treated as not stated"; E2E-038 says unknown ids "appear in no line". The last two contradict FR-NEW-001 in this one case. This spec modifies them (FR-MOD-001, E2E-MOD-001) and does not edit SPEC-0067.

### 2.3 What is verified today
- One test of SPEC-0067 expects the `ready` line naming an unknown id (E2E-038, on 20 000 objects) and one compares a mix of cases with git's own server (`ready_is_withheld_like_git_does`). Neither is tied to a requirement that says so. Files and command: Section 14.1.

## 3. Scope
### 3.1 In scope
- The reference behavior for held ids the project lacks, stated as requirements.
- The corrected wording of the two SPEC-0067 statements.
### 3.2 Out of scope, and why
- Any change of behavior: the service already answers as git does.
- Hiding the unknown id from `ready` lines (a deliberate difference from git): not chosen: the owner took the reference side (recommendation A); it needs new code and leaves the reference rule.
- Options the client chooses (BL-0119) and the cap on held ids (BL-0117): separate entries.
### 3.3 Constraints imposed by the context
- The answer lines are the contract with the git client, as in SPEC-0067 Section 3.3 (project owner: git compatibility). "The reference answer" keeps its SPEC-0067 meaning.

## 5. Usage scenarios
### SC-001: Unknown ids after enough held commits
**Actor:** Developer (through the git client)
**Preconditions:** project `demo` holds `c1` to `c6` on `main`; the client wants `c6` and holds `c4`; it also states ids `U1`, `U2`, `U3` that the project never held.
**Flow:**
1. The client states `c4`, then `U1`, `U2`, `U3` → the service acknowledges `c4`; since `c4` already satisfies the want, each following unknown id is answered by a `ready` line naming it → the round ends with `NAK`.
**Postconditions:** the client knows it has enough; no object is sent in this round; the project is unchanged.

### SC-002: Unknown ids when the held commits are not enough
**Actor:** Developer (through the git client)
**Preconditions:** project `demo` as above.
**Flow:**
1. The client states an unknown id before any held commit, or states only unknown ids, or holds a commit newer than the one it wants → the service sends no `ready` line.
**Postconditions:** the answer holds only the acknowledgements of held commits and the closing line; the project is unchanged.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: A `ready` line names an unknown id met after enough held commits
> WHEN a stated held id that is well formed and names no object of the project follows held commits that already satisfy every wanted commit, THE git service SHALL answer, at that point, the line `ACK <that id> ready`.
- **Business rules:** one such line per unknown id, in the order stated; "satisfy" means every wanted commit is a held commit or has one in its history; wanted objects that lead to no commit are skipped.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-O]: Unknown held ids are never acknowledged as held (modifies SPEC-0067/FR-NEW-004)
> IF a stated held id is well formed and names no object of the project, THEN THE git service SHALL NOT send an `ACK <id> common` line for it and SHALL NOT send `ACK <last held commit> ready` at the end of the exchange.
- **Before:** "treat it as not stated" (SPEC-0067 FR-NEW-004); with E2E-038 reading "appear in no line", which excludes the `ready` line of FR-NEW-001 above.
- **After:** the statement above; the only line that can name an unknown id is the `ready` line of FR-NEW-001. A malformed id and the id of an object that is not a commit stay "not stated" (SPEC-0067 FR-NEW-004 and FR-NEW-007, unchanged).
- **Reason:** git's own server withholds the closing `ready` once an unknown id was stated, and names the unknown id in the `ready` line it sends on the spot.

### 6.3 Removed
None.

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. Each test runs against project `demo` with commits `c1` to `c6` in a line on `main`, through a request written by hand with the lines a git client sends, and compares the answer with git's own server (SPEC-0067 Section 3.3). `U1`, `U2`, `U3` are well formed ids the project never held.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-002 | New | failure | SC-002 | FR-MOD-001, FR-NEW-001 | High |
| E2E-003 | New | failure | SC-002 | FR-MOD-001 | High |
| E2E-004 | New | failure | SC-002 | FR-MOD-001 | High |
| E2E-005 | New | journey | SC-001 | FR-NEW-001 | High |
| E2E-006 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-MOD-001 | Modified | edge | SC-001 | FR-NEW-001, FR-MOD-001 | High |

Coverage: happy 2 (001, 005), failure 3 (002, 003, 004), side effect 0 (the project is checked in each test), edge 2 (006, MOD-001), state transition 0 (the service keeps nothing between requests). Happy to failure ratio 2:3 (beats 1:1).

### 9.2 New tests
#### E2E-001: unknown ids after a satisfying held commit, round not ended
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** a request wanting `c6`, stating `c4`, then `U1`, then `U2` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <c4> common`, `ACK <U1> ready`, `ACK <U2> ready`, `NAK`, and equals the answer of git's own server
- **And** no object is sent; no `common` line names `U1` or `U2`; the project is unchanged
- **Priority:** Critical

#### E2E-002: an unknown id before any held commit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001, FR-NEW-001
- **Given** a request wanting `c6`, stating `U1`, then `c4`, then `U2` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <c4> common`, `ACK <U2> ready`, `ACK <c4>`, followed by exactly the 6 objects of `c5` and `c6`, and equals the answer of git's own server
- **And** no line names `U1`; the project is unchanged
- **Priority:** High

#### E2E-003: the held commit is newer than the wanted one
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001
- **Given** a request wanting `c4`, stating `c6`, then `U1` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c6> common` then `NAK`, with no `ready` line, and equals the answer of git's own server
- **And** no object is sent; the project is unchanged
- **Priority:** High

#### E2E-004: only unknown ids
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-MOD-001
- **Given** a request wanting `c6`, stating `U1` then `U2` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer is the single line `NAK`, and equals the answer of git's own server
- **And** no object is sent; the project is unchanged
- **Priority:** High

#### E2E-005: an unknown id between two held commits, round ended
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** a request wanting `c6`, stating `c3`, then `U1`, then `c4` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <c3> common`, `ACK <U1> ready`, `ACK <c4> common`, `ACK <c4>`, followed by exactly the 6 objects of `c5` and `c6`, and equals the answer of git's own server
- **And** the project is unchanged
- **Priority:** High

#### E2E-006: three unknown ids after a satisfying held commit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** a request wanting `c6`, stating `c4`, then `U1`, `U2`, `U3` as held, with no end of negotiation
- **When** the request is sent
- **Then** the answer has one `ready` line per unknown id, in the order stated (`U1`, `U2`, `U3`), between `ACK <c4> common` and the final `NAK`, and equals the answer of git's own server
- **And** no object is sent; the project is unchanged
- **Priority:** Medium

### 9.4 Modified tests
#### E2E-MOD-001: up to date request with unknown ids, on 20 000 objects (was SPEC-0067/E2E-038)
- **Validated before:** the unknown ids "appear in no line" (SPEC-0067 E2E-038 Then/And), while the test itself expects a `ready` line naming `U2`.
- **Validates now:** unknown ids are never named by a `common` line, and an unknown id met after the held tip is named by a `ready` line.
- **Given** a project of 20 000 objects with tip `T`; a request wanting `T`, stating `U1`, then `T`, then `U2` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer is, in this order, `ACK <T> common`, `ACK <U2> ready`, `ACK <T>`, carries 0 objects, and arrives in under 1 second
- **And** no line names `U1`
- **Priority:** High

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-MOD-001 | E2E-001, 005 | none: nothing fails when the held commits are enough; the refusing cases are SC-002 | E2E-006, MOD-001 |
| SC-002 | FR-MOD-001, FR-NEW-001 | none: nothing is declared ready, so there is no successful outcome to show | E2E-002, 003, 004 | none: each case is a failure of the "enough" condition |

Tests per requirement: FR-NEW-001 (001, 002, 005, 006, MOD-001); FR-MOD-001 (001, 002, 003, 004, MOD-001).

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Held id | an id a client states as already held | negotiation |
| Unknown id | a well formed id that names no object of the project | negotiation |
| Ready line | the line telling the client it has stated enough held commits to end the negotiation | negotiation |
| Enough | every wanted commit is a held commit or has one in its history | negotiation |
| Reference answer | as defined in SPEC-0067: the answer of git's own server for the same project data and request | tests |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline checklist, depth S) |

**Amendments applied:** none. Checklist read back from disk: no forced choice (order and multiplicity of the `ready` lines stated); every line spelled; the answer lines are the stated contract of SPEC-0067 Section 3.3, no other technical name outside Section 14; every test concrete and compared with git's own server; each requirement has 5 tests, failure 3 against happy 2; EARS clean, no forbidden modal in a requirement; Right nature: FEAT that restates existing behavior, no code change; claims cited (lines re-read); length 192 of 80 to 200. Tests written by the requirements author (depth S).
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)
### 14.1 Current State evidence
| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | an unknown well formed id is not acknowledged; after satisfying held commits it is named by `ACK <id> ready`; a flag withholds the final `ready` | `crates/core/src/git/http/negotiate.rs:221-266` (`acknowledge`), `:86-93` (the closing `ready` of a round that does not end the negotiation is sent only when no unknown id was stated, `!got_unknown` at `:89`) |
| 2 | one `ready` line per unknown id after enough commits | `crates/core/src/git/http/negotiate.rs:245-262` (one `ACK {oid} ready` at `:259` per unknown id, once the held commits so far satisfy every want) |
| 3 | reference answers | `git upload-pack --stateless-rpc` (git 2.50.1) run on 2026-11-01 on a six commit line: held,U1,U2 gives `ACK held common`, `ACK U1 ready`, `ACK U2 ready`, `NAK`; want older gives `ACK held common`, `NAK`; U1,held,U2 with done gives `ACK held common`, `ACK U2 ready`, `ACK held`; held,U1,held2 with done gives `ACK held common`, `ACK U1 ready`, `ACK held2 common`, `ACK held2` |
| 4 | tests that exist | `e2e_038_up_to_date_request_with_unknown_ids_besides_the_held_tip` in `crates/core/src/git/http/mod.rs`; `ready_is_withheld_like_git_does` in `crates/core/tests/git_negotiation_e2e.rs` |
Test command: `cargo test -p mcp-fs-core --test git_negotiation_e2e` and `cargo test -p mcp-fs-core --lib git::http`.

### 14.2 Class sweep
Not applicable (`Security: n/a`).

### 14.3 Design inputs
- No production change is expected. The tests E2E-001 to E2E-006 are guards that pass today; E2E-MOD-001 restates `e2e_038_…` (the expectation and name already match, add the "no line names U1" assertion). Their red-before-the-fix signal is a mutation: make `acknowledge` skip the `ready` line for unknown ids (E2E-001, 002, 005, 006 and MOD-001 fail) or send the final `ready` after an unknown id (E2E-001, 004 fail).
- Cases decided by git alone (an unknown id after a satisfying commit that was repeated, uppercase ids) are compared by the same harness and need no line here.
