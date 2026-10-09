> Id: SPEC-0006
> Nature: FEAT
> Status: as-built
> Area: documents
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Documents, Extraction and Conversion — Specification

## 1. Summary

This spec covers everything that turns bytes into text an LLM can read, and text back into documents a human can open:

1. **Built-in extraction** to Markdown for PDF, OOXML, HTML, CSV, text and images, with a companion `.md` cached beside the source.
2. **The external document service**, an optional third-party converter reachable as a CLI binary or an HTTP endpoint, sandboxed per call, driving `fs.documentize`, the `trigger_documentation_service` flag on `fs.write_bytes`, and the same flag on REST upload.
3. **Document generation**: Markdown to `.docx` in-process, plus pandoc-backed `doc.to_docx` and `doc.to_pptx`.
4. **The HTML editor**: a per-editor HTTP and WebSocket server giving bidirectional sync between a browser and a volume file.
5. **Code intelligence**: tree-sitter symbol extraction with a lexical fallback, the matcher behind `fs.find_definition` and `fs.find_references`.

The unifying idea is the **companion**: the external service writes its Markdown to exactly the path the built-in extractor looks at, so a converted document is served as a cache hit at no extra cost.

## 2. Current State

### 2.1 None

(No prior state section; this feature was present at the audited commit. See §2.3.)

### 2.2 None

### 2.3 Relevant Architecture

- **Extraction**: `crates/core/src/docs/extract.rs` (1751 lines), format tables near the top, `companion_md_path` derivation.
- **Document service**: `crates/core/src/docs/service.rs` (868 lines), the `DocService` trait, `DOC_SERVICE_EXTS`, eligibility, the factory, CLI and API implementations.
- **Engine orchestration**: `crates/core/src/core/fs_ops.rs`, functions `documentize` (:679), `ensure_documentable` (:714), `ensure_input_size` (:732), `write_companion` (:750), `extract_document` (:1525), `write_docx` (:1565).
- **Document generation**: `crates/core/src/docs/docx.rs` (626 lines), in-process Markdown-to-OOXML.
- **Symbols**: `crates/core/src/docs/symbols.rs` (679 lines), extension-to-language table, definition kinds, ten statically linked grammars, lexical fallback.
- **OCR**: `crates/core/src/docs/ocr.rs` (208 lines), the `OcrProvider` trait, `NullOcrProvider`, `MultimodalOcrProvider`, the factory.
- **MIME**: `crates/core/src/docs/mime.rs` (109 lines), a closed extension table.
- **Editor**: `crates/core/src/tools/editor.rs` (1169 lines), a per-editor axum server with a polled mtime watch.
- **Pandoc tools**: `crates/core/src/tools/doc.rs` (736 lines), conditional registration on pandoc's presence.
- **Document tools**: `crates/core/src/tools/document.rs`, the three `fs.*` document tools.

## 3. Scope

### 3.1 In Scope

- Built-in extraction: supported formats and their backends, the unsupported set, `max_chars` and `preview_chars`, the companion `.md` cache and its refresh.
- `companion_md_path` derivation.
- The `DocService` trait, its two implementations, eligibility gating, the size gate, and the CLI sandbox.
- The three `fs.*` document tools: `fs.extract_text`, `fs.write_docx`, `fs.documentize`.
- The `trigger_documentation_service` flag on `fs.write_bytes` and the ordering rules around it.
- `doc.to_docx` and `doc.to_pptx`, and their conditional registration on pandoc's presence.
- The three editor tools and their sync protocol.
- Symbol extraction: the language table, the ten tree-sitter grammars, the lexical fallback, definition kinds.
- MIME guessing by extension.
- OCR as a pluggable provider, null by default.

### 3.2 Out of Scope

- The safety contract, error vocabulary and storage seam.
- The filesystem engine and the other `fs.*` tools; only the symbol matcher and the document tools are covered here, not the two symbol tools' gates and result shapes.
- REST routing for the document endpoints.
- Search indexing of extracted text.
- Audio and video transcription: deliberately unsupported by the built-in extractor, though the external service accepts those extensions.
- Any guarantee about a third-party converter's output quality. The contract is bytes in, Markdown out.

