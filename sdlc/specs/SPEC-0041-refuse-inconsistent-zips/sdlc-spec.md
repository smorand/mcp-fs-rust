# Refuse zip archives whose headers disagree: functional specification

> Id: SPEC-0041
> Nature: FEAT
> Depth: M
> Depth evidence: FEAT / depth M: 2 features (archive extraction, document text extraction), 6 requirements, one modified rule from SPEC-0036, no persisted change
> Status: implemented
> Generated on: 2026-10-19
> Project: existing
> From backlog: BL-0063
> Depends on: SPEC-0036 (implemented)
> Security: internal finding (hardening: archives whose directory and entry headers disagree let two readers see different contents)
> CVSS: not scored
> Affected: every release reading zips with header checks off, including main at de60a5f
> Fixed in: unreleased

## 1. Summary
A project member who extracts a zip archive, converts a document with the document service, or extracts the text of a Word, Excel or PowerPoint document (`.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx`, `.ppsx`), gets the archive as its directory describes it, even when the entries themselves say something else. This increment refuses such malformed archives with a clear message, before anything is written, so what the server stores always matches what any other tool would read.

## 2. Current State
### 2.1 What users can do today
- A member extracts a zip archive into their project. When an entry's own header disagrees with the archive's directory (another name, size, compression or flags), the archive is extracted the way the directory describes it, with no warning.
- When two entries of an archive carry the exact same name, only the last one is extracted, silently.
- A member extracts the text of a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` document. A part of the document that cannot be read is silently left out of the text.
- A corrupt archive is refused with `archive is corrupt or not a valid zip file: ` followed by a detail; a corrupt document with `could not extract <path>: not a valid OOXML package: ` followed by a detail. An encrypted archive given without a password is answered `password required to extract this archive`.
- Two entries with different stored names that read as the same name are already refused with `archive is corrupt or not a valid zip file: duplicate entry name '<name>'`, before the password is asked (SPEC-0036/DR-009).
- A member converts a document with the document service (on demand, or when uploading or writing with the conversion option); the document is handed to the converter as is, malformed or not.
### 2.2 Specs governing this area
- SPEC-0015 (archive extraction): refusal before any write, the corrupt archive message, the password rules.
- SPEC-0036/DR-008: identical raw names keep the last entry. **This spec reverses it** (FR-MOD-001).
### 2.3 What is verified today
Archive extraction has refusal, password and name decoding tests; SPEC-0036 DT-039 case b asserts the "last entry wins" rule. No test covers an archive whose two headers disagree, nor a malformed document.

## 3. Scope
### 3.1 In scope
- Archive extraction refuses an archive with any header inconsistency or any duplicated entry name.
- Document text extraction refuses such a document.
- Document conversion refuses such a document before it reaches the converter.
### 3.2 Out of scope, and why
- Documents the converter accepts that are not zip based (`.doc`, `.ppt`, PDF, audio, video): nothing to check.
- Zips the server itself writes (export, generated documents, folder download): never malformed by construction.
- Tar and 7z archives: they have no second directory to disagree with.
- Repairing a malformed archive: refusing is the safe outcome; the user rebuilds it.
### 3.3 Constraints imposed by the context
None.

## 4. Actors
- **Member:** a person with access to a project, extracting archives and document text.

## 4.5 Bounded contexts
| Context | What it owns | Key concepts |
|---|---|---|
| Project files | the files of a project and their extraction | archive, entry, document, text extraction |

## 5. Usage scenarios
### SC-001: Extract an archive
**Actor:** Member
**Preconditions:** the archive is a file in the project; the destination holds no file of the same names.
**Flow:**
1. The member asks to extract the archive → the server checks every entry of the archive against the archive's directory.
2. Every entry agrees and every name is unique → the entries are written, as today.
**Postconditions:** the files of the archive exist in the destination.
**Exceptions:**
- EXC-001a: one entry disagrees with the directory → refused with `archive is corrupt or not a valid zip file: header inconsistency detected` followed by a detail; nothing is written.
- EXC-001b: two entries carry the same name → refused with the same message prefix; nothing is written.
- EXC-001c: the archive is encrypted, malformed, and no password is given → `password required to extract this archive`, as today; given the right password, it is refused as in EXC-001a.
### SC-002: Extract the text of a document
**Actor:** Member
**Preconditions:** a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` file in the project.
**Flow:**
1. The member asks for the document's text → the server checks every part of the document.
2. Every part agrees and every name is unique → the text is returned, as today.
**Postconditions:** the text is returned; nothing stored changes.
**Exceptions:**
- EXC-002a: one part disagrees, or two parts share a name, even a part the extraction would not read → refused with `could not extract <path>: not a valid OOXML package: header inconsistency detected` followed by a detail; no text is returned.

