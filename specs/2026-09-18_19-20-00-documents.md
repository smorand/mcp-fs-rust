# mcp-fs Documents, Extraction and Conversion — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

**ID convention.** S5 of eight. S5 owns the `5xx` block: `SC-5xx`, `FR-5xx`, `E2E-5xx`, `DEC-5xx`, `EXC-5xx`.

## 1. Executive Summary

This document specifies everything that turns bytes into text an LLM can read, and text back into documents a human can open. Five capabilities:

1. **Built-in extraction** to Markdown for PDF, OOXML, HTML, CSV, text and images, with a companion `.md` cached beside the source.
2. **The external document service**, an optional third-party converter reachable as a CLI binary or an HTTP endpoint, sandboxed per call, driving `fs.documentize`, the `trigger_documentation_service` flag on `fs.write_bytes`, and the same flag on REST upload.
3. **Document generation**: Markdown to `.docx` in-process, plus pandoc-backed `doc.to_docx` and `doc.to_pptx`.
4. **The HTML editor**: a per-editor HTTP and WebSocket server giving bidirectional sync between a browser and a volume file.
5. **Code intelligence**: tree-sitter symbol extraction with a lexical fallback, the matcher behind S2's `fs.find_definition` and `fs.find_references`.

The unifying idea is the **companion**: the external service writes its Markdown to exactly the path the built-in extractor looks at, so a converted document is served as a cache hit at no extra cost.

Retro-specification of shipped behaviour; every claim carries a `file:LINE` citation.

## 2. Current State Analysis

### 2.1 Project Overview

`docs/` is 4267 lines: `extract.rs` (1751), `service.rs` (868), `symbols.rs` (679), `docx.rs` (626), `ocr.rs` (208), `mime.rs` (109), `mod.rs` (26). The tool surface is `tools/document.rs` (3 `fs.*` tools), `tools/doc.rs` (2 pandoc tools, conditional) and `tools/editor.rs` (3 editor tools).

### 2.2 Existing Specifications

- **S1**: the safety contract this layer charges and audits through, the error vocabulary, the storage seam.
- **S2**: the engine. S2 §3.2 deferred `fs.extract_text`, `fs.write_docx` and `fs.documentize` here, and deferred the symbol matcher behind `fs.find_definition` and `fs.find_references` here while keeping those two tools' contracts (S2 FR-226).
- **S3**: the REST plumbing for `extract-text`, `write-docx`, `documentize` and the upload documentation flag (S3 DEC-301, FR-312). S3 specified the flag's parsing rule; this spec specifies what the flag does.

### 2.3 Relevant Architecture

- **Extraction**: `docs/extract.rs`, with format tables at `:31-41` and `companion_md_path` at `:155-164`.
- **Document service**: `docs/service.rs`, the `DocService` trait (`:61-75`), `DOC_SERVICE_EXTS` (`:42-48`), eligibility (`:78-96`), the factory (`:100`), CLI (`:142`) and API (`:269`) implementations.
- **Engine orchestration**: `core/fs_ops.rs`, `documentize` (`:670-696`), `ensure_documentable` (`:704-720`), `ensure_input_size` (`:722-731`), `write_companion` (`:740`), `extract_document` (`:1420-1453`), `write_docx` (`:1459`).
- **Symbols**: `docs/symbols.rs`, extension-to-language table (`:39`), definition kinds (`:55`), ten grammars (`:102-111`), lexical fallback (`:264`).
- **OCR**: `docs/ocr.rs`, the trait (`:22`), `NullOcrProvider` (`:34`), `MultimodalOcrProvider` (`:51`), the factory (`:147`).
- **Editor**: `tools/editor.rs`, a 500 ms mtime poll (`:251`).

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
- MIME guessing by extension (`docs/mime.rs`).
- OCR as a pluggable provider, null by default.

### 3.2 Out of Scope (Non-Goals)

- The safety contract, error vocabulary and storage seam (S1).
- The filesystem engine and the other 32 `fs.*` tools (S2). The two symbol tools' gates and result shapes are S2 FR-226; only the matcher is here.
- REST routing for the document endpoints (S3).
- Search indexing of extracted text (S7).
- Audio and video transcription. It is deliberately unsupported by the built-in extractor, though the external service accepts those extensions.
- Any guarantee about a third-party converter's output quality. The contract is bytes in, Markdown out.

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **LLM agent** | Reads documents as Markdown. Cannot consume a PDF, so extraction is what makes stored documents usable at all. |
| **Project member** | Uploads documents and receives generated ones. Owns the session that is charged and audited for every companion written. |
| **Operator** | Decides whether the external service exists, in which mode, with which binary or endpoint, which extensions and which size cap. Also decides whether pandoc is installed. |
| **Document author** | A human opening the HTML editor to write a document or slides in a browser, saving back into the volume. |
| **Third-party converter** | An arbitrary binary or HTTP service. Untrusted by construction: it is sandboxed, capped and timed out. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Extraction** | Bytes to Markdown, in-process. | Extractor, ExtractResult, companion |
| **Conversion** | Bytes to Markdown through an external service. | DocService, accepted extensions, sandbox |
| **Generation** | Markdown to a document format. | docx renderer, pandoc invocation |
| **Editing** | The browser-side editing session. | editor_id, WebSocket, mtime poll |
| **Code intelligence** | Source text to symbols. | language, grammar, definition kind, reference |

A "document" in the Extraction context is a stored file being read; in the Generation context it is a file being produced. A "companion" is the `.md` written beside a source, and it is the single artifact both Extraction and Conversion produce.

## 5. Usage Scenarios

### SC-501: Agent extracts text from a stored PDF

**Actor:** LLM agent
**Preconditions:** member of the project; `/report.pdf` exists.
**Flow:**
1. Agent calls `fs.extract_text` with defaults `max_chars` 200000, `preview_chars` 4000, `ocr` true, `refresh` false.
2. The extractor checks for a companion at `companion_md_path` and serves it when present.
3. Otherwise the PDF's text layer is extracted page by page through `pdf-extract`.
4. The Markdown is written as the companion beside the source.
5. A newly written companion is charged against the quota, recorded as read, and audited under the op `extract_text` (`core/fs_ops.rs:1444-1450`).

**Postconditions:** the agent holds the Markdown and a preview; `/report.md` exists; a second call is a cache hit costing no quota.
**Exceptions:**
- EXC-501a: an audio or video file → `ERR_NOT_SUPPORTED`, because transcription needs a speech model and belongs outside a filesystem server (`docs/extract.rs:16-18`)
- EXC-501b: a legacy binary Office file (`.doc`, `.xls`, `.ppt`) → falls through to the text decoder with a note rather than failing
- EXC-501c: an image with OCR disabled or the null provider configured → no text
- EXC-501d: `refresh` true → the companion is regenerated and charged again

### SC-502: Operator plugs in an external converter and an agent documentizes a file

**Actor:** Operator, then LLM agent
**Preconditions:** `doc_service` configured in `cli` or `api` mode.
**Flow:**
1. Operator configures the mode, the command or endpoint, the accepted extensions and the size cap.
2. The service is built **once at boot** so an HTTP client and its pool are reused (`docs/service.rs:4-7`).
3. Agent calls `fs.documentize` on `/deck.pptx`.
4. The engine checks the file is a file, then eligibility **before** reading, then the size cap after reading (`core/fs_ops.rs:680-686`).
5. The converter runs; its Markdown is written to the companion path, charged and audited under its own op.

**Postconditions:** `/deck.md` exists and is a cache hit for `fs.extract_text`; the source is unchanged.
**Exceptions:**
- EXC-502a: no service configured → `ERR_NOT_SUPPORTED`, `"document service is not configured"` (`core/fs_ops.rs:712`)
- EXC-502b: an ineligible extension → `ERR_NOT_SUPPORTED` naming the accepted set (`core/fs_ops.rs:715-718`)
- EXC-502c: input over the cap → `ERR_INVALID_ARGUMENT` naming the size and the limit (`core/fs_ops.rs:725-729`)
- EXC-502d: the path is a directory → `ERR_NOT_FOUND`, `"not a file: {path}"` (`core/fs_ops.rs:681`)
- EXC-502e: the converter fails → the error carries the exit code and only the stderr tail
- EXC-502f: the converter exceeds its timeout → the child is killed and reaped before the sandbox directory is removed

