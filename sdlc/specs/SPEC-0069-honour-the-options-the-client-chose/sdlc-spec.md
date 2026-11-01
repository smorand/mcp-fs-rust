# Honour the options the client chose: functional specification

> Id: SPEC-0069
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one observable behavior (what the service offers and does when a client does not ask for the side band), one area (the git service), 3 requirements, no persisted change.
> Status: implemented
> Generated on: 2026-11-01
> Project: existing
> From backlog: BL-0119
> Depends on: SPEC-0067
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary
A git client tells the service which options it wants. The service always answers in the form of the most common client: it frames the objects on a side band and uses the detailed acknowledgements, even when the client asked for neither. After this change a client that does not ask for the side band receives the objects unframed, as it does from git's own server, and the service stops announcing the plain `multi_ack` option that no client can use over HTTP. A real git client sees no difference.

## 2. Current State (MANDATORY, every depth)
### 2.1 What users can do today
- The service announces `multi_ack`, `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and its agent text (row 1). It honours `multi_ack_detailed` and `side-band-64k`; the plain `multi_ack` is announced but never honoured: a request asking only for it gets the detailed answer (rows 2 and 3).
- The objects are always sent framed on a side band and followed by a closing byte and a flush, whether the client asked for the side band or not (row 4). Git's own server sends them unframed when the side band was not asked for (row 5).
- Real git clients over HTTP always ask for `multi_ack_detailed` and `side-band-64k` (git refuses to negotiate over HTTP without the first), so none is affected today.
### 2.2 Specs governing this area
- SPEC-0067 (implemented): Section 3.3 and FR-NEW-012 name `multi_ack` among the options the service offers; BL-0119 records that only two of the four are honoured. This spec modifies FR-NEW-012 (FR-MOD-001) and does not edit SPEC-0067.
### 2.3 What is verified today
- Real clones and fetches are exercised by the SPEC-0067 tests (they ask for the side band). No test sends a request without it, and no test checks the announced options (row 6). Command and files: Section 14.1.

## 3. Scope
### 3.1 In scope
- The announced list of options: `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and the agent text.
- Objects sent unframed when the client did not ask for the side band.
- A request that asks for an option not announced is answered as if it had not asked (SPEC-0067 FR-NEW-012, list corrected).
### 3.2 Out of scope, and why
- Plain `multi_ack` answers (`ACK <id> continue`): no client can use them over HTTP, so supporting them is not worth the code (owner decision, recommendation accepted).
- The older, smaller side band option and the options git's own server announces beyond these (shallow, filter, thin packs, early end): not offered by this service, see SPEC-0067.
- The cap on held ids (BL-0117, SPEC-0070).
### 3.3 Constraints imposed by the context
- The option names and the answer lines are git's (project owner: git compatibility), as in SPEC-0067 Section 3.3. Real git clients must keep working unchanged.

## 5. Usage scenarios
### SC-001: A normal git client
**Actor:** Developer
**Preconditions:** project `demo` with commits `c1` to `c6`.
**Flow:**
1. The client reads the options the service announces, then asks for `multi_ack_detailed` and `side-band-64k` (and others it likes) → the service answers as before.
**Postconditions:** clone and fetch work; the announced options are the four listed.

### SC-002: A client that does not ask for the side band
**Actor:** A program that talks to the service in git's protocol without the side band
**Preconditions:** as above.
**Flow:**
1. The client asks for the objects without `side-band-64k` → the service sends the acknowledgement lines, then the objects as they are, with no framing and no closing bytes.
**Postconditions:** the client receives the same objects as from git's own server.

