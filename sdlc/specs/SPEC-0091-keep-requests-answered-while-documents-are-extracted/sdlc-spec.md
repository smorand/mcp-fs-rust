# Keep requests answered while documents are extracted: functional specification

> Id: SPEC-0091
> Nature: FEAT
> Depth: M
> Depth evidence: 1 bounded context (document text extraction, its six built in readers), 6 functional requirements, observable change (latency of every other request while a document decodes, a ceiling on documents decoding at once, a defined answer when a reader stops unexpectedly), no persisted change, no change of any extraction answer. Override with --depth.
> Status: designed
> Generated on: 2026-11-11
> Project: existing
> From backlog: BL-0146
> Depends on: none (SPEC-0088 and SPEC-0089 built: their limits, messages and measurement are kept as they are)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a
>
> Note: written non interactively. The user authorized every decision; the request "specify BL-0146" is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

While the server reads the text of a large document, every other request a member or an operator sends is held up: one PDF at the SPEC-0089 stream limit keeps a request processing thread busy for about 5.5 seconds, and a health check or a folder listing scheduled on that thread waits the whole time (5.50 seconds measured with one thread; 0.76 seconds behind two Word documents with two threads). This increment keeps every request that extracts no document answered within 250 ms while documents are being extracted, keeps the number of documents decoding at once at today's ceiling, and defines what the requester sees when a reader stops unexpectedly or when the requester gives up. No extraction answer, limit or message changes.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member asks for the text of a document through the assistant tool or the HTTP API. Word, Excel, PowerPoint, PDF, HTML and CSV documents are decoded by the server's built in readers; images go to the configured OCR provider; text files are decoded as text. [14.1 row 1]
- The decoding runs on the same request processing thread that received the request, from the first byte to the last, without giving the thread back. [row 2]
- The server runs as many request processing threads as the machine has processors (10 on the reference machine). Every request, the health check included, is served by one of them. [row 3]
- Measured on the reference machine (Section 14.5): the PDF of SPEC-0089 E2E-007 (one page decoding to exactly 104 857 600 bytes) takes 5.5 to 6.2 seconds to extract; the Word document of SPEC-0088 E2E-006 (one part of exactly 104 857 600 bytes) takes 0.7 to 0.9 seconds. With one request processing thread, a trivial request sent 50 ms after the PDF extraction started waited 5.50 seconds; with two threads and one free, it waited 57 microseconds; with two threads both decoding a Word document, it waited 0.76 seconds. [row 4]
- So today at most as many documents decode at once as there are request processing threads; any further request, extraction or not, waits for one to finish. [row 5]
- Archive extraction, the file diff, the converter's bundle reading and the database work already run apart from the request processing threads; a long archive does not hold up other requests. [row 6]
- An extraction answers from the up to date companion Markdown without decoding (only the package header check runs). Every extraction failure other than an unsupported format is answered `ERR_INVALID_ARGUMENT` with a message starting `could not extract <path>: `; a refusal leaves nothing (no companion, no artifact, no audit line, no charge) and is logged once at the informational level; a failure coded `ERR_INTERNAL_ERROR` is logged once at the error level. [row 7]
- If a reader stops unexpectedly today, the request's own processing ends without an extraction answer; no reader is known to do so. ASSUMED: no failing case is known; the outcome is untested. [row 8]
- The search index reads files as plain text and never calls these readers; the converter service (documentize, upload with conversion) is an external program and never calls them either. [row 9]

### 2.2 Specs governing this area

- SPEC-0088 (FR-NEW-001 to FR-NEW-011): Office part, call and content limits, their messages and the decoded bytes measurement. Unchanged.
- SPEC-0089 (FR-NEW-001 onward): PDF stream, load and object limits, their messages, the converter output cap and the PDF measurement. Unchanged.
- SPEC-0041 FR-NEW-002: the package header check comes first. Unchanged.
- SPEC-0019: companion, table and image artifacts and the write charge of an extraction. Unchanged.
- No spec defines how long other requests wait during an extraction: this increment decides it (FR-NEW-001).

### 2.3 What is verified today

- Every limit and refusal of SPEC-0088 and SPEC-0089 is exercised (48 and 50 tests), including three extractions at once (SPEC-0088 E2E-036). The decoded bytes figures they assert are read from a measurement taken on the thread that ran the request. [row 10]
- The answers of the six readers on small documents are covered by the extraction unit tests (51). [row 11]
- No test measures how long another request waits while a document decodes, nor what a requester sees when a reader stops unexpectedly or when the requester gives up mid extraction. [row 12]

## 3. Scope

### 3.1 In scope

- Every document text extraction that runs a built in reader (Word, Excel, PowerPoint, PDF, HTML, CSV), through the assistant tool and the HTTP API.
- The waiting time of every request that extracts no document while extractions run.
- The ceiling on documents decoding at once, and the order in which waiting extractions start.
- The answer when a reader stops unexpectedly, and what an abandoned extraction leaves.

### 3.2 Out of scope, and why

- Changing any extraction answer, limit, message or code: SPEC-0088 and SPEC-0089 own them; this increment only changes where and when the work runs.
- Images through the OCR provider and plain text files: the provider is already waited for without holding a thread, and decoding text is negligible.
- Symbol search across a folder and the Markdown to Word rendering: they also compute on the request thread, but on member sized source files and generated text; not measured here, reported as side findings.
- A time limit on an extraction, a queue length limit, or a refusal when the ceiling is reached: nothing asks for one; a waiting extraction waits as it does today.
- Stopping a decoding half way when its requester gives up: the readers have no stopping point; the abandoned decoding ends on its own (FR-NEW-005).
- A configurable ceiling: today's ceiling is the number of request processing threads, and it stays so.

### 3.3 Constraints imposed by the context

