# Document artifacts (images and tables of an unchanged document): functional specification

> Id: SPEC-0019
> Nature: FEAT
> Depth: L
> Depth evidence: FEAT / depth L: 4 areas touched (document conversion, file changes, quota, access), a new persisted concept (document artifacts), 39 requirements. Rescoped after gate round 5 (non convergence): artifacts following their source moved to BL-0038.
> Status: designed
> Generated on: 2026-10-09
> Project: existing
> From backlog: BL-0009
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary

When a project member converts a document today, only its text survives: the images and tables the
converter extracted are thrown away. This increment keeps them as artifacts of the document. A member
lists and gets the images (picture and caption), lists and gets the tables (as Markdown or CSV), and
gets the full document with image lines and tables inline or as references. Artifacts stay readable
while the document is unchanged at its path; any change to the document makes them go away until the
document is converted again. It is the first of the specs split from BL-0009 (Section 3.2).

## 2. Current State

### 2.1 What users can do today

- A member converts a document (PDF, Word, PowerPoint, Excel, HTML, CSV, images) and receives one
  Markdown text, stored as a sibling file with the same name and the `.md` extension, e.g.
  `report.pdf` gives `report.md`.
- Three operations produce that Markdown sibling: the explicit convert action and a write or upload
  requesting documentation (both use the external converter), and text extraction (the built-in
  conversion).
- With the external converter configured, the converter produces figures, an image catalog and
  tables, but the product keeps only the Markdown text; every other output is discarded.
- Without the external converter, the convert action is refused with `document service is not
  configured`; a format the converter does not accept is refused with `'<path>' cannot be
  documented, the document service accepts: <extensions>`. Text extraction still works.
- Built-in conversion renders Word, Excel and CSV tables only as Markdown tables inside the text,
  capped at 400 rows. No CSV of a table is available. Images yield OCR text only, and by default
  nothing.
- The convert action refuses to replace an existing Markdown sibling unless overwrite is requested,
  with `'<sibling path>' exists (pass overwrite=true)`.
- The limit actually enforced is the bytes a member is allowed to write in a session; writing or
  copying a file counts its full size, and nothing is given back. The convert action checks it before
  writing; text extraction writes the sibling first and checks after, so a refused extraction still
  leaves a new sibling. A project storage limit can be recorded but is not enforced.
- Access is project membership: a member can read and write every document of the project; there is
  no read only role.
- There is no notion of per document metadata: a file is only its content and timestamps.

### 2.2 Specs governing this area

- SPEC-0006 (documents, as built): conversion, the Markdown sibling and its refresh. This spec
  modifies one behavior, the sibling on a quota refused text extraction (FR-MOD-001); every other
  SPEC-0006 behavior is kept (FR-NEW-022).
- SPEC-0010 (project storage quota): its enforcement and usage reading are not built; this spec does
  not depend on them. Artifact bytes are charged to the session write quota only (FR-NEW-013).

### 2.3 What is verified today

Conversion, the Markdown sibling, OCR and the external converter are exercised by existing tests.
No test exercises images or tables as separate items, because none exist. No existing test is
modified or removed by this spec.

## 3. Scope

### 3.1 In scope

- Keeping every image and table found when a document is converted.
- Listing and getting images; listing and getting tables as Markdown or CSV.
- Getting the full document in a chosen view.
- Re-conversion; artifacts going away whenever the document changes.
- Quota, access and partial failure rules for artifacts.
- The same results for AI assistants and HTTP API consumers.

### 3.2 Out of scope, and why

| Non goal | Reason | Backlog |
|---|---|---|
| Artifacts following their document through move, copy, trash, restore, purge, folders, version control, export | made the spec non convergent; specified on top of this one | BL-0038 |
| Chunks as readable items, semantic chunking | separate increment, changes search | BL-0033 |
| Summaries of long chunks, concept tagging | new language model dependency | BL-0034 |
| Per project extraction profile at creation | each increment brings its own defaults first | BL-0035 |
| Search hits pointing at tables and images | needs this spec first | BL-0036 |
| Audio and video screenshots and transcripts | keep the first increment to images and tables | BL-0037 |
| Editing an artifact by hand | artifacts are derived output; re-convert to change them | none, decided |
| Ids stable across re-conversion | renumbering is stated (FR-NEW-010) | none, decided |
| Bulk upgrade of documents converted before this spec | re-conversion upgrades a document | none, decided |
| Changes to the converter itself | separate product; gaps listed in Section 14.3 | none here |

### 3.3 Constraints imposed by the context

- Imposed by the owner: the converter is a command line tool today and will be a remote service
  tomorrow; the observable behavior of this spec SHALL NOT depend on which one is used.
- Imposed by the project conventions: every operation is offered identically to AI assistants and
  to HTTP API consumers.

## 4. Actors

| Actor | Description |
|---|---|
| Member | Belongs to the project, hence can read and write all its documents. |
| Outsider | Does not belong to the project. |
| Operator | Runs the server and configures the converter. |

## 4.5 Bounded contexts

| Context | What it owns | Key concepts |
|---|---|---|
| Documents | conversion, artifacts, document views | document, artifact, image, picture, table, data row, caption, view, image line, table reference line, table mode, CSV quality, converter, built-in conversion, text extraction, OCR, Markdown sibling, conversion text, reading position, re-conversion, continuation marker |
| Files | paths, content, every operation changing them | document change |
| Quota | the bytes a member is allowed to write in a session | session write quota |
| Access | who belongs to a project | member, outsider, refusal |

## 5. Usage scenarios

### SC-001: Convert a document, artifacts kept
**Actor:** Member
**Preconditions:** the member belongs to the project; the document exists.
**Flow:**
1. The member converts the document (convert action, a write or upload requesting documentation, or
   text extraction) → the system writes the Markdown sibling as today and keeps each image and table.
2. The system reports `<N> images, <M> tables` and every missing item.
**Postconditions:** the artifacts are listable; their bytes were charged to the session write quota.
**Exceptions:**
- EXC-001a: the session write quota is exceeded → `session write quota of <N> bytes exceeded`;
  nothing kept, nothing charged.
- EXC-001b: one item fails → the rest is kept, each failure reported as `<id>: <reason>`.
- EXC-001c: no converter, or a format it does not accept → convert action refused as today; text
  extraction keeps Word, Excel and CSV tables.

### SC-002: List and get images
**Actor:** Member
**Preconditions:** the document was converted and has not changed since.
**Flow:**
1. The member lists the images → `image-1` to `image-N`, caption, page, 100 per page.
2. The member gets one image → its picture, format and caption.
**Postconditions:** nothing changes.
**Exceptions:**
- EXC-002a: unknown id → `Not found: image-7`.
- EXC-002b: never converted, or changed since → `No artifacts: document not extracted`.
- EXC-002c: outsider → the refusal a non member receives today for any operation on the project.

### SC-003: List and get tables
**Actor:** Member
**Preconditions:** as SC-002.
**Flow:**
1. The member lists the tables → `table-1` to `table-N`, caption, data rows, columns, CSV quality.
2. The member gets one table as `markdown` (default) or `csv`.
**Postconditions:** nothing changes.
**Exceptions:**
- EXC-003a: unknown id → `Not found: table-9`.
- EXC-003b: unsupported format → `Unsupported format: xml (use markdown or csv)`.
- EXC-003c: as EXC-002b and EXC-002c.

### SC-004: Get the full document in a view
**Actor:** Member
**Preconditions:** as SC-002.
**Flow:**
1. The member asks for the full document, choosing captions on (default) or off and the table mode
   `markdown` (default), `csv-reference` or `both`.
2. The system returns the conversion text with an image line at each image and each table in the
   chosen mode.
**Postconditions:** nothing changes; the Markdown sibling is untouched.
**Exceptions:**
- EXC-004a: unsupported mode → `Unsupported table mode: html (use markdown, csv-reference or both)`.
- EXC-004b: as EXC-002b and EXC-002c.

### SC-005: Re-convert a document
**Actor:** Member
**Preconditions:** the document was converted before.
**Flow:**
1. The member re-converts it (convert action with overwrite, a write or upload requesting
   documentation, or text extraction with refresh) → the whole previous set is replaced, numbered
   from 1.
**Postconditions:** only the new set exists.
**Exceptions:**
- EXC-005a: quota exceeded or converter unavailable → refused; previous set and sibling unchanged.
- EXC-005b: convert action without overwrite → today's refusal; previous set unchanged.
- EXC-005c: two members re-convert at once → exactly one complete set survives.

### SC-006: The document changes
**Actor:** Member
**Preconditions:** the document was converted.
**Flow:**
1. The member moves, renames, copies, deletes, trashes or replaces the bytes of the document, by any
   operation → its artifacts go away; a copy starts with none.
2. Asking for artifacts afterwards answers `No artifacts: document not extracted` at the document's
   current path; converting again brings them back.
**Postconditions:** artifacts never describe content other than the one converted.
**Exceptions:**
- EXC-006a: the old path after a move → `not a file: /report.pdf`.

## 6. Functional requirements

### 6.1 New

#### FR-NEW-001 [EARS-E]: Keep artifacts on conversion
> WHEN a member converts a document THE Documents context SHALL keep each image (picture, caption,
> page, reading position) and each table found, as artifacts of that document.
- **Business rules:** converting means the convert action, a write or upload requesting
  documentation, and text extraction that writes the Markdown sibling. Page is the 1 based page number
  for PDF, the 1 based slide number for PowerPoint, and empty for every other format, Word, Excel,
  HTML, CSV and images included. Ids are assigned in reading order, `image-1` to
  `image-N` and `table-1` to `table-M`, before any item is kept, so a failed item leaves its id unused
  and later items keep theirs. Each placement of a picture in reading order is a separate image with
  its own id, even when its bytes equal an earlier image's. A text extraction that yields empty text
  writes no Markdown sibling, as today, and still records the document as converted with an empty set.
- **User visible text:** the result reports `<N> images, <M> tables`, N and M counting kept items.
- **Scenario:** SC-001, SC-002, SC-006 **Priority:** Must

#### FR-NEW-002 [EARS-U]: Tables in two formats with a quality label
> The Documents context SHALL offer every table as Markdown and as CSV, each table carrying a CSV
> quality label.
- **Business rules:** the label reads `CSV quality: exact` for a table built from the original cells
  and `CSV quality: approximate` for a table derived from Markdown text. A Markdown table reads
  `| h1 | h2 |`, then `| --- | --- |`, then one `| c1 | c2 |` line per data row, lines separated by a
  single newline, no trailing newline, cells trimmed, a short row padded with empty cells to the
  widest row, a pipe inside a cell escaped as `\|`, each CR LF pair, lone CR or lone LF inside a cell turned into one space. CSV
  uses comma separators, double quote quoting, doubled inner quotes, a newline inside a cell kept
  inside quotes. CSV: a field is quoted with double quotes only when it contains a comma, a double quote, a carriage
  return or a newline; lines are separated by a single LF, with no trailing newline; a short row is
  padded with empty fields to the widest row. The column count of a table is the cell count of its
  widest row, header included. A cell's value is the same in both formats: leading and trailing
  whitespace removed, and for a table derived from Markdown text each `\|` read as `|`; no other
  Markdown markup is removed (`**x**` stays `**x**`). A kept table has every row found: the 400 row cap of the Markdown sibling does not
  apply to it.
- **Scenario:** SC-001, SC-003 **Priority:** Must

#### FR-NEW-003 [EARS-E]: List images
> WHEN a member lists a document's images THE Documents context SHALL return, in reading order, each
> image's id, caption and page, 100 per page.
- **Business rules:** a continuation marker appears only when more items remain (exactly 100 items:
  one page, no marker); no images: an empty list, not an error. A continuation marker is valid only
  for the project, the list kind (images or tables) and the document path that issued it; any other
  marker, including one from another project, the other list kind or another document, answers
  `Invalid continuation marker: <value>`. A marker is not bound to the person it was issued to: any member
  presenting it gets the same next page. A marker whose position is past the end of the current set
  answers an empty list with no continuation marker. A marker issued before a re-conversion answers the
  next page of the current set.
- **Scenario:** SC-001, SC-002, SC-003 **Priority:** Must

#### FR-NEW-004 [EARS-E]: Get an image
> WHEN a member gets an image by id THE Documents context SHALL return its picture, byte identical to
> what the converter extracted, its format and its caption.
- **Business rules:** format is one of `png`, `jpeg`, `gif`, `webp`, `tiff`, `bmp`: `jpeg` for a
  `.jpg` or `.jpeg` picture, `tiff` for a `.tif` or `.tiff` picture, otherwise its lowercase extension. A missing caption is returned empty.
  Captions are returned exactly, accents, emoji, pipes and quotes included.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-005 [EARS-E]: List tables
> WHEN a member lists a document's tables THE Documents context SHALL return, in reading order, each
> table's id, caption, data row count, column count and CSV quality, 100 per page.
- **Business rules:** paging as FR-NEW-003; the header row is not counted as a data row.
- **Scenario:** SC-001, SC-003 **Priority:** Must

