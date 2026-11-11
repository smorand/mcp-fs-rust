# Bound PDF stream inflation and the document converter output: functional specification

> Id: SPEC-0089
> Nature: FEAT
> Depth: M
> Depth evidence: 2 bounded contexts (document text extraction for PDF, document conversion through the external converter), 14 requirements, public behavior change (a PDF whose streams, objects or text pass fixed limits is refused instead of being decoded whole; a converter answer past the converter limit is refused instead of being read whole), no persisted change. Override with --depth.
> Status: implemented
> Generated on: 2026-11-10
> Project: existing
> From backlog: BL-0142, BL-0143
> Depends on: none (SPEC-0088 built: its limit numbers, chunk and content limit message are reused)
> Security: internal finding
> CVSS: not scored
> Affected: every build whose text extraction reads `.pdf` documents, and every deployment with the document converter enabled
> Fixed in: unreleased
>
> Note: written non interactively. The user authorized every decision; the request "spec BL-0142 and BL-0143 together" is the explicit approval of the test plan in Section 9. Every default applied is listed in Section 14.4.

## 1. Summary

A member who asks for the text of a PDF gets it as today when the PDF is of a reasonable size, and a clear refusal when one of its compressed streams decodes to more than 104 857 600 bytes, when its compressed object streams together or the streams one page decodes pass 268 435 456 bytes, when it holds more than 1 000 000 objects, or when its extracted text passes 268 435 456 bytes. Today the PDF reader inflates every stream whole, so a PDF of a few hundred kilobytes can make the server hold gigabytes. A member who asks the external converter for a document's Markdown gets it as today when the converter's answer fits the converter limit the operator set, and a refusal naming that limit when the answer passes it, whatever channel the answer comes through; today only the files of a bundle answer are capped, while a text answer, a web answer and the converter's diagnostic output are read whole. Both doors, the assistant tool and the HTTP API, answer alike.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member asks for the text of a `.pdf` document (any letter case) through the assistant tool or the HTTP API. The server reads the stored file whole, hands it to a PDF reader, and builds the text page by page, each non empty page under a `*[Page N]*` marker; it reports the page count and, when present, the document title; a PDF with no text layer answers an empty text with a note. [14.1 row 1]
- The PDF reader decodes every compressed stream it touches whole into memory, with no bound set by the server: the compressed object streams when the document is loaded, each page's content, the font files and character maps of each font a page selects (again on every page), a colour profile every time a page selects a colour space backed by it, and a form or image every time a page draws it, forms drawn inside forms included. Streams in an encoding the reader cannot decode are copied as stored. [row 2]
- The document is loaded a second time to read its title, decoding its compressed object streams again. [row 3]
- Nothing limits the number of objects a PDF holds, objects inside compressed object streams included, nor the number of pages. [row 4]
- The text of every page is built whole and the member's character limit (200 000 by default) is applied only after; one character code can be mapped to a long text and a form can be drawn many times, so the text can grow far beyond the decoded bytes. [row 5]
- A reader failure is refused with `ERR_INVALID_ARGUMENT` and a message starting `could not extract <path>: `; a refusal writes no companion, stores no artifact, charges nothing and records no audit line, and is logged once at the informational level. An up to date companion answers without reading the PDF unless the member asks for a refresh. [row 6]
- When the converter is enabled, a member can ask for the Markdown companion of a stored document (documentize, through both doors), or ask for it while uploading or writing the document (the conversion flag of the upload and of the binary write). The document handed to the converter is capped by the converter limit, 536 870 912 bytes by default, set by the operator; a larger one is refused with `'<path>' is <n> bytes, over the doc_service.max_input_bytes limit of <cap>`. [row 7]
- The converter runs either as a local program, answering on its standard output or by writing a bundle (a Markdown file plus pictures and a manifest) into a folder, or as a web endpoint, answering with Markdown or with a bundle. [row 8]
- What the server reads of the answer: a bundle's files are capped by the converter limit, each file's size checked before it is read, and refused with `document service output is over the doc_service.max_input_bytes limit of <cap>`; the standard output of the local program, its diagnostic output and the web answer body are read whole, with no bound but the converter timeout (900 seconds by default). In bundle mode the standard output is read whole although it is never used. [row 9]
- A failing converter is refused with `ERR_INTERNAL_ERROR`: `document service exited with exit code <n>: <tail>` quoting at most the last 8 192 bytes of its diagnostic output, `doc service timed out after <s>s`, or `document service returned HTTP <status>: <head>` quoting the first 2 048 characters of the body. On timeout the program is stopped before its working folder is removed. [row 10]
- A conversion refused during documentize writes no companion and no artifact and records no audit line; during an upload or a binary write with the flag, the document is still stored and the answer carries the error code and message under its documentation report. Internal errors are logged at the error level. [row 11]

### 2.2 Specs governing this area

- SPEC-0088 bounded the Office readers (part limit 104 857 600 bytes, call limit and content limit 268 435 456 bytes, chunk 65 536 bytes, message `could not extract <path>: the extracted content exceeds 268435456 bytes`) and deferred PDF (BL-0143) and the converter output (BL-0142). This increment reuses its numbers, its chunk and its content limit message; SPEC-0088 is not modified.
- SPEC-0019 DEC-006 defined the bundle answer and its cap by the converter limit. This increment extends that cap to every answer channel; the bundle rule and its message are unchanged.
- SPEC-0041 FR-NEW-005: an Office document with disagreeing parts is refused before the converter sees it. Unchanged; it stays before every conversion refusal of this increment.
- SPEC-0085 set the convention that a decoder stops one chunk of 65 536 bytes past a limit. [row 12]

### 2.3 What is verified today

- PDF extraction is tested for text, pages and title, a corrupt PDF refused without a crash, and a UTF-16 title; the converter is tested for a text answer, sandbox confinement, timeout, failure with the diagnostic tail, empty output, bundle reading, a missing bundle Markdown, the bundle cap, a symlinked output folder, the web answer forms and failure status, and documentize and upload through both doors. [row 13]
- No test covers a PDF stream past a limit, decoded bytes held, objects counted, text multiplication, or a converter answer past the limit on the standard output, the diagnostic output or the web answer. [row 13]

## 3. Scope

### 3.1 In scope

- PDF text extraction, any letter case, both doors, with and without refresh: a limit per decoded stream, on the compressed object streams together, on the streams one page decodes, on the number of objects, on the extracted text; the bound on what the server holds.
- The converter's answer on every channel: standard output in text mode, a web answer of any form, bundle files, diagnostic output, standard output in bundle mode; for documentize, the upload flag and the binary write flag, through both doors.

### 3.2 Out of scope, and why

- The Office readers: bounded by SPEC-0088.
- Archive extraction: bounded by SPEC-0084 and SPEC-0085.
- A form that draws itself, directly or through other forms: an endless recursion that ends the server process, a crash rather than memory held; BL-0147.
- The PDF page tree walked once per page, a cost in time growing with the square of the page count: BL-0149.
- A bundle file that keeps growing after the converter exited, written by a helper the converter left behind: the converter is an operator chosen program; BL-0150.
- The web fetch tool reading a remote page whole: not document extraction; BL-0148.
- PDF extraction running on the request's worker thread: already BL-0146.
- A configurable PDF limit: fixed numbers are enough (14.4 D-5). The converter limit stays the existing operator setting.

### 3.3 Constraints imposed by the context