- Project rule: a request thread is never blocked on the database; the project's coding standards extend it to long computations. Imposed by the owner (14.1 row 13).
- The measurement of decoded bytes and content held, used by the SPEC-0088 and SPEC-0089 tests, keeps reporting each extraction's figures to the request that asked for it, so those tests pass unmodified. Imposed by the owner (backlog note, SPEC-0084 precedent).

## 4. Actors

- **Member:** a person in the project's member list who asks for a document's text or does anything else in the project.
- **Operator:** runs the server, sends health checks, reads the log.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Document text extraction | Reading a document's text, the companion, the limits | decoding, ceiling, place, companion |
| Project files | Listing, reading, writing files | folder listing, file read |
| Server operation | Health and log | health check, log line |

## 5. Usage scenarios

### SC-001: Another request while a document decodes
**Actor:** member or operator
**Preconditions:** a member's extraction of a large document is decoding.
**Flow:**
1. Another member lists a folder, reads a file, or the operator sends a health check → the answer arrives at most 250 ms later than when nothing decodes.
2. The extraction ends → its answer is exactly what it is today.
**Postconditions:** both requests got their usual answers.
**Exceptions:**
- EXC-001a: the extraction is refused by a limit → the refusal is exactly today's; the other requests were still answered within 250 ms.
- EXC-001b: the other request is refused (not a member) → its refusal arrives within 250 ms.

### SC-002: More extractions than places
**Actor:** several members
**Preconditions:** the server has N request processing threads; N documents are decoding.
**Flow:**
1. A member asks for one more document → the request waits, silently, for a place.
2. One decoding ends → the waiting extraction starts; waiting extractions start in arrival order.
3. Each extraction answers as today.
**Postconditions:** never more than N documents decoded at once.
**Exceptions:**
- EXC-002a: a decoding ends with a refusal → its place is free at once.
- EXC-002b: a request is refused before decoding (not a member, missing path, folder, audio or video) or answered from an up to date companion → it never waits for a place.

### SC-003: Answers unchanged
**Actor:** member
**Preconditions:** any document of the six formats.
**Flow:** 1. The member asks for its text → the answer, companion, artifacts, audit line and charge are byte identical to today's.
**Exceptions:** EXC-003a: every refusal keeps its code and message.

### SC-004: A reader stops unexpectedly
**Actor:** member
**Preconditions:** a reader stops before giving a result on `/docs/report.pdf`.
**Flow:**
1. The member gets `ERR_INTERNAL_ERROR` with `could not extract /docs/report.pdf: the document reader stopped unexpectedly`.
2. The operator finds one error line; the server keeps answering; the place is free.
**Exceptions:** EXC-004a: another extraction running at the same time ends normally.

### SC-005: The requester gives up
**Actor:** member
**Preconditions:** the member's extraction is decoding, or waiting for a place.
**Flow:**
1. The member abandons the request (closes the connection, cancels the call) → nothing is written, stored, audited or charged for it.
2. A decoding already started finishes on its own and keeps its place until then; a waiting one never starts.
**Exceptions:** EXC-005a: asking again later gives the full ordinary answer.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-S]: Other requests answered while documents decode
> WHILE one or more documents are being decoded for text extraction THE server SHALL add at most 250 ms to the answer time of every request that decodes no document, compared with the same request when no document is decoding.

- **Business rules:** holds whatever the number of extractions decoding or waiting; covers the health check, folder listings, file reads and writes, archive extraction, refusals, and extraction answers from an up to date companion; answer time is measured from the moment the request is sent to the moment its answer is received, on the server's machine; a request answered in under 1 ms when nothing decodes (a probe) is therefore answered within 250 ms; today's measured added wait is up to 5.5 seconds.
- **User visible text:** none new.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-U]: Ceiling on documents decoding at once
> The document text extraction SHALL decode at most as many documents at once as the server has request processing threads, and SHALL start a waiting extraction as soon as a decoding ends, in arrival order.

- **Business rules:** only the six built in readers (Word, Excel, PowerPoint, PDF, HTML, CSV) take a place; images, text files and other suffixes do not; a place is taken when the decoding starts and freed when it ends, success, refusal or unexpected stop alike; a request refused before decoding (not a member, missing path, folder, audio or video) and an answer from an up to date companion take no place and never wait; waiting has no time limit and no message.
- **User visible text:** none.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-U]: Extraction answers unchanged
> The document text extraction SHALL answer, for the same document, options and member, exactly what it answers today: text, preview, character count, truncation, notes, companion, table and image artifacts, audit line, write charge, and every refusal code and message.

- **Business rules:** the limits of SPEC-0088 (104 857 600, 268 435 456, 268 435 456 bytes) and SPEC-0089 (1 000 000 objects, 104 857 600 and 268 435 456 bytes) are unchanged; the order of refusals is unchanged.
- **User visible text:** unchanged.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: A reader stopping unexpectedly
> IF a reader stops before giving a result THEN THE document text extraction SHALL refuse that request with `ERR_INTERNAL_ERROR` and the message `could not extract <path>: the document reader stopped unexpectedly`, leave no companion, artifact, audit line or charge, log one error line naming the code and the path, and free its place.

- **Business rules:** `<path>` is the document path as the member's request names it after normalization (as in every `could not extract` message); every other request continues unaffected.
- **User visible text:** as above.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-005 [EARS-E]: An abandoned extraction
> WHEN a member abandons an extraction request while its document is decoding or waiting for a place THE document text extraction SHALL write no companion, store no artifact, record no audit line and charge nothing for it.

