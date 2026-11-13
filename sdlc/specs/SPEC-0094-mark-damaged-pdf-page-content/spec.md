# Tell a member when a PDF page is damaged or cut: functional specification

> Id: SPEC-0094
> Nature: FEAT
> Depth: M
> Depth evidence: 1 observable behavior, 2 modules touched (the PDF page reading and the text extraction answer, including the answer given from a stored text copy), 9 anticipated requirements, no change to the answer shape. M by exclusion from S (two modules) and L.
> Status: implemented
> Generated on: 2026-11-12
> Project: existing
> From backlog: BL-0154
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Summary

When a member extracts the text of a PDF whose page content is damaged or cut, the text of that page is silently shorter, or empty, and the answer looks like a success. This increment keeps the extracted text exactly as it is today and adds a note to the answer that names the damaged pages, so a member knows the text may be incomplete before relying on it or indexing it. The backlog entry listed three options; the owner chose the marker (the "may be incomplete" note), not a refusal.

## 2. Current State (MANDATORY, every depth)

### 2.1 What users can do today

- A member extracts the text of a PDF. The answer holds the text, one `*[Page N]*` marker before each page, the number of pages, the title when there is one, and a note.
- The note is empty, except when the PDF has no text at all: then it reads `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`.
- When the content of a page is cut (for example inside a text string) or holds bytes that cannot be read as page content, the reader keeps what it understood before the problem and drops the rest. The page appears with less text, or none, and nothing in the answer says so. The same happens when the content of a form the page draws is damaged.
- Only a few damages are refused today, for example an inline image with no data.
- The extraction stores a text copy next to the PDF. A second extraction of the same PDF is answered from that stored copy: that answer carries the path, a preview of the text, its length and the indication that it came from the stored copy, and no note, no page count and no title.
- The same extraction is offered through the assistant tool interface and through the web API, with identical answers.

### 2.2 Specs governing this area

- SPEC-0089 and SPEC-0092 bound what the PDF reader may decode and how deep forms nest. Their refusals stay as they are and keep priority over this increment (FR-NEW-008). This increment changes none of their requirements.

### 2.3 What is verified today

- PDF text extraction has tests for pages, titles, bounds, nesting and refusals. None asserts anything about a damaged or cut page content: such a page extracts as less text and no test notices. The command is given in the evidence appendix.

## 3. Scope

### 3.1 In scope

- A note, in the extraction answer, naming every page whose content is damaged or cut.
- The same note when the answer comes from a stored text copy, and a way to examine again a PDF (the refresh option that exists today).
- Damage in the page content, in any of its parts, and in the content of forms the page draws.

### 3.2 Out of scope, and why

