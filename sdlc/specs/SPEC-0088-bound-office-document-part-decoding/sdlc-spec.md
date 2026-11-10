# Bound the decoded size of Office document parts: functional specification

> Id: SPEC-0088
> Nature: FEAT
> Depth: M
> Depth evidence: 1 module (document text extraction, its Word, Excel and PowerPoint readers), 11 requirements, public behavior change (a document whose parts decode past a fixed limit is refused instead of being read whole; a part larger than it declares is refused instead of being read as absent), no persisted change. Override with --depth.
> Status: specified
> Generated on: 2026-11-10
> Project: existing
> From backlog: BL-0134
> Depends on: none (SPEC-0041 built: its header check keeps coming first)
> Security: internal finding
> CVSS: not scored
> Affected: every build whose text extraction reads `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` with its built in readers
> Fixed in: unreleased
>
> Note: written non interactively. The user authorized every decision; the request "spec BL-0134" is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

A member who asks for the text of a Word, Excel or PowerPoint document gets it as today when the document is of a reasonable size, and a clear refusal when one of the document's parts would decode to more than 104 857 600 bytes, when the parts read together would decode to more than 268 435 456 bytes, when a part holds more than it declares, or when the extracted content itself would pass 268 435 456 bytes. Today each part is decoded whole into the server's memory, so a document of a few hundred kilobytes whose main part holds gigabytes of repeated text uses that memory; a spreadsheet can also repeat one long shared text in every cell and multiply it without limit. Parts the extraction never reads (images, embedded files, nested archives) do not count and are still not opened. Both doors, the assistant tool and the HTTP API, answer alike.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member asks for the text of a document through the assistant tool or the HTTP API; for Word (`.docx`), Excel (`.xlsx`, `.xlsm`) and PowerPoint (`.pptx`, `.pptm`, `.potx`, `.ppsx`) documents, in any letter case, the server opens the document and reads the parts it needs; other suffixes (`.docm`, `.dotx`, legacy `.doc`) are decoded as plain text with a note. [14.1 row 1]
- Before reading a part, the whole document is checked for header inconsistencies (SPEC-0041) and refused with `could not extract <path>: not a valid OOXML package: <reason>`; a file that is not a zip at all is refused with the same prefix. [row 2]
- Parts read, in this order. Word: the main document part. Excel: the shared strings, the workbook, the workbook relationships, then each sheet in workbook order, a sheet listed twice being read twice. PowerPoint: the presentation, the presentation relationships, then per slide in presentation order the slide, its relationships and its notes; without a usable presentation part the slides are read in number order. [row 3]
- Each part read is decoded whole into memory, with no bound but the size the part declares; that declared size can be anything up to 18 446 744 073 709 551 615 bytes. Nothing counts the parts read together, and a sheet listed many times is decoded many times. [row 4]
- An Excel cell that refers to a shared text receives its own copy of that text, so one long shared text referred to by many cells multiplies in memory; the table grid kept for table artifacts is uncapped (SPEC-0019), only the Markdown table is cut at 400 rows. The character limit asked by the member (200 000 by default) is applied only after the whole text is built. [row 5]
- A part that decodes to more bytes than it declares, or that cannot be decoded at all, is treated as absent: a Word document answers `could not extract <path>: docx: word/document.xml is missing`, an Excel sheet or a PowerPoint slide is silently left out and the extraction succeeds with less text. [row 6]
- Images, media, embedded objects and nested archives inside the document are never read. [row 7]
- Every extraction failure other than an unsupported format is answered with `ERR_INVALID_ARGUMENT` and a message starting `could not extract <path>: `; a refusal writes no companion, stores no artifact, charges nothing and records no audit line, and is logged once at the informational level. On success the companion Markdown is written, charged to the member's session write allowance with the table artifacts, and one audit line is recorded. [row 8]
- When an up to date companion exists and the member did not ask to refresh, the stored text is returned without reading any part (only the header check runs). [row 9]
- Asking the converter service (documentize, upload with conversion) hands the document to an external program and never uses these readers; the search index reads files as plain text and skips a document that is not valid text. [row 10]

### 2.2 Specs governing this area

- SPEC-0041 FR-NEW-002: a document with disagreeing or duplicated parts is refused before text is extracted. Unchanged; it stays before every refusal of this increment (FR-NEW-009).
- SPEC-0019 DEC-009 (design) and its spec keep table grids uncapped for table artifacts. This increment bounds the whole extracted content, grids included, at 268 435 456 bytes (FR-NEW-005): a document past it is refused, where today its grid is kept whole. SPEC-0019 is not modified.
- SPEC-0085 deferred this class to BL-0134 (its D-11 and sweep occurrence 5) and set the convention reused here: a decoder stops one chunk of 65 536 bytes past a limit.
- SPEC-0036 moved the zip reader to its current major version; its name handling is unchanged. [row 11]

### 2.3 What is verified today

- Word, Excel and PowerPoint extraction is tested for styles, tables, entities, slide order, notes, shared strings, sheet names, a missing main part and a file that is not a zip; the SPEC-0041 refusals are tested through the extraction. [row 12]
- No test covers a part larger than a limit, the bytes held while decoding, parts read together, a part larger than it declares, a sheet listed twice, a shared text multiplied by many cells, or a declared size of 2^63 bytes or more. [row 12]

## 3. Scope

### 3.1 In scope

- The seven suffixes read by the built in readers, any letter case, from both doors, with and without a refresh.
- A limit on each part read, a limit on the parts read together, the refusal of a part larger than it declares, a limit on the extracted content.
- The bound on what the server holds while extracting, observable by the operator.

### 3.2 Out of scope, and why

- Parts never read (images, media, embedded objects, nested archives): they are not decoded, so they cost nothing; opening them is not asked for.
- A part that is damaged for another reason than its size: still read as absent, BL-0144.
- The converter service's output, read whole into memory: other source, BL-0142.
- PDF content streams inflated by the PDF reader: same class, other decoder, BL-0143.
- Archive extraction: bounded by SPEC-0084 and SPEC-0085.
- `.docm`, `.dotx` and legacy formats: decoded as plain text, the stored file itself, already within the project's storage.
- A configurable limit: fixed numbers are enough for this increment (Section 14.4 D-4).

### 3.3 Constraints imposed by the context

None.

## 4. Actors

- **Member:** a person with access to a project; asks for a document's text through the assistant tool or the HTTP API.
- **Operator:** runs the server, watches its memory and reads its log and audit trail.
- **Attacker:** a member, or someone who got a member to upload a file, who crafts an Office document to exhaust the server's memory.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Document text extraction | turning a stored document into Markdown text and table artifacts | part, declared size, decoded size, reading order, extracted content, refusal |
| Project storage | the files, the companion, the session write allowance, the audit trail | companion, session write allowance, audit line |

## 5. Usage scenarios

### SC-001: Extract the text of an Office document within the limits
**Actor:** Member.
**Preconditions:** project `atlas`, member `mia@example.com`; `/docs/memo.docx` whose main part holds the paragraph `Quarterly results`.
**Flow:**
1. The member asks for the text of `/docs/memo.docx` → the system reads the main part, which is within every limit, and builds the text.
2. The system writes `/docs/memo.md`, answers with its path and a preview containing `Quarterly results`, and records one audit line.
3. Parts the extraction does not read are ignored, whatever their size.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: one part is one byte over the part limit → SC-002.