- **Business rules:** a decoding already started runs to its end and keeps its place until then; an extraction still waiting for a place never starts decoding; an abandon after the decoding ended is not covered and keeps today's behavior; asking again later is an ordinary extraction.
- **User visible text:** none.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-006 [EARS-U]: Both doors alike
> The server SHALL apply FR-NEW-001 to FR-NEW-005 alike to extractions asked through the assistant tool and through the HTTP API.

- **Business rules:** the HTTP API carries the same code and message as the error and detail of its answer, as for every refusal today.
- **Scenario:** SC-001, SC-004, SC-005
- **Priority:** Must

### 6.2 Modified

None. FR-NEW-003 keeps every requirement of SPEC-0088, SPEC-0089, SPEC-0041 and SPEC-0019 as written.

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- FR-NEW-001 is the performance requirement. An extraction alone takes no more than 10% longer than today for the PDF of E2E-001 and the Word document of E2E-002 (measured in Section 14.5).

### 7.2 Security and access
- Unchanged: membership is checked before any place is taken (E2E-021).

### 7.5 Reliability
- A reader stopping unexpectedly ends only its own request (FR-NEW-004); the server keeps running.

### 7.6 Operability
- The operator's measurement of decoded bytes and content held per document keeps reporting each extraction's figures to the request that asked for it; the measurement additionally reports the most documents decoding at once (used by E2E-006, E2E-007, E2E-014). The SPEC-0088 and SPEC-0089 suites read it unmodified.
- One informational log line per refusal and one error line per unexpected stop, as today for each code.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | no extraction running | requests answered as today | E2E-001 (before the extraction starts) |
| Boundary values | exactly N documents decoding, one more asked | the extra one waits, starts when one ends | E2E-003, E2E-006 |
| Boundary values | one request processing thread | extractions decode one at a time | E2E-007, E2E-014 |
| Concurrent actions | three members extract at once | all answer, at most two decode at once | E2E-006, E2E-024 |
| Slow or lost connectivity | requester gives up mid decoding | nothing left, place held until decoding ends | E2E-013, E2E-014, E2E-027 |
| Slow or lost connectivity | requester gives up while waiting | never starts | E2E-028 |
| Access states | not a member asks during a full ceiling | refused within 250 ms, no place | E2E-021 |
| Data variations | each of the six formats | answers byte identical | E2E-016 |
| Navigation | refresh, repeat after abandon | ordinary extraction | E2E-015 |
| Partial operations | reader stops unexpectedly | refused, nothing left, place freed | E2E-009, E2E-012 |
| Dependency failures | not applicable: no new dependency; the OCR provider is out of scope | | |
| Ordering and timing | waiting extractions | start in arrival order | E2E-007 |
| Exhaustion | refusal by a limit during other requests | refusal unchanged, others answered | E2E-004, E2E-005 |
| Exhaustion | up to date companion during a full ceiling | answered within 250 ms | E2E-018 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files are built by the design. Unless a test says otherwise: the project is `atlas`; members `mia@example.com` (owner), `noa@example.com` and `ola@example.com`; the call goes through the assistant tool with default options (`max_chars` 200 000, no refresh, OCR on); the session write allowance is the default 52 428 800 bytes; the server runs with **one request processing thread** unless the test says two (one thread is where today's stall shows with a single extraction; with two, today's stall shows once both decode). "The large PDF" is `/docs/large.pdf`, the document of SPEC-0089 E2E-007 (page 1 decodes to exactly 104 857 600 bytes, the letter `b` then spaces). "The large Word document" is `/docs/large.docx`, the document of SPEC-0088 E2E-006 (one part of exactly 104 857 600 bytes of the letter `b`). `/notes/todo.txt` holds `buy milk`. "A probe" is one of: the operator's health check (answered healthy), noa's listing of `/notes` (shows `todo.txt`), noa's read of `/notes/todo.txt` (returns `buy milk`); "answered in time" means within 250 ms of being sent. "Nothing left" means as in SPEC-0088 Section 9: no companion appears (or an existing one keeps its bytes), no artifact is listed, no audit line, and a following 52 428 800 byte write by the same member succeeds. "Most at once" is the operator measurement of the most documents decoding at the same time during the test. "Recorded answer" means the answer recorded on the code before this increment for the same file and options.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-NEW-006 | Critical |
| E2E-002 | New | happy | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002 | Critical |
| E2E-004 | New | failure | SC-001 | FR-NEW-001, FR-NEW-003 | High |
| E2E-005 | New | failure | SC-001 | FR-NEW-001, FR-NEW-003 | High |
| E2E-006 | New | happy | SC-002 | FR-NEW-002 | Critical |
| E2E-007 | New | edge | SC-002 | FR-NEW-002 | High |
| E2E-008 | New | failure | SC-002 | FR-NEW-002 | High |
| E2E-009 | New | failure | SC-004 | FR-NEW-004 | Critical |
| E2E-010 | New | side effect | SC-004 | FR-NEW-004 | High |
| E2E-011 | New | failure | SC-004 | FR-NEW-004, FR-NEW-006 | High |
| E2E-012 | New | edge | SC-004 | FR-NEW-002, FR-NEW-004 | High |
| E2E-013 | New | failure | SC-005 | FR-NEW-005 | High |
| E2E-014 | New | side effect | SC-005 | FR-NEW-002, FR-NEW-005 | High |
| E2E-015 | New | happy | SC-005 | FR-NEW-005 | Medium |
| E2E-016 | New | happy | SC-003 | FR-NEW-003 | Critical |
| E2E-017 | New | failure | SC-003 | FR-NEW-003 | Critical |
| E2E-018 | New | edge | SC-003 | FR-NEW-001, FR-NEW-002, FR-NEW-003 | High |
| E2E-019 | New | happy | SC-001 | FR-NEW-001, FR-NEW-006 | High |
| E2E-020 | New | failure | SC-003 | FR-NEW-003, FR-NEW-006 | High |
| E2E-021 | New | failure | SC-001 | FR-NEW-001, FR-NEW-002 | Medium |
| E2E-022 | New | failure | SC-002 | FR-NEW-001, FR-NEW-002 | Medium |
| E2E-023 | New | side effect | SC-001 | FR-NEW-001, FR-NEW-003 | High |
| E2E-024 | New | side effect | SC-002 | FR-NEW-002, FR-NEW-003 | Medium |
| E2E-025 | New | side effect | SC-003 | FR-NEW-003 | High |
| E2E-026 | New | happy | SC-004 | FR-NEW-003, FR-NEW-004 | Medium |
| E2E-027 | New | failure | SC-005 | FR-NEW-005, FR-NEW-006 | Medium |
| E2E-028 | New | edge | SC-005 | FR-NEW-002, FR-NEW-005 | High |