- No new third party component is added unless the design shows it unavoidable, and no unsafe code (the user's brief); the design states which.

## 4. Actors

- **Member:** a person with access to a project; asks for a document's text or Markdown companion through the assistant tool or the HTTP API.
- **Operator:** runs the server, enables and configures the converter, watches memory and reads the log and audit trail.
- **Attacker:** a member, or someone who got a member to store a file, who crafts a PDF or a document that makes the converter answer huge, to exhaust the server's memory.
- **Converter:** the external program or web endpoint the operator plugs in; it turns a document into Markdown and is not trusted to answer small.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Document text extraction | turning a stored PDF into Markdown text | stream, decoded size, object, page, load, page decode count, extracted content, refusal |
| Document conversion | handing a document to the converter and reading its answer | converter limit, answer, bundle, diagnostic output, stand in converter |
| Project storage | files, companions, artifacts, session write allowance, audit trail | companion, session write allowance, audit line |

## 5. Usage scenarios

### SC-001: Extract the text of a PDF within the limits
**Actor:** Member.
**Preconditions:** project `atlas`, member `mia@example.com`; `/docs/report.pdf` with two pages reading `Quarterly results` and `Outlook`, titled `Report 2026`.
**Flow:**
1. The member asks for its text → the system reads the PDF, every limit holding, and builds the text.
2. The system writes `/docs/report.md`, answers with its path, 2 pages, the title and a preview, and records one audit line.
**Postconditions:** as today.
**Exceptions:**
- EXC-001a: a stream decodes one byte past the stream limit → SC-002.

### SC-002: A PDF passing a limit is refused
**Actor:** Member or attacker.
**Preconditions:** `/docs/bomb.pdf`, stored in under 1 000 000 bytes, whose page 1 content stream inflates to 200 000 000 bytes.
**Flow:**
1. The member asks for its text → decoding stops one chunk past 104 857 600 bytes and the call is refused.
**Postconditions:** no companion written or changed; no artifact; the session write allowance unchanged; no audit line; the decoded bytes held never exceed 104 923 136.
**Exceptions:**
- EXC-002a: the compressed object streams together pass 268 435 456 bytes → refused on the load total.
- EXC-002b: the streams one page decodes, repeats counted, pass 268 435 456 bytes → refused naming the page.
- EXC-002c: more than 1 000 000 objects → refused on the object count.
- EXC-002d: the extracted text passes 268 435 456 bytes → refused on the content.

### SC-003: Order when several refusals apply to a PDF
**Actor:** Member.
**Flow:**
1. Membership and a missing path come first, as today.
2. Then the load: objects outside compressed object streams are counted; then each compressed object stream in increasing object number is decoded (stream limit, then load total) and its objects counted (object limit). A document that cannot be loaded is refused with today's message where loading fails.
3. Then pages in page order; within a page each decode in drawing order (stream limit, then page total); the content limit as the text grows.
4. Then the title read, under the same limits.

### SC-004: Repeat, refresh and the stored companion of a PDF
**Actor:** Member, operator.
**Flow:**
1. A refused call repeated gives the same refusal and writes nothing.
2. With an up to date companion and no refresh, the stored text is returned without reading the PDF.
3. With a refresh, the PDF is read again and refused if it passes a limit; the stored companion is left as is.
4. The operator finds one informational log line per refused call and no audit line.

### SC-005: Convert a document with an answer within the converter limit
**Actor:** Member.
**Preconditions:** converter enabled, converter limit 1 048 576 bytes; `/docs/deck.pptx` stored.
**Flow:**
1. The member asks for its Markdown companion → the converter answers 2 000 bytes of Markdown (text mode, bundle mode or web endpoint).
2. The system writes `/docs/deck.md`, keeps the bundle's pictures as artifacts, answers with the path and bytes written.
**Postconditions:** as today.
**Exceptions:**
- EXC-005a: the answer passes the converter limit → SC-006.
- EXC-005b: the converter fails, times out or answers a failure status → refused as today.

### SC-006: A converter answer past the converter limit is refused
**Actor:** Attacker, converter.
**Preconditions:** `/docs/bomb.docx`, which makes the converter answer 1 073 741 824 bytes of Markdown.
**Flow:**
1. The member asks for its companion → the system reads the answer up to one chunk past the converter limit, stops, ends the converter's work (the local program is stopped, the web answer is abandoned) and removes its working folder.
2. The system refuses with `document service output is over the doc_service.max_input_bytes limit of 1048576`.
**Postconditions:** documentize: no companion, no artifact, no charge, no audit line. Upload or binary write with the flag: the document is stored, the documentation report carries `ERR_INTERNAL_ERROR` and the message, no companion.
**Exceptions:**
- EXC-006a: the converter floods its diagnostic output then fails → refused as today with the last 8 192 bytes quoted, the server holding no more than 73 728 bytes of it.
- EXC-006b: in bundle mode the converter floods its standard output, which is not its answer → the standard output is discarded as it comes; the conversion is judged on its bundle.
- EXC-006c: the web endpoint answers a failure status with a huge body → refused as today with the head of the body, the server reading at most one chunk past the converter limit.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-O]: Object limit
> IF a PDF holds more than 1 000 000 objects THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the PDF holds more than 1000000 objects`.
- **Business rules:** objects stored inside compressed object streams count. Exactly 1 000 000 is accepted. `<path>` is the document's project path as given, normalized, as in every extraction message.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-002 [EARS-O]: Stream limit
> IF a stream of a PDF decodes to more than 104 857 600 bytes THEN THE document text extraction SHALL stop decoding it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: a PDF stream decodes to more than 104857600 bytes`.
- **Business rules:** applies to every decode, whatever its purpose (object stream, page content, font file, character map, colour profile, form, image) and whatever pass (text or title). Exactly 104 857 600 is accepted. Decoding stops at most one chunk of 65 536 bytes past the limit. A stream in an encoding the extraction does not decode counts 0 here and in FR-NEW-003 and FR-NEW-004: it is not decoded.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-003 [EARS-O]: Load total
> IF the compressed object streams of a PDF decode to more than 268 435 456 bytes together THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the PDF object streams decode to more than 268435456 bytes together`.
- **Business rules:** each compressed object stream counts once per load; equal is accepted; FR-NEW-002 is checked first for the stream being decoded.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-O]: Page total
> IF the streams one page decodes total more than 268 435 456 bytes THEN THE document text extraction SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: page <N> decodes more than 268435456 bytes of PDF streams`.
- **Business rules:** `<N>` is the page number, from 1, in plain digits. Counted for that page: its content streams once; a font's file and character maps the first time the page selects that font name; a colour profile every time the page selects a colour space backed by it; a form or an image every time the page draws it, forms drawn inside forms included. The count starts again at 0 on each page. Equal is accepted. FR-NEW-002 is checked first for the stream being decoded.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-005 [EARS-O]: Content limit for a PDF
> IF the extracted content of a PDF passes 268 435 456 bytes THEN THE document text extraction SHALL stop building it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: the extracted content exceeds 268435456 bytes`.
- **Business rules:** the extracted content is the text of every page with its page markers, before the member's character cut, in bytes. Equal is accepted. The message is SPEC-0088's, verbatim.
- **User visible text:** as above.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-006 [EARS-U]: Bound on what PDF extraction holds
> THE document text extraction SHALL hold, during one PDF call, at most 536 936 448 bytes of decoded stream data and at most 268 500 992 bytes of extracted content.
- **Business rules:** 536 936 448 is the load total plus one page total plus one chunk; 268 500 992 is the content limit plus one chunk. A copy of decoded data the extraction keeps (a colour space kept in a saved drawing state, for instance) counts as held. A call refused before its first decoded byte holds 0. The bound is per call.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Order of PDF refusals
> IF several refusals apply to one text extraction of a PDF THEN THE document text extraction SHALL report the first met in this order: (1) the caller is not a member (`ERR_FORBIDDEN`, `'<person>' is not a member of '<project>'`); (2) the path is not a file (`ERR_NOT_FOUND`, `not a file: <path>`); (3) the load: objects outside compressed object streams counted against FR-NEW-001, then each compressed object stream in increasing object number decoded against FR-NEW-002 then FR-NEW-003, its objects then counted against FR-NEW-001; a document that cannot be loaded is refused where loading fails with today's message; (4) pages in page order, each decode of a page in drawing order against FR-NEW-002 then FR-NEW-004, and FR-NEW-005 at the moment the content crosses its limit; (5) the title read, under the same limits.
- **Business rules:** messages of items 1 and 2 and of a document that cannot be loaded are unchanged.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-008 [EARS-UB]: A PDF refusal leaves no trace
> THE document text extraction SHALL NOT write or change a companion, store an artifact, charge the session write allowance or the project's storage, or record an audit line, for a call refused under FR-NEW-001 to FR-NEW-005.
- **Business rules:** an existing companion, stale or up to date, keeps its bytes. The refusal is logged once at the informational level.
- **Scenario:** SC-002, SC-004
- **Priority:** Must

#### FR-NEW-009 [EARS-U]: PDF extraction is otherwise unchanged
> THE document text extraction SHALL answer a PDF within every limit with the same text, page markers, page count, title, note, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading the PDF when no refresh is asked, and SHALL extract every other format exactly as before.
- **Business rules:** includes today's refusal of a PDF that cannot be read (`ERR_INVALID_ARGUMENT`, message starting `could not extract <path>: `) and the note for a PDF without a text layer.
- **Scenario:** SC-001, SC-004
- **Priority:** Must

#### FR-NEW-010 [EARS-O]: Converter answer limit
> IF the converter's answer passes the converter limit THEN THE document conversion SHALL stop reading it, end the converter's work, remove its working folder, and refuse with `ERR_INTERNAL_ERROR` and the message `document service output is over the doc_service.max_input_bytes limit of <cap>`.
- **Business rules:** the answer is the standard output in text mode, the body of the web answer whatever its form (Markdown, bundle, named field), or the bundle's files together (unchanged rule). `<cap>` is the converter limit in plain digits, the value the operator set (536 870 912 by default). Equal is accepted. Reading stops at most one chunk of 65 536 bytes past the limit. The local program is stopped and its end awaited before its working folder is removed; the web answer is abandoned. The message is today's bundle message, verbatim.
- **User visible text:** as above.
- **Scenario:** SC-006
- **Priority:** Must

#### FR-NEW-011 [EARS-U]: What conversion holds
> THE document conversion SHALL hold, during one conversion, at most the converter limit plus 65 536 bytes of the converter's answer, at most 73 728 bytes of its diagnostic output, and at most 65 536 bytes of its standard output in bundle mode.
- **Business rules:** 73 728 is the 8 192 bytes quoted on failure plus one chunk. In bundle mode the standard output is discarded as it comes and does not count against the converter limit; its flood is bounded in time by the converter timeout, as today. A web answer with a failure status is read no further than the converter limit plus one chunk.
- **Scenario:** SC-006
- **Priority:** Must

#### FR-NEW-012 [EARS-O]: Order of conversion refusals
> IF several refusals apply to one conversion THEN THE document conversion SHALL report the first met in this order: (1) membership; (2) not a file (documentize); (3) converter not configured or extension not accepted (`ERR_NOT_SUPPORTED`, today's messages); (4) the document over the converter limit (today's message); (5) the SPEC-0041 Office check; (6) the companion exists and overwrite is not asked (documentize, `ERR_NO_CLOBBER`); (7) during the conversion, whichever comes first: the answer passing the converter limit (FR-NEW-010), the timeout; then, once the converter ended within the limit, its failure exit, a web failure status, an empty answer or a bundle without its Markdown, with today's messages.
- **Business rules:** a web failure status is known before its body is read and is reported with today's message, even when its body passes the converter limit. Items 1 to 6 are unchanged.
- **Scenario:** SC-006
- **Priority:** Must

#### FR-NEW-013 [EARS-UB]: A conversion refusal leaves no companion
> THE document conversion SHALL NOT write or change a companion, store an artifact, charge the session write allowance for a companion, or record an audit line for a companion, for a conversion refused under FR-NEW-010.
- **Business rules:** documentize answers the refusal. An upload or binary write with the conversion flag still stores the document, as today, and its documentation report carries `ERR_INTERNAL_ERROR` and the message; the document's own write charge and audit line are as today. A documentize refusal is logged once at the error level, as every internal error; an upload or write that succeeds with a failed conversion logs as today.
- **Scenario:** SC-006
- **Priority:** Must

#### FR-NEW-014 [EARS-U]: Both doors, and the rest unchanged
> THE product SHALL answer a PDF extraction or a conversion through the HTTP API with the same code, message and effects as through the assistant tool, and SHALL answer a conversion within the converter limit with the same companion, artifacts, report, charge, audit line and failure messages as before.
- **Business rules:** the failure messages kept: `document service exited with exit code <n>: <tail>`, `doc service timed out after <s>s`, `document service returned HTTP <status>: <head>`, the empty answer and the missing bundle Markdown messages.
- **Scenario:** SC-001, SC-005
- **Priority:** Must

### 6.2 Modified

None. No spec defines a bound on the PDF reader or on the converter's text, web or diagnostic answer; the bundle cap of SPEC-0019 DEC-006 keeps its rule and message (2.2).

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
- WHEN a converter answer passes the converter limit THE document conversion SHALL refuse without waiting for the converter to finish or for the timeout.

### 7.2 Security and access
- THE product SHALL require the same project membership as before for extraction and conversion.
- THE document text extraction SHALL NOT let a PDF's streams, objects or character maps make the server hold more than FR-NEW-006 states; THE document conversion SHALL NOT let a converter's answer make it hold more than FR-NEW-011 states.

### 7.5 Reliability
- IF a call is refused under this specification THEN THE product SHALL leave no converter process running and no working folder behind.

### 7.6 Operability
- WHEN a PDF call is refused under FR-NEW-001 to FR-NEW-005 THE document text extraction SHALL leave exactly one informational log line naming `ERR_INVALID_ARGUMENT` and the message, and no audit line.
- THE product SHALL make the decoded bytes held, the content held, the converter answer held and the diagnostic output held of one call measurable by the verification of this specification (the measurement hook of Section 9).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | PDF without text layer; converter answering nothing; no compressed object stream | note as today; empty answer message as today; load total 0, extraction as today | E2E-006, E2E-047, E2E-006 |
| Boundary values | stream at 104 857 600 and one over; load total, page total, object count, content and converter limit each at and one over | accepted; refused | E2E-007 to E2E-016, E2E-033 to E2E-038 |
| Concurrent actions | two PDF bombs and a normal PDF at once; two converter floods at once | bombs refused within the bound, normal succeeds | E2E-031, E2E-046 |
| Slow or lost connectivity | converter slower than the timeout; web endpoint stalls mid answer | timeout message as today | E2E-044 |
| Access states | not a member | membership refusal first | E2E-024, E2E-048 |
| Data variations | upper case `.PDF`; character map to long accented text; stream in an undecoded encoding | refused alike; content counted in bytes; not counted | E2E-029, E2E-SEC-011, E2E-017 |
| Navigation | refresh asked on a bomb with companion; up to date companion without refresh | refused, companion kept; stored text returned | E2E-021, E2E-020 |
| Partial operations | refused call repeated; converter stopped mid answer | same refusal; no process, no folder | E2E-022, E2E-SEC-013 |
| Dependency failures | converter exits with failure, times out, answers a failure status with a huge body | today's messages; bounded read | E2E-042, E2E-044, E2E-043 |
| Ordering and timing | load before pages; page 1 content limit before page 2 bomb; failure status before body limit; input limit before conversion | FR-NEW-007, FR-NEW-012 | E2E-026 to E2E-028, E2E-043, E2E-045 |
| Exhaustion | each PDF stream kind as a bomb; forms and colour profiles repeated; 2 000 000 objects; text multiplied; converter floods on every channel | refused within the bound | E2E-SEC-001 to E2E-SEC-017 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. Fixture files and the stand in converter are built by the design. Unless a test says otherwise: project `atlas`, member `mia@example.com`, the call goes through the assistant tool with default options (`max_chars` 200 000, no refresh), the session write allowance is 1 000 000 000 bytes with nothing written yet, the project has no storage limit, and the converter, when used, is enabled with a converter limit of 1 048 576 bytes and a timeout of 30 seconds. A "stand in converter" is a converter built by the tests whose answer is described in the test, in the mode named (text, bundle or web). "Decoded bytes held" is the largest amount of decoded PDF stream data the server holds at once during the call, "content held" the largest extracted content it holds, "answer held" the largest amount of the converter's answer it holds, "diagnostic held" the largest amount of the converter's diagnostic output it holds, and "bundle mode output held" the largest amount of standard output it holds in bundle mode; all are read by the operator's measurement hook. A "PDF bomb stream" is a compressed stream of under 300 000 stored bytes inflating to exactly 200 000 000 bytes of spaces, its PDF stored in under 1 000 000 bytes. "Nothing left" means: no file `/docs/<name>.md` appears in the listing of `/docs` (or an existing one keeps its bytes), no table or image artifact is listed for the document, the audit trail has no extraction or conversion line for it, a following write of a 52 428 800 byte file `/docs/fill.bin` by the same member succeeds, no converter process started by the call is still running, and the server's temporary folder holds no working folder of the call.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-009 | High |
| E2E-002 | New | happy | SC-001 | FR-NEW-009, FR-NEW-014 | High |
| E2E-003 | New | happy | SC-005 | FR-NEW-014 | High |
| E2E-004 | New | happy | SC-005 | FR-NEW-014 | High |
| E2E-005 | New | happy | SC-005 | FR-NEW-014 | High |
| E2E-006 | New | happy | SC-001 | FR-NEW-009 | Medium |
| E2E-007 | New | edge | SC-001 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-008 | New | failure | SC-002 | FR-NEW-002, FR-NEW-006, FR-NEW-008 | Critical |
| E2E-009 | New | edge | SC-001 | FR-NEW-003 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-003, FR-NEW-006 | High |
| E2E-011 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-012 | New | failure | SC-002 | FR-NEW-001 | High |
| E2E-013 | New | edge | SC-001 | FR-NEW-004 | High |
| E2E-014 | New | failure | SC-002 | FR-NEW-004, FR-NEW-006 | High |
| E2E-015 | New | edge | SC-001 | FR-NEW-005 | High |
| E2E-016 | New | failure | SC-002 | FR-NEW-005, FR-NEW-006 | High |
| E2E-017 | New | edge | SC-001 | FR-NEW-002, FR-NEW-009 | Medium |
| E2E-018 | New | failure | SC-002 | FR-NEW-004 | Medium |
| E2E-019 | New | failure | SC-001 | FR-NEW-009 | Medium |
| E2E-020 | New | edge | SC-004 | FR-NEW-009 | High |
| E2E-021 | New | failure | SC-004 | FR-NEW-008, FR-NEW-009 | High |
| E2E-022 | New | failure | SC-004 | FR-NEW-008 | Medium |
| E2E-023 | New | failure | SC-002 | FR-NEW-002, FR-NEW-014 | Critical |
| E2E-024 | New | failure | SC-003 | FR-NEW-007 | Medium |
| E2E-025 | New | failure | SC-003 | FR-NEW-007 | Medium |
| E2E-026 | New | failure | SC-003 | FR-NEW-001, FR-NEW-007 | Medium |
| E2E-027 | New | failure | SC-003 | FR-NEW-003, FR-NEW-007 | Medium |
| E2E-028 | New | failure | SC-003 | FR-NEW-005, FR-NEW-007 | Medium |
| E2E-029 | New | failure | SC-002 | FR-NEW-002 | Medium |
| E2E-030 | New | side effect | SC-004 | FR-NEW-008 | Medium |
| E2E-031 | New | edge | SC-002 | FR-NEW-006, FR-NEW-009 | Medium |
| E2E-032 | New | failure | SC-003 | FR-NEW-007 | Medium |
| E2E-033 | New | edge | SC-005 | FR-NEW-010 | High |
| E2E-034 | New | failure | SC-006 | FR-NEW-010, FR-NEW-011, FR-NEW-013 | Critical |
| E2E-035 | New | edge | SC-005 | FR-NEW-010 | High |
| E2E-036 | New | failure | SC-006 | FR-NEW-010, FR-NEW-011 | Critical |
| E2E-037 | New | edge | SC-005 | FR-NEW-010, FR-NEW-014 | High |
| E2E-038 | New | failure | SC-006 | FR-NEW-010 | High |
| E2E-039 | New | side effect | SC-006 | FR-NEW-013 | Critical |
| E2E-040 | New | failure | SC-006 | FR-NEW-013, FR-NEW-014 | High |
| E2E-041 | New | failure | SC-006 | FR-NEW-010, FR-NEW-014 | High |
| E2E-042 | New | failure | SC-005 | FR-NEW-012, FR-NEW-014 | Medium |
| E2E-043 | New | failure | SC-006 | FR-NEW-011, FR-NEW-012 | High |
| E2E-044 | New | failure | SC-005 | FR-NEW-012, FR-NEW-014 | Medium |
| E2E-045 | New | failure | SC-006 | FR-NEW-012 | Medium |
| E2E-046 | New | side effect | SC-006 | FR-NEW-011, FR-NEW-013 | Medium |
| E2E-047 | New | failure | SC-005 | FR-NEW-012, FR-NEW-014 | Medium |
| E2E-048 | New | failure | SC-006 | FR-NEW-012 | Medium |
| E2E-049 | New | failure | SC-006 | FR-NEW-012, FR-NEW-013 | Medium |
| E2E-SEC-001 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006, FR-NEW-008 | Critical |
| E2E-SEC-002 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-003 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-004 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-005 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-006 | New | failure, security | SC-002 | FR-NEW-002, FR-NEW-006 | Critical |
| E2E-SEC-007 | New | failure, security | SC-002 | FR-NEW-004, FR-NEW-006 | Critical |
| E2E-SEC-008 | New | failure, security | SC-002 | FR-NEW-004, FR-NEW-006 | Critical |
| E2E-SEC-009 | New | failure, security | SC-002 | FR-NEW-003, FR-NEW-006 | Critical |
| E2E-SEC-010 | New | failure, security | SC-002 | FR-NEW-001, FR-NEW-006 | Critical |
| E2E-SEC-011 | New | failure, security | SC-002 | FR-NEW-005, FR-NEW-006 | Critical |
| E2E-SEC-012 | New | failure, security | SC-003 | FR-NEW-002, FR-NEW-007 | High |
| E2E-SEC-013 | New | failure, security | SC-006 | FR-NEW-010, FR-NEW-011, FR-NEW-013 | Critical |
| E2E-SEC-014 | New | failure, security | SC-006 | FR-NEW-011, FR-NEW-014 | Critical |
| E2E-SEC-015 | New | failure, security | SC-006 | FR-NEW-010, FR-NEW-011 | Critical |
| E2E-SEC-016 | New | failure, security | SC-006 | FR-NEW-010, FR-NEW-011 | Critical |
| E2E-SEC-017 | New | edge, security | SC-006 | FR-NEW-011, FR-NEW-014 | Critical |

Coverage (66 tests: 66 new, 0 modified, 0 removed): happy 6, failure 45 (16 of them also security), side effect 3, edge 12 (1 of them also security), state transition 0 (no state machine: a document is extracted or converted, or refused; nothing persists between calls). Happy to failure ratio 1:7.5.

### 9.2 New tests

#### E2E-001: PDF within the limits
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/docs/report.pdf`, two pages reading `Quarterly results` and `Outlook`, title `Report 2026`, every stream compressed
- **When** the member asks for its text
- **Then** the answer names `/docs/report.md`, format `pdf`, 2 pages, title `Report 2026`, not cached; its preview reads `*[Page 1]*`, `Quarterly results`, `*[Page 2]*`, `Outlook`, in that order
- **And** `/docs/report.md` holds that text; the audit trail has one extraction line for `/docs/report.md`