### SC-003: A client that names an option the service does not offer
**Actor:** A program or an old git client
**Preconditions:** as above.
**Flow:**
1. The client asks for the plain `multi_ack` or for other options → the service ignores what it does not offer and answers as for the detailed form.
**Postconditions:** no error; the answer is the usual one.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-U]: The options the service offers
> THE git service SHALL announce to a client exactly the options `multi_ack_detailed`, `side-band-64k` and `ofs-delta`, followed by its agent text `agent=mcp-fs/0.1.0`, for a project with commits and for a project without.
- **Business rules:** the options are announced once, with the first branch listed, or against the empty id when the project has no branch.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-E]: Objects without the side band
> WHEN a client does not ask for `side-band-64k`, THE git service SHALL send the objects right after the acknowledgement lines, with no framing, no progress text and no closing byte.
- **Business rules:** this holds for an empty set of objects too; the acknowledgement lines are unchanged.
- **Scenario:** SC-002
- **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-O]: Options that are not offered are ignored (modifies SPEC-0067/FR-NEW-012)
> IF the first wanted line of a request carries an option other than `multi_ack_detailed`, `side-band-64k`, `ofs-delta` or `agent`, THEN THE git service SHALL ignore that option and answer the request as for the same request without it.
- **Before:** the list of options not to ignore also held `multi_ack` (SPEC-0067 FR-NEW-012), which the service announced and did not honour.
- **After:** `multi_ack` is an option that is not offered, so asking for it alone gives the detailed answer, like any other unknown option.
- **Reason:** BL-0119: the announced list and the honoured list must be the same.

### 6.3 Removed
None.

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. Each test runs against a project served by the git service, through a real git client or a request written by hand with the lines a git client sends, and compares the answer with git's own server where it offers the same option (SPEC-0067 Section 3.3).

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, FR-MOD-001 | Critical |
| E2E-002 | New | requirement | SC-001 | FR-NEW-001 | High |
| E2E-003 | New | failure | SC-002 | FR-NEW-002 | High |
| E2E-004 | New | failure | SC-003 | FR-MOD-001 | High |
| E2E-005 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-006 | New | failure | SC-001 | FR-NEW-001 | Medium |
| E2E-007 | New | failure | SC-003 | FR-MOD-001 | Medium |
| E2E-008 | New | edge | SC-002 | FR-NEW-002 | Medium |

Coverage: happy 2, failure 4, edge 2, side effect 0 (the project is checked in each test), state transition 0 (the service keeps nothing between requests). Happy to failure ratio 2:4 (beats 1:1).

### 9.2 New tests
#### E2E-001: a real client still clones and fetches
- **Category:** journey; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a developer with a real git client
- **When** the developer runs `git clone` on `demo`, then the project gains `c7` and the developer runs `git fetch`
- **Then** the copy holds 7 commits, `origin/main` is at `c7` and the integrity check reports no problem
- **And** the client, which also offers options the service does not (such as `thin-pack` and `no-done`), gets no error; the project is unchanged
- **Priority:** Critical

#### E2E-002: the options the service offers
- **Category:** requirement; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`
- **When** a client reads the project's list of branches and tags before fetching
- **Then** the options announced with the first branch are exactly `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and the text `agent=mcp-fs/0.1.0`, and `multi_ack` is not among them
- **And** the branches and tags listed are unchanged
- **Priority:** High

#### E2E-003: a client that does not ask for the side band
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6`, stating `c4` as held, asking for `multi_ack_detailed` and `ofs-delta` but not `side-band-64k`, ending the negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c4> common`, `ACK <c4>`, then the 6 objects of `c5` and `c6` unframed, the same lines and the same objects as git's own server gives for that request
- **And** the answer carries no channel byte, no progress text and no closing byte after the objects; the project is unchanged
- **Priority:** High

#### E2E-004: a client that asks only for the plain multi_ack
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6`, stating `c4` as held, asking for `multi_ack` and `side-band-64k` (not `multi_ack_detailed`), ending the negotiation
- **When** the request is sent
- **Then** the answer equals the answer to the same request asking for `multi_ack_detailed`: `ACK <c4> common`, `ACK <c4>`, then the 6 objects of `c5` and `c6`
- **And** no line is of the form `ACK <id> continue`; no error is reported; the project is unchanged
- **Priority:** High

#### E2E-005: an up to date client without the side band
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6`, stating `c6` as held, asking for `multi_ack_detailed` but not `side-band-64k`, ending the negotiation
- **When** the request is sent
- **Then** the answer is `ACK <c6> common`, `ACK <c6>`, then an empty set of objects unframed, as git's own server gives
- **And** the project is unchanged
- **Priority:** Medium

#### E2E-006: the options of an empty project
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001
- **Given** a project with no commit
- **When** a client reads the project's list of branches and tags
- **Then** the options announced are exactly `multi_ack_detailed`, `side-band-64k`, `ofs-delta` and `agent=mcp-fs/0.1.0`
- **And** no branch is listed
- **Priority:** Medium

