> Id: SPEC-0006
> Nature: FEAT
> Status: as-built
> Area: documents
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Documents, Extraction and Conversion — Design

Source: `specs/SPEC-0006_2026-09-18_19-20-00-documents/spec.md` (pre-move)

## 1. Components

| Component | File | Role |
|---|---|---|
| Extractor | `crates/core/src/docs/extract.rs` | Bytes → Markdown by format, companion derivation/caching. |
| DocService trait + impls | `crates/core/src/docs/service.rs` | CLI and API converters, sandbox, eligibility, size gate. |
| Docx renderer | `crates/core/src/docs/docx.rs` | In-process Markdown → OOXML. |
| Symbol matcher | `crates/core/src/docs/symbols.rs` | Language table, 10 tree-sitter grammars, lexical fallback. |
| OCR | `crates/core/src/docs/ocr.rs` | Pluggable image-to-text provider, null by default. |
| MIME table | `crates/core/src/docs/mime.rs` | Closed extension-to-MIME table for REST download. |
| Engine orchestration | `crates/core/src/core/fs_ops.rs` | `documentize`, `extract_document`, `write_docx`, `ensure_documentable`, `ensure_input_size`, `write_companion`. |
| Document tools | `crates/core/src/tools/document.rs` | `fs.extract_text`, `fs.write_docx`, `fs.documentize`. |
| Pandoc tools | `crates/core/src/tools/doc.rs` | `doc.to_docx`, `doc.to_pptx`, conditionally registered. |
| Editor tools/server | `crates/core/src/tools/editor.rs` | `doc.open_editor`, `doc.close_editor`, `doc.list_editors`, per-editor axum server + WebSocket. |

## 2. Flows

1. **Extraction (SC-001):** tool → `core::fs_ops::extract_document` → check companion via `companion_md_path` → cache hit returns immediately (no charge/audit) → cache miss dispatches to `docs::extract` backend by extension → write companion via engine write path → charge quota, record read, audit under `extract_text`.
2. **Documentize (SC-002):** tool → `core::fs_ops::documentize` → `safety`/ACL checks → is-file check → `ensure_documentable` (eligibility) → read bytes → `ensure_input_size` (size gate) → `DocService::to_markdown` → `write_companion` (charge + audit under its own op).
3. **Triggered upload (SC-003):** `fs.write_bytes`/REST upload with `trigger_documentation_service` → precondition validation for the whole batch before any byte is written → commit source(s) → best-effort conversion per file → companion written on success, failure leaves source intact and `documentation` null.
4. **Generation (SC-004):** `fs.write_docx` → `docs::docx` render → engine write path (quota/audit/no-clobber). `doc.to_docx`/`doc.to_pptx` → pandoc subprocess with optional template, registered only if pandoc resolves at startup.
5. **Editing (SC-005):** `doc.open_editor` → spawn axum server on ephemeral port, serve inline shell at `/`, upgrade `/ws` → WebSocket `save` → volume write; background task polls mtime every 500 ms → broadcasts `reload` on change.
6. **Symbol matching (SC-006):** `fs.find_definition`/`fs.find_references` (S2-owned tools) → per-file: resolve language by extension → tree-sitter grammar if linked, else lexical fallback → filter by `kind` if given.

## 3. Interfaces

- `fs.extract_text(path, max_chars=200000, preview_chars=4000, ocr=true, refresh=false)` → `{content, preview, cached, md_path, ...}`.
- `fs.documentize(path, overwrite=false)` → `{path, md_path, bytes_written, overwritten}`.
- `fs.write_docx(markdown, title?, overwrite=false)` → `{path, bytes_written, overwritten}`.
- `fs.write_bytes(..., trigger_documentation_service=false)` → includes `documentation` (null or companion info).
- REST upload: same `trigger_documentation_service` flag, same ordering semantics (S3-owned routing).
- `doc.to_docx(path, template_path?)`, `doc.to_pptx(path, template_path?)` — pandoc-backed, conditionally registered.
- `doc.open_editor(path, mode)` → `{editor_id, url}`; `doc.close_editor(editor_id)`; `doc.list_editors()` → list.
- Editor HTTP/WS: `GET /` (inline shell), `GET /ws` (upgrade); WS messages `{"type":"save","html":...}` from client, `{"type":"reload","html":...}` from server.
- `DocService` trait: `to_markdown(bytes, name) -> Result<String>`, `accepted_extensions() -> &[&str]`, `max_input_bytes() -> usize`.
- `OcrProvider` trait: image bytes → text, `is_enabled()`.

## 4. Data and State