### SC-003: Convert a document
**Actor:** Member
**Preconditions:** the document service is enabled; a `.docx`, `.pptx`, `.pptm`, `.potx` or `.ppsx` file in the project.
**Flow:**
1. The member converts the document on demand, or uploads or writes it with the conversion option → the server checks every part before calling the converter.
2. Every part agrees and every name is unique → converted, as today.
**Postconditions:** the companion text exists.
**Exceptions:**
- EXC-003a: on demand, the document is malformed → refused with `could not convert <path>: not a valid OOXML package: header inconsistency detected` followed by a detail; the converter is not called; no companion is written.
- EXC-003b: on upload or write with the conversion option, the document is malformed → the file itself is stored as today, and the conversion report carries the same refusal; the converter is not called; no companion is written.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-O]: Archive with disagreeing headers refused
> IF any entry of a zip archive disagrees with the archive's directory on its name, alternate name, compression, encryption flag, AES encryption settings, trailing size record flag, last modified time, checksum, compressed size or uncompressed size, or two entries occupy overlapping bytes of the archive, THEN the product SHALL refuse the extraction before writing any file.
- **User visible text:** `archive is corrupt or not a valid zip file: header inconsistency detected: <reason>`, where `<reason>` names what differs (the field, `duplicate file names`, or `overlapping entries`); tests assert the prefix up to `header inconsistency detected`.
- **Business rules:** every entry is checked, not only the ones that would be written; one bad entry refuses the whole call. When an archive both holds a disqualifying entry (symlink, unsafe path, or distinct names reading alike) and has a header inconsistency, the existing refusal for that entry is returned, as today; the header inconsistency refusal applies only to archives that pass those checks and, if encrypted, have been given a password. An encrypted archive given a wrong password that also has a header inconsistency is refused with the header inconsistency message: the check reads no encrypted content and runs before any entry is decrypted.
- **Scenario:** SC-001 · **Priority:** Must
#### FR-NEW-002 [EARS-O]: Document with disagreeing or duplicated parts refused
> IF any part of a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` document disagrees with the document's directory on its name, alternate name, compression, encryption flag, AES encryption settings, trailing size record flag, last modified time, checksum, compressed size or uncompressed size, or two parts occupy overlapping bytes of the archive, or two parts share a name, THEN the product SHALL refuse the text extraction and return no text.
- **User visible text:** `could not extract <path>: not a valid OOXML package: header inconsistency detected: <reason>`, `<path>` being the document's project path and `<reason>` as in FR-NEW-001; tests assert the prefix up to `header inconsistency detected`.
- **Business rules:** every part is checked, including parts the extraction does not read. A document whose text was already extracted and stored is still checked before that stored text is returned: a malformed document is refused even then, and the stored text is left as is.
- **Scenario:** SC-002 · **Priority:** Must
#### FR-NEW-003 [EARS-U]: Password asked before the structural refusal
> The product SHALL answer an encrypted archive given without a password with `password required to extract this archive`, whether or not the archive is malformed.
- **Business rules:** the SPEC-0036/DR-009 refusal (distinct stored names reading as the same name) keeps its message and its place before this answer.
- **Scenario:** SC-001 · **Priority:** Should
#### FR-NEW-004 [EARS-U]: Well formed archives and documents unchanged
> The product SHALL extract every well formed archive and document exactly as before this change.
- **Scenario:** SC-001, SC-002 · **Priority:** Must
#### FR-NEW-005 [EARS-O]: Malformed document refused before conversion
> IF a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` document sent to the document service is malformed in the sense of FR-NEW-002 THEN the product SHALL refuse the conversion without calling the converter and without writing a companion.
- **User visible text:** `could not convert <path>: not a valid OOXML package: header inconsistency detected: <reason>`, `<reason>` as in FR-NEW-001; tests assert the prefix up to `header inconsistency detected`.
- **Business rules:** on demand conversion fails with that refusal. Conversion triggered by an upload or write still stores the uploaded file, exactly as a conversion failure does today, and reports the refusal in the conversion report instead of a companion path. Documents that are not zip based are converted as today.
- The refusal carries the error code `ERR_INVALID_ARGUMENT` (HTTP 400 on the REST plane), as the archive and text refusals do. On upload or write, the conversion report is `{"error": {"code": "ERR_INVALID_ARGUMENT", "message": "could not convert <path>: not a valid OOXML package: header inconsistency detected: <reason>"}}`.
- The check applies whenever the document's extension is one of the seven document extensions of FR-NEW-002, whatever the operator's list of accepted extensions; any other extension is converted as today.
- The existing refusals (service not configured, extension not accepted, input over the size limit) keep their place and message and come first; the malformed check runs after them and never prevents the uploaded file from being stored.
- **Scenario:** SC-003 · **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-O]: Duplicated entry names refused (modifies SPEC-0036/DR-008)
> IF two entries of a zip archive carry the same name THEN the product SHALL refuse the extraction before writing any file.
- **Business rules:** "same name" means byte for byte identical stored names; the existing DR-009 refusal of distinct stored names that read alike keeps its message `duplicate entry name '<name>'` and its order.
- **Before:** the last entry of that name is extracted, silently (Section 2.1).
- **After:** refused with `archive is corrupt or not a valid zip file: header inconsistency detected` followed by a detail.
- **Reason:** silently dropping an entry the user put in the archive hides data; a duplicate is a sign of a crafted archive.
### 6.3 Removed
None.