### SC-503: Member uploads a document and asks for conversion in the same call

**Actor:** Project member
**Preconditions:** service configured; the caller is a member.
**Flow:**
1. Member calls `fs.write_bytes` with `trigger_documentation_service` true, or POSTs the REST upload with the same flag.
2. **Every precondition of the conversion is checked before the first byte is written**, so a flag set against an ineligible file stores nothing (`core/fs_ops.rs:624-627`).
3. The source is committed.
4. The conversion runs; on success the companion is written.

**Postconditions:** the source is stored; the companion exists when conversion succeeded, and `documentation` is null when it was not requested.
**Exceptions:**
- EXC-503a: the flag set against an ineligible file → the call fails and **nothing is written**
- EXC-503b: conversion fails after the source is committed → the source stands and the call succeeds without a companion, because deleting a just-uploaded file because a third-party converter crashed is worse than returning it without its companion (`core/fs_ops.rs:626-630`)
- EXC-503c: a multi-file upload where one file is ineligible → every file is validated before the first is written (`core/fs_ops.rs:706-708`)

### SC-504: Agent generates a Word document

**Actor:** LLM agent
**Preconditions:** member of the project.
**Flow:**
1. Agent calls `fs.write_docx` with Markdown, an optional title, and `overwrite` false by default.
2. The Markdown is rendered to OOXML in-process.
3. The result is written through the engine's write path, charged and audited.
4. For richer output the agent instead calls `doc.to_docx` or `doc.to_pptx`, which shell out to pandoc and accept a `.docx` template for styles, headers and footers.

**Postconditions:** the document exists in the volume and opens in Word or PowerPoint.
**Exceptions:**
- EXC-504a: the target exists without `overwrite` → `ERR_NO_CLOBBER`
- EXC-504b: pandoc absent from `PATH` → `doc.to_docx` and `doc.to_pptx` are **not registered at all**, with a warning logged so the operator knows why (`tools/doc.rs:3-5`)
- EXC-504c: numbered list markers are preserved in generated docx, which is a deliberate behaviour

### SC-505: Author edits a document in a browser

**Actor:** Document author
**Preconditions:** member of the project; `doc.enabled` true.
**Flow:**
1. Author calls `doc.open_editor` with a path and a mode, `doc` or `slides`.
2. A lightweight axum server starts on an OS-assigned port, creating the file if absent, and the browser is opened (`tools/editor.rs:1-18`).
3. `GET /` returns the whole editor shell inline; `GET /ws` upgrades to a WebSocket.
4. Browser edits arrive as a `save` message and are written to the volume; a task polls the file's mtime every 500 ms and broadcasts `reload` on external change (`tools/editor.rs:251`).
5. Author calls `doc.close_editor`, or lists active editors with `doc.list_editors`.

**Postconditions:** the file holds the browser's content; the port is released on close.
**Exceptions:**
- EXC-505a: opening the same path twice → the existing editor is returned, idempotently
- EXC-505b: the browser command unavailable → the editor still runs and its URL is returned; the command is skipped during tests
- EXC-505c: an external write between polls → the browser reloads within one poll cycle

### SC-506: Agent locates a symbol in source code

**Actor:** LLM agent
**Preconditions:** member; the volume holds source files.
**Flow:**
1. Agent calls `fs.find_definition` (S2 FR-226), which walks the volume and calls this layer's matcher per file.
2. The file's language is resolved from its extension (`docs/symbols.rs:89-99`).
3. When a tree-sitter grammar exists for that language it is used; otherwise the lexical fallback runs.
4. Definitions are filtered by kind when a `kind` was given.

**Postconditions:** the agent holds definitions with a path, name, kind and line, or references with a path, line and kind.
**Exceptions:**
- EXC-506a: an unknown extension → the file is skipped, not an error
- EXC-506b: a file whose grammar fails to parse → the lexical fallback still produces results
- EXC-506c: no match anywhere → an empty array

## 6. Functional Requirements

### Extraction

#### FR-501 [EARS-E]: Format support
> WHEN a document is extracted THE extractor SHALL select a backend by extension: `pdf-extract` for PDF, a `zip` plus `quick-xml` scan for DOCX, PPTX and XLSX, a tag stripper for HTML, an RFC 4180 parser for CSV, direct decoding for text formats, and the configured OCR provider for images.

- **Inputs:** the file bytes and its path.
- **Outputs:** Markdown plus metadata (`docs/extract.rs:4-14`).
- **Business Rules:** text extensions are `.txt`, `.md`, `.markdown`, `.rst`, `.log`, `.text`; fenced extensions, wrapped in a code block tagged with the extension, are `.json`, `.yaml`, `.yml`, `.xml`, `.toml`, `.ini`, `.env`; image extensions are `.png`, `.jpg`, `.jpeg`, `.gif`, `.bmp`, `.tif`, `.tiff`, `.webp` (`docs/extract.rs:31-37`).
- **Priority:** Must-have

#### FR-502 [EARS-O]: Unsupported formats
> IF a document's extension is an audio or video format THEN the extractor SHALL refuse with `ERR_NOT_SUPPORTED`.

- **Inputs:** the path extension.
- **Outputs:** the refusal (`docs/extract.rs:16-23,38-41`).
- **Business Rules:** the audio and video set is `.mp3`, `.wav`, `.m4a`, `.ogg`, `.flac`, `.aac`, `.mp4`, `.mkv`, `.mov`, `.avi`, `.webm`, `.wmv`. Transcription needs a speech model and belongs outside a filesystem server. The code is `ERR_NOT_SUPPORTED` rather than `ERR_INVALID_ARGUMENT`, a deliberate divergence from the original so the code says what actually happened; the message text is unchanged. Legacy binary Office formats are **not** in this set: they fall through to the text decoder with a note.
- **Priority:** Must-have

#### FR-503 [EARS-U]: Companion path derivation
> The companion Markdown path SHALL be the source path with its final extension replaced by `.md`.

- **Inputs:** the source path.
- **Outputs:** the companion path (`docs/extract.rs:155-164`).
- **Business Rules:** the final `.` is honoured only when it follows the final `/`, so a dot in a directory name does not truncate the path; a path with no extension gets `.md` appended to the whole path. `report.pdf` yields `report.md`.
- **Priority:** Must-have

#### FR-504 [EARS-E]: Companion caching
> WHEN a companion already exists and `refresh` is false THE extractor SHALL serve it rather than re-extracting.

- **Inputs:** `refresh`, default false.
- **Outputs:** the payload with `cached` true (`core/fs_ops.rs:1444`).
- **Business Rules:** a cache hit is not charged against the quota and is not audited; only a newly written companion is (`core/fs_ops.rs:1444-1450`). `refresh` true forces regeneration and is charged.
- **Priority:** Must-have

#### FR-505 [EARS-E]: Extraction accounting
> WHEN a companion is newly written THE engine SHALL charge its size against the session quota, record a read for it, and audit it under the op `extract_text`.

- **Inputs:** the written companion.
- **Outputs:** session state and an audit entry (`core/fs_ops.rs:1445-1449`).
- **Business Rules:** the audit detail is `{bytes} bytes`. Recording the read means the agent can immediately edit the companion without a separate read.
- **Priority:** Must-have

#### FR-506 [EARS-E]: Extraction output caps
> WHEN text is extracted THE extractor SHALL bound the returned content by `max_chars` and the preview by `preview_chars`.

- **Inputs:** `max_chars` default 200000, `preview_chars` default 4000.
- **Outputs:** the bounded payload.
- **Business Rules:** the caps bound what reaches an LLM context, so they are contract rather than tuning, exactly as the walk ceilings are in S2 FR-223.
- **Priority:** Must-have

### The external document service

#### FR-507 [EARS-U]: The service is optional and built once
> The document service SHALL be off by default, and WHEN enabled it SHALL be constructed once at boot.

- **Inputs:** the `doc_service` config block.
- **Outputs:** `Option<Arc<dyn DocService>>` (`docs/service.rs:100`, `app.rs:89-91`).
- **Business Rules:** building once means the HTTP client and its connection pool are reused across conversions, unlike the OCR provider which is rebuilt per call (`docs/service.rs:4-7`). An unknown mode is rejected both at boot validation and at construction, because a service can be built from a config that never went through `validate` (`docs/service.rs:96-99`).
- **Priority:** Must-have