## 4. Actors

| Actor | Description |
|---|---|
| **LLM agent** | Reads documents as Markdown. Cannot consume a PDF, so extraction is what makes stored documents usable at all. |
| **Project member** | Uploads documents and receives generated ones. Owns the session that is charged and audited for every companion written. |
| **Operator** | Decides whether the external service exists, in which mode, with which binary or endpoint, which extensions and which size cap. Also decides whether pandoc is installed. |
| **Document author** | A human opening the HTML editor to write a document or slides in a browser, saving back into the volume. |
| **Third-party converter** | An arbitrary binary or HTTP service. Untrusted by construction: it is sandboxed, capped and timed out. |

## 5. Usage Scenarios

### SC-001: Agent extracts text from a stored PDF

**Actor:** LLM agent
**Preconditions:** member of the project; `/report.pdf` exists.
**Flow:**
1. Agent calls `fs.extract_text` with defaults `max_chars` 200000, `preview_chars` 4000, `ocr` true, `refresh` false.
2. The extractor checks for a companion at `companion_md_path` and serves it when present.
3. Otherwise the PDF's text layer is extracted page by page through `pdf-extract`.
4. The Markdown is written as the companion beside the source.
5. A newly written companion is charged against the quota, recorded as read, and audited under the op `extract_text`.

**Postconditions:** the agent holds the Markdown and a preview; `/report.md` exists; a second call is a cache hit costing no quota.
**Exceptions:**
- an audio or video file → `ERR_NOT_SUPPORTED`, because transcription needs a speech model and belongs outside a filesystem server
- a legacy binary Office file (`.doc`, `.xls`, `.ppt`) → falls through to the text decoder with a note rather than failing
- an image with OCR disabled or the null provider configured → no text
- `refresh` true → the companion is regenerated and charged again

### SC-002: Operator plugs in an external converter and an agent documentizes a file

**Actor:** Operator, then LLM agent
**Preconditions:** `doc_service` configured in `cli` or `api` mode.
**Flow:**
1. Operator configures the mode, the command or endpoint, the accepted extensions and the size cap.
2. The service is built once at boot so an HTTP client and its pool are reused.
3. Agent calls `fs.documentize` on `/deck.pptx`.
4. The engine checks the file is a file, then eligibility before reading, then the size cap after reading.
5. The converter runs; its Markdown is written to the companion path, charged and audited under its own op.

**Postconditions:** `/deck.md` exists and is a cache hit for `fs.extract_text`; the source is unchanged.
**Exceptions:**
- no service configured → `ERR_NOT_SUPPORTED`, `"document service is not configured"`
- an ineligible extension → `ERR_NOT_SUPPORTED` naming the accepted set
- input over the cap → `ERR_INVALID_ARGUMENT` naming the size and the limit
- the path is a directory → `ERR_NOT_FOUND`, `"not a file: {path}"`
- the converter fails → the error carries the exit code and only the stderr tail
- the converter exceeds its timeout → the child is killed and reaped before the sandbox directory is removed

### SC-003: Member uploads a document and asks for conversion in the same call

**Actor:** Project member
**Preconditions:** service configured; the caller is a member.
**Flow:**
1. Member calls `fs.write_bytes` with `trigger_documentation_service` true, or POSTs the REST upload with the same flag.
2. Every precondition of the conversion is checked before the first byte is written, so a flag set against an ineligible file stores nothing.
3. The source is committed.
4. The conversion runs; on success the companion is written.

**Postconditions:** the source is stored; the companion exists when conversion succeeded, and `documentation` is null when it was not requested.
**Exceptions:**
- the flag set against an ineligible file → the call fails and nothing is written
- conversion fails after the source is committed → the source stands and the call succeeds without a companion
- a multi-file upload where one file is ineligible → every file is validated before the first is written

### SC-004: Agent generates a Word document

**Actor:** LLM agent
**Preconditions:** member of the project.
**Flow:**
1. Agent calls `fs.write_docx` with Markdown, an optional title, and `overwrite` false by default.
2. The Markdown is rendered to OOXML in-process.
3. The result is written through the engine's write path, charged and audited.
4. For richer output the agent instead calls `doc.to_docx` or `doc.to_pptx`, which shell out to pandoc and accept a `.docx` template for styles, headers and footers.