Coverage (28 tests: 28 new, 0 modified, 0 removed): happy 7, failure 11, side effect 5, edge 5, state transition 0 (no persisted state: a request waits, decodes, answers), security 0 (not a security lot). Happy to failure ratio 1:1.57. Existing tests modified: 0; the 48 SPEC-0088 and 50 SPEC-0089 tests and the 51 extraction unit tests pass unmodified (E2E-017).

### 9.2 New tests

#### E2E-001: Health check during a large PDF extraction
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-006; **Priority:** Critical
- **Given** the large PDF; one health check answered in time before anything starts
- **When** mia asks for its text and, from 50 ms after, the operator sends a health check every 100 ms until mia's answer arrives
- **Then** every health check is answered healthy in time (today: the first waits about 5.5 seconds)
- **And** mia's answer reports truncated text whose preview after the page heading is only the letter `b`, as in SPEC-0089 E2E-007

#### E2E-002: Folder listing during a large Word extraction
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** Critical
- **Given** the large Word document
- **When** mia asks for its text and noa lists `/notes` every 100 ms from 50 ms after until mia's answer arrives
- **Then** every listing shows `todo.txt` and is answered in time
- **And** mia's answer reports truncated text made of the letter `b`

#### E2E-003: Every place taken, one more waiting, probes still answered
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Critical
- **Given** a server with two request processing threads; three copies of the large PDF, `/docs/a.pdf`, `/docs/b.pdf`, `/docs/c.pdf`
- **When** mia, noa and ola ask for one each at the same moment, and noa reads `/notes/todo.txt` every 100 ms from 50 ms after until all three answers arrive
- **Then** every read returns `buy milk` in time
- **And** the three answers are identical apart from their paths; most at once is 2

#### E2E-004: A PDF refused by its limit while probes run
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** `/docs/bomb.pdf`, the document of SPEC-0089 E2E-SEC-001
- **When** mia asks for its text while the operator sends a health check every 100 ms from 50 ms after
- **Then** mia is refused `ERR_INVALID_ARGUMENT` with `could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** every health check is answered in time; nothing left

#### E2E-005: An Excel workbook refused by the content limit while probes run
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** `/docs/long.xlsx`, the workbook of SPEC-0088 E2E-021 (140 rows)
- **When** mia asks for its text while noa lists `/notes` every 100 ms from 50 ms after
- **Then** mia is refused `ERR_INVALID_ARGUMENT` with `could not extract /docs/long.xlsx: the extracted content exceeds 268435456 bytes`
- **And** every listing shows `todo.txt` in time; nothing left

#### E2E-006: Three extractions, two places
- **Category:** happy; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Critical
- **Given** a server with two request processing threads; `/docs/a.pdf`, `/docs/b.pdf`, `/docs/c.pdf` as in E2E-003
- **When** mia, noa and ola ask for one each at the same moment
- **Then** all three succeed with the answer of E2E-001 for their own path
- **And** most at once is 2

#### E2E-007: One place, arrival order
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** `/docs/a.pdf` and `/docs/b.pdf` as in E2E-003; `/docs/memo.docx` whose text is `# Results` then `Quarterly results`
- **When** mia asks for `/docs/a.pdf`, then 50 ms later noa asks for `/docs/memo.docx`, then 50 ms later ola asks for `/docs/b.pdf`
- **Then** the answers arrive in the order a, memo, b; memo's preview is `# Results\n\nQuarterly results`
- **And** most at once is 1; a health check sent 100 ms after ola's request is answered in time

#### E2E-008: A refused decoding frees its place
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** High
- **Given** a server with two request processing threads; `/docs/m.pdf` and `/docs/n.pdf`, two copies of `/docs/bomb.pdf` (E2E-004), and `/docs/memo.docx` (E2E-007)
- **When** mia and noa ask for m and n at the same moment, and ola asks for memo 50 ms later
- **Then** mia and noa are refused with the message of E2E-004 for their paths; ola gets memo's preview `# Results\n\nQuarterly results`
- **And** most at once is 2; nothing left for m and n

#### E2E-009: A reader stops unexpectedly
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004; **Priority:** Critical
- **Given** `/docs/report.pdf`, a one page PDF reading `Revenue`, and the operator's test fault that makes the PDF reader stop unexpectedly on it
- **When** mia asks for its text
- **Then** mia is refused `ERR_INTERNAL_ERROR` with `could not extract /docs/report.pdf: the document reader stopped unexpectedly`
- **And** nothing left

#### E2E-010: What the operator sees after an unexpected stop
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** E2E-009 just happened
- **When** the operator reads the log and sends a health check
- **Then** the log holds exactly one error line containing `ERR_INTERNAL_ERROR` and `/docs/report.pdf`, and no audit line names `/docs/report.pdf`
- **And** the health check is answered healthy in time