| Artifact | Shape | Home |
|---|---|---|
| **Companion** | a `.md` file beside its source, path per FR-003 | the volume |
| **ExtractResult** | Markdown, preview, metadata, `cached` flag, `md_path` | `docs/extract.rs` |
| **DocService** | trait: `to_markdown`, `accepted_extensions`, `max_input_bytes` | `docs/service.rs` |
| **Definition** | `{path, name, kind, line}` | `docs/symbols.rs` |
| **Reference** | `{path, line, kind}` | `docs/symbols.rs` |
| **Editor** | `{editor_id, path, mode, url}` | `tools/editor.rs` |

Companions are content-addressed like any volume file: identical converted output shares bytes across sources. No separate database table backs companions; they are ordinary nodes at a derived path.

## 5. Configuration

- `doc_service`: off by default. `mode` (`cli`|`api`), command/endpoint, `extensions` (empty = built-in `DOC_SERVICE_EXTS`), `max_input_bytes`.
- `extract.ocr.provider`: `null` (default), `multimodal` (enabled with endpoint), `tesseract` (never enabled regardless of config, per FR-026).
- `doc.enabled`: gates editor tool registration.
- Pandoc: resolved from `PATH` or a configured bin path at registration time; absence silently skips `doc.to_docx`/`doc.to_pptx` registration with a logged warning.
- `scripts/doc_service_fake.py --port 8099`: test double for `api` mode.

## 6. Observability

- A pandoc-absent registration logs a warning naming the reason (FR-018).
- A converter failure surfaces exit code + stderr tail (CLI) or status + body head (API) in the error returned to the caller, not only in a server log (FR-012).
- Companion writes are audited under a distinct op from plain `write`, so the audit log distinguishes conversion output from user writes (FR-013).
- **Gap (TBD-001):** conversion duration is unmeasured; no span is emitted. A converter that has become slow is invisible until its timeout fires.

## 7. Decisions

- **DEC-001:** The sandbox's limits are stated explicitly rather than described as containment. **Rationale:** a spec that implied a hard sandbox would let an operator run an untrusted converter believing it was confined. **Alternatives considered:** describing the measures without the caveat. **Implemented by:** FR-011, §7.2(spec), TBD-001. **Code evidence:** `crates/core/src/docs/service.rs` (sandbox setup).
- **DEC-002:** The asymmetric ordering rule around a triggered write is specified as one requirement covering both halves. **Rationale:** the two halves only make sense together: strict before the write, forgiving after. Splitting them would let one be changed without the other. **Implemented by:** FR-014, E2E-024, E2E-025. **Code evidence:** `crates/core/src/core/fs_ops.rs` (triggered-write ordering).
- **DEC-003:** The companion path is specified as the shared artifact rather than as an implementation detail of each producer. **Rationale:** it is the reason a converted document is a free cache hit, the layer's main economy. **Implemented by:** FR-003, FR-013, E2E-022. **Code evidence:** `crates/core/src/core/fs_ops.rs` (`write_companion`).
- **DEC-004:** Conditional registration of the pandoc tools is specified as a requirement. **Rationale:** it changes the tool catalogue an LLM sees, so it is observable contract, not deployment trivia. **Alternatives considered:** registering them always and failing at call time. **Implemented by:** FR-018, E2E-031. **Code evidence:** `crates/core/src/tools/doc.rs` (registration gate).
- **DEC-005:** The audio/video divergence between the built-in extractor and the external service is recorded as a consistency note rather than resolved. **Rationale:** it is coherent as-built; resolving it either way is a product decision. **Implemented by:** FR-002, FR-009, §Consistency Notes item 1. **Code evidence:** `crates/core/src/docs/extract.rs` AV_EXTS against `crates/core/src/docs/service.rs` DOC_SERVICE_EXTS.
- **DEC-006:** E2E-048 is written to compare the two MIME tables rather than to assert one is correct. **Rationale:** the duplication is real and unresolved across specs; a test that reports disagreement converts an open question into evidence. **Implemented by:** FR-027, E2E-048. **Code evidence:** `crates/core/src/docs/mime.rs` against `crates/core/src/core/fs_ops.rs` (read_bytes MIME table).

## 8. Requirement to Code Map