#### FR-508 [EARS-U]: Two interchangeable modes
> The service SHALL be either a CLI binary reading a file and answering on stdout (CLI mode), or an HTTP endpoint taking `multipart/form-data` (API mode), and both SHALL be stateless.

- **Inputs:** the configured mode.
- **Outputs:** Markdown (`docs/service.rs:61-64,142,269`).
- **Business Rules:** the trait is `bytes in, Markdown out`, with no session and no state between calls.
- **Priority:** Must-have

#### FR-509 [EARS-O]: Extension eligibility
> IF `doc_service.extensions` is empty THEN eligibility SHALL be decided against the built-in set; otherwise it SHALL be decided against the configured list, case-insensitively.

- **Inputs:** the path and the configured extension list.
- **Outputs:** the eligibility decision (`docs/service.rs:78-96`).
- **Business Rules:** the built-in set `DOC_SERVICE_EXTS` covers PowerPoint, Word, PDF, audio and video (`docs/service.rs:42-48`); note it accepts audio and video, which the **built-in** extractor refuses under FR-502. The accepted set is resolved once at construction and is never empty, and it lives on the trait because the engine holds a `&dyn DocService` and no config yet must name the accepted set in its error (`docs/service.rs:66-75`).
- **Priority:** Must-have

#### FR-510 [EARS-O]: Input size gate
> IF an input exceeds `doc_service.max_input_bytes` THEN the engine SHALL refuse with `ERR_INVALID_ARGUMENT` naming the actual size and the limit.

- **Inputs:** the byte length.
- **Outputs:** the refusal (`core/fs_ops.rs:722-731`).
- **Business Rules:** the gate is separate from the eligibility gate only because `documentize` learns the size after reading while an upload knows it up front.
- **Priority:** Must-have

#### FR-511 [EARS-U]: The CLI converter runs in a per-call sandbox
> Each CLI conversion SHALL run in a fresh temporary directory, with the input written inside it under a sanitized single-segment name, the child's working directory set to it, a relative path handed to the child, and `TMPDIR`, `TMP` and `TEMP` pointed at it.

- **Inputs:** the bytes and the file name.
- **Outputs:** the child's stdout (`docs/service.rs:9-26`).
- **Business Rules:** the command is an argv list, never a shell string. Only stdout is read; stderr is captured, capped at 8 KiB keeping the **tail**, and surfaced only on failure, because `doc-convert` writes megabytes of progress bars and the tail holds the real cause (`docs/service.rs:50-53`). On timeout the child is killed and reaped **before** the directory is removed, so no process is left writing into a directory being deleted. **This is not a hard OS sandbox**: a converter writing to an absolute path or to `$HOME` escapes it; real containment is the operator's call through argv[0], which is why the command is a list.
- **Priority:** Must-have

#### FR-512 [EARS-E]: Failure reporting
> WHEN a conversion fails THE service SHALL report the failure with the exit code and the captured stderr tail, or for the HTTP mode the status and the first 2 KiB of the body.

- **Inputs:** the child's exit status or the HTTP response.
- **Outputs:** the error (`docs/service.rs:50-58`).
- **Business Rules:** `STDERR_CAP` is 8 KiB and keeps the tail; `BODY_CAP` is 2 KiB and keeps the head, because an error page says what went wrong in its first line.
- **Priority:** Must-have

#### FR-513 [EARS-E]: The companion is the shared artifact
> WHEN the service produces Markdown THE engine SHALL write it to the same companion path the built-in extractor reads.

- **Inputs:** the converted Markdown.
- **Outputs:** the companion file (`core/fs_ops.rs:735-739`).
- **Business Rules:** writing to that exact path makes a service companion a cache hit for `fs.extract_text` at no extra cost. It is charged and audited like any write, under its own op so the audit log distinguishes it from a plain write.
- **Priority:** Must-have

#### FR-514 [EARS-E]: Precondition ordering around a triggered write
> WHEN `trigger_documentation_service` is set THE engine SHALL check every conversion precondition before writing the first byte, and SHALL NOT roll the source back if conversion fails afterwards.

- **Inputs:** the flag, the path, the byte length.
- **Outputs:** the source, and a companion when conversion succeeded (`core/fs_ops.rs:619-633`).
- **Business Rules:** the two halves are deliberate opposites. Before the write, a flag set against an ineligible file stores nothing. After the source is committed, a conversion failure neither rolls it back nor fails the call, because deleting a user's just-uploaded file because a third-party converter crashed is worse than returning it without its companion, and `fs.documentize` is the retry surface.
- **Priority:** Must-have

#### FR-515 [EARS-E]: Batch validation before any write
> WHEN several files are uploaded with the flag set THE engine SHALL validate every file before writing the first.

- **Inputs:** the batch.
- **Outputs:** all files written, or none (`core/fs_ops.rs:704-708`).
- **Business Rules:** `ensure_documentable` is public precisely so this rule lives in the engine rather than as a second copy in the REST layer.
- **Priority:** Must-have

#### FR-516 [EARS-E]: Documentize preconditions
> WHEN `fs.documentize` is called THE engine SHALL require the path to be a file, then check eligibility before reading, then the size gate after reading.

- **Inputs:** `path`, `overwrite` default false.
- **Outputs:** `{path, md_path, bytes_written, overwritten}` (`core/fs_ops.rs:690-695`).
- **Business Rules:** eligibility precedes the read so a gigabyte the service goes on to refuse is never pulled (`core/fs_ops.rs:682-683`). A non-file yields `ERR_NOT_FOUND` with `"not a file: {path}"`. The source is recorded as read on success.
- **Priority:** Must-have

### Generation

#### FR-517 [EARS-E]: In-process docx rendering
> WHEN `fs.write_docx` is called THE engine SHALL render the Markdown to a `.docx` in-process and write it through the shared write path.

- **Inputs:** `markdown`, optional `title`, `overwrite` default false.
- **Outputs:** `{path, bytes_written, overwritten}` (`core/fs_ops.rs:1457-1459`).
- **Business Rules:** numbered list markers are preserved in the generated document, a deliberate behaviour. Writing through the engine means the quota and audit apply as for any write.
- **Priority:** Must-have

#### FR-518 [EARS-O]: Pandoc tools register only when pandoc exists
> IF pandoc is absent from `PATH` and from the configured bin path THEN `doc.to_docx` and `doc.to_pptx` SHALL NOT be registered.

- **Inputs:** the pandoc lookup at registration time.
- **Outputs:** two tools, or none (`tools/doc.rs:3-5`).
- **Business Rules:** registration is a silent no-op with a warning logged so the operator knows why the tools are absent. An LLM must not see a tool it cannot call, which is the same principle as the git family gating in S1. The `doc.*` editor tools are registered regardless.
- **Priority:** Must-have

#### FR-519 [EARS-E]: Pandoc conversion accepts a template
> WHEN `doc.to_docx` or `doc.to_pptx` is called THE server SHALL convert the named Markdown or HTML file in the volume, applying a `.docx` template when `template_path` is given.

- **Inputs:** the source path, an optional `template_path`.
- **Outputs:** the generated document in the volume (`tools/doc.rs:18-21`).
- **Business Rules:** the template supplies custom styles, headers and footers. Conversion runs with a timeout.
- **Priority:** Must-have

### Editing

#### FR-520 [EARS-E]: Opening an editor
> WHEN `doc.open_editor` is called THE server SHALL start an HTTP and WebSocket server on an OS-assigned port, create the file when absent, open the browser, and return an `editor_id` with its URL.

- **Inputs:** the path and the mode, `doc` or `slides`.
- **Outputs:** the editor identifier and URL (`tools/editor.rs:1-8`).
- **Business Rules:** `GET /` returns the complete editor shell with HTML, CSS and JavaScript inline, so the page needs no external asset. Opening the same path a second time returns the existing editor, idempotently. The browser command is skipped during tests.
- **Priority:** Must-have

#### FR-521 [EARS-E]: Bidirectional synchronization
> WHEN the browser saves THE server SHALL write the content to the volume, and WHEN the volume file changes externally THE server SHALL broadcast a reload to every connected client.

