# US-0007: Images through every conversion path

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0006
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Artifacts must be kept and reported identically whether the member uses the convert action, a documented upload, or text extraction, and every refusal of today must stay word for word.

## Requirements covered
- SPEC-0019/FR-NEW-014: IF a write or upload requesting documentation writes the source document but its conversion exceeds the session write quota THEN THE Documents context SHALL keep the source written and charged, keep no artifact and no new Markdown sibling, and report the documentation outcome as `session write quota of <N> bytes exceeded`.
- SPEC-0019/FR-NEW-016: The Documents context SHALL NOT let anyone who is not a member of the project list, get or view the artifacts of its documents, nor convert or re-convert them.
- SPEC-0019/FR-NEW-017: WHEN a member extracts a document's text THE Documents context SHALL keep the tables the built-in conversion finds in Word, Excel and CSV documents, labelled `CSV quality: exact`, and no images.
- SPEC-0019/FR-NEW-018: IF the convert action or a write or upload requesting documentation targets a format the configured converter does not accept THEN THE Documents context SHALL answer today's refusal, verbatim `'<path>' cannot be documented, the document service accepts: <extensions>`, and keep no artifact.
- SPEC-0019/FR-NEW-029: IF the convert action or a write or upload requesting documentation is used while no external converter is configured THEN THE Documents context SHALL answer today's refusal, verbatim `document service is not configured`, and keep no artifact.