- Refusing a damaged PDF (the entry's option a): a member would lose the text that can be read, and PDFs with harmless trailing garbage would stop working.
- Reporting where in the page the damage starts (the entry's offset idea): the page number is enough for a member to decide.
- Any other document format: only PDF is read by this reader.
- The external document converter: it is a separate path.
- Reporting a content stream whose compression cannot be undone, a content reference that leads to nothing, or an unreadable page entry: these are a different kind of damage, were not asked for, and can be a later increment if the owner wants them.
- Changing what text is extracted: the text stays byte identical to today.
- Stored text copies written before this change: they have no record of damage and answer without a note until the member uses the refresh option (FR-NEW-005, FR-NEW-006). Re examining every stored copy at once is not done.
- Adding a machine readable list of pages next to the note: the note is enough for now.

### 3.3 Constraints imposed by the context

- The note that already exists in a fresh answer carries the damage message; no other new part is added to any answer. The answer given from a stored text copy gains that same note (FR-NEW-005), and keeps everything else it has today. The owner chose the marker (option b of the backlog entry BL-0154).

## 4. Actors

- **Member:** a person with read access to the project, extracting the text of a PDF.
- **Assistant tool or program of the member:** calls the same extraction through the assistant tool interface or the web API, and reads the same note.

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Document reading | turning a PDF into text and page markers, and judging whether a page was read to its end | page, damaged page, note |
| Stored text copies | the copy of an extracted text kept next to the document, and the answer given from it | stored text copy, refresh |

## 5. Usage scenarios

### SC-001: Extract a PDF with a cut page
**Actor:** Member
**Preconditions:** a PDF `report-3p.pdf` of 3 pages: page 1 reads `Quarterly results`, page 2 reads `Revenue rose` followed by a text cut in the middle (`Costs fell by` with no closing), page 3 reads `Outlook stable`. No stored text copy exists.
**Flow:**
1. The member extracts the text of `report-3p.pdf`. → The answer holds the text of the three pages as it does today (page 2 reads `Revenue rose`) and the note `pdf: damaged or cut content on page 2; the text may be incomplete`.
**Postconditions:** a stored text copy exists; nothing else changed.
**Exceptions:**
- EXC-001a: the PDF also carries 65 nested forms on page 1 → the member gets the same refusal as before this change and no damage note.
- EXC-001b: the page that is cut is made of two parts and the second one is cut → the page is named all the same.
**Interactions:** SC-002, SC-003.

### SC-002: Extract a clean PDF
**Actor:** Member
**Preconditions:** a PDF `clean-3p.pdf` of 3 pages, each content complete.
**Flow:**
1. The member extracts the text. → The answer is identical to today: same text, empty note.
**Postconditions:** a stored text copy exists.
**Exceptions:**
- EXC-002a: the PDF is a scan with no text and one cut page → the note holds the damage note then the scanned PDF message; a scan with no cut page keeps the scanned PDF message alone.

### SC-003: Extract the same PDF again
**Actor:** Member
**Preconditions:** SC-001 happened.
**Flow:**
1. The member extracts the text again. → The answer comes from the stored text copy and carries the same note as the first time.
2. The member extracts again with the refresh option. → The PDF is examined again; the note is the same.
**Postconditions:** unchanged.
**Exceptions:**
- EXC-003a: the stored text copy was written before this change and the PDF is damaged → the answer carries no damage note; with the refresh option it does.
- EXC-003b: the PDF was replaced by a clean file → the next extraction carries no damage note, with or without the refresh option.

### SC-004: Many damaged pages
**Actor:** Member
**Preconditions:** a PDF of 25 pages, every page cut like page 2 of SC-001.
**Flow:**
1. The member extracts the text. → The note lists the first 20 pages and says how many more.
**Postconditions:** none.
**Exceptions:**
- EXC-004a: the text asked for is shorter than the PDF, so the damaged pages are not in the returned text → they are still listed in the note.

### SC-005: Damage inside a form
**Actor:** Member
**Preconditions:** a PDF of 3 pages where page 2 draws a form whose content is cut, and page 3 is cut in its own content.
**Flow:**
1. The member extracts the text. → The note names pages 2 and 3; a page damaged in both its content and a form it draws is named once.
**Postconditions:** none.
**Exceptions:**
- EXC-005a: a page is cut in its own content and also draws a cut form → it is named once.
- EXC-005b: a form ends with a stray `>>` → the page is named.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Damaged pages are reported
> WHEN a member extracts the text of a PDF in which the content of one or more pages is damaged or cut, THE text extraction SHALL return the text of every page exactly as it returns it today AND a note naming the damaged pages.

- **Business rules:** page content is a sequence of instructions, each made of operands followed by one operator word. A page is damaged if and only if reading the content of the page, or of any one of the parts it is made of, or of a form it draws, instruction by instruction from the start, stops before the end of that content. Only blanks, line breaks and comments after the last readable instruction are accepted as the end. An operator word the reader does not know, and an instruction that opens a block never closed, are not damage. A stray `>>`, a text string never closed and an instruction cut in the middle are damage. Only content that was read as bytes is judged: a content stream whose compression cannot be undone, a content reference that leads to nothing, a page entry that cannot be read as a page and a page with no content at all are not judged and stay exactly as today (not reported, not refused). The note lists every damaged page of the whole PDF, including pages whose text is not returned because of the maximum length asked for. The text, the `*[Page N]*` markers, the page count and the title are the same as before this change.
- **User visible text:** see FR-NEW-002.
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-002 [EARS-U]: Exact wording of the note
> THE text extraction SHALL write the damage note as `pdf: damaged or cut content on page <N>; the text may be incomplete` for one damaged page, and `pdf: damaged or cut content on pages <N1>, <N2>; the text may be incomplete` for several, listing page numbers in ascending order, numbered as in the `*[Page N]*` markers, separated by a comma and a space.

- **Business rules:** at most 20 page numbers are listed. When more than 20 pages are damaged, the first 20 are listed followed by ` and <M> more`, where M is the number of damaged pages minus 20. Exactly 20 damaged pages are all listed with no ` and ... more`. Example for 25 damaged pages: `pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20 and 5 more; the text may be incomplete`.
- **User visible text:** the forms above, verbatim.
- **Scenario:** SC-001, SC-004
- **Priority:** Must

#### FR-NEW-003 [EARS-UB]: No note when nothing is damaged
> THE text extraction SHALL NOT write a damage note for a PDF in which every page content is complete, nor for any document that is not a PDF.

- **Business rules:** a PDF with an empty page, or with a space, a line break or a comment after the last instruction of a page, is complete. The answer for such a PDF is identical to today's.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-004 [EARS-E]: Damage anywhere in a page counts, once
> WHEN any part of the content of a page, or the content of a form the page draws, is damaged or cut, THE text extraction SHALL count that page as damaged once.

- **Business rules:** a page damaged both in its own content and in a form it draws is listed once.
- **Scenario:** SC-005
- **Priority:** Must

#### FR-NEW-005 [EARS-E]: The stored text copy keeps the note
> WHEN a member extracts the text of a PDF and the answer comes from its stored text copy, THE text extraction SHALL include in that answer a note equal to the note of the extraction that stored the copy: the damage note, or the empty text. The note is present in the answer even when it is empty.

- **Business rules:** a stored text copy written before this change counts as one whose note was empty. The note is the one of the extraction that stored the copy, whatever maximum length that extraction or the current one asked for; it lists every damaged page of the whole PDF. The answer from a stored text copy of a document that is not a PDF stays exactly as it is today, with no note. The answer keeps everything else it has today. A PDF whose extracted text is empty has no stored text copy today and gets none: every extraction of it reads the PDF again and answers as a first extraction.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-006 [EARS-E]: The refresh option examines again the PDF
> WHEN a member extracts the text of a PDF with the refresh option, THE text extraction SHALL read the PDF again and answer exactly as for a first extraction, damage note included.

- **Business rules:** a PDF replaced after the copy was stored is read again without the refresh option, as today.
- **Scenario:** SC-003
- **Priority:** Must

#### FR-NEW-007 [EARS-O]: Both notes together
> IF the extracted text of a PDF is empty AND one or more pages are damaged, THEN THE text extraction SHALL write the damage note first, then one space, then `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`.

- **Business rules:** the damage note and the scanned message both appear on every extraction of such a PDF, since no stored text copy exists for it (FR-NEW-005). When no page is damaged the note is the existing message alone. When pages are damaged and the text is not empty the note is the damage note alone.
- **Scenario:** SC-002
- **Priority:** Must

#### FR-NEW-008 [EARS-UB]: Damage never changes the outcome
> THE text extraction SHALL NOT refuse, shorten or alter the extraction of any PDF because a page is damaged or cut.

- **Business rules:** a PDF refused today is refused with the same message and no damage note, including the refusal for forms nested more than 64 deep (`page <N> draws PDF forms nested more than 64 deep`).
- **Scenario:** SC-001
- **Priority:** Must

#### FR-NEW-009 [EARS-U]: Same note on every way in
> THE text extraction SHALL give the same damage note through the assistant tool interface and through the web API.

- **Scenario:** SC-001, SC-005
- **Priority:** Must

### 6.2 Modified
None.

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
Not applicable as a new promise: the extraction of a clean PDF keeps the limits and refusals of earlier specs; FR-NEW-008 protects them. The cost of the check is left to design.
### 7.2 Security and access
Not applicable: who may extract a PDF is unchanged, and the note holds only page numbers.
### 7.3 Privacy and compliance
Not applicable: the note holds no document content.
### 7.4 Usability and accessibility
The note is plain text, in English like the existing notes (FR-NEW-002).
### 7.5 Reliability
FR-NEW-008 is the reliability promise: an extraction that succeeds today still succeeds with the same text.
### 7.6 Operability
Not applicable: no operator facing change.

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | a page with no content; a PDF with no text | no damage note for an empty page; scanned message only | E2E-016, E2E-021 |
| Boundary values | a different maximum length on the second call; exactly 20 damaged pages; 25 damaged pages; text asked shorter than the PDF | all listed; first 20 and ` and 5 more`; pages beyond the returned text still listed | E2E-020, E2E-005, E2E-004, E2E-019 |
| Unusual but valid content | an operator word the reader does not know; an opened block never closed; a stray `>>` in a form | the first two are not damage; the stray `>>` is | E2E-017, E2E-018 |
| Concurrent actions | not applicable: extraction of one file, no shared mutable state seen by the member | none | n/a |
| Slow or lost connectivity | not applicable: no network in the reading | none | n/a |
| Access states | not applicable: access rules unchanged | none | n/a |
| Data variations | a cut inside a text string with accents (`Coût en baisse de`) | page listed; text up to the cut unchanged | E2E-011 |
| Navigation | not applicable | none | n/a |
| Partial operations | a cut in the second part of a page made of two parts | page listed | E2E-015 |
| Dependency failures | not applicable | none | n/a |
| Ordering and timing | file replaced after the copy was stored | read again, no stale note | E2E-009 |
| Exhaustion | the 65 deep forms refusal on a PDF that is also damaged | same refusal, no note | E2E-012 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, 002, 007, 008, 009 | Critical |
| E2E-002 | New | requirement | SC-002 | FR-NEW-003, 005, 007 | Critical |
| E2E-003 | New | failure | SC-005 | FR-NEW-001, 002, 004, 009 | High |
| E2E-004 | New | edge | SC-004 | FR-NEW-002, 005 | High |
| E2E-005 | New | edge | SC-004 | FR-NEW-002 | High |
| E2E-006 | New | edge | SC-005 | FR-NEW-002, 004, 009 | High |
| E2E-007 | New | side effect | SC-003 | FR-NEW-005, 006 | Critical |
| E2E-008 | New | state transition | SC-003 | FR-NEW-005, 006 | High |
| E2E-009 | New | state transition | SC-003 | FR-NEW-003, 006 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-005, 007 | High |
| E2E-011 | New | failure | SC-001 | FR-NEW-001, 008 | High |
| E2E-012 | New | failure | SC-001 | FR-NEW-008 | High |
| E2E-013 | New | edge | SC-002 | FR-NEW-003 | Medium |
| E2E-014 | New | cross scenario | SC-001 | FR-NEW-001, 002, 009 | High |
| E2E-015 | New | edge | SC-001 | FR-NEW-001, 004 | High |
| E2E-016 | New | edge | SC-002 | FR-NEW-003 | High |
| E2E-017 | New | edge | SC-002 | FR-NEW-001, 003 | High |
| E2E-018 | New | edge | SC-005 | FR-NEW-001, 004 | High |
| E2E-019 | New | failure | SC-004 | FR-NEW-001, 002 | High |
| E2E-020 | New | edge | SC-003 | FR-NEW-002, 005 | High |
| E2E-021 | New | edge | SC-002 | FR-NEW-003, 007 | Medium |

Coverage: happy 2 (E2E-001, 002), failure 5 (E2E-003, 010, 011, 012, 019), edge 10 (E2E-004, 005, 006, 013, 015, 016, 017, 018, 020, 021), side effect 1 (E2E-007), state transition 2 (E2E-008, 009), cross scenario 1 (E2E-014), security 0 (not applicable). Total 21. Happy to failure ratio 1:2.5, above 1:1.

### 9.2 New tests

#### E2E-001: Cut page is reported and the text is kept
- **Category:** journey
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-008, FR-NEW-009
- **Given** the PDF `report-3p.pdf` of SC-001 stored in a project, no stored text copy, and the member extracts through the assistant tool interface
- **When** the member extracts its text
- **Then** the text holds, in order, `*[Page 1]*` and `Quarterly results`, `*[Page 2]*` and `Revenue rose`, `*[Page 3]*` and `Outlook stable`, the page count is 3, and the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- **And** the text is identical to what the same file gave before this change; the scanned PDF message is absent
- **Priority:** Critical

#### E2E-002: Clean PDF is untouched
- **Category:** requirement
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003, FR-NEW-005, FR-NEW-007
- **Given** `clean-3p.pdf` (pages `Alpha`, `Beta`, `Gamma`, all complete)
- **When** the member extracts its text, then extracts it again
- **Then** both answers hold the text of the three pages (the second one as a preview) and an empty note, and the second answer comes from the stored text copy
- **And** neither note contains `damaged` nor `scanned`, and the second answer holds a note that is exactly the empty text
- **Priority:** Critical

#### E2E-003: Two damaged pages through the web API, one cut in a form
- **Category:** failure
- **Scenario:** SC-005
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-009
- **Given** a 3 page PDF: page 1 cut in its own content (`Opening remarks` then `Agenda i` with no closing), page 2 complete, page 3 draws a form whose content is cut, and the member uses the web API
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on pages 1, 3; the text may be incomplete`
- **And** the text holds `Opening remarks` on page 1 and the complete text of page 2
- **Priority:** High

#### E2E-004: Twenty five damaged pages
- **Category:** edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-002, FR-NEW-005
- **Given** a 25 page PDF where every page is cut like page 2 of `report-3p.pdf`
- **When** the member extracts its text, then extracts it again
- **Then** both notes are exactly `pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20 and 5 more; the text may be incomplete`
- **And** the second answer comes from the stored text copy
- **Priority:** High

#### E2E-005: Exactly twenty damaged pages
- **Category:** edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-002
- **Given** a 22 page PDF where pages 1 to 20 are cut and pages 21 and 22 are complete
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20; the text may be incomplete`, with no `more`
- **Priority:** High

#### E2E-006: A page damaged twice is named once
- **Category:** edge
- **Scenario:** SC-005
- **Requirements:** FR-NEW-002, FR-NEW-004, FR-NEW-009
- **Given** a 3 page PDF where page 2 is cut in its own content AND draws a form whose content is cut; pages 1 and 3 are complete
- **When** the member extracts its text through the assistant tool interface
- **Then** the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- **Priority:** High

#### E2E-007: Repeating the extraction keeps the note
- **Category:** side effect
- **Scenario:** SC-003
- **Requirements:** FR-NEW-005, FR-NEW-006
- **Given** `report-3p.pdf` extracted once (E2E-001)
- **When** the member extracts it a second time, then a third time with the refresh option
- **Then** the second answer comes from the stored text copy with the note `pdf: damaged or cut content on page 2; the text may be incomplete`; the third answer does not come from the stored copy and carries the same note
- **Priority:** Critical

#### E2E-008: A stored copy with no damage record
- **Category:** state transition
- **Scenario:** SC-003
- **Requirements:** FR-NEW-005, FR-NEW-006
- **Given** `report-3p.pdf` and a stored text copy for it that is newer than the PDF and was written without any damage record (as copies written before this change are)
- **When** the member extracts its text, then extracts it with the refresh option
- **Then** the first answer comes from the stored copy with a note that is present and exactly the empty text; the second answer carries `pdf: damaged or cut content on page 2; the text may be incomplete`
- **Priority:** High

#### E2E-009: File replaced by a clean one
- **Category:** state transition
- **Scenario:** SC-003
- **Requirements:** FR-NEW-003, FR-NEW-006
- **Given** `report-3p.pdf` extracted once, then replaced by the complete `clean-3p.pdf` content under the same name
- **When** the member extracts it without, then with, the refresh option
- **Then** both answers hold the text `Alpha`, `Beta`, `Gamma` and an empty note
- **Priority:** High

#### E2E-010: Damaged page and no text
- **Category:** failure
- **Scenario:** SC-002
- **Requirements:** FR-NEW-005, FR-NEW-007
- **Given** a 1 page PDF whose only content is a text string opened and never closed (`(`), so no text is extractable
- **When** the member extracts its text, then extracts again without the refresh option
- **Then** both notes are exactly `pdf: damaged or cut content on page 1; the text may be incomplete no extractable text layer (scanned PDF?); enable a multimodal OCR provider`
- **And** neither answer comes from a stored text copy, and no stored text copy exists next to the PDF
- **Priority:** High

#### E2E-011: Text before the cut is kept verbatim, accents included
- **Category:** failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-008
- **Given** a 1 page PDF with the content `Bilan` then `Coût en baisse de` cut with no closing
- **When** the member extracts its text
- **Then** the text equals what the same file gave before this change (`Bilan` and nothing from the cut part), and the note is exactly `pdf: damaged or cut content on page 1; the text may be incomplete`
- **Priority:** High

#### E2E-012: Existing refusal wins
- **Category:** failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008
- **Given** a 2 page PDF stored as `/nested-and-cut.pdf` where page 1 draws forms nested 65 deep and page 2 is cut
- **When** the member extracts its text
- **Then** the extraction is refused with exactly `could not extract /nested-and-cut.pdf: page 1 draws PDF forms nested more than 64 deep`, the same as before this change
- **And** no stored text copy is written and no damage note appears
- **Priority:** High

#### E2E-021: A scan with no cut page keeps its own message
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003, FR-NEW-007
- **Given** a 1 page PDF with no text at all and complete content (an empty page)
- **When** the member extracts its text twice
- **Then** both notes are exactly `no extractable text layer (scanned PDF?); enable a multimodal OCR provider`, and neither contains `damaged`
- **Priority:** Medium

#### E2E-013: Other formats are untouched
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003
- **Given** `notes.txt` containing `(unclosed`, `memo.docx` with one paragraph `Hello`, and `sheet.xlsx` with cell A1 holding `42`
- **When** the member extracts the text of each
- **Then** each note is exactly empty, the text of `notes.txt` is `(unclosed`, and no note contains `damaged`
- **And** extracting `memo.docx` a second time gives the answer from its stored text copy exactly as it did before this change
- **Priority:** Medium

#### E2E-014: Both ways in give the same note
- **Category:** cross scenario
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-009
- **Given** `report-3p.pdf` and a second project copy of it with no stored text copy
- **When** the member extracts one through the assistant tool interface and the other through the web API
- **Then** the two notes are both exactly `pdf: damaged or cut content on page 2; the text may be incomplete` and the two texts are equal
- **Priority:** High

#### E2E-015: Cut in the second part of a page
- **Category:** edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** a 2 page PDF where page 1 is complete and page 2 is made of two parts, the first complete (`First part`) and the second cut (`Second pa`, no closing)
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete` and the text of page 2 holds `First part`
- **Priority:** High

#### E2E-016: Empty page and trailing blanks are not damage
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003
- **Given** a 3 page PDF: page 1 complete, page 2 with no content at all, page 3 complete and followed by two line breaks and a comment line
- **When** the member extracts its text
- **Then** the note is empty and the text equals what the same file gave before this change
- **Priority:** High

#### E2E-017: Unknown operator and unclosed block are not damage
- **Category:** edge
- **Scenario:** SC-002
- **Requirements:** FR-NEW-001, FR-NEW-003
- **Given** a 2 page PDF: page 1 content is `BT (Known) Tj ET` followed by an instruction with the operator word `Zz` the reader does not know; page 2 opens a block with `q` and never closes it
- **When** the member extracts its text
- **Then** the note is empty and the text equals what the same file gave before this change
- **Priority:** High

#### E2E-018: Stray closing marks in a form
- **Category:** edge
- **Scenario:** SC-005
- **Requirements:** FR-NEW-001, FR-NEW-004
- **Given** a 2 page PDF where page 2 draws a form whose content is `BT (Fine) Tj ET` followed by `>>`, and page 1 is complete
- **When** the member extracts its text
- **Then** the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete` and the text of page 2 holds `Fine`
- **Priority:** High

#### E2E-019: Damaged page beyond the text asked for
- **Category:** failure
- **Scenario:** SC-004
- **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** `report-3p.pdf` and a maximum length of 20 characters
- **When** the member extracts its text
- **Then** the returned text is cut at 20 characters and flagged as cut, and the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- **Priority:** High

#### E2E-020: The note does not depend on the length asked for
- **Category:** edge
- **Scenario:** SC-003
- **Requirements:** FR-NEW-002, FR-NEW-005
- **Given** `report-3p.pdf` extracted once with a maximum length of 20 characters
- **When** the member extracts it again with a maximum length of 1000
- **Then** the answer comes from the stored text copy and the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- **Priority:** High

### 9.3 Exploit tests
Not applicable: no security flag, and the note carries no document content.

### 9.4 Modified tests
None. No existing test changes.

### 9.5 Removed tests
None.

## 10. Traceability

Columns: **Main flow** is the test of the scenario's flow; **Exception flow** is a test of one of its EXC- paths; **Edge** is a boundary or unusual input test. A test category in 9.1 can differ from its column (a state transition test can be the exception flow of SC-003).

| Scenario | Requirements | Main flow | Exception flow | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, 002, 007, 008, 009 | E2E-001, E2E-014 | E2E-012 (EXC-001a), E2E-011 | E2E-015 (EXC-001b) |
| SC-002 | FR-NEW-003, 005, 007 | E2E-002 | E2E-010 (EXC-002a) | E2E-013, E2E-016, E2E-017, E2E-021 |
| SC-003 | FR-NEW-005, 006 | E2E-007 | E2E-008 (EXC-003a) | E2E-009 (EXC-003b), E2E-020 |
| SC-004 | FR-NEW-001, 002, 005 | E2E-004 | E2E-019 (EXC-004a) | E2E-005 |
| SC-005 | FR-NEW-001, 002, 004, 009 | E2E-003 | E2E-006 (EXC-005a) | E2E-018 (EXC-005b) |

Requirement to test (each at least 3): FR-NEW-001: E2E-001, 003, 011, 014, 015, 017, 018, 019. FR-NEW-002: E2E-001, 003, 004, 005, 006, 014, 019, 020. FR-NEW-003: E2E-002, 009, 013, 016, 017, 021. FR-NEW-004: E2E-003, 006, 015, 018. FR-NEW-005: E2E-002, 004, 007, 008, 010, 020. FR-NEW-006: E2E-007, 008, 009. FR-NEW-007: E2E-001, 002, 010, 021. FR-NEW-008: E2E-001, 011, 012. FR-NEW-009: E2E-001, 003, 006, 014.

## 11. Open questions

None.

## 12. Glossary (MANDATORY)

| Term | Definition | Context |
|---|---|---|
| damaged page | a page whose content, or one of its parts, or a form it draws, is cut or holds bytes that cannot be read as page content | Document reading |
| note | the free text line of the extraction answer that already exists today | Document reading |
| stored text copy | the text of an extracted document, kept next to it and used to answer a repeated extraction | Stored text copies |
| refresh option | the option of text extraction that reads the document again instead of using its stored text copy | Stored text copies |
| page | one numbered sheet of the PDF, numbered from 1 as in the page markers | Document reading |
| page marker | the line `*[Page N]*` placed before the text of each page | Document reading |
| text extraction | the operation that turns a document into text and returns an answer | Document reading |
| answer | what text extraction returns to the member: text or preview, page count, title and note | Document reading |
| assistant tool interface | the way an assistant or program calls text extraction as a tool | Document reading |
| web API | the way a program calls text extraction over the web | Document reading |
| maximum length | the number of characters of text the member asks to get back | Document reading |
| form | a reusable block of page content that a page draws by name | Document reading |
| part (of a page) | one of the pieces of content a page is made of when it has several | Document reading |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 5 | 3 | NOT-IMPLEMENTABLE |
| 2 | 3 | 2 | NOT-IMPLEMENTABLE |
| 3 | 1 | 0 | NOT-IMPLEMENTABLE |
| 4 | 1 | 2 | NOT-IMPLEMENTABLE |

**Amendments applied:** round 1: FR-NEW-001 damage rule and whole PDF listing, FR-NEW-004 wording, FR-NEW-005 cached answer, E2E-012 verbatim refusal, E2E-017 to E2E-019, Section 10 columns, glossary. Round 2: stored copy never exists for empty text (FR-NEW-005, FR-NEW-007 rules), E2E-010 and E2E-013 rewritten, E2E-020. Round 3: the note is present even when empty on a PDF cached answer and absent on a non PDF one (FR-NEW-005), E2E-002 and E2E-008 asserts, EXC-002a, wording fixes.
Round 4 (granted by the owner beyond the three round cap): F on the undefined status of undecodable streams and dangling references. Two consecutive rounds returned only new F findings, so this was settled as a scope decision, not a fifth round: those cases are out of scope and stay as today (FR-NEW-001 rule, Section 3.2). Test gaps found by the reviewer closed with E2E-021 and a second extraction in E2E-013.
**Open:** the round 4 changes were not re audited.
**No implementation detail outside Section 14:** PASS with MINOR notes (PDF format terms and the verbatim refusal message are product terms).

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Page content is parsed leniently and a parse refusal ends the page quietly | `crates/core/src/docs/pdf_bound.rs:1514` (`let Ok(parsed) = Content::decode(&content) else { return Ok(page.into_walked(invalid)) }`) |
| Form content parse refusal ends the form quietly | `crates/core/src/docs/pdf_bound.rs:1604` (`let Ok(parsed) = Content::decode(&bytes) else { break };`) |
| The lenient decode drops the unparsed rest; a strict decode exists | `lopdf-0.42.0/src/parser/mod.rs:31` (`strip_nom` drops the rest), `:734-742` (`content_strict` refuses a non empty rest), `lopdf-0.42.0/src/parser_aux.rs:20-26` (`decode`, `decode_strict`) |
| Only a few damages are refused today (`BI ID`) | `crates/core/src/tools/nesting_e2e.rs:999-1002` (`parity_do_unparsable_form`), `sdlc/backlog/BL-0154-pdf-damaged-content-is-accepted-instead-of-reported.md` (Evidence section) |
| The answer carries text, meta (`pages`, `title`), `truncated` and a note | `crates/core/src/docs/extract.rs:69-77` (`ExtractResult`), `:582-598` (`into_payload` adds `truncated`, `meta`, `note`) |
| The scanned PDF message | `crates/core/src/docs/extract.rs:805-809` |
| Page markers `*[Page N]*` | `crates/core/src/docs/pdf_bound.rs:224` |
| A repeated extraction is answered from the stored copy with no note and no meta | `crates/core/src/docs/extract.rs:628-648` (`Prepared::Cached(doc_payload(...))`), `:674-690` (`doc_payload` has no `note`) |
| `refresh` skips the stored copy | `crates/core/src/docs/extract.rs:602-626` (`!refresh` condition) |
| The built in reader is reached only by this extraction | `crates/core/src/core/fs_ops.rs:1739` (`prepare_extraction`), `crates/core/src/api/dataplane.rs:171,1487` (REST `extract-text`), `crates/core/src/mcp/server.rs:2402` (`fs_extract_text`); `rg 'pdf_bound' crates/core/src --glob '!*test*'` lists docs/mod.rs, docs/pdf_bound.rs, docs/extract.rs, tools/mod.rs only |
| Forms nested limit and message | `crates/core/src/docs/pdf_bound.rs:1472` (`FORM_DEPTH_LIMIT = 64`), `:1597-1603` |
| Existing PDF tests, none about damaged content | `crates/core/src/docs/extract.rs:2315` (`pdf_extracts_text_pages_and_metadata`), `crates/core/src/tools/nesting_e2e.rs`, `crates/core/src/tools/pdf_bound_e2e.rs` |

Test command: `./test.sh` (`cargo test --workspace`); PDF bound tests also `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0089_ e2e_0092_ --test-threads=1` (AGENTS.md).

### 14.2 Class sweep
Not applicable (no `--security`).

### 14.3 Design inputs
- Detection candidate: `Content::decode_strict` refuses a non empty rest (`lopdf-0.42.0/src/parser/mod.rs:734-742`). Signal that reveals a false positive: E2E-002, E2E-016 and every existing PDF fixture must stay clean; the design must run the strict check over all PDF fixtures of the test tree before choosing it.
- The page walk concatenates a page's parts with a line break before parsing (`pdf_bound.rs:1498-1514`), so the part that is damaged is not known there. E2E-015 needs a part level or whole page strict check to list page 2; both give the same page list.
- The stored copy is the text only, and the cached answer builds a payload with no note (`extract.rs:639-646`). FR-NEW-005 needs the damage note to survive; where it is stored (in the copy, next to it, in a metadata row) is a design decision, and must not change the copy's text bytes unless the design declares and tests it.
- Walk output must reach the answer: `pdf_bound::extract` returns `PdfText` (`pdf_bound.rs:197`); the page list must travel through it to `pdf()` in `extract.rs:793-810`.
- The note joins the existing scanned message with one space (FR-NEW-007); `with_note` today overwrites (`extract.rs:89-92`).
- Cost: the strict parse runs on every page content and form once more; measure on the largest fixture, no numeric promise in this spec (7.1).
- E2E-010 exact fixture: a content stream consisting of `(` only; confirm the reader returns empty text for it (ASSUMED: lenient decode returns no operations).
- The note text uses `Content::decode_strict` failure only as a signal; if two parts of a page are parsed separately, the page list must still be unique and ascending (FR-NEW-004).
- Grammar edge cases of FR-NEW-001 rest on the content grammar of lopdf 0.42 (`parser/mod.rs:721-742`): ASSUMED that an unknown operator word and an unclosed `q` are accepted and that a stray `>>` is not. Signal: E2E-017 and E2E-018 fail if wrong; the design must confirm with a first failing test.
- The cached answer today has no note, no meta and no page count (`extract.rs:674-690`); FR-NEW-005 adds only the note. Where the note of the storing extraction is kept is a design decision (see the bullet on the stored copy above).
- Pages are all walked before the text is cut to the maximum length (`pdf_bound.rs:209-212` walks every page; the cut happens in the extractor), which is why FR-NEW-001 can list pages beyond the returned text (E2E-019). ASSUMED for the cut location; signal: E2E-019.
- The refusal text of E2E-012 is built as `could not extract {path}: {message}` (`extract.rs:657-662`); the path is the member's normalized path with a leading slash.
- A PDF with empty text gets no stored text copy (`extract.rs:556-570`, `md_path` filters empty text; empty pages also lose their page marker, `pdf_bound.rs:128-131`), so a cached answer can only carry a damage note or none. Signal: E2E-010.
- Presence of the note in a cached answer: the fresh answer always carries a `note` key (empty string when none; `crates/core/src/tools/offload_e2e.rs:1109` snapshots); the cached answer built by `doc_payload` has none (`extract.rs:674-690`). FR-NEW-005 requires the key present with the empty string on a PDF cached answer and no key on a non PDF cached answer. Signal: E2E-002, E2E-008, E2E-013.
- Fixture tokens in E2E-015, E2E-017 and E2E-018 (`BT`, `Tj`, `ET`, `q`, `Zz`, `>>`) are PDF content syntax, not design constraints.
- Re verify the 14.1 citations (notably `pdf_bound.rs:1472`, `:1490-1504`, `:1603`) against the code at design time.
- Out of scope by FR-NEW-001: undecodable compression, dangling content references and unreadable page entries (`pdf_bound.rs:1490`, `:1500-1504`, `:1603`) keep today's quiet behavior.