- **Inputs:** a WebSocket `save` message; the file's mtime.
- **Outputs:** the written file, or a `reload` message carrying the new HTML (`tools/editor.rs:4-12`).
- **Business Rules:** the mtime is polled every 500 ms (`tools/editor.rs:251`), so an external change surfaces within one poll cycle. The client reconnects automatically after a disconnect.
- **Priority:** Must-have

#### FR-522 [EARS-E]: Editor lifecycle
> WHEN `doc.close_editor` is called THE server SHALL stop that editor and release its port, and `doc.list_editors` SHALL report the active editors.

- **Inputs:** the `editor_id`.
- **Outputs:** the stopped editor, or the active list (`tools/editor.rs:13-14`).
- **Business Rules:** an editor lives until closed or until the server shuts down.
- **Priority:** Must-have

### Code intelligence

#### FR-523 [EARS-E]: Language resolution
> WHEN a file is examined for symbols THE matcher SHALL resolve its language from its extension, and SHALL skip the file when no language is known.

- **Inputs:** the path.
- **Outputs:** the language name, or none (`docs/symbols.rs:39,89-99`).
- **Business Rules:** an unknown extension is skipped silently rather than treated as an error, which is what makes a mixed-content volume searchable at all.
- **Priority:** Must-have

#### FR-524 [EARS-O]: Grammar first, lexical fallback second
> IF a tree-sitter grammar exists for the resolved language THEN the matcher SHALL use it; otherwise it SHALL use the lexical fallback.

- **Inputs:** the language and the file text.
- **Outputs:** definitions or references (`docs/symbols.rs:100-111,264`).
- **Business Rules:** ten grammars are linked statically: Python, JavaScript, TypeScript, TSX, Go, Rust, Java, C, C++ and Ruby. Static linking means there is no grammar loading path and no runtime dependency. The lexical fallback matches an identifier pattern and is what keeps results flowing for the other languages in the extension table.
- **Priority:** Must-have

#### FR-525 [EARS-E]: Definition kinds
> WHEN definitions are requested with a `kind` filter THE matcher SHALL return only definitions of that kind.

- **Inputs:** the `kind` argument.
- **Outputs:** the filtered definitions (`docs/symbols.rs:55`).
- **Business Rules:** kinds are per-language node names such as `function_definition` for Python and `function_item` for Rust, which is why S2's documented example output shows both for one search (`TOOL_CONTRACT.txt:283`).
- **Priority:** Must-have

### Supporting engines

#### FR-526 [EARS-U]: OCR is pluggable and null by default
> The extractor SHALL obtain image text from a configured `OcrProvider`, defaulting to a provider that returns nothing.

- **Inputs:** `extract.ocr.provider`.
- **Outputs:** the provider (`docs/ocr.rs:22-34,147`).
- **Business Rules:** the default is a no-op provider, so the build carries no native Tesseract dependency and stays a single static binary. Swapping in image understanding is a config change and nothing else in the pipeline moves. A `multimodal` provider with an endpoint is enabled; a `tesseract` provider is **not** enabled even when an endpoint is configured (`docs/ocr.rs:177-184`). The provider is rebuilt per call, unlike the document service.
- **Priority:** Must-have

#### FR-527 [EARS-U]: MIME guessing is a closed extension table
> MIME types SHALL be guessed from the extension against a deliberately closed subset rather than a full MIME database.

- **Inputs:** the path.
- **Outputs:** the type, or none (`docs/mime.rs:1-6`).
- **Business Rules:** a smaller, predictable answer set is what the tool contract promises. This table holds 32 extensions and serves the REST download route. `fs.read_bytes` uses a **separate** table (`core/fs_ops.rs:1286-1310`) which the cross-spec audit confirmed holds the same 32 extensions with identical values. The module comment at `docs/mime.rs:2` claims this table serves `fs.read_bytes`, which the code contradicts (S2 §13 item 3, TBD-202, S3 TBD-301).
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- The companion cache means a repeated extraction costs one file read (FR-504).
- The document service is built once so its HTTP pool is reused (FR-507).
- Extraction output is capped by `max_chars` and `preview_chars` (FR-506).
- The editor polls at 500 ms, which bounds change-detection latency and the idle cost of an open editor (FR-521).

### 7.2 Security
- The CLI converter is untrusted and confined by construction, not by trust: fresh tempdir, relative input path, redirected `TMPDIR`, argv list, stdout only, capped stderr, killed and reaped on timeout (FR-511).
- The sandbox's limits are stated rather than overclaimed: a converter writing to an absolute path escapes it.
- Every companion write passes the same quota, ACL and audit rules as any other write (FR-505, FR-513).
- The editor serves its own port with content inline; it holds no credential.

### 7.3 Usability
- An ineligible file is refused with the accepted extension set in the message (FR-509).
- A missing pandoc removes the tools rather than offering a tool that always fails (FR-518).
- `fs.documentize` exists as the retry surface for a conversion that failed after a successful upload (FR-514).
- The stderr tail rather than the head is what an operator needs to diagnose a converter failure (FR-512).

### 7.4 Reliability
- A conversion failure never destroys a stored source (FR-514).
- A batch is validated before any file is written (FR-515).
- A timed-out child is killed and reaped before its directory is removed (FR-511).
- The lexical fallback keeps symbol search working where no grammar exists (FR-524).

### 7.5 Observability
Unchanged from S1 §7.5. Two document-specific behaviours: a pandoc-absent registration logs a warning naming the reason (FR-518), and a converter failure surfaces the exit code and the stderr tail in the error rather than in a log the caller cannot see (FR-512). Conversion duration is not measured and no span is emitted; recorded as TBD-503.

### 7.6 Deployment
- `doc_service` is off by default; enabling it means providing a binary or an endpoint.
- `doc.to_docx` and `doc.to_pptx` need pandoc on the host.
- `scripts/doc_service_fake.py --port 8099` provides a fake service for testing api mode.
- The editor binds an OS-assigned port per open editor and opens a local browser, so it suits a workstation rather than a headless deployment.

### 7.7 Scalability
- Every conversion is a subprocess or an HTTP round trip with no pooling beyond the shared HTTP client, so throughput is bounded by the converter.
- Companion files double the stored object count for converted documents, though content addressing means identical companions share bytes.
- One editor is one server and one port; the count of simultaneous editors is bounded by available ports and is not capped in configuration. Recorded as TBD-502.

## 8. Data Model

| Artifact | Shape | Home |
|---|---|---|
| **Companion** | a `.md` file beside its source, path per FR-503 | the volume |
| **ExtractResult** | Markdown, preview, metadata, `cached` flag, `md_path` | `docs/extract.rs` |
| **DocService** | trait: `to_markdown`, `accepted_extensions`, `max_input_bytes` | `docs/service.rs:61-75` |
| **Definition** | `{path, name, kind, line}` | `docs/symbols.rs` |
| **Reference** | `{path, line, kind}` | `docs/symbols.rs` |
| **Editor** | `{editor_id, path, mode, url}` | `tools/editor.rs` |

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/docs/extract.rs` | Specified, unchanged | FR-501..FR-506 |
| `crates/mcp-fs/src/docs/service.rs` | Specified, unchanged | FR-507..FR-512 |
| `crates/mcp-fs/src/docs/docx.rs` | Specified, unchanged | FR-517 |
| `crates/mcp-fs/src/docs/symbols.rs` | Specified, unchanged | FR-523..FR-525 |
| `crates/mcp-fs/src/docs/ocr.rs` | Specified, unchanged | FR-526 |
| `crates/mcp-fs/src/docs/mime.rs` | Specified, unchanged | FR-527 |
| `crates/mcp-fs/src/core/fs_ops.rs` | Specified, unchanged | FR-513..FR-516, the document operations only |
| `crates/mcp-fs/src/tools/{document,doc,editor}.rs` | Specified, unchanged | FR-516..FR-522 |

### 9.2 Affected Requirements

S2 §3.2 deferred three `fs.*` tools and the symbol matcher here; FR-516, FR-517 and FR-523..FR-525 discharge that. S3 DEC-301 deferred the document routes' semantics here; FR-513..FR-516 discharge that. S3 FR-312 specified the upload flag's parsing; FR-514 specifies its effect.

### 9.3 Affected Tests

| Test location | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/docs/service.rs` `#[cfg(test)]:546-650` | CLI stdout, sandbox removal, timeout kill, stderr tail | Keep; annotate |
| `crates/mcp-fs/src/docs/extract.rs` `#[cfg(test)]` | per-format extraction | Keep; annotate |
| `crates/mcp-fs/src/docs/symbols.rs` `#[cfg(test)]:622+` | grammar and fallback matching | Keep; annotate |
| `crates/mcp-fs/src/docs/ocr.rs` `#[cfg(test)]:177-184` | provider selection | Keep; annotate |
| `crates/mcp-fs/src/tools/editor.rs` | 25 test functions, counted during the cross-spec audit | Keep; annotate |
| `tests/functional/scenarios/07_documents.sh` | document tools over HTTP | Extend |
| `tests/functional/scenarios/11_pandoc.sh` | pandoc tools | Extend |
| `tests/functional/scenarios/12_editor.sh` | editor tools | Extend |
| `tests/functional/scenarios/25_doc_service_cli.sh`, `26_doc_service_api.sh` | both service modes | Extend |

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | overview, documentation index | Reference this spec |
| `BACKLOG.md` | editor entry | Already marked implemented; no change |