## 7. Non functional requirements
### 7.2 Security and access
- A refused archive or document leaves the project exactly as it was: no file, no partial result.
### 7.5 Reliability
- The check covers the whole archive; a refusal never depends on which entry the reader reached first.

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Data variations | entry whose two headers differ only in last modified time | refused | E2E-011 |
| Data variations | macro enabled workbook with a malformed part | refused | E2E-012 |
| Data variations | two directory rows pointing at the same bytes | refused | E2E-019 |
| Boundary values | only the last of 3 entries is malformed | refused, none of the 3 written | E2E-005 |
| Partial operations | refusal mid archive | nothing written | E2E-002, E2E-005 |
| Data variations | malformed part the extraction never reads | refused | E2E-010, E2E-012 |
| Access states | encrypted malformed archive, no password then password | password asked, then refused | E2E-006 |
| Empty states | archive with no entries | unchanged behavior, not in scope of the check | not applicable: no entry to disagree |
| Concurrent actions, connectivity, navigation, ordering, exhaustion, dependency failures | | not applicable: a single local read with no shared state or remote dependency | |

## 9. Acceptance tests
### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-004 | Critical |
| E2E-002 | New | failure, side effect | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | failure | SC-001 | FR-NEW-001 | High |
| E2E-004 | New | failure, side effect | SC-001 | FR-MOD-001 | Critical |
| E2E-005 | New | edge | SC-001 | FR-NEW-001 | High |
| E2E-006 | New | edge | SC-001 | FR-NEW-003, FR-NEW-001 | Medium |
| E2E-007 | New | journey | SC-002 | FR-NEW-004 | Critical |
| E2E-008 | New | failure | SC-002 | FR-NEW-002 | Critical |
| E2E-009 | New | failure | SC-002 | FR-NEW-002 | High |
| E2E-010 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-011 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-012 | New | edge | SC-002 | FR-NEW-002 | Medium |
| E2E-SEC-001 | New | security | SC-001 | FR-NEW-001 | Critical |
| E2E-SEC-002 | New | security | SC-002 | FR-NEW-002 | Critical |
| E2E-013 | New | failure | SC-002 | FR-NEW-002 | High |
| E2E-014 | New | failure | SC-001 | FR-NEW-003 | Medium |
| E2E-015 | New | failure | SC-001 | FR-NEW-003 | Medium |
| E2E-016 | New | edge | SC-001 | FR-MOD-001 | Medium |
| E2E-017 | New | edge | SC-001 | FR-NEW-004 | High |
| E2E-018 | New | failure | SC-001 | FR-NEW-001, FR-NEW-003 | Medium |
| E2E-019 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-020 | New | journey | SC-003 | FR-NEW-005, FR-NEW-004 | High |
| E2E-021 | New | failure, side effect | SC-003 | FR-NEW-005 | Critical |
| E2E-022 | New | edge, side effect | SC-003 | FR-NEW-005 | High |
| E2E-023 | New | edge | SC-003 | FR-NEW-005 | Medium |
| E2E-024 | New | failure | SC-003 | FR-NEW-005 | High |
| E2E-MOD-001 | Modified | failure | SC-001 | FR-MOD-001 | High |