#### E2E-011: Unexpected stop through the HTTP API
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004, FR-NEW-006; **Priority:** High
- **Given** as E2E-009
- **When** mia asks for the text through the HTTP API
- **Then** the answer is a server error whose error is `ERR_INTERNAL_ERROR` and whose detail is `ERR_INTERNAL_ERROR: could not extract /docs/report.pdf: the document reader stopped unexpectedly`
- **And** nothing left

#### E2E-012: One reader stops, another extraction runs on
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** High
- **Given** a server with two request processing threads; `/docs/report.pdf` with the fault of E2E-009, the large PDF, and `/docs/memo.docx`
- **When** noa asks for the large PDF, mia for `/docs/report.pdf` 50 ms later, and ola for memo 50 ms after mia's refusal
- **Then** mia gets the refusal of E2E-009; noa gets the answer of E2E-001; ola gets memo's preview while noa's decoding still runs
- **And** most at once is 2

#### E2E-013: Mia gives up mid decoding
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** the large PDF
- **When** mia asks for its text and abandons the request 1 second later
- **Then** after 10 seconds no `/docs/large.md` is in the listing of `/docs`, no artifact is listed for `/docs/large.pdf`, no audit line names it
- **And** a following 52 428 800 byte write by mia succeeds

#### E2E-014: An abandoned decoding keeps its place until it ends
- **Category:** side effect; **Scenario:** SC-005; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** High
- **Given** the large PDF; `/docs/memo.docx`
- **When** mia asks for the large PDF and abandons it 1 second later, and noa asks for memo 100 ms after the abandon
- **Then** noa's answer (memo's preview) arrives after the large PDF's decoding ended, and no earlier than 4 seconds after mia's request
- **And** most at once is 1; a health check sent with noa's request is answered in time

#### E2E-015: Asking again after giving up
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** E2E-013 just happened and its decoding ended
- **When** mia asks for the text of the large PDF again
- **Then** the answer equals the answer of E2E-001, not served from a companion
- **And** one audit line names `/docs/large.pdf` and `/docs/large.md` now exists

#### E2E-016: Every format answers as recorded
- **Category:** happy; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Critical
- **Given** `/docs/memo.docx` (E2E-007), `/docs/book.xlsx` (one sheet `Data`, cell A1 `cell`), `/docs/deck.pptx` (one slide `Intro`), `/docs/report.pdf` (E2E-009, no fault), `/docs/page.html` (`<h1>Title</h1><p>Body é</p>`), `/docs/data.csv` (`a,b` then `1,2`), `/docs/notes.txt` (`line one`), `/docs/scan.png` with OCR disabled
- **When** mia asks for the text of each
- **Then** each answer equals its recorded answer byte for byte
- **And** each companion written equals its recorded companion byte for byte

#### E2E-017: Every refusal and limit as recorded
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** Critical
- **Given** the 48 SPEC-0088 tests, the 50 SPEC-0089 tests and the 51 extraction unit tests, unmodified
- **When** they run against this increment
- **Then** all pass, including every decoded bytes and content held figure they assert
- **And** none of their files changed in this increment

#### E2E-018: An up to date companion answers during a full ceiling
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-003; **Priority:** High
- **Given** a server with two request processing threads; `/docs/memo.docx` already extracted, its companion up to date; `/docs/a.pdf` and `/docs/b.pdf` decoding for noa and ola
- **When** mia asks for memo's text 50 ms after they started
- **Then** mia's answer arrives in time, marked as served from the companion, preview `# Results\n\nQuarterly results`
- **And** most at once stays 2

#### E2E-019: The HTTP API door during a large extraction
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-006; **Priority:** High
- **Given** the large PDF
- **When** mia asks for its text through the HTTP API, and noa reads `/notes/todo.txt` through the HTTP API every 100 ms from 50 ms after
- **Then** every read returns `buy milk` in time
- **And** mia's answer equals the answer of E2E-001

#### E2E-020: A refusal through the HTTP API, as recorded
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/bomb.pdf` (E2E-004)
- **When** mia asks for its text through the HTTP API
- **Then** the answer's error is `ERR_INVALID_ARGUMENT` and its detail `ERR_INVALID_ARGUMENT: could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** nothing left

#### E2E-021: Not a member, during a full ceiling
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** a server with two request processing threads; `/docs/a.pdf` and `/docs/b.pdf` decoding for mia and noa; `eve@example.com` is not a member of `atlas`
- **When** eve asks for the text of `/docs/memo.docx` 50 ms after they started
- **Then** eve is refused in time with `ERR_FORBIDDEN` and a message containing `'eve@example.com' is not a member of 'atlas'`
- **And** most at once stays 2

#### E2E-022: Missing path, during a full ceiling
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Medium
- **Given** as E2E-021
- **When** ola asks for the text of `/docs/none.pdf`, which does not exist
- **Then** ola is refused in time with `ERR_NOT_FOUND` and `not a file: /docs/none.pdf`
- **And** most at once stays 2

#### E2E-023: The extraction's own traces are unchanged by the probes
- **Category:** side effect; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-003; **Priority:** High
- **Given** E2E-001 just ran
- **When** mia lists `/docs` and reads the audit trail
- **Then** `/docs/large.md` exists and equals the recorded companion of the large PDF byte for byte
- **And** the audit trail holds exactly one extraction line for `/docs/large.pdf`, and noa's and the operator's probes added no line

#### E2E-024: Three extractions, three companions, three charges
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-003; **Priority:** Medium
- **Given** E2E-006 just ran
- **When** each member lists `/docs` and reads the audit trail
- **Then** `/docs/a.md`, `/docs/b.md`, `/docs/c.md` exist, each equal to the recorded companion of the large PDF
- **And** one audit line per document, each in its requester's name; each member's write allowance was charged exactly the recorded charge of one extraction