**Postconditions:** the document exists in the volume and opens in Word or PowerPoint.
**Exceptions:**
- the target exists without `overwrite` → `ERR_NO_CLOBBER`
- pandoc absent from `PATH` → `doc.to_docx` and `doc.to_pptx` are not registered at all, with a warning logged
- numbered list markers are preserved in generated docx, which is a deliberate behaviour

### SC-005: Author edits a document in a browser

**Actor:** Document author
**Preconditions:** member of the project; `doc.enabled` true.
**Flow:**
1. Author calls `doc.open_editor` with a path and a mode, `doc` or `slides`.
2. A lightweight axum server starts on an OS-assigned port, creating the file if absent, and the browser is opened.
3. `GET /` returns the whole editor shell inline; `GET /ws` upgrades to a WebSocket.
4. Browser edits arrive as a `save` message and are written to the volume; a task polls the file's mtime every 500 ms and broadcasts `reload` on external change.
5. Author calls `doc.close_editor`, or lists active editors with `doc.list_editors`.

**Postconditions:** the file holds the browser's content; the port is released on close.
**Exceptions:**
- opening the same path twice → the existing editor is returned, idempotently
- the browser command unavailable → the editor still runs and its URL is returned; the command is skipped during tests
- an external write between polls → the browser reloads within one poll cycle

### SC-006: Agent locates a symbol in source code

**Actor:** LLM agent
**Preconditions:** member; the volume holds source files.
**Flow:**
1. Agent calls `fs.find_definition`, which walks the volume and calls this layer's matcher per file.
2. The file's language is resolved from its extension.
3. When a tree-sitter grammar exists for that language it is used; otherwise the lexical fallback runs.
4. Definitions are filtered by kind when a `kind` was given.

**Postconditions:** the agent holds definitions with a path, name, kind and line, or references with a path, line and kind.
**Exceptions:**
- an unknown extension → the file is skipped, not an error
- a file whose grammar fails to parse → the lexical fallback still produces results
- no match anywhere → an empty array

## 6. Functional Requirements

### Extraction

#### FR-001 [EARS-E]: Format support
> WHEN a document is extracted THE extractor SHALL select a backend by extension: `pdf-extract` for PDF, a `zip` plus `quick-xml` scan for DOCX, PPTX and XLSX, a tag stripper for HTML, an RFC 4180 parser for CSV, direct decoding for text formats, and the configured OCR provider for images.
- **Business Rules:** text extensions are `.txt`, `.md`, `.markdown`, `.rst`, `.log`, `.text`; fenced extensions, wrapped in a code block tagged with the extension, are `.json`, `.yaml`, `.yml`, `.xml`, `.toml`, `.ini`, `.env`; image extensions are `.png`, `.jpg`, `.jpeg`, `.gif`, `.bmp`, `.tif`, `.tiff`, `.webp`.
- **Priority:** Must-have

#### FR-002 [EARS-O]: Unsupported formats
> IF a document's extension is an audio or video format THEN the extractor SHALL refuse with `ERR_NOT_SUPPORTED`.
- **Business Rules:** the audio and video set is `.mp3`, `.wav`, `.m4a`, `.ogg`, `.flac`, `.aac`, `.mp4`, `.mkv`, `.mov`, `.avi`, `.webm`, `.wmv`. Transcription needs a speech model and belongs outside a filesystem server. The code is `ERR_NOT_SUPPORTED` rather than `ERR_INVALID_ARGUMENT`. Legacy binary Office formats are not in this set: they fall through to the text decoder with a note.
- **Priority:** Must-have

#### FR-003 [EARS-U]: Companion path derivation
> The companion Markdown path SHALL be the source path with its final extension replaced by `.md`.
- **Business Rules:** the final `.` is honoured only when it follows the final `/`, so a dot in a directory name does not truncate the path; a path with no extension gets `.md` appended to the whole path. `report.pdf` yields `report.md`.
- **Priority:** Must-have