## Independent Test
`make check`; then upload `report.pdf` requesting documentation with a session at 0 bytes and a 100 000 byte set: the upload succeeds, the documentation outcome reads the quota message, and the session still accepts exactly 62 144 more bytes.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0019): One charge per conversion of (set bytes + sibling UTF-8 bytes + conversion text UTF-8 bytes) through `charge_write` before any write; set bytes as FR-NEW-013 defines; a documented upload keeps its source charge and reports the quota message as its documentation outcome. **Obliged by:** FR-NEW-013, FR-NEW-014, FR-MOD-001. **Rationale:** a refusal leaves nothing. **Alternatives considered:** charging after commit (today's extraction order, refused by FR-MOD-001). **Source:** spec. **Evidence:** `safety.rs:130-136`, `fs_ops.rs:1553-1555`, `fs_ops.rs:651-666`.
- DEC-006 (SPEC-0019): `DocService::convert` returns a `Conversion` bundle; CLI `{outdir}` placeholder and API JSON bundle; optional `artifacts.json` manifest; `to_markdown` kept as a wrapper. **Obliged by:** FR-NEW-001, FR-NEW-033, constraint 3.3. **Rationale:** same observable result in both modes. **Alternatives considered:** parsing the converter's `images.md` (internal format, no failure marker). **Source:** design. **Evidence:** `docs/service.rs:60-70`, `:139-227`, `:278-315`, `config.rs:765-794`, `:1063`.
### Data model
No schema change in this story; it reads `doc_artifact_current`, `doc_artifact_sets`, `doc_images`, `doc_tables` (design Section 4).
### Contracts
### 5.1 MCP tools (5 new, contract 102 to 107; all read only, idempotent, not open world)

| Tool | Parameters | Result |
|---|---|---|
| `fs.list_images` | `mount_id`, `path`, `marker` (optional) | `{"images":[{"id","caption","page"}],"marker":<string or null>}` |
| `fs.get_image` | `mount_id`, `path`, `id` | `{"id","format","caption","page","base64"}` |
| `fs.list_tables` | `mount_id`, `path`, `marker` (optional) | `{"tables":[{"id","caption","rows","columns","csv_quality"}],"marker":...}` |
| `fs.get_table` | `mount_id`, `path`, `id`, `format` (optional, default `markdown`) | `{"id","format","content"}` |
| `fs.get_document_view` | `mount_id`, `path`, `captions` (bool, default true), `table_mode` (default `markdown`) | `{"path","text"}` |

`csv_quality` carries the full label `CSV quality: exact` or `CSV quality: approximate`. `page` is
JSON `null` when empty. Page size is the constant `ARTIFACT_PAGE_SIZE = 100`.

### 5.2 REST routes (same functions, DEC-001)

`GET /api/fs/{mount_id}/images?path=&marker=`, `GET .../image?path=&id=`,
`GET .../tables?path=&marker=`, `GET .../table?path=&id=&format=`,
`GET .../document-view?path=&captions=&table_mode=`. Each added to `REST_ROUTES`
(`api/dataplane.rs:60-90`) and the OpenAPI `Op` table (pattern `api/openapi.rs:1016-1025`).

### 5.3 Conversion report (FR-NEW-030, FR-MOD-002)

A new key `artifacts_report` (string, lines joined by `\n`) is added to the result of
`fs.extract_text` (only when the sibling is rewritten), `fs.documentize`, and inside the
`documentation` object of `fs.write_bytes` and the REST upload. Every existing key is unchanged.
First line `<N> images, <M> tables`, then `<id>: <reason>` lines, images by id then tables by id.

### 5.4 Errors (exact)

| Message | Code | Where |
|---|---|---|
| `No artifacts: document not extracted` | `ERR_NOT_FOUND` | no current set, or a stale rev |
| `Not found: <id>` | `ERR_NOT_FOUND` | unknown `image-n` or `table-n` |
| `not a file: <path>` | `ERR_NOT_FOUND` | path missing or a folder (wording of `extract.rs:183`) |
| `Unsupported format: <v> (use markdown or csv)` | `ERR_INVALID_ARGUMENT` | `fs.get_table` |
| `Unsupported table mode: <v> (use markdown, csv-reference or both)` | `ERR_INVALID_ARGUMENT` | view |
| `Invalid continuation marker: <v>` | `ERR_INVALID_ARGUMENT` | list tools |
| `Document changed during conversion: <path>` | `ERR_INVALID_ARGUMENT` | capture commit |
| `session write quota of <N> bytes exceeded` | `ERR_WRITE_QUOTA_EXCEEDED` (existing) | capture |

Check order (FR-NEW-031): `authorize`, then the path is a file, then argument validation (format,
mode, marker), then the current set with a matching rev, then the id lookup.

### 5.5 Converter contract (DEC-006, DEC-007)

`DocService` gains `async fn convert(&self, bytes: &[u8], file_name: &str) -> Result<Conversion>`
with `Conversion { markdown: String, files: BTreeMap<String, Vec<u8>>, manifest: Option<Manifest> }`;
`to_markdown` stays as a thin wrapper (`convert(..).markdown`) for existing callers.

- CLI mode: `doc_service.cli.command` accepts an optional `{outdir}` placeholder (0 or 1, beside the
  required single `{document}`; validation at `config.rs:1063`). With `{outdir}`, the service passes
  a fresh empty directory inside the per call sandbox, reads `{outdir}/document.md` as the markdown,
  every regular file under `{outdir}` (relative paths, symlinks not followed, total size capped by
  `max_input_bytes`) into `files`, and `{outdir}/artifacts.json` as the manifest. Without it, stdout
  as today and no files. New default command: `doc-convert --quiet -o {outdir} {document}`.
- API mode: a response with `Content-Type: application/json` is a bundle
  `{"markdown": "...", "files": {"figures/f1.png": "<base64>"}, "manifest": {...}}`; any other
  content type is today's Markdown body.
- Manifest (optional): `{"pictures": {"<relative target>": {"caption": "...", "caption_failed": bool,
  "page": 3}}}`. Absent fields mean an empty caption, no failure marker, an empty page.

### 5.6 Built-in conversion (DEC-009)

`docs::extract::extract_text` keeps its result shape and additionally returns, internally, a
`Vec<BuiltinTable { caption, cells: Vec<Vec<String>>, line_start, line_count }>` built from the grid
before `md_table` (`extract.rs:288`), uncapped (the 400 row cap at `extract.rs:49`, `:616`, `:912`,
`:1040` keeps applying to the rendered sibling only), Excel captions from the sheet name
(`extract.rs:805`), blank Excel rows dropped as today (`extract.rs:909`).

### 5.7 Signatures

```rust
// docs/artifacts/capture.rs
pub(crate) async fn capture(ctx: CaptureCtx<'_>, source: CaptureSource) -> Result<Report>;
// tools/artifacts.rs
pub(crate) async fn list_images(state: &AppState, mount_id: &str, path: &str, marker: Option<&str>) -> Result<Value>;
pub(crate) async fn get_image(state: &AppState, mount_id: &str, path: &str, id: &str) -> Result<Value>;
pub(crate) async fn list_tables(state: &AppState, mount_id: &str, path: &str, marker: Option<&str>) -> Result<Value>;
pub(crate) async fn get_table(state: &AppState, mount_id: &str, path: &str, id: &str, format: Option<&str>) -> Result<Value>;
pub(crate) async fn document_view(state: &AppState, mount_id: &str, path: &str, captions: bool, table_mode: Option<&str>) -> Result<Value>;
```

`CaptureSource` is `Converter(Conversion)` or `Builtin { text, tables, truncated_at: Option<usize> }`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/core/fs_ops.rs` | modify: documented write outcome with quota message |
| `crates/core/src/docs/artifacts/capture.rs` | modify: built-in set with no images |
| `crates/core/src/tools/artifacts.rs` | modify: e2e tests |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-002
- Category: happy; scenario: SC-001; requirements: FR-NEW-017, FR-NEW-001, FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured and `budget.xlsx` has 2 sheets of one table each. When Alice extracts its text. Then the result reports `0 images, 2 tables`, each table labelled `CSV quality: exact`. And listing images returns an empty list.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-003
- Category: happy; scenario: SC-001; requirements: FR-NEW-017
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter accepts only `.pdf` and `budget.xlsx` holds 2 tables. When Alice extracts the text of `budget.xlsx`. Then 2 tables are listed, each `CSV quality: exact`. And listing images returns an empty list.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-004
- Category: failure; scenario: SC-001; requirements: FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given Alice has written 250 000 bytes this session and the artifacts, Markdown sibling and conversion text of `report.pdf` weigh 50 000 bytes together. When she converts it. Then she receives `session write quota of 262144 bytes exceeded`. And listing images answers `No artifacts: document not extracted`, and `report.md` does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-009
- Category: failure; scenario: SC-001; requirements: FR-NEW-017
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured and `scan.pdf` holds 3 images and 1 table. When Alice extracts its text. Then listing images returns an empty list. And listing tables returns an empty list.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-012
- Category: failure; scenario: SC-001; requirements: FR-NEW-014
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given Alice has written 0 bytes this session, `report.pdf` weighs 200 000 bytes, and its artifacts, Markdown sibling and conversion text weigh 100 000 bytes together. When she uploads `report.pdf` requesting documentation. Then the upload succeeds and `report.pdf` exists. And the documentation outcome reads `session write quota of 262144 bytes exceeded`, `report.md` does not exist, and listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-013
- Category: failure; scenario: SC-001; requirements: FR-NEW-016
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` is in Atlas. When Olga converts it. Then she receives the refusal she receives today for any operation on Atlas. And listing its images as Alice answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-015
- Category: edge; scenario: SC-001; requirements: FR-NEW-001, FR-NEW-003, FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `plain.pdf` holds text only. When Alice converts it. Then the result reports `0 images, 0 tables`. And listing images and listing tables both return empty lists.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-016
- Category: edge; scenario: SC-001; requirements: FR-NEW-017, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter accepts only `.pdf` and `photo.png` is an image. When Alice extracts the text of `photo.png`. Then the result reports `0 images, 0 tables`. And listing its images returns an empty list.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-017
- Category: edge; scenario: SC-001; requirements: FR-NEW-014
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the upload of E2E-012 was done. When Alice writes a 62 144 byte file `notes.txt`. Then the write succeeds, only the 200 000 bytes of `report.pdf` having been charged.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-018
- Category: edge; scenario: SC-001; requirements: FR-NEW-027, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `fresh.pdf` (1 image) was never converted and has no Markdown sibling. When Alice uses the convert action without overwrite. Then the result reports `1 images, 0 tables`. And listing its images shows `image-1`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-100
- Category: edge; scenario: SC-001; requirements: FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured and `blank.png` yields no text. When Alice extracts its text. Then the result reports `0 images, 0 tables` and `blank.md` does not exist. And listing its images returns an empty list.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-111
- Category: failure; scenario: SC-001; requirements: FR-NEW-018
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter accepts only `.pdf`. When Alice uses the convert action on `photo.png`. Then she receives `'/photo.png' cannot be documented, the document service accepts: .pdf`. And listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-124
- Category: side effect; scenario: SC-001; requirements: FR-NEW-030, FR-MOD-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` is in Atlas. When Alice uses the convert action. Then the result carries the same items as today, the Markdown sibling path among them. And it also carries the report `2 images, 2 tables`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-125
- Category: side effect; scenario: SC-001; requirements: FR-NEW-030, FR-MOD-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` is in Atlas. When Alice uploads it requesting documentation. Then the documentation outcome carries the Markdown sibling path as today. And it carries the report `2 images, 2 tables`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: view (US-0008).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0007: Images through every conversion path`, body `Spec: SPEC-0019` and the requirement ids.