### 9.5 Dependencies & Risks

1. **The converter is arbitrary third-party code.** The sandbox is explicitly not a hard OS sandbox (FR-511). A deployment running an untrusted converter without `sandbox-exec`, `bwrap` or a container is exposed, and the spec says so rather than implying otherwise.
2. **`DOC_SERVICE_EXTS` accepts audio and video while the built-in extractor refuses them** (FR-502 against FR-509). This is coherent, the external service is exactly how transcription becomes possible, but a reader can mistake it for a contradiction. Recorded in §13.
3. **Two MIME tables persist** (FR-527), carried from S2 TBD-202 and S3 TBD-301.

## 10. Documentation Requirements

### 10.1 README.md
Document that `doc_service` is off by default and that pandoc is an optional host dependency.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: add this spec to the index; its `doc_service` paragraph already describes FR-507 and FR-514 and should point here.
- A `.agent_docs/documents.md` does not exist; the subject is currently spread across `AGENTS.md` and `tools.md`. Creating it is optional and not required by this spec.

### 10.3 docs/*
None required.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-501 | FR-501, FR-502, FR-503, FR-504, FR-505, FR-506 | E2E-501, E2E-502 | E2E-503, E2E-504, E2E-505, E2E-506 | E2E-507, E2E-508, E2E-509, E2E-510 |
| SC-502 | FR-507, FR-508, FR-509, FR-510, FR-511, FR-512, FR-516 | E2E-511, E2E-512 | E2E-513, E2E-514, E2E-515, E2E-516, E2E-517 | E2E-518, E2E-519, E2E-520, E2E-521 |
| SC-503 | FR-513, FR-514, FR-515 | E2E-522 | E2E-523, E2E-524, E2E-525 | E2E-526, E2E-527 |
| SC-504 | FR-517, FR-518, FR-519 | E2E-528, E2E-529 | E2E-530, E2E-531 | E2E-532, E2E-533 |
| SC-505 | FR-520, FR-521, FR-522 | E2E-534, E2E-535 | E2E-536, E2E-537 | E2E-538, E2E-539 |
| SC-506 | FR-523, FR-524, FR-525, FR-526, FR-527 | E2E-540, E2E-541 | E2E-542, E2E-543, E2E-544 | E2E-545, E2E-546, E2E-547, E2E-548 |

Per-FR coverage:

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-501 | E2E-501, E2E-503, E2E-507 | FR-515 | E2E-524, E2E-526, E2E-527 |
| FR-502 | E2E-504, E2E-507, E2E-508 | FR-516 | E2E-512, E2E-516, E2E-520 |
| FR-503 | E2E-501, E2E-505, E2E-509 | FR-517 | E2E-528, E2E-530, E2E-532 |
| FR-504 | E2E-502, E2E-506, E2E-510 | FR-518 | E2E-529, E2E-531, E2E-533 |
| FR-505 | E2E-502, E2E-505, E2E-510 | FR-519 | E2E-529, E2E-530, E2E-533 |
| FR-506 | E2E-503, E2E-506, E2E-509 | FR-520 | E2E-534, E2E-536, E2E-538 |
| FR-507 | E2E-511, E2E-513, E2E-518 | FR-521 | E2E-535, E2E-537, E2E-539 |
| FR-508 | E2E-511, E2E-514, E2E-519 | FR-522 | E2E-534, E2E-536, E2E-539 |
| FR-509 | E2E-512, E2E-515, E2E-521 | FR-523 | E2E-540, E2E-542, E2E-545 |
| FR-510 | E2E-513, E2E-516, E2E-518 | FR-524 | E2E-541, E2E-543, E2E-546 |
| FR-511 | E2E-514, E2E-517, E2E-520 | FR-525 | E2E-540, E2E-544, E2E-547 |
| FR-512 | E2E-515, E2E-517, E2E-519 | FR-526 | E2E-542, E2E-545, E2E-548 |
| FR-513 | E2E-522, E2E-523, E2E-526 | FR-527 | E2E-543, E2E-546, E2E-548 |
| FR-514 | E2E-522, E2E-524, E2E-525, E2E-527 | | |

## 12. End-to-End Test Suite

**Placement.** Engine-level tests are Rust tests in `docs/*.rs` and `core/fs_ops.rs`. Service-mode tests extend `tests/functional/scenarios/25_doc_service_cli.sh` and `26_doc_service_api.sh`, which already exist for both modes. Document, pandoc and editor tool tests extend `07_documents.sh`, `11_pandoc.sh` and `12_editor.sh`.

**Fixtures:** project `spec-docs`; `/report.pdf` with a text layer containing `Quarterly results`; `/deck.pptx` with one slide titled `Roadmap`; `/data.csv` with a header and two rows; `/notes.txt`; `/photo.png`; `/song.mp3`; `/src/app.py` defining `hello`; `/src/lib.rs` defining `hello`. The fake service is `scripts/doc_service_fake.py --port 8099`.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-501 | Existing | Core Journey | SC-501 | FR-501, FR-503 | Critical |
| E2E-502 | New | Feature | SC-501 | FR-504, FR-505 | Critical |
| E2E-503 | Existing | Error | SC-501 | FR-501, FR-506 | High |
| E2E-504 | Existing | Error | SC-501 | FR-502 | Critical |
| E2E-505 | New | Side Effect | SC-501 | FR-503, FR-505 | Critical |
| E2E-506 | New | Error | SC-501 | FR-504, FR-506 | High |
| E2E-507 | New | Edge | SC-501 | FR-501, FR-502 | High |
| E2E-508 | New | Edge | SC-501 | FR-502 | High |
| E2E-509 | New | Edge | SC-501 | FR-503, FR-506 | High |
| E2E-510 | New | Edge | SC-501 | FR-504, FR-505 | High |
| E2E-511 | Existing | Core Journey | SC-502 | FR-507, FR-508 | Critical |
| E2E-512 | Existing | Feature | SC-502 | FR-509, FR-516 | Critical |
| E2E-513 | New | Error | SC-502 | FR-507, FR-510 | Critical |
| E2E-514 | Existing | Security | SC-502 | FR-508, FR-511 | Critical |
| E2E-515 | Existing | Error | SC-502 | FR-509, FR-512 | High |
| E2E-516 | New | Error | SC-502 | FR-510, FR-516 | High |
| E2E-517 | Existing | Error | SC-502 | FR-511, FR-512 | Critical |
| E2E-518 | New | Edge | SC-502 | FR-507, FR-510 | Medium |
| E2E-519 | New | Edge | SC-502 | FR-508, FR-512 | High |
| E2E-520 | Existing | Edge | SC-502 | FR-511, FR-516 | Critical |
| E2E-521 | New | Edge | SC-502 | FR-509 | High |
| E2E-522 | New | Core Journey | SC-503 | FR-513, FR-514 | Critical |
| E2E-523 | New | Error | SC-503 | FR-513 | High |
| E2E-524 | New | Data Integrity | SC-503 | FR-514, FR-515 | Critical |
| E2E-525 | New | Error | SC-503 | FR-514 | Critical |
| E2E-526 | New | Edge | SC-503 | FR-513, FR-515 | High |
| E2E-527 | New | Edge | SC-503 | FR-514, FR-515 | High |
| E2E-528 | Existing | Feature | SC-504 | FR-517 | Critical |
| E2E-529 | Existing | Feature | SC-504 | FR-518, FR-519 | High |
| E2E-530 | New | Error | SC-504 | FR-517, FR-519 | High |
| E2E-531 | New | Error | SC-504 | FR-518 | High |
| E2E-532 | New | Edge | SC-504 | FR-517 | Medium |
| E2E-533 | New | Edge | SC-504 | FR-518, FR-519 | Medium |
| E2E-534 | Existing | Core Journey | SC-505 | FR-520, FR-522 | Critical |
| E2E-535 | Existing | Feature | SC-505 | FR-521 | Critical |
| E2E-536 | New | Error | SC-505 | FR-520, FR-522 | High |
| E2E-537 | New | Error | SC-505 | FR-521 | High |
| E2E-538 | Existing | Edge | SC-505 | FR-520 | High |
| E2E-539 | New | Edge | SC-505 | FR-521, FR-522 | Medium |
| E2E-540 | Existing | Feature | SC-506 | FR-523, FR-525 | Critical |
| E2E-541 | Existing | Feature | SC-506 | FR-524 | Critical |
| E2E-542 | New | Error | SC-506 | FR-523, FR-526 | High |
| E2E-543 | New | Error | SC-506 | FR-524, FR-527 | High |
| E2E-544 | New | Error | SC-506 | FR-525 | Medium |
| E2E-545 | New | Edge | SC-506 | FR-523, FR-526 | High |
| E2E-546 | New | Edge | SC-506 | FR-524, FR-527 | High |
| E2E-547 | Existing | Edge | SC-506 | FR-525 | High |
| E2E-548 | New | Edge | SC-506 | FR-526, FR-527 | Medium |