#### FR-004 [EARS-E]: Companion caching
> WHEN a companion already exists and `refresh` is false THE extractor SHALL serve it rather than re-extracting.
- **Business Rules:** a cache hit is not charged against the quota and is not audited; only a newly written companion is. `refresh` true forces regeneration and is charged.
- **Priority:** Must-have

#### FR-005 [EARS-E]: Extraction accounting
> WHEN a companion is newly written THE engine SHALL charge its size against the session quota, record a read for it, and audit it under the op `extract_text`.
- **Business Rules:** the audit detail is `{bytes} bytes`. Recording the read means the agent can immediately edit the companion without a separate read.
- **Priority:** Must-have

#### FR-006 [EARS-E]: Extraction output caps
> WHEN text is extracted THE extractor SHALL bound the returned content by `max_chars` and the preview by `preview_chars`.
- **Business Rules:** `max_chars` default 200000, `preview_chars` default 4000. The caps bound what reaches an LLM context, so they are contract rather than tuning.
- **Priority:** Must-have

### The external document service

#### FR-007 [EARS-U]: The service is optional and built once
> The document service SHALL be off by default, and WHEN enabled it SHALL be constructed once at boot.
- **Business Rules:** building once means the HTTP client and its connection pool are reused across conversions, unlike the OCR provider which is rebuilt per call. An unknown mode is rejected both at boot validation and at construction.
- **Priority:** Must-have

#### FR-008 [EARS-U]: Two interchangeable modes
> The service SHALL be either a CLI binary reading a file and answering on stdout (CLI mode), or an HTTP endpoint taking `multipart/form-data` (API mode), and both SHALL be stateless.
- **Business Rules:** the trait is `bytes in, Markdown out`, with no session and no state between calls.
- **Priority:** Must-have

#### FR-009 [EARS-O]: Extension eligibility
> IF `doc_service.extensions` is empty THEN eligibility SHALL be decided against the built-in set; otherwise it SHALL be decided against the configured list, case-insensitively.
- **Business Rules:** the built-in set `DOC_SERVICE_EXTS` covers PowerPoint, Word, PDF, audio and video; it accepts audio and video, which the built-in extractor refuses under FR-002. The accepted set is resolved once at construction and is never empty, and it lives on the trait because the engine holds a `&dyn DocService` and must name the accepted set in its error.
- **Priority:** Must-have

#### FR-010 [EARS-O]: Input size gate
> IF an input exceeds `doc_service.max_input_bytes` THEN the engine SHALL refuse with `ERR_INVALID_ARGUMENT` naming the actual size and the limit.
- **Business Rules:** the gate is separate from the eligibility gate only because `documentize` learns the size after reading while an upload knows it up front.
- **Priority:** Must-have

#### FR-011 [EARS-U]: The CLI converter runs in a per-call sandbox
> Each CLI conversion SHALL run in a fresh temporary directory, with the input written inside it under a sanitized single-segment name, the child's working directory set to it, a relative path handed to the child, and `TMPDIR`, `TMP` and `TEMP` pointed at it.
- **Business Rules:** the command is an argv list, never a shell string. Only stdout is read; stderr is captured, capped at 8 KiB keeping the tail, and surfaced only on failure. On timeout the child is killed and reaped before the directory is removed. This is not a hard OS sandbox: a converter writing to an absolute path or to `$HOME` escapes it; real containment is the operator's call through argv[0].
- **Priority:** Must-have

#### FR-012 [EARS-E]: Failure reporting
> WHEN a conversion fails THE service SHALL report the failure with the exit code and the captured stderr tail, or for the HTTP mode the status and the first 2 KiB of the body.
- **Business Rules:** `STDERR_CAP` is 8 KiB and keeps the tail; `BODY_CAP` is 2 KiB and keeps the head.
- **Priority:** Must-have

#### FR-013 [EARS-E]: The companion is the shared artifact
> WHEN the service produces Markdown THE engine SHALL write it to the same companion path the built-in extractor reads.
- **Business Rules:** writing to that exact path makes a service companion a cache hit for `fs.extract_text` at no extra cost. It is charged and audited like any write, under its own op so the audit log distinguishes it from a plain write.
- **Priority:** Must-have

