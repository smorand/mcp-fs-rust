# Bound nesting depth in PDF extraction and code search: functional specification

> Id: SPEC-0092
> Nature: FEAT
> Depth: M
> Depth evidence: 2 bounded contexts (document text extraction for PDF, code search), 9 requirements, public behavior change (a PDF whose forms, page tree or character maps nest past fixed depths is refused instead of ending the server; code search answers on deeply nested source instead of ending the server), no persisted change. Override with --depth.
> Status: implemented
> Generated on: 2026-11-11
> Project: existing
> From backlog: BL-0147
> Depends on: none (SPEC-0089 built: its refusal family, its order of PDF refusals and its "nothing left" checks are reused)
> Security: internal finding
> CVSS: not scored
> Affected: every build whose text extraction reads `.pdf` documents, and every build serving the definition and reference search
> Fixed in: unreleased
>
> Note: written non interactively. The user authorized every decision; the request "specify BL-0147" is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4; the reproduction that preceded the spec is in Section 14.5.

## 1. Summary

A member who asks for the text of a PDF, or who searches the project's source code for a definition or its references, can today end the whole server, for every project, with one small file: a PDF form that draws itself, two forms that draw each other, a chain of a few hundred forms, a page whose chain of parent page groups loops or runs thousands of levels deep, a font character map nested a thousand levels deep, or a source file with ten thousand nested parentheses. The server process stops at once; nothing it does today contains it. After this increment the PDF is refused with a clear message naming the nesting limit it passed, and the code search answers the deeply nested file normally. Both doors, the assistant tool and the HTTP API, answer alike, and the server keeps serving every other member.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member asks for the text of a `.pdf` document (any letter case) through the assistant tool or the HTTP API. The server measures every stream it will decode under the SPEC-0089 limits, then builds the text page by page. A PDF it cannot read is refused with `ERR_INVALID_ARGUMENT` and a message starting `could not extract <path>: `; a refusal writes no companion, stores no artifact, charges nothing, records no audit line and is logged once at the informational level. [14.1 row 1]
- A page draws a form by name; a form draws other forms in turn. The SPEC-0089 checks, run before the text is built, follow these draws without a depth limit and, meeting a form already being drawn, skip it without refusing. Building the text then draws them again, one level inside the other, with no limit on how deep. A form that draws itself, directly or through other forms, never ends; a chain of 400 distinct forms is already too deep. In both cases the server process ends abruptly. [rows 2, 3]
- A page takes its resources and its page size from the page groups above it when it lacks them. Building the text climbs from the page to its parent, then to that parent's parent, until it finds them, with no limit. A parent chain that loops back on itself without holding them never ends; a chain of 10 000 page groups is already too deep. In both cases the server process ends. A chain of 3 000 page groups is extracted today. [row 4]
- A font can carry a character map, the table that turns its codes into text, and a composite font carries a code map in the same notation. Building the text reads each one it selects with no limit on how deeply its lists, dictionaries and bracketed strings nest; a nesting of 1 000 levels ends the server process. A nesting of 65 levels is read today. [row 5]
- A member searches a folder of the project for a definition by name, or for the references to a name, through the assistant tool or the HTTP API. The server reads every file of a recognised language under the folder and scans its syntax. The scan goes one level deeper for every nested construct, with no limit; a Python file with 10 000 nested parentheses ends the server process. A nesting of 1 000 is answered today. [row 6]
- Ending the server process stops every project, every member and every request in flight until an operator restarts it. Nothing is logged for the call that caused it. [row 7]

### 2.2 Specs governing this area

- SPEC-0089 bounded the bytes the PDF reader decodes (stream limit, load total, page total, object limit, content limit), set the order of PDF refusals (FR-NEW-007) and kept PDF extraction otherwise unchanged (FR-NEW-009). It deferred the self drawing form as BL-0147, this entry. This increment adds three refusals to that family and places them in that order; SPEC-0089 is not modified. Its design skipped a form already being drawn (DEC-007); this increment turns that case into a refusal, which no SPEC-0089 requirement or test forbids.
- SPEC-0006 FR-023 to FR-025 and SPEC-0003 FR-026 define the definition and reference search: language from the extension, syntax scan first, word matching second, kinds per language. This increment keeps their answers and adds that the scan answers whatever the nesting.
- SPEC-0088 set the refusal convention for document extraction (the `could not extract <path>: ` prefix, nothing left). [row 8]

### 2.3 What is verified today

- PDF extraction is tested for text, pages, title, a corrupt PDF, and the SPEC-0089 limits, forms included, every form drawn directly by its page. Code search is tested for definitions and references in each language, kinds, word matching fallback, and both doors. [row 9]
- No test covers a form drawn inside a form, a form drawing itself, a page inheriting from page groups above it, a character map nesting, or a deeply nested source file. No test checks that the server is still running after a call. [row 9]

## 3. Scope

### 3.1 In scope

- PDF text extraction, any letter case, both doors, with and without refresh: a limit on how deeply forms are drawn inside forms, on how far a page climbs its parent page groups for its resources and its page size, and on how deeply a character map or a composite font's code map nests.
- Definition and reference search, both doors: answering a source file of any nesting.
- The server staying up for every other member through all of the above.

### 3.2 Out of scope, and why

- The bytes a PDF makes the server decode: bounded by SPEC-0089.
- Nesting inside a PDF's own objects and page content: already refused by the reader, reproduced (14.5 rows 9 to 12).
- The PDF page listing: already limited to 256 levels and safe against loops (14.2 occurrence 11).
- Office documents, HTML, Markdown, converter answers: read without unbounded nesting (14.2 occurrences 13 to 16).
- Folder trees and git trees nested deeply by a member: not document or source content; reported to the owner as side findings, not specified here (14.4 D-12).
- Where in the server extraction runs: BL-0146.
- A configurable nesting limit: fixed numbers are enough (14.4 D-6).

### 3.3 Constraints imposed by the context

- No new third party component unless the design shows it unavoidable, and no unsafe code (house rule kept from SPEC-0089 3.3); the design states which.

## 4. Actors

- **Member:** a person with access to a project; asks for a document's text, or searches its source code, through the assistant tool or the HTTP API.
- **Operator:** runs the server, restarts it when it stops, reads its log and audit trail.
- **Attacker:** a member, or someone who got a member to store a file, who crafts a PDF or a source file to stop the server for everyone.
- **Bystander:** another member, in another project, whose requests must keep being answered.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Document text extraction | turning a stored PDF into Markdown text | form, form depth, page group, page tree climb, character map, code map, nesting, refusal |
| Code search | finding definitions and references in the project's source files | source file, language, definition, reference, nesting |
| Project storage | files, companions, artifacts, session write allowance, audit trail | companion, session write allowance, audit line |

## 5. Usage scenarios

### SC-001: Extract the text of a PDF within the nesting limits
**Actor:** Member.
**Preconditions:** project `atlas`, member `mia@example.com`; `/docs/forms.pdf`, one page that draws form `/A`, which draws form `/B`, which draws form `/C`, which shows `Deep text`.
**Flow:**
1. The member asks for its text → the system reads the PDF, every nesting within its limit.
2. The system writes `/docs/forms.md`, answers with its path, 1 page and a preview showing `Deep text`, and records one audit line.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: a nesting passes its limit → SC-002.

### SC-002: A PDF nesting past a limit is refused, the server keeps running
**Actor:** Member or attacker; bystander.
**Preconditions:** `/docs/loop.pdf`, under 2 000 bytes, whose page 1 draws form `/A`, form `/A` drawing `/A`; member `leo@example.com` of project `orbit`, which holds `/notes/hello.txt` reading `hello`.
**Flow:**
1. The member asks for its text → the system refuses with the form depth message for page 1.
2. The bystander reads `/notes/hello.txt` → receives `hello`.
**Postconditions:** no companion, no artifact, no charge, no audit line; the server answers every request as before.
**Exceptions:**
- EXC-002a: a page climbs more than 256 page groups for its resources or its page size, or its parent chain loops without holding them → refused with the page tree message.
- EXC-002b: a character map or code map the page selects nests more than 64 levels → refused with the character map message.

### SC-003: Order when several refusals apply to a PDF
**Actor:** Member.
**Flow:**
1. Membership and a missing path first, then the load, as SPEC-0089 FR-NEW-007 states.
2. Then pages in page order; for each page, first its climb for resources then for page size, then its decodes and draws in drawing order, a draw checked for form depth before its stream is measured, a character map checked for nesting after its stream is measured.
3. Then the title read, as today.

### SC-004: Repeat, refresh and the stored companion
**Actor:** Member, operator.
**Flow:**
1. A refused call repeated gives the same refusal and writes nothing.
2. With an up to date companion and no refresh, the stored text is returned without reading the PDF.
3. With a refresh, the PDF is read again and refused; the stored companion keeps its bytes.
4. The operator finds one informational log line per refused call and no audit line.