**Coverage Statistics** (48 tests):
- Happy path (Core Journey + Feature): 13
- Failure/error (Error + Security): 20
- Side effects: 1
- Edge cases: 13
- Data integrity: 1
- Happy:Failure ratio: 1:1.54

### 12.2 New Test Specifications

#### E2E-502: A second extraction is a free cache hit
- **Category:** Feature | **Scenario:** SC-501 | **Requirements:** FR-504, FR-505
- **Preconditions:** `/report.pdf` exists; a fresh session; no `/report.md`.
- **Steps:**
  - Given a fresh session
  - When `fs.extract_text` is called on `/report.pdf`
  - Then the result has `cached` false and `md_path` `/report.md`
  - And `fs.audit_log` holds one entry with `op` `extract_text` and `path` `/report.md`
  - And when the same call is repeated, the result has `cached` true
  - And the audit log still holds exactly one `extract_text` entry, proving a cache hit costs no quota and no audit
- **Cleanup:** delete `/report.md`
- **Priority:** Critical

#### E2E-505: A newly written companion is charged and readable
- **Category:** Side Effect | **Scenario:** SC-501 | **Requirements:** FR-503, FR-505
- **Preconditions:** a fresh session; `/report.pdf` exists.
- **Steps:**
  - Given a fresh session with `bytes_written` at 0
  - When `fs.extract_text` is called on `/report.pdf`
  - Then `/report.md` exists and `fs.stat` reports a non-zero size
  - And the audit entry's `detail` equals that size followed by ` bytes`
  - And `fs.edit` on `/report.md` succeeds immediately without a separate read, because extraction recorded the read
- **Cleanup:** delete `/report.md`
- **Priority:** Critical

#### E2E-506: max_chars and preview_chars bound the payload
- **Category:** Error | **Scenario:** SC-501 | **Requirements:** FR-504, FR-506
- **Preconditions:** a text file `/long.txt` of 50000 characters.
- **Steps:**
  - Given that file
  - When `fs.extract_text` is called with `max_chars` 100 and `preview_chars` 20
  - Then the returned content is at most 100 characters
  - And the preview is at most 20 characters
  - And the call succeeds rather than failing on the oversized input
- **Cleanup:** delete `/long.txt` and its companion
- **Priority:** High

#### E2E-507: Each format reaches its own backend
- **Category:** Edge | **Scenario:** SC-501 | **Requirements:** FR-501, FR-502
- **Preconditions:** the fixture set.
- **Steps:**
  - Given `/report.pdf`, `/deck.pptx`, `/data.csv`, `/notes.txt` and a `/config.json`
  - When `fs.extract_text` is called on each
  - Then the PDF result contains `Quarterly results`
  - And the PPTX result contains `Roadmap`
  - And the CSV result renders the header row and both data rows
  - And the JSON result is wrapped in a fenced code block tagged `json`
  - And the txt result is the decoded text with no fence
- **Cleanup:** delete the companions
- **Priority:** High

#### E2E-508: Audio and video are refused with the right code
- **Category:** Edge | **Scenario:** SC-501 | **Requirements:** FR-502
- **Preconditions:** `/song.mp3` and a `/clip.mp4` exist.
- **Steps:**
  - Given those files
  - When `fs.extract_text` is called on each
  - Then both fail with `ERR_NOT_SUPPORTED`, not `ERR_INVALID_ARGUMENT`
  - And a `.doc` legacy file does **not** fail, instead falling through to the text decoder
- **Cleanup:** none
- **Priority:** High

#### E2E-509: The companion path follows the derivation rule
- **Category:** Edge | **Scenario:** SC-501 | **Requirements:** FR-503, FR-506
- **Steps:**
  - Given the paths `/report.pdf`, `/dir.with.dots/report.pdf` and `/noext`
  - When `companion_md_path` is applied to each
  - Then the results are `/report.md`, `/dir.with.dots/report.md` and `/noext.md`
  - And no result truncates the directory name at its dot
- **Priority:** High

#### E2E-510: refresh forces regeneration and is charged again
- **Category:** Edge | **Scenario:** SC-501 | **Requirements:** FR-504, FR-505
- **Preconditions:** `/report.pdf` already extracted once, so `/report.md` exists.
- **Steps:**
  - Given an existing companion
  - When `fs.extract_text` is called with `refresh` true
  - Then the result has `cached` false
  - And a second `extract_text` audit entry appears
  - And the companion's mtime has advanced
- **Cleanup:** delete `/report.md`
- **Priority:** High

#### E2E-513: An unconfigured service refuses clearly
- **Category:** Error | **Scenario:** SC-502 | **Requirements:** FR-507, FR-510
- **Preconditions:** a server with `doc_service` disabled.
- **Steps:**
  - Given no document service
  - When `fs.documentize` is called on `/deck.pptx`
  - Then the call fails with `ERR_NOT_SUPPORTED` and the message `document service is not configured`
  - And `fs.write_bytes` with `trigger_documentation_service` true fails the same way
  - And no file is written by the second call
- **Cleanup:** stop the server
- **Priority:** Critical

#### E2E-516: The size gate names the size and the limit
- **Category:** Error | **Scenario:** SC-502 | **Requirements:** FR-510, FR-516
- **Preconditions:** a service configured with `max_input_bytes` 1024; a `/big.pdf` of 2048 bytes.
- **Steps:**
  - Given that configuration
  - When `fs.documentize` is called on `/big.pdf`
  - Then the call fails with `ERR_INVALID_ARGUMENT`
  - And the message contains both `2048` and `1024`
  - And no companion is written
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-518: The service is constructed once
- **Category:** Edge | **Scenario:** SC-502 | **Requirements:** FR-507, FR-510
- **Preconditions:** api mode pointed at the fake service, which logs each connection.
- **Steps:**
  - Given a running server
  - When three `fs.documentize` calls are made in sequence
  - Then all three succeed
  - And the fake service observes connection reuse rather than three fresh clients, confirmed by its connection log
- **Cleanup:** stop both processes
- **Priority:** Medium

#### E2E-519: Both modes satisfy the same contract
- **Category:** Edge | **Scenario:** SC-502 | **Requirements:** FR-508, FR-512
- **Preconditions:** two servers, one in `cli` mode and one in `api` mode, both converting `/deck.pptx` to the same known Markdown.
- **Steps:**
  - Given both servers
  - When `fs.documentize` is called on each
  - Then both produce `/deck.md` with identical content
  - And both report the same result keys `path`, `md_path`, `bytes_written` and `overwritten`