#### E2E-025: Companion, artifacts and charge as recorded
- **Category:** side effect; **Scenario:** SC-003; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/docs/book.xlsx` of E2E-016, extracted by mia
- **When** mia lists the table artifacts of `/docs/book.xlsx` and checks her remaining write allowance
- **Then** the table list equals the recorded one (one table, sheet `Data`)
- **And** the remaining allowance equals the recorded remaining allowance

#### E2E-026: The same document after the fault is removed
- **Category:** happy; **Scenario:** SC-004; **Requirements:** FR-NEW-003, FR-NEW-004; **Priority:** Medium
- **Given** E2E-009 just happened, then the test fault is removed
- **When** mia asks for the text of `/docs/report.pdf`
- **Then** the answer equals the recorded answer of `/docs/report.pdf`, preview containing `Revenue`
- **And** `/docs/report.md` exists and one audit line names it

#### E2E-027: Giving up through the HTTP API
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-005, FR-NEW-006; **Priority:** Medium
- **Given** the large PDF
- **When** mia asks for its text through the HTTP API and closes the connection 1 second later
- **Then** after 10 seconds no `/docs/large.md`, no artifact, no audit line for it
- **And** a following 52 428 800 byte write by mia succeeds

#### E2E-028: Giving up while waiting for a place
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** High
- **Given** the large PDF decoding for noa; `/docs/c.pdf` as in E2E-003
- **When** mia asks for `/docs/c.pdf` 50 ms later and abandons it 500 ms later
- **Then** after noa's answer and 10 more seconds, no `/docs/c.md`, no artifact and no audit line for `/docs/c.pdf`
- **And** most at once stayed 1, and `/docs/c.pdf` was never decoded (its decoded bytes measurement is absent)

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-006 | E2E-001, E2E-002, E2E-019 | E2E-004, E2E-005, E2E-021 | E2E-003 | E2E-023 |
| SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-003 | E2E-006 | E2E-008, E2E-022 | E2E-007 | E2E-024 |
| SC-003 | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-006 | E2E-016 | E2E-017, E2E-020 | E2E-018 | E2E-025 |
| SC-004 | FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-006 | E2E-026 | E2E-009, E2E-011 | E2E-012 | E2E-010 |
| SC-005 | FR-NEW-002, FR-NEW-005, FR-NEW-006 | E2E-015 | E2E-013, E2E-027 | E2E-028 | E2E-014 |

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, 002, 003, 004, 005, 018, 019, 021, 022, 023 |
| FR-NEW-002 | E2E-003, 006, 007, 008, 012, 014, 018, 021, 022, 024, 028 |
| FR-NEW-003 | E2E-004, 005, 016, 017, 018, 020, 023, 024, 025, 026 |
| FR-NEW-004 | E2E-009, 010, 011, 012, 026 |
| FR-NEW-005 | E2E-013, 014, 015, 027, 028 |
| FR-NEW-006 | E2E-001, 011, 019, 020, 027 |

## 11. Open questions

None.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Decoding | The work a built in reader does to turn a document's bytes into its text, from start to result | Document text extraction |
| Request processing thread | One of the server's threads that receive and answer requests; the server runs as many as the machine has processors | Server operation |
| Place | The right to decode one document; there are as many as request processing threads | Document text extraction |
| Ceiling | The most documents decoding at once: the number of places | Document text extraction |
| Probe | A request that decodes no document, used by the tests to measure waiting: health check, folder listing, file read | Server operation |
| Answered in time | Answered within 250 ms of being sent | Server operation |
| Companion | The Markdown file written next to a document holding its extracted text | Document text extraction |
| Abandon | The requester closes the connection or cancels the call before the answer | Document text extraction |
| Unexpected stop | A reader ends without giving a text or a refusal | Document text extraction |
| Most at once | The operator measurement of the most documents decoding at the same time | Server operation |
| Recorded answer | The answer recorded on the code before this increment for the same file and options | Document text extraction |
| Nothing left | No companion, no artifact, no audit line, no charge, as defined in SPEC-0088 Section 9 | Document text extraction |
| Health check | The operator's request asking whether the server is up | Server operation |
| Write allowance | The bytes a member is allowed to write in a session, 52 428 800 by default | Project files |
| Artifact | A table or image kept from a document's extraction, listed per document | Document text extraction |
| Audit line | The one line the audit trail records per successful extraction | Server operation |
| Answer time | From the moment a request is sent to the moment its answer is received | Server operation |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 4 | 6 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied after round 1:** F-1 (G1) FR-NEW-001 bound every request that decodes nothing to 250 ms absolute, which a slow request of its own (an archive extraction) cannot meet: restated as at most 250 ms added. F-2 (G1) FR-NEW-005 promised no companion for any abandon before the answer, which races the companion write: restricted to an abandon while decoding or waiting, the rest keeps today's behavior. F-3 (G7) with two threads and one decoding, today's probes already answer (57 µs, 14.5), so E2E-001, 002, 004, 005, 019 did not exercise the change: the default test server became one thread, tests needing two places say so. F-4 (G1) which suffixes take a place was open: only the six built in readers. A-1 to A-4 citations recounted (`extract.rs:165-179`, `:166-171`, `:172`, `:357-364`, `:180-190`). A-5 a file name (`AGENTS.md`) in Section 3.3 moved to 14.1 row 13. A-6 two non requirement modals rephrased (2.2, glossary); the remaining `could` hits are the verbatim message prefix `could not extract`.

**How the gates ran:** the brief forbids subagents, so the self consistency check (Phase 10) and the audit (Phase 11, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks: ids (28 summarized, 28 specified), categories (happy 7, failure 11, edge 5, side effect 5), every requirement covered at least 5 times, no foreign id prefix, modal and implementation term scans. Round 2 re ran the scripts and re read Sections 5, 6 and 9.

**No implementation detail outside Section 14:** PASS. Error codes, verbatim messages, document suffixes and the option `max_chars` are WHAT, as in SPEC-0088 and SPEC-0089. "Request processing thread" is the operator facing name of the server's capacity, defined in the glossary; how it is counted is DI-4.

**Length:** about 590 lines, over the depth M target of 500, because of the 28 fully specified tests and the measurement table; no section is padding.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| Claim (Section 2) | Evidence |
|---|---|
| 1. Six built in readers, OCR for images, text decode | `crates/core/src/docs/extract.rs:165-179` (dispatch), readers `:515` `pdf`, `:896` `docx`, `:1054` `pptx`, `:1193` `xlsx`, `:1347` `html`, `:1443` `csv`; doors `crates/core/src/mcp/server.rs:2401-2422`, `crates/core/src/api/dataplane.rs:1487-1510`; both call `crates/core/src/core/fs_ops.rs:1720-1748` which awaits `crates/core/src/docs/extract.rs:330` `prepare_extraction`, which calls `Extractor::extract` at `:377-378` |
| 2. Decoding on the request thread | `crates/core/src/docs/extract.rs:166-171`: the six readers are plain synchronous calls inside `async fn extract` (`:151`), no `spawn_blocking`; the only `.await` in the match is the OCR image arm (`:172`, `:209`) |
| 3. Thread count | `crates/mcp-fs/src/main.rs:6-8`: `#[tokio::main]` with no `worker_threads`, so tokio's default multi thread runtime: one worker per CPU core, overridable by `TOKIO_WORKER_THREADS` (ASSUMED: tokio documentation, not in this repository); `rg -n "worker_threads" crates/core/src crates/mcp-fs/src` finds only test attributes; `sysctl -n hw.ncpu` = 10 on the reference machine. No request concurrency limit in `crates/core/src/app.rs` (`rg "ConcurrencyLimit|Semaphore" app.rs`: 0 hits) |
| 4. Measurements | Section 14.5 |
| 5. Today's implicit ceiling | follows from rows 2 and 3: a decoding holds its worker until it returns |
| 6. Other heavy work already off the request threads | `crates/core/src/tools/archive.rs:213-229`, `:344-354`, `:364`; `crates/core/src/tools/archive_single.rs:287`; `crates/core/src/tools/diff_files.rs:94`; `crates/core/src/docs/service.rs:680` (bundle read) and `:394` (`tokio::process::Command`); `spawn_blocking` count per file: archive.rs 6, search/bm25_sqlite.rs 6, search/vector_sqlite.rs 6, storage/rel/sqlite.rs 2, logging.rs 2, tools/git.rs 2, and 1 each in git/http/mod.rs, docs/service.rs, storage/sqlite.rs, tools/archive_single.rs, storage/blob/local.rs, tools/archive_rar.rs, tools/diff_files.rs; 0 in docs/extract.rs and core/fs_ops.rs |
| 7. Cached answer, refusal shape and logging | cached path `crates/core/src/docs/extract.rs:349-371` (header check `:357-364`); error mapping `:378-387`; MCP log `crates/core/src/mcp/server.rs:54-60` and `crates/core/src/logging.rs:83` (ERROR for a 5xx); REST `crates/core/src/api/dataplane.rs:259-266` (`{"error": code, "detail": "<CODE>: <message>"}`); `ERR_INTERNAL_ERROR` `crates/core/src/errors.rs:22`, status 500 `:139` |
| 8. Unexpected stop today | a panic in a reader unwinds the request future on its worker; no code catches it in `extract.rs`; ASSUMED outcome (no test) |
| 9. Search and converter do not call the readers | `crates/core/src/search/indexer.rs:168-175` (UTF-8 read only); `Extractor::new` has one production caller, `crates/core/src/docs/extract.rs:377` |
| 10. Limit suites and the measurement hook | `crates/core/src/tools/office_bound_e2e.rs` 48 tests, `crates/core/src/tools/pdf_bound_e2e.rs` 50 tests (`rg -c "#\[tokio::test\|#\[test\]"`); hook thread locals `crates/core/src/docs/extract.rs:575-582` (`LAST_OFFICE_HELD`, `OFFICE_HELD`), `:598-605` (`LAST_PDF_HELD`, `PDF_HELD`); set by the reader on its own thread `:740-750` and `crates/core/src/docs/pdf_bound.rs:179`; copied per file name on the caller's thread `crates/core/src/docs/extract.rs:180-190`; read by `crates/core/src/tools/archive_testkit.rs:505-517`, `crates/core/src/tools/pdf_testkit.rs:956-971`, and directly `crates/core/src/tools/pdf_bound_e2e.rs:544`, `:605`; three concurrent extractions `crates/core/src/tools/office_bound_e2e.rs:984-1008`; log capture on the locking thread `:939-948` |
| 11. Extraction unit tests | `crates/core/src/docs/extract.rs` 51 test functions (`rg -c`) |
| 12. No latency, stop or abandon test | `rg -n "Instant\|elapsed" crates/core/src/tools/office_bound_e2e.rs crates/core/src/tools/pdf_bound_e2e.rs crates/core/src/docs/extract.rs`: no timing assertion |
| 13. Project rule (Section 3.3) | `AGENTS.md` Conventions: "Never block a request thread on the database"; the rust coding standards: CPU bound work goes through `spawn_blocking` |