### SC-005: Search source code of any nesting
**Actor:** Member or attacker; bystander.
**Preconditions:** project `atlas`; `/src/deep.py` holding `def f():` then a line `    return ` followed by 100 000 `(`, the digit `1` and 100 000 `)`.
**Flow:**
1. The member asks for the definition of `f` under `/src` → the system answers one definition: `/src/deep.py`, name `f`, kind `function_definition`, line 1.
2. The member asks for the references to `f` → the system answers them, as for any file.
3. The bystander reads `/notes/hello.txt` → receives `hello`.
**Postconditions:** nothing written; the server answers every request as before.
**Exceptions:**
- EXC-005a: the caller is not a member, or asks references for an empty name → refused as today.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-O]: Form depth limit
> IF a page of a PDF draws a form at depth 65 THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> draws PDF forms nested more than 64 deep`.
- **Business rules:** the page content is depth 0; a form the page draws is depth 1; a form drawn by a form at depth d is depth d + 1. Depth 64 is accepted; the draw that reaches depth 65 is refused before that form is decoded. Drawing the same form many times one after the other does not add depth. A form that draws itself, directly or through other forms, reaches depth 65 and is refused by this rule. `<N>` is the page number, from 1, in plain digits; `<path>` is the document's project path, normalized, as in every extraction message.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Page tree climb limit
> IF finding a page's resources, or finding its page size, climbs more than 256 page groups above the page THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> inherits from more than 256 levels of the PDF page tree`.
- **Business rules:** each climb starts at the page and stops at the first of the page or a page group above it that holds a usable value: a dictionary for the resources, a list for the page size; a value of another kind is passed over, as today, and the climb goes on. The page's own value climbs 0, its parent's 1. Resources and page size are climbed separately; either one passing 256 refuses. Exactly 256 is accepted. A parent chain that loops back before holding the value counts as passing 256. A climb that reaches a page group with no parent without finding the value keeps today's answer: resources absent read as empty, a page size absent is refused as today (`could not extract <path>: pdf parser failed on this document`).
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Character map nesting limit
> IF a character map or a composite font's code map that a page selects nests more than 64 levels THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: a PDF character map is nested more than 64 deep`.
- **Business rules:** a level is a list `[ ]`, a dictionary `<< >>` or a bracketed string `( )` opened inside another one, the three kinds counted together; brackets escaped inside a string and the delimiters of hexadecimal strings `< >` do not count. 64 levels are accepted. Only maps the extraction reads count: the character map of each font a page selects and the code map of each composite font a page selects. A map within the limit that cannot be read keeps today's answer, whatever it is.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Order of PDF refusals
> IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in the order of SPEC-0089 FR-NEW-007, item 4 read as: pages in page order; for each page, first FR-NEW-002 for the resources, then FR-NEW-002 for the page size, then each decode and draw in drawing order, where a draw checks FR-NEW-001 before its stream is measured and a character map or code map checks SPEC-0089 FR-NEW-002, then SPEC-0089 FR-NEW-004, then FR-NEW-003.
- **Business rules:** items 1 to 3 and 5 of SPEC-0089 FR-NEW-007 are unchanged: membership, missing path and load come before every refusal of this specification.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-005 [EARS-UB]: A nesting refusal leaves no trace
> THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-003.
- **Business rules:** an existing companion, stale or up to date, keeps its bytes. The refusal is logged once at the informational level, naming `ERR_INVALID_ARGUMENT` and the message.
- **Scenario:** SC-002, SC-004
- **Priority:** Must

#### FR-NEW-006 [EARS-UB]: No document or source file ends the server
> THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- **Business rules:** after any call of this specification, refused or answered, the server answers the next request of any member in any project within 1 second. Calls of other members running at the same time complete as if the call had not been made.
- **Scenario:** SC-002, SC-005
- **Priority:** Must

#### FR-NEW-007 [EARS-U]: PDF extraction is otherwise unchanged
> THE document text extraction SHALL answer a PDF whose nestings are within FR-NEW-001 to FR-NEW-003 with the same text, page markers, page count, title, note, companion, preview, charge, audit line and refusals as before, and SHALL answer from an up to date companion without reading the PDF when no refresh is asked.
- **Business rules:** includes a form drawn many times side by side, a parent chain that loops above a page holding its own resources and page size, and today's refusal of a PDF that cannot be read.
- **Scenario:** SC-001, SC-004
- **Priority:** Must

#### FR-NEW-008 [EARS-U]: Code search answers any nesting
> THE code search SHALL answer a definition or reference search over source files of any nesting with the same definitions and references, in the same order, with the same fields, as it answers today for the same files when it does not end the server.
- **Business rules:** for a file whose nesting ends the server today, the answer is the one the same search gives on the file read without depth limit: every definition and every reference it holds, in the order they appear in the file. Refusals are unchanged: not a member, missing folder, and `name is required` for a reference search with an empty name. No new refusal is added.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-009 [EARS-U]: Both doors alike
> THE product SHALL answer a PDF text extraction and a definition or reference search through the HTTP API with the same code, message, results and effects as through the assistant tool.
- **Scenario:** SC-002, SC-005
- **Priority:** Must

### 6.2 Modified

None. No spec defines a nesting bound on PDF forms, page trees or character maps, nor on the code search scan. SPEC-0089 FR-NEW-007 is extended, not changed, by FR-NEW-004 (2.2).

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN a PDF is refused under FR-NEW-001 to FR-NEW-003 THE document text extraction SHALL answer within 2 seconds for a file stored in under 2 000 000 bytes.
- WHEN a definition or reference search reads a source file of 100 000 nested parentheses THE code search SHALL answer within 10 seconds.

### 7.2 Security and access
- THE product SHALL require the same project membership as before for extraction and search.
- THE product SHALL NOT let any PDF or source file a member stores make the server stop for other members (FR-NEW-006).

### 7.5 Reliability
- WHILE a refusal under this specification is being answered THE product SHALL keep answering other members' calls.