Coverage: happy 3, failure 12 (E2E-002, 003, 004, 008, 009, 013, 014, 015, 018, 021, 024, MOD-001), side effect 4, edge 10, security 2 (E2E-SEC-001, 002). Happy to failure ratio 1:4.

### 9.2 New tests
#### E2E-001: well formed archive extracts unchanged
- **Category:** journey · **Scenario:** SC-001 · **Requirements:** FR-NEW-004
- **Given** Olivia, a member of project `atlas`, and `/in/good.zip` holding `a.txt` (`alpha`) and `b.txt` (`beta`)
- **When** she extracts it into `/out`
- **Then** `/out/a.txt` reads `alpha` and `/out/b.txt` reads `beta`
- **Priority:** Critical
#### E2E-002: an entry named differently in its own header is refused
- **Category:** failure, side effect · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/mismatch.zip` whose directory lists `a.txt` (`alpha`) while that entry's own header names it `b.txt`
- **When** Olivia extracts it into `/out`
- **Then** she is refused with a message starting `archive is corrupt or not a valid zip file: header inconsistency detected`
- **And** listing the project root shows no `out` folder
- **Priority:** Critical
#### E2E-003: an entry with different sizes or compression in its two headers is refused
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/sizes.zip` whose single entry `a.txt` declares a different compressed size in its own header than in the directory
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** High
#### E2E-004: two entries with the same name are refused
- **Category:** failure, side effect · **Scenario:** SC-001 · **Requirements:** FR-MOD-001
- **Given** `/in/dup.zip` holding two entries both named `dup.txt`, contents `first` and `second`
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002
- **And** listing the project root shows no `out` folder
- **Priority:** Critical
#### E2E-005: the last of three entries malformed, nothing written
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/late.zip` holding `a.txt`, `b.txt`, `c.txt`, only `c.txt` named `d.txt` in its own header
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; none of `/out/a.txt`, `/out/b.txt`, `/out/c.txt` exists
- **Priority:** High
#### E2E-006: encrypted and malformed: password first, then refused
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** `/in/locked.zip`, entry `a.txt` encrypted with password `s3cret`, named `b.txt` in its own header
- **When** Olivia extracts it into `/out` without a password, then again with `s3cret`
- **Then** the first attempt answers `password required to extract this archive`; the second is refused with the message of E2E-002; no `out` folder exists after either
- **Priority:** Medium
#### E2E-007: well formed document text unchanged
- **Category:** journey · **Scenario:** SC-002 · **Requirements:** FR-NEW-004
- **Given** `/docs/memo.docx`, a well formed Word document whose body is `hello`
- **When** Olivia extracts its text
- **Then** the returned text contains `hello`
- **Priority:** Critical
#### E2E-008: document whose main part is named differently is refused
- **Category:** failure · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/bad.docx`, the document of E2E-007 with its main text part named differently in that part's own header
- **When** Olivia extracts its text
- **Then** refused with a message starting `could not extract /docs/bad.docx: not a valid OOXML package: header inconsistency detected`; no text is returned
- **Priority:** Critical
#### E2E-009: document with two parts of the same name is refused
- **Category:** failure · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/dup.docx`, the document of E2E-007 with its main text part present twice under the same name
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **Priority:** High
#### E2E-010: a malformed part the extraction does not read still refuses
- **Category:** edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/props.docx`, the document of E2E-007 plus a properties part named differently in its own header
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **Priority:** Medium

#### E2E-011: headers differing only in modification time are refused
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/time.zip` whose single entry `a.txt` carries a different modification time in its own header than in the directory
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** Medium
#### E2E-012: macro enabled workbook with a malformed part is refused
- **Category:** edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/bad.xlsm`, a workbook whose worksheet part is named differently in its own header
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **Priority:** Medium

#### E2E-013: previously stored text does not bypass the check
- **Category:** failure · **Scenario:** SC-002 · **Requirements:** FR-NEW-002
- **Given** `/docs/stale.docx`, malformed as in E2E-008, whose text `hello` was extracted and stored before this change
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path; no text is returned
- **Priority:** High