#### FR-NEW-006 [EARS-E]: Get a table
> WHEN a member gets a table by id THE Documents context SHALL return it in the requested format,
> `markdown` when no format is given.
- **Business rules:** formats `markdown`, `csv`; a table with 0 data rows returns, as `markdown`, exactly its header line and
  its separator line (`| h1 | h2 |` then `| --- | --- |`), and as `csv`, exactly its header line.
- **Scenario:** SC-003 **Priority:** Must

#### FR-NEW-007 [EARS-E]: Full document view
> WHEN a member gets the full document THE Documents context SHALL return the conversion text with an
> image line at each image's reading position and each table rendered in the chosen table mode at its
> reading position.
- **Business rules:** the conversion text is the Markdown text the last conversion produced. The image
  line is exactly `[Image image-<n>: <caption>]`, or `[Image image-<n>]` when the caption is empty,
  preceded and followed by one empty line. It replaces exactly the one line holding the picture
  reference; when the first non empty line immediately after it equals the image's caption verbatim,
  that line is removed too; no other line is removed. The table rendering replaces the table's Markdown as written in the
  conversion text: `markdown` (default) renders the Markdown table of FR-NEW-002, whole, `csv-reference`
  renders exactly `[Table table-<n>: <caption> (CSV)]` or `[Table table-<n> (CSV)]` when the caption is
  empty, `both` renders the Markdown table followed by the reference line. No other text is altered.
- **Scenario:** SC-001, SC-004 **Priority:** Must

#### FR-NEW-008 [EARS-E]: Captions off
> WHEN a member gets the full document with captions turned off THE Documents context SHALL render
> every image line as `[Image image-<n>]`.
- **Business rules:** table reference lines keep their caption.
- **Scenario:** SC-004 **Priority:** Must

#### FR-NEW-009 [EARS-E]: View independent of the sibling file
> WHEN a member gets the full document after the Markdown sibling was rewritten or deleted THE
> Documents context SHALL build the view from the conversion text.
- **Scenario:** SC-004 **Priority:** Must

#### FR-NEW-010 [EARS-E]: Re-conversion replaces the set
> WHEN a member re-converts a document THE Documents context SHALL replace its whole artifact set and
> its conversion text, numbering the new items from 1.
- **Business rules:** re-converting means the convert action with overwrite, a write or upload
  requesting documentation, or text extraction with refresh. A reader during re-conversion sees the
  complete old set or the complete new set, never a mix. When two re-conversions of the same document overlap, both
  succeed and each returns its own conversion report, each charged to its own member's session write
  quota; the one that commits last supplies the set, the conversion text and the Markdown sibling that
  remain, all three from that same conversion. A convert action or a text extraction that fails or is refused leaves the
  previous set, the conversion text and the Markdown sibling unchanged. A write or upload requesting
  documentation replaces the document's bytes, so FR-NEW-011 discards the previous set whatever the
  conversion outcome; the Markdown sibling is left unchanged when that conversion fails or is refused. An old id that no longer exists answers `Not found: <id>`.
- **Scenario:** SC-005 **Priority:** Must

#### FR-NEW-011 [EARS-E]: A changed document loses its artifacts
> WHEN a converted document is moved, renamed, deleted or has its bytes replaced, by any operation,
> THE Documents context SHALL discard its artifacts and its conversion text.
- **Business rules:** this holds whatever the operation: write, edit, upload, copy or move onto it,
  archive extraction, trash, version control pull, checkout, merge, revert or file restore, and an
  operation on a folder containing it. It holds even when the new bytes equal the old ones, and when
  the document comes back to its former path. Writing the Markdown sibling does not change the
  document. A write or upload requesting documentation discards, then keeps the new set if its
  conversion succeeds. Soft deleting the project and restoring it does not change its documents:
  artifacts stay readable. A version control pull, checkout, merge or revert discards the artifacts of
  exactly the documents whose content differs between the tree before and after the operation, plus
  every document it deletes; a document identical in both trees keeps its artifacts.
- **User visible text:** any later artifact request on the document answers
  `No artifacts: document not extracted`.
- **Scenario:** SC-004, SC-005, SC-006 **Priority:** Must

#### FR-NEW-012 [EARS-E]: A copy starts with no artifacts
> WHEN a member copies a converted document THE Documents context SHALL give the copy no artifacts and
> keep the source's artifacts.
- **Business rules:** the copy is charged its file bytes only, as today.
- **Scenario:** SC-006 **Priority:** Must

#### FR-NEW-013 [EARS-O]: Session write quota on conversion
> IF a conversion (artifacts and Markdown sibling together) exceeds the member's session write quota
> THEN THE Documents context SHALL refuse it, charge nothing for it, and leave every existing artifact
> and the Markdown sibling unchanged.
- **Business rules:** artifact bytes are charged to the session write quota like file bytes; nothing
  is given back on re-conversion or discard. The bytes of a set are the sum of every picture's bytes,
  each table's CSV bytes, each table's Markdown bytes and each caption's UTF-8 bytes. No project
  storage limit is enforced by this spec. The bytes of a conversion are the bytes of its set, plus the UTF-8 bytes
  of its Markdown sibling, plus the UTF-8 bytes of its conversion text. For a write or upload
  requesting documentation, FR-NEW-011 and FR-NEW-014 govern: the previous set is already discarded.
- **User visible text:** `session write quota of <N> bytes exceeded`, N being the session limit.
- **Scenario:** SC-001, SC-005, SC-006 **Priority:** Must

#### FR-NEW-014 [EARS-O]: Quota refusal during a documentation write
> IF a write or upload requesting documentation writes the source document but its conversion exceeds
> the session write quota THEN THE Documents context SHALL keep the source written and charged, keep no
> artifact and no new Markdown sibling, and report the documentation outcome as
> `session write quota of <N> bytes exceeded`.
- **Scenario:** SC-001, SC-005 **Priority:** Must

#### FR-NEW-015 [EARS-O]: Partial failure
> IF an image or a table the conversion text references fails THEN THE Documents context SHALL keep
> everything else and report each failed item as `<id>: <reason>`.
- **User visible text:** `image-<n>: caption unavailable` when the converter's output marks that
  caption as failed (the image is kept with an empty caption); `image-<n>: picture unavailable` when
  the referenced picture cannot be read from the converter's output (not kept);
  `image-<n>: unsupported picture format <ext>` for a picture outside the FR-NEW-004 list (not kept);
  `table-<n>: malformed table` for a pipe block whose separator line does not have its header's cell
  count (not kept). A picture reference whose target is a URL, an absolute path, or a relative path
  that, once resolved, lies outside the converter's output, is a picture that cannot be read:
  `image-<n>: picture unavailable`, not kept; nothing outside the converter's output is ever read.
  An item the converter omits from its output is not an item: it takes no id and is
  not reported.
- **Business rules:** the result reports `<N> images, <M> tables` first, then one `<id>: <reason>`
  entry per failed item: all images in id order, then all tables in id order.
- **Scenario:** SC-001, SC-002 **Priority:** Must

#### FR-NEW-016 [EARS-UB]: Access
> The Documents context SHALL NOT let anyone who is not a member of the project list, get or view the
> artifacts of its documents, nor convert or re-convert them.
- **User visible text:** the refusal a non member receives today for any operation on the project.
- **Scenario:** SC-001, SC-002, SC-003, SC-004, SC-005 **Priority:** Must

#### FR-NEW-017 [EARS-E]: Text extraction keeps built-in tables
> WHEN a member extracts a document's text THE Documents context SHALL keep the tables the built-in
> conversion finds in Word, Excel and CSV documents, labelled `CSV quality: exact`, and no images.
- **Business rules:** text extraction always uses the built-in conversion, whether a converter is
  configured or not. A built-in Excel table's caption is its sheet name, verbatim; a built-in Word or
  CSV table's caption is empty. A built-in Excel row whose cells are all blank is not a row: it is not
  kept and not counted as a data row, as in the Markdown sibling today.
- **Scenario:** SC-001, SC-003 **Priority:** Must

#### FR-NEW-018 [EARS-O]: Format the converter does not accept
> IF the convert action or a write or upload requesting documentation targets a format the configured
> converter does not accept THEN THE Documents context SHALL answer today's refusal, verbatim
> `'<path>' cannot be documented, the document service accepts: <extensions>`, and keep no artifact.
- **Scenario:** SC-001 **Priority:** Must

#### FR-NEW-019 [EARS-O]: Unknown id
> IF a member requests an artifact id the document does not have THEN THE Documents context SHALL
> answer `Not found: <id>`.
- **Scenario:** SC-002, SC-003, SC-005 **Priority:** Must

#### FR-NEW-020 [EARS-U]: Same results for every consumer
> The Documents context SHALL give byte identical results to AI assistants and HTTP API consumers.
- **Scenario:** SC-002, SC-003, SC-004 **Priority:** Must

#### FR-NEW-021 [EARS-UB]: Artifacts are not files
> The Documents context SHALL NOT include artifacts in folder listings, file searches, trash listings,
> project exports, or version control history and pushes.
- **Scenario:** SC-001, SC-006 **Priority:** Must

#### FR-NEW-022 [EARS-U]: Markdown sibling kept
> The Documents context SHALL keep producing the Markdown sibling exactly as before this spec.
- **Business rules:** FR-MOD-001 governs the sibling on a quota refused text extraction.
- **Scenario:** SC-001, SC-004, SC-005, SC-006 **Priority:** Must

#### FR-NEW-023 [EARS-O]: No artifacts
> IF a member requests artifacts of a document that was never converted, or that changed since its
> last conversion, THEN THE Documents context SHALL answer `No artifacts: document not extracted`.
- **Scenario:** SC-002, SC-003, SC-004 **Priority:** Must

#### FR-NEW-024 [EARS-O]: Unsupported table format
> IF a member requests a table format other than `markdown` or `csv` THEN THE Documents context SHALL
> refuse with `Unsupported format: <value> (use markdown or csv)` and return no content.
- **Business rules:** values are case sensitive.
- **Scenario:** SC-003 **Priority:** Must

#### FR-NEW-025 [EARS-O]: Unsupported table mode
> IF a member requests a table mode other than `markdown`, `csv-reference` or `both` THEN THE
> Documents context SHALL refuse with `Unsupported table mode: <value> (use markdown, csv-reference or
> both)` and return no content.
- **Business rules:** values are case sensitive.
- **Scenario:** SC-004 **Priority:** Must

#### FR-NEW-026 [EARS-E]: Text extraction refresh
> WHEN text extraction rewrites the Markdown sibling of a document THE Documents context SHALL replace
> its artifact set with the set the built-in conversion produces (FR-NEW-010).
- **Business rules:** a text extraction answered from the existing sibling, without rewriting it,
  leaves the artifact set unchanged.
- **Scenario:** SC-005 **Priority:** Must

#### FR-NEW-027 [EARS-O]: Convert without overwrite
> IF a convert action is refused because the Markdown sibling exists and overwrite was not requested
> THEN THE Documents context SHALL leave the artifact set unchanged.
- **User visible text:** today's refusal, verbatim `'<sibling path>' exists (pass overwrite=true)`.
- **Business rules:** with no existing sibling the convert action succeeds without overwrite.
- **Scenario:** SC-001, SC-005 **Priority:** Must

#### FR-NEW-028 [EARS-O]: Artifact request on a path holding no file
> IF a member requests artifacts at a path that holds no file THEN THE Documents context SHALL answer
> `not a file: <path>`.
- **Scenario:** SC-002, SC-006 **Priority:** Must

#### FR-NEW-029 [EARS-O]: No converter configured
> IF the convert action or a write or upload requesting documentation is used while no external
> converter is configured THEN THE Documents context SHALL answer today's refusal, verbatim
> `document service is not configured`, and keep no artifact.
- **Business rules:** as today, a write or upload refused this way writes no source document; a
  refused convert action leaves the previous set unchanged (FR-NEW-010).
- **Scenario:** SC-001, SC-005 **Priority:** Must

#### FR-NEW-030 [EARS-E]: Conversion report
> WHEN a conversion completes THE Documents context SHALL return, in addition to every item the
> operation returns today and with those items unchanged, a conversion report.
- **Business rules:** the report is the line `<N> images, <M> tables` followed by one line per failed
  item, `<id>: <reason>`, in the order of FR-NEW-015. For a write or upload requesting documentation
  the report is part of the documentation outcome. A text extraction answered from the existing
  sibling returns no report. The extracted text returned by text extraction is never altered by the
  report.
- **Scenario:** SC-001, SC-005 **Priority:** Must

#### FR-NEW-031 [EARS-U]: Refusal order
> The Documents context SHALL check an artifact request in this order and answer the first refusal
> that applies: the non member refusal; `not a file: <path>`; `Unsupported format: <value> (use
> markdown or csv)`, `Unsupported table mode: <value> (use markdown, csv-reference or both)` or
> `Invalid continuation marker: <value>`; `No artifacts: document not extracted`; `Not found: <id>`.
- **Scenario:** SC-002, SC-003, SC-004 **Priority:** Must