### 7.6 Operability
- WHEN a PDF call is refused under FR-NEW-001 to FR-NEW-003 THE document text extraction SHALL leave exactly one informational log line naming `ERR_INVALID_ARGUMENT` and the message, and no audit line.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | page drawing no form; page with its own resources and page size; font without character map; source file with no nesting | as today | E2E-001, E2E-009, E2E-003 |
| Boundary values | forms at depth 64 and 65; climb of 256 and 257; character map nesting 64 and 65 | accepted; refused | E2E-005, E2E-012, E2E-006, E2E-013, E2E-007, E2E-014 |
| Concurrent actions | two self drawing PDFs and a normal PDF at once; a bystander reading during a refusal | bombs refused, normal succeeds, bystander served | E2E-028, E2E-SEC-001 |
| Slow or lost connectivity | not applicable: everything is local to the server | none | none |
| Access states | not a member, PDF and search | membership refusal first, as today | E2E-018, E2E-025 |
| Data variations | upper case `.PDF`; Python, C and JavaScript sources; form without its own resources | refused alike; answered alike | E2E-016, E2E-SEC-011 to E2E-SEC-013, E2E-SEC-002 |
| Navigation | refresh on a refused PDF with a companion; up to date companion without refresh | refused, companion kept; stored text returned | E2E-024, E2E-011 |
| Partial operations | refused call repeated | same refusal, nothing written | E2E-023 |
| Dependency failures | not applicable: no external service is involved | none | none |
| Ordering and timing | load before pages; page 1 stream limit before page 2 loop; climb before draws; draw depth before a later character map | FR-NEW-004 | E2E-019 to E2E-022 |
| Exhaustion | form loops and long chains; looping and long parent chains; deep character and code maps; deeply nested sources in three languages | refused (PDF) or answered (search), server up | E2E-SEC-001 to E2E-SEC-013 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Unless a test says otherwise: project `atlas`, member `mia@example.com`, the call goes through the assistant tool with default options (`max_chars` 200 000, no refresh), the session write allowance is 1 000 000 000 bytes with nothing written yet. Every PDF is written by the test as the attacker writes it: catalog object 1, page tree object 2 listing page object 3 directly, page 3 with `/MediaBox [0 0 612 792]`, `/Resources << /Font << /F1 5 0 R >> ... >>` and content object 4, font 5 `<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>`, uncompressed streams, unless the test replaces an object. "Nothing left" is SPEC-0089's: no `/docs/<name>.md` in the listing of `/docs` (or an existing one keeps its bytes), no artifact listed for the document, no extraction line in the audit trail, and a following write of a 52 428 800 byte file `/docs/fill.bin` by the same member succeeds. "The server is still up" means: within 1 second after the answer, member `leo@example.com` of project `orbit` reads `/notes/hello.txt` and receives `hello`, and the server's process is the one that answered before the call.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-007 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-007, FR-NEW-009 | High |
| E2E-003 | New | happy | SC-005 | FR-NEW-008 | High |
| E2E-004 | New | happy | SC-005 | FR-NEW-008, FR-NEW-009 | High |
| E2E-005 | New | edge | SC-001 | FR-NEW-001, FR-NEW-007 | Critical |
| E2E-006 | New | edge | SC-001 | FR-NEW-002, FR-NEW-007 | Critical |
| E2E-007 | New | edge | SC-001 | FR-NEW-003, FR-NEW-007 | Critical |
| E2E-008 | New | edge | SC-001 | FR-NEW-001, FR-NEW-007 | High |
| E2E-009 | New | edge | SC-001 | FR-NEW-002, FR-NEW-007 | High |
| E2E-010 | New | edge | SC-005 | FR-NEW-008 | High |
| E2E-011 | New | edge | SC-004 | FR-NEW-007 | Medium |
| E2E-012 | New | failure | SC-002 | FR-NEW-001, FR-NEW-005 | Critical |
| E2E-013 | New | failure | SC-002 | FR-NEW-002, FR-NEW-005 | Critical |
| E2E-014 | New | failure | SC-002 | FR-NEW-003, FR-NEW-005 | Critical |
| E2E-015 | New | failure | SC-002 | FR-NEW-001, FR-NEW-006, FR-NEW-009 | Critical |
| E2E-016 | New | failure | SC-002 | FR-NEW-001 | Medium |
| E2E-017 | New | failure | SC-002 | FR-NEW-002, FR-NEW-006 | High |
| E2E-018 | New | failure | SC-003 | FR-NEW-004 | Medium |
| E2E-019 | New | failure | SC-003 | FR-NEW-004 | Medium |
| E2E-020 | New | failure | SC-003 | FR-NEW-004 | Medium |
| E2E-021 | New | failure | SC-003 | FR-NEW-002, FR-NEW-004 | Medium |
| E2E-022 | New | failure | SC-003 | FR-NEW-001, FR-NEW-004 | Medium |
| E2E-023 | New | failure | SC-004 | FR-NEW-005 | Medium |
| E2E-024 | New | failure | SC-004 | FR-NEW-005, FR-NEW-007 | High |
| E2E-025 | New | failure | SC-005 | FR-NEW-008 | Medium |
| E2E-026 | New | failure | SC-005 | FR-NEW-008, FR-NEW-009 | Medium |
| E2E-027 | New | side effect | SC-004 | FR-NEW-005 | Medium |
| E2E-028 | New | side effect | SC-002 | FR-NEW-006 | High |
| E2E-SEC-001 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-005, FR-NEW-006 | Critical |
| E2E-SEC-002 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-006 | Critical |
| E2E-SEC-003 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-006 | Critical |
| E2E-SEC-004 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-006 | Critical |
| E2E-SEC-005 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-006 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-007 | New | failure, security | SC-002 | FR-NEW-003, FR-NEW-006 | Critical |
| E2E-SEC-008 | New | failure, security | SC-002 | FR-NEW-003, FR-NEW-006 | Critical |
| E2E-SEC-009 | New | failure, security | SC-002 | FR-NEW-003, FR-NEW-006 | Critical |
| E2E-SEC-010 | New | failure, security | SC-002 | FR-NEW-003, FR-NEW-006 | Critical |
| E2E-SEC-011 | New | edge, security | SC-005 | FR-NEW-006, FR-NEW-008 | Critical |
| E2E-SEC-012 | New | edge, security | SC-005 | FR-NEW-006, FR-NEW-008 | Critical |
| E2E-SEC-013 | New | edge, security | SC-005 | FR-NEW-006, FR-NEW-008, FR-NEW-009 | Critical |

Coverage (41 tests: 41 new, 0 modified, 0 removed): happy 4, failure 25 (10 of them also security), side effect 2, edge 10 (3 of them also security), state transition 0 (no state machine: a document is extracted or refused, a search answers; nothing persists between calls). Happy to failure ratio 1:6.25.

### 9.2 New tests

#### E2E-001: PDF with three nested forms
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007; **Priority:** High
- **Given** `/docs/forms.pdf` whose page content is `BT /F1 12 Tf 72 700 Td (Cover) Tj ET /A Do`, `/A` a form drawing `/B`, `/B` a form drawing `/C`, `/C` a form showing `BT /F1 12 Tf 72 600 Td (Deep text) Tj ET`, each form naming the next one and `/F1` in its own resources
- **When** the member asks for its text
- **Then** the answer names `/docs/forms.md`, format `pdf`, 1 page, not cached; its preview reads `*[Page 1]*`, `Cover`, `Deep text`, in that order
- **And** `/docs/forms.md` holds that text; the audit trail has one extraction line for `/docs/forms.md`

#### E2E-002: Nested forms through the HTTP API
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-007, FR-NEW-009; **Priority:** High
- **Given** the document of E2E-001
- **When** a client asks the HTTP API for its text
- **Then** the answer carries the same companion path, format, page count and preview as E2E-001
- **And** the audit trail has one extraction line for `/docs/forms.md`

#### E2E-003: Definition in a moderately nested Python file
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/src/mid.py` holding `def f():` then `    return ` followed by 1 000 `(`, the digit `1` and 1 000 `)`
- **When** the member asks for the definition of `f` under `/src`
- **Then** the answer lists exactly one definition: path `/src/mid.py`, name `f`, kind `function_definition`, line 1

#### E2E-004: References in a moderately nested file through the HTTP API
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-009; **Priority:** High
- **Given** `/src/mid.py` of E2E-003 with a third line `f()`
- **When** a client asks the HTTP API for the references to `f` under `/src`
- **Then** the answer lists exactly the references the assistant tool lists for the same call, the line 3 reference among them, with the same fields and order

#### E2E-005: Forms nested exactly 64 deep
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/deep64.pdf` whose page content is `/A Do` with `/A` form 10; forms 10 to 72 each `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /A n+1 0 R >> >> >>` drawing `/A Do`, n being its own number; form 73 showing `BT /F1 12 Tf 72 600 Td (Bottom) Tj ET` with `/F1` in its resources (64 forms)
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Bottom`
- **And** the server is still up

#### E2E-006: Resources and page size 256 page groups up
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/tall.pdf` whose page 3 is `<< /Type /Page /Parent 100 0 R /Contents 4 0 R >>` (no resources, no page size), page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`, objects 100 to 355 each `<< /Type /Pages /Parent n+1 0 R >>` except object 355, `<< /Type /Pages /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] >>` (256 page groups)
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Hello`

#### E2E-007: Character map nested exactly 64 deep
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003, FR-NEW-007; **Priority:** Critical
- **Given** `/docs/cmap64.pdf` whose page content is `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`, font 5 `<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 9 0 R >>`, object 9 a stream `/CIDInit /ProcSet findresource begin 12 dict begin begincmap /X ` then 64 `[`, 64 `]`, then ` def endcmap end end`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Hello`

#### E2E-008: One form drawn 50 times side by side
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-007; **Priority:** High
- **Given** `/docs/repeat.pdf` whose page content is `/A Do` written 50 times, `/A` a form showing `BT /F1 12 Tf 72 600 Td (Tile) Tj ET` with `/F1` in its resources
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Tile` 50 times

#### E2E-009: Looping parent chain above a self sufficient page
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-007; **Priority:** High
- **Given** `/docs/selfloop.pdf` whose page 3 holds its own resources and page size and `/Parent 100 0 R`, object 100 being `<< /Type /Pages /Parent 100 0 R >>`, page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview shows `Hello`, as today

#### E2E-010: References keep their order
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/src/order.py` holding `total = 1`, `print(total)`, `total += 2` on lines 1 to 3
- **When** the member asks for the references to `total` under `/src`
- **Then** the answer lists the identifier references on lines 1, 2 and 3, in the same order and with the same kinds as the answer recorded before the change

#### E2E-011: Up to date companion without refresh
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/loop.pdf` of E2E-SEC-001 and an up to date `/docs/loop.md` reading `kept`
- **When** the member asks for its text without refresh
- **Then** the answer is cached and its preview reads `kept` (reading the PDF refuses the call, E2E-SEC-001)
- **And** the server is still up

#### E2E-012: Forms nested 65 deep
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-005; **Priority:** Critical
- **Given** `/docs/deep65.pdf` as E2E-005 with one form more: forms 10 to 73 draw the next, form 74 shows `Bottom` (65 forms)
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/deep65.pdf: page 1 draws PDF forms nested more than 64 deep`
- **And** nothing left; the server is still up

#### E2E-013: Resources 257 page groups up
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-005; **Priority:** Critical
- **Given** `/docs/taller.pdf` as E2E-006 with objects 100 to 356, object 356 holding the resources and page size (257 page groups)
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/taller.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- **And** nothing left; the server is still up

#### E2E-014: Character map nested 65 deep
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-005; **Priority:** Critical
- **Given** `/docs/cmap65.pdf` as E2E-007 with 65 `[` and 65 `]`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/cmap65.pdf: a PDF character map is nested more than 64 deep`
- **And** nothing left; the server is still up