#### E2E-014: encrypted well formed archive without password is asked for it
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-003
- **Given** `/in/pw.zip`, entry `a.txt` (`alpha`) encrypted with `s3cret`, headers consistent
- **When** Olivia extracts it into `/out` without a password
- **Then** answered `password required to extract this archive`; no `out` folder exists
- **Priority:** Medium
#### E2E-015: distinct names reading alike are refused before the password is asked
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-003
- **Given** `/in/pwdup.zip`, encrypted with `s3cret`, holding an entry whose stored name is `é.txt` in UTF-8 flagged as UTF-8, and a second whose stored bytes differ but read as `é.txt` in CP437
- **When** Olivia extracts it into `/out` without a password
- **Then** refused with `archive is corrupt or not a valid zip file: duplicate entry name 'é.txt'`; no `out` folder exists
- **Priority:** Medium
#### E2E-016: three entries with the same name, the distinct sibling not written
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-MOD-001
- **Given** `/in/dup3.zip` holding `keep.txt` (`k`) and `dup.txt` three times
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; `/out/keep.txt` does not exist
- **Priority:** Medium
#### E2E-017: a well formed archive whose sizes follow the data extracts unchanged
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-004
- **Given** `/in/stream.zip`, written in streaming mode so `a.txt` (`alpha`) carries its sizes after its data, with zeros in its own header
- **When** Olivia extracts it into `/out`
- **Then** `/out/a.txt` reads `alpha`
- **Priority:** High

#### E2E-018: encrypted malformed archive with a wrong password
- **Category:** failure · **Scenario:** SC-001 · **Requirements:** FR-NEW-001, FR-NEW-003
- **Given** `/in/locked.zip` of E2E-006
- **When** Olivia extracts it into `/out` with the password `wrong`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** Medium
#### E2E-019: overlapping entries are refused
- **Category:** edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001
- **Given** `/in/overlap.zip` whose directory lists `a.txt` and `b.txt` both pointing at the same entry header and data, each header consistent with its own directory row
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002; no `out` folder exists
- **Priority:** Medium

#### E2E-020: well formed document converts as today
- **Category:** journey · **Scenario:** SC-003 · **Requirements:** FR-NEW-005, FR-NEW-004
- **Given** the document service enabled and `/docs/memo.docx` of E2E-007
- **When** Olivia converts it on demand
- **Then** the companion `/docs/memo.docx.md` exists and the converter was called once
- **Priority:** High
#### E2E-021: malformed document refused on demand, converter not called
- **Category:** failure, side effect · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled and `/docs/bad.docx` of E2E-008
- **When** Olivia converts it on demand
- **Then** refused with code `ERR_INVALID_ARGUMENT` and a message starting `could not convert /docs/bad.docx: not a valid OOXML package: header inconsistency detected`
- **And** the converter was never called, and no `/docs/bad.docx.md` exists
- **Priority:** Critical
#### E2E-022: malformed deck uploaded with conversion: file stored, conversion refused
- **Category:** edge, side effect · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled and a `.pptx` deck whose slide part is named differently in its own header
- **When** Olivia uploads it as `/docs/deck.pptx` with the conversion option
- **Then** `/docs/deck.pptx` is stored with the uploaded bytes; the report's `documentation.error.code` is `ERR_INVALID_ARGUMENT` and its `documentation.error.message` starts with `could not convert /docs/deck.pptx: not a valid OOXML package: header inconsistency detected`
- **And** the converter was never called, and no `/docs/deck.pptx.md` exists
- **Priority:** High

#### E2E-023: operator added extension still checked
- **Category:** edge · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled with its accepted list set to `.xlsm`, and `/docs/bad.xlsm` of E2E-012
- **When** Olivia converts it on demand
- **Then** refused with code `ERR_INVALID_ARGUMENT` and a message starting `could not convert /docs/bad.xlsm: not a valid OOXML package: header inconsistency detected`; the converter was never called; no `/docs/bad.xlsm.md` exists
- **Priority:** Medium
#### E2E-024: document with two parts of the same name refused on demand
- **Category:** failure · **Scenario:** SC-003 · **Requirements:** FR-NEW-005
- **Given** the document service enabled and `/docs/dup.docx` of E2E-009
- **When** Olivia converts it on demand
- **Then** refused with a message starting `could not convert /docs/dup.docx: not a valid OOXML package: header inconsistency detected`; the converter was never called; no `/docs/dup.docx.md` exists
- **Priority:** High