#### FR-NEW-032 [EARS-U]: Refusal categories
> The Documents context SHALL class `Not found: <id>`, `not a file: <path>` and `No artifacts:
> document not extracted` as not found refusals, and `Unsupported format: <value> (use markdown or
> csv)`, `Unsupported table mode: <value> (use markdown, csv-reference or both)` and `Invalid
> continuation marker: <value>` and `Document changed during conversion: <path>` as invalid argument
> refusals.
- **Business rules:** these are the existing refusal categories consumers already branch on.
- **Scenario:** SC-002, SC-003 **Priority:** Must

#### FR-NEW-033 [EARS-E]: What the converter's output yields
> WHEN the external converter returns its output THE Documents context SHALL take as images the
> picture references of the conversion text and as tables its pipe table blocks, in reading order.
- **Business rules:** an image's caption is the description the converter gives for that picture;
  when the converter gives none it is empty; when the converter reports that describing it failed,
  the caption is empty and `<id>: caption unavailable` is reported. A picture reference is a line whose only content, after trimming, is one Markdown
  image `![<alt>](<target>)`; an image written inside a line of other text, or a line holding two
  images, is not a picture reference and stays as text in the view. A separator line is a line made
  only of `|`, `-`, `:` and spaces, holding at least one `-` per cell. A pipe table block is a header
  line immediately followed by a separator line, then zero or more consecutive lines starting with
  `|`; a separator line whose cell count differs from its header's makes the block a malformed table
  (FR-NEW-015). Lines inside a fenced code block (between two lines starting with three backticks)
  yield no picture reference and no table. A header line, a separator line and a data row are each a
  line whose first non space character is `|`; a line not starting with `|` is never part of a pipe
  table block. A line's cells are obtained by removing one leading `|` and, if present, one trailing
  `|`, then splitting on every `|` not preceded by `\`; its cell count is the number of parts. When the first non empty line immediately before the block
  reads `Table <number>: <text>`, the table's caption is `<text>` verbatim (`Table 1: Q1 sales` gives
  `Q1 sales`); otherwise it is empty.
- **Scenario:** SC-001, SC-002, SC-003 **Priority:** Must

#### FR-NEW-034 [EARS-E]: Text limit on extraction
> WHEN text extraction cuts the conversion text at the requested character limit THE Documents
> context SHALL keep only the tables whose whole Markdown lies before the cut.
- **Business rules:** kept tables are numbered in reading order and keep every row; the view ends
  where the conversion text ends.
- **Scenario:** SC-001, SC-003, SC-004 **Priority:** Must

#### FR-NEW-035 [EARS-E]: Document changed during conversion
> WHEN a document changes while its conversion is running THE Documents context SHALL keep no artifact
> and no conversion text from that conversion and report `Document changed during conversion: <path>`.
- **Business rules:** the document is then in the not extracted state. The refusal is in the invalid
  argument category; the Markdown sibling is left as it was before the request; nothing is charged to
  the session write quota for that conversion. For a write or upload requesting documentation, the
  source write stays done and charged, and the refusal is the documentation outcome.
- **Scenario:** SC-005, SC-006 **Priority:** Must

#### FR-NEW-036 [EARS-E]: Empty lines around an image line
> WHEN the Documents context places an image line in a view THE Documents context SHALL remove the
> picture reference line, the matched caption line (FR-NEW-007) and every empty line directly before,
> between or after those removed lines, then write exactly one empty line before and one after the
> image line.
- **Business rules:** the empty line before is omitted when the image line is the first line of the
  view, the one after when it is the last. An empty line is a line holding nothing or only spaces. No
  other empty line of the conversion text is added or removed.
- **Scenario:** SC-004 **Priority:** Must

#### FR-NEW-037 [EARS-E]: Picture reference not kept
> WHEN a picture reference yields no kept image (picture unavailable or unsupported picture format)
> THE Documents context SHALL leave its line in the view exactly as written in the conversion text
> and render no image line for its id.
- **Scenario:** SC-001, SC-004 **Priority:** Must

### 6.2 Modified

#### FR-MOD-001 [EARS-O]: Sibling on a quota refused text extraction (modifies SPEC-0006 sibling refresh)
> IF a text extraction is refused for the session write quota THEN THE Documents context SHALL leave
> the Markdown sibling as it was before the request.
- **Before:** text extraction writes the sibling and then checks the quota, so a refused extraction
  leaves a new sibling (Section 2.1).
- **After:** the statement above: no sibling is created or rewritten.
- **Reason:** a refusal leaves nothing changed, for artifacts and sibling alike.
- **Who is affected:** members whose text extraction hits the session write quota.
- **Scenario:** SC-001, SC-005 **Priority:** Must

#### FR-MOD-002 [EARS-E]: Conversion results carry a report (modifies SPEC-0006 results of text extraction, convert action and documentation write)
> WHEN a text extraction, convert action or documentation write converts a document THE Documents
> context SHALL add the conversion report of FR-NEW-030 to its result.
- **Before:** these results carry no report (Section 2.1).
- **After:** every item of today kept unchanged, plus the report.
- **Reason:** the member sees what was kept and what failed.
- **Who is affected:** every consumer of these three results; nothing they read today changes.
- **Scenario:** SC-001, SC-005 **Priority:** Must

### 6.3 Removed

None.

## 7. Non functional requirements (user observable)

### 7.1 Performance
No new target: listing and getting artifacts returns within the time a file read of the same size
takes today.

### 7.2 Security and access
FR-NEW-016: artifacts are never readable by a non member.

### 7.3 Privacy and compliance
No change: artifacts stay in the project's own storage.

### 7.4 Usability and accessibility
Captions and cells are returned exactly as extracted (accents, emoji, right to left text).

### 7.5 Reliability
A failed or refused conversion never leaves a half replaced set (FR-NEW-010, FR-NEW-013).

### 7.6 Operability
Conversion results report every missing item with its reason (FR-NEW-015); a refused conversion
names the session write quota it exceeded (FR-NEW-013).

## 8. Edge cases

| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | text only document; never converted | empty lists; `No artifacts: document not extracted` | E2E-015, E2E-028 |
| Boundary values | 100 and 101 items; 1 item; 0 data rows; 401 rows | one page; two pages; one entry; header only; whole table | E2E-029, 030, 031, 045, 046, 047 |
| Concurrent actions | two re-conversions; read during re-conversion | one complete set; old or new, never mixed | E2E-080, E2E-081 |
| Slow or lost connectivity | converter unreachable | existing failure message, old set kept | E2E-072 |
| Access states | outsider | today's non member refusal | E2E-013, 027, 038, 058, 074 |
| Data variations | emoji, accents, pipes, quotes, commas | exact, escaped per FR-NEW-002 | E2E-032, E2E-044 |
| Navigation | stale id; stale marker; old path after move; back to former path | `Not found: <id>`; `Invalid continuation marker: <value>`; `not a file: <path>`; no artifacts | E2E-079, E2E-088, E2E-097 |
| Partial operations | caption, picture, table fails | rest kept, failures reported | E2E-005, 006, 007, 008 |
| Dependency failures | no converter; format refused | today's refusals; extraction keeps tables | E2E-009, 010, 011 |
| Ordering and timing | ids in reading order, gaps on failure | `table-1`, `table-3` | E2E-006 |
| Exhaustion | session write quota | refused, nothing changed | E2E-004, 012, 014, 071, 075, 076, 077 |

## 9. Acceptance tests (the contract)

> Behavioral and technology agnostic. /sdlc-design maps each test to a driver.

Shared data: project `Atlas`; members Alice and Bob; outsider Olga. Alice's session write quota is
262 144 bytes unless stated. `report.pdf` has 3 pages, 2 PNG images (`Figure 1: Revenue 2024` on
page 1, `Schéma réseau 🌐` on page 3) and 2 tables (`Q1 sales`, 3 data rows by 4 columns; `Costs`, header
`Item`, `EUR`, rows `Rent`, `900` and `Power`, `120`). Its conversion text puts the line `Table 1: Q1 sales` before the first table and `Table 2: Costs`
before the second. `Q1 sales` has header `Region`, `Jan`, `Feb`, `Mar` and rows `North`, `10`, `11`,
`12`; `South`, `20`, `21`, `22`; `West`, `30`, `31`, `32`. The external converter is configured, accepts
`.pdf` documents, and returns tables as Markdown (CSV quality approximate), unless stated. "The
converter now returns X for it" means the converter's output for the same bytes changed.

### 9.1 Summary

| Test | Action | Category | Scenario | Requirements (FR-NEW- unless MOD-) | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | 001, 015 | Critical |
| E2E-002 | New | happy | SC-001 | 017, 001, 002 | Critical |
| E2E-003 | New | happy | SC-001 | 017 | High |
| E2E-004 | New | failure | SC-001 | 013 | Critical |
| E2E-005 | New | failure | SC-001 | 015 | Critical |
| E2E-006 | New | failure | SC-001 | 015, 001 | High |
| E2E-007 | New | failure | SC-001 | 015 | High |
| E2E-008 | New | failure | SC-001 | 015, 004 | Medium |
| E2E-009 | New | failure | SC-001 | 017 | High |
| E2E-010 | New | failure | SC-001 | 018 | High |
| E2E-011 | New | failure | SC-001 | 029 | High |
| E2E-012 | New | failure | SC-001 | 014 | Critical |
| E2E-013 | New | failure | SC-001 | 016 | High |
| E2E-014 | New | failure | SC-001 | MOD-001, 013 | High |
| E2E-015 | New | edge | SC-001 | 001, 003, 005 | High |
| E2E-016 | New | edge | SC-001 | 017, 001 | Medium |
| E2E-017 | New | edge | SC-001 | 014 | Medium |
| E2E-018 | New | edge | SC-001 | 027, 001 | Medium |
| E2E-019 | New | side effect | SC-001 | 021 | Critical |
| E2E-020 | New | side effect | SC-001 | 022 | Critical |
| E2E-021 | New | side effect | SC-001 | 013 | High |
| E2E-022 | New | side effect | SC-001 | 021 | Medium |
| E2E-023 | New | side effect | SC-001 | 021 | Medium |
| E2E-024 | New | happy | SC-002 | 003 | Critical |
| E2E-025 | New | happy | SC-002 | 004 | Critical |
| E2E-026 | New | failure | SC-002 | 019 | Critical |
| E2E-027 | New | failure | SC-002 | 016 | Critical |
| E2E-028 | New | failure | SC-002 | 023 | Critical |
| E2E-029 | New | edge | SC-002 | 003 | High |
| E2E-030 | New | edge | SC-002 | 003 | High |
| E2E-031 | New | edge | SC-002 | 003, 004 | High |
| E2E-032 | New | edge | SC-002 | 004 | Medium |
| E2E-033 | New | side effect | SC-002 | 020 | High |
| E2E-034 | New | happy | SC-003 | 005, 002 | Critical |
| E2E-035 | New | happy | SC-003 | 006 | Critical |
| E2E-036 | New | happy | SC-003 | 006, 002 | Critical |
| E2E-037 | New | failure | SC-003 | 019 | Critical |
| E2E-038 | New | failure | SC-003 | 016 | Critical |
| E2E-039 | New | failure | SC-003 | 024 | High |
| E2E-040 | New | failure | SC-003 | 024 | High |
| E2E-041 | New | failure | SC-003 | 024 | Medium |
| E2E-042 | New | failure | SC-003 | 023 | High |
| E2E-043 | New | edge | SC-003 | 002, 017 | Critical |
| E2E-044 | New | edge | SC-003 | 002 | High |
| E2E-045 | New | edge | SC-003 | 005 | High |
| E2E-046 | New | edge | SC-003 | 005, 006 | Medium |
| E2E-047 | New | edge | SC-003 | 002, 006 | High |
| E2E-048 | New | edge | SC-003 | 002, 006 | Medium |
| E2E-049 | New | side effect | SC-003 | 020 | High |
| E2E-050 | New | happy | SC-004 | 007 | Critical |
| E2E-051 | New | happy | SC-004 | 007 | Critical |
| E2E-052 | New | happy | SC-004 | 007, 008 | High |
| E2E-053 | New | happy | SC-004 | 008 | High |
| E2E-054 | New | failure | SC-004 | 025 | High |
| E2E-055 | New | failure | SC-004 | 025 | High |
| E2E-056 | New | failure | SC-004 | 025 | Medium |
| E2E-057 | New | failure | SC-004 | 023 | Critical |
| E2E-058 | New | failure | SC-004 | 016 | Critical |
| E2E-059 | New | failure | SC-004 | 009, 011 | High |
| E2E-060 | New | edge | SC-004 | 007 | High |
| E2E-061 | New | edge | SC-004 | 008 | Medium |
| E2E-062 | New | edge | SC-004 | 007 | Medium |
| E2E-063 | New | edge | SC-004 | 007 | High |
| E2E-064 | New | edge | SC-004 | 007 | High |
| E2E-065 | New | edge | SC-004 | 009 | High |
| E2E-066 | New | edge | SC-004 | 009 | Medium |
| E2E-067 | New | side effect | SC-004 | 020 | High |
| E2E-068 | New | side effect | SC-004 | 022 | High |
| E2E-069 | New | happy | SC-005 | 010 | Critical |
| E2E-070 | New | happy | SC-005 | 026, 010 | High |
| E2E-071 | New | failure | SC-005 | 013, 010 | Critical |
| E2E-072 | New | failure | SC-005 | 010 | Critical |
| E2E-073 | New | failure | SC-005 | 027 | High |
| E2E-074 | New | failure | SC-005 | 016 | High |
| E2E-075 | New | failure | SC-005 | MOD-001, 013, 026 | High |
| E2E-076 | New | failure | SC-005 | MOD-001 | Critical |
| E2E-077 | New | failure | SC-005 | 013, 022 | High |
| E2E-078 | New | failure | SC-005 | 014, 011 | High |
| E2E-079 | New | edge | SC-005 | 010, 019 | Critical |
| E2E-080 | New | edge | SC-005 | 010 | Critical |
| E2E-081 | New | edge | SC-005 | 010 | Critical |
| E2E-082 | New | edge | SC-005 | 026 | Medium |
| E2E-083 | New | edge | SC-005 | 027 | Medium |
| E2E-084 | New | edge | SC-005 | 010, 011 | High |
| E2E-085 | New | side effect | SC-005 | 013 | High |
| E2E-086 | New | happy | SC-006 | 011 | Critical |
| E2E-087 | New | happy | SC-006 | 011, 001 | Critical |
| E2E-088 | New | failure | SC-006 | 011, 028 | Critical |
| E2E-089 | New | failure | SC-006 | 012 | Critical |
| E2E-090 | New | failure | SC-006 | 011 | Critical |
| E2E-091 | New | failure | SC-006 | 011 | Critical |
| E2E-092 | New | failure | SC-006 | 011 | High |
| E2E-093 | New | failure | SC-006 | 011, 012 | High |
| E2E-094 | New | failure | SC-006 | 011 | High |
| E2E-095 | New | edge | SC-006 | 011 | High |
| E2E-096 | New | edge | SC-006 | 011, 022 | Medium |
| E2E-097 | New | edge | SC-006 | 011 | Medium |
| E2E-098 | New | side effect | SC-006 | 021, 011 | Medium |
| E2E-099 | New | side effect | SC-006 | 012, 013 | High |
| E2E-100 | New | edge | SC-001 | 001 | Medium |
| E2E-101 | New | edge | SC-003 | 017 | High |
| E2E-102 | New | edge | SC-001 | 001 | Medium |
| E2E-103 | New | failure | SC-002 | 003 | Medium |
| E2E-104 | New | failure | SC-002 | 028 | Medium |
| E2E-105 | New | failure | SC-002 | 028 | Medium |
| E2E-106 | New | failure | SC-001 | 029 | High |
| E2E-107 | New | failure | SC-005 | 029, 010 | High |
| E2E-108 | New | failure | SC-006 | 011 | High |
| E2E-109 | New | failure | SC-006 | 011 | Medium |
| E2E-110 | New | failure | SC-001 | 018 | High |
| E2E-111 | New | failure | SC-001 | 018 | Medium |
| E2E-112 | New | edge | SC-003 | 002, 006 | High |
| E2E-113 | New | edge | SC-002 | 004, 015 | Medium |
| E2E-114 | New | failure | SC-001 | 015 | Medium |
| E2E-115 | New | edge | SC-002 | 001, 003 | Medium |
| E2E-116 | New | side effect | SC-005 | 030, MOD-002 | High |
| E2E-117 | New | failure | SC-003 | 031 | Medium |
| E2E-118 | New | failure | SC-003 | 031 | Medium |
| E2E-119 | New | failure | SC-004 | 031 | Medium |
| E2E-120 | New | failure | SC-002 | 032 | Medium |
| E2E-121 | New | failure | SC-003 | 032 | Medium |
| E2E-122 | New | failure | SC-002 | 032 | Medium |
| E2E-123 | New | failure | SC-003 | 003 | Medium |
| E2E-124 | New | side effect | SC-001 | 030, MOD-002 | High |
| E2E-125 | New | side effect | SC-001 | 030, MOD-002 | High |
| E2E-126 | New | edge | SC-005 | 030 | Medium |
| E2E-127 | New | happy | SC-001 | 033 | High |
| E2E-128 | New | edge | SC-003 | 033 | Medium |
| E2E-129 | New | failure | SC-001 | 033, 015 | Medium |
| E2E-130 | New | edge | SC-003 | 034 | High |
| E2E-131 | New | failure | SC-003 | 034 | Medium |
| E2E-132 | New | edge | SC-004 | 034 | Medium |
| E2E-133 | New | failure | SC-005 | 035 | High |
| E2E-134 | New | failure | SC-006 | 035 | Medium |
| E2E-135 | New | edge | SC-005 | 035 | Medium |
| E2E-136 | New | edge | SC-002 | 003 | Medium |
| E2E-137 | New | edge | SC-006 | 011 | Medium |
| E2E-138 | New | failure | SC-005 | 035, 013 | High |
| E2E-139 | New | edge | SC-004 | 007 | High |
| E2E-140 | New | edge | SC-003 | 006 | Medium |
| E2E-141 | New | edge | SC-006 | 011 | Medium |
| E2E-142 | New | edge | SC-003 | 002 | High |
| E2E-143 | New | edge | SC-001 | 015 | Medium |
| E2E-144 | New | edge | SC-001 | 033, 007 | Medium |
| E2E-145 | New | edge | SC-001 | 033 | Medium |
| E2E-146 | New | edge | SC-001 | 033 | Medium |
| E2E-147 | New | edge | SC-003 | 002, 006 | Medium |
| E2E-148 | New | edge | SC-003 | 033 | Medium |
| E2E-149 | New | failure | SC-001 | 033, 015 | Medium |
| E2E-150 | New | edge | SC-003 | 033 | Medium |
| E2E-151 | New | failure | SC-001 | 015 | High |
| E2E-152 | New | failure | SC-001 | 015 | High |
| E2E-153 | New | failure | SC-001 | 015 | High |
| E2E-154 | New | edge | SC-003 | 002 | Medium |
| E2E-155 | New | edge | SC-002 | 003 | Medium |
| E2E-156 | New | failure | SC-001 | 013 | High |
| E2E-157 | New | edge | SC-005 | 010 | High |
| E2E-158 | New | edge | SC-003 | 017 | Medium |
| E2E-159 | New | edge | SC-004 | 036, 007 | High |
| E2E-160 | New | edge | SC-004 | 036 | Medium |
| E2E-161 | New | edge | SC-004 | 036 | Medium |
| E2E-162 | New | failure | SC-004 | 037 | High |
| E2E-163 | New | failure | SC-004 | 037 | Medium |
| E2E-164 | New | edge | SC-004 | 037 | Medium |
| E2E-165 | New | failure | SC-002 | 003 | Medium |
| E2E-166 | New | edge | SC-002 | 003 | Medium |
| E2E-167 | New | failure | SC-002 | 003 | Medium |

Coverage: happy 17, failure 72, edge 63, side effect 15, state transition 0 separate (SC-005 and SC-006
exercise every transition), security 0 (not a security lot). Total 167. Happy to failure ratio
1:4.24.

### 9.2 New tests

Each test reads Given / When / Then / And; scenario and requirements are in 9.1.

- **E2E-001** Given Alice has `report.pdf` in Atlas. When she converts it. Then the result reports `2 images, 2 tables` and no missing item. And listing images shows `image-1` `Figure 1: Revenue 2024` page 1 and `image-2` `Schéma réseau 🌐` page 3.
- **E2E-002** Given no external converter is configured and `budget.xlsx` has 2 sheets of one table each. When Alice extracts its text. Then the result reports `0 images, 2 tables`, each table labelled `CSV quality: exact`. And listing images returns an empty list.
- **E2E-003** Given the converter accepts only `.pdf` and `budget.xlsx` holds 2 tables. When Alice extracts the text of `budget.xlsx`. Then 2 tables are listed, each `CSV quality: exact`. And listing images returns an empty list.
- **E2E-004** Given Alice has written 250 000 bytes this session and the artifacts, Markdown sibling and conversion text of `report.pdf` weigh 50 000 bytes together. When she converts it. Then she receives `session write quota of 262144 bytes exceeded`. And listing images answers `No artifacts: document not extracted`, and `report.md` does not exist.
- **E2E-005** Given `deck.pdf` has 5 images and the converter's output marks the caption of the 4th as failed. When Alice converts it. Then the result reports `5 images, 0 tables` and `image-4: caption unavailable`. And `image-4` is listed with an empty caption, images 1, 2, 3, 5 with theirs.
- **E2E-006** Given `three.pdf` converts to text holding 3 pipe tables, the 2nd with a 2 cell header and a 3 cell separator line. When Alice converts it. Then the result reports `0 images, 2 tables` and `table-2: malformed table`. And listing tables shows exactly `table-1` and `table-3`.
- **E2E-007** Given `deck2.pdf` converts to text referencing 5 pictures and the 3rd referenced picture is missing from the converter's output. When Alice converts it. Then the result reports `4 images, 0 tables` and `image-3: picture unavailable`. And listing images shows `image-1`, `image-2`, `image-4`, `image-5`.
- **E2E-008** Given `mixed.pdf` whose 1st figure is extracted as `.jpg` and 2nd as `.svg`. When Alice converts it and gets `image-1`. Then the result reports `image-2: unsupported picture format svg`. And `image-1` has format `jpeg`.
- **E2E-009** Given no external converter is configured and `scan.pdf` holds 3 images and 1 table. When Alice extracts its text. Then listing images returns an empty list. And listing tables returns an empty list.
- **E2E-010** Given the converter accepts only `.pdf`. When Alice uses the convert action on `budget.xlsx`. Then she receives `'/budget.xlsx' cannot be documented, the document service accepts: .pdf`. And listing its tables answers `No artifacts: document not extracted`.
- **E2E-011** Given no external converter is configured. When Alice uses the convert action on `budget.xlsx`. Then she receives `document service is not configured`. And listing its tables answers `No artifacts: document not extracted`.
- **E2E-012** Given Alice has written 0 bytes this session, `report.pdf` weighs 200 000 bytes, and its artifacts, Markdown sibling and conversion text weigh 100 000 bytes together. When she uploads `report.pdf` requesting documentation. Then the upload succeeds and `report.pdf` exists. And the documentation outcome reads `session write quota of 262144 bytes exceeded`, `report.md` does not exist, and listing its images answers `No artifacts: document not extracted`.
- **E2E-013** Given `report.pdf` is in Atlas. When Olga converts it. Then she receives the refusal she receives today for any operation on Atlas. And listing its images as Alice answers `No artifacts: document not extracted`.
- **E2E-014** Given no external converter is configured, Alice has written 262 000 bytes this session and `budget.xlsx` has no Markdown sibling; its Markdown sibling, tables and conversion text weigh 5 000 bytes together. When Alice extracts its text. Then she receives `session write quota of 262144 bytes exceeded`. And `budget.md` does not exist, and listing its tables answers `No artifacts: document not extracted`.
- **E2E-015** Given `plain.pdf` holds text only. When Alice converts it. Then the result reports `0 images, 0 tables`. And listing images and listing tables both return empty lists.
- **E2E-016** Given the converter accepts only `.pdf` and `photo.png` is an image. When Alice extracts the text of `photo.png`. Then the result reports `0 images, 0 tables`. And listing its images returns an empty list.
- **E2E-017** Given the upload of E2E-012 was done. When Alice writes a 62 144 byte file `notes.txt`. Then the write succeeds, only the 200 000 bytes of `report.pdf` having been charged.
- **E2E-018** Given `fresh.pdf` (1 image) was never converted and has no Markdown sibling. When Alice uses the convert action without overwrite. Then the result reports `1 images, 0 tables`. And listing its images shows `image-1`.
- **E2E-019** Given `report.pdf` was converted. When Alice lists its folder and pushes the project. Then the folder shows `report.pdf` and `report.md` only. And the pushed history contains no image or table of `report.pdf`.
- **E2E-020** Given the Markdown sibling of `report.pdf` produced before this spec is kept as reference R. When Alice converts `report.pdf` with overwrite after this spec. Then `report.md` exists at the same place. And its content is byte identical to R.
- **E2E-021** Given Alice has written 0 bytes this session and the artifacts, Markdown sibling and conversion text of `report.pdf` weigh 200 000 bytes together. When she converts it, then writes a 70 000 byte file `notes.txt`. Then the conversion succeeds. And the write of `notes.txt` is refused with `session write quota of 262144 bytes exceeded`.
- **E2E-022** Given `report.pdf` was converted. When Alice searches the project for every file name and for files containing `Q1 sales`. Then no image or table of `report.pdf` appears as a file. And `report.pdf` and `report.md` do.
- **E2E-023** Given `report.pdf` was converted. When Alice exports the project root as a zip. Then the archive holds `report.pdf` and `report.md` only.
- **E2E-024** Given `report.pdf` was converted. When Alice lists its images. Then she gets exactly `image-1` then `image-2`, with captions and pages as in the shared data. And there is no continuation marker.
- **E2E-025** Given `report.pdf` was converted. When Alice gets `image-2`. Then she receives the picture byte identical to the one the converter extracted, its format `png`, and caption `Schéma réseau 🌐`.
- **E2E-026** Given `report.pdf` was converted. When Alice gets `image-7`. Then she receives `Not found: image-7`.
- **E2E-027** Given `report.pdf` was converted. When Olga lists its images, then gets `image-1`. Then both answers are the refusal she receives today for any operation on Atlas. And no caption or picture is returned.
- **E2E-028** Given `notes.pdf` was never converted. When Alice lists its images, then gets `image-1`. Then both answers are `No artifacts: document not extracted`.
- **E2E-029** Given `atlas.pdf` was converted with 100 images. When Alice lists its images. Then one page holds `image-1` to `image-100`. And there is no continuation marker.
- **E2E-030** Given `atlas2.pdf` was converted with 101 images. When Alice lists its images and follows the marker. Then page 1 holds `image-1` to `image-100` with a marker. And page 2 holds only `image-101`, with no marker.
- **E2E-031** Given `one.pdf` has 1 image with no caption, on page 2, converted. When Alice lists its images, then gets `image-1`. Then the list has one entry, `image-1`, page 2, empty caption. And getting it returns the picture with an empty caption.
- **E2E-032** Given `cap.pdf` was converted and its only image has caption `Coût | Été 😀 "v2"`. When Alice gets `image-1`. Then the caption is returned exactly `Coût | Été 😀 "v2"`.
- **E2E-033** Given `report.pdf` was converted. When Alice lists images and gets `image-1` once as an AI assistant and once as an HTTP API consumer. Then ids, captions, pages, picture bytes and format are identical.
- **E2E-034** Given `report.pdf` was converted. When Alice lists its tables. Then she gets `table-1` `Q1 sales` 3 rows 4 columns and `table-2` `Costs` 2 rows 2 columns. And each is labelled `CSV quality: approximate`.
- **E2E-035** Given `report.pdf` was converted. When Alice gets `table-1` with no format. Then she receives exactly the 5 lines `| Region | Jan | Feb | Mar |`, `| --- | --- | --- | --- |`, `| North | 10 | 11 | 12 |`, `| South | 20 | 21 | 22 |`, `| West | 30 | 31 | 32 |`, with no trailing newline.
- **E2E-036** Given `report.pdf` was converted. When Alice gets `table-1` as `csv`. Then she receives 4 lines (header and 3 rows) of 4 comma separated fields.
- **E2E-037** Given `report.pdf` was converted. When Alice gets `table-9`. Then she receives `Not found: table-9`.
- **E2E-038** Given `report.pdf` was converted. When Olga lists its tables, then gets `table-1` as `markdown` and as `csv`. Then each answer is the refusal she receives today for any operation on Atlas.
- **E2E-039** Given `report.pdf` was converted. When Alice gets `table-1` as `xml`. Then she receives `Unsupported format: xml (use markdown or csv)`. And no table content is returned.
- **E2E-040** Given `report.pdf` was converted. When Alice gets `table-1` as `CSV`. Then she receives `Unsupported format: CSV (use markdown or csv)`. And no table content is returned.
- **E2E-041** Given `report.pdf` was converted. When Alice gets `table-1` as `md`. Then she receives `Unsupported format: md (use markdown or csv)`. And no table content is returned.
- **E2E-042** Given `notes.pdf` was never converted. When Alice lists its tables, then gets `table-1` as `csv`. Then both answers are `No artifacts: document not extracted`.
- **E2E-043** Given `report.pdf` was converted and the text of `budget.xlsx` was extracted. When Alice lists the tables of both. Then those of `report.pdf` read `CSV quality: approximate`. And those of `budget.xlsx` read `CSV quality: exact`.
- **E2E-044** Given the text of `names.xlsx` was extracted, holding one table with cells `a|b`, `Dupont, "Jr"` and `Zoë 🚀`. When Alice gets it as `markdown`, then as `csv`. Then the Markdown holds `a\|b` and keeps 3 columns. And the CSV holds `"Dupont, ""Jr"""` and `Zoë 🚀`, and reading it back gives the three original cells exactly.
- **E2E-045** Given `big.pdf` was converted with 101 tables. When Alice lists its tables and follows the marker. Then page 1 holds `table-1` to `table-100`. And page 2 holds only `table-101`.
- **E2E-046** Given the text of `empty.xlsx` was extracted, holding one table with a header and 0 data rows. When Alice lists its tables, then gets `table-1` as `csv`. Then the list shows 0 rows. And the CSV is the header line only.
- **E2E-047** Given no external converter is configured and `ledger.xlsx` holds one table of 401 data rows by 3 columns. When Alice extracts its text, lists its tables and gets `table-1` as `csv`. Then the list shows 401 rows. And the CSV has 402 lines (header and 401 rows).
- **E2E-048** Given `report.pdf` was converted. When Alice gets `table-2` as `markdown`. Then she receives exactly 4 lines, `| Item | EUR |`, `| --- | --- |`, `| Rent | 900 |`, `| Power | 120 |`, with no trailing newline.
- **E2E-049** Given `report.pdf` was converted. When Alice gets `table-1` as `markdown` and as `csv`, once as an AI assistant and once as an HTTP API consumer. Then both pairs are byte identical.
- **E2E-050** Given `report.pdf` was converted. When Alice gets the full document with default options. Then the line `[Image image-1: Figure 1: Revenue 2024]` stands at the first image's place. And the 5 lines of E2E-035 stand right after the line `Table 1: Q1 sales`, and the 4 lines of E2E-048 right after `Table 2: Costs`.
- **E2E-051** Given `report.pdf` was converted. When Alice gets the full document with table mode `csv-reference`. Then the first table reads exactly `[Table table-1: Q1 sales (CSV)]`. And no Markdown table appears.
- **E2E-052** Given `report.pdf` was converted. When Alice gets the full document with table mode `both` and captions off. Then each Markdown table is followed by its reference line, the first being `[Table table-1: Q1 sales (CSV)]`. And the image lines read `[Image image-1]` and `[Image image-2]`.
- **E2E-053** Given `report.pdf` was converted. When Alice gets the full document with captions off in `markdown` mode. Then the text holds the lines `[Image image-1]` and `[Image image-2]`, and neither `Figure 1: Revenue 2024` nor `Schéma réseau 🌐` appears. And both tables appear as Markdown tables.
- **E2E-054** Given `report.pdf` was converted. When Alice asks for table mode `html`. Then she receives `Unsupported table mode: html (use markdown, csv-reference or both)`. And no document text is returned.
- **E2E-055** Given `report.pdf` was converted. When Alice asks for table mode `CSV-REFERENCE`. Then she receives `Unsupported table mode: CSV-REFERENCE (use markdown, csv-reference or both)`. And no document text is returned.
- **E2E-056** Given `report.pdf` was converted. When Alice asks for table mode `markdown,both`. Then she receives `Unsupported table mode: markdown,both (use markdown, csv-reference or both)`. And no document text is returned.
- **E2E-057** Given `notes.pdf` was never converted. When Alice asks for its full document view. Then she receives `No artifacts: document not extracted`. And no `notes.md` is created.
- **E2E-058** Given `report.pdf` was converted. When Olga asks for its full document view in `both` mode. Then she receives the refusal she receives today for any operation on Atlas. And no text is returned.
- **E2E-059** Given `report.pdf` was converted, then Alice deleted `report.md`, then overwrote `report.pdf`. When Alice asks for its full document view. Then she receives `No artifacts: document not extracted`.
- **E2E-060** Given `report3.pdf`, with tables `Q1 sales`, `Costs` and a third table with no caption, was converted. When Alice gets the full document in `csv-reference`. Then the third table reads exactly `[Table table-3 (CSV)]`. And the first reads `[Table table-1: Q1 sales (CSV)]`.
- **E2E-061** Given `plain.pdf` (no image) was converted. When Alice gets its full document with captions off, then with captions on. Then both texts are byte identical.
- **E2E-062** Given `one.pdf` (1 image, no caption) was converted. When Alice gets its full document with default options. Then the text holds the line `[Image image-1]`.
- **E2E-063** Given `report.pdf` was converted. When Alice gets the full document with default options. Then the line `[Image image-1: Figure 1: Revenue 2024]` appears exactly once, and `[Image image-2: Schéma réseau 🌐]` exactly once after it. And each stands between empty lines.
- **E2E-064** Given `report.pdf` was converted. When Alice gets the full document with default options. Then no image reference or figure path from the conversion text remains. And the text outside image and table positions equals the conversion text.
- **E2E-065** Given `report.pdf` was converted, then Alice replaced `report.md` with the single line `edited`. When Alice gets the full document with default options. Then the text contains `[Image image-1: Figure 1: Revenue 2024]`. And it does not contain the line `edited`.
- **E2E-066** Given `report.pdf` was converted, then Alice deleted `report.md`. When Alice gets the full document in `csv-reference` mode. Then the text contains `[Table table-1: Q1 sales (CSV)]`.
- **E2E-067** Given `report.pdf` was converted. When Alice gets the full document in `both` mode once as an AI assistant and once as an HTTP API consumer. Then the two texts are byte identical.
- **E2E-068** Given `report.pdf` was converted. When Alice gets the full document in each of the three table modes, then reads `report.md`. Then `report.md` is byte identical to its content before the views.
- **E2E-069** Given `report.pdf` was converted (2 images) and the converter now returns 3 images for it, captioned `A`, `B`, `C`. When Alice re-converts it with overwrite. Then listing images shows exactly `image-1` `A`, `image-2` `B`, `image-3` `C`. And no caption `Schéma réseau 🌐` remains.
- **E2E-070** Given `report.pdf` was converted (2 images, 2 tables). When Alice extracts its text with refresh. Then listing its images returns an empty list. And listing its tables returns an empty list (the built-in conversion finds no table in a PDF).
- **E2E-071** Given `report.pdf` was converted, Alice has 10 000 bytes of session write quota left and the new set weighs 50 000 bytes. When Alice re-converts it with overwrite. Then she receives `session write quota of 262144 bytes exceeded`. And its 2 images, 2 tables and `report.md` are unchanged byte for byte.
- **E2E-072** Given `report.pdf` was converted and the converter is unreachable. When Alice re-converts it with overwrite. Then she receives the converter's existing failure message, unchanged from today. And its 2 images, 2 tables and `report.md` are unchanged.
- **E2E-073** Given `report.pdf` was converted (2 images) and the converter now returns 3 images for it. When Alice uses the convert action without overwrite. Then she receives `'/report.md' exists (pass overwrite=true)`. And listing images shows `image-1` and `image-2` with their former captions.
- **E2E-074** Given `report.pdf` was converted. When Olga re-converts it with overwrite. Then she receives the refusal she receives today for any operation on Atlas. And the 2 images and 2 tables are unchanged.
- **E2E-075** Given `report.pdf` was converted (2 images) and Alice has 10 bytes of session write quota left; the new sibling and set
  weigh 4 000 bytes together. When Alice extracts its text with refresh. Then she receives `session write quota of 262144 bytes exceeded`. And listing its images still returns `image-1` and `image-2`.
- **E2E-076** Given `report.pdf` was converted, `report.md` is kept as reference R, and Alice has 10 bytes of session write quota left; the new sibling and set
  weigh 4 000 bytes together. When Alice extracts its text with refresh. Then she receives `session write quota of 262144 bytes exceeded`. And `report.md` is byte identical to R.
- **E2E-077** Given `report.pdf` was converted with `report.md` kept as reference R, and Alice has 10 bytes of session write quota left; the new sibling and set
  weigh 4 000 bytes together. When Alice re-converts it with overwrite. Then she receives `session write quota of 262144 bytes exceeded`. And `report.md` is byte identical to R, and its 2 images are unchanged.
- **E2E-078** Given `report.pdf` was converted with `report.md` kept as reference R, and Alice's session write quota is 600 000 bytes with 300 000 left. When she uploads a new 290 000 byte `report.pdf` requesting documentation, whose new artifacts, Markdown
  sibling and conversion text weigh 50 000 bytes together. Then the new `report.pdf` is kept and the documentation outcome reads `session write quota of 600000 bytes exceeded`. And `report.md` is byte identical to R, and listing its images answers `No artifacts: document not extracted`.
- **E2E-079** Given `report.pdf` was converted with 2 images and the converter now returns 1 image for it. When Alice re-converts it with overwrite, then gets `image-2`. Then she receives `Not found: image-2`.
- **E2E-080** Given `report.pdf` was converted (2 images). When Alice and Bob re-convert it with overwrite at the same moment and both finish, then Alice lists its images. Then she gets exactly `image-1` and `image-2`, once each.
- **E2E-081** Given `report.pdf` was converted (2 images) and the converter now returns 5 images for it. When Alice re-converts it with overwrite while Bob lists its images 20 times. Then every answer Bob gets holds exactly 2 or exactly 5 images, never another count or a mix of captions.
- **E2E-082** Given `report.pdf` was converted (2 images). When Alice extracts its text without refresh and the existing Markdown sibling answers it. Then listing its images still returns `image-1` and `image-2`.
- **E2E-083** Given `report.pdf` was converted and the converter now returns 1 image for it. When Alice uses the convert action with overwrite. Then the result reports `1 images, 2 tables`. And listing images shows only `image-1`.
- **E2E-084** Given `report.pdf` was converted (2 images). When Alice uploads a new `report.pdf` with 1 image requesting documentation. Then listing its images shows only `image-1`, with the new caption. And `Schéma réseau 🌐` is not listed.
- **E2E-085** Given `report.pdf` was converted, the converter now returns a different set for it, and Alice has 60 000 bytes of session write quota left; the new set, sibling and conversion text weigh 50 000 bytes. When she re-converts it with overwrite, then writes a 20 000 byte file. Then the re-conversion succeeds. And the write is refused with `session write quota of 262144 bytes exceeded`, the old set's bytes not having been given back.
- **E2E-086** Given `report.pdf` was converted. When Alice reads `report.pdf`, lists its folder and reads `report.md`. Then listing its images still returns `image-1` and `image-2`.
- **E2E-087** Given `report.pdf` was converted and moved to `archive/r.pdf`. When Alice converts `archive/r.pdf`. Then `archive/r.pdf` lists `image-1` and `image-2`. And before that conversion it answered `No artifacts: document not extracted`.
- **E2E-088** Given `report.pdf` was converted. When Alice moves it to `b.pdf`, then lists the images of `b.pdf` and of `report.pdf`. Then `b.pdf` answers `No artifacts: document not extracted`. And `report.pdf` answers `not a file: /report.pdf`.
- **E2E-089** Given `report.pdf` was converted. When Alice copies it to `copy.pdf`. Then `copy.pdf` answers `No artifacts: document not extracted`. And `report.pdf` still lists `image-1` and `image-2`.
- **E2E-090** Given `report.pdf` was converted. When Alice deletes it to the trash, then restores it. Then listing its images answers `No artifacts: document not extracted`.
- **E2E-091** Given `report.pdf` was converted. When Alice writes the same bytes again to `report.pdf`. Then listing its images answers `No artifacts: document not extracted`.
- **E2E-092** Given converted `report.pdf` is tracked in version control and a remote commit changes it. When Alice pulls. Then listing its images answers `No artifacts: document not extracted`.
- **E2E-093** Given `report.pdf` was converted and `notes.pdf` never was. When Alice copies `notes.pdf` over `report.pdf` with overwrite. Then `report.pdf` answers `No artifacts: document not extracted`.
- **E2E-094** Given `report.pdf` was converted. When Alice extracts an archive in the same folder that holds a `report.pdf`, replacing it. Then listing its images answers `No artifacts: document not extracted`.
- **E2E-095** Given folder `q1/` holds converted `report.pdf`. When Alice moves `q1/` to `archive/`. Then `archive/report.pdf` answers `No artifacts: document not extracted`.
- **E2E-096** Given `report.pdf` was converted. When Alice rewrites `report.md` with the line `edited`. Then listing the images of `report.pdf` still returns `image-1` and `image-2`.
- **E2E-097** Given `report.pdf` was converted. When Alice moves it to `b.pdf` and back to `report.pdf`. Then listing its images answers `No artifacts: document not extracted`.
- **E2E-098** Given `report.pdf` was converted. When Alice deletes it to the trash and lists the trash. Then the trash shows `report.pdf` only, with no image or table entry.
- **E2E-099** Given Alice has 10 000 bytes of session write quota left and converted `small.pdf` weighs 5 000 bytes with 200 000 bytes of artifacts. When she copies it to `c.pdf`, then writes a 4 000 byte file. Then both succeed, only the 5 000 file bytes having been charged for the copy.
- **E2E-100** Given no external converter is configured and `blank.png` yields no text. When Alice extracts its text. Then the result reports `0 images, 0 tables` and `blank.md` does not exist. And listing its images returns an empty list.
- **E2E-101** Given `budget.xlsx` has sheets `Q1` and `Q2`, one table each. When Alice extracts its text and lists its tables. Then `table-1` has caption `Q1` and `table-2` has caption `Q2`.
- **E2E-102** Given `logo.pdf` shows the same logo on pages 1 and 2. When Alice converts it. Then listing images shows `image-1` page 1 and `image-2` page 2. And their pictures are byte identical.
- **E2E-103** Given `report.pdf` was converted. When Alice lists its images with continuation marker `zzz`. Then she receives `Invalid continuation marker: zzz`.
- **E2E-104** Given folder `q1/` exists. When Alice lists the images of `q1`. Then she receives `not a file: /q1`.
- **E2E-105** Given no file `ghost.pdf` exists. When Alice lists the tables of `ghost.pdf`. Then she receives `not a file: /ghost.pdf`.
- **E2E-106** Given no external converter is configured. When Alice uploads `report.pdf` requesting documentation. Then she receives `document service is not configured`. And `report.pdf` does not exist.
- **E2E-107** Given `report.pdf` was converted while a converter was configured, then the operator removed the converter. When Alice uses the convert action with overwrite. Then she receives `document service is not configured`. And listing its images still returns `image-1` and `image-2`.
- **E2E-108** Given `report.pdf` was converted. When Alice edits `report.pdf` in place (any successful edit). Then listing its images answers `No artifacts: document not extracted`.
- **E2E-109** Given `report.pdf` is tracked in version control, its committed version differs from the current one, and the current one was converted. When Alice reverts `report.pdf` to its committed version. Then listing its images answers `No artifacts: document not extracted`.
- **E2E-110** Given the converter accepts only `.pdf`. When Alice uploads `budget.xlsx` requesting documentation. Then she receives `'/budget.xlsx' cannot be documented, the document service accepts: .pdf`. And `budget.xlsx` does not exist.
- **E2E-111** Given the converter accepts only `.pdf`. When Alice uses the convert action on `photo.png`. Then she receives `'/photo.png' cannot be documented, the document service accepts: .pdf`. And listing its images answers `No artifacts: document not extracted`.
- **E2E-112** Given `report.pdf` was converted. When Alice gets `table-2` as `csv`. Then she receives exactly the 3 lines `Item,EUR`, `Rent,900`, `Power,120`, separated by a single LF, with no trailing newline. And no field is quoted.
- **E2E-113** Given `scan2.pdf` whose only figure is extracted as `.tif`. When Alice converts it and gets `image-1`. Then the result reports `1 images, 0 tables` and no missing item. And `image-1` has format `tiff`.
- **E2E-114** Given `mix2.pdf` converts to text referencing 3 pictures and holding 2 pipe tables, the 2nd referenced picture is missing from the converter's output and the 1st table's separator line does not match its header. When Alice converts it. Then the result reports `2 images, 1 tables`, then `image-2: picture unavailable`, then `table-1: malformed table`, in that order.
- **E2E-115** Given `memo.docx` holds 1 image captioned `Org chart` and the converter accepts `.docx`. When Alice converts it and lists its images. Then she gets `image-1`, `Org chart`, with an empty page.
- **E2E-116** Given `report.pdf` was converted. When Alice extracts its text with refresh. Then the result reports `0 images, 0 tables`. And the full document view in `markdown` mode with captions on equals the content of `report.md` byte for byte.
- **E2E-117** Given `notes.pdf` was never converted. When Alice gets `table-9` of `notes.pdf` as `xml`. Then she receives `Unsupported format: xml (use markdown or csv)`.
- **E2E-118** Given `report.pdf` was converted. When Alice gets `table-9` as `xml`. Then she receives `Unsupported format: xml (use markdown or csv)`.
- **E2E-119** Given no file `ghost.pdf` exists. When Alice asks for the full document of `ghost.pdf` with mode `html`. Then she receives `not a file: /ghost.pdf`.
- **E2E-120** Given `notes.pdf` was never converted. When Alice lists its images as an HTTP API consumer and as an AI assistant. Then both answers carry the not found category and the text `No artifacts: document not extracted`.
- **E2E-121** Given `report.pdf` was converted. When Alice gets `table-1` as `xml`. Then the refusal carries the invalid argument category.
- **E2E-122** Given `report.pdf` was converted. When Alice lists its images with marker `zzz`. Then the refusal carries the invalid argument category.
- **E2E-123** Given `atlas2.pdf` (101 images) and `big.pdf` (101 tables) were converted. When Alice lists the tables of `big.pdf` with the marker from page 1 of the images of `atlas2.pdf`. Then she receives `Invalid continuation marker: <that marker>`, the marker spelled as issued.
- **E2E-124** Given `report.pdf` is in Atlas. When Alice uses the convert action. Then the result carries the same items as today, the Markdown sibling path among them. And it also carries the report `2 images, 2 tables`.
- **E2E-125** Given `report.pdf` is in Atlas. When Alice uploads it requesting documentation. Then the documentation outcome carries the Markdown sibling path as today. And it carries the report `2 images, 2 tables`.
- **E2E-126** Given `report.pdf` was converted. When Alice extracts its text without refresh and the existing sibling answers it. Then the result carries no conversion report.
- **E2E-127** Given the converter returns for `cap2.pdf` a picture reference described `Revenue chart`, then a line `Table 1: Costs`, then a 2 row pipe table. When Alice converts it and lists its images and tables. Then `image-1` has caption `Revenue chart`. And `table-1` has caption `Costs` and 2 data rows.
- **E2E-128** Given the converter returns for `nocap.pdf` a pipe table preceded by the line `Summary`. When Alice converts it and lists its tables. Then `table-1` has an empty caption.
- **E2E-129** Given the converter's output for `blur.pdf` marks the caption of its only picture as failed. When Alice converts it. Then the result reports `1 images, 0 tables`, then `image-1: caption unavailable`. And `image-1` is listed with an empty caption.
- **E2E-130** Given `two.xlsx` holds sheet `A` (3 rows) then sheet `B` (3 rows), and the character limit Alice requests cuts its text inside sheet `B`'s table. When Alice extracts its text and lists its tables. Then exactly `table-1`, caption `A`, is listed.
- **E2E-131** Given `two.xlsx` as in E2E-130 and a character limit cutting its text inside sheet `A`'s table. When Alice extracts its text and lists its tables. Then the list is empty.
- **E2E-132** Given `two.xlsx` was extracted as in E2E-130. When Alice gets its full document. Then the text ends where the cut text ends, and `table-1` appears once.
- **E2E-133** Given `report.pdf` was converted and the converter takes 5 seconds. When Alice re-converts it with overwrite and Bob writes new bytes to `report.pdf` 1 second later. Then Alice receives `Document changed during conversion: /report.pdf`. And listing its images answers `No artifacts: document not extracted`.
- **E2E-134** Given `fresh.pdf` was never converted and the converter takes 5 seconds. When Alice converts it and Bob moves it to `moved.pdf` 1 second later. Then Alice receives `Document changed during conversion: /fresh.pdf`. And `moved.pdf` answers `No artifacts: document not extracted`.
- **E2E-135** Given `report.pdf` was converted and the converter takes 5 seconds. When Alice re-converts it with overwrite and Bob only reads `report.pdf` meanwhile. Then the re-conversion succeeds and lists its images.
- **E2E-136** Given `atlas2.pdf` (101 images) was converted, Alice holds the page 1 marker, and the converter now returns 3 images for it. When Alice re-converts it with overwrite, then lists its images with that marker. Then she receives an empty list with no continuation marker.
- **E2E-137** Given `report.pdf` in Atlas was converted. When an admin deletes the project Atlas and then restores it. Then Alice listing its images gets `image-1` and `image-2`.
- **E2E-138** Given `report.pdf` was converted, `report.md` is kept as reference R, and Alice has written 0 bytes this session; the converter takes 5 seconds. When Alice re-converts it with overwrite and Bob writes new bytes to `report.pdf` 1 second later, and then Alice writes a 262 144 byte file `n.txt`. Then Alice's re-conversion is refused with `Document changed during conversion: /report.pdf` in the invalid argument category. And `report.md` is byte identical to R, and the write of `n.txt` succeeds.
- **E2E-139** Given the converter returns for `fig.pdf` the lines `Intro`, the picture reference, `Figure 1: Revenue 2024`, `Body text`, and describes the picture `Figure 1: Revenue 2024`. When Alice converts it and gets the full document with default options. Then the text reads exactly `Intro`, an empty line, `[Image image-1: Figure 1: Revenue 2024]`, an empty line, `Body text`.
- **E2E-140** Given the text of `empty.xlsx` (header `A`, `B`, 0 data rows) was extracted. When Alice gets `table-1` as `markdown`. Then she receives exactly the 2 lines `| A | B |` and `| --- | --- |`, with no trailing newline.
- **E2E-141** Given converted `report.pdf` is tracked and identical on branches `main` and `dev`, and `other.txt` differs between them. When Alice checks out `dev`. Then listing the images of `report.pdf` returns `image-1` and `image-2`.
- **E2E-142** Given the converter returns for `esc.pdf` the pipe table `| k | v |`, `| --- | --- |`, `|  a\|b  | **x** |`. When Alice converts it and gets `table-1` as `csv`, then as `markdown`. Then the CSV is exactly the 2 lines `k,v` and `a|b,**x**`, separated by one LF. And the Markdown's last line is exactly `| a\|b | **x** |`.
- **E2E-143** Given the converter silently drops the 2nd of 3 figures of `drop.pdf`, so its conversion text references only 2 pictures. When Alice converts it. Then the result reports `2 images, 0 tables` with no missing item. And listing images shows `image-1` and `image-2`.
- **E2E-144** Given the converter returns for `inline.pdf` the line `See ![chart](figures/figure_1.png) here` and that picture. When Alice converts it and gets the full document with default options. Then the result reports `0 images, 0 tables`. And the view holds the line `See ![chart](figures/figure_1.png) here` unchanged.
- **E2E-145** Given the converter returns for `code.pdf` a fenced code block holding `| a | b |` then `| --- | --- |`. When Alice converts it and lists its tables. Then the list is empty and the result reports `0 images, 0 tables`.
- **E2E-146** Given the converter returns for `sep.pdf` the lines `| a | b |` then `| c | d |`. When Alice converts it. Then the result reports `0 images, 0 tables` with no missing item.
- **E2E-147** Given `ledger.csv` holds a header and 401 data rows of 3 columns. When Alice extracts its text, lists its tables and gets `table-1` as `csv`. Then the list shows 401 rows. And the CSV has 402 lines.
- **E2E-148** Given the converter returns for `bare.pdf` the lines `a | b` then `--- | ---` then `1 | 2`. When Alice converts it. Then the result reports `0 images, 0 tables` with no missing item.
- **E2E-149** Given the converter returns for `esc2.pdf` the lines `| a\|b | c |` then `| --- | --- | --- |`. When Alice converts it. Then the result reports `0 images, 0 tables` then `table-1: malformed table`.
- **E2E-150** Given the converter returns for `trail.pdf` the lines `| a | b` then `| --- | ---` then `| 1 | 2`. When Alice converts it and lists its tables. Then `table-1` shows 1 data row and 2 columns.
- **E2E-151** Given the converter returns for `trav.pdf` the lines `![x](../../secret.png)` then `![y](figures/figure_1.png)` with that figure. When Alice converts it. Then the result reports `1 images, 0 tables` then `image-1: picture unavailable`. And listing images shows only `image-2`.
- **E2E-152** Given the converter returns for `url.pdf` the line `![x](https://example.com/a.png)`. When Alice converts it. Then the result reports `0 images, 0 tables` then `image-1: picture unavailable`.
- **E2E-153** Given the converter returns for `abs.pdf` the line `![x](/etc/hosts.png)`. When Alice converts it. Then the result reports `0 images, 0 tables` then `image-1: picture unavailable`.
- **E2E-154** Given `crlf.csv` holds header `k`, `v` and one row `x` and a `v` cell made of `a`, CR LF, `b`. When Alice extracts its text and gets `table-1` as `markdown`, then as `csv`. Then the Markdown's last line is exactly `| x | a b |`. And in the CSV the cell reads `"a`, CR LF, `b"`.
- **E2E-155** Given `atlas2.pdf` (101 images) was converted and Alice holds the page 1 marker. When Bob lists its images with that marker. Then he gets only `image-101`, with no marker.
- **E2E-156** Given Alice has written 0 bytes this session, and for `t.pdf` the set weighs 100 000 bytes, the Markdown sibling 80 000 bytes and the conversion text 80 000 bytes. When she converts it, then writes a 10 000 byte file `n.txt`. Then the conversion succeeds. And the write of `n.txt` is refused with `session write quota of 262144 bytes exceeded`.
- **E2E-157** Given `report.pdf` was converted, and the converter returns 3 images for Alice's request and 4 for Bob's, Bob's finishing last. When both re-convert it with overwrite at the same moment. Then Alice's result reports `3 images, 2 tables` and Bob's reports `4 images, 2 tables`. And listing images shows `image-1` to `image-4`, and `report.md` is the sibling of Bob's conversion.
- **E2E-158** Given `gaps.xlsx` holds header `a`, `b`, a row `1`, `2`, a fully blank row, then a row `3`, `4`. When Alice extracts its text, lists its tables and gets `table-1` as `csv`. Then the list shows 2 data rows. And the CSV is exactly `a,b`, `1,2`, `3,4`, separated by one LF.
- **E2E-159** Given the converter returns for `gap.pdf` the lines `Intro`, an empty line, the picture reference, an empty line, `Figure 1: Revenue 2024`, an empty line, `Body text`, and describes the picture `Figure 1: Revenue 2024`. When Alice converts it and gets the full document with default options. Then the text is exactly the 5 lines `Intro`, an empty line, `[Image image-1: Figure 1: Revenue 2024]`, an empty line, `Body text`.
- **E2E-160** Given the converter returns for `top.pdf` the picture reference as its first line, then an empty line, then `Body text`, with no description. When Alice converts it and gets the full document. Then the text is exactly the 3 lines `[Image image-1]`, an empty line, `Body text`.
- **E2E-161** Given the converter returns for `end.pdf` the lines `Intro`, two empty lines, the picture reference, with no description. When Alice converts it and gets the full document. Then the text is exactly the 3 lines `Intro`, an empty line, `[Image image-1]`.
- **E2E-162** Given `trav.pdf` as in E2E-151 was converted. When Alice gets its full document with default options. Then the text holds the line `![x](../../secret.png)` unchanged and the line `[Image image-2]`. And it holds no line starting with `[Image image-1`.
- **E2E-163** Given `mixed.pdf` as in E2E-008 was converted. When Alice gets its full document. Then the reference line of its `.svg` figure stands unchanged. And no line starting with `[Image image-2` appears.
- **E2E-164** Given `url.pdf` as in E2E-152 was converted. When Alice gets its full document with captions off. Then the text holds `![x](https://example.com/a.png)` unchanged.
- **E2E-165** Given project `Borealis`, where Alice is a member, holds a converted `/atlas2.pdf` with 101 images, and Alice holds the page 1 marker of `/atlas2.pdf` in Atlas. When she lists the images of `/atlas2.pdf` in Borealis with that marker. Then she receives `Invalid continuation marker: <that marker>`, the marker spelled as issued.
- **E2E-166** Given the marker of E2E-165. When Alice uses it in Atlas on `/atlas2.pdf`. Then she gets only `image-101`.
- **E2E-167** Given the marker of E2E-165. When Alice lists the images of `/big.pdf` in Borealis with it. Then she receives `Invalid continuation marker: <that marker>`, the marker spelled as issued.

### 9.3 to 9.5

No exploit test (not a security lot), no modified test, no removed test.

## 10. Traceability

| Scenario | Requirements (FR-NEW- unless MOD-) | Happy | Failure | Edge | Side effect |
|---|---|---|---|---|---|
| SC-001 | 001, 002, 003, 004, 005, 007, 013, 014, 015, 016, 017, 018, 021, 022, 027, 029, 030, 033, MOD-001, MOD-002 | E2E-001, 002, 003, 127 | E2E-004, 005, 006, 007, 008, 009, 010, 011, 012, 013, 014, 106, 110, 111, 114, 129, 149, 151, 152, 153, 156 | E2E-015, 016, 017, 018, 100, 102, 143, 144, 145, 146 | E2E-019, 020, 021, 022, 023, 124, 125 |
| SC-002 | 001, 003, 004, 015, 016, 019, 020, 023, 028, 032 | E2E-024, 025 | E2E-026, 027, 028, 103, 104, 105, 120, 122, 165, 167 | E2E-029, 030, 031, 032, 113, 115, 136, 155, 166 | E2E-033 |
| SC-003 | 002, 003, 005, 006, 016, 017, 019, 020, 023, 024, 031, 032, 033, 034 | E2E-034, 035, 036 | E2E-037, 038, 039, 040, 041, 042, 117, 118, 121, 123, 131 | E2E-043, 044, 045, 046, 047, 048, 101, 112, 128, 130, 140, 142, 147, 148, 150, 154, 158 | E2E-049 |
| SC-004 | 007, 008, 009, 011, 016, 020, 022, 023, 025, 031, 034, 036, 037 | E2E-050, 051, 052, 053 | E2E-054, 055, 056, 057, 058, 059, 119, 162, 163 | E2E-060, 061, 062, 063, 064, 065, 066, 132, 139, 159, 160, 161, 164 | E2E-067, 068 |
| SC-005 | 010, 011, 013, 014, 016, 019, 022, 026, 027, 029, 030, 035, MOD-001, MOD-002 | E2E-069, 070 | E2E-071, 072, 073, 074, 075, 076, 077, 078, 107, 133, 138 | E2E-079, 080, 081, 082, 083, 084, 126, 135, 157 | E2E-085, 116 |
| SC-006 | 001, 011, 012, 013, 021, 022, 028, 035 | E2E-086, 087 | E2E-088, 089, 090, 091, 092, 093, 094, 108, 109, 134 | E2E-095, 096, 097, 137, 141 | E2E-098, 099 |

Tests per requirement (FR-NEW- unless MOD-, from 9.1): 001: 10; 002: 11; 003: 13; 004: 5; 005: 4; 006: 8; 007: 10; 008: 3; 009: 3; 010: 10; 011: 19; 012: 3; 013: 10; 014: 3; 015: 13; 016: 5; 017: 7; 018: 3; 019: 3; 020: 3; 021: 4; 022: 4; 023: 3; 024: 3; 025: 3; 026: 3; 027: 3; 028: 3; 029: 3; 030: 4; 031: 3; 032: 3; 033: 9; 034: 3; 035: 4; 036: 3; 037: 3; MOD-001: 3; MOD-002: 3.

## 11. Open questions

None.

## 12. Glossary

| Term | Definition | Context |
|---|---|---|
| Document | a file of the project that can be converted | Documents |
| Artifact | an image or a table extracted from a document and kept with it | Documents |
| Image | an artifact holding a picture and its caption | Documents |
| Picture | the image bytes of an image artifact | Documents |
| Table | an artifact holding rows and columns, offered as Markdown and CSV | Documents |
| Data row | a table row other than its header | Documents |
| Caption | the text describing an image or a table, as extracted | Documents |
| Converter | the external tool or service turning a document into text and artifacts | Documents |
| Built-in conversion | the conversion the product performs itself, used by text extraction | Documents |
| Text extraction | reading a document's text with the built-in conversion, which writes or reuses the Markdown sibling | Documents |
| OCR | reading text out of a picture | Documents |
| CSV quality | `exact` when built from original cells, `approximate` when derived from Markdown | Documents |
| Markdown sibling | the `.md` file written beside a converted document, as today | Documents |
| Conversion text | the Markdown text the last conversion produced, kept apart from the editable sibling | Documents |
| View | the full document assembled from the conversion text, image lines and tables | Documents |
| Image line | the line `[Image image-<n>: <caption>]` standing for an image in a view | Documents |
| Table mode | how tables appear in a view: `markdown`, `csv-reference`, `both` | Documents |
| Table reference line | the line `[Table table-<n>: <caption> (CSV)]` standing for a table in a view | Documents |
| Reading position | where an artifact sits in the conversion text | Documents |
| Re-conversion | converting an already converted document again | Documents |
| Continuation marker | the signal that a list has another page | Documents |
| Document change | a move, rename, deletion or replacement of the document's bytes | Files |
| Session write quota | the bytes a member is allowed to write during a session; never given back | Quota |
| Member | a person who belongs to the project and can read and write its documents | Access |
| Outsider | a person who does not belong to the project | Access |
| Refusal | the answer given to an outsider for any operation on the project | Access |

## 13. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 to 5 | 7, 6, 4, 5, 3 | 6, 3, 2, 0, 2 | NOT-IMPLEMENTABLE: non convergent on artifact lifecycle; owner cut the scope (lifecycle following moved to BL-0038) |
| 6 (rescoped) | 7 | 4 | NOT-IMPLEMENTABLE, precision only, applied |
| 7 | 4 | 1 | NOT-IMPLEMENTABLE, precision only, applied |
| 8 | 4 | 1 | NOT-IMPLEMENTABLE, precision only, applied |
| 9 | 6 | 2 | NOT-IMPLEMENTABLE, applied (caption source chosen per owner's standing instruction to follow recommendations) |
| 10 | 4 | 2 | NOT-IMPLEMENTABLE, precision only, applied |
| 11 | 2 | 1 | NOT-IMPLEMENTABLE, precision only, applied |
| 12 | 1 | 3 | NOT-IMPLEMENTABLE, precision only, applied |
| 13 | 4 | 0 | NOT-IMPLEMENTABLE, precision only, applied |
| 14 | 4 | 2 | NOT-IMPLEMENTABLE, precision only, applied |
| 15 | 3 | 0 | NOT-IMPLEMENTABLE, precision only, applied |

**Closure (owner decision, 2026-10-09):** committed as specified with no known open F finding; every
finding of rounds 1 to 15 is amended. The final amended file was not re-audited: successive auditors
kept finding 2 to 4 new output detail gaps per round. Any further gap found by `/sdlc-design` is
escalated to the owner, never decided in design.

**Amendments applied:** the scope cut; every round 1 to 5 amendment that survives the cut (session
write quota, conversion entry points, membership access, refusals of today kept, image line, table
layout, picture formats, id gaps, refused conversions, documentation write quota, artifacts not files).
Round 6: documentation write vs re-conversion (FR-NEW-010, 013), empty extraction (FR-NEW-001), built-in
captions (FR-NEW-017), repeated pictures (FR-NEW-001), paging markers (FR-NEW-003), FR-NEW-028 path
holding no file, FR-NEW-029 no converter, FR-NEW-017/018 retagged, concrete quota Givens, glossary.
Round 7: CSV bytes (FR-NEW-002) + E2E-112, `tiff` (FR-NEW-004) + E2E-113, report order (FR-NEW-015) +
E2E-114, page per format (FR-NEW-001) + E2E-115, validity token design input. Round 8: FR-NEW-030
report, FR-MOD-002, FR-NEW-031 refusal order, FR-NEW-032 refusal categories, marker scope
(FR-NEW-003), E2E-116 to 126, a citation fix. Round 9:
FR-NEW-033 converter output reading, FR-NEW-034 text limit, FR-NEW-035 change during conversion,
past end marker, project restore, E2E-116 fixed, E2E-127 to 137. Round 10: FR-NEW-035 outcome,
image line removal rule (FR-NEW-007), empty Markdown table (FR-NEW-006), version control diff rule
(FR-NEW-011), E2E-138 to 141, converter caption facts. Round 11: failures limited to what the product
detects (FR-NEW-015), cell normalization (FR-NEW-002), E2E-005 to 007, 114, 129 rewritten, E2E-142, 143. Round 12:
picture reference, separator line and code block rules (FR-NEW-033), E2E-144 to 147, citation fixes. Round 13: cell splitting (FR-NEW-033), picture target confined to
the converter's output (FR-NEW-015), CR handling (FR-NEW-002), marker not bound to a person
(FR-NEW-003), E2E-148 to 155. Round 14: conversion bytes include the conversion text (FR-NEW-013),
overlapping re-conversions (FR-NEW-010), blank Excel rows (FR-NEW-017), exact `Q1 sales` data, E2E-156
to 158, citation fixes. Round 15: FR-NEW-036 empty lines, FR-NEW-037 reference not kept, marker
scoped to the project (FR-NEW-003), E2E-159 to 167.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix

### 14.1 Current State evidence

| Claim (Section 2) | Evidence |
|---|---|
| Markdown sibling path `report.pdf` gives `report.md` | `crates/core/src/docs/extract.rs:163`, test `extract.rs:1148-1153` |
| Formats with a Markdown sibling | `crates/core/src/docs/extract.rs:44-47` |
| Converter called with stdout only, other outputs discarded | `crates/core/src/docs/service.rs:183-206`; `config.rs` default command `doc-convert --stdout {document}` (AGENTS.md) |
| Converter output folder (figures, images.md, tables PNG) | `~/projects/perso/docling-scripts/README.md:305-317`, `src/doc_convert/base.py:173-182`, `:330` |
| Built-in tables rendered as Markdown, 400 row cap | `crates/core/src/docs/extract.rs:49`, `:286-312` |
| Images: OCR only, null by default | `crates/core/src/docs/ocr.rs:19-44` |
| Re-conversion overwrites the sibling | `crates/core/src/core/fs_ops.rs:654-657`, `:750` |
| Text extraction writes the sibling, then charges | `crates/core/src/docs/extract.rs:221`, `crates/core/src/core/fs_ops.rs:1553-1555` |
| Convert action charges before writing | `crates/core/src/core/fs_ops.rs:766`, `:573-574` |
| Convert refusals of today | `crates/core/src/core/fs_ops.rs:719`, `:721-725` |
| Documentation write: source written first, documentation outcome in a successful result | `crates/core/src/core/fs_ops.rs:647-666` |
| Quota check is strict greater than | `crates/core/src/safety.rs:134` |
| Refusal when the sibling exists without overwrite | `crates/core/src/core/fs_ops.rs:762-763` |
| Copy charges the full size | `crates/core/src/core/fs_ops.rs:1187` |
| Session quota message | `crates/core/src/safety.rs:135-136` |
| No per file metadata | nodes schema in `crates/core/src/storage/meta.rs` (path, size, times, hash only); `RelationalMetaStore` at `meta.rs:195-198` |
| Enforced limit is the session write quota | `crates/core/src/safety.rs:130-136` |
| Project storage limit stored, not enforced, usage unreadable | `sdlc/specs/SPEC-0010-project-storage-quota/spec.md:129-134` |
| Access is membership only | `crates/core/src/state.rs:59-60`; `storage/admin.rs:44` stores a role never checked |
| Conversion entry points | `crates/core/src/core/fs_ops.rs:650-660` (write with documentation), `:679` (documentize), extraction companion `docs/extract.rs:163` |
| Converter accepted formats configurable | `crates/core/src/docs/service.rs:42-48`, `:65-71`, `:84` |
| Markdown cell escaping | `crates/core/src/docs/extract.rs:292` |
| SPEC-0006 and SPEC-0008 as built | `sdlc/specs/SPEC-0006-documents/spec.md:3`, `sdlc/specs/SPEC-0008-search-and-rag/spec.md:3` |
| Existing doc tests | `docs/extract.rs` 49, `docs/service.rs` 16, `docs/ocr.rs` 6, `docs/docx.rs` 23 (`#[test]` + `#[tokio::test]` counts) |

Test command: `cargo test --workspace` (`make test`); full stack `make test-e2e-full`.

### 14.2 Class sweep

Not applicable: not a security lot.

### 14.3 Design inputs

- The converter is invoked with `--stdout` in a per call temporary directory
  (`crates/core/src/docs/service.rs:183-206`); keeping artifacts needs its output folder. Signal:
  E2E-001.
- The converter has no artifact contract: figures are referenced by relative path in `document.md`
  (`docling-scripts/src/doc_convert/base.py:173-182`), tables have no id, no catalog, no CSV. A
  fallback reading today's Markdown is required; a converter change (CSV export, table catalog, id
  markers) is to be filed in `docling-scripts`, which has no `sdlc/backlog/`. Signal: E2E-034, E2E-043.
- The HTTP converter mode returns one Markdown string (`service.rs:286-293`); it needs a bundle shape.
  Constraint 3.3. Signal: E2E-001 in api mode.
- No per node metadata exists (`storage/meta.rs:195-198`); artifacts and the conversion text are new
  persisted state, scoped by `volume_id` (AGENTS.md conventions). Signal: conformance suite.
- Validity rule FR-NEW-011: the content hash is NOT a sufficient key; `put_file` keeps path, sha256
  and ctime on a same bytes rewrite (`crates/core/src/storage/meta.rs:927-950`), and a move away and
  back restores the same (path, sha256). Artifacts need a validity token renewed by every successful
  mutation of the node (put, move, copy target, delete, trash, restore, archive extraction, git pull,
  checkout, merge, revert), compared on read; stale rows are never served. Signal: E2E-091, E2E-097.
- Built-in extractor builds a cell grid before `md_table` (`extract.rs:286-312`): source of
  `CSV quality: exact`; the 400 row cap (`extract.rs:49`) must not apply to the kept table. Signal:
  E2E-047.
- Text extraction writes the sibling before charging (`extract.rs:221`, `fs_ops.rs:1555`); FR-MOD-001
  needs the charge moved before the write there. The convert action already charges first
  (`fs_ops.rs:573-574`). Signal: E2E-014, E2E-076.
- The documentation write embeds the documentation outcome in a successful result
  (`fs_ops.rs:651-666`); FR-NEW-014 keeps that shape with the quota message. Signal: E2E-012.
- Text extraction cache hit writes nothing (`fs_ops.rs:1547-1555`); FR-NEW-026 replacement happens
  only on a real rewrite. Signal: E2E-070, E2E-082.
- E2E-010 literal `.pdf` assumes extensions configured with a leading dot (`docs/service.rs:42-48`).
- Quota: only `charge_write` (`safety.rs:130-136`, strict `>`) is enforced; project quota enforcement
  (SPEC-0010 FR-NEW-009) is unbuilt and out of scope.
- The converter records no page per figure (`docling-scripts/src/doc_convert/base.py:165-185` stores
  only `figures/figure_N.png` and a self ref); FR-NEW-001 page needs a converter change or docling
  provenance. Signal: E2E-001.
- The converter re-encodes every figure as PNG (`base.py:168`); the `jpeg` and unsupported format
  cases (FR-NEW-004, FR-NEW-015) need it to keep the source format. Signal: E2E-008.
- The converter deduplicates identical pictures by hash (`base.py:172-178`); FR-NEW-001 keeps one image
  per placement. Signal: E2E-102.
- The built-in Word parser caps rows while parsing (`extract.rs:616`), not only when rendering; a kept
  Word table needs an uncapped grid. Signal: a Word variant of E2E-047.
- The built-in Excel parser stops at 400 rows while parsing (`extract.rs:912`) and drops all-blank rows
  (`extract.rs:909`); the CSV parser truncates at 400 (`extract.rs:1040`). Kept tables need uncapped
  grids for both. Signal: E2E-047 (Excel), E2E-147 (CSV).
- Built-in Excel captions come from the sheet heading (`extract.rs:805`). Signal: E2E-101.
- Empty extraction writes no sibling (`extract.rs:211-219`). Signal: E2E-100.
- `not a file: <path>` mirrors the extraction message (`extract.rs:183`). Signal: E2E-104.
- A documentation write with no converter is refused before the source is written
  (`fs_ops.rs:640-644`), as is one for a format the converter does not accept. Signal: E2E-106, E2E-110.
- Refusal categories map to `ERR_NOT_FOUND` and `ERR_INVALID_ARGUMENT` (`crates/core/src/errors.rs:16`,
  `:20`). Signal: E2E-120 to E2E-122.
- The report is added beside today's result keys (`fs_ops.rs:699-704`, `:658-666`, `extract.rs:225`).
  Signal: E2E-124, E2E-125, `tool_contract_golden_is_current`.
- Converter captions come from its image description (`docling-scripts/src/doc_convert/markdown.py:537`
  image catalog, `vlm.py:119`); the converter can describe tables (`base.py:456-459`), but this spec takes
  table captions only from the `Table <number>: <text>` line; a converter table description is ignored.
  Converter captions default to off (`base.py:86`): image captions are non empty only when the operator
  enables converter captions; Section 9 shared data assumes so. Signal: E2E-127, E2E-129, E2E-001 with
  the default command.
- Text extraction truncates at `max_chars` (`extract.rs:119`, `:203`). Signal: E2E-130.
- The convert action reads bytes then converts outside any lock (`fs_ops.rs:694`, `:766`); FR-NEW-035
  needs a validity token check at commit. Signal: E2E-133.
- The converter drops failed pictures (`docling-scripts/src/doc_convert/base.py:161-165`) and blanks
  failed captions (`vlm.py:129`) without any signal; an explicit caption failure marker in its output is
  a converter change to file in docling-scripts. Until then `caption unavailable` never appears with
  the real converter. Signal: E2E-129 run against the real `doc-convert`.
- Picture targets are resolved inside the converter's output folder only; a URL, absolute path or
  escaping relative path is never read (path traversal guard, FR-NEW-015). Signal: E2E-151 to E2E-153.
- `md_table` replaces only `\n` (`extract.rs:292`); CR handling is new. Signal: E2E-154.
- MCP and REST parity: new tools need `TOOL_CONTRACT.txt` and golden updates, plus REST routes.
  Signal: `tool_contract_golden_is_current`.
- Artifacts excluded from folder listings, `fs.glob`/`fs.grep`, trash listing, `fs.export_zip` and git
  tree building. Signal: E2E-019, E2E-022, E2E-023, E2E-098.