#### E2E-002: PDF through the HTTP API
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009, FR-NEW-014; **Priority:** High
- **Given** the document of E2E-001
- **When** a client asks the HTTP API for its text
- **Then** the answer carries the same companion path, format, page count, title and preview as E2E-001
- **And** the audit trail has one extraction line for `/docs/report.md`

#### E2E-003: Text mode conversion within the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** `/docs/deck.pptx` and a text mode stand in converter answering `# Deck` followed by 1 990 bytes of the letter `d`
- **When** the member asks for its companion
- **Then** the answer names `/docs/deck.md` with 2 000 bytes written, not overwritten
- **And** `/docs/deck.md` holds that answer; the audit trail has one line for `/docs/deck.md`

#### E2E-004: Bundle mode conversion within the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** a bundle mode stand in converter writing `document.md` with `# Deck` and `![Chart](figures/f1.png)`, a 500 byte picture `figures/f1.png` and a manifest captioning it `Chart`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` holds the Markdown, and one image artifact captioned `Chart` is listed for `/docs/deck.pptx`

#### E2E-005: Web endpoint conversion within the limit
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-014; **Priority:** High
- **Given** a web stand in converter answering a bundle with the Markdown `# Deck` and no file
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` holds `# Deck`; the answer reports 6 bytes written