#### FR-014 [EARS-E]: Precondition ordering around a triggered write
> WHEN `trigger_documentation_service` is set THE engine SHALL check every conversion precondition before writing the first byte, and SHALL NOT roll the source back if conversion fails afterwards.
- **Business Rules:** before the write, a flag set against an ineligible file stores nothing. After the source is committed, a conversion failure neither rolls it back nor fails the call, because deleting a user's just-uploaded file because a third-party converter crashed is worse than returning it without its companion, and `fs.documentize` is the retry surface.
- **Priority:** Must-have

#### FR-015 [EARS-E]: Batch validation before any write
> WHEN several files are uploaded with the flag set THE engine SHALL validate every file before writing the first.
- **Business Rules:** `ensure_documentable` is public precisely so this rule lives in the engine rather than as a second copy in the REST layer.
- **Priority:** Must-have

#### FR-016 [EARS-E]: Documentize preconditions
> WHEN `fs.documentize` is called THE engine SHALL require the path to be a file, then check eligibility before reading, then the size gate after reading.
- **Business Rules:** eligibility precedes the read so a gigabyte the service goes on to refuse is never pulled. A non-file yields `ERR_NOT_FOUND` with `"not a file: {path}"`. The source is recorded as read on success.
- **Priority:** Must-have

### Generation

#### FR-017 [EARS-E]: In-process docx rendering
> WHEN `fs.write_docx` is called THE engine SHALL render the Markdown to a `.docx` in-process and write it through the shared write path.
- **Business Rules:** numbered list markers are preserved in the generated document. Writing through the engine means the quota and audit apply as for any write.
- **Priority:** Must-have

#### FR-018 [EARS-O]: Pandoc tools register only when pandoc exists
> IF pandoc is absent from `PATH` and from the configured bin path THEN `doc.to_docx` and `doc.to_pptx` SHALL NOT be registered.
- **Business Rules:** registration is a silent no-op with a warning logged so the operator knows why the tools are absent. The `doc.*` editor tools are registered regardless.
- **Priority:** Must-have

#### FR-019 [EARS-E]: Pandoc conversion accepts a template
> WHEN `doc.to_docx` or `doc.to_pptx` is called THE server SHALL convert the named Markdown or HTML file in the volume, applying a `.docx` template when `template_path` is given.
- **Business Rules:** the template supplies custom styles, headers and footers. Conversion runs with a timeout.
- **Priority:** Must-have

### Editing

#### FR-020 [EARS-E]: Opening an editor
> WHEN `doc.open_editor` is called THE server SHALL start an HTTP and WebSocket server on an OS-assigned port, create the file when absent, open the browser, and return an `editor_id` with its URL.
- **Business Rules:** `GET /` returns the complete editor shell with HTML, CSS and JavaScript inline. Opening the same path a second time returns the existing editor, idempotently. The browser command is skipped during tests.
- **Priority:** Must-have

#### FR-021 [EARS-E]: Bidirectional synchronization
> WHEN the browser saves THE server SHALL write the content to the volume, and WHEN the volume file changes externally THE server SHALL broadcast a reload to every connected client.
- **Business Rules:** the mtime is polled every 500 ms, so an external change surfaces within one poll cycle. The client reconnects automatically after a disconnect.
- **Priority:** Must-have

#### FR-022 [EARS-E]: Editor lifecycle
> WHEN `doc.close_editor` is called THE server SHALL stop that editor and release its port, and `doc.list_editors` SHALL report the active editors.
- **Business Rules:** an editor lives until closed or until the server shuts down.
- **Priority:** Must-have

### Code intelligence

#### FR-023 [EARS-E]: Language resolution
> WHEN a file is examined for symbols THE matcher SHALL resolve its language from its extension, and SHALL skip the file when no language is known.
- **Business Rules:** an unknown extension is skipped silently rather than treated as an error, which is what makes a mixed-content volume searchable at all.
- **Priority:** Must-have

#### FR-024 [EARS-O]: Grammar first, lexical fallback second
> IF a tree-sitter grammar exists for the resolved language THEN the matcher SHALL use it; otherwise it SHALL use the lexical fallback.
- **Business Rules:** ten grammars are linked statically: Python, JavaScript, TypeScript, TSX, Go, Rust, Java, C, C++ and Ruby. Static linking means there is no grammar loading path and no runtime dependency. The lexical fallback matches an identifier pattern and is what keeps results flowing for the other languages in the extension table.
- **Priority:** Must-have