### SC-002: A document decoding past a limit is refused
**Actor:** Member or attacker.
**Preconditions:** `/docs/bomb.docx`, stored in under 1 000 000 bytes, whose main part declares and holds 200 000 000 bytes of the letter `a`.
**Flow:**
1. The member asks for its text → the system reads the declared size of the main part, finds it over 104 857 600 bytes and refuses before decoding it.
**Postconditions:** no companion written or changed; no artifact; the session write allowance unchanged; no audit line; the decoded bytes held are 0.
**Exceptions:**
- EXC-002a: every part fits but the parts read together pass 268 435 456 bytes → refused on the total, before decoding the part that crosses it.
- EXC-002b: the parts fit but the content built from them passes 268 435 456 bytes (one long shared text in many cells) → refused on the content.

### SC-003: A part larger than it declares
**Actor:** Attacker.
**Preconditions:** `/docs/liar.docx` whose main part declares 10 bytes and holds 1 000 000 bytes.
**Flow:**
1. The member asks for its text → decoding stops one chunk past the declared size and the call is refused naming the part and its declared size.
**Postconditions:** as SC-002; the decoded bytes held never exceed 65 546.
**Exceptions:**
- EXC-003a: the lying part is an Excel sheet or a PowerPoint slide → refused, where today the extraction succeeds without it.

### SC-004: Order when several refusals apply
**Actor:** Member.
**Flow:**
1. Membership, a missing path and the header check of SPEC-0041 come first, as today.
2. Then the parts are read in their reading order; the first part that meets a limit or lies about its size decides the refusal.
3. The content limit is met while the content is built, so it is reported where it is crossed.

### SC-005: Repeat, refresh and the stored companion
**Actor:** Member, operator.
**Flow:**
1. A refused call repeated gives the same refusal and still writes nothing.
2. With an up to date companion and no refresh, the stored text is returned, no part being read.
3. With a refresh, the document is read again and refused if it passes a limit; the stored companion is left as is.
4. The operator finds one informational log line per refused call and no audit line.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-U]: Part limit
> THE document text extraction SHALL decode no part of a Word, Excel or PowerPoint document beyond 104 857 600 bytes.
- **Business rules:** applies to every part read (Section 2.1 reading order), for the seven suffixes in any letter case, through both doors. A part of exactly 104 857 600 decoded bytes is accepted.
- **Scenario:** SC-001, SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Refusal of a part declared over the limit
> IF a part about to be read declares more than 104 857 600 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: document part '<part>' is <D> bytes once decoded, over the limit of 104857600 bytes`, before decoding that part.
- **Business rules:** `<path>` is the document's project path as given, normalized; `<part>` the part's name inside the document (`word/document.xml`); `<D>` the declared size, exact, in plain digits, up to 18446744073709551615, never rounded or wrapped.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Refusal on the parts read together
> IF the declared sizes of the parts already read in this call plus the declared size of the part about to be read exceed 268 435 456 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the document parts read total more than 268435456 bytes once decoded`, before decoding that part.
- **Business rules:** a part read twice counts twice; parts not read count 0; equal is accepted; the sum is exact. FR-NEW-002 is checked first for the part about to be read.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: A part larger than it declares
> IF a part decodes to more bytes than it declares THEN THE document text extraction SHALL stop decoding it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: document part '<part>' decodes to more bytes than declared <D>`.
- **Business rules:** `<D>` the declared size. A part declared 0 bytes that holds data is such a part. A part holding exactly its declared size is accepted. Applies to every part read, whatever the format.
- **User visible text:** as above.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Content limit
> IF the extracted content built from a document passes 268 435 456 bytes THEN THE document text extraction SHALL stop building it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the extracted content exceeds 268435456 bytes`.
- **Business rules:** the extracted content is the Markdown text before the member's character cut plus every table cell kept for table artifacts, in bytes. Equal is accepted. Applies to the three formats.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-006 [EARS-U]: Bound on what extraction holds
> THE document text extraction SHALL hold, during one call, at most 104 857 600 plus 65 536 bytes of any one decoded part, at most 268 435 456 plus 65 536 bytes of decoded part data in total, and at most 373 293 056 bytes of extracted content.
- **Business rules:** 373 293 056 is the content limit plus the part limit, the largest single piece of content one cell or paragraph can add. A call refused under FR-NEW-002 or FR-NEW-003 before its first decoded byte holds 0 bytes of that part. The bound is per call: two calls at once may each hold it.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-007 [EARS-UB]: A refusal leaves no trace
> THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-002 to FR-NEW-005.
- **Business rules:** an existing companion, stale or up to date, is left byte for byte as it was. The refusal is logged once at the informational level, as every invalid argument is today.
- **Scenario:** SC-002, SC-005
- **Priority:** Must

#### FR-NEW-008 [EARS-UB]: Parts not read are not opened
> THE document text extraction SHALL NOT decode, count or open a part outside the reading order of Section 2.1, nor any archive nested inside a part.
- **Business rules:** a 200 000 000 byte image or a nested zip archive inside a document within the limits leaves the extraction successful and unchanged.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-009 [EARS-O]: Order of refusals
> IF several refusals apply to one text extraction of a Word, Excel or PowerPoint document THEN THE document text extraction SHALL report the first of this list: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the file is not a zip or fails the SPEC-0041 header check (`could not extract <path>: not a valid OOXML package: <reason>`); (4) parts in reading order, and for each part FR-NEW-002, then FR-NEW-003, then FR-NEW-004 while it decodes; (5) FR-NEW-005 at the moment the content crosses its limit, which can fall between two parts.
- **Business rules:** item 5 can precede a later part's item 4 refusal: the content built from the earlier parts is checked as it grows. Messages of items 1 to 3 unchanged.
- **Scenario:** SC-004
- **Priority:** Must

#### FR-NEW-010 [EARS-U]: Both doors answer alike
> THE document text extraction SHALL refuse or answer a call through the HTTP API with the same code and message, and the same effects, as the same call through the assistant tool.
- **Scenario:** SC-002, SC-003
- **Priority:** Must

#### FR-NEW-011 [EARS-U]: The rest is unchanged
> THE document text extraction SHALL answer a Word, Excel or PowerPoint document within every limit with the same text, metadata, tables, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading any part when no refresh is asked, and SHALL extract every other format exactly as before.
- **Business rules:** includes today's refusals for a missing Word main part (`could not extract <path>: docx: word/document.xml is missing`) and a file that is not a zip; a part damaged for another reason than its size keeps reading as absent (BL-0144).
- **Scenario:** SC-001, SC-005
- **Priority:** Must

### 6.2 Modified

None. No spec defines the behavior FR-NEW-004 replaces (a part larger than it declares read as absent, Section 2.1); FR-NEW-005 bounds the SPEC-0019 grid without changing its definition (2.2).

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN a call is refused under FR-NEW-002 or FR-NEW-003 on the first part read THE document text extraction SHALL have held 0 bytes of decoded part data.

### 7.2 Security and access
- THE document text extraction SHALL require the same project membership as before.
- THE document text extraction SHALL NOT let a document's declared sizes, actual sizes or cell references make the server hold more than FR-NEW-006 states.

### 7.5 Reliability
- IF a call is refused THEN THE document text extraction SHALL leave the project's files, companions, artifacts, storage and session write allowance unchanged.