#### E2E-006: PDF without a text layer
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/scan.pdf`, one page whose content draws nothing, and no compressed object stream
- **When** the member asks for its text
- **Then** the text is empty, 1 page, with the note `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`

#### E2E-007: Stream of exactly the stream limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** Critical
- **Given** `/docs/large.pdf` whose page 1 content stream inflates to exactly 104 857 600 bytes, showing the letter `b` repeated, then spaces
- **When** the member asks for its text
- **Then** the extraction succeeds; the answer reports the text as truncated, its preview made of the letter `b`
- **And** the decoded bytes held never exceed 104 923 136 plus the load's decoded object streams (under 10 000 bytes)

#### E2E-008: Stream one byte over the stream limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-006, FR-NEW-008; **Priority:** Critical
- **Given** `/docs/large.pdf` as in E2E-007 but inflating to 104 857 601 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/large.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-009: Object streams totalling exactly the load total
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-003; **Priority:** High
- **Given** `/docs/objs.pdf` with three compressed object streams inflating to 100 000 000, 100 000 000 and 68 435 456 bytes (objects padded with spaces), one page reading `Fine`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview reads `*[Page 1]*` then `Fine`

#### E2E-010: Object streams one byte over the load total
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** `/docs/objs.pdf` as in E2E-009 with its third object stream inflating to 68 435 457 bytes
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/objs.pdf: the PDF object streams decode to more than 268435456 bytes together`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-011: Exactly 1 000 000 objects
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/docs/many.pdf` holding exactly 1 000 000 objects in total, the padding objects being `null` objects in compressed object streams, one page reading `Many`
- **When** the member asks for its text
- **Then** the extraction succeeds; the preview reads `*[Page 1]*` then `Many`

#### E2E-012: 1 000 001 objects
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-001; **Priority:** High
- **Given** `/docs/many.pdf` as in E2E-011 with one more `null` object
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/many.pdf: the PDF holds more than 1000000 objects`
- **And** nothing left

#### E2E-013: A page decoding exactly the page total
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-004; **Priority:** High
- **Given** `/docs/form.pdf` whose page 1 draws one form 4 times, the page content and the four draws of the form decoding exactly 268 435 456 bytes together, the form showing `Hi`
- **When** the member asks for its text
- **Then** the extraction succeeds and the text holds `Hi`

#### E2E-014: A page one byte over the page total
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004, FR-NEW-006; **Priority:** High
- **Given** `/docs/form.pdf` as in E2E-013 with one more byte in the page content
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/form.pdf: page 1 decodes more than 268435456 bytes of PDF streams`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-015: Content of exactly the content limit
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-005; **Priority:** High
- **Given** `/docs/text.pdf` whose pages, through a character map turning one code into 256 letters `e`, produce exactly 268 435 456 bytes of extracted content, page markers included
- **When** the member asks for its text
- **Then** the extraction succeeds; the answer reports the text as truncated at 200 000 characters

#### E2E-016: Content one byte over the content limit
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-005, FR-NEW-006; **Priority:** High
- **Given** `/docs/text.pdf` as in E2E-015 producing one more byte
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/text.pdf: the extracted content exceeds 268435456 bytes`
- **And** the content held never exceeds 268 500 992; nothing left

#### E2E-017: A stream the extraction does not decode
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-009; **Priority:** Medium
- **Given** `/docs/photo.pdf` whose page 1 shows `Caption` and draws, 200 times, a 900 000 byte image in an encoding the extraction does not decode
- **When** the member asks for its text
- **Then** the answer, success or refusal, is identical to the one the server gives before this change for the same file, recorded by the design before the change; it is not one of the five limit refusals of this specification
- **And** the decoded bytes held never exceed 10 000 (the image is not decoded, so it is not counted)

#### E2E-018: Page 2 over the page total, page 1 fine
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-004; **Priority:** Medium
- **Given** `/docs/two.pdf` whose page 1 reads `One` and whose page 2 draws a form decoding 1 000 000 bytes 300 times
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/two.pdf: page 2 decodes more than 268435456 bytes of PDF streams`

#### E2E-019: A PDF that cannot be read
- **Category:** failure; **Scenario:** SC-001; **Requirements:** FR-NEW-009; **Priority:** Medium
- **Given** `/docs/bad.pdf` holding `%PDF-1.4` then the word `garbage`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with a message starting `could not extract /docs/bad.pdf: `, as today
- **And** nothing left

#### E2E-020: Up to date companion answers without reading a PDF bomb
- **Category:** edge; **Scenario:** SC-004; **Requirements:** FR-NEW-009; **Priority:** High
- **Given** `/docs/bomb.pdf` whose page 1 content is a PDF bomb stream, and `/docs/bomb.md` holding `kept text`, written after it
- **When** the member asks for the text of `/docs/bomb.pdf` without refresh
- **Then** the answer is cached, names `/docs/bomb.md` and previews `kept text`
- **And** the decoded bytes held are 0

#### E2E-021: Refresh on a PDF bomb keeps the companion
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-008, FR-NEW-009; **Priority:** High
- **Given** the situation of E2E-020
- **When** the member asks for the text of `/docs/bomb.pdf` with refresh
- **Then** the refusal is the stream limit message for `/docs/bomb.pdf`
- **And** `/docs/bomb.md` still reads `kept text`

#### E2E-022: Repeating a refused PDF call
- **Category:** failure; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/docs/bomb.pdf` of E2E-020 without companion
- **When** the member asks for its text twice
- **Then** both answers are the same stream limit refusal; nothing left after the second

#### E2E-023: PDF bomb through the HTTP API
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002, FR-NEW-014; **Priority:** Critical
- **Given** `/docs/bomb.pdf` of E2E-022
- **When** a client asks the HTTP API for its text
- **Then** the refusal carries `ERR_INVALID_ARGUMENT` and `could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-024: Not a member, PDF bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/bomb.pdf` of E2E-022 and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its text
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- **And** the decoded bytes held are 0

#### E2E-025: Missing path
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** no file `/docs/none.pdf`
- **When** the member asks for its text
- **Then** the refusal is `ERR_NOT_FOUND` with `not a file: /docs/none.pdf`

#### E2E-026: Object count met before a later object stream bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-001, FR-NEW-007; **Priority:** Medium
- **Given** `/docs/order1.pdf` whose object stream number 5 holds 1 000 001 `null` objects within 10 000 000 decoded bytes and whose object stream number 9 is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the object limit message for `/docs/order1.pdf`

#### E2E-027: Load total met before a page bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-003, FR-NEW-007; **Priority:** Medium
- **Given** `/docs/order2.pdf` with the object streams of E2E-010 and a page 1 drawing a form decoding 1 000 000 bytes 300 times
- **When** the member asks for its text
- **Then** the refusal is the load total message for `/docs/order2.pdf`

#### E2E-028: Content limit on page 1 before a page 2 bomb
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-005, FR-NEW-007; **Priority:** Medium
- **Given** `/docs/order3.pdf` whose page 1 produces 268 435 457 bytes of content through the character map of E2E-015 and whose page 2 content is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the content limit message for `/docs/order3.pdf`

#### E2E-029: Upper case suffix
- **Category:** failure; **Scenario:** SC-002; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/docs/BOMB.PDF`, the bytes of `/docs/bomb.pdf` of E2E-022
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/BOMB.PDF`

#### E2E-030: What the operator sees for a PDF refusal
- **Category:** side effect; **Scenario:** SC-004; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/docs/bomb.pdf` of E2E-022
- **When** the member asks for its text
- **Then** the server log has exactly one informational line for the call naming `ERR_INVALID_ARGUMENT` and `a PDF stream decodes to more than 104857600 bytes`, and no error level line
- **And** the audit trail has no line for `/docs/bomb.pdf` or `/docs/bomb.md`

#### E2E-031: Two PDF bombs and a normal PDF at once
- **Category:** edge; **Scenario:** SC-002; **Requirements:** FR-NEW-006, FR-NEW-009; **Priority:** Medium
- **Given** members `mia@example.com` and `noa@example.com`, each with a copy of `/docs/bomb.pdf` of E2E-022 named `/docs/m.pdf` and `/docs/n.pdf`, and `ola@example.com` with `/docs/report.pdf` of E2E-001
- **When** the three ask for their texts at the same time
- **Then** the two bombs are refused with the stream limit message and the report succeeds as in E2E-001
- **And** each bomb call's decoded bytes held never exceed 104 933 136

#### E2E-032: Title read under the same limits
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-007; **Priority:** Medium
- **Given** `/docs/titled.pdf` of E2E-001 whose information dictionary, holding the title, sits in a compressed object stream that is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message for `/docs/titled.pdf`; no title is reported

#### E2E-033: Text answer of exactly the converter limit
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** a text mode stand in converter answering `#` then 1 048 575 letters `d`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` is written with 1 048 576 bytes

#### E2E-034: Text answer one byte over the converter limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010, FR-NEW-011, FR-NEW-013; **Priority:** Critical
- **Given** the stand in converter of E2E-033 answering one more letter
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; nothing left

#### E2E-035: Web answer of exactly the converter limit
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** a web stand in converter answering, as plain Markdown, `#` then 1 048 575 letters `d`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` is written with 1 048 576 bytes

#### E2E-036: Web answer one byte over the converter limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010, FR-NEW-011; **Priority:** Critical
- **Given** the web stand in converter of E2E-035 answering one more letter
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; nothing left

#### E2E-037: Bundle of exactly the converter limit
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-010, FR-NEW-014; **Priority:** High
- **Given** a bundle mode stand in converter writing `document.md` with `# Deck` (6 bytes) and `big.bin` of 1 048 570 bytes
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** `/docs/deck.md` holds `# Deck`

#### E2E-038: Bundle one byte over the converter limit, unchanged
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010; **Priority:** High
- **Given** the converter of E2E-037 with `big.bin` of 1 048 571 bytes
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`, as today; nothing left

#### E2E-039: Upload with conversion, answer over the limit
- **Category:** side effect; **Scenario:** SC-006; **Requirements:** FR-NEW-013; **Priority:** Critical
- **Given** the stand in converter of E2E-034
- **When** a client uploads `deck.pptx` (2 bytes `PK`) into `/docs` through the HTTP API with the conversion flag
- **Then** the upload succeeds and `/docs/deck.pptx` exists with its 2 bytes
- **And** the documentation report of that file carries `ERR_INTERNAL_ERROR` and `document service output is over the doc_service.max_input_bytes limit of 1048576`; `/docs/deck.md` does not exist; no converter process of the call runs; no working folder remains

#### E2E-040: Binary write with conversion, answer over the limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-013, FR-NEW-014; **Priority:** High
- **Given** the stand in converter of E2E-034
- **When** the member writes `/docs/deck.pptx` through the assistant tool's binary write with the conversion flag
- **Then** the write succeeds and its documentation report carries the same code and message as E2E-039
- **And** `/docs/deck.md` does not exist

#### E2E-041: Documentize through the HTTP API, answer over the limit
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-010, FR-NEW-014; **Priority:** High
- **Given** `/docs/deck.pptx` stored and the stand in converter of E2E-034
- **When** a client asks the HTTP API for its companion
- **Then** the refusal carries `ERR_INTERNAL_ERROR` and the message of E2E-034; nothing left

#### E2E-042: Converter failure within the limits, unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012, FR-NEW-014; **Priority:** Medium
- **Given** a text mode stand in converter writing `the real cause` to its diagnostic output and ending with exit code 3
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service exited with exit code 3: the real cause`