#### E2E-015: Self drawing form through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001, FR-NEW-006, FR-NEW-009; **Priority:** Critical
- **Given** `/docs/loop.pdf` of E2E-SEC-001
- **When** a client asks the HTTP API for its text
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep`, the same as through the assistant tool
- **And** nothing left; the server is still up

#### E2E-016: Upper case suffix
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** Medium
- **Given** the bytes of `/docs/loop.pdf` stored as `/docs/LOOP.PDF`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/LOOP.PDF: page 1 draws PDF forms nested more than 64 deep`

#### E2E-017: Page size missing along a looping chain
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** High
- **Given** `/docs/nosize.pdf` whose page 3 holds its resources but no page size, and `/Parent 100 0 R`, object 100 being `<< /Type /Pages /Parent 100 0 R >>`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/nosize.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- **And** the server is still up

#### E2E-018: Not a member, self drawing form
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/loop.pdf` and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its text
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`

#### E2E-019: Load refusal before a form loop
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/both.pdf`, `/docs/loop.pdf` with object streams holding 2 000 000 more objects written `null`, stored in under 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/both.pdf: the PDF holds more than 1000000 objects`

#### E2E-020: Page 1 stream limit before a page 2 form loop
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/two.pdf` whose page 1 content is a PDF bomb stream (SPEC-0089: under 300 000 stored bytes inflating to 200 000 000 spaces) and whose page 2 draws the self drawing form of E2E-SEC-001
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/two.pdf: a PDF stream decodes to more than 104857600 bytes`

#### E2E-021: Page tree climb before the page's draws
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-002, FR-NEW-004; **Priority:** Medium
- **Given** `/docs/climbloop.pdf` whose page 3 has no resources and no page size, `/Parent 100 0 R`, object 100 `<< /Type /Pages /Parent 100 0 R >>`, and whose page content is `/A Do`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/climbloop.pdf: page 1 inherits from more than 256 levels of the PDF page tree`

#### E2E-022: Form depth before a later character map on the same page
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-004; **Priority:** Medium
- **Given** `/docs/order.pdf` whose page content is `/A Do BT /F2 12 Tf (x) Tj ET`, `/A` the self drawing form of E2E-SEC-001, `/F2` in the page resources a font whose character map nests 1 000 `[`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/order.pdf: page 1 draws PDF forms nested more than 64 deep`

#### E2E-023: Refusal repeated
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/docs/loop.pdf`
- **When** the member asks for its text twice
- **Then** both answers are the same refusal
- **And** nothing left; the audit trail has no line for `/docs/loop.md`

#### E2E-024: Refresh with an existing companion
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-005, FR-NEW-007; **Priority:** High
- **Given** `/docs/loop.pdf` and an existing `/docs/loop.md` reading `kept`
- **When** the member asks for its text with refresh
- **Then** the refusal is the form depth message for page 1 of `/docs/loop.pdf`
- **And** `/docs/loop.md` still reads `kept`; nothing else left; the server is still up

#### E2E-025: Not a member, deep source
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/src/deep.py` of E2E-SEC-011 and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for the definition of `f` under `/src`
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`

#### E2E-026: Empty name over a deep source through the HTTP API
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-009; **Priority:** Medium
- **Given** `/src/deep.py` of E2E-SEC-011
- **When** a client asks the HTTP API for the references to an empty name under `/src`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `name is required`, as through the assistant tool

#### E2E-027: Operator trail of a refusal
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-005; **Priority:** Medium
- **Given** `/docs/loop.pdf`
- **When** the member asks for its text
- **Then** the server log holds exactly one informational line naming `ERR_INVALID_ARGUMENT` and `could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep`
- **And** the audit trail holds no line for the call

#### E2E-028: Two attacks and a normal extraction at once
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-006; **Priority:** High
- **Given** `/docs/loop.pdf`, `/docs/pair.pdf` of E2E-SEC-003 and `/docs/forms.pdf` of E2E-001
- **When** three calls of the member, one per document, start at the same time
- **Then** the first two are refused with the form depth message for page 1 of their document; the third succeeds as in E2E-001
- **And** the server is still up

### 9.3 Exploit tests

Each covers one occurrence of the class sweep (Section 14.2) at its own location. The payload is written as the attacker writes the file. "Red before the fix" was observed in the reproduction (14.5): the process serving the call aborts, so no answer arrives.

#### E2E-SEC-001: A form that draws itself
- **Covers:** occurrence 1 (the originating occurrence of BL-0147)
- **Red before the fix because:** `Then the refusal is ...` fails: the process serving the call aborts (signal 6, 14.5 row 1) and no answer arrives; `And the server is still up` fails with it.
- **Given** `/docs/loop.pdf`, 862 bytes, whose page content is `BT /F1 12 Tf 72 700 Td (Hello) Tj ET /A Do`, page resources `/XObject << /A 10 0 R >>`, object 10 `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /A 10 0 R >> >> /Length 5 >> stream` `/A Do` `endstream`
- **When** the member asks for its text, while `leo@example.com` reads `/notes/hello.txt` in a loop
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/loop.pdf: page 1 draws PDF forms nested more than 64 deep`
- **And** every read of `leo@example.com` receives `hello`; nothing left; the server is still up

#### E2E-SEC-002: A form inheriting the resources that name it
- **Covers:** occurrence 1, the form without resources of its own
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 2).
- **Given** `/docs/inherit.pdf` as `/docs/loop.pdf` but object 10 is `<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 5 >> stream` `/A Do` `endstream`, using the page's resources
- **When** the member asks for its text
- **Then** the refusal is the form depth message for page 1 of `/docs/inherit.pdf`
- **And** nothing left; the server is still up

#### E2E-SEC-003: Two forms drawing each other
- **Covers:** occurrence 1, through another form
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 3).
- **Given** `/docs/pair.pdf` whose page draws `/A`, object 10 a form with resources `/XObject << /B 11 0 R >>` and content `/B Do`, object 11 a form with resources `/XObject << /A 10 0 R >>` and content `/A Do`
- **When** the member asks for its text
- **Then** the refusal is the form depth message for page 1 of `/docs/pair.pdf`
- **And** nothing left; the server is still up

#### E2E-SEC-004: A chain of 10 000 distinct forms
- **Covers:** occurrence 1, depth without a loop
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 6; a chain of 400 already ends it, row 5).
- **Given** `/docs/chain.pdf`, about 1 700 000 bytes, whose page draws `/A` (object 10), each object n from 10 to 10 008 a form with resources `/XObject << /A n+1 0 R >>` and content `/A Do`, object 10 009 a form showing `BT /F1 12 Tf 72 600 Td (Deep) Tj ET`
- **When** the member asks for its text
- **Then** the refusal is the form depth message for page 1 of `/docs/chain.pdf`, within 2 seconds
- **And** nothing left; the server is still up