### 7.6 Operability
- WHEN a call is refused under FR-NEW-002 to FR-NEW-005 THE document text extraction SHALL leave exactly one informational log line naming `ERR_INVALID_ARGUMENT` and the message, and no audit line.
- THE document text extraction SHALL make the decoded bytes held and the content held of one call measurable by the verification of this specification (the measurement hook of Section 9).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | part declared 0 holding data; Excel with no sheet; Word without main part | size lie refusal; empty text as today; missing message as today | E2E-015; existing `xlsx_without_sheets_yields_empty_text`; E2E-028 |
| Boundary values | part at 104 857 600 and one over; parts read at 268 435 456 and one over; content just under and over; part exactly its declared size | accepted; refused; accepted; refused; accepted; refused; accepted | E2E-006, E2E-007, E2E-008, E2E-009, E2E-020, E2E-021, E2E-016 |
| Concurrent actions | two members extracting bombs while a third extracts a normal document | both refused, each within the bound; third succeeds | E2E-036 |
| Slow or lost connectivity | not applicable: the document is already stored, no network | none | none |
| Access states | not a member | membership refusal first | E2E-025 |
| Data variations | upper case suffix; the four other suffixes; declared 2^63 and the largest declarable size; accented part content | refused alike; exact digits; unchanged decoding | E2E-033, E2E-034, E2E-SEC-012, E2E-SEC-013, E2E-002 |
| Navigation | refresh asked; up to date companion without refresh | refused; stored text returned | E2E-031, E2E-005 |
| Partial operations | slide 1 lies, slide 2 is a bomb; a refused call repeated | first part decides; same refusal, nothing written | E2E-024, E2E-032 |
| Dependency failures | not applicable: no external service in this path | none | none |
| Ordering and timing | header inconsistency versus bomb; shared strings versus sheet; missing path | FR-NEW-009 | E2E-022, E2E-023, E2E-026 |
| Exhaustion | honest bombs in every part read; many slides; a sheet listed three times; shared text multiplied; unread huge parts and nested archives | refused, within the bound; unread parts ignored | E2E-SEC-001 to E2E-SEC-011, E2E-010, E2E-011, E2E-017 to E2E-019 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files are built by the design. Unless a test says otherwise, the project is `atlas`, the member `mia@example.com`, the call goes through the assistant tool with default options (`max_chars` 200 000, no refresh), the session write allowance is the default 52 428 800 bytes with nothing written yet, and the project has no storage limit. "Decoded bytes held" is the largest amount of decoded part data the server holds at once during the call, and "content held" the largest extracted content it holds, both read by the operator's measurement hook. A "bomb part" declares and holds exactly 200 000 000 bytes, the letter `a` filling what the part's markup leaves, and its document is stored in under 1 000 000 bytes; every other part of a bomb document is under 2 000 bytes. "Nothing left" means: no file `/docs/<name>.md` appears in the listing of `/docs` (or an existing one keeps its bytes), no table or image artifact is listed for the document, the audit trail has no extraction line for it, and a following write of a 52 428 800 byte file `/docs/fill.bin` by the same member succeeds (nothing was charged).

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-NEW-011 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-011 | High |
| E2E-003 | New | happy | SC-001 | FR-NEW-011 | High |
| E2E-004 | New | happy | SC-001 | FR-NEW-010, FR-NEW-011 | High |
| E2E-005 | New | edge | SC-005 | FR-NEW-008, FR-NEW-011 | High |
| E2E-006 | New | edge | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-006 | Critical |
| E2E-007 | New | failure | SC-002 | FR-NEW-001, FR-NEW-002, FR-NEW-007 | Critical |
| E2E-008 | New | edge | SC-001 | FR-NEW-003 | High |
| E2E-009 | New | failure | SC-002 | FR-NEW-003, FR-NEW-006 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-003, FR-NEW-006 | High |
| E2E-011 | New | failure | SC-002 | FR-NEW-003 | High |
| E2E-012 | New | happy | SC-001 | FR-NEW-003, FR-NEW-011 | Medium |
| E2E-013 | New | failure | SC-003 | FR-NEW-004, FR-NEW-006, FR-NEW-007 | High |
| E2E-014 | New | failure | SC-003 | FR-NEW-004, FR-NEW-010 | High |
| E2E-015 | New | failure | SC-003 | FR-NEW-004 | Medium |
| E2E-016 | New | edge | SC-003 | FR-NEW-004 | Medium |
| E2E-017 | New | edge | SC-001 | FR-NEW-008 | High |
| E2E-018 | New | edge | SC-001 | FR-NEW-008 | High |
| E2E-019 | New | edge | SC-001 | FR-NEW-008 | Medium |
| E2E-020 | New | edge | SC-001 | FR-NEW-005, FR-NEW-011 | High |
| E2E-021 | New | failure | SC-002 | FR-NEW-005, FR-NEW-006 | High |
| E2E-022 | New | failure | SC-004 | FR-NEW-009 | High |
| E2E-023 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-024 | New | failure | SC-004 | FR-NEW-004, FR-NEW-009 | Medium |
| E2E-025 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-026 | New | failure | SC-004 | FR-NEW-009 | Medium |
| E2E-027 | New | failure | SC-001 | FR-NEW-011 | Medium |
| E2E-028 | New | failure | SC-001 | FR-NEW-011 | Medium |
| E2E-029 | New | failure | SC-002 | FR-NEW-002, FR-NEW-010 | Critical |
| E2E-030 | New | side effect | SC-005 | FR-NEW-007 | High |
| E2E-031 | New | failure | SC-005 | FR-NEW-007, FR-NEW-011 | High |
| E2E-032 | New | failure | SC-005 | FR-NEW-007 | Medium |
| E2E-033 | New | failure | SC-002 | FR-NEW-002 | Medium |
| E2E-034 | New | failure | SC-002 | FR-NEW-002 | Medium |
| E2E-035 | New | side effect | SC-005 | FR-NEW-007 | Medium |
| E2E-036 | New | edge | SC-002 | FR-NEW-006, FR-NEW-011 | Medium |
| E2E-037 | New | edge | SC-001 | FR-NEW-011 | Low |
| E2E-SEC-001 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006, FR-NEW-007 | Critical |
| E2E-SEC-002 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-003 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-004 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-005 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-006 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-007 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-008 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-009 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-010 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-011 | New | failure, security | SC-002 | FR-NEW-005, FR-NEW-006 | Critical |
| E2E-SEC-012 | New | failure, security | SC-002 | FR-NEW-002 | High |
| E2E-SEC-013 | New | failure, security | SC-002 | FR-NEW-002 | High |

Coverage (50 tests: 50 new, 0 modified, 0 removed): happy 5, failure 33 (13 of them also security), side effect 2, edge 10, state transition 0 (no state machine: a document is extracted or refused, nothing persists between calls). Happy to failure ratio 1:6.6.

### 9.2 New tests

#### E2E-001: Word document within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-011; **Priority:** High
- **Given** `/docs/memo.docx` whose main part holds a heading `Results` (style Heading 1) and the paragraph `Quarterly results`
- **When** the member asks for its text
- **Then** the answer names `/docs/memo.md`, format `docx`, not cached, and its preview reads `# Results` then `Quarterly results`
- **And** `/docs/memo.md` holds that text; the audit trail has one extraction line for `/docs/memo.md`

#### E2E-002: Excel workbook within the limits, accented shared text
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/docs/budget.xlsx` with one sheet `Q1` whose cells A1 `Poste`, B1 `Montant`, A2 shared text `Café`, B2 `12`
- **When** the member asks for its text
- **Then** the preview holds `## Sheet: Q1` and a table whose header is `Poste | Montant` and whose row is `Café | 12`
- **And** one table artifact captioned `Q1` is listed for `/docs/budget.xlsx`

#### E2E-003: PowerPoint deck within the limits, with notes
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** High
- **Given** `/docs/deck.pptx` with slide 1 `Welcome` (notes `Say hello`) and slide 2 `Plan`
- **When** the member asks for its text
- **Then** the preview reads `## Slide 1`, `Welcome`, `> Notes: Say hello`, `## Slide 2`, `Plan`, in that order