#### E2E-043: Web failure status with a huge body
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-011, FR-NEW-012; **Priority:** High
- **Given** a web stand in converter answering failure status 500 with a body of `upstream broke` followed by 1 073 741 824 letters `x`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `document service returned HTTP 500: upstream broke` and quoting at most 2 048 characters of the body
- **And** the answer held never exceeds 1 114 112; nothing left

#### E2E-044: Timeout, unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012, FR-NEW-014; **Priority:** Medium
- **Given** timeout 1 second and a text mode stand in converter that waits 30 seconds before answering
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `doc service timed out after 1s`; nothing left

#### E2E-045: Document over the converter limit refused before conversion
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/docs/big.pptx` of 1 048 577 bytes and the stand in converter of E2E-034
- **When** the member asks for its companion
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `'/docs/big.pptx' is 1048577 bytes, over the doc_service.max_input_bytes limit of 1048576`
- **And** the stand in converter was never started

#### E2E-046: Two converter floods at once
- **Category:** side effect; **Scenario:** SC-006; **Requirements:** FR-NEW-011, FR-NEW-013; **Priority:** Medium
- **Given** members `mia@example.com` and `noa@example.com`, each with `/docs/deck.pptx` in their own project, and the stand in converter of E2E-SEC-013
- **When** both ask for the companion at the same time
- **Then** both are refused with the message of E2E-034
- **And** each call's answer held never exceeds 1 114 112; the server log has exactly one error level line per call naming `ERR_INTERNAL_ERROR` and that message; no converter process and no working folder remain

#### E2E-047: Empty answer, unchanged
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-012, FR-NEW-014; **Priority:** Medium
- **Given** a text mode stand in converter that answers nothing and ends with exit code 0
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `document service exited with exit code 0 but produced no output`

#### E2E-048: Not a member, converter flood
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012; **Priority:** Medium
- **Given** `/docs/deck.pptx`, the stand in converter of E2E-SEC-013 and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for its companion
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- **And** the stand in converter was never started

#### E2E-049: Companion exists without overwrite, converter flood
- **Category:** failure; **Scenario:** SC-006; **Requirements:** FR-NEW-012, FR-NEW-013; **Priority:** Medium
- **Given** `/docs/deck.pptx`, `/docs/deck.md` holding `old`, and the stand in converter of E2E-SEC-013
- **When** the member asks for its companion without overwrite
- **Then** the refusal is `ERR_NO_CLOBBER` with `'/docs/deck.md' exists (pass overwrite=true)`
- **And** `/docs/deck.md` still reads `old`; the stand in converter was never started

### 9.3 Exploit tests

Each covers one occurrence of the class sweep (Section 14.2) at its own location. PDF payloads are written as the attacker builds the file; "PDF bomb stream" is defined in the header of Section 9.

#### E2E-SEC-001: Page content stream bomb
- **Covers:** class sweep occurrence 2 (Section 14.2)
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the whole page content, 200 000 000 bytes, is inflated before anything is checked.
- **Given** `/docs/bomb.pdf` whose page 1 content is `<< /Length 194000 /Filter /FlateDecode >> stream` followed by the compressed form of 200 000 000 spaces, then `endstream`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/bomb.pdf: a PDF stream decodes to more than 104857600 bytes`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-002: Compressed object stream bomb
- **Covers:** occurrence 1
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the object stream is inflated whole when the document is loaded.
- **Given** `/docs/objstm.pdf` whose object stream `<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode >>` holds `1 0 ` then a dictionary padded to 200 000 000 bytes with spaces, compressed
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/objstm.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-003: Font file bomb
- **Covers:** occurrence 3
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the font file is inflated whole when page 1 selects the font.
- **Given** `/docs/font.pdf` whose page 1 shows `(Hi) Tj` with font `/F1`, whose font descriptor's `/FontFile` is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/font.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-004: Character map bomb
- **Covers:** occurrence 4
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the character map is inflated whole.
- **Given** `/docs/cmap.pdf` whose font `/F1` used on page 1 has a `/ToUnicode` that is a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/cmap.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-005: Colour profile bomb
- **Covers:** occurrence 5
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the colour profile is inflated whole when page 1 runs `/CS0 cs`.
- **Given** `/docs/icc.pdf` whose page 1 runs `/CS0 cs` with `/CS0 [/ICCBased 7 0 R]`, object 7 being a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/icc.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-006: Image bomb drawn once
- **Covers:** occurrence 6
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the image is inflated whole when page 1 runs `/Im0 Do`.
- **Given** `/docs/image.pdf` whose page 1 runs `/Im0 Do`, `/Im0` being `<< /Subtype /Image /Width 10000 /Height 20000 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode >>` over a PDF bomb stream
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/image.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-007: One form drawn 300 times
- **Covers:** occurrence 7
- **Red before the fix because:** `Then the refusal is ...` fails: today the extraction inflates the form 300 times, 300 000 000 bytes, and succeeds.
- **Given** `/docs/forms.pdf`, stored in under 100 000 bytes, whose page 1 content is `/Fm0 Do` written 300 times, `/Fm0` being a form whose compressed content inflates to 1 000 000 spaces
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/forms.pdf: page 1 decodes more than 268435456 bytes of PDF streams`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-SEC-008: A colour profile kept in 300 saved states
- **Covers:** occurrence 8
- **Red before the fix because:** `And the decoded bytes held never exceed 268 500 992` fails: today each `q` keeps a copy of the inflated profile, 300 000 000 bytes held at once.
- **Given** `/docs/states.pdf` whose page 1 content is `/CS0 cs q` written 300 times, `/CS0` being `[/ICCBased 7 0 R]` and object 7 a compressed profile inflating to 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is the page total message for page 1 of `/docs/states.pdf`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-SEC-009: Three object streams over the load total
- **Covers:** occurrence 9
- **Red before the fix because:** `And the decoded bytes held never exceed 268 500 992` fails: today the three object streams, 300 000 000 bytes, are inflated and kept for the whole call.
- **Given** `/docs/objs3.pdf` with three object streams each inflating to 100 000 000 bytes, stored in under 1 000 000 bytes
- **When** the member asks for its text
- **Then** the refusal is the load total message for `/docs/objs3.pdf`
- **And** the decoded bytes held never exceed 268 500 992; nothing left

#### E2E-SEC-010: Two million objects
- **Covers:** occurrence 10
- **Red before the fix because:** `Then the refusal is ...` fails: today the 2 000 000 objects are parsed into the server's object table and the extraction succeeds.
- **Given** `/docs/nulls.pdf`, stored in under 1 000 000 bytes, whose object streams hold 2 000 000 objects written `null`
- **When** the member asks for its text
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `could not extract /docs/nulls.pdf: the PDF holds more than 1000000 objects`
- **And** nothing left

#### E2E-SEC-011: One code mapped to a long text, shown 2 000 000 times
- **Covers:** occurrence 11
- **Red before the fix because:** `And the content held never exceeds 268 500 992` fails: today the page text, 512 000 000 bytes, is built whole before the character cut.
- **Given** `/docs/expand.pdf` whose font `/F1` maps code `<01>` to 128 letters `é` (256 bytes) and whose page 1 shows `<0101...01> Tj` with 2 000 000 codes
- **When** the member asks for its text
- **Then** the refusal is the content limit message for `/docs/expand.pdf`
- **And** the content held never exceeds 268 500 992; nothing left

#### E2E-SEC-012: Title read decoding again
- **Covers:** occurrence 12
- **Red before the fix because:** `And the decoded bytes held never exceed 104 933 136` fails: today the title read loads the document again and inflates its object streams whole.
- **Given** `/docs/titled.pdf` of E2E-032
- **When** the member asks for its text
- **Then** the refusal is the stream limit message naming `/docs/titled.pdf`
- **And** the decoded bytes held never exceed 104 933 136; nothing left

#### E2E-SEC-013: Text answer flood
- **Covers:** occurrence 14
- **Red before the fix because:** `And the answer held never exceeds 1 114 112` fails: today the 1 073 741 824 bytes are read whole before the empty answer check.
- **Given** `/docs/bomb.docx` and a text mode stand in converter answering 1 073 741 824 letters `a`, the answer of a converter unpacking a zip bomb
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`, before the converter has written its whole answer
- **And** the answer held never exceeds 1 114 112; the operating system lists no process of the stand in converter; the server's temporary folder holds no working folder of the call; nothing left

#### E2E-SEC-014: Diagnostic output flood
- **Covers:** occurrence 15
- **Red before the fix because:** `And the diagnostic held never exceeds 73 728` fails: today the 1 073 741 824 bytes of diagnostic output are read whole and only their tail kept.
- **Given** a text mode stand in converter writing 1 073 741 824 letters `n`, then `the real cause`, to its diagnostic output and ending with exit code 3
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with a message starting `document service exited with exit code 3: ` and ending with `the real cause`, of at most 8 320 bytes
- **And** the diagnostic held never exceeds 73 728; nothing left

#### E2E-SEC-015: Web answer flood
- **Covers:** occurrence 16
- **Red before the fix because:** `And the answer held never exceeds 1 114 112` fails: today the 1 073 741 824 byte body is read whole.
- **Given** a web stand in converter answering, with success status, 1 073 741 824 letters `a` as plain Markdown
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; the stand in converter records that the server closed the connection before 1 073 741 824 bytes were sent; nothing left

#### E2E-SEC-016: Web bundle answer flood
- **Covers:** occurrence 17
- **Red before the fix because:** `And the answer held never exceeds 1 114 112` fails: today the whole JSON body is read before the bundle files are counted.
- **Given** a web stand in converter answering, with success status, a bundle written `{"markdown":"` followed by 1 073 741 824 letters `a` and `"}`
- **When** the member asks for the companion of `/docs/bomb.docx`
- **Then** the refusal is `ERR_INTERNAL_ERROR` with `document service output is over the doc_service.max_input_bytes limit of 1048576`
- **And** the answer held never exceeds 1 114 112; the stand in converter records the connection closed early; nothing left