#### E2E-SEC-005: A page whose parent is itself a loop
- **Covers:** occurrence 2, loop
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 7).
- **Given** `/docs/parentloop.pdf` whose page 3 is `<< /Type /Page /Parent 100 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>` (no resources), object 100 `<< /Type /Pages /Parent 100 0 R >>`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/parentloop.pdf: page 1 inherits from more than 256 levels of the PDF page tree`
- **And** nothing left; the server is still up

#### E2E-SEC-006: A parent chain of 10 000 page groups
- **Covers:** occurrence 2, depth without a loop
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 8; a chain of 3 000 is extracted today).
- **Given** `/docs/parents.pdf` as E2E-006 with objects 100 to 10 099, object 10 099 holding the resources and page size
- **When** the member asks for its text
- **Then** the refusal is the page tree message for page 1 of `/docs/parents.pdf`
- **And** nothing left; the server is still up

#### E2E-SEC-007: Character map of 10 000 nested lists
- **Covers:** occurrence 3, lists
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 14; 1 000 already ends it).
- **Given** `/docs/cmapdeep.pdf` as E2E-007 with 10 000 `[` and 10 000 `]`
- **When** the member asks for its text
- **Then** the refusal is `could not extract /docs/cmapdeep.pdf: a PDF character map is nested more than 64 deep`
- **And** nothing left; the server is still up

#### E2E-SEC-008: Character map with 100 000 nested bracketed strings
- **Covers:** occurrence 3, strings
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 15).
- **Given** `/docs/cmapparen.pdf` as E2E-007 but object 9 is `/CIDInit /ProcSet findresource begin begincmap /X ` then 100 000 `(`, 100 000 `)`, then ` def endcmap end`
- **When** the member asks for its text
- **Then** the refusal is the character map message for `/docs/cmapparen.pdf`
- **And** nothing left; the server is still up

#### E2E-SEC-009: Character map with 100 000 nested dictionaries
- **Covers:** occurrence 3, dictionaries
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 16).
- **Given** `/docs/cmapdict.pdf` as E2E-007 but the nesting is `<< /K ` written 100 000 times then `>> ` written 100 000 times
- **When** the member asks for its text
- **Then** the refusal is the character map message for `/docs/cmapdict.pdf`
- **And** nothing left; the server is still up

#### E2E-SEC-010: Composite font code map of 10 000 nested lists
- **Covers:** occurrence 4
- **Red before the fix because:** `Then the refusal is ...` fails: the process ends (14.5 row 17).
- **Given** `/docs/type0.pdf` whose font 5 is `<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding 9 0 R /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X >>] >>`, object 9 a stream `/CIDInit /ProcSet findresource begin begincmap /X ` then 10 000 `[`, 10 000 `]`, then ` def endcmap end`, page content `BT /F1 12 Tf 72 700 Td (Hello) Tj ET`
- **When** the member asks for its text
- **Then** the refusal is the character map message for `/docs/type0.pdf`
- **And** nothing left; the server is still up

#### E2E-SEC-011: Definition search over 100 000 nested parentheses
- **Covers:** occurrence 5
- **Red before the fix because:** `Then the answer lists ...` fails: the process ends (14.5 row 19; 10 000 already ends it, row 18).
- **Given** `/src/deep.py`, about 200 000 bytes: `def f():` newline, `    return ` followed by 100 000 `(`, the digit `1`, 100 000 `)` and a newline
- **When** the member asks for the definition of `f` under `/src`, while `leo@example.com` reads `/notes/hello.txt` in a loop
- **Then** the answer lists exactly one definition: path `/src/deep.py`, name `f`, kind `function_definition`, line 1, within 10 seconds
- **And** every read of `leo@example.com` receives `hello`; the server is still up

#### E2E-SEC-012: C definition whose name hides in a deep body
- **Covers:** occurrence 6
- **Red before the fix because:** `Then the answer lists ...` fails: the process ends (14.5 row 20). A C definition has no name field, so its whole subtree is scanned for the name (occurrence 6) once the outer scan (occurrence 5) reaches it; both go one level deeper per nesting level, so fixing occurrence 5 alone leaves this test red.
- **Given** `/src/deep.c` holding `int f(void) { return ` followed by 100 000 `(`, the digit `1`, 100 000 `)` and `; }`
- **When** the member asks for the definition of `f` under `/src`
- **Then** the answer lists exactly one definition: path `/src/deep.c`, name `f`, kind `function_definition`, line 1
- **And** the server is still up

#### E2E-SEC-013: Reference search through the HTTP API over 100 000 nested lists
- **Covers:** occurrence 5, the second door and another language
- **Red before the fix because:** `Then the answer lists ...` fails: the process ends (14.5 row 21).
- **Given** `/src/deep.js` holding `const g = ` followed by 100 000 `[`, `g`, 100 000 `]` and `;`
- **When** a client asks the HTTP API for the references to `g` under `/src`
- **Then** the answer lists exactly two references to `g`, both on line 1
- **And** the server is still up

### 9.4 Modified tests

None. The existing PDF tests draw every form directly from its page and hold no parent chain or character map nesting; the existing code search tests use shallow files whose answers, order included, stay the same (14.1 row 9).

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001 to FR-NEW-003, FR-NEW-007, FR-NEW-009 | E2E-001, E2E-002 | E2E-012 (EXC-001a) | E2E-005, E2E-006, E2E-007, E2E-008, E2E-009 | E2E-001 (companion and audit line) |
| SC-002 | FR-NEW-001 to FR-NEW-003, FR-NEW-005, FR-NEW-006, FR-NEW-009 | E2E-005 (success branch at the limit) | E2E-012 to E2E-017, E2E-SEC-001 to E2E-SEC-010 | E2E-005 | E2E-028 |
| SC-003 | FR-NEW-002, FR-NEW-004 | E2E-001 (success branch) | E2E-018 to E2E-022 | E2E-021 (climb before draws) | E2E-018 (nothing written) |
| SC-004 | FR-NEW-005, FR-NEW-007 | E2E-011 (cached success) | E2E-023, E2E-024 | E2E-011 | E2E-027 |
| SC-005 | FR-NEW-006, FR-NEW-008, FR-NEW-009 | E2E-003, E2E-004 | E2E-025, E2E-026 | E2E-010, E2E-SEC-011 to E2E-SEC-013 | E2E-SEC-011 (bystander served) |

SC-002 and SC-003 are refusal scenarios; their success branch is named.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-005, E2E-008, E2E-012, E2E-015, E2E-016, E2E-022, E2E-SEC-001 to E2E-SEC-004 | 10 |
| FR-NEW-002 | E2E-006, E2E-009, E2E-013, E2E-017, E2E-021, E2E-SEC-005, E2E-SEC-006 | 7 |
| FR-NEW-003 | E2E-007, E2E-014, E2E-SEC-007 to E2E-SEC-010 | 6 |
| FR-NEW-004 | E2E-018, E2E-019, E2E-020, E2E-021, E2E-022 | 5 |
| FR-NEW-005 | E2E-012, E2E-013, E2E-014, E2E-023, E2E-024, E2E-027, E2E-SEC-001 | 7 |
| FR-NEW-006 | E2E-015, E2E-017, E2E-028, E2E-SEC-001 to E2E-SEC-013 | 16 |
| FR-NEW-007 | E2E-001, E2E-002, E2E-005, E2E-006, E2E-007, E2E-008, E2E-009, E2E-011, E2E-024 | 9 |
| FR-NEW-008 | E2E-003, E2E-004, E2E-010, E2E-025, E2E-026, E2E-SEC-011, E2E-SEC-012, E2E-SEC-013 | 8 |
| FR-NEW-009 | E2E-002, E2E-004, E2E-015, E2E-026, E2E-SEC-013 | 5 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.
- How the server guarantees FR-NEW-006: a HOW question for `/sdlc-design` (14.3), does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| PDF | a `.pdf` document, any letter case | document text extraction |
| Form | a reusable drawing a page or another form draws by name | document text extraction |
| Form depth | how many forms are being drawn one inside the other at a draw; the page content is 0 | document text extraction |
| Page group | a node of a PDF's page tree above a page, which can hold resources and a page size its pages inherit | document text extraction |
| Page tree climb | the walk from a page up through its page groups to find its resources or its page size | document text extraction |
| Load | reading a PDF's objects before its pages (SPEC-0089) | document text extraction |
| Drawing order | the order in which a page's content selects fonts and draws forms | document text extraction |
| Page marker | the `*[Page N]*` line before each non empty page's text | document text extraction |
| Preview | the start of the extracted text returned in the answer | document text extraction |
| Resources | the fonts, forms and other named items a page or a form uses | document text extraction |
| Page size | the area a page covers, which every page has or inherits | document text extraction |
| Character map | the table turning a font's codes into text, written in a notation of nested lists, dictionaries and bracketed strings | document text extraction |
| Code map | the table a composite font uses to split its text into codes, in the same notation | document text extraction |
| Composite font | a font whose codes span one or more bytes, described by a code map | document text extraction |
| Nesting | lists, dictionaries, bracketed strings, forms, page groups or source constructs opened inside one another | document text extraction, code search |
| PDF bomb stream | SPEC-0089's compressed stream of under 300 000 stored bytes inflating to 200 000 000 spaces | document text extraction |
| Source file | a project file in a language the code search recognises (Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby) | code search |
| Definition | a named construct the code search reports with its path, name, kind and line | code search |
| Reference | an occurrence of a name the code search reports with its path, line and kind | code search |
| Companion | the Markdown file written next to the document, `/docs/forms.md` for `/docs/forms.pdf` | project storage |
| Refresh | the member's choice to read the document again even when its companion is up to date | document text extraction |
| Artifact | a table or picture kept from a conversion, listed on its own | project storage |
| Session write allowance | the bytes a member can still write in the current session | project storage |
| Audit line | the one line an operator reads to see who extracted what | project storage |
| Nothing left | no companion written or changed, no artifact, no audit line, no charge (Section 9 header) | project storage |
| The server is still up | the bystander's read answered within 1 second by the same server process (Section 9 header) | project storage |
| Refusal | an error answer; the project is left unchanged | document text extraction, code search |
| Member | a person with access to a project | project storage |
| Bystander | another member, in another project, whose requests must keep being answered | project storage |
| Operator | the person who runs the server, restarts it and reads its log and audit trail | project storage |
| Attacker | a person crafting a PDF or a source file to stop the server | document text extraction, code search |
| Assistant tool | the tool an AI assistant calls | document text extraction, code search |
| HTTP API | the web endpoint a program calls; the second door | document text extraction, code search |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 3 | 5 | NOT-IMPLEMENTABLE |
| 2 | 0 | 0 | IMPLEMENTABLE |

**Round 1 findings, all amended in place:** F-1 (G1) FR-NEW-002 left open whether a resources or page size key holding a value of the wrong kind stops the climb; now it is passed over and the climb goes on, as today (`lib.rs:2353-2360`). F-2 (G1) FR-NEW-003 "a map that does not parse within the limit" read two ways; now a map within the limit that cannot be read keeps today's answer. F-3 (G7) the 7.1 bounds had no test; now E2E-SEC-004 asserts 2 seconds and E2E-SEC-011 asserts 10 seconds for 100 000 nested parentheses (the 1 000 000 figure was dropped: no test carried it). A-1 twelve citations recounted (the Office reader at `extract.rs:1296` added to occurrence 13 by the re run of search 5) against the files (`lib.rs:2410`, `:2413`, `:175-180`; `symbols.rs:218-234`, `:225`, `:225-233`; `pdf_bound.rs` break sites `:1506` to `:1557`; `extract.rs:381-387`). A-2 reproduction rows 20 and 21 were assumptions; both run, both abort at 100 000 (14.5). A-3 forbidden modals in Section 2 prose and in FR-NEW-001, FR-NEW-002 statements rewritten. A-4 technical words in Sections 2, 3.2, 9.3 and 11 rewritten in product terms (G4 MINOR). A-5 E2E-028 named three members; now three calls of one member. Glossary completed (load, drawing order, page marker, preview).
**Round 2:** the same scripted checks re run on the amended file: 41 tests summarized and 41 specified, the 9.1 scenario and requirement columns equal to each test's header, every FR covered 5 to 16 times and the Section 10 counts equal to the 9.1 columns, happy 4 against failure 25, no forbidden modal outside verbatim messages, no implementation term in Sections 1 to 13 outside payloads and messages. No finding.
**How the gates ran:** subagents were not allowed for this run, so the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14, G14 searches re run) were run inline by the author against the file on disk. Recorded as a concern in the handoff: the model that chose the class also certified its width.
**No implementation detail outside Section 14:** PASS. Suffixes, verbatim messages, error codes, the option name `max_chars`, language names and the output kind `function_definition` (the member's data and the tool's visible answer, as in SPEC-0006), and the literal attacker payloads of Section 9 (PDF operators, dictionary keys and source text as the attacker writes them) are WHAT, as in SPEC-0088 and SPEC-0089.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

Registry paths below are under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`; versions from `Cargo.lock`: lopdf 0.42.0 (`Cargo.lock:3677-3678`), pdf-extract 0.12.1, adobe-cmap-parser 0.4.1 (pom 1.1.0), tree-sitter with ten grammars (`crates/core/Cargo.toml:109-118`), tokio 1.54.1 (`Cargo.lock:5973-5974`).

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | two doors, PDF routing, SPEC-0089 measure then decode, refusal prefix and no trace, informational log | `crates/core/src/mcp/server.rs:2401-2426` (`fs.extract_text`); `crates/core/src/api/dataplane.rs:171`, `:1487`; `crates/core/src/docs/extract.rs:166` (`.pdf` routing), `:515-534` (`pdf`, `catch_unwind` `:519-523`), `:381-387` (`could not extract`); `crates/core/src/docs/pdf_bound.rs:197-261` (`extract`: `walk` `:211`, `output_doc_page` `:229-236`); `crates/core/src/core/fs_ops.rs:1720-1790`; `crates/core/src/logging.rs:81` (client errors at info) |
| 2 | the pre pass follows draws without limit and skips a form already being drawn | `crates/core/src/docs/pdf_bound.rs:1455-1570` (`walk`, explicit `stack` `:1490-1491`), `"Do"` `:1542-1565`, skip `:1550-1552`; SPEC-0089 design DEC-007 (`sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/sdlc-design.md:539-556`) |
| 3 | the text pass draws forms one inside the other with no limit; a loop or a 400 chain ends the process | `pdf-extract-0.12.1/src/lib.rs:1860-1868` (`Do` calls `process_stream` recursively), `:1581` (`process_stream`); no depth counter in the function (`rg -n depth pdf-extract-0.12.1/src/lib.rs` finds none in `process_stream`); reproduction 14.5 rows 1 to 6 |
| 4 | parent climb with no limit; loop and 10 000 end the process; 3 000 extracted | `pdf-extract-0.12.1/src/lib.rs:2352-2362` (`get_inherited`, recursive), called at `:2410` (resources) and `:2413` (page size, `expect("MediaBox")`); reproduction 14.5 rows 7, 8, 22, 23 |
| 5 | character and code maps parsed with no nesting limit; 1 000 ends the process; 65 read | `pdf-extract-0.12.1/src/lib.rs:920-929` (`get_unicode_map` → `adobe_cmap_parser::get_unicode_map`), `:989-992` (Type0 `Encoding` stream → `get_byte_mapping`); `adobe-cmap-parser-0.4.1/src/lib.rs:61-66` (`nested_literal_string` calls itself), `:112-113` (`dictionary` → `call(value)`), `:129` (`value`), `:145-146` (`array` → `call(value)`); reproduction 14.5 rows 13 to 17 |
| 6 | code search scans with no limit; 10 000 ends the process; 1 000 answered | `crates/core/src/docs/symbols.rs:131-169` (`find_definitions`), `:171-196` (`find_references`), `:207-213` (`walk`, recursive), `:240-247` (`walk_descendants`, recursive), `:218-234` (`name_of`); `crates/core/src/core/fs_ops.rs:1666-1704`; `crates/core/src/mcp/server.rs:2033-2060`; `crates/core/src/api/dataplane.rs:152-153`, `:1184-1207`; reproduction 14.5 rows 18 to 21; tree-sitter itself parses and a cursor walks 1 000 000 levels (row 24) |
| 7 | the whole process ends, nothing contains it, nothing logged | a stack overflow aborts the process (signal 6, `fatal runtime error: stack overflow, aborting`, 14.5); `catch_unwind` catches panics only (`crates/core/src/docs/extract.rs:519`); extraction and search run inside the server process on a runtime worker (`crates/mcp-fs/src/main.rs:6-9` `#[tokio::main]`; `crates/core/src/docs/extract.rs:151-166` async, no separate process); worker stack 2 MiB by default (`tokio-1.54.1/src/runtime/builder.rs:730`) |
| 8 | governing specs | `sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/sdlc-spec.md:196-213` (FR-NEW-007, FR-NEW-009), 3.2 (BL-0147 deferred); `sdlc/specs/SPEC-0006-documents/spec.md:304-319` (FR-023 to FR-025); `sdlc/specs/SPEC-0003-filesystem-engine/spec.md:488-489` (FR-026); `sdlc/specs/SPEC-0088-bound-office-document-part-decoding/sdlc-spec.md` |
| 9 | tests today | 19 tests in `crates/core/src/docs/pdf_bound.rs`, 50 in `crates/core/src/tools/pdf_bound_e2e.rs`, forms built by `crates/core/src/tools/pdf_testkit.rs:786-830` (every form drawn by the page content), the only `/Parent` in the kit at `:303` (`2 0 R`, one level); 22 tests in `crates/core/src/docs/symbols.rs`, 13 in `crates/core/src/tools/search.rs`, `crates/core/src/api/dataplane.rs:2151` (both doors); none on nesting or process survival (`rg -n 'nest\|depth\|cycle' crates/core/src/tools/pdf_bound_e2e.rs crates/core/src/docs/symbols.rs` finds none) |