#### E2E-004: HTTP API within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-010, FR-NEW-011; **Priority:** High
- **Given** the document of E2E-001
- **When** a client asks the HTTP API for its text
- **Then** the answer carries the same companion path, format and preview as E2E-001
- **And** the audit trail has one extraction line for `/docs/memo.md`

#### E2E-005: Up to date companion answers without reading a bomb
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-011; **Priority:** High
- **Given** `/docs/bomb.docx`, a bomb part document, and `/docs/bomb.md` holding `kept text`, written after it
- **When** the member asks for the text of `/docs/bomb.docx` without refresh
- **Then** the answer is cached, names `/docs/bomb.md` and previews `kept text`
- **And** the decoded bytes held are 0

#### E2E-006: Part of exactly the part limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-006; **Priority:** Critical
- **Given** `/docs/large.docx` whose main part declares and holds exactly 104 857 600 bytes, one paragraph of the letter `b`
- **When** the member asks for its text
- **Then** the extraction succeeds; the answer reports the text as truncated, its preview made of the letter `b`
- **And** the decoded bytes held never exceed 104 923 136

#### E2E-007: Part one byte over the part limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/large.docx` as in E2E-006 but its main part is 104 857 601 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/large.docx: document part 'word/document.xml' is 104857601 bytes once decoded, over the limit of 104857600 bytes`
- **And** the decoded bytes held are 0; nothing left

#### E2E-008: Parts read totalling exactly the call limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** session allowance 1 000 000 000 and `/docs/wide.xlsx` whose shared strings, workbook, workbook relationships and three sheets, each sheet under 100 000 000 bytes, declare 268 435 456 bytes together, every sheet holding numbers only in rows of 10 cells
- **When** the member asks for its text
- **Then** the extraction succeeds with 3 sheets in its metadata

#### E2E-009: Parts read one byte over the call limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/wide.xlsx` as in E2E-008 with its third sheet one byte longer
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/wide.xlsx: the document parts read total more than 268435456 bytes once decoded`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-010: Many slides together over the call limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/many.pptx` with 30 slides, each declaring and holding 10 000 000 bytes of the letter `c`
- **When** the member asks for its text
- **Then** the refusal is the call limit message of E2E-009 naming `/docs/many.pptx`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-011: One sheet listed three times
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/docs/repeat.xlsx` whose workbook lists sheets `A`, `B`, `C`, all three pointing to the same sheet part of 100 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/repeat.xlsx: the document parts read total more than 268435456 bytes once decoded`; nothing left

#### E2E-012: Many small slides within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-003, FR-NEW-011; **Priority:** Medium
- **Given** `/docs/talk.pptx` with 500 slides, slide N holding the text `Slide text N`
- **When** the member asks for its text
- **Then** the extraction succeeds with 500 slides in its metadata and the preview starts `## Slide 1` then `Slide text 1`

#### E2E-013: Word main part larger than it declares
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-006, FR-NEW-007; **Priority:** High
- **Given** `/docs/liar.docx` whose main part declares 10 bytes and holds 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/liar.docx: document part 'word/document.xml' decodes to more bytes than declared 10`, not `docx: word/document.xml is missing`
- **And** the decoded bytes held never exceed 65 546; nothing left

#### E2E-014: Excel sheet larger than it declares, through the HTTP API
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004, FR-NEW-010; **Priority:** High
- **Given** `/docs/liar.xlsx` with sheets `Good` (cell `ok`) and `Bad`, whose part `xl/worksheets/sheet2.xml` declares 100 bytes and holds 500 000 bytes
- **When** a client asks the HTTP API for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/liar.xlsx: document part 'xl/worksheets/sheet2.xml' decodes to more bytes than declared 100` (today: success with sheet `Good` only)
- **And** nothing left

#### E2E-015: Slide declared 0 holding data
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/zero.pptx` whose part `ppt/slides/slide1.xml` declares 0 bytes and holds a slide with the text `hello`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/zero.pptx: document part 'ppt/slides/slide1.xml' decodes to more bytes than declared 0`

#### E2E-016: Part holding exactly its declared size
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/exact.docx` whose main part declares and holds 5 000 bytes, a paragraph `exact` padded with spaces
- **When** the member asks for its text
- **Then** the extraction succeeds and the preview reads `exact`

#### E2E-017: Huge embedded image is not read
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/docs/photo.docx` holding the paragraph `See photo` and `word/media/image1.png`, declaring and holding 200 000 000 bytes
- **When** the member asks for its text
- **Then** the extraction succeeds and the preview reads `See photo`
- **And** the decoded bytes held never exceed 65 536

#### E2E-018: Nested archive is not opened
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/docs/nested.docx` holding the paragraph `Outer` and `word/embeddings/inner.zip`, a zip archive whose entry declares and holds 200 000 000 bytes
- **When** the member asks for its text
- **Then** the extraction succeeds, the preview reads `Outer`, and no file appears under `/docs/nested`
- **And** the decoded bytes held never exceed 65 536

#### E2E-019: Huge unread part in a workbook
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/docs/media.xlsx` with sheet `S` (cell `fine`) and `xl/media/video.bin`, declaring and holding 200 000 000 bytes
- **When** the member asks for its text
- **Then** the extraction succeeds and the preview holds `fine`

#### E2E-020: Content just under the content limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-005, FR-NEW-011; **Priority:** High
- **Given** session allowance 1 000 000 000 and `/docs/long.xlsx` whose shared strings hold one text of 1 000 000 letters `d` and whose sheet `S` has 100 rows, each a single cell referring to that text
- **When** the member asks for its text
- **Then** the extraction succeeds; one table artifact of 100 rows is listed for the document
- **And** the content held never exceeds 373 293 056

#### E2E-021: Content over the content limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006; **Priority:** High
- **Given** `/docs/long.xlsx` as in E2E-020 with 140 rows
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/long.xlsx: the extracted content exceeds 268435456 bytes`
- **And** the content held never exceeds 373 293 056; nothing left

#### E2E-022: Header inconsistency wins over a bomb
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/docs/twice.docx`, a bomb part document whose main part's own header names it `word/documenX.xml`
- **When** the member asks for its text
- **Then** the refusal starts `could not extract /docs/twice.docx: not a valid OOXML package: header inconsistency detected`
- **And** the decoded bytes held are 0

#### E2E-023: Shared strings named before a sheet
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/two.xlsx` whose shared strings part and only sheet are both bomb parts
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/two.xlsx: document part 'xl/sharedStrings.xml' is 200000000 bytes once decoded, over the limit of 104857600 bytes`

#### E2E-024: First slide lies, second is a bomb
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-004, FR-NEW-009; **Priority:** Medium
- **Given** `/docs/order.pptx` whose slide 1 declares 10 bytes and holds 1 000 bytes and whose slide 2 is a bomb part
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/order.pptx: document part 'ppt/slides/slide1.xml' decodes to more bytes than declared 10`

#### E2E-025: Not a member
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/bomb.docx`, a bomb part document, and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its text
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`; the decoded bytes held are 0

#### E2E-026: Missing path
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** nothing at `/docs/none.docx`
- **When** the member asks for its text
- **Then** the refusal is `ERR_NOT_FOUND` with `not a file: /docs/none.docx`

#### E2E-027: Not a zip, unchanged
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/docs/fake.docx` holding the text `plain words`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `could not extract /docs/fake.docx: not a valid OOXML package:`