#### E2E-SEC-017: Standard output flood in bundle mode
- **Covers:** occurrence 18
- **Red before the fix because:** `And the bundle mode output held never exceeds 65 536` fails: today the 1 073 741 824 bytes of standard output are read whole and then ignored.
- **Given** a bundle mode stand in converter writing 1 073 741 824 letters `p` to its standard output and a valid bundle with `document.md` holding `# Deck`
- **When** the member asks for the companion of `/docs/deck.pptx`
- **Then** the conversion succeeds and `/docs/deck.md` holds `# Deck`
- **And** the bundle mode output held never exceeds 65 536

### 9.4 Modified tests

None. The existing PDF tests use a document of a few hundred bytes, and the existing converter tests stay within the limit or assert the bundle message kept verbatim (`cli_outdir_mode_caps_the_bundle_size`, `bundle_size_cap_is_checked_before_reading`); they stay unchanged and green.

### 9.5 Removed tests

None.

## 10. Traceability

| Scenario | Requirements | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | FR-NEW-001 to FR-NEW-005, FR-NEW-009, FR-NEW-014 | E2E-001, E2E-002, E2E-006 | E2E-019 | E2E-007, E2E-009, E2E-011, E2E-013, E2E-015, E2E-017 | E2E-001 (companion and audit line) |
| SC-002 | FR-NEW-001 to FR-NEW-006, FR-NEW-008, FR-NEW-014 | E2E-001 (success branch, SC-001) | E2E-008, E2E-010, E2E-012, E2E-014, E2E-016, E2E-018, E2E-023, E2E-029, E2E-SEC-001 to E2E-SEC-011 | E2E-031 | E2E-SEC-001 (nothing left) |
| SC-003 | FR-NEW-001, FR-NEW-003, FR-NEW-005, FR-NEW-007 | E2E-001 (success branch, SC-001) | E2E-024, E2E-025, E2E-026, E2E-027, E2E-028, E2E-032, E2E-SEC-012 | E2E-026 (load before pages) | E2E-024 (held 0) |
| SC-004 | FR-NEW-008, FR-NEW-009 | E2E-020 (cached success, categorized edge) | E2E-021, E2E-022 | E2E-020 | E2E-030 |
| SC-005 | FR-NEW-010, FR-NEW-012, FR-NEW-014 | E2E-003, E2E-004, E2E-005 | E2E-042, E2E-044, E2E-047 | E2E-033, E2E-035, E2E-037 | E2E-003 (companion and audit line) |
| SC-006 | FR-NEW-010 to FR-NEW-014 | E2E-SEC-017 (bundle succeeds despite the flood) | E2E-034, E2E-036, E2E-038, E2E-040, E2E-041, E2E-043, E2E-045, E2E-048, E2E-049, E2E-SEC-013 to E2E-SEC-016 | E2E-033 (success branch at the limit) | E2E-039, E2E-046 |

SC-002, SC-003 and SC-006 are refusal scenarios; their success branch is named.

| Requirement | Tests | Count |
|---|---|---|
| FR-NEW-001 | E2E-011, E2E-012, E2E-026, E2E-SEC-010 | 4 |
| FR-NEW-002 | E2E-007, E2E-008, E2E-017, E2E-023, E2E-029, E2E-SEC-001 to E2E-SEC-006, E2E-SEC-012 | 12 |
| FR-NEW-003 | E2E-009, E2E-010, E2E-027, E2E-SEC-009 | 4 |
| FR-NEW-004 | E2E-013, E2E-014, E2E-018, E2E-SEC-007, E2E-SEC-008 | 5 |
| FR-NEW-005 | E2E-015, E2E-016, E2E-028, E2E-SEC-011 | 4 |
| FR-NEW-006 | E2E-007, E2E-008, E2E-010, E2E-014, E2E-016, E2E-031, E2E-SEC-001 to E2E-SEC-011 | 17 |
| FR-NEW-007 | E2E-024, E2E-025, E2E-026, E2E-027, E2E-028, E2E-032, E2E-SEC-012 | 7 |
| FR-NEW-008 | E2E-008, E2E-021, E2E-022, E2E-030, E2E-SEC-001 | 5 |
| FR-NEW-009 | E2E-001, E2E-002, E2E-006, E2E-017, E2E-019, E2E-020, E2E-021, E2E-031 | 8 |
| FR-NEW-010 | E2E-033, E2E-034, E2E-035, E2E-036, E2E-037, E2E-038, E2E-041, E2E-SEC-013, E2E-SEC-015, E2E-SEC-016 | 10 |
| FR-NEW-011 | E2E-034, E2E-036, E2E-043, E2E-046, E2E-SEC-013 to E2E-SEC-017 | 9 |
| FR-NEW-012 | E2E-042, E2E-043, E2E-044, E2E-045, E2E-047, E2E-048, E2E-049 | 7 |
| FR-NEW-013 | E2E-034, E2E-039, E2E-040, E2E-046, E2E-049, E2E-SEC-013 | 6 |
| FR-NEW-014 | E2E-002, E2E-003, E2E-004, E2E-005, E2E-023, E2E-037, E2E-040, E2E-041, E2E-042, E2E-044, E2E-047, E2E-SEC-014, E2E-SEC-017 | 13 |

## 11. Open questions

- The tests were designed in the main run, not by a separate test designer: does not change observable behavior.
- Whether the bound can be met while keeping the current PDF reader, or needs a change of component: a HOW question for `/sdlc-design` (14.3), does not change observable behavior.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| PDF | a `.pdf` document, any letter case | document text extraction |
| Stream | a block of data inside a PDF, usually compressed: page content, object stream, font file, character map, colour profile, form, image | document text extraction |
| Decoded size | the bytes a stream yields once decompressed | document text extraction |
| Object | one numbered item of a PDF; objects can be stored inside compressed object streams | document text extraction |
| Compressed object stream | a stream holding many objects, decoded when the document is loaded | document text extraction |
| Load | reading a PDF's objects, compressed object streams included, before its pages | document text extraction |
| Page decode count | the decoded sizes counted for one page under FR-NEW-004 | document text extraction |
| Form | a reusable drawing a page draws by name, possibly many times | document text extraction |
| Character map | the table turning a font's codes into text | document text extraction |
| Colour profile | a stream describing a colour space, decoded when a page selects that colour space | document text extraction |
| Saved state | a drawing state a page saves to restore later, keeping its colour space | document text extraction |
| Stream limit | 104 857 600 bytes, the most one stream is allowed to decode to | document text extraction |
| Load total | 268 435 456 bytes, the most the compressed object streams are allowed to decode to together | document text extraction |
| Page total | 268 435 456 bytes, the most one page's decode count is allowed to reach | document text extraction |
| Object limit | 1 000 000 objects, the most a PDF is allowed to hold | document text extraction |
| Content limit | 268 435 456 bytes, the most the extracted content is allowed to reach | document text extraction |
| Extracted content | the text of every page with its page markers, before the member's character cut | document text extraction |
| Page marker | the `*[Page N]*` line before each non empty page's text | document text extraction |
| PDF bomb stream | a compressed stream of under 300 000 stored bytes inflating to exactly 200 000 000 bytes of spaces | document text extraction |
| Chunk | the fixed 65 536 bytes read at most past a limit before stopping | document text extraction, document conversion |
| Decoded bytes held | the largest amount of decoded PDF stream data the server holds at once during one call | document text extraction |
| Content held | the largest extracted content the server holds during one call | document text extraction |
| Converter | the external program or web endpoint the operator plugs in to turn a document into Markdown | document conversion |
| Stand in converter | a converter built by the tests with a described answer | document conversion |
| Converter limit | the operator's setting capping the document handed to the converter and its answer; 536 870 912 bytes by default | document conversion |
| Answer | what the converter returns: its standard output in text mode, the web answer body, or the bundle's files | document conversion |
| Text mode | the converter answers Markdown on its standard output | document conversion |
| Bundle mode | the converter writes a Markdown file, pictures and a manifest into a folder | document conversion |
| Bundle | the Markdown, files and manifest a converter returns in bundle mode or as a web bundle | document conversion |
| Diagnostic output | what the converter program writes besides its answer; its tail is quoted on failure | document conversion |
| Answer held | the largest amount of the converter's answer the server holds at once during one conversion | document conversion |
| Diagnostic held | the largest amount of diagnostic output the server holds at once during one conversion | document conversion |
| Bundle mode output held | the largest amount of standard output the server holds at once in bundle mode | document conversion |
| Working folder | the temporary folder a converter program runs in, removed after each call | document conversion |
| Documentize | asking the converter for the Markdown companion of a stored document | document conversion |
| Conversion flag | the option of an upload or binary write that asks for the companion too | document conversion |
| Documentation report | the part of an upload or write answer telling how the conversion went | document conversion |
| Measurement hook | the operator facing reading of decoded bytes held, content held, answer held, diagnostic held and bundle mode output held for one call | document text extraction, document conversion |
| Companion | the Markdown file written next to the document, `/docs/report.md` for `/docs/report.pdf` | project storage |
| Refresh | the member's choice to read the document again even when its companion is up to date | document text extraction |
| Table artifact, image artifact | a table or picture kept from a conversion, listed and read on its own | project storage |
| Session write allowance | the bytes a member can still write in the current session; 52 428 800 bytes by default | project storage |
| Audit line | the one line an operator reads to see who extracted or converted what | project storage |
| Refusal | an error answer; the project is left unchanged | document text extraction, document conversion |
| Member | a person with access to a project | project storage |
| Operator | the person who runs the server, configures the converter, watches its memory and reads its log and audit trail | document conversion |
| Attacker | a person crafting a document to exhaust the server's memory | document text extraction |
| Assistant tool | the tool an AI assistant calls | document text extraction, document conversion |
| HTTP API | the web endpoint a program calls; the second door | document text extraction, document conversion |

## 13. Implementability gate (MANDATORY)

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 8 | IMPLEMENTABLE |

**Amendments applied after round 1 (all A, amended in place):** A-1 citations recounted against the files (`service.rs` cli `convert` `:198-319`, api `:355-422`, bundle stdout `:305-308`, timeout kill `:287-295`, length check `:513-518`; `extract.rs` cached `:344-368`, hook `:181-185`, images `:194-217`; `fs_ops.rs:659-680`; `dataplane.rs:1544-1600`, `:2963-2978`; lopdf `object.rs:818-846`, `:849-872`, `document.rs:596`; pdf-extract `lib.rs:2389-2396`); A-2 E2E-006 no longer draws an image (what a drawn image does today is unverified, `lib.rs:1582` unwraps the content parse); A-3 E2E-017 records today's answer first instead of asserting a success never observed (14.3 line added); A-4 FR-NEW-013 log statement limited to documentize (an upload with a failed conversion answers success and is not logged as an error today, `fs_ops.rs:679`); A-5 Section 8 empty states row pointed at E2E-006, not E2E-010; A-6 fonts counted once per font name per page, matching the reader's cache (`lib.rs:1712`, `:2408`); A-7 E2E-SEC-017 categorized edge (the conversion succeeds); A-8 modal words removed from the summary, a test and the glossary.
**How the gates ran:** this harness exposes no subagent tool, so the self consistency check (Phase 10) and the audit (Phase 11, contract of 11.3, gap classes G1 to G14) were run inline by the author against the file on disk, with scripted checks for ids (66 summarized, 66 specified), categories (happy 6, failure 45, edge 12, side effect 3, security 17), per test scenario and requirement agreement between 9.1 and 9.2, requirement coverage (every FR at least 4), modals and implementation terms, and the sweep searches re run. Recorded as a concern in the handoff.
**No implementation detail outside Section 14:** PASS. Suffixes, verbatim messages (they name the operator setting `doc_service.max_input_bytes`, as today), error codes, the option name `max_chars` and the literal attacker payloads of 9.3 (PDF operators and dictionary keys as the attacker writes them) are WHAT, as in SPEC-0085 and SPEC-0088.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence

| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | two doors, suffix routing, stored file read whole, page markers, page count, title, no text layer note | `crates/core/src/mcp/server.rs:2401-2426` (`fs.extract_text`); `crates/core/src/api/dataplane.rs:1484-1505`; `crates/core/src/docs/extract.rs:371` (`read_bytes`), `:157`, `:166` (routing), `:510-534` (`pdf`), `:520` (marker), `:524` (pages), `:525-527` (title), `:528-531` (note) |
| 2 | every stream decoded whole, no server bound | lopdf 0.42.0 (`Cargo.lock:3677-3680`), pdf-extract 0.12.1 (`Cargo.lock:4270-4273`): object streams at load `lopdf-0.42.0/src/reader.rs:889`, `src/document.rs:484`, `src/object_stream.rs:42`; inflate `src/object.rs:794-815`, `:849-872` (`read_to_end` into an unbounded `Vec` from `:856`), LZW `:818-846`, ASCII85 `:874-920`; page content `src/document.rs:589-600`, called at `pdf-extract-0.12.1/src/lib.rs:2418`; `get_contents` `lib.rs:1255-1261`; font files `lib.rs:386-411`, font cache cleared per page `lib.rs:2408`, cache by name `lib.rs:1712`; character maps `lib.rs:926`, `:990`; colour profiles on every `cs`/`CS` `lib.rs:1633-1639`, `:1478`, `:1519`; forms and images on every `Do` `lib.rs:1860-1867`; undecoded encodings copied as stored `lib.rs:1257`, `lopdf src/document.rs:596` (an unknown filter is an error, `src/object.rs:811`); no size option: `lopdf src/load_options.rs` (`password`, `filter`, `strict` only) |
| 3 | title loads the document again | `crates/core/src/docs/extract.rs:539` (`Document::load_mem` after the text pass at `:513`) |
| 4 | no object or page count limit | `pdf-extract-0.12.1/src/lib.rs:2323-2333` (pages read until a page number is missing); `lopdf src/object_stream.rs:126` (`max_objects` applies to writing only) |
| 5 | page text built whole, character cut after | `pdf-extract-0.12.1/src/lib.rs:2323-2333`; `crates/core/src/docs/extract.rs:518-521`, `:188` (`truncate_chars` after extraction); code to long text through the character map `lib.rs:921-935` |
| 6 | error wrapping, no trace on refusal, log level, cached companion | `crates/core/src/docs/extract.rs:513-515` (`catch_unwind`, `pdf: {e}`), `:373-381` (`could not extract`), `:344-368` (cached); `crates/core/src/core/fs_ops.rs:1745-1796` (error returned before the landing, charge and audit on success); `crates/core/src/errors.rs:154`; `.agent_docs/architecture.md:349-357` |
| 7 | converter doors, flag, input limit and message | `crates/core/src/mcp/server.rs:1811` (`fs.write_bytes`), `:2575-2599` (`fs.documentize`); `crates/core/src/api/dataplane.rs:174`, `:605-700` (upload flag, `upload` at `:609`), `:1544-1600` (`write_bytes`, `documentize` at `:1581`); `crates/core/src/core/fs_ops.rs:635-685`, `:696-722`, `:764-772`; `crates/core/src/config.rs:818-820` (536 870 912 default), `:901`, timeout `:821-823` |
| 8 | text, bundle and web modes | `crates/core/src/docs/service.rs:198-319` (cli `convert`), `:355-422` (api `convert`), `crates/core/src/config.rs:810-813` (placeholders), `:824-832` (default command) |
| 9 | bundle capped, others read whole | `crates/core/src/docs/service.rs:478-536` (`read_bundle_blocking`, length checked `:513-518`, total `:521-526`, `std::fs::read` `:519`), `:552-588` (`parse_api_bundle`, counted after decode `:574-579`); unbounded: `:268-279` (`read_to_end` on stdout and stderr), `:387` (`response.text()`); stdout unused in bundle mode `:305-308`; timeout `config.rs:821-823` |
| 10 | failure messages, stderr tail, body head, kill on timeout | `crates/core/src/docs/service.rs:58` (`STDERR_CAP` 8 192), `:62` (`BODY_CAP` 2 048), `:281-302` (I/O error, timeout kill at `:287-295`, failure exit), `:388-394`, `:592-612` |
| 11 | conversion refusal effects | `crates/core/src/core/fs_ops.rs:659-680` (upload keeps the source, report `error` at `:679`), `:799-845` (`write_companion`: convert before write and charge); `crates/core/src/api/dataplane.rs:2963-2978` (test proving it); `crates/core/src/api/dataplane.rs:259-266` (`error_response`, ERROR for 5xx) |
| 12 | governing specs | `sdlc/specs/SPEC-0088-bound-office-document-part-decoding/sdlc-spec.md` FR-NEW-001 to FR-NEW-006, 3.2; `sdlc/specs/SPEC-0019-document-artifacts/sdlc-design.md` DEC-006; `sdlc/specs/SPEC-0041-refuse-inconsistent-zips/sdlc-spec.md` FR-NEW-005; `sdlc/specs/SPEC-0085-admit-declared-archive-sizes-before-decoding/sdlc-spec.md` |
| 13 | tests today: 3 PDF tests (`crates/core/src/docs/extract.rs:2029`, `:2039`, `:2045`), `tiny_pdf` used 8 times in `docs/extract.rs` and `tools/artifacts.rs`; 22 tests in `crates/core/src/docs/service.rs` (bundle cap `:978`, `:1021`); documentize tests `crates/core/src/tools/document.rs:355-430`; upload tests `crates/core/src/api/dataplane.rs:2963`; none on PDF limits, held bytes, objects, or a flood on stdout, stderr or the web body (`rg -n 'bomb\|held\|flood' crates/core/src/docs/service.rs` finds none) | as cited |

Test command: `cargo test -p mcp-fs-core --lib docs::extract`, `cargo test -p mcp-fs-core --lib docs::service`, `cargo test -p mcp-fs-core --lib tools::document`, `cargo test -p mcp-fs-core --lib api::dataplane`; full gate `cargo test --workspace`.

### 14.2 Class sweep

**Class:** untrusted bytes (a stored document, or the answer of a converter fed one) reaching an in memory decode or read with no bound on the bytes produced (CWE-409, CWE-400).

**Searches run** (whole `crates/core/src` and the two PDF crates in the cargo registry, tests filtered by their `#[cfg(test)]` module start):
1. `rg -n 'read_to_end|read_to_string|\.text\(\)\.await|\.bytes\(\)\.await|std::fs::read\(|fs::read\(' crates/core/src`: idiom "a stream, body or file read whole". Production hits: `docs/service.rs:272`, `:273`, `:387`, `:519`; `tools/web.rs:288`, `:401`; `tools/doc.rs:312`; `tools/archive.rs:1051`; `storage/blob/local.rs:110`; `api/dataplane.rs:634`, `:640`; operator files `config.rs:941`, `keys.rs:111`, `identity.rs:55`; `tools/sqlite.rs:71`. The rest are tests.
2. `rg -n 'pdf_extract::' crates/core/src`: idiom "a third party decoder handed the whole file": `docs/extract.rs:513`, `:539`, `:542-543`.
3. `rg -n 'decompressed_content|get_plain_content|decompress\(|read_to_end|get_contents\(|get_page_content' <registry>/lopdf-0.42.0/src <registry>/pdf-extract-0.12.1/src`: idiom "a stream inflated inside the PDF crates"; hits listed in 14.1 row 2.
4. `rg -n 'read_bytes\(' crates/core/src/docs crates/core/src/core/fs_ops.rs`: idiom "a stored file read whole before a decode": `docs/extract.rs:353`, `:371`; `core/fs_ops.rs:711`.
5. `rg -n '"Do"|"cs"|"CS"|"Tf"|"q"' <registry>/pdf-extract-0.12.1/src/lib.rs`: idiom "an expansion by reference without new input" (repeats).
Not searched: libgit2 pack decoding (separate surface, `git.max_pack_size_mb`), image decoders (none: images go to the OCR provider as stored bytes, `docs/extract.rs:194-217`), the Office readers and archives (bounded by SPEC-0084, SPEC-0085, SPEC-0088).

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | compressed object streams at load, `lopdf src/reader.rs:889`, `src/object_stream.rs:42`, reached from `crates/core/src/docs/extract.rs:513` | vulnerable | FR-NEW-002, FR-NEW-003; E2E-SEC-002 |
| 2 | page content `lopdf src/document.rs:589-600` via `pdf-extract src/lib.rs:2418` | vulnerable | FR-NEW-002; E2E-SEC-001 (originating occurrence of BL-0143) |
| 3 | font files `pdf-extract src/lib.rs:386-411` | vulnerable | FR-NEW-002; E2E-SEC-003 |
| 4 | character maps `lib.rs:926`, `:990` | vulnerable | FR-NEW-002; E2E-SEC-004 |
| 5 | colour profiles `lib.rs:1478`, `:1519` | vulnerable | FR-NEW-002; E2E-SEC-005 |
| 6 | forms and images on `Do` `lib.rs:1860-1867` | vulnerable | FR-NEW-002; E2E-SEC-006 |
| 7 | the same form decoded on every draw `lib.rs:1866` | vulnerable (repeat) | FR-NEW-004; E2E-SEC-007 |
| 8 | colour space copies kept in saved states `lib.rs:1633-1639` with the graphics state stack | vulnerable (copies held) | FR-NEW-004, FR-NEW-006; E2E-SEC-008 |
| 9 | object streams kept decoded for the whole call (`lopdf src/document.rs:473-496`) | vulnerable (total) | FR-NEW-003; E2E-SEC-009 |
| 10 | object table without a count bound (`lopdf src/reader.rs:831-900`) | vulnerable (expansion) | FR-NEW-001; E2E-SEC-010 |
| 11 | page text built whole, `crates/core/src/docs/extract.rs:518-521`, character map expansion `lib.rs:921-935` | vulnerable (expansion) | FR-NEW-005; E2E-SEC-011 |
| 12 | title load `crates/core/src/docs/extract.rs:539` | vulnerable (second load) | FR-NEW-002, FR-NEW-007; E2E-SEC-012 |
| 13 | stored file read whole, `crates/core/src/docs/extract.rs:353`, `:371`, `crates/core/src/core/fs_ops.rs:711` | safe: bounded by the stored size, admitted against the session allowance and the project's storage on write (`crates/core/src/quota.rs:47` `admit_file`, called at `crates/core/src/core/fs_ops.rs:578`, `:613`), uploads capped at 134 217 728 bytes (`crates/core/src/api/dataplane.rs:117`, `:134`) | none |
| 14 | converter standard output, text mode, `crates/core/src/docs/service.rs:272` | vulnerable | FR-NEW-010, FR-NEW-011; E2E-SEC-013 (originating occurrence of BL-0142) |
| 15 | converter diagnostic output `crates/core/src/docs/service.rs:273` | vulnerable | FR-NEW-011; E2E-SEC-014 |
| 16 | web answer body `crates/core/src/docs/service.rs:387` | vulnerable | FR-NEW-010, FR-NEW-011; E2E-SEC-015 |
| 17 | web bundle body, decoded after the whole body is held, `crates/core/src/docs/service.rs:562-579` | vulnerable through 16 | FR-NEW-010; E2E-SEC-016 |
| 18 | standard output in bundle mode, read whole and unused, `crates/core/src/docs/service.rs:272`, `:305-308` | vulnerable | FR-NEW-011; E2E-SEC-017 |
| 19 | bundle file read `crates/core/src/docs/service.rs:519` | safe for a converter that exited: length checked before the read (`:513-518`), total after (`:521-526`); residual: a file grown after its length was taken by a helper left running | BL-0150 |
| 20 | a form drawing itself, recursion without depth bound `pdf-extract src/lib.rs:1866-1867` | other class (uncontrolled recursion, CWE-674): ends the process, which `catch_unwind` at `crates/core/src/docs/extract.rs:513` does not contain (ASSUMED: a stack overflow aborts) | BL-0147 |
| 21 | page tree walked once per page `pdf-extract src/lib.rs:2389-2396` (`output_doc_page` calls `get_pages` at `:2394`), `lopdf src/document.rs:524-526` | other class (CPU, CWE-407) | BL-0149 |
| 22 | web fetch body `crates/core/src/tools/web.rs:288`, `:401` | same idiom, not document extraction: a remote page read whole in the optional web tools | BL-0148 |
| 23 | pandoc output `crates/core/src/tools/doc.rs:312` | out of class: output generated from the member's Markdown, not a decode of an uploaded container | none |
| 24 | tar entry `crates/core/src/tools/archive.rs:1051` | safe: decoded under SPEC-0084's bound (`crates/core/src/tools/archive.rs:24` `TAR_FRAMING`) and checked against its declared size (`:1052`) | none |
| 25 | multipart fields `crates/core/src/api/dataplane.rs:634`, `:640` | safe: body capped by `UPLOAD_BODY_LIMIT` (`:117`, `:134`) | none |
| 26 | `storage/blob/local.rs:110`, `config.rs:941`, `keys.rs:111`, `identity.rs:55`, `tools/sqlite.rs:71` | out of class: stored blobs (size admitted), operator files, an optional tool's own temp file | none |