- **Cleanup:** stop both servers
- **Priority:** High

#### E2E-521: A configured extension list overrides the built-in set
- **Category:** Edge | **Scenario:** SC-502 | **Requirements:** FR-509
- **Preconditions:** a service configured with `extensions: [".pptx"]`.
- **Steps:**
  - Given that configuration
  - When `fs.documentize` is called on `/deck.pptx`
  - Then it succeeds
  - And when called on `/report.pdf`, which is in the built-in set but not the configured list, it fails with `ERR_NOT_SUPPORTED`
  - And the failure message names `.pptx` as the accepted set
  - And with `extensions: []` the built-in set applies and `/report.pdf` succeeds
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-522: An upload with the flag produces source and companion
- **Category:** Core Journey | **Scenario:** SC-503 | **Requirements:** FR-513, FR-514
- **Preconditions:** service configured; a fresh session.
- **Steps:**
  - Given a `fs.write_bytes` call carrying a base64 PPTX at `/new.pptx` with `trigger_documentation_service` true
  - When the call completes
  - Then `/new.pptx` exists with the uploaded bytes
  - And `/new.md` exists holding the converted Markdown
  - And a subsequent `fs.extract_text` on `/new.pptx` returns `cached` true, proving the companion path is shared
- **Cleanup:** delete both files
- **Priority:** Critical

#### E2E-523: The companion is written under its own audit op
- **Category:** Error | **Scenario:** SC-503 | **Requirements:** FR-513
- **Preconditions:** service configured; a fresh session.
- **Steps:**
  - Given a triggered upload of `/new.pptx`
  - When `fs.audit_log` is read
  - Then it contains one entry for the source with op `write`
  - And one entry for the companion whose op is **not** `write`, distinguishing it from a plain write
- **Cleanup:** delete both files
- **Priority:** High

#### E2E-524: An ineligible file with the flag set writes nothing
- **Category:** Data Integrity | **Scenario:** SC-503 | **Requirements:** FR-514, FR-515
- **Preconditions:** service configured with the built-in extension set.
- **Steps:**
  - Given a `fs.write_bytes` call at `/notes.txt` with `trigger_documentation_service` true, `.txt` being outside the accepted set
  - When the call is made
  - Then it fails with `ERR_NOT_SUPPORTED`
  - And `fs.exists` on `/notes.txt` reports absence, proving the precondition was checked before the first byte was written
  - And the session's audit log holds no entry for that path
- **Cleanup:** none
- **Priority:** Critical

#### E2E-525: A conversion failure after the write keeps the source
- **Category:** Error | **Scenario:** SC-503 | **Requirements:** FR-514
- **Preconditions:** a service configured to fail, for example the fake service started in failing mode.
- **Steps:**
  - Given a converter that always fails
  - When `fs.write_bytes` is called at `/new.pptx` with the flag true
  - Then `/new.pptx` exists with the uploaded bytes
  - And no `/new.md` exists
  - And a later `fs.documentize` on `/new.pptx` against a working converter produces the companion, confirming documentize is the retry surface
- **Cleanup:** delete both files
- **Priority:** Critical

#### E2E-526: A batch upload validates every file before writing any
- **Category:** Edge | **Scenario:** SC-503 | **Requirements:** FR-513, FR-515
- **Preconditions:** service configured; REST upload available.
- **Steps:**
  - Given a multipart upload of `good.pptx` and `bad.txt` with `trigger_documentation_service` set to `true`
  - When the request is made
  - Then it fails
  - And neither `good.pptx` nor `bad.txt` exists in the volume, proving validation precedes the first write for the whole batch
- **Cleanup:** none
- **Priority:** High

#### E2E-527: The flag left false writes the source and no companion
- **Category:** Edge | **Scenario:** SC-503 | **Requirements:** FR-514, FR-515
- **Preconditions:** service configured.
- **Steps:**
  - Given `fs.write_bytes` at `/plain.pptx` with `trigger_documentation_service` omitted
  - When the call completes
  - Then `/plain.pptx` exists
  - And no `/plain.md` exists
  - And the result's `documentation` field is null
- **Cleanup:** delete `/plain.pptx`
- **Priority:** High