#### E2E-028: Word document without its main part, unchanged
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Medium
- **Given** `/docs/empty.docx` holding only `word/styles.xml`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/empty.docx: docx: word/document.xml is missing`

#### E2E-029: The HTTP API refuses a bomb alike
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-010; **Priority:** Critical
- **Given** `/docs/bomb.docx`, a bomb part document
- **When** a client asks the HTTP API for its text
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and the message of E2E-SEC-001
- **And** the decoded bytes held are 0; nothing left

#### E2E-030: A refusal leaves a stale companion and the allowance untouched
- **Category:** side effect; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/docs/report.md` holding `old text`, then `/docs/report.docx` written after it as a bomb part document
- **When** the member asks for the text of `/docs/report.docx`
- **Then** the refusal is the part limit message for `/docs/report.docx`
- **And** `/docs/report.md` still reads `old text`; no artifact is listed for `/docs/report.docx`; a following write of a 52 428 800 byte file succeeds

#### E2E-031: Refresh on a bomb with an up to date companion
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007, FR-NEW-011; **Priority:** High
- **Given** the situation of E2E-005
- **When** the member asks for the text of `/docs/bomb.docx` with refresh
- **Then** the refusal is the part limit message for `/docs/bomb.docx`
- **And** `/docs/bomb.md` still reads `kept text`

#### E2E-032: Repeating a refused call
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/bomb.docx`, a bomb part document
- **When** the member asks for its text twice
- **Then** both answers are the same refusal; nothing left after the second

#### E2E-033: Upper case suffix
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/docs/BOMB.DOCX`, a bomb part document
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `/docs/BOMB.DOCX` and `word/document.xml`

#### E2E-034: The four other suffixes
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/docs/b.xlsm` with a bomb sheet, `/docs/b.pptm`, `/docs/b.potx` and `/docs/b.ppsx` each with a bomb slide 1
- **When** the member asks for the text of each
- **Then** each is refused with the part limit message naming its path and `xl/worksheets/sheet1.xml` or `ppt/slides/slide1.xml`

#### E2E-035: What the operator sees for a refusal
- **Category:** side effect; **Scenario:** SC-005; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/bomb.docx`, a bomb part document
- **When** the member asks for its text
- **Then** the server log has exactly one informational line for the call naming `ERR_INVALID_ARGUMENT` and `document part 'word/document.xml'`, and no error level line
- **And** the audit trail has no line for `/docs/bomb.docx` or `/docs/bomb.md`

#### E2E-036: Two bombs and a normal document at once
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006, FR-NEW-011; **Priority:** Medium
- **Given** members `mia@example.com` and `noa@example.com`, each with a bomb part document `/docs/m.docx` and `/docs/n.docx`, and `ola@example.com` with `/docs/memo.docx` of E2E-001
- **When** the three ask for their texts at the same time
- **Then** the two bombs are refused with the part limit message and the memo succeeds as in E2E-001
- **And** each bomb call's decoded bytes held are 0

#### E2E-037: A `.docm` file is not read as a Word document
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-011; **Priority:** Low
- **Given** `/docs/macro.docm`, a zip whose main part is a bomb part
- **When** the member asks for its text
- **Then** the extraction succeeds as today, with format `text` and the note `unknown extension .docm; decoded as text`
- **And** the decoded bytes held are 0 (no part is decoded, the stored bytes are)

### 9.3 Exploit tests

Each covers one call site of the reader that decodes a part whole (Section 14.2 occurrences 1 to 10), plus the cell multiplication (occurrence 11) and the uncountable declared size (occurrences 12 and 13). Payload for E2E-SEC-001 to 010: the named part is a bomb part, exactly 200 000 000 bytes declared and held, the letter `a` filling what its markup leaves (as text where the part has text, as an attribute value of the root element otherwise), the document being stored in under 1 000 000 bytes, every other part well formed and under 2 000 bytes. "Nothing left" as defined in the header of this section.

#### E2E-SEC-001: Word main part bomb
- **Covers:** class sweep occurrence 1 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today the whole part, 200 000 000 bytes, is decoded before anything is checked.
- **Given** `/docs/bomb.docx` whose `word/document.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/bomb.docx: document part 'word/document.xml' is <D> bytes once decoded, over the limit of 104857600 bytes` with `<D>` = `200000000`
- **And** the decoded bytes held are 0; nothing left

#### E2E-SEC-002: PowerPoint slide bomb
- **Covers:** occurrence 2
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today slide 2, 200 000 000 bytes, is decoded whole.
- **Given** `/docs/bomb.pptx` with slide 1 `Intro` and slide 2 `ppt/slides/slide2.xml` the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `/docs/bomb.pptx` and `ppt/slides/slide2.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-003: PowerPoint notes bomb
- **Covers:** occurrence 3
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the notes part is decoded whole.
- **Given** `/docs/notes.pptx` with slide 1 `Intro` whose notes part `ppt/notesSlides/notesSlide1.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/notesSlides/notesSlide1.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-004: Presentation part bomb
- **Covers:** occurrence 4
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today the presentation part is decoded whole to find the slide order.
- **Given** `/docs/pres.pptx` whose `ppt/presentation.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/presentation.xml`
- **And** the decoded bytes held are 0; nothing left

#### E2E-SEC-005: Presentation relationships bomb
- **Covers:** occurrence 5
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the relationships part is decoded whole.
- **Given** `/docs/prels.pptx` whose `ppt/_rels/presentation.xml.rels` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/_rels/presentation.xml.rels`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-006: Slide relationships bomb
- **Covers:** occurrence 6
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the slide's relationships part is decoded whole while looking for its notes.
- **Given** `/docs/srels.pptx` with slide 1 `Intro` whose `ppt/slides/_rels/slide1.xml.rels` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `ppt/slides/_rels/slide1.xml.rels`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-007: Excel sheet bomb
- **Covers:** occurrence 7
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the sheet is decoded whole.
- **Given** `/docs/sheet.xlsx` whose only sheet `xl/worksheets/sheet1.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/worksheets/sheet1.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-008: Shared strings bomb
- **Covers:** occurrence 8
- **Red before the fix because:** `And the decoded bytes held are 0` fails: today the shared strings part is decoded whole, first of all parts.
- **Given** `/docs/shared.xlsx` whose `xl/sharedStrings.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/sharedStrings.xml`
- **And** the decoded bytes held are 0; nothing left

#### E2E-SEC-009: Workbook part bomb
- **Covers:** occurrence 9
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the workbook part is decoded whole.
- **Given** `/docs/book.xlsx` whose `xl/workbook.xml` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/workbook.xml`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-010: Workbook relationships bomb
- **Covers:** occurrence 10
- **Red before the fix because:** `And the decoded bytes held never exceed 65 536` fails: today the relationships part is decoded whole.
- **Given** `/docs/brels.xlsx` whose `xl/_rels/workbook.xml.rels` is the payload
- **When** the member asks for its text
- **Then** the refusal is the part limit message naming `xl/_rels/workbook.xml.rels`
- **And** the decoded bytes held never exceed 65 536; nothing left

#### E2E-SEC-011: One shared text multiplied by 400 cells
- **Covers:** occurrence 11
- **Red before the fix because:** `And the content held never exceeds 373 293 056` fails: today 400 copies of the text, 400 000 000 bytes, go into the grid and as much into the Markdown table.
- **Given** `/docs/mult.xlsx`, stored in under 100 000 bytes, whose shared strings hold one text of 1 000 000 letters `a` and whose sheet `S` has 400 rows, each a single cell referring to that text
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/mult.xlsx: the extracted content exceeds 268435456 bytes`
- **And** the content held never exceeds 373 293 056; nothing left