#### FR-025 [EARS-E]: Definition kinds
> WHEN definitions are requested with a `kind` filter THE matcher SHALL return only definitions of that kind.
- **Business Rules:** kinds are per-language node names such as `function_definition` for Python and `function_item` for Rust.
- **Priority:** Must-have

### Supporting engines

#### FR-026 [EARS-U]: OCR is pluggable and null by default
> The extractor SHALL obtain image text from a configured `OcrProvider`, defaulting to a provider that returns nothing.
- **Business Rules:** the default is a no-op provider, so the build carries no native Tesseract dependency and stays a single static binary. A `multimodal` provider with an endpoint is enabled; a `tesseract` provider is not enabled even when an endpoint is configured. The provider is rebuilt per call, unlike the document service.
- **Priority:** Must-have

#### FR-027 [EARS-U]: MIME guessing is a closed extension table
> MIME types SHALL be guessed from the extension against a deliberately closed subset rather than a full MIME database.
- **Business Rules:** a smaller, predictable answer set is what the tool contract promises. This table holds 32 extensions and serves the REST download route. `fs.read_bytes` uses a separate table which the cross-spec audit confirmed holds the same 32 extensions with identical values. The module comment on this table claims it serves `fs.read_bytes`, which the code contradicts (open item, see Drift Register in design.md).
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- The companion cache means a repeated extraction costs one file read (FR-004).
- The document service is built once so its HTTP pool is reused (FR-007).
- Extraction output is capped by `max_chars` and `preview_chars` (FR-006).
- The editor polls at 500 ms, which bounds change-detection latency and the idle cost of an open editor (FR-021).

### 7.2 Security
- The CLI converter is untrusted and confined by construction, not by trust: fresh tempdir, relative input path, redirected `TMPDIR`, argv list, stdout only, capped stderr, killed and reaped on timeout (FR-011).
- The sandbox's limits are stated rather than overclaimed: a converter writing to an absolute path escapes it.
- Every companion write passes the same quota, ACL and audit rules as any other write (FR-005, FR-013).
- The editor serves its own port with content inline; it holds no credential.

### 7.3 Usability
- An ineligible file is refused with the accepted extension set in the message (FR-009).
- A missing pandoc removes the tools rather than offering a tool that always fails (FR-018).
- `fs.documentize` exists as the retry surface for a conversion that failed after a successful upload (FR-014).
- The stderr tail rather than the head is what an operator needs to diagnose a converter failure (FR-012).

### 7.4 Reliability
- A conversion failure never destroys a stored source (FR-014).
- A batch is validated before any file is written (FR-015).
- A timed-out child is killed and reaped before its directory is removed (FR-011).
- The lexical fallback keeps symbol search working where no grammar exists (FR-024).

### 7.5 Observability
A pandoc-absent registration logs a warning naming the reason (FR-018). A converter failure surfaces the exit code and the stderr tail in the error rather than only in a log the caller cannot see (FR-012). Conversion duration is not measured and no span is emitted (open item, see design.md).

### 7.6 Deployment
- `doc_service` is off by default; enabling it means providing a binary or an endpoint.
- `doc.to_docx` and `doc.to_pptx` need pandoc on the host.
- `scripts/doc_service_fake.py --port 8099` provides a fake service for testing api mode.
- The editor binds an OS-assigned port per open editor and opens a local browser, so it suits a workstation rather than a headless deployment.

### 7.7 Scalability
- Every conversion is a subprocess or an HTTP round trip with no pooling beyond the shared HTTP client, so throughput is bounded by the converter.
- Companion files double the stored object count for converted documents, though content addressing means identical companions share bytes.
- One editor is one server and one port; the count of simultaneous editors is bounded by available ports and is not capped in configuration (open item, see design.md).

## 8. End-to-End Tests