#### E2E-007: several options the service does not offer
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-MOD-001
- **Given** project `demo` with commits `c1` to `c6` in a line on `main`; a request wanting `c6` asking for `multi_ack`, `multi_ack_detailed`, `side-band-64k`, `thin-pack` and `include-tag`, stating `c4` as held, ending the negotiation
- **When** the request is sent
- **Then** the answer equals the answer to the same request asking for `multi_ack_detailed` and `side-band-64k` only
- **And** no error is reported; the project is unchanged
- **Priority:** Medium

#### E2E-008: a clone larger than one packet without the side band
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002
- **Given** a project holding one commit whose file is 400 KB of text, and a request wanting that commit, asking for `multi_ack_detailed` but not `side-band-64k`, with no held commit, ending the negotiation
- **When** the request is sent
- **Then** the answer is the line `NAK` followed by all 3 objects of the commit unframed, the same objects as git's own server gives
- **And** the objects arrive in one piece with no length prefix every 65 515 bytes; the project is unchanged
- **Priority:** Medium

### 9.4 Modified tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-MOD-001 | E2E-001, 002 | E2E-006 | none: the announced list has no boundary case beyond the empty project |
| SC-002 | FR-NEW-002 | none: a client without the side band is rare and its success is the edge case | E2E-003 | E2E-005, 008 |
| SC-003 | FR-MOD-001 | none: ignoring an option is the failure handling itself | E2E-004, 007 | none: every case names an option the service does not offer |

Tests per requirement (each at least 3): FR-MOD-001 (001, 004, 007); FR-NEW-001 (001, 002, 006); FR-NEW-002 (003, 005, 008).

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Option | a word a client asks for, or a service announces, to choose how an exchange runs | all |
| Side band | the framing that lets the service interleave objects with progress text | fetch |
| Unframed | the objects sent as they are, with no side band framing | fetch |
| Agent text | the service's name and version announced with the options | all |
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
| 1 | the announced options | `crates/core/src/git/http/mod.rs:67-68` (`UPLOAD_PACK_CAPABILITIES`), used at `:383`; asserted by tests at `:639` and `:684` |
| 2 | the options of the first wanted line are dropped | `crates/core/src/git/http/negotiate.rs:45` (`rest.trim().split(' ').next()`) |
| 3 | plain `multi_ack` gets the detailed answer | `crates/core/src/git/http/negotiate.rs:61-103` (`answer`, one form only); git's own server answers `ACK <id> continue` to it (checked 2026-11-01 with git 2.50.1) |
| 4 | objects always framed, closing byte and flush | `crates/core/src/git/http/negotiate.rs:105-125` (`finish`, closing byte at `:119`), `SIDE_BAND_CHUNK` at `mod.rs:75` |
| 5 | git's own server unframed without the side band | `git upload-pack --stateless-rpc` with a request that does not ask for `side-band-64k`: `ACK <id> common`, `ACK <id>`, then `PACK` bytes directly (checked 2026-11-01) |
| 6 | no test of the announced list or of the unframed form | `rg -n 'side-band' crates/core/tests/git_negotiation_e2e.rs` shows only `CAPS` that includes it (`:31`) |
Test command: `cargo test -p mcp-fs-core --test git_negotiation_e2e` and `cargo test -p mcp-fs-core --lib git::http`.

### 14.2 Class sweep
Not applicable (`Security: n/a`).

### 14.3 Design inputs
- The answer must take the chosen form from the first wanted line; `Request` (`negotiate.rs:18`) has no field for it today. Signal: E2E-003, E2E-005, E2E-008 fail until it does.
- The harness parser `parse_answer` of `crates/core/tests/git_negotiation_e2e.rs` expects the side band; a raw form parser is needed for E2E-003, 005, 008.
- Removing `multi_ack` from the announced text changes `UPLOAD_PACK_CAPABILITIES` and the unit tests that assert it (they assert the constant itself, so they follow (`mod.rs:639,684`).
- Clients that announce `no-progress`, `thin-pack` or `include-tag` keep being ignored (SPEC-0067 FR-NEW-012).