#### E2E-SEC-012: Part declaring 2^63 bytes
- **Covers:** occurrence 12
- **Red before the fix because:** `Then the refusal is ...` fails: today the part reads its 5 real bytes and the extraction succeeds.
- **Given** `/docs/huge.docx` whose `word/document.xml` declares 9 223 372 036 854 775 808 bytes in its large size record and holds a 5 byte content, its checksum matching
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/huge.docx: document part 'word/document.xml' is 9223372036854775808 bytes once decoded, over the limit of 104857600 bytes`
- **And** the decoded bytes held are 0; nothing left

#### E2E-SEC-013: Part declaring the largest size
- **Covers:** occurrence 13
- **Red before the fix because:** `Then the refusal is ...` fails: today the extraction succeeds on the 5 real bytes.
- **Given** `/docs/max.xlsx` whose `xl/sharedStrings.xml` declares 18 446 744 073 709 551 615 bytes and holds 5 bytes, checksum matching
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/max.xlsx: document part 'xl/sharedStrings.xml' is 18446744073709551615 bytes once decoded, over the limit of 104857600 bytes`, the digits exact
- **And** nothing left

### 9.4 Modified tests

None. The existing extraction tests (Section 14.1 row 12) use parts of a few hundred bytes, none larger than it declares; they stay unchanged and green.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-003, FR-NEW-005, FR-NEW-008, FR-NEW-010, FR-NEW-011 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-012 | E2E-027, E2E-028 | E2E-006, E2E-008, E2E-017, E2E-018, E2E-019, E2E-020, E2E-037 | E2E-001 (companion and audit line) |
| SC-002 | FR-NEW-001 to FR-NEW-003, FR-NEW-005 to FR-NEW-007, FR-NEW-010 | E2E-006 (success branch, SC-001) | E2E-007, E2E-009, E2E-010, E2E-011, E2E-021, E2E-029, E2E-033, E2E-034, E2E-SEC-001 to E2E-SEC-013 | E2E-036 | E2E-SEC-001 (nothing left) |
| SC-003 | FR-NEW-004, FR-NEW-006, FR-NEW-007, FR-NEW-010 | E2E-016 (success branch, categorized edge) | E2E-013, E2E-014, E2E-015 | E2E-016 | E2E-013 (nothing left) |
| SC-004 | FR-NEW-004, FR-NEW-009 | E2E-001 (success branch, SC-001) | E2E-022, E2E-023, E2E-024, E2E-025, E2E-026 | E2E-022 (held 0 before the header check passes) | E2E-025 (held 0) |
| SC-005 | FR-NEW-007, FR-NEW-008, FR-NEW-011 | E2E-005 (cached success, categorized edge) | E2E-031, E2E-032 | E2E-005 | E2E-030, E2E-035 |

SC-002, SC-003 and SC-004 are refusal scenarios; their success branch is named.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-001, E2E-006, E2E-007 | 3 |
| FR-NEW-002 | E2E-006, E2E-007, E2E-029, E2E-033, E2E-034, E2E-SEC-001 to E2E-SEC-010, E2E-SEC-012, E2E-SEC-013 | 17 |
| FR-NEW-003 | E2E-008, E2E-009, E2E-010, E2E-011, E2E-012 | 5 |
| FR-NEW-004 | E2E-013, E2E-014, E2E-015, E2E-016, E2E-024 | 5 |
| FR-NEW-005 | E2E-020, E2E-021, E2E-SEC-011 | 3 |
| FR-NEW-006 | E2E-006, E2E-009, E2E-010, E2E-013, E2E-021, E2E-036, E2E-SEC-001 to E2E-SEC-011 | 17 |
| FR-NEW-007 | E2E-007, E2E-013, E2E-030, E2E-031, E2E-032, E2E-035, E2E-SEC-001 | 7 |
| FR-NEW-008 | E2E-005, E2E-017, E2E-018, E2E-019 | 4 |
| FR-NEW-009 | E2E-022, E2E-023, E2E-024, E2E-025, E2E-026 | 5 |
| FR-NEW-010 | E2E-004, E2E-014, E2E-029 | 3 |
| FR-NEW-011 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-012, E2E-020, E2E-027, E2E-028, E2E-031, E2E-036, E2E-037 | 12 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| Office document | a Word (`.docx`), Excel (`.xlsx`, `.xlsm`) or PowerPoint (`.pptx`, `.pptm`, `.potx`, `.ppsx`) file, a zip package of parts | document text extraction |
| Part | one named file inside an Office document, such as `word/document.xml` | document text extraction |
| Declared size | the size a part's headers announce for its decoded content, readable before decoding | document text extraction |
| Decoded size | the bytes a part actually yields once decompressed | document text extraction |
| Reading order | the order in which the extraction reads the parts of each format, Section 2.1 | document text extraction |
| Part limit | 104 857 600 bytes, the most one part may decode to | document text extraction |
| Call limit | 268 435 456 bytes, the most the parts read in one call may declare together | document text extraction |
| Content limit | 268 435 456 bytes, the most the extracted content may reach | document text extraction |
| Extracted content | the Markdown text before the member's character cut plus every table cell kept for table artifacts | document text extraction |
| Shared text | a text an Excel workbook stores once and cells refer to by number | document text extraction |
| Size lie | a part decoding to more bytes than it declares | document text extraction |
| Bomb part | a part declaring and holding exactly 200 000 000 bytes, mostly one repeated letter, in a document stored in under 1 000 000 bytes | document text extraction |
| Chunk | the fixed 65 536 bytes decoding reads at most past a limit before it stops | document text extraction |
| Decoded bytes held | the largest amount of decoded part data the server holds at once during one call | document text extraction |
| Content held | the largest extracted content the server holds during one call | document text extraction |
| Measurement hook | the operator facing reading of decoded bytes held and content held for one call | document text extraction |
| Nested archive | an archive stored as a part of an Office document | document text extraction |
| Header check | the SPEC-0041 refusal of a document whose parts disagree with its directory | document text extraction |
| Companion | the Markdown file written next to the document, `/docs/memo.md` for `/docs/memo.docx` | project storage |
| Refresh | the member's choice to read the document again even when its companion is up to date | document text extraction |
| Table artifact | a table kept from a conversion, listed and read on its own | project storage |
| Session write allowance | the bytes a member can still write in the current session; 52 428 800 bytes by default | project storage |
| Audit line | the one line an operator reads to see who extracted what | project storage |
| Refusal | an error answer; the project is left unchanged | document text extraction |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server, watches its memory and reads its log and audit trail | document text extraction |
| Attacker | a person crafting a document to exhaust the server's memory | document text extraction |
| Assistant tool | the tool an AI assistant calls to extract a document's text | document text extraction |
| HTTP API | the web endpoint a program calls to extract a document's text; the second door | document text extraction |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 9 | IMPLEMENTABLE |

**Amendments applied after round 1 (all A, amended in place):** A-1 citations recounted (14.1 rows 1, 9; 14.3 `BuiltinTable` lines; `server.rs:2396-2422`, `fs_ops.rs:1720-1796`, `extract.rs:157-178`, `:337-362`); A-2 the bomb part fixed at exactly 200 000 000 bytes, markup included, so E2E-023 and E2E-SEC-001 spell their message literally (the draft said "200 000 000 plus its markup" and contradicted its own message); A-3 tool names removed from Sections 2.1, 3.2 and the front block (`fs.documentize`, `fs.extract_archive`, `fs.extract_text`); A-4 an untestable performance line removed from 7.1; A-5 the 7.6 measurement line restated: the hook is test only today (`archive_single.rs:55`), recorded in 14.3; A-6 sweep occurrences 14 and 18 given the controls they rely on (`quota.rs:47`, `fs_ops.rs:578`, `:613`, `archive.rs:24`); A-7 E2E-006 Then made observable (truncated flag and preview, not a character count); A-8 the zip64 short part acceptance kept ASSUMED with its signal (14.3); A-9 the "could" hits of the modal scan are the verbatim prefix `could not extract`, a user visible message, not a modal.