| FR | Code |
|---|---|
| FR-001, FR-002 | `crates/core/src/docs/extract.rs` (format dispatch, `TEXT_EXTS`/`FENCED_EXTS`/`IMAGE_EXTS`/`AV_EXTS`) |
| FR-003 | `crates/core/src/docs/extract.rs` (`companion_md_path`) |
| FR-004, FR-005, FR-006 | `crates/core/src/core/fs_ops.rs:1525` (`extract_document`) |
| FR-007, FR-008 | `crates/core/src/docs/service.rs` (`DocService` trait, factory) |
| FR-009 | `crates/core/src/docs/service.rs` (`DOC_SERVICE_EXTS`, eligibility) |
| FR-010 | `crates/core/src/core/fs_ops.rs:732` (`ensure_input_size`) |
| FR-011, FR-012 | `crates/core/src/docs/service.rs` (CLI sandbox, stderr/body caps) |
| FR-013 | `crates/core/src/core/fs_ops.rs:750` (`write_companion`) |
| FR-014, FR-015 | `crates/core/src/core/fs_ops.rs:714` (`ensure_documentable`), `fs_ops.rs` write-flow ordering |
| FR-016 | `crates/core/src/core/fs_ops.rs:679` (`documentize`) |
| FR-017 | `crates/core/src/core/fs_ops.rs:1565` (`write_docx`), `crates/core/src/docs/docx.rs` |
| FR-018, FR-019 | `crates/core/src/tools/doc.rs` |
| FR-020, FR-021, FR-022 | `crates/core/src/tools/editor.rs` |
| FR-023, FR-024, FR-025 | `crates/core/src/docs/symbols.rs` |
| FR-026 | `crates/core/src/docs/ocr.rs` |
| FR-027 | `crates/core/src/docs/mime.rs`, `crates/core/src/core/fs_ops.rs` (read_bytes table) |

## 9. Legacy Mapping (Source: specs/SPEC-0006_2026-09-18_19-20-00-documents/spec.md (pre-move))

| Pre-move ID | Current ID | Mapping/Note |
|---|---|---|
| FR-501..FR-527 | FR-001..FR-027 | Straight renumber, 1:1, no content change. |
| E2E-501..E2E-548 | E2E-001..E2E-048 | Straight renumber, 1:1 by offset -500. |
| DEC-501..DEC-506 | DEC-001..DEC-006 | Straight renumber, 1:1. |
| SC-501..SC-506 | SC-001..SC-006 | Straight renumber, 1:1. |
| File paths `crates/mcp-fs/src/docs/*.rs`, `crates/mcp-fs/src/tools/*.rs`, `crates/mcp-fs/src/core/fs_ops.rs` | `crates/core/src/docs/*.rs`, `crates/core/src/tools/*.rs`, `crates/core/src/core/fs_ops.rs` | The crate was restructured since the original spec was written (per AGENTS.md, logic now lives in `crates/core`, package `mcp-fs-core`). All file citations updated; line numbers re-verified against current tree where spot-checked, otherwise carried forward and should be treated as approximate. |
| TBD-501 (sandbox hardening) | TBD-001 | Carried forward unresolved. |
| TBD-502 (editor concurrency cap) | TBD-002 | Carried forward unresolved. |
| TBD-503 (conversion duration unmeasured) | TBD-003 | Carried forward unresolved. |
| TBD-504 (editor test count, closed by original audit at 25) | — | Re-opened: current tree shows 24 test functions in `tools/editor.rs`, not 25. See FINDINGS FOR BACKLOG. |

### Consistency Notes (carried forward)

1. The external service accepts audio and video; the built-in extractor refuses them. Both are correct and not in conflict (FR-002 vs FR-009).
2. Two MIME tables exist (`docs/mime.rs` and the `read_bytes` table in `fs_ops.rs`); E2E-048 is designed to detect divergence rather than assume agreement.
3. The symbol tools' gates/result shapes are owned elsewhere (not in this spec's scope); only the matcher (FR-023..FR-025) is owned here.

### Implementability Audit (notes)

The original cross-spec audit (2026-09-18) found 0 functional-blocking issues and 2 drift items (editor test count, MIME duplication), both amended in place rather than registered as open drift. This retro-spec re-verified the amended figures against the current tree and found the editor test count has drifted again (25 → 24); see Drift Register below.

## Drift Register

### FINDINGS FOR BACKLOG

- **Editor test count drift.** The pre-move spec's §13 note states `tools/editor.rs` holds 25 test functions (correcting an older `BACKLOG.md` figure of 17). The current tree (base commit `27bc282acdfd2a5c972802e60ed0e7b566e37ab5`) has 24 `#[test]`/`#[tokio::test]` functions in `crates/core/src/tools/editor.rs`. Non-blocking (a test may have been merged or removed since); recommend a one-line fix to any doc still citing "25" and a note that this count is not contract-stable, so should not be hand-maintained in prose going forward.
- **MIME table doc comment still says it serves `fs.read_bytes`.** `crates/core/src/docs/mime.rs`'s header comment claims it is used by `fs.read_bytes`, which is contradicted by the code (the separate table in `fs_ops.rs` does). This was flagged (not fixed) by the original spec and remains unresolved in the current tree; both tables currently agree in values, but the stale comment should be corrected to avoid future divergence going unnoticed.
- **Conversion duration still unmeasured (TBD-003).** No tracing span wraps a `DocService::to_markdown` call; a converter that becomes slow is invisible until its timeout fires. Still true in the current tree. Recommend a `#[tracing::instrument]` span per the project's observability conventions.
- **No cap on simultaneous open editors (TBD-002).** Still true; each editor is a server + a port with no configured ceiling. Low priority unless editor usage grows.