#### E2E-530: write_docx respects no-clobber and charges the quota
- **Category:** Error | **Scenario:** SC-504 | **Requirements:** FR-517, FR-519
- **Preconditions:** `/out.docx` already exists; the session has not read it.
- **Steps:**
  - Given an existing target
  - When `fs.write_docx` is called with `overwrite` false
  - Then the call fails with `ERR_NO_CLOBBER`
  - And with `overwrite` true but no prior read it fails with `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And after a read it succeeds and the audit log records the write
- **Cleanup:** delete `/out.docx`
- **Priority:** High

#### E2E-531: Absent pandoc removes the tools from the catalogue
- **Category:** Error | **Scenario:** SC-504 | **Requirements:** FR-518
- **Preconditions:** a server started with `doc.enabled` true and `PATH` scrubbed of pandoc.
- **Steps:**
  - Given that server
  - When `tools/list` is called
  - Then neither `doc.to_docx` nor `doc.to_pptx` appears
  - And `doc.open_editor`, `doc.close_editor` and `doc.list_editors` **do** appear
  - And calling `doc.to_docx` returns the JSON-RPC unknown-tool error `-32602`
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-532: A generated docx preserves numbered list markers
- **Category:** Edge | **Scenario:** SC-504 | **Requirements:** FR-517
- **Steps:**
  - Given the Markdown `1. first\n2. second\n`
  - When `fs.write_docx` renders it
  - Then the generated document's text retains the markers `1.` and `2.`
  - And the file opens as a valid OOXML package, confirmed by unzipping it and finding `word/document.xml`
- **Cleanup:** delete the output
- **Priority:** Medium

#### E2E-533: A template applies styles to the pandoc output
- **Category:** Edge | **Scenario:** SC-504 | **Requirements:** FR-518, FR-519
- **Preconditions:** pandoc present; a `/template.docx` in the volume.
- **Steps:**
  - Given a Markdown source and the template
  - When `doc.to_docx` is called with `template_path` `/template.docx`
  - Then the call succeeds and the output exists
  - And the output is a valid OOXML package
  - And calling without the template also succeeds, producing a different byte length
- **Cleanup:** delete the outputs
- **Priority:** Medium

#### E2E-536: Closing an editor releases its port and reopening is idempotent
- **Category:** Error | **Scenario:** SC-505 | **Requirements:** FR-520, FR-522
- **Steps:**
  - Given `doc.open_editor` called on `/doc.html`, returning `editor_id` and a URL on port P
  - When `doc.list_editors` is called
  - Then it lists exactly one editor with that id
  - And calling `doc.open_editor` again on the same path returns the **same** `editor_id`
  - And after `doc.close_editor`, `doc.list_editors` is empty and port P accepts no connection
- **Cleanup:** none
- **Priority:** High

#### E2E-537: An external change reaches the browser within a poll cycle
- **Category:** Error | **Scenario:** SC-505 | **Requirements:** FR-521
- **Preconditions:** an open editor on `/doc.html` with a connected WebSocket client.
- **Steps:**
  - Given a connected client that has received its initial content
  - When `/doc.html` is modified through `fs.write` by another caller
  - Then the client receives a message whose `type` is `reload` within 2 seconds, comfortably more than one 500 ms cycle
  - And the message's `html` field carries the new content
- **Cleanup:** close the editor
- **Priority:** High

#### E2E-539: A save from the browser reaches the volume
- **Category:** Edge | **Scenario:** SC-505 | **Requirements:** FR-521, FR-522
- **Preconditions:** an open editor on `/doc.html`.
- **Steps:**
  - Given a connected WebSocket client
  - When it sends `{"type":"save","html":"<p>edited</p>"}`
  - Then `fs.read` of `/doc.html` returns content containing `<p>edited</p>`
  - And the editor remains listed as active
- **Cleanup:** close the editor
- **Priority:** Medium

#### E2E-542: An unknown extension is skipped silently
- **Category:** Error | **Scenario:** SC-506 | **Requirements:** FR-523, FR-526
- **Preconditions:** `/notes.xyz` containing `def hello():` and `/src/app.py` defining `hello`.
- **Steps:**
  - Given both files
  - When `fs.find_definition` is called for `hello`
  - Then the result includes the definition in `/src/app.py`
  - And no entry references `/notes.xyz`
  - And the call succeeds rather than erroring on the unknown extension
- **Cleanup:** delete `/notes.xyz`
- **Priority:** High

#### E2E-543: The lexical fallback runs where no grammar exists
- **Category:** Error | **Scenario:** SC-506 | **Requirements:** FR-524, FR-527
- **Preconditions:** a source file in a language present in the extension table but with no linked grammar.
- **Steps:**
  - Given such a file defining a symbol `hello`
  - When `fs.find_definition` is called for `hello`
  - Then at least one definition is returned
  - And its `path` is that file, proving the fallback produced a result where the grammar path could not
- **Cleanup:** delete the file
- **Priority:** High

#### E2E-544: The kind filter narrows the result
- **Category:** Error | **Scenario:** SC-506 | **Requirements:** FR-525
- **Preconditions:** `/src/app.py` defining both a function `hello` and a class `hello`.
- **Steps:**
  - Given both definitions
  - When `fs.find_definition` is called with `kind` `function_definition`
  - Then every returned entry has `kind` `function_definition`
  - And calling without a kind returns both entries
- **Cleanup:** none
- **Priority:** Medium

#### E2E-545: Ten grammars are available and resolve by extension
- **Category:** Edge | **Scenario:** SC-506 | **Requirements:** FR-523, FR-526
- **Steps:**
  - Given one source file per supported language: `.py`, `.js`, `.ts`, `.tsx`, `.go`, `.rs`, `.java`, `.c`, `.cpp`, `.rb`, each defining `hello`
  - When `fs.find_definition` is called for `hello` with `root` `/`
  - Then every one of the ten files appears in `definitions`
  - And each entry's `kind` is the language's own node name rather than a normalized label
- **Cleanup:** delete the fixtures
- **Priority:** High

#### E2E-546: OCR is null by default and selected by config
- **Category:** Edge | **Scenario:** SC-506 | **Requirements:** FR-524, FR-527
- **Steps:**
  - Given the default configuration
  - When the provider is built
  - Then it reports itself as not enabled
  - And a provider configured as `tesseract` with an endpoint is also not enabled
  - And a provider configured as `multimodal` with an endpoint **is** enabled
- **Priority:** High

#### E2E-548: MIME guessing is a closed table
- **Category:** Edge | **Scenario:** SC-506 | **Requirements:** FR-526, FR-527
- **Steps:**
  - Given the paths `/a.txt`, `/report.pdf`, `/photo.png` and `/thing.zzz`
  - When `docs::guess_mime` is applied to each
  - Then the first three return their expected types
  - And `/thing.zzz` returns none, which the caller renders as `application/octet-stream`
  - And the same four paths are passed to `core::fs_ops::mime_guess`, which returns identical values, so the test fails the moment the two tables diverge
- **Priority:** Medium

### 12.3 Modified Test Specifications

None.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

1. **The external service accepts audio and video; the built-in extractor refuses them.** FR-502 refuses them for extraction because transcription needs a speech model; FR-509's `DOC_SERVICE_EXTS` includes them because an external converter is exactly how transcription becomes possible. Both are correct and they are not in conflict.
2. **Two MIME tables**, from S2 TBD-202 and S3 TBD-301. E2E-548 is written to detect disagreement between them rather than to assume there is none.
3. **S2 FR-226 and S5 FR-523..FR-525 split the symbol tools.** S2 owns the tools' gates and result shapes, S5 owns the matcher. Neither restates the other.
4. **`BACKLOG.md` at the repository root marks the editor implemented** and states it has 17 tests in `tools::editor`. The cross-spec audit recounted them: `tools/editor.rs` holds 25 test functions. `BACKLOG.md` is stale on that number; this specification uses 25.

## 14. Migration & Implementation Notes

No production code change. Test work, in order:

1. **Annotate existing tests** in `docs/*.rs`, `tools/editor.rs` and the five functional scenario scripts.
2. **Add pure-function tests first**: E2E-509 (companion path), E2E-546 (provider selection), E2E-548 (MIME tables). They need no server, no volume and no converter.
3. **Add extraction tests** (E2E-502, E2E-505, E2E-506, E2E-507, E2E-508, E2E-510), which need a volume but no external process.
4. **Add service tests** (E2E-513 through E2E-527), which need the fake service. E2E-525 needs it in a **failing** mode; if `scripts/doc_service_fake.py` has no failure switch, adding one to the script, not to the server, is the correct move.
5. **Symbol tests** (E2E-542 through E2E-545) need one fixture file per language; keep them in their own scenario so the fixture cost is paid once.
6. **Editor tests** (E2E-536, E2E-537, E2E-539) need a WebSocket client. E2E-537 must wait at least two poll cycles before failing, otherwise it is flaky by construction.
7. **E2E-531 scrubs pandoc from `PATH`**, so it starts its own server and must not run concurrently with E2E-529 or E2E-533.

## 15. Open Questions & TBDs

- **TBD-501:** The CLI sandbox is not a hard OS sandbox (FR-511). Whether the project should ship a recommended `sandbox-exec` or `bwrap` wrapper, rather than leaving containment entirely to the operator, is undecided.
- **TBD-502:** Nothing caps the number of simultaneously open editors (§7.7). Each is a server and a port.
- **TBD-503:** Conversion duration is unmeasured (§7.5). A converter that has become slow is invisible until a timeout fires.
- **TBD-504:** Closed by the cross-spec audit. `BACKLOG.md` states 17 editor tests; the actual count in `tools/editor.rs` is 25. `BACKLOG.md` is stale and updating it is a documentation task, not a specification question.

## 16. Glossary

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

## 17. Interview Decisions Log

Produced non-interactively from the code.

- **DEC-501:** The sandbox's limits are stated explicitly rather than described as containment. **Rationale:** a spec that implied a hard sandbox would let an operator run an untrusted converter believing it was confined. **Alternatives considered:** describing the measures without the caveat. **Implemented by:** FR-511, §7.2, TBD-501. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/docs/service.rs:22-26`.
- **DEC-502:** The asymmetric ordering rule around a triggered write is specified as one requirement covering both halves. **Rationale:** the two halves only make sense together: strict before the write, forgiving after. Splitting them would let one be changed without the other. **Alternatives considered:** two requirements. **Implemented by:** FR-514, E2E-524, E2E-525. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/core/fs_ops.rs:624-632`.
- **DEC-503:** The companion path is specified as the shared artifact rather than as an implementation detail of each producer. **Rationale:** it is the reason a converted document is a free cache hit, which is the layer's main economy. **Alternatives considered:** letting each producer own its output path. **Implemented by:** FR-503, FR-513, E2E-522. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/core/fs_ops.rs:735-739`.
- **DEC-504:** Conditional registration of the pandoc tools is specified as a requirement. **Rationale:** it changes the tool catalogue an LLM sees, so it is observable contract, not deployment trivia. **Alternatives considered:** registering them always and failing at call time. **Implemented by:** FR-518, E2E-531. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/doc.rs:3-5`.
- **DEC-505:** The audio/video divergence between the two engines is recorded in §13 rather than resolved. **Rationale:** it is coherent as-built and resolving it either way is a product decision. **Alternatives considered:** aligning the two extension sets. **Implemented by:** FR-502, FR-509, §13 item 1. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/docs/extract.rs:38-41` against `crates/mcp-fs/src/docs/service.rs:42-48`.
- **DEC-506:** E2E-548 is written to compare the two MIME tables rather than to assert one is correct. **Rationale:** the duplication is real and unresolved across three specs; a test that reports disagreement converts an open question into evidence. **Alternatives considered:** asserting one table's values only. **Implemented by:** FR-527, E2E-548, TBD-301 in S3. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/docs/mime.rs:1-6` against `crates/mcp-fs/src/core/fs_ops.rs:1286`.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 0 | 2 (A-3 editor test count, A-6 MIME duplication) | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** editor count corrected 17 to 25; TBD-504 closed; FR-527 and E2E-548 restated with evidence
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