### 9.3 Exploit tests
#### E2E-SEC-001: a disguised entry cannot be smuggled in by archive extraction
- **Covers:** class sweep occurrence #1 (Section 14.2)
- **Red before the fix because:** today `/out/a.txt` is written with `alpha` and the refusal assertion fails.
- **Given** `/in/smuggle.zip` whose directory lists `a.txt` (`alpha`) while the entry's own header names it `../../evil.txt`
- **When** Olivia extracts it into `/out`
- **Then** refused with the message of E2E-002
- **And** a search of the whole project for `*.txt` finds no file written by this call
#### E2E-SEC-002: a disguised part cannot change the text a document yields
- **Covers:** class sweep occurrence #2 (Section 14.2)
- **Red before the fix because:** today the text `hello` is returned and the refusal assertion fails.
- **Given** `/docs/smuggle.docx`, the document of E2E-007 whose main part is named `word/other.xml` in its own header
- **When** Olivia extracts its text
- **Then** refused with the message of E2E-008, carrying its own path
- **And** the document's stored companion text, read separately, is absent (no extraction result was kept)

### 9.4 Modified tests
#### E2E-MOD-001: identical names refused (was SPEC-0036 DT-039 case b)
- **Validated before:** two entries with identical raw name `é.txt`, the first flagged as UTF-8 (`first`), the second not (`second`), extract one file `/out/é.txt` reading `second`.
- **Validates now:** the same archive is refused with the message of E2E-002 and no `out` folder exists.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-MOD-001 | E2E-001 | E2E-002, E2E-003, E2E-004, E2E-014, E2E-015, E2E-018, E2E-MOD-001, E2E-SEC-001 | E2E-005, E2E-006, E2E-011, E2E-016, E2E-017, E2E-019 |
| SC-003 | FR-NEW-005, FR-NEW-004 | E2E-020 | E2E-021, E2E-024 | E2E-022, E2E-023 |
| SC-002 | FR-NEW-002, FR-NEW-004 | E2E-007 | E2E-008, E2E-009, E2E-013, E2E-SEC-002 | E2E-010 |

## 11. Open questions
None.

## 12. Glossary
| Term | Definition | Context |
|---|---|---|
| Archive | a zip file stored in a project | Project files |
| Directory | the list at the end of an archive naming every entry | Project files |
| Entry | one file stored inside an archive, with its own header | Project files |
| Document | a `.docx`, `.xlsx`, `.xlsm`, `.pptx`, `.pptm`, `.potx` or `.ppsx` file, itself an archive of parts | Project files |
| Part | an entry of a document | Project files |
| Header inconsistency | an entry whose own header disagrees with the directory, or two entries sharing a name | Project files |