**How the gates ran:** this harness exposes no subagent tool, so the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks for ids (50 summarized, 50 specified), categories (happy 5, failure 33, security 13, edge 10, side effect 2), requirement coverage (every FR at least 3), modals and implementation terms, and the five sweep searches re run. Recorded as a concern in the handoff.

**No implementation detail outside Section 14:** PASS. Suffixes, part names (they appear in the user visible messages), error codes, verbatim messages and the option name `max_chars` are WHAT, as in SPEC-0041 and SPEC-0085.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | two doors, suffix routing, lowercased, other suffixes as text | `crates/core/src/mcp/server.rs:2396-2422` (`fs.extract_text` calls `fs_ops::extract_document`); `crates/core/src/api/dataplane.rs:1484-1505` (same function); `crates/core/src/core/fs_ops.rs:1720-1796`; `crates/core/src/docs/extract.rs:157-178` (routing), `:409-414` (`extension_of` lowercases), `:176-177` (unknown suffix note) |
| 2 | header check, not a zip | `crates/core/src/docs/extract.rs:550-560` (`open_zip`), `crates/core/src/docs/zip_check.rs:19-26`; SPEC-0041 FR-NEW-002 |
| 3 | reading order per format | Word `crates/core/src/docs/extract.rs:716`; Excel `:986-992` (`xlsx_shared_strings` then `xlsx_sheets` then each sheet), `:1046-1048`; PowerPoint `:868-881`, `:896-897`, `:924-930` (number order fallback), `:936-942` |
| 4 | each part decoded whole, no bound, no total, repeated reads | `crates/core/src/docs/extract.rs:563-568` (`entry_text`: `by_name` then `read_to_end` into an unbounded `Vec`, then `decode` copies it, `:418-420`); zip stops only past the declared size: `zip-9.0.5/src/crc32.rs:43-51`, `zip-9.0.5/src/read/readers.rs:318`, `:335` |
| 5 | shared text copied per cell; grid uncapped; Markdown cut at 400 rows; character cut after | `crates/core/src/docs/extract.rs:1112-1118` (`shared.get(index).cloned()`), `:1071-1110`, `:997-999` (grid kept uncapped), `:49`, `:462-466` (400 rows), `:181` (`truncate_chars` after extraction) |
| 6 | size lie or damaged part read as absent | `crates/core/src/docs/extract.rs:564-566` (`.ok()?` twice), `:716-717` (missing message), `:871`, `:992` (`continue`) |
| 7 | images, media, nested archives never read | `crates/core/src/docs/extract.rs` reads parts only by the names of row 3; no `media/` or `embeddings/` name appears (`rg -n 'media/|embeddings' crates/core/src/docs/extract.rs` finds none) |
| 8 | error wrapping, no trace on refusal, log level, success charge and audit | `crates/core/src/docs/extract.rs:366-374`; `crates/core/src/core/fs_ops.rs:1739-1748` (error returned before `land_conversion`), `:1771-1788` (charge, write, audit on success); `crates/core/src/errors.rs:89-90`; `.agent_docs/architecture.md:349-357` (4xx logged once at INFO) |
| 9 | cached companion, header check only | `crates/core/src/docs/extract.rs:337-362` |
| 10 | converter and search never use these readers | `crates/core/src/core/fs_ops.rs:696-723`, `:649`, `:668` (`write_companion` to the service); `crates/core/src/search/indexer.rs:170-175` (`read_utf8`: `String::from_utf8`, a zip is not UTF-8) |
| 11 | governing specs | `sdlc/specs/SPEC-0041-refuse-inconsistent-zips/sdlc-spec.md:95-98`; `sdlc/specs/SPEC-0019-document-artifacts/sdlc-design.md:327` (DEC-009); `sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/sdlc-spec.md:63`, `:752`, `:789` |
| 12 | tests: 51 in `docs/extract.rs` of which 14 Office reader tests (`rg -n 'fn (docx\|pptx\|xlsx\|non_zip)\w*\(\)' crates/core/src/docs/extract.rs`), 16 in `tools/document.rs`, SPEC-0041 extraction tests in `core/fs_ops.rs:4895-5110`; no limit, held bytes, size lie or repeated sheet test (`rg -n 'declared\|zip64\|bomb' crates/core/src/docs/extract.rs crates/core/src/core/fs_ops.rs crates/core/src/tools/document.rs` finds none) | as cited |

Test command: `cargo test -p mcp-fs-core --lib docs::extract` and `cargo test -p mcp-fs-core --lib tools::document`; SPEC-0041 extraction tests `cargo test -p mcp-fs-core --lib core::fs_ops`; full gate `cargo test --workspace`.

### 14.2 Class sweep

**Class:** content of a compressed container uploaded by a member reaching an in memory decode or an in memory expansion with no bound on the decoded size (CWE-409, CWE-400).

**Searches run** (whole `crates/core/src`, tests included then filtered by their `#[cfg(test)]` module start):
1. `rg -n 'by_name\(|by_index\(|ZipArchive::new' crates/core/src`: idiom "a zip entry opened"; 40 hits. Outside `tools/archive*.rs` (SPEC-0084, SPEC-0085): `docs/extract.rs:551`, `:564` (production); `docs/zip_check.rs:36` (production, headers only); every other hit is test code (`api/dataplane.rs:2604-3011` after `#[cfg(test)]` at `:1603`; `docs/docx.rs:379-424` after `:373`; `docs/zip_check.rs:203-209` after `:183`; `exports.rs:243-246` after `:153`; `tools/export.rs:280-283`, `tools/artifacts.rs:4152-4154`, `tools/case_paths_e2e.rs:296-298`, test modules) or a name match in a test function name.
2. `rg -n 'read_to_end|read_to_string' crates/core/src`: idiom "a stream read whole"; 27 hits. Production outside archives: `docs/extract.rs:566` (this class), `docs/service.rs:272-273` (converter output, other source: BL-0142), `storage/blob/local.rs:110` (the stored blob, bounded by its stored size), `config.rs:941`, `keys.rs:111` (operator files). The rest are tests.
3. `rg -n 'GzDecoder|ZlibDecoder|DeflateDecoder|BzDecoder|XzDecoder|flate2::read' crates/core/src`: idiom "a stream decompressor"; 9 hits, all in `tools/archive*.rs` (SPEC-0081 to SPEC-0085) or test encoders in `api/dataplane.rs` (`:3314`, `:3517`, `:3563`).
4. `rg -n '\.cloned\(\)' crates/core/src/docs`: idiom "a decoded value copied per reference" (expansion without decoding); `docs/extract.rs:1116` is the one copying uploaded content per cell; `docs/artifacts/capture.rs:143` copies failure strings the server built.
5. `rg -n 'pdf_extract::' crates/core/src`: idiom "a third party decoder handed the whole file"; `docs/extract.rs:497`, `:523` (BL-0143).