### 14.3 Design inputs

- **Dependency question for /sdlc-design (not decided here):** lopdf 0.42.0 and pdf-extract 0.12.1 expose no decoded size cap (`lopdf src/load_options.rs`: password, filter, strict; `src/object.rs:849-856` reads to the end); object streams are inflated during `load_mem` itself (`src/reader.rs:889`), before any caller code runs on them. FR-NEW-001 to FR-NEW-006 therefore need either a server side pass that parses and inflates under a cap before the reader sees the file and then constrains what the reader decodes again (repeats of occurrences 7 and 8 are not visible to a pre scan alone), or a change of component (a patched or vendored reader, or another reader), or an isolated worker bounded by the operating system. The user's constraint (3.3): safe Rust, no new dependency unless unavoidable; the design states which and why. Signal: E2E-SEC-001 to 012.
- pdf-extract offers an `OutputDev` trait (`pdf-extract src/lib.rs:1878-1885`) whose methods return a `Result`, through `output_doc`/`output_doc_page` (`lib.rs:2374`, `:2389`): a caller can count the text as it is produced and stop at the content limit. Signal: E2E-016, E2E-SEC-011.
- `LoadOptions::filter` (`lopdf src/load_options.rs`) is a plain `fn` pointer seen at load; whether it runs before object streams are inflated must be checked before relying on it. ASSUMED unusable as a cap until shown.
- `get_contents` falls back to the stored bytes when the encoding is not decoded (`pdf-extract src/lib.rs:1257`); FR-NEW-002 counts such streams 0, held raw copies remain bounded by the stored file. What a drawn image does today is UNVERIFIED: `process_stream` parses whatever `Do` hands it with `Content::decode(..).unwrap()` (`lib.rs:1582`), so an image's bytes may panic into `pdf parser failed on this document` (`crates/core/src/docs/extract.rs:514`). E2E-017 therefore records today's answer first and asserts it is kept. Signal: E2E-017.
- The colour space is cloned into each saved state (`lib.rs:1633-1639`, graphics state stack); FR-NEW-006 counts these copies. Signal: E2E-SEC-008.
- "Decoded bytes held", "content held", "answer held", "diagnostic held" and "bundle mode output held" need test hooks like `LAST_OFFICE_HELD` (`crates/core/src/docs/extract.rs:181-185`, test only), reading 0 when nothing was decoded or read. Signal: every test asserting held bytes.
- Converter reads: a capped reader like `read_capped` (`crates/core/src/tools/archive_single.rs:92-108`, `CHUNK` 65 536) on stdout and the web body (`response.chunk()` or `bytes_stream` instead of `text()`, `crates/core/src/docs/service.rs:387`); a tail ring of 8 192 bytes on stderr (`:273`, `tail_of` `:592`); a drain that discards stdout in bundle mode. On the cap, kill and reap the child before the `TempDir` drops, as the timeout path does (`:287-295`). Signal: E2E-SEC-013 to 017.
- A web failure status must be read only far enough to quote the head (`BODY_CAP`, `service.rs:62`) and never past the cap. Signal: E2E-043.
- E2E-SEC-015 and 016 "connection closed early" need the web stand in to record how many bytes it sent; the local multipart stub at `crates/core/src/docs/service.rs:1048` (`stub_server`) is the starting shape.
- E2E-SEC-013 "process absent" and "working folder absent" reuse the probes of `cli_timeout_kills_the_child_and_removes_the_tempdir` (`crates/core/src/docs/service.rs:845`).
- E2E-030 and E2E-046 need the log lines of a 4xx and a 5xx tool error; `logging.rs` classifies by `ToolError::is_client_error` (`crates/core/src/errors.rs:154`).
- Docs to update: `.agent_docs/tools.md` (`fs.extract_text` PDF limits, `fs.documentize` answer cap), `.agent_docs/architecture.md`, `.agent_docs/config.md` (`doc_service.max_input_bytes` now also caps every answer channel), `README.md`; `TOOL_CONTRACT.txt` only if the design mentions the limits in a description (contract change, golden regenerated).

### 14.4 Interview answers and defaults applied

Rounds 1 to 5 were answered from the code and the two backlog entries, without the user, under their authorization.
- R1 scope: PDF text extraction and the converter's answer, both doors; out: Office (SPEC-0088), archives, recursion (BL-0147), web fetch (BL-0148), page walk cost (BL-0149), bundle growth after exit (BL-0150), worker thread (BL-0146).
- R1 why now: BL-0142 and BL-0143, deferred from SPEC-0088, exploitable until shipped.
- R2 current state: Section 2 and 14.1, read from the code and the two PDF crates' sources.
- R3 scenarios: SC-001 to SC-006. R4 requirements: derived, Section 6. R5 NFR: refusal without waiting, no process or folder left, one log line, held bounds.
- D-1 routing: both entries read `Nature: BUG`; the brief asks for one FEAT spec following SPEC-0084, SPEC-0087 and SPEC-0088. Chosen FEAT: observable behavior changes (new refusals) and no spec defines a bound the code violates. Override recorded here.
- D-2 one increment for two entries, as the user asked; two bounded contexts, depth M (14 requirements).
- D-3 security `internal finding` (CWE-409, CWE-400); sweep and exploit tests included.
- D-4 PDF numbers reuse SPEC-0088: stream limit 104 857 600 (its part limit), load total, page total and content limit 268 435 456 (its call and content limits), chunk 65 536.
- D-5 PDF limits fixed, not configured: no configuration value bounds extraction today (`crates/core/src/config.rs`).
- D-6 object limit 1 000 000: large PDFs of a few thousand pages hold tens of thousands of objects; 2 000 000 `null` objects fit in under 1 000 000 compressed bytes and cost the server far more than their decoded size. Pages are objects, so the object limit also bounds the page count; no separate page limit.
- D-7 repeats: a page counts every decode the reader makes for it today (forms and images per draw, colour profiles per selection, fonts once per page), so the page total bounds repeats; counting fonts per selection would refuse ordinary documents (a font is selected many times per page), so fonts count once per font name per page, matching what is decoded (`pdf-extract src/lib.rs:1712`, cache by name, cleared per page at `:2408`).
- D-8 load and page totals instead of one call total: fonts are decoded again on every page today, so a call total counting repeats would refuse long ordinary documents; the page total bounds what one page can make the server decode, and the held bound (FR-NEW-006) bounds memory.
- D-9 streams in an encoding the extraction does not decode count 0: they are not decoded; their stored bytes are already bounded by the stored file.
- D-10 the converter limit is the existing operator setting (`doc_service.max_input_bytes`), already the bundle cap, extended to every answer channel; its message kept verbatim so the existing bundle tests stay unchanged; code `ERR_INTERNAL_ERROR` as every converter failure today.
- D-11 diagnostic output: only its 8 192 byte tail is kept (today's quoted size), not counted against the converter limit; standard output in bundle mode is discarded and not counted; both bounded in time by the timeout as today.
- D-12 on the cap the converter program is stopped and awaited, its working folder removed, and a web answer abandoned, as the timeout path does today.
- D-13 a web failure status keeps today's message even when its body passes the limit (the status is known first).
- D-14 PDF refusal code `ERR_INVALID_ARGUMENT` with the `could not extract <path>: ` prefix, as every extraction failure.
- D-15 tests: 66 new, 0 modified, 0 removed; the test plan is approved by the user's request "spec BL-0142 and BL-0143 together".
- D-16 BL-0147, BL-0148, BL-0149 and BL-0150 filed for what the sweep found outside this increment, deferred under the same authorization.
- D-17 the HOW of the PDF bound (pre scan, component change or isolated worker) is left to `/sdlc-design` as a dependency question (14.3).