## 13. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 7 | 0 | 1 | IMPLEMENTABLE: A1 citation fixed; FR-NEW-005 statement aligned to its seven extension rule |
| 6 | 4 | 1 | NOT-IMPLEMENTABLE: F1 error code, F2 operator extensions (E2E-023), F3 order against existing refusals, F4 E2E-024; A1 REST lines; all applied |
| 5 | | | owner chose (b): FR-NEW-005, SC-003, E2E-020..022 added |
| 4 | 3 | 1 | NOT-IMPLEMENTABLE (confirmation round, not committed): F2 wrong password order (E2E-018), F3 overlap test (E2E-019), A1 citation, applied; F1 external document converter scope awaits the owner |
| 3 | 3 | 2 | NOT-IMPLEMENTABLE (cap reached, not committed): F1 user visible document message carries the `could not extract <path>: ` wrapper, F2 E2E-014/015, F3 E2E-016/017; A1 sweep row fixed, A2 wrapper cited; all applied, awaiting owner approval for a confirmation round |
| 2 | 3 | 3 | NOT-IMPLEMENTABLE: F1 stored text still checked (E2E-013), F2 order against existing refusals, F3 AES settings in both lists; A1 to A3 applied |
| 1 | 4 | 3 | NOT-IMPLEMENTABLE: F1 field list made exhaustive (zip's own check), F2 DR-009 message and order kept, F3 seven extensions, F4 message shape fixed; A1 summary rows, A2 this row, A3 sweep readers, all applied |

**No implementation detail outside Section 14:** PASS (file extensions named are the user facing document kinds).

## 14. Evidence appendix
### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| archive extraction reads with default options, check off | `crates/core/src/tools/archive.rs:402`, `:453`, `:467`; `zip-9.0.5/src/read/mod.rs:251`, `:270` |
| identical names keep the last entry | `zip-9.0.5/src/read/zip_archive.rs:82-86`; test `dt_039_b_identical_raw_names_keep_the_last_entry` (`archive.rs:2494`) |
| document text skips an unreadable part | `crates/core/src/docs/extract.rs:545-548` |
| corrupt archive message | `crates/core/src/tools/archive.rs:350` (`corrupt`) |
| corrupt document message | `crates/core/src/docs/extract.rs:541` (inner), wrapped as `could not extract <path>: ...` at `:358-362` |
| password message, checked before any part is read | `crates/core/src/tools/archive.rs:440-442` |
Test command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`

### 14.2 Class sweep
**Class:** untrusted archive structure (entry metadata that disagrees between two places in the same archive) reaching a reader that trusts only one of them (CWE-436, interpretation conflict).
**Searches run:**
1. `rg -n "ZipArchive::new|ZipStreamReader|read_zipfile_from_stream" crates --type rust`: every place a zip is opened (16 hits).
2. `rg -n "by_index\(|by_index_decrypt|by_name\(|by_index_raw" crates/core/src --type rust`: every way an entry is read.
**Not searched:** tar and 7z readers: no second directory exists to disagree with.

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | `crates/core/src/tools/archive.rs:402` archive extraction | vulnerable: reads entries with the check off | FR-NEW-001, FR-MOD-001 |
| 2 | `crates/core/src/docs/extract.rs:539` (`open_zip`), used by the readers at `:697` (docx), `:849` (pptx family), `:968` (xlsx family), routed from seven extensions at `:167-168` | vulnerable: reads parts with the check off, skips failures | FR-NEW-002 |
| 4 | `crates/core/src/core/fs_ops.rs:688`, `:635` document conversion | vulnerable: hands the bytes to an external converter unchecked | FR-NEW-005 (owner decision 2026-10-19) |
| 3 | `exports.rs:243`, `tools/export.rs:280`, `docs/docx.rs:379`, `:397`, `:422`, `api/dataplane.rs:2508`, `:2637`, `:2911`, `tools/artifacts.rs:4152`, `tools/archive.rs:975`, `:1311`, `:2420`, `:2448` | safe: test code reading archives the server built itself | none |

### 14.3 Design inputs
- Conversion entry points: on demand `crates/core/src/core/fs_ops.rs:688` (`documentize`, a failure is the result); on write or upload `fs_ops.rs:635` (`write_bytes_documented`, the source is stored and a failure lands in the `documentation` report, `:671`); REST at `crates/core/src/api/dataplane.rs:583` (multipart upload), `:1456` (write with conversion), `:1488` (documentize); the operator list replaces the built in one when set (`crates/core/src/config.rs:851-854`). Zip based converter extensions: `docs/service.rs:44-48` (`.pptx`, `.pptm`, `.potx`, `.ppsx`, `.docx`); the check reuses FR-NEW-002's.
- zip 9's whole archive check covers FR-NEW-001's list including AES settings (`zip-9.0.5/src/read/mod.rs:356-471`, `zip_archive.rs:754-768`); its error reads `invalid Zip archive: header inconsistency detected: ...` (`zip-9.0.5/src/result.rs:70`), so the `invalid Zip archive: ` prefix is dropped to form the user visible text.
- Document text returns stored text without opening the document (`crates/core/src/core/fs_ops.rs:1709-1723`, `Prepared::Cached`); the check must run before that short circuit for the seven document extensions (detected by E2E-013).
- The existing refusals (symlink, unsafe path, DR-009) run in the pre scan at `crates/core/src/tools/archive.rs:417-427`, before the password answer at `:440`.
- zip 9 offers the check per read (`ZipReadOptions::enforce_header_consistency`, `zip-9.0.5/src/read/mod.rs:324`) and once for the whole archive (`ZipArchive::validate_header_consistency`, `zip_archive.rs:754`); FR-NEW-001 and FR-NEW-002 need the whole archive variant or an equivalent, since parts the reader skips must count.
- The password rule (FR-NEW-003) means the whole archive check of an encrypted archive runs after the existing missing password answer (`archive.rs:440`).
- `entry_text` (`extract.rs:545-548`) swallows read errors; a header error must surface instead.