| Test ID | Action | Scenario | FR refs | Priority |
|---|---|---|---|---|
| E2E-001 | Extract PDF with text layer | SC-001 | FR-001, FR-003 | Critical |
| E2E-002 | Second extraction is a free cache hit | SC-001 | FR-004, FR-005 | Critical |
| E2E-003 | max_chars/preview_chars bound a text extraction | SC-001 | FR-001, FR-006 | High |
| E2E-004 | Audio/video refused with ERR_NOT_SUPPORTED | SC-001 | FR-002 | Critical |
| E2E-005 | Newly written companion is charged and readable | SC-001 | FR-003, FR-005 | Critical |
| E2E-006 | max_chars/preview_chars bound a 50000-char text file | SC-001 | FR-004, FR-006 | High |
| E2E-007 | Each format reaches its own backend (pdf/pptx/csv/json/txt) | SC-001 | FR-001, FR-002 | High |
| E2E-008 | mp3/mp4 refused; .doc falls through to text decoder | SC-001 | FR-002 | High |
| E2E-009 | companion_md_path handles dotted dirs and no-extension paths | SC-001 | FR-003, FR-006 | High |
| E2E-010 | refresh forces regeneration and is charged again | SC-001 | FR-004, FR-005 | High |
| E2E-011 | documentize with a configured service, both modes stateless | SC-002 | FR-007, FR-008 | Critical |
| E2E-012 | Eligible extension documentizes successfully | SC-002 | FR-009, FR-016 | Critical |
| E2E-013 | Unconfigured service refuses with the exact message | SC-002 | FR-007, FR-010 | Critical |
| E2E-014 | CLI sandbox isolation (fresh tempdir, argv list) | SC-002 | FR-008, FR-011 | Critical |
| E2E-015 | Service failure surfaces exit code + stderr tail / status + body head | SC-002 | FR-009, FR-012 | High |
| E2E-016 | Size gate names size and limit | SC-002 | FR-010, FR-016 | High |
| E2E-017 | Timeout kills and reaps before tempdir removal | SC-002 | FR-011, FR-012 | Critical |
| E2E-018 | Service constructed once; connection reuse observed | SC-002 | FR-007, FR-010 | Medium |
| E2E-019 | CLI and API modes satisfy the same contract | SC-002 | FR-008, FR-012 | High |
| E2E-020 | Companion is a cache hit for extract_text after documentize | SC-002 | FR-011, FR-016 | Critical |
| E2E-021 | Configured extension list overrides the built-in set | SC-002 | FR-009 | High |
| E2E-022 | Triggered upload produces source + companion | SC-003 | FR-013, FR-014 | Critical |
| E2E-023 | Companion written under its own audit op, not `write` | SC-003 | FR-013 | High |
| E2E-024 | Ineligible file with flag set writes nothing | SC-003 | FR-014, FR-015 | Critical |
| E2E-025 | Conversion failure after write keeps the source; documentize retries | SC-003 | FR-014 | Critical |
| E2E-026 | Batch upload validates every file before writing any | SC-003 | FR-013, FR-015 | High |
| E2E-027 | Flag left false writes source and no companion, documentation null | SC-003 | FR-014, FR-015 | High |
| E2E-028 | write_docx renders Markdown in-process | SC-004 | FR-017 | Critical |
| E2E-029 | doc.to_docx/to_pptx convert with pandoc | SC-004 | FR-018, FR-019 | High |
| E2E-030 | write_docx no-clobber and edit-without-prior-read rules | SC-004 | FR-017, FR-019 | High |
| E2E-031 | Pandoc absent removes doc.to_docx/to_pptx from the catalogue | SC-004 | FR-018 | High |
| E2E-032 | Generated docx preserves numbered list markers | SC-004 | FR-017 | Medium |
| E2E-033 | Template applies styles to pandoc output | SC-004 | FR-018, FR-019 | Medium |
| E2E-034 | Opening an editor creates the file and returns editor_id/URL | SC-005 | FR-020, FR-022 | Critical |
| E2E-035 | Browser save reaches the volume | SC-005 | FR-021 | Critical |
| E2E-036 | Closing releases the port; reopening is idempotent | SC-005 | FR-020, FR-022 | High |
| E2E-037 | External change reaches the browser within a poll cycle | SC-005 | FR-021 | High |
| E2E-038 | Opening the same path twice returns the same editor | SC-005 | FR-020 | High |
| E2E-039 | WebSocket save message reaches the volume; editor stays listed | SC-005 | FR-021, FR-022 | Medium |
| E2E-040 | find_definition resolves language and returns kind/line | SC-006 | FR-023, FR-025 | Critical |
| E2E-041 | Grammar used when available | SC-006 | FR-024 | Critical |
| E2E-042 | Unknown extension skipped silently | SC-006 | FR-023, FR-026 | High |
| E2E-043 | Lexical fallback runs where no grammar exists | SC-006 | FR-024, FR-027 | High |
| E2E-044 | kind filter narrows the result | SC-006 | FR-025 | Medium |
| E2E-045 | Ten grammars resolve by extension | SC-006 | FR-023, FR-026 | High |
| E2E-046 | OCR null by default; tesseract not enabled; multimodal enabled | SC-006 | FR-024, FR-027 | High |
| E2E-047 | No match anywhere returns empty array | SC-006 | FR-025 | High |
| E2E-048 | MIME guessing is a closed table; two tables agree | SC-006 | FR-026, FR-027 | Medium |