Test command: `cargo test --workspace` (the house `./test.sh`); the bound suites alone: `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0088_ e2e_0089_ --test-threads=1`.

### 14.3 Design inputs

- DI-1 Constraint on tests, not a design: `OFFICE_HELD` and `PDF_HELD` are `cfg(test)` thread locals written by the reader on the thread it runs on (`extract.rs:740-750`, `pdf_bound.rs:179`) and copied on the caller's thread (`extract.rs:180-190`). Moving the decoding to another thread must carry the figures back so they land in the caller thread's map, as SPEC-0084 DEC-006 does for `LAST_HIGH_WATER`; otherwise every hook assertion of E2E-017 reads 0 or panics (`pdf_testkit.rs:963-966` panics on a missing record). Signal: E2E-017.
- DI-2 Constraint on tests: the log capture records on the thread that took its lock (`office_bound_e2e.rs:939-948`, `logging.rs:174`); the refusal and error lines must stay emitted on the request side (`server.rs:57`, `dataplane.rs:264`), not from the decoding thread. Signal: SPEC-0088 E2E-035 and E2E-010 here.
- DI-3 Most of the suite runs on a current thread test runtime (`#[tokio::test]`); the ceiling must be at least 1 there, or the three concurrent extractions of `office_bound_e2e.rs:997-1001` deadlock. Signal: that test hangs.
- DI-4 The ceiling is "the number of request processing threads"; the design picks how the server knows it (runtime metrics, configured count) and how E2E-007, E2E-014, E2E-028 run a one thread server. Signal: "most at once" assertions.
- DI-5 "Most at once" is a new figure of the operator measurement (7.6); test only like the existing hook. Signal: E2E-006 does not compile without it.
- DI-6 The unexpected stop needs a test only fault (E2E-009) and a mapping of the stopped task to `ERR_INTERNAL_ERROR` with the message of FR-NEW-004, not through the `could not extract {path}: {}` wrap that turns every error into `ERR_INVALID_ARGUMENT` (`extract.rs:378-387`); the archive precedent words its own message (`archive.rs:229`). Signal: E2E-009 code assertion.
- DI-7 The decoding takes owned bytes (today `&[u8]` borrowed from `prepare_extraction`, `extract.rs:376-378`) and owned path; the OCR arm stays async on the request side.
- DI-8 E2E-005 relies on the content limit refusal decoding long enough to be measurable (more than 250 ms in the test build); if the design finds it shorter, it substitutes a document of the same refusal that decodes longer and says so. Signal: the test is green before the change.
- DI-9 Timing bounds hold in the test build (`[profile.dev]`, `Cargo.toml:166-168`, dependencies not optimized): measurements of 14.5 were taken there; 250 ms leaves a margin of 20 times the worst probe seen with a free thread (11.3 ms).
- DI-10 7.1's 10% allowance compares extraction durations alone before and after on the reference machine, informative, not an acceptance test.

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code and the backlog entry, without the user, under their authorization.
- D-1 routing: BL-0146 reads `Nature: DEBT`. Q1 answered yes: the waiting time of every other request is observable (5.5 s measured, 14.5) and changes; so rule 1 is contradicted by the code and the run takes FEAT, with "other requests are not stalled while a large extraction runs" as the testable behavior. Q2 no: no spec or contract bounds this latency; the AGENTS.md rule names the database, not computation.
- D-2 depth M: one bounded context, 6 requirements, no persisted change, no public contract change.
- D-3 security `n/a`: no bypass, injection, exposure, deserialization, traversal or advisory; latency under load of legitimate requests, bounded already by SPEC-0088 and SPEC-0089.
- D-4 bound: at most 250 ms added to any request that decodes nothing (20 times the worst free thread probe, 22 times below today's stall); stated as an added delay so a request slow on its own (an archive extraction) is not bound by it.
- D-4b the tests run a one thread server by default, so the probe tests fail on today's code (with a free thread today's probes already pass, 14.5).
- D-5 ceiling kept at today's implicit value (request processing threads) rather than unbounded, so memory held by concurrent extractions cannot grow past today's; waiting has no time limit and no message, as today.
- D-6 arrival order for waiting extractions.
- D-7 refused before decoding and cached answers take no place.
- D-8 unexpected stop: `ERR_INTERNAL_ERROR` with `could not extract <path>: the document reader stopped unexpectedly`; nothing left; one error line.
- D-9 abandon while decoding or waiting: nothing left; a started decoding runs to its end holding its place; a waiting one never starts; an abandon after the decoding ended keeps today's behavior.
- D-10 out of scope: symbol search and Markdown to Word rendering (reported as side findings), OCR, text files, time limits, configurable ceiling.
- D-11 tests: 28 new, 0 modified, 0 removed; the test plan is approved by the user's request "specify BL-0146".
- D-12 document file named `sdlc-spec.md`, as SPEC-0088 and SPEC-0089 (house style), not `spec.md`.
- D-13 no backlog entry created (project rule); side findings go to the handoff report.

### 14.5 Measurements

Throwaway test (deleted before commit) in `crates/core` test build, macOS, 10 cores, 2026-11-11. The readers were called through `Extractor::extract` on a `tokio` multi thread runtime of 1 or 2 workers; "trivial" is an empty task spawned and awaited.

| Runtime | Heavy work | Extraction time | Trivial request waited |
|---|---|---|---|
| 1 worker | large Word (one part of 104 857 600 bytes) | 0.92 s | 0.86 s (probe sent 50 ms after start) |
| 1 worker | large PDF (stream of 104 857 600 bytes) | 5.55 s | 5.50 s |
| 2 workers | large PDF, one worker free | 5.77 s | 57 µs (single probe); worst of 820 probes 3.1 ms, of 737 probes 11.3 ms |
| 2 workers | large Word, one worker free | 0.76 to 0.90 s | worst of 118 probes 3.9 ms |
| 2 workers | two large Word at once, both workers busy | 0.79 to 0.84 s | 0.76 s |
| idle | none | n/a | 51 to 851 µs |