Not searched: decoders inside libgit2 pack handling (git objects pushed by a member: separate surface, bounded by `git.max_pack_size_mb` on input only) and the image decoders (none: images are passed to the OCR provider as stored bytes, `docs/extract.rs:187-199`).

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | `crates/core/src/docs/extract.rs:716` Word main part through `entry_text` (`:563-568`) | vulnerable (memory) | FR-NEW-001, FR-NEW-002; E2E-SEC-001 |
| 2 | `:871` PowerPoint slide | vulnerable | FR-NEW-002; E2E-SEC-002 |
| 3 | `:879` PowerPoint notes | vulnerable | FR-NEW-002; E2E-SEC-003 |
| 4 | `:896` presentation part | vulnerable | FR-NEW-002; E2E-SEC-004 |
| 5 | `:897` presentation relationships | vulnerable | FR-NEW-002; E2E-SEC-005 |
| 6 | `:939` slide relationships | vulnerable | FR-NEW-002; E2E-SEC-006 |
| 7 | `:992` Excel sheet (and its repeated reads) | vulnerable | FR-NEW-002, FR-NEW-003; E2E-SEC-007, E2E-011 |
| 8 | `:1006` shared strings | vulnerable | FR-NEW-002; E2E-SEC-008 |
| 9 | `:1046` workbook part | vulnerable | FR-NEW-002; E2E-SEC-009 |
| 10 | `:1048` workbook relationships | vulnerable | FR-NEW-002; E2E-SEC-010 |
| 11 | `:1116` shared text copied per cell into the grid and the Markdown | vulnerable (expansion) | FR-NEW-005; E2E-SEC-011 |
| 12 | `:563-568` declared sizes from 2^63 up: not counted today (no check reads them) | vulnerable to the bound once one exists; read as is today | FR-NEW-002 exact digits; E2E-SEC-012 |
| 13 | same, the largest declarable size | as 12 | FR-NEW-002; E2E-SEC-013 |
| 14 | `crates/core/src/docs/extract.rs:551` whole stored file copied to open the package | safe: bounded by the stored file size, admitted against the session allowance and the project's storage when it was written (`crates/core/src/quota.rs:47` `admit_file`, called at `crates/core/src/core/fs_ops.rs:578`, `:613`) | none |
| 15 | `crates/core/src/docs/zip_check.rs:36` header check | safe: reads headers only, never entry content (`zip_check.rs:17-19`) | none |
| 16 | `crates/core/src/docs/service.rs:272-273`, `:387`, `:519` converter output read whole | out of class source (the converter's output), still unbounded | BL-0142 |
| 17 | `crates/core/src/docs/extract.rs:497`, `:523` PDF streams inflated by the PDF library | same class, third party decoder | BL-0143 |
| 18 | `crates/core/src/tools/archive.rs`, `archive_rar.rs`, `archive_single.rs` | safe: bounded by SPEC-0084 (`crates/core/src/tools/archive.rs:24` `TAR_FRAMING` bound), SPEC-0085, SPEC-0081 to SPEC-0083 (`crates/core/src/tools/archive_single.rs:92-108` `read_capped`) | none |
| 19 | `crates/core/src/search/indexer.rs:173` | safe: reads the stored file, no decode (`String::from_utf8` rejects a zip, `:174`) | none |

### 14.3 Design inputs

- One reader serves every call site (`entry_text`, `extract.rs:563-568`); a capped read there, like `read_capped` (`archive_single.rs:92-108`, `CHUNK` 65 536 at `:14`), plus a per call counter threaded through the three format functions covers occurrences 1 to 10. Signal: E2E-SEC-001 to 010.
- `entry_text` swallows every error with `.ok()?` (`:564-566`); FR-NEW-002 to FR-NEW-004 need it to return a refusal while other errors keep returning absent (BL-0144). Signal: E2E-013, E2E-014, E2E-028.
- The declared size is the central directory's uncompressed size (`ZipFile::size()`), already checked equal to the local header by SPEC-0041 (`zip_check.rs:19-26`); zip64 sizes are 64 bit and must be compared unsigned, never cast to signed. Signal: E2E-SEC-012, 013.
- E2E-SEC-012 and 013 rest on the zip reader accepting a part holding fewer bytes than it declares with a matching checksum (`zip-9.0.5/src/crc32.rs:43-51` only checks the excess), as SPEC-0085 assumed: ASSUMED. The design reproduces it first; if the header check or the reader refuses the fixture, that refusal is recorded as the control and the tests stay as guards with the observed message.
- `decode` (`:418-420`) copies the part into a `String`, so the part is held twice for a moment; FR-NEW-006 counts decoded part data, and the design states whether the copy counts. Signal: E2E-006 held figure.
- The content count (FR-NEW-005) must include the grid cells kept in `BuiltinTable::cells` (`:52-67`, `:104-130`) and the Markdown lines; the Excel row builder (`:1071-1110`) and the Word table builder are where it grows. Signal: E2E-020, E2E-021, E2E-SEC-011.
- "Decoded bytes held" and "content held" need a hook like `LAST_HIGH_WATER` (`archive_single.rs:55-61`, test only today) for the extraction, reading 0 when no part was decoded. Signal: every test asserting held bytes.
- A bomb part is exactly 200 000 000 bytes, markup included, so messages carry `200000000`; the fixture builder pads the letter run to that total (E2E-023, E2E-SEC-001 to 010).
- E2E-035 needs the log line of a 4xx tool error; `logging.rs` classifies by `ToolError::is_client_error` (`errors.rs:154`).
- Docs to update: `.agent_docs/tools.md` (`fs.extract_text` limits), `.agent_docs/architecture.md` (document extraction bound), `README.md`; `TOOL_CONTRACT.txt` description of `fs.extract_text` only if the design chooses to mention the limits (a contract change, golden regenerated).

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code and the backlog entry, without the user, under their authorization.
- R1 scope: the three Office readers, both doors; out: unread parts, damaged parts (BL-0144), converter output (BL-0142), PDF (BL-0143), archives (SPEC-0084, SPEC-0085).
- R1 why now: BL-0134, deferred from SPEC-0085 occurrence 5, exploitable until shipped.
- R2 current state: Section 2 and 14.1, read from the code.
- R3 scenarios: SC-001 to SC-005, failures from the sweep and today's refusals.
- R4 requirements: derived, Section 6. R5 NFR: 0 held on early refusal, no trace, one log line, no slowdown.
- D-1 routing: BL-0134 reads `Nature: BUG` and the prompt would hand a BUG to /sdlc-analysis; the user's brief asks for a FEAT spec as for SPEC-0084 and SPEC-0087. Chosen FEAT: observable behavior changes (new refusals, a lying part refused instead of read as absent) and no spec defines a bound that today's code violates. Override recorded here.
- D-2 depth M: one module, 11 requirements, public behavior change, no persisted change.
- D-3 security `internal finding` (memory exhaustion, CWE-409, CWE-400); class sweep and exploit tests included as the prompt's security rules require.
- D-4 limits fixed, not configured: no existing configuration value bounds extraction (`crates/core/src/config.rs` holds `max_read_lines`, `max_diff_file_bytes`, `doc_service.max_input_bytes`, none for built in extraction); part limit 104 857 600 bytes (100 MiB, above any realistic main part), call limit and content limit 268 435 456 bytes (256 MiB).
- D-5 the refusal code is `ERR_INVALID_ARGUMENT`, the code every extraction failure carries today (`extract.rs:369-373`); no new code. Messages keep the `could not extract <path>: ` prefix.
- D-6 the declared size decides the part and call limits (checked before decoding, as SPEC-0085 does for archives); the actual size is bounded by the size lie refusal.
- D-7 a part larger than it declares is refused (FR-NEW-004) instead of read as absent; other damage stays as today (BL-0144).
- D-8 parts not read are neither counted nor opened; nested archives are not recursed.
- D-9 a part read twice counts twice (a sheet listed twice is decoded twice today).
- D-10 the content limit counts the Markdown text and the table cells, because the uncapped grid is where cell multiplication lands.
- D-11 no quota interplay: a read writes nothing; the refusal charges nothing and records no audit line, as every extraction failure today.
- D-12 chunk 65 536 bytes, as SPEC-0081 to SPEC-0085.
- D-13 tests: 50 new, 0 modified, 0 removed; the test plan is approved by the user's request "spec BL-0134".
- D-14 BL-0142, BL-0143, BL-0144 filed for what the sweep found outside this increment.
