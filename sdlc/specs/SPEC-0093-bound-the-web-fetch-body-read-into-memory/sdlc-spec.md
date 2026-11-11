# Bound the web fetch body read into memory: functional specification

> Id: SPEC-0093
> Nature: FEAT
> Depth: M
> Depth evidence: 3 bounded contexts (web lookup, library documentation lookup, project storage), 10 requirements, public behavior change on two optional tool families (an answer past a limit is refused instead of being read whole), no persisted change. Override with --depth.
> Status: specified
> Generated on: 2026-11-11
> Project: existing
> From backlog: BL-0148
> Depends on: none (SPEC-0089 built: its capped reading of a remote answer is the shape reused; the pull request provider's 16 MiB refusal sets the number and the message)
> Security: internal finding
> CVSS: not scored
> Affected: every deployment that enables the optional `web` or `context7` tool family (both off by default)
> Fixed in: unreleased
>
> Note: written non interactively. The user authorized every decision; the request "specify BL-0148" is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

When an operator turns the optional web tools or the library documentation tools on, an assistant can make the server read a remote answer: a search result page, a news or suggestion list, a fetched page, a downloaded file, a documentation page. Today the server reads that answer whole into memory before looking at its size, so a remote site answering gigabytes, or a body that never ends until the time limit, costs the server that memory. After this increment an answer handed back to the assistant is refused once it passes 16 MiB, with the message the pull request tools already use, and a download or a saved page is refused once it passes what the member is allowed to store anyway, with the storage refusal they already get today after the whole read. Answers within the limits behave exactly as today.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- Both families are off by default. An operator turns them on in the configuration or with a command line switch; while off, their tools are not offered and a call answers that the tool does not exist. They are reachable through the assistant tool only: the HTTP API lists them in its description but offers no route to call them. [14.1 row 1]
- With the web family on, a member can: search the web, search news, get query suggestions (all three ask a fixed search service); fetch a page, receiving its text with tags stripped and cut at 50 000 characters; fetch a page into a project file instead; download a URL into a project file, the answer naming the remote content type. [row 2]
- With the documentation family on, a member can resolve a library name to an identifier and read that library's documentation, both from the documentation service the operator configured. [row 3]
- Every one of these reads the remote answer whole into memory, with no size check, before using it. The only bound is the time limit of the call: 10 seconds for search, news and suggestions, at most 30 seconds for a fetch, at most 120 seconds for a download, 30 seconds by default for the documentation service. The time limit covers the reading of the answer, so a remote site that keeps sending can send as much as the link carries in that time. [row 4]
- A remote answer marked as compressed is not expanded: the server neither asks for compression nor decodes it, and keeps the bytes as received. [row 5]
- A download or a saved page goes through the ordinary project write: the member must be a member of the project, a file already there must have been read first in the session, and the project storage limit and the session write allowance (52 428 800 bytes by default) are checked, but only after the whole answer was read into memory. A refusal leaves no file and charges nothing; it answers `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of <quota> bytes exceeded`, or `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of <N> MB exceeded: <used> bytes used, <growth> bytes requested`. [row 6]
- Remote failures answer `ERR_INTERNAL_ERROR` with today's messages (`fetch request failed: ...`, `read response: ...`, `parse news response: ...`, `request timed out`, `context7 error: <status>`, `library '<id>' not found`, ...). A refused tool call is logged once, at the informational level for a refusal the caller caused and at the error level otherwise. [row 7]
- Elsewhere in the server, the pull request tools already read a provider's answer piece by piece and refuse it past 16 MiB with `ERR_INVALID_ARGUMENT` and `response from host '<host>' exceeds the 16 MiB limit`, and the document converter's web answer is read under the converter limit (SPEC-0089). [row 8]

### 2.2 Specs governing this area

- SPEC-0009 defined the two families (FR-002: a volume write goes through the engine, the write allowance applies; FR-004 to FR-007: search, fetch, save, download; FR-008 onward: documentation). It names "result caps and size limits" in scope but sets no size limit on a remote answer. This increment adds one; SPEC-0009 is not modified.
- SPEC-0027 made the optional families reachable through the assistant tool when enabled and absent when disabled. Unchanged.
- SPEC-0020 defined the project storage limit and its refusal; SPEC-0084 set the precedent of a decode bound equal to the lower of the project side and the session side with the refusal of the side met first. Reused here, not modified.
- SPEC-0089 bounded the converter's web answer and deferred this entry (BL-0148). [row 9]

### 2.3 What is verified today

- Tests check the families' registration and schemas, the helpers (tag stripping, result cap, token cap), that a disabled family is unknown, and that a page fetch through the assistant tool returns a small stand in page. [row 10]
- No test covers an answer of any size past a few bytes, a download, a saved page, a declared length, a compressed answer, or the memory held. [row 10]

## 3. Scope

### 3.1 In scope

- Every remote answer the two optional families read: search, news, suggestions, page fetch returned to the assistant, page fetch saved into a project, download, library resolution, library documentation.
- A limit on what is read, a refusal before reading when the answer declares a length past the limit, the bound on what the server holds, the order of refusals, and the absence of any trace after a refusal.

### 3.2 Out of scope, and why

- Which addresses a member is allowed to fetch (loopback, private networks, cloud metadata addresses): a different class (CWE-918), not the size of the answer; reported to the owner as a side finding, no backlog entry (project rule).
- Redirects: followed as today; the limit applies to the answer finally read.
- The time limits: kept as today; they already end a body that never ends.
- The pull request provider client: already bounded at 16 MiB (2.1).
- The document converter: bounded by SPEC-0089.
- The sign in flow with a code host and the search, embedding, reranking and recognition endpoints: their hosts are chosen by the operator, not by a member or a page; reported as side findings.
- Remote clone and fetch of a repository: the transfer goes to a temporary folder on disk, not into memory; reported as a side finding.
- A configurable limit: the existing provider limit and the existing storage rules are enough (14.4 D-5).

### 3.3 Constraints imposed by the context

- No new third party component and no unsafe code (the user's standing brief, as in SPEC-0088 and SPEC-0089).

## 4. Actors

- **Member:** a person with access to a project; asks the assistant to search, fetch, download or read documentation.
- **Operator:** runs the server, turns the families on, watches memory, reads the log.
- **Attacker:** someone who controls a site a member fetches or downloads, or a page a search returns, and answers huge or endlessly.
- **Remote site:** the server at the other end of a search, fetch, download or documentation call; not trusted to answer small.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Web lookup | asking the search service and remote sites, returning what they answer | answer, answer limit, declared length, remote site, stand in site |
| Library documentation lookup | asking the documentation service for a library identifier and its documentation | answer, answer limit |
| Project storage | files, session write allowance, project storage limit, audit trail | write limit, project side, session side, replaced file |

## 5. Usage scenarios

### SC-001: Read a remote answer within the answer limit
**Actor:** Member.
**Preconditions:** the web family on; a remote site answers a page of a few kilobytes.
**Flow:**
1. The member asks to fetch the page (or to search, get news, get suggestions, resolve a library, read its documentation) → the server reads the answer, which stays within 16 777 216 bytes.
2. The server answers exactly as today.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: the answer passes the answer limit → SC-002.
- EXC-001b: the site is unreachable, slow past the time limit, or answers a failure → today's refusal.

### SC-002: An answer past the answer limit is refused
**Actor:** Member, attacker.
**Preconditions:** the site answers 1 073 741 824 bytes, with or without declaring its length.
**Flow:**
1. The member asks to fetch it → when the length is declared past the limit, the server refuses before reading the body; otherwise it reads until the answer passes 16 777 216 bytes, then stops reading and drops the connection.
2. The server refuses with `ERR_INVALID_ARGUMENT` and `response from host '<host>' exceeds the 16 MiB limit`.
**Postconditions:** the server held at most 17 825 792 bytes of the answer; nothing is returned.

### SC-003: Download or save within the write limit
**Actor:** Member.
**Preconditions:** project `atlas`, member `mia@example.com`, nothing written in the session.
**Flow:**
1. The member asks to download `http://<site>/report.pdf` into `/downloads/report.pdf` (or to fetch a page into a project file) → the server computes the write limit, reads the answer within it.
2. The file is written as today; the answer reports its path, bytes written and, for a download, the content type.
**Postconditions:** as today: one file, one audit line, the allowance charged.
**Exceptions:**
- EXC-003a: the answer passes the write limit → SC-004.

### SC-004: A download or save past the write limit is refused
**Actor:** Member, attacker.
**Preconditions:** the site answers more than the member is allowed to store.
**Flow:**
1. The member asks to download it → the server refuses before reading when the declared length passes the write limit; otherwise it stops once the answer passes it.
2. The server refuses with the refusal of the side met: the session allowance or the project storage limit, with today's messages.
**Postconditions:** no file written or changed, nothing charged, no audit line; the server held at most the write limit plus 1 048 576 bytes.

### SC-005: Order when several refusals apply
**Actor:** Member.
**Flow:**
1. Argument checks and membership come first, as today, before any remote request.
2. Then, while asking the site and reading its answer: a transport failure, the time limit, a declared length past the limit, or the limit passed while reading, whichever comes first.
3. Then today's write checks, on the whole answer.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-O]: Answer limit
> IF a remote answer handed back to the assistant passes 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL stop reading it and refuse with `ERR_INVALID_ARGUMENT` and the message `response from host '<host>' exceeds the 16 MiB limit`.
- **Business rules:** applies to search, news, suggestions, a page fetch returned to the assistant, library resolution and library documentation. Exactly 16 777 216 bytes is accepted. `<host>` is the host name of the address the server asked, without port, lower case: `html.duckduckgo.com` for search, `duckduckgo.com` for news and suggestions, the host of the documentation service address for the documentation tools, the host of the address the member gave for a fetch. After a redirect, `<host>` stays the host first asked. The connection is dropped when reading stops.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Declared length past the answer limit
> IF a remote answer handed back to the assistant declares a length past 16 777 216 bytes THEN THE web lookup and the library documentation lookup SHALL refuse with the message of FR-NEW-001 without reading any byte of its body.
- **Business rules:** a declared length of exactly 16 777 216 is read. An answer without a declared length is judged by FR-NEW-001 only. An answer with a failure status that the tool reports without reading its body today (documentation, not found or other failure) keeps today's refusal whatever its declared length.
- **User visible text:** as in FR-NEW-001.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Write limit for a download or a saved page
> IF the answer of a download or of a page fetch saved into a project passes the write limit THEN THE web lookup SHALL stop reading it and refuse with the refusal of the side that sets the write limit.
- **Business rules:** the write limit is computed after membership and before the request: the lower of the project side and the session side. The session side is what the member's session can still write (the session write allowance minus what the session wrote). The project side exists only when the project has a storage limit: the limit minus the storage used, plus the size of the file the write replaces at the destination. A tie goes to the project side. Exactly the write limit is accepted. Session refusal: `ERR_WRITE_QUOTA_EXCEEDED`, `session write quota of <quota> bytes exceeded`, `<quota>` the configured allowance in plain digits. Project refusal: `ERR_PROJECT_QUOTA_EXCEEDED`, `project storage quota of <N> MB exceeded: <used> bytes used, <requested> bytes requested`, `<N>` the limit divided by 1 048 576 rounded down, `<used>` the storage used, `<requested>` the limit minus the storage used plus 1. The messages are today's, verbatim.
- **User visible text:** as above.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Declared length past the write limit
> IF the answer of a download or of a saved page declares a length past the write limit THEN THE web lookup SHALL refuse without reading any byte of its body, with the refusal of the side that sets the write limit.
- **Business rules:** for the project refusal, `<requested>` is the declared length minus the size of the file the write replaces. The session refusal message is unchanged. A declared length equal to the write limit is read.
- **User visible text:** as in FR-NEW-003.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-005 [EARS-U]: What the server holds of one answer
> THE web lookup and the library documentation lookup SHALL hold, during one call, at most the call's limit plus 1 048 576 bytes of the remote answer.
- **Business rules:** what is held counts the bytes of the answer as received and kept, together with the piece just received; text, results or a file built from an answer within its limit are not counted (they exist today and are bounded by the limit they come from). The call's limit is the answer limit (FR-NEW-001), so 17 825 792 bytes, or the write limit (FR-NEW-003). 1 048 576 is the transport margin: the piece the connection delivers at once, measured up to 507 904 bytes (14.1 row 11). A call refused before its body is read holds 0. The bound is per call; two calls at once each have their own.
- **Scenario:** SC-002, SC-004
- **Priority:** Must

#### FR-NEW-006 [EARS-U]: The limits count what the server keeps
> THE web lookup and the library documentation lookup SHALL count against every limit of this specification each byte of the answer as the server keeps it, an answer marked compressed being counted and kept as received.
- **Business rules:** the server does not ask for compression and does not expand an answer (unchanged). If a later change makes the server expand answers, the limits apply to the expanded bytes: this requirement stays true.
- **Scenario:** SC-001, SC-004
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Order of refusals
> IF several refusals apply to one call THEN THE web lookup SHALL report the first met in this order: (1) today's argument checks of the tool (a download address not starting with `http://` or `https://`: `url must start with http:// or https://`; a page fetch given only one of the project and the destination: `mount_id and save_path must both be provided, or neither`); (2) for a download or a saved page, membership (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`) and the destination path checks, as today; (3) while asking and reading, whichever comes first: a transport failure, the time limit, a declared length past the limit (FR-NEW-002, FR-NEW-004), the limit passed while reading (FR-NEW-001, FR-NEW-003); (4) for a download or a saved page, today's write checks on the whole answer: a file at the destination not read first in the session (`ERR_EDIT_WITHOUT_PRIOR_READ`, `edit '<path>' requires reading it first in this session`), then the project storage limit, then the session allowance.
- **Business rules:** items (1) and (2) make no remote request. A page fetch returned to the assistant checks its address prefix as today. Messages of items (1), (2) and (4) are unchanged.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-008 [EARS-UB]: A refusal leaves no trace
> THE web lookup SHALL NOT write or change a file, charge the session write allowance or the project storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-004.
- **Business rules:** a file at the destination keeps its bytes. The refusal is logged once, as every refused tool call today.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-009 [EARS-U]: Everything else unchanged
> THE web lookup and the library documentation lookup SHALL answer a call whose answer stays within its limit with the same result, file, charge, audit line, time limit and failure messages as before.
- **Business rules:** includes the fetch text cut at 50 000 characters, the search and news result cap of 50, the content type reported by a download (`application/octet-stream` when the site names none), the documentation not found and failure status messages, and the unreachable, timeout and unreadable answer messages.
- **Scenario:** SC-001, SC-003
- **Priority:** Must

#### FR-NEW-010 [EARS-S]: Off by default, unchanged
> WHILE a family is off THE product SHALL NOT offer its tools nor make any remote request on their behalf.
- **Business rules:** both families stay off by default; a call to one of their tools answers that the tool does not exist, as today.
- **Scenario:** SC-001
- **Priority:** Must

### 6.2 Modified

None. No spec sets a size limit on these answers (2.2).

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN an answer passes its limit THE web lookup SHALL refuse without waiting for the remote site to finish or for the time limit.

### 7.2 Security and access
- THE product SHALL keep membership as the gate of a download or a saved page, checked before any remote request.
- THE web lookup and the library documentation lookup SHALL NOT let a remote answer make the server hold more than FR-NEW-005 states.

### 7.5 Reliability
- IF a call is refused under this specification THEN THE product SHALL leave no connection to the remote site open.

### 7.6 Operability
- THE product SHALL make the answer held of one call measurable by the verification of this specification (the measurement hook of Section 9).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | empty page; nothing written yet in the session | returned as today; full allowance is the session side | E2E-001, E2E-008 |
| Boundary values | answer at 16 777 216 and one over; write limit exactly and one over, each side | accepted; refused | E2E-006, E2E-007, E2E-013, E2E-008, E2E-009, E2E-017, E2E-019 |
| Concurrent actions | two floods by two members at once, a normal fetch alongside | both refused within their bound, the normal fetch succeeds | E2E-034 |
| Slow or lost connectivity | site unreachable; answer trickling until the time limit | today's messages | E2E-029, E2E-030 |
| Access states | not a member; family off | membership refusal, no request; tool unknown | E2E-024, E2E-012, E2E-038, E2E-039 |
| Data variations | answer marked compressed; declared length vs none | counted as received; refused before body vs while reading | E2E-011, E2E-037, E2E-014, E2E-013 |
| Navigation | not applicable: one call, no flow across screens | | |
| Partial operations | refused call repeated; file at destination unread | same refusal, nothing left; limit before the read check | E2E-031, E2E-027 |
| Dependency failures | documentation service failing with a huge body | today's message, body not read | E2E-032 |
| Ordering and timing | tie between sides; argument and membership checks first; limit before write checks | FR-NEW-003, FR-NEW-007 | E2E-022, E2E-024 to E2E-028 |
| Exhaustion | every tool flooded; session partly spent; project nearly full; overwrite of a file | refused within the bound | E2E-SEC-001 to E2E-SEC-010, E2E-019 to E2E-023 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Unless a test says otherwise: project `atlas`, member `mia@example.com`, both families on, the call goes through the assistant tool, the session write allowance is 52 428 800 bytes with nothing written yet, the project has no storage limit. A "stand in site" is a web site built by the tests on the local machine at host `127.0.0.1`, answering what the test describes; it records how many body bytes it sent and whether the connection was closed before it finished. The search service, the documentation service and every fetched address are pointed at a stand in site for the test, so `<host>` reads `127.0.0.1`. "Chunked" means the answer declares no length. "Answer held" is the largest amount of the remote answer the server holds at once during the call, read by the operator's measurement hook. "Nothing left" means: no file at the destination appears in the listing of its folder (or an existing one keeps its bytes), the audit trail has no line for it, and a following write of 52 428 800 bytes to `/docs/fill.bin` by the same member succeeds (nothing was charged).

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-009 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-009 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-009 | High |
| E2E-004 | New | happy | SC-003 | FR-NEW-009 | High |
| E2E-005 | New | happy | SC-003 | FR-NEW-009 | High |
| E2E-006 | New | edge | SC-001 | FR-NEW-001 | Critical |
| E2E-007 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-008 | New | edge | SC-003 | FR-NEW-003 | Critical |
| E2E-009 | New | edge | SC-003 | FR-NEW-003 | High |
| E2E-010 | New | edge | SC-003 | FR-NEW-003 | High |
| E2E-011 | New | edge | SC-001 | FR-NEW-006 | Medium |
| E2E-012 | New | edge | SC-001 | FR-NEW-010 | Medium |
| E2E-013 | New | failure | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-014 | New | failure | SC-002 | FR-NEW-002, FR-NEW-005 | Critical |
| E2E-015 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-016 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-017 | New | failure | SC-004 | FR-NEW-003, FR-NEW-005, FR-NEW-008 | Critical |
| E2E-018 | New | failure | SC-004 | FR-NEW-004, FR-NEW-005 | Critical |
| E2E-019 | New | failure | SC-004 | FR-NEW-003 | High |
| E2E-020 | New | failure | SC-004 | FR-NEW-004 | High |
| E2E-021 | New | failure | SC-004 | FR-NEW-003, FR-NEW-004 | High |
| E2E-022 | New | failure | SC-005 | FR-NEW-003, FR-NEW-007 | Medium |
| E2E-023 | New | failure | SC-004 | FR-NEW-003 | High |
| E2E-024 | New | failure | SC-005 | FR-NEW-007 | Medium |
| E2E-025 | New | failure | SC-005 | FR-NEW-007 | Medium |
| E2E-026 | New | failure | SC-005 | FR-NEW-007 | Medium |
| E2E-027 | New | failure | SC-005 | FR-NEW-007, FR-NEW-008 | High |
| E2E-028 | New | failure | SC-005 | FR-NEW-007, FR-NEW-009 | Medium |
| E2E-029 | New | failure | SC-001 | FR-NEW-009 | Medium |
| E2E-030 | New | failure | SC-001 | FR-NEW-009 | Medium |
| E2E-031 | New | failure | SC-004 | FR-NEW-008 | Medium |
| E2E-032 | New | failure | SC-002 | FR-NEW-002, FR-NEW-009 | Medium |
| E2E-033 | New | side effect | SC-004 | FR-NEW-008 | High |
| E2E-034 | New | side effect | SC-002 | FR-NEW-001, FR-NEW-005 | High |
| E2E-035 | New | failure | SC-002 | FR-NEW-002 | High |
| E2E-036 | New | failure | SC-002 | FR-NEW-002 | High |
| E2E-037 | New | edge | SC-003 | FR-NEW-006 | Medium |
| E2E-038 | New | edge | SC-001 | FR-NEW-010 | Medium |
| E2E-039 | New | edge | SC-003 | FR-NEW-010 | Medium |
| E2E-SEC-001 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-SEC-002 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-SEC-003 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-SEC-004 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-SEC-005 | New | failure, security | SC-004 | FR-NEW-003, FR-NEW-005, FR-NEW-008 | Critical |
| E2E-SEC-006 | New | failure, security | SC-004 | FR-NEW-003, FR-NEW-005, FR-NEW-008 | Critical |
| E2E-SEC-007 | New | failure, security | SC-004 | FR-NEW-004, FR-NEW-005 | Critical |
| E2E-SEC-008 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-SEC-009 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-SEC-010 | New | failure, security | SC-004 | FR-NEW-003, FR-NEW-006 | Critical |

Coverage (49 tests: 49 new, 0 modified, 0 removed): happy 5, failure 32 (10 of them security), side effect 2, edge 10, state transition 0 (no state machine: a call is answered or refused, nothing persists between calls). Happy to failure ratio 1:6.4.

### 9.2 New tests

#### E2E-001: Page fetch within the limit
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** a stand in site answering `/page` with `<p>Hello &amp; welcome</p>`, marked as HTML
- **When** the member asks to fetch `http://127.0.0.1:<port>/page`
- **Then** the answer is the text `Hello & welcome`
- **And** the answer held never exceeds 1 048 576

#### E2E-002: Search within the limit
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** the search service is a stand in site answering a result page with two results, `Rust` at `https://www.rust-lang.org/` with snippet `A language`, and `Tokio` at `https://tokio.rs/` with snippet `A runtime`
- **When** the member searches `rust` with at most 10 results
- **Then** the answer lists exactly those two results, title, address and snippet, in that order

#### E2E-003: Library documentation within the limit
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** the documentation service is a stand in site answering `# Tokio docs` followed by 10 000 letters `t` for `/tokio-rs/tokio`
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the answer is exactly that text, 10 012 bytes

#### E2E-004: Download within the allowance
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** a stand in site answering `/report.pdf` with 1 000 000 bytes, declared, marked `application/pdf`
- **When** the member downloads it into `/downloads/report.pdf` of `atlas`
- **Then** the answer names `/downloads/report.pdf`, 1 000 000 bytes written, not overwritten, content type `application/pdf`
- **And** the file holds the 1 000 000 bytes sent; the audit trail has one write line for it; a following write of 51 428 801 bytes by the member is refused with `session write quota of 52428800 bytes exceeded` (the download was charged)

#### E2E-005: Page saved into a project within the allowance
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** a stand in site answering `/page` with `<p>Saved</p>`, chunked
- **When** the member fetches it into `/pages/saved.html` of `atlas`
- **Then** the answer names `/pages/saved.html` with 12 bytes written
- **And** the file holds `<p>Saved</p>`; the audit trail has one write line for it

#### E2E-006: Fetch answer of exactly the answer limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Critical
- **Given** a stand in site answering 16 777 216 letters `b`, chunked, not marked as HTML
- **When** the member fetches it
- **Then** the answer is 50 000 letters `b`
- **And** the answer held never exceeds 17 825 792

#### E2E-007: News answer of exactly the answer limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the news service answers a list with one item titled `Launch`, padded with spaces to exactly 16 777 216 bytes
- **When** the member asks for news about `launch`
- **Then** the answer lists one item titled `Launch`

#### E2E-008: Download of exactly the session side
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Critical
- **Given** a stand in site answering 52 428 800 bytes, chunked
- **When** the member downloads it into `/downloads/big.bin`
- **Then** the answer reports 52 428 800 bytes written
- **And** the answer held never exceeds 53 477 376

#### E2E-009: Download of exactly the project side
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `atlas` with a storage limit of 2 097 152 bytes and 1 000 000 bytes used; a stand in site answering 1 097 152 bytes, chunked
- **When** the member downloads it into `/downloads/fit.bin`
- **Then** the answer reports 1 097 152 bytes written; the project uses 2 097 152 bytes

#### E2E-010: Overwrite counts the replaced file
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `atlas` with a storage limit of 2 097 152 bytes, 1 000 000 bytes used of which `/downloads/old.bin` holds 500 000 and was read by the member; a stand in site answering 1 597 152 bytes, chunked
- **When** the member downloads it into `/downloads/old.bin`
- **Then** the answer reports 1 597 152 bytes written, overwritten

#### E2E-011: Compressed answer counted as received
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a stand in site answering 100 000 bytes marked as compressed, which expands to 1 073 741 824 bytes once decompressed
- **When** the member fetches it
- **Then** the fetch succeeds, the answer is built from the 100 000 bytes as received (at most 50 000 characters)
- **And** the answer held never exceeds 1 148 576

#### E2E-012: Web family off
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the web family off and a stand in site
- **When** the member calls the page fetch tool on it
- **Then** the call answers that the tool does not exist
- **And** the stand in site received no request

#### E2E-013: Fetch answer one byte over, chunked
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-005; **Priority:** Critical
- **Given** a stand in site answering 16 777 217 letters `b`, chunked
- **When** the member fetches it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`
- **And** the answer held never exceeds 17 825 792

#### E2E-014: Fetch answer declaring one byte over
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** Critical
- **Given** a stand in site that declares a length of 16 777 217, sends its headers, then waits 10 seconds before sending the body
- **When** the member fetches it
- **Then** the refusal of E2E-013 arrives before the stand in site sends its first body byte
- **And** the stand in site records the connection closed with 0 body bytes sent; the answer held is 0

#### E2E-015: Suggestions answer over the limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the suggestion service answers a list padded with spaces to 16 777 217 bytes, chunked
- **When** the member asks for suggestions for `ru`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`

#### E2E-016: Library resolution answer over the limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** the documentation service answers the resolution of `tokio` with 16 777 217 bytes, chunked
- **When** the member resolves `tokio`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`

#### E2E-017: Download one byte over the session side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003, FR-NEW-005, FR-NEW-008; **Priority:** Critical
- **Given** a stand in site answering 52 428 801 bytes, chunked
- **When** the member downloads it into `/downloads/big.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 53 477 376; nothing left

#### E2E-018: Download declaring one byte over the session side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004, FR-NEW-005; **Priority:** Critical
- **Given** a stand in site declaring 52 428 801 bytes, waiting 10 seconds after its headers
- **When** the member downloads it into `/downloads/big.bin`
- **Then** the refusal of E2E-017 arrives before the first body byte is sent
- **And** the stand in site records 0 body bytes sent; nothing left

#### E2E-019: Download over the project side, chunked
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `atlas` with a storage limit of 2 097 152 bytes and 1 000 000 bytes used; a stand in site answering 5 000 000 bytes, chunked
- **When** the member downloads it into `/downloads/over.bin`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 2 MB exceeded: 1000000 bytes used, 1097153 bytes requested`
- **And** the answer held never exceeds 2 145 728; nothing left

#### E2E-020: Download declaring a length over the project side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** the project of E2E-019 and a stand in site declaring 5 000 000 bytes, waiting 10 seconds after its headers
- **When** the member downloads it into `/downloads/over.bin`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 2 MB exceeded: 1000000 bytes used, 5000000 bytes requested`, before the first body byte is sent

#### E2E-021: Overwrite over the project side
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003, FR-NEW-004; **Priority:** High
- **Given** the project and file of E2E-010; first a stand in site answering 1 597 153 bytes, chunked; then one declaring 1 597 153 bytes
- **When** the member downloads each into `/downloads/old.bin`
- **Then** both refusals are `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 2 MB exceeded: 1000000 bytes used, 1097153 bytes requested`
- **And** `/downloads/old.bin` still holds its 500 000 bytes

#### E2E-022: Tie between the two sides
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-003, FR-NEW-007; **Priority:** Medium
- **Given** `atlas` with a storage limit of 62 914 560 bytes and 10 485 760 bytes used (project side 52 428 800, equal to the session side); a stand in site answering 52 428 801 bytes, chunked
- **When** the member downloads it into `/downloads/tie.bin`
- **Then** the refusal is `ERR_PROJECT_QUOTA_EXCEEDED` with `project storage quota of 60 MB exceeded: 10485760 bytes used, 52428801 bytes requested`

#### E2E-023: Session partly spent
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** the member already wrote 50 000 000 bytes in the session; a stand in site answering 3 000 000 bytes, chunked
- **When** the member downloads it into `/downloads/late.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 3 477 376

#### E2E-024: Not a member
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `eve@example.com`, not a member of `atlas`, and the stand in site of E2E-SEC-006
- **When** `eve@example.com` downloads it into `/downloads/x.bin` of `atlas`
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- **And** the stand in site received no request

#### E2E-025: Download address that is not a web address
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` downloads `ftp://127.0.0.1/x` into `/downloads/x.bin` of `atlas`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `url must start with http:// or https://` (the address check comes before membership)

#### E2E-026: Half given destination
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** the stand in site of E2E-SEC-004
- **When** the member fetches it naming project `atlas` and no destination
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `mount_id and save_path must both be provided, or neither`
- **And** the stand in site received no request

#### E2E-027: Unread file at the destination, answer over the limit
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007, FR-NEW-008; **Priority:** High
- **Given** `/downloads/old.bin` holding `old`, never read by the member; a stand in site answering 60 000 000 bytes, chunked
- **When** the member downloads it into `/downloads/old.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** `/downloads/old.bin` still reads `old`

#### E2E-028: Unread file at the destination, answer within the limit
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007, FR-NEW-009; **Priority:** Medium
- **Given** `/downloads/old.bin` holding `old`, never read; a stand in site answering `new`
- **When** the member downloads it into `/downloads/old.bin`
- **Then** the refusal is `ERR_EDIT_WITHOUT_PRIOR_READ` with `edit '/downloads/old.bin' requires reading it first in this session`
- **And** `/downloads/old.bin` still reads `old`

#### E2E-029: Site unreachable
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** no site listens on `127.0.0.1:<closed port>`
- **When** the member downloads `http://127.0.0.1:<closed port>/x` into `/downloads/x.bin`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `fetch request failed: `; nothing left

#### E2E-030: Answer trickling until the time limit
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** a stand in site sending its headers then one letter per second for 60 seconds, chunked
- **When** the member fetches it with a time limit of 2 seconds
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `read response: `, within 5 seconds of the call

#### E2E-031: Refused download repeated
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** the stand in site of E2E-017
- **When** the member downloads it twice into `/downloads/big.bin`
- **Then** both refusals are the refusal of E2E-017
- **And** nothing left after the second

#### E2E-032: Documentation failure status with a huge body
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-009; **Priority:** Medium
- **Given** the documentation service answers a server failure status, declaring a length of 1 073 741 824 bytes
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `context7 error: 500 Internal Server Error`
- **And** the answer held is 0

#### E2E-033: What a refused download leaves
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/downloads/keep.bin` holding `keep`, read by the member, and the stand in site of E2E-017
- **When** the member downloads it into `/downloads/keep.bin`
- **Then** the refusal is the refusal of E2E-017
- **And** `/downloads/keep.bin` still reads `keep`; nothing left; the server log holds exactly one line for the refused call, naming `ERR_WRITE_QUOTA_EXCEEDED`

#### E2E-034: Two floods and a normal fetch at once
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-005; **Priority:** High
- **Given** members `mia@example.com` and `noa@example.com`, the stand in site of E2E-SEC-004, and the page of E2E-001
- **When** both members fetch the flood and a third call fetches the page, at the same time
- **Then** both floods are refused with the message of E2E-013; the page fetch answers `Hello & welcome`
- **And** each flood call's answer held never exceeds 17 825 792; the stand in site records both connections closed early

#### E2E-035: Search answer declaring a length over the limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** the search service declares 20 000 000 bytes and waits 10 seconds after its headers
- **When** the member searches `rust`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`, before the first body byte is sent

#### E2E-036: Documentation answer declaring one byte over
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** the documentation service answers success declaring 16 777 217 bytes, waiting 10 seconds after its headers
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`, before the first body byte is sent

#### E2E-037: Compressed download saved as received
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** a stand in site answering 1 000 000 bytes marked as compressed, declared
- **When** the member downloads it into `/downloads/packed.gz`
- **Then** the answer reports 1 000 000 bytes written; the file holds the bytes exactly as sent

#### E2E-038: Documentation family off
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the documentation family off and a stand in documentation service
- **When** the member asks for the documentation of `/tokio-rs/tokio`
- **Then** the call answers that the tool does not exist; the stand in received no request

#### E2E-039: Download while the web family is off
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-010; **Priority:** Medium
- **Given** the web family off and the stand in site of E2E-004
- **When** the member calls the download tool on it
- **Then** the call answers that the tool does not exist; the stand in site received no request; no file appears under `/downloads`

### 9.3 Exploit tests

Each covers one occurrence of the class sweep (Section 14.2) at its own location. "A flood" is the literal answer `HTTP/1.1 200 OK`, `Transfer-Encoding: chunked`, then the letter `a` repeated 1 073 741 824 times, as fast as the link allows.

#### E2E-SEC-001: Search answer flood
- **Covers:** occurrence 1
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole 1 073 741 824 bytes are read as text before the results are parsed.
- **Given** the search service answers a flood
- **When** the member searches `rust`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `response from host '127.0.0.1' exceeds the 16 MiB limit`
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed before 1 073 741 824 bytes were sent

#### E2E-SEC-002: News answer flood
- **Covers:** occurrence 2
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read before it is parsed as a list.
- **Given** the news service answers `{"results":[` followed by the letter `a` repeated 1 073 741 824 times, chunked
- **When** the member asks for news about `launch`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early

#### E2E-SEC-003: Suggestions answer flood
- **Covers:** occurrence 4
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read before it is parsed.
- **Given** the suggestion service answers a flood
- **When** the member asks for suggestions for `ru`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early

#### E2E-SEC-004: Page fetch flood (originating occurrence)
- **Covers:** occurrence 3, page returned to the assistant
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read into memory, then copied as text, before the 50 000 character cut.
- **Given** a stand in site answering a flood at `/flood`
- **When** the member fetches `http://127.0.0.1:<port>/flood`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early

#### E2E-SEC-005: Saved page flood
- **Covers:** occurrence 3, page saved into a project
- **Red before the fix because:** `And the answer held never exceeds 53 477 376` fails: today the whole body is read before the write allowance is checked.
- **Given** a stand in site answering a flood at `/flood`
- **When** the member fetches it into `/pages/flood.html` of `atlas`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 53 477 376; the stand in site records the connection closed early; nothing left

#### E2E-SEC-006: Download flood (originating occurrence)
- **Covers:** occurrence 3, download
- **Red before the fix because:** `And the answer held never exceeds 53 477 376` fails: today the whole body is read before the write allowance is checked.
- **Given** a stand in site answering a flood at `/flood`, then continuing endlessly
- **When** the member downloads it into `/downloads/flood.bin` with a time limit of 120 seconds
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`, within 30 seconds of the call
- **And** the answer held never exceeds 53 477 376; the stand in site records the connection closed early; nothing left

#### E2E-SEC-007: Download declaring a huge length
- **Covers:** occurrence 3, declared length
- **Red before the fix because:** `Then the refusal ... before the first body byte is sent` fails: today the server waits for the body and reads it.
- **Given** a stand in site answering `HTTP/1.1 200 OK`, `Content-Length: 1073741824`, then waiting 10 seconds before a flood of the same length
- **When** the member downloads it into `/downloads/flood.bin`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`, before the first body byte is sent
- **And** the answer held is 0; the stand in site records 0 body bytes sent; nothing left

#### E2E-SEC-008: Library resolution flood
- **Covers:** occurrence 5
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read before it is parsed.
- **Given** the documentation service answers the resolution with a flood
- **When** the member resolves `tokio`
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early

#### E2E-SEC-009: Library documentation flood
- **Covers:** occurrence 6
- **Red before the fix because:** `And the answer held never exceeds 17 825 792` fails: today the whole body is read as text and returned.
- **Given** the documentation service answers the documentation of `/tokio-rs/tokio` with a flood
- **When** the member asks for it
- **Then** the refusal is the message of E2E-SEC-001
- **And** the answer held never exceeds 17 825 792; the stand in site records the connection closed early

#### E2E-SEC-010: Compressed marked download flood
- **Covers:** occurrence 3, compressed answer
- **Red before the fix because:** `And the answer held never exceeds 53 477 376` fails: today the 1 073 741 824 bytes are read whole.
- **Given** a stand in site answering a flood marked `Content-Encoding: gzip`
- **When** the member downloads it into `/downloads/flood.gz`
- **Then** the refusal is `ERR_WRITE_QUOTA_EXCEEDED` with `session write quota of 52428800 bytes exceeded`
- **And** the answer held never exceeds 53 477 376; nothing left

### 9.4 Modified tests

None. The existing tests use answers of a few bytes or check registration and helpers (14.1 row 10); they stay unchanged and green.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-006, FR-NEW-009, FR-NEW-010 | E2E-001, E2E-002, E2E-003 | E2E-029, E2E-030 | E2E-006, E2E-007, E2E-011, E2E-012, E2E-038 | E2E-001 (held bound) |
| SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-005 | E2E-001 (success branch, SC-001) | E2E-013 to E2E-016, E2E-032, E2E-035, E2E-036, E2E-SEC-001 to E2E-SEC-004, E2E-SEC-008, E2E-SEC-009 | E2E-006 (at the limit) | E2E-034 |
| SC-003 | FR-NEW-003, FR-NEW-006, FR-NEW-009, FR-NEW-010 | E2E-004, E2E-005 | E2E-028 | E2E-008, E2E-009, E2E-010, E2E-037, E2E-039 | E2E-004 (file, audit line, charge) |
| SC-004 | FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-008 | E2E-004 (success branch, SC-003) | E2E-017 to E2E-021, E2E-023, E2E-031, E2E-SEC-005 to E2E-SEC-007, E2E-SEC-010 | E2E-008 (at the limit) | E2E-033 |
| SC-005 | FR-NEW-003, FR-NEW-007 | E2E-004 (no refusal applies) | E2E-022, E2E-024 to E2E-028 | E2E-022 (tie) | E2E-024 (no request) |

SC-002, SC-004 and SC-005 are refusal scenarios; their success branch is named.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-006, E2E-007, E2E-013, E2E-015, E2E-016, E2E-034, E2E-SEC-001 to E2E-SEC-004, E2E-SEC-008, E2E-SEC-009 | 12 |
| FR-NEW-002 | E2E-014, E2E-032, E2E-035, E2E-036 | 4 |
| FR-NEW-003 | E2E-008, E2E-009, E2E-010, E2E-017, E2E-019, E2E-021, E2E-022, E2E-023, E2E-SEC-005, E2E-SEC-006, E2E-SEC-010 | 11 |
| FR-NEW-004 | E2E-018, E2E-020, E2E-021, E2E-SEC-007 | 4 |
| FR-NEW-005 | E2E-013, E2E-014, E2E-017, E2E-018, E2E-034, E2E-SEC-001 to E2E-SEC-009 | 14 |
| FR-NEW-006 | E2E-011, E2E-037, E2E-SEC-010 | 3 |
| FR-NEW-007 | E2E-022, E2E-024, E2E-025, E2E-026, E2E-027, E2E-028 | 6 |
| FR-NEW-008 | E2E-017, E2E-027, E2E-031, E2E-033, E2E-SEC-005, E2E-SEC-006 | 6 |
| FR-NEW-009 | E2E-001 to E2E-005, E2E-028, E2E-029, E2E-030, E2E-032 | 9 |
| FR-NEW-010 | E2E-012, E2E-038, E2E-039 | 3 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Web family | the optional tools that search the web, get news and suggestions, fetch a page and download a file; off by default | web lookup |
| Documentation family | the optional tools that resolve a library and read its documentation; off by default | library documentation lookup |
| Search service | the fixed web search service the search, news and suggestion tools ask | web lookup |
| Documentation service | the service at the address the operator configured for the documentation family | library documentation lookup |
| Remote site | the server at the other end of a call | web lookup, library documentation lookup |
| Answer | the body a remote site sends back | web lookup, library documentation lookup |
| Answer limit | 16 777 216 bytes (16 MiB), the most an answer handed back to the assistant is allowed to reach | web lookup, library documentation lookup |
| Declared length | the size an answer announces before its body | web lookup |
| Chunked | an answer that announces no size and sends its body in pieces | web lookup |
| Compressed answer | an answer the remote site marks as compressed; kept as received | web lookup |
| Transport margin | 1 048 576 bytes, the most the server is allowed to hold past a limit, for the piece the connection delivers at once | web lookup |
| Download | the tool writing a remote answer into a project file | web lookup, project storage |
| Saved page | a page fetch given a project and a destination, writing the answer into that file | web lookup, project storage |
| Write limit | the lower of the project side and the session side, computed before the request | project storage |
| Session side | what the member's session can still write | project storage |
| Project side | the project's storage limit minus the storage used plus the size of the file replaced; absent when the project has no limit | project storage |
| Replaced file | the file already at the destination, which the write overwrites | project storage |
| Session write allowance | the bytes a member can write in one session; 52 428 800 by default | project storage |
| Answer held | the largest amount of a remote answer, as received, the server holds at once during one call, the piece just received included; text built from it not counted | web lookup |
| Measurement hook | the operator facing reading of the answer held of one call | web lookup |
| Stand in site | a web site built by the tests on the local machine, answering as the test describes and recording what it sent | web lookup |
| Flood | the attacker answer of Section 9.3 | web lookup |
| Nothing left | the post condition defined in the header of Section 9 | project storage |
| Audit line | the one line an operator reads to see who wrote what | project storage |
| Refusal | an error answer; nothing is returned or written | web lookup, project storage |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server, turns the families on and reads its log | web lookup |
| Attacker | someone controlling a remote site who answers huge or endlessly | web lookup |
| Assistant tool | the tool an AI assistant calls; the only door of these families | web lookup |
| Time limit | the duration after which a call to a remote site is abandoned, as today | web lookup |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 1 | 3 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied after round 1:** F-1 (G1, pure precision, applied directly) whether text or results built from an answer count as "answer held" was unstated, so two measurement hooks can disagree: FR-NEW-005 and the glossary now count the answer as received plus the piece just received, and nothing built from it; A-1 seven citations pointed a few lines off (storage remaining, session remaining, the cap rule and its refusal, the download registration, both families' test modules), corrected in 14.1, 14.2 and 14.3 from the files; A-2 sweep search 2 hit three outbound clients the table did not list (object store reads, the purge command's and the server's search clients), rows 14 and 15 added; A-3 modal words in the summary, Section 3.2, SC-004, FR-NEW-006 and E2E-011 rewritten (Phase 10 item 4).
**Round 2:** re run on the amended file: ids (49 summarized, 49 specified), per test agreement of category, scenario and requirements between 9.1 and 9.2, requirement coverage from 9.1 equal to the Section 10 table (every FR at least 3), modal and implementation term scan of Sections 1 to 13, every citation of Section 14 opened; no finding.
**How the gates ran:** the brief forbids subagents, so the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks for ids (49 summarized, 49 specified), categories, per test agreement between 9.1 and 9.2, requirement coverage (every FR at least 3), forbidden modals and implementation terms, and the sweep searches re run. Recorded as a concern in the handoff.
**No implementation detail outside Section 14:** PASS. Error codes, verbatim messages, the option names quoted inside today's messages (`mount_id`, `save_path`), host names the member sees in a message, and the literal attacker payloads of 9.3 (header lines as the attacker writes them) are WHAT, as in SPEC-0085, SPEC-0088 and SPEC-0089.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | off by default, switches, one door | `crates/core/src/config.rs:630-637` (`WebConfig` `enabled: false`), `:656-662` (`Context7Config`); `crates/core/src/cli.rs:60-64`, `:305-310` (`--web`, `--context7`); `crates/core/src/mcp/server.rs:69-77` (`add_optional_families`, registered only when enabled); disabled call unknown `crates/core/src/tools/tool_surface_e2e.rs:487-497` (`bt_012`); listed only in `crates/core/src/api/openapi.rs:145-150`; no call route (`rg -n web crates/core/src/api/dataplane.rs` finds no route); no template enables them (`config/local.yaml.template`, `config/minio.yaml.template`, `run.sh`: no `web:` key, no `--web`) |
| 2 | web tools | `crates/core/src/tools/web.rs:25-180` (`register`); fixed hosts `:277` (`html.duckduckgo.com`), `:346`, `:348` (news), `:421` (suggestions); fetch text and cut `:405-417` (`take(50_000)` at `:415`); save mode `:96-116`; download `:123-164`; content type `:394-399` |
| 3 | documentation tools | `crates/core/src/tools/context7.rs:12-63`; address from configuration `:99`, `:134`; default `crates/core/src/config.rs:641-643` |
| 4 | read whole, time limit only | `crates/core/src/tools/web.rs:288` (`resp.text()`), `:361` (`resp.json()`), `:400-401` (`resp.bytes()`), `:432-435` (`resp.json()`); `crates/core/src/tools/context7.rs:111-114` (`json()`), `:157-158` (`text()`); time limits `web.rs:237-243` (`build_client`, client wide `.timeout`, which in reqwest covers the body read), `:279`, `:351`, `:423` (10 s), `:90` (fetch clamp 1 to 30), `:144` (download clamp 1 to 120); `context7.rs:86-91`, `config.rs:644-646` (30 s) |
| 5 | no decompression | `Cargo.toml:110` (`reqwest` with `default-features = false`, features `json`, `rustls-tls`, `multipart`); `cargo tree --workspace --all-features -e features -i reqwest` lists no `gzip`, `brotli`, `deflate` or `zstd`; measured 2026-11-11 (throwaway probe, reqwest 0.12.28, deleted): the request carries only `accept: */*` and a body marked `Content-Encoding: gzip` arrives byte for byte |
| 6 | write path after the read | `crates/core/src/tools/web.rs:103`, `:145` (`volume`: membership, `crates/core/src/tools/mod.rs:148-153`), `:106-115`, `:148-157` (`fs_ops::write_bytes` with overwrite true); `crates/core/src/core/fs_ops.rs:594-622` (read before write `:608-610`, `admit_file` `:612-613`); `crates/core/src/quota.rs:47-61`, `:96-114` (project growth, then session); messages `crates/core/src/safety.rs:142-144`, `crates/core/src/errors.rs:103-111`, `crates/core/src/safety.rs:130`; default allowance `config.rs` template snapshot `write_quota_bytes: 52428800` (`crates/core/src/config.rs:1691`) |
| 7 | failure messages and logging | `crates/core/src/tools/web.rs:280-286`, `:288`, `:352-361`, `:387-401`; `crates/core/src/tools/context7.rs:102-114`, `:140-158` (`404` at `:150`, status at `:153-155`); `crates/core/src/mcp/server.rs:46-62` (`to_call_result`), `crates/core/src/logging.rs:79-85` |
| 8 | precedents | `crates/core/src/git/provider/mod.rs:39-42` (`MAX_RESPONSE_BYTES` 16 MiB), `:398-448` (`read_capped`, message `:439`, `ERR_INVALID_ARGUMENT`); `crates/core/src/docs/service.rs:225-250` (`read_body_capped`, SPEC-0089) |
| 9 | governing specs | `sdlc/specs/SPEC-0009-optional-integrations/spec.md:12-14`, `:49`, FR-002 to FR-008 (`:176-232`); `sdlc/specs/SPEC-0027-mcp-tool-surface-ignores-flags/sdlc-spec.md`; `sdlc/specs/SPEC-0020-*`; `sdlc/specs/SPEC-0084-*` and `crates/core/src/tools/archive_single.rs:185-253` (`Caps`, `pick_caps` at `:246`, tie to the project); `sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/sdlc-spec.md` 3.2, 14.2 row 22 |
| 10 | tests today | `crates/core/src/tools/web.rs` 5 tests (module `:457-509`, tests at `:468`, `:477`, `:486`, `:495`, `:503`), `crates/core/src/tools/context7.rs` 4 tests (module `:178-217`, tests at `:189`, `:195`, `:203`, `:211`), `crates/core/src/tools/tool_surface_e2e.rs:487-497` (`bt_012`), `:499-530` (`bt_014`, a 14 byte stand in page), `crates/core/src/tools/all.rs:120-121`; none send a large answer, a declared length, a compressed answer, or download (`rg -n '"web\.download"' crates/core/src` finds only the registration) |
| 11 | transport piece size | measured 2026-11-11 with a throwaway probe (reqwest 0.12.28 as locked, `Cargo.lock:4766-4767`, local server, 20 971 520 byte body, declared and chunked): `chunk()` returned 72 to 73 pieces, the largest 507 904 bytes |

Test command: `cargo test -p mcp-fs-core --lib tools::web`, `cargo test -p mcp-fs-core --lib tools::context7`, `cargo test -p mcp-fs-core --lib tools::tool_surface_e2e`; full gate `cargo test --workspace`.

### 14.2 Class sweep

**Class:** a remote HTTP answer body, controlled by whoever runs the remote server, read into server memory with no bound on its size (CWE-400, CWE-770).

**Searches run** (whole repository source, `crates/`; tests filtered by their `#[cfg(test)]` module start):
1. `rg -n '\.text\(\)|\.bytes\(\)|\.json(::<[^>]*>)?\(\)|bytes_stream|\.chunk\(\)|read_to_end|read_to_string' crates/core/src crates/agent/src`: idiom "a response body read whole, or read piece by piece".
2. `rg -n 'reqwest::Client::builder|reqwest::get|\.send\(\)\.await' crates/core/src crates/agent/src`: idiom "a place that issues an outbound HTTP request", to catch a body read through a helper.
3. `rg -n 'clone_to_temp|RepoBuilder|FetchOptions|transfer_progress' crates/core/src/git`: idiom "a remote transfer through libgit2", which reads no body through reqwest.
Not searched: inbound request bodies (the REST upload, the git push route): another source, bounded by `UPLOAD_BODY_LIMIT` (`crates/core/src/api/dataplane.rs:117`, `:134`) and `git.max_pack_size_mb` (`crates/core/src/git/http/mod.rs:95-115`, `:203`).

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | search page `crates/core/src/tools/web.rs:288` | vulnerable (web family on) | FR-NEW-001, FR-NEW-002; E2E-SEC-001 |
| 2 | news list `crates/core/src/tools/web.rs:360-361` | vulnerable | FR-NEW-001; E2E-SEC-002 |
| 3 | fetch and download body `crates/core/src/tools/web.rs:400-401` (`fetch_raw`), reached from fetch `:410`, saved page `:105`, download `:147` | vulnerable; originating occurrence of BL-0148 | FR-NEW-001 to FR-NEW-006; E2E-SEC-004, E2E-SEC-005, E2E-SEC-006, E2E-SEC-007, E2E-SEC-010 |
| 4 | suggestions `crates/core/src/tools/web.rs:432-435` | vulnerable | FR-NEW-001; E2E-SEC-003 |
| 5 | library resolution `crates/core/src/tools/context7.rs:111-114` | vulnerable (documentation family on; the operator's service, answering from the internet) | FR-NEW-001; E2E-SEC-008 |
| 6 | library documentation `crates/core/src/tools/context7.rs:157-158` | vulnerable | FR-NEW-001, FR-NEW-002; E2E-SEC-009 |
| 7 | pull request provider answers `crates/core/src/git/provider/mod.rs:417` and every `resp.json()`/`resp.text()` in `crates/core/src/tools/git_pr.rs` (`:651`, `:713`, `:755`, `:817`, `:878`, `:993`, `:1032`, `:1055`, `:1135`, `:1170`, `:1211`, `:1260-1264`) | safe: these read an already capped buffer; `read_capped` refuses past `MAX_RESPONSE_BYTES` (`crates/core/src/git/provider/mod.rs:437-441`) or truncates at the diff cap (`:412-433`) | none |
| 8 | converter web answer `crates/core/src/docs/service.rs:237` | safe: `read_body_capped` (`:230-250`) stops at `doc_service.max_input_bytes` (SPEC-0089) | none |
| 9 | sign in flow with a code host `crates/core/src/git/oauth/device_flow.rs:219` (`resp.json::<T>()`, no timeout `:157-160`) | not in scope: the host is the configured GitHub endpoint or a GitLab base the operator declared in `git.hosts` (`crates/core/src/tools/git_auth.rs:317-322`), not a member chosen site | side finding to the owner (no backlog entry, project rule) |
| 10 | recognition `crates/core/src/docs/ocr.rs:116-119`, embedding `crates/core/src/search/embedding.rs:46-49`, reranking `crates/core/src/search/rerank.rs:64-67` | not in scope: operator configured endpoints | side finding to the owner |
| 11 | remote clone and fetch `crates/core/src/git/remote.rs:418-440` (`clone_to_temp`, libgit2 into a temporary folder, bounded by the deadline `:398-400`) | other idiom: disk, not a body in memory; time bound only | side finding to the owner |
| 12 | the client program `crates/agent/src/mcp.rs:125`, `:172`, `crates/agent/src/llm.rs:145` | out of class: the command line client reading its own server and its model provider, not the server | none |
| 13 | multipart fields `crates/core/src/api/dataplane.rs:634`, `:640`; `crates/core/src/docs/service.rs:1274` | out of class: inbound request bodies (capped, `dataplane.rs:117`) and a test stand in | none |
| 14 | object store reads `crates/core/src/storage/blob/s3.rs:110-122` (`body.collect()`) | out of class: the operator's object store returning blobs the server wrote, each admitted at write (`crates/core/src/quota.rs:47-61`) | none |
| 15 | search clients `crates/core/src/app.rs:111`, `crates/core/src/cli.rs:210` | the clients of row 10, built once; same disposition | side finding (row 10) |

### 14.3 Design inputs

- One capped reader serves every occurrence: the shape of `read_capped` (`crates/core/src/git/provider/mod.rs:398-448`) and `read_body_capped` (`crates/core/src/docs/service.rs:225-250`): read with `chunk()`, check before extending, drop the response on refusal (dropping closes the connection). Signal: E2E-SEC-001 to 010.
- The declared length is `Response::content_length()`; since the build has no decompression (14.1 row 5) it equals the bytes kept. Signal: E2E-014, E2E-018, E2E-SEC-007.
- `chunk()` hands pieces up to 507 904 bytes (14.1 row 11): the held bound counts buffer plus the piece just received, as `read_body_capped` measures `peak`; FR-NEW-005's margin of 1 048 576 covers it. SPEC-0089's FR-NEW-011 states 65 536 for the converter; whether its tests hold under larger pieces is for SPEC-0089's owner (side finding). Signal: every held assertion.
- The write limit reuses `quota::project_remaining_for` (`crates/core/src/quota.rs:221-231`) and `SafetyManager::session_remaining` (`crates/core/src/safety.rs:153-156`), tie to the project as `pick_caps` (`crates/core/src/tools/archive_single.rs:246-253`); the project refusal's `requested` differs from `Caps::refusal` (`:200-219`, the cap plus one, the cap including the replaced file) on purpose: FR-NEW-003 states net growth, as the write admission reports today (`crates/core/src/quota.rs:109-110`). Signal: E2E-019 to E2E-022.
- The destination path must be normalized and its replaced size read before the request (`crates/core/src/tools/web.rs:102-104`, `:145-146`: `volume` and `norm` already precede `fetch_raw`). Signal: E2E-010, E2E-021.
- Search, news and suggestions use fixed hosts (`crates/core/src/tools/web.rs:277`, `:346`, `:348`, `:421`): the tests need a seam pointing them at a stand in site (a test only base address, as `with_github_urls` does for the sign in flow, `crates/core/src/git/oauth/device_flow.rs:172-180`). Signal: E2E-002, E2E-007, E2E-015, E2E-035, E2E-SEC-001 to 003.
- "Answer held" needs a test hook like `LAST_OFFICE_HELD` (`crates/core/src/docs/extract.rs:181-185`) or SPEC-0089's converter hook, reading 0 when no body byte was read. Signal: every held assertion.
- "Stand in records bytes sent and early close" can reuse the stand in of `crates/core/src/tools/converter_bound_e2e.rs` (SPEC-0089). Signal: E2E-SEC-*.
- Today's `read response: ` prefix on a body timeout must survive the change of read (`crates/core/src/tools/web.rs:401`). Signal: E2E-030.
- The `web.fetch` description says it caps at 50 000 characters; whether to mention 16 MiB is the design's call (optional families are outside the golden contract, `.agent_docs/tools.md:636-644`).
- Docs to update: `.agent_docs/tools.md` (optional families), `.agent_docs/config.md` (web and context7 sections), `README.md` if it lists the families.

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code and the backlog entry, without the user, under their authorization.
- R1 scope: every remote answer read by the `web` and `context7` families; out: address filtering (CWE-918), redirects, time limits, operator configured endpoints, remote clone, the provider client and the converter (already bounded).
- R1 why now: BL-0148, deferred from SPEC-0089, exploitable on any deployment enabling the web family until shipped.
- R2 current state: Section 2 and 14.1, read from the code; reqwest behavior measured with a throwaway probe outside the repository, deleted.
- R3 scenarios: SC-001 to SC-005. R4 requirements: derived, Section 6. R5 NFR: refusal without waiting, membership before any request, no open connection, measurable held bytes.
- D-1 routing: the entry reads `Nature: BUG`; following SPEC-0084, SPEC-0087, SPEC-0088 and SPEC-0089, written as one FEAT spec: observable behavior changes (new refusals) and no spec defines a size bound the code violates (SPEC-0009 sets none). Override recorded here.
- D-2 depth M: three bounded contexts, 10 requirements, no persisted change.
- D-3 security `internal finding` (CWE-400, CWE-770); sweep and exploit tests included.
- D-4 reachability: both families off by default and absent from the shipped templates; in the default configuration nothing in this class is reachable; the spec covers deployments that turn them on.
- D-5 answer limit fixed at 16 777 216 bytes with the provider client's message and code, verbatim: the tree already has this rule for remote answers (`crates/core/src/git/provider/mod.rs:42`, `:439`); `WebConfig` and `Context7Config` hold no size setting (`crates/core/src/config.rs:623-662`) and adding one is not needed.
- D-6 download and saved page: the limit is the write limit (lower of project side and session side, tie to the project), with today's storage refusals verbatim, as SPEC-0084 did for a decode: a legitimate download within the allowance is not refused by a smaller fixed number, and the refusal a member gets is the one they get today after the whole read. No fixed ceiling on top: the operator's allowance is the bound, as for every other write.
- D-7 project refusal `requested`: net growth, as the write admission reports today; for a chunked answer, one byte past what the project can still take (limit minus used plus 1); for a declared length, that length minus the replaced file.
- D-8 declared length past a limit: refused before reading the body, both limits.
- D-9 compressed answers: counted as kept; today nothing is expanded and this increment adds no expansion.
- D-10 redirects out of scope; `<host>` is the host first asked.
- D-11 transport margin 1 048 576 bytes, from the measurement (507 904 bytes largest piece), not SPEC-0089's 65 536.
- D-12 order: argument checks, membership and path before any request; then transport, time limit and size, whichever first; then today's write checks. The read before write check stays after the read (unchanged); the size refusal therefore comes first when both apply (E2E-027).
- D-13 code `ERR_INVALID_ARGUMENT` for the answer limit (the provider's), the storage codes for the write limit; `ERR_INTERNAL_ERROR` kept for today's transport failures.
- D-14 tests: 49 new, 0 modified, 0 removed; the test plan is approved by the user's request "specify BL-0148".
- D-15 side findings (address filtering, unused web configuration values, operator endpoint reads, the sign in flow without timeout, remote clone without size bound, SPEC-0089's margin) are reported to the owner in the handoff, not filed: the project rule forbids a backlog entry without the owner's validation.