**Fixtures:** project `spec-docs`; `/report.pdf` with a text layer containing `Quarterly results`; `/deck.pptx` with one slide titled `Roadmap`; `/data.csv` with a header and two rows; `/notes.txt`; `/photo.png`; `/song.mp3`; `/src/app.py` defining `hello`; `/src/lib.rs` defining `hello`. The fake service is `scripts/doc_service_fake.py --port 8099`.

## 9. Glossary

| Term | Definition | Context |
|---|---|---|
| **Companion** | The `.md` file written beside a source document, at the path both the extractor and the service use. | Extraction |
| **Extraction** | Turning stored bytes into Markdown in-process. | Extraction |
| **Document service** | The optional external converter, in CLI or HTTP mode. | Conversion |
| **CLI mode** | A converter invoked as a binary reading a file and answering on stdout. | Conversion |
| **API mode** | A converter invoked as an HTTP endpoint taking multipart form data. | Conversion |
| **Eligibility** | The extension gate deciding whether the service accepts a path. | Conversion |
| **Sandbox** | The per-call temporary directory and environment a CLI converter runs in. | Conversion |
| **Size gate** | The `max_input_bytes` refusal applied before conversion. | Conversion |
| **Cache hit** | An extraction served from an existing companion, costing no quota. | Extraction |
| **OCR provider** | The pluggable image-to-text backend, null by default. | Extraction |
| **Grammar** | A statically linked tree-sitter parser for one language. | Code intelligence |
| **Lexical fallback** | The pattern-based matcher used where no grammar exists. | Code intelligence |
| **Definition kind** | The per-language node name classifying a definition. | Code intelligence |
| **Editor** | One browser editing session: a server, a port, a WebSocket and a polled file. | Editing |
| **Poll cycle** | The 500 ms interval at which an editor checks its file's mtime. | Editing |

## 10. Confidence Notes

Confidence: **high**. Spot-checked against the current tree at base commit `27bc282acdfd2a5c972802e60ed0e7b566e37ab5`:
- `crates/core/src/docs/extract.rs` format tables and extension lists match the original spec's citations verbatim (text/fenced/image/AV extension sets confirmed).
- `fs_ops.rs` function names (`documentize`, `ensure_documentable`, `ensure_input_size`, `write_companion`, `extract_document`, `write_docx`) and the `"document service is not configured"` message confirmed present.
- Line numbers have drifted from the original pre-move spec (e.g. `write_docx` now at `:1565` vs the pre-move `:1459`, `extract_document` at `:1525` vs `:1420`); the pre-move spec's citations used `crates/mcp-fs/src/...` paths, the current tree uses `crates/core/src/...` (the crate was renamed/restructured since). Functional content is unaffected; design.md's legacy mapping records the path change.
- `tools/editor.rs` currently counts 24 `#[test]`/`#[tokio::test]` functions, not the 25 recorded in the original spec text (which itself corrected an older `BACKLOG.md` figure of 17). The count has drifted again since; treated as a minor, non-blocking drift and logged for backlog in design.md.
- `doc.rs` conditional pandoc-registration comment and behaviour confirmed present.