Test command: `cargo test -p mcp-fs-core --lib docs::pdf_bound`, `cargo test -p mcp-fs-core --lib tools::pdf_bound_e2e`, `cargo test -p mcp-fs-core --lib docs::symbols`, `cargo test -p mcp-fs-core --lib tools::search`, `cargo test -p mcp-fs-core --lib api::dataplane`; full gate `cargo test --workspace`.

### 14.2 Class sweep

**Class:** untrusted structured content (a stored document, or a source file, member supplied) reaching a descent that consumes one level of call stack per level of nesting, with no depth bound (CWE-674, uncontrolled recursion), ending the whole server process.

**Searches run** (scope: `crates/core/src/docs`, the extraction and search entry points, and the parsers they hand member bytes to in the registry):
1. Self call per function body, idiom "a function that calls itself": `python3 -I scan.py crates/core/src/docs <registry>/pdf-extract-0.12.1/src <registry>/adobe-cmap-parser-0.4.1/src`, where `scan.py` prints `file:LINE fn NAME calls itself at LINE` for every `fn NAME` whose body, up to the next `fn`, holds `NAME(` or `self.NAME(` / `Self::NAME(` (the script is 14 lines, kept with the reproduction; the same idea runs as `rg -n 'fn (\w+)' ` then a per body `rg -n '\bNAME\('`). Hits: `docs/pdf_bound.rs:840` (`visit_spaces`), `docs/symbols.rs:207` (`walk`), `docs/symbols.rs:240` (`walk_descendants`), `pdf-extract lib.rs:1581` (`process_stream`), `:2352` (`get_inherited`), `:211`, `:221`, `:231` (`from_obj`); false positives `docs/extract.rs:740` (`office`, a closure parameter), `docs/pdf_bound.rs:1015` and the `new` constructors (another type's `new`).
2. Mutual recursion through combinators, idiom "a parser that calls a parser": `rg -n 'call\(|\.repeat\(' <registry>/adobe-cmap-parser-0.4.1/src/lib.rs`: `:65` (`call(nested_literal_string)`), `:113` (`call(value)`), `:146` (`call(value).repeat(0..)`).
3. Reference following in the reader, idiom "a link followed to another object": `rg -n '"Do"|get_inherited|Parent|dereference|maybe_deref' <registry>/pdf-extract-0.12.1/src/lib.rs <registry>/lopdf-0.42.0/src/document.rs`: `lib.rs:1860-1868`, `:2352-2362`, `:175-180`; lopdf `document.rs:144-156`, `:779-839`.
4. Existing depth guards, idiom "a bound already in place": `rg -n 'MAX_NESTING_DEPTH|MAX_BRACKET|DEPTH_LIMIT|DEREF_LIMIT|remaining_depth' <registry>/lopdf-0.42.0/src <registry>/serde_json-1.0.154/src/de.rs`.
5. Structured readers of member bytes in our code, idiom "a parser of untrusted input": `rg -n 'serde_json::from_(slice|str|reader)|serde_norway::from|read_event|tree_sitter' crates/core/src/docs`.
Not searched, and why: folder trees and git trees (not document or source content, outside `docs/`; side findings, 14.4 D-12); archives (flat entry lists); libgit2 and tantivy (C and indexing internals, other surfaces).

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | form drawn inside form, `pdf-extract lib.rs:1860-1868` → `:1581`, reached from `crates/core/src/docs/pdf_bound.rs:229-236`; the pre pass skips a form already drawn (`pdf_bound.rs:1550-1552`) and so never stops it | vulnerable (originating occurrence of BL-0147) | FR-NEW-001; E2E-SEC-001 to E2E-SEC-004 |
| 2 | parent climb `pdf-extract lib.rs:2352-2362`, from `:2410`, `:2413` | vulnerable | FR-NEW-002; E2E-SEC-005, E2E-SEC-006 |
| 3 | character map parse `adobe-cmap-parser lib.rs:61-66`, `:112-146`, from `pdf-extract lib.rs:929` | vulnerable | FR-NEW-003; E2E-SEC-007 to E2E-SEC-009 |
| 4 | composite font code map parse, same parser, from `pdf-extract lib.rs:992` | vulnerable | FR-NEW-003; E2E-SEC-010 |
| 5 | code search scan `crates/core/src/docs/symbols.rs:207-213` | vulnerable | FR-NEW-008; E2E-SEC-011, E2E-SEC-013 |
| 6 | name search inside a definition `crates/core/src/docs/symbols.rs:240-247`, from `:225` | vulnerable | FR-NEW-008; E2E-SEC-012 |
| 7 | resources climb in the pre pass `crates/core/src/docs/pdf_bound.rs:1349-1362` | safe: a loop with a `seen` set, no recursion (`:1351-1360`) | none |
| 8 | colour space visit `crates/core/src/docs/pdf_bound.rs:840-858` | safe: recursion over one parsed object, whose nesting the loader caps at 100 (`lopdf-0.42.0/src/reader.rs:456` `MAX_NESTING_DEPTH`, enforced at `src/parser/mod.rs:396-401`); reproduced: 100 000 nested arrays or dictionaries in an object load and extract (14.5 rows 10, 11) | none |
| 9 | trailer scan `crates/core/src/docs/pdf_bound.rs:404-420` | safe: a loop with a depth counter | none |
| 10 | page content and form content parse `lopdf Content::decode` | safe: same parser cap (`src/parser/mod.rs:277-281`, `:396-401`); reproduced: 100 000 nested arrays in page content refused as today, `pdf parser failed on this document` (14.5 row 9); 100 000 nested saved states read (row 12) | none |
| 11 | page listing `lopdf-0.42.0/src/document.rs:779-839` | safe: iterative, `PAGE_TREE_DEPTH_LIMIT` 256 (`:787`, `:839`); reproduced: a loop lists 0 pages, a 10 000 chain extracts (14.5 rows 25, 26) | none |
| 12 | reference chains `lopdf document.rs:144-156`; `pdf-extract lib.rs:175-180` (`maybe_deref`, one step); `from_obj` `lib.rs:211-234` | safe: `DEREF_LIMIT` 128 (`document.rs:98`); `from_obj` recursion follows the static type, not the data | none |
| 13 | Office readers `crates/core/src/docs/extract.rs:832`, `:873`, `:913`, `:1115`, `:1159`, `:1225`, `:1268`, `:1296` | safe: event loops over a streaming reader, no self call (search 1) | none |
| 14 | HTML text `crates/core/src/docs/extract.rs:1347-1352` | safe: a single `while` loop over bytes | none |
| 15 | Markdown to document `crates/core/src/docs/docx.rs:74-79` | safe: line based, no self call (search 1) | none |
| 16 | converter JSON `crates/core/src/docs/service.rs:613`, `:744`, `:765` | safe: `serde_json` refuses nesting past 128 (`serde_json-1.0.154/src/de.rs:63`, `:1375`) | none |
| 17 | configuration YAML `crates/core/src/config.rs:962` | out of class: operator file, trusted | none |

### 14.3 Design inputs

- **Where the bound lives (HOW, for /sdlc-design):** the reader recursions (occurrences 1 to 4) are inside third party code with no depth option. Either the SPEC-0089 pre pass refuses before the reader runs (it already walks forms in drawing order, iteratively, and reads the resources climb; it would add the form depth count at `pdf_bound.rs:1542-1565` instead of skipping at `:1550-1552`, a page size climb mirroring `lib.rs:2413`, and a nesting scan of every character and code map it measures, since it already decodes them at `Tf`, SPEC-0089 DEC-007), or the reader is patched or replaced, or the reader runs where exhaustion cannot end the server (a bigger worker stack only moves the 400 and 10 000 thresholds, it cannot stop a loop). Signal: E2E-SEC-001 to E2E-SEC-010 red until the chosen bound holds.
- **Walk and reader parity is now load bearing:** wherever the pre pass stops a branch without refusing (`pdf_bound.rs:1506`, `:1513`, `:1520`, `:1528`, `:1543`, `:1547`, `:1553`, `:1557` `break`) the reader must stop too (it panics there today, contained), otherwise the reader reaches a nesting the pre pass never counted. The climb must use the value the reader would accept: `get_inherited` goes on climbing when the key holds a value of the wrong type (`lib.rs:2353-2360`, `from_obj` returns `None`). Signal: an exploit variant per `break` site; the design names its parity test.
- **Character map nesting count:** the parser counts `(` inside strings unless escaped (`adobe-cmap-parser lib.rs:61-66`, `escape_sequence`), `<<` dictionaries and `[` arrays; `<` hex strings do not nest. Counting must follow the same lexing, comments included. Signal: E2E-007, E2E-014, E2E-SEC-008.
- **Thresholds measured in the dev profile** on a 2 MiB thread (the runtime worker default, `tokio-1.54.1/src/runtime/builder.rs:730`): forms 200 fine, 400 abort; parent chain 3 000 fine, 10 000 abort; character map 65 fine, 1 000 abort; source 1 000 fine, 10 000 abort. Release frames are smaller; the chosen limits (64, 256, 64) sit well under every dev threshold. The design states the worker stack it relies on.
- **Code search fix shape:** an explicit stack or the syntax tree's own cursor walk visits nodes in the same pre order as today (`symbols.rs:207-213`), so answers and their order are unchanged (FR-NEW-008); reproduced: a cursor walk covers 1 000 000 levels in 4.7 s in the dev profile (14.5 row 24). `walk_descendants` collects then visits (`:240-247`); `name_of` keeps the first identifier found (`:225-233`), so the visit order must stay the same.
- **Testability, the tests must not crash the test process:** before the fix every exploit test ends the process that runs it. Each must run the attacked call where an abort is observed, not suffered: the real server binary as a child process driven by a client (the bystander read is then a second request to the same process, the process id read before and after), or the test binary re running itself for one case as a child process, the pattern of `crates/core/src/storage/blob/s3.rs:282-300` (`run_child`, `std::env::current_exe`), asserting the child's exit status and output; a thread with a bounded stack is not enough, because a stack overflow on any thread aborts the whole process. The child must run with the production worker stack (2 MiB) so a regression reproduces. Signal: E2E-SEC-001 red before the fix as a failed assertion, not as a test runner crash.
- **"The server is still up":** needs a second project `orbit` with member `leo@example.com` and `/notes/hello.txt`; the existing office fixture (`crates/core/src/tools/archive_testkit.rs:457-522`, `office_fixture`, `nothing_left`) holds one project; extend it.
- **Order:** FR-NEW-004 places the climb before the page's content decode, as the reader does (`lib.rs:2410-2422`); the pre pass today decodes the content first (`pdf_bound.rs:1468-1484`) and reads resources before it (`:1466`); the page size climb is new to the pre pass.
- **Large fixtures:** E2E-SEC-004 is about 1 700 000 bytes and E2E-SEC-006 about 750 000 bytes (14.5); SPEC-0089's `small()` helper asserts under 1 000 000 bytes (`crates/core/src/tools/pdf_testkit.rs:322-330`), so the chain fixture bypasses it.
- **Docs to update:** `.agent_docs/tools.md` (`fs.extract_text` nesting limits; `fs.find_definition`, `fs.find_references` any nesting), `.agent_docs/architecture.md` (`pdf_bound` walk), `AGENTS.md` (`pdf_bound.rs` limits line), `README.md`. `TOOL_CONTRACT.txt` only if the design mentions the limits in a description (contract change, golden regenerated).

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code, the backlog entry and the reproduction, without the user, under their authorization.
- R1 scope: the three PDF nestings and the code search scan, both doors; out: SPEC-0089 byte bounds, Office, HTML, Markdown, converter answers (safe, 14.2), folder and git trees (D-12), worker thread (BL-0146).
- R1 why now: BL-0147, deferred from SPEC-0089; one file under 2 000 bytes stops the server for every project.
- R2 current state: Section 2 and 14.1, from the code, the three crates' sources, and the reproduction (14.5).
- R3 scenarios: SC-001 to SC-005. R4 requirements: derived, Section 6. R5 NFR: refusal within 2 seconds, deep search within 30 seconds, server up, one log line.
- D-1 routing: the entry reads `Nature: BUG`; the brief and precedent (SPEC-0084, SPEC-0087, SPEC-0088, SPEC-0089) ask for a FEAT spec. Chosen FEAT: new refusals change observable behavior and no spec defines a nesting bound the code violates. Override recorded here.
- D-2 the sweep's two code search occurrences are fixed in this lot rather than deferred: same class, same severity (one file stops the server), small fix, and the project rule forbids filing a backlog entry without the owner. Second bounded context, depth M.
- D-3 security `internal finding` (CWE-674); sweep and exploit tests included.
- D-4 form depth limit 64: ordinary documents nest forms a few levels; 64 sits far under the measured abort (between 200 and 400 in the dev profile) and matches the depth other PDF renderers allow.
- D-5 page tree climb limit 256: the page listing already stops at 256 levels (`lopdf document.rs:787`), so a deeper page is not listed by its tree anyway; a chain of 3 000 extracted today is refused after (behavior change accepted, no natural document has it).
- D-6 limits fixed, not configured: no configuration value bounds extraction today.
- D-7 character map nesting limit 64, the three kinds counted together: real character maps nest 2 or 3 levels; 64 matches the form limit; 65 was measured to work today and is refused after (accepted).
- D-8 a loop counts as passing the limit, with the same message as a long chain: the member sees one cause, nesting, whatever its shape; no separate "loop" message.
- D-9 form depth is checked before the form's stream is measured: nothing is decoded for a draw that is refused.
- D-10 code search gets no limit and no refusal: the syntax tree is built without depth trouble (14.5 row 24), so only the scan changes and today's answers are kept.
- D-11 refusal code `ERR_INVALID_ARGUMENT` with the `could not extract <path>: ` prefix, page number in the form and page tree messages (as SPEC-0089's page total message), none in the character map message (as its stream limit message).
- D-12 folder trees and git trees nested deeply by a member are other surfaces of the same class, not verified; reported to the owner in the handoff, no backlog entry (project rule).
- D-13 tests: 41 new, 0 modified, 0 removed; the test plan is approved by the user's request "specify BL-0147".
- D-14 testability: every exploit test runs the attacked call in a child process (14.3); a bounded stack thread cannot contain an abort.

### 14.5 Reproduction evidence (2026-11-11, before this spec)

Throwaway test in `crates/core/src/docs` (deleted before commit), built with the existing kit (`pages_writer`, `plain`, `PdfWriter`), run once per case in its own process: `REPRO_CASE=<case> target/debug/deps/mcp_fs_core-39a16e4321fb9a7b --exact docs::repro_0147::repro_0147_run --nocapture`, the call made through `Extractor::extract` (the real extraction path: `pdf` → `catch_unwind` → `pdf_bound::extract` → pre pass → `output_doc_page`) or `docs::find_definitions` and `docs::find_references`, on a thread of 2 MiB (the runtime worker default) unless stated. Dev profile. Exit 134 is signal 6 (abort) with `fatal runtime error: stack overflow, aborting`.

| # | Case | File size | Outcome |
|---|---|---|---|
| 1 | form draws itself, own resources | 862 B | exit 134, stack overflow, 1 s; also on an 8 MiB thread (0.09 s) and a 64 MiB thread (0.19 s) |
| 2 | form draws itself through the page's resources | 821 B | exit 134, stack overflow |
| 3 | A draws B draws A | 1 025 B | exit 134, stack overflow |
| 4 | chain of 200 distinct forms | | OK, text `Hello`, `Deep`, 34 ms (20, 50, 100 also OK) |
| 5 | chain of 400, 800, 1 000 forms | 165 526 B (1 000) | exit 134, stack overflow |
| 6 | chain of 10 000 forms | 1 668 549 B | exit 134, stack overflow, 3 s |
| 7 | page parent loop, no resources | 2 505 B | exit 134, stack overflow; also on 8 MiB (0.11 s) and 64 MiB (0.32 s) |
| 8 | parent chain of 10 000, no resources anywhere | 740 825 B | exit 134, stack overflow |
| 9 | 100 000 nested arrays in page content | 200 599 B | `ERR_INVALID_ARGUMENT pdf parser failed on this document` (contained panic), 84 ms |
| 10 | 100 000 nested arrays in an object | 200 688 B | OK, text `Hello`, 3 ms |
| 11 | 100 000 nested dictionaries in an object | 800 689 B | OK, text `Hello`, 4 ms |
| 12 | 100 000 nested `q` / `Q` | 400 596 B | OK, text `Hello`, 2.4 s |
| 13 | character map nested 64 and 65 lists; 64 dictionaries | | OK, text `Hello` |
| 14 | character map nested 1 000 and 10 000 lists | | exit 134, stack overflow |
| 15 | character map 100 000 nested bracketed strings | | exit 134, stack overflow |
| 16 | character map 100 000 nested dictionaries | | exit 134, stack overflow |
| 17 | composite font code map nested 10 lists; 10 000 lists | | 10: `pdf parser failed on this document` (panic at `lib.rs:1006`, after the map is parsed); 10 000: exit 134, stack overflow |
| 18 | Python definition and references, 100 and 1 000 nested parentheses; 10 000 | | 100, 1 000: `defs=1 refs=1`, 11 ms; 10 000: exit 134, stack overflow. Also `total = 1`, `print(total)`, `total += 2`: references on lines 1, 2, 3 in that order; 1 000 nested with a third line `f()`: definition `f` line 1, references lines 1 and 3 |
| 19 | same, 100 000 | | exit 134, stack overflow |
| 20 | C definition `int f(void) { return ((...1...)); }`, 10 and 100 000 nested parentheses | | 10: `[("f", "function_definition", 1)]`, 2 ms; 100 000: exit 134, stack overflow |
| 21 | JavaScript `const g = [[...g...]];` references to `g`, 10 and 100 000 nested lists | | 10: two `identifier` references on line 1, 0 ms; 100 000: exit 134, stack overflow |
| 22 | parent chain of 256 and 3 000, resources and page size at the top | | OK, text `Hello`, 19 ms and 208 ms |
| 23 | parent chain of 10 000, resources and page size at the top | | exit 134, stack overflow |
| 24 | syntax tree of 10 000, 100 000, 1 000 000 nested parentheses parsed, then walked with the tree cursor (no recursion) | | OK, depth 1 000 004, 4.7 s for 1 000 000 |
| 25 | page tree `Kids` loop | 593 B | OK, 0 pages, empty text |
| 26 | page tree `Kids` chain of 10 000 | 830 878 B | OK, text `Hello`, 1.4 s |

Not reproduced: the real server binary over HTTP (the abort above ends any process the call runs in, and the call runs inside the server process, 14.1 row 7), and a release build (thresholds differ, loops do not end).
