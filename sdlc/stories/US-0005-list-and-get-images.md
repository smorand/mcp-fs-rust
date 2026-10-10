# US-0005: List and get images

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0004
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
Pictures the converter extracts are thrown away today; this story keeps them with caption, page and format, lets a member list and get them through MCP and REST, and pages lists with scoped continuation markers.

## Requirements covered
- SPEC-0019/FR-NEW-003: WHEN a member lists a document's images THE Documents context SHALL return, in reading order, each image's id, caption and page, 100 per page.
- SPEC-0019/FR-NEW-004: WHEN a member gets an image by id THE Documents context SHALL return its picture, byte identical to what the converter extracted, its format and its caption.

## Independent Test
`make check`; then convert `report.pdf` through the fake converter, call `fs.get_image` `image-2`, decode the base64 and compare it byte for byte with the bytes the fake converter returned.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0019): Picture and conversion text bytes are content addressed blobs counted in `blob_refs`, incref on set commit, decref on set deletion, GC at zero. **Obliged by:** FR-NEW-004, FR-NEW-009. **Rationale:** reuses layout, refcount and GC. **Alternatives considered:** BLOB columns (three dialect typings, large rows). **Source:** existing code. **Evidence:** `storage/meta.rs:140-148`, `:436`, `:455`.
- DEC-006 (SPEC-0019): `DocService::convert` returns a `Conversion` bundle; CLI `{outdir}` placeholder and API JSON bundle; optional `artifacts.json` manifest; `to_markdown` kept as a wrapper. **Obliged by:** FR-NEW-001, FR-NEW-033, constraint 3.3. **Rationale:** same observable result in both modes. **Alternatives considered:** parsing the converter's `images.md` (internal format, no failure marker). **Source:** design. **Evidence:** `docs/service.rs:60-70`, `:139-227`, `:278-315`, `config.rs:765-794`, `:1063`.
- DEC-007 (SPEC-0019): Picture targets resolve only to keys of `Conversion.files` after normalizing the relative path; URLs, absolute paths and `..` escapes are `picture unavailable`; the filesystem is never read by target. **Obliged by:** FR-NEW-015. **Rationale:** traversal impossible by construction. **Alternatives considered:** canonicalize then prefix check on disk (TOCTOU). **Source:** design. **Evidence:** n/a.
- DEC-010 (SPEC-0019): Continuation marker = base64url (no pad) of `v1\n<project>\n<kind>\n<path>\n<offset>`; any decode or field mismatch is invalid; an offset past the end gives an empty page with no marker. **Obliged by:** FR-NEW-003. **Rationale:** stateless, scoped, person independent. **Alternatives considered:** plain offset (cannot detect cross list use), a server side cursor table. **Source:** design. **Evidence:** offset paging precedent `tools/trash.rs:79-116`.
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
| `crates/core/src/docs/artifacts/capture.rs` | modify: pictures from `Conversion.files`, manifest captions and pages, formats |
| `crates/core/src/tools/artifacts.rs` | modify: `list_images`, `get_image` |
| `crates/core/src/mcp/server.rs` | modify: `fs.list_images`, `fs.get_image`, counts 104 to 106 |
| `crates/core/src/api/dataplane.rs` | modify: `GET images`, `GET image` |
| `crates/core/src/api/openapi.rs` | modify: two `Op` entries |
| `TOOL_CONTRACT.txt, tool-contract-golden.json, crates/core/src/tools/contract_golden.rs` | regenerate (`MCPFS_REWRITE_TOOL_CONTRACT=1`) and bump the five count assertions; one mechanical unit |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-001
- Category: happy; scenario: SC-001; requirements: FR-NEW-001, FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given Alice has `report.pdf` in Atlas. When she converts it. Then the result reports `2 images, 2 tables` and no missing item. And listing images shows `image-1` `Figure 1: Revenue 2024` page 1 and `image-2` `Schéma réseau 🌐` page 3.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-024
- Category: happy; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice lists its images. Then she gets exactly `image-1` then `image-2`, with captions and pages as in the shared data. And there is no continuation marker.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-025
- Category: happy; scenario: SC-002; requirements: FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `image-2`. Then she receives the picture byte identical to the one the converter extracted, its format `png`, and caption `Schéma réseau 🌐`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-026
- Category: failure; scenario: SC-002; requirements: FR-NEW-019
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `image-7`. Then she receives `Not found: image-7`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-027
- Category: failure; scenario: SC-002; requirements: FR-NEW-016
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Olga lists its images, then gets `image-1`. Then both answers are the refusal she receives today for any operation on Atlas. And no caption or picture is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-028
- Category: failure; scenario: SC-002; requirements: FR-NEW-023
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `notes.pdf` was never converted. When Alice lists its images, then gets `image-1`. Then both answers are `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-029
- Category: edge; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `atlas.pdf` was converted with 100 images. When Alice lists its images. Then one page holds `image-1` to `image-100`. And there is no continuation marker.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-030
- Category: edge; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `atlas2.pdf` was converted with 101 images. When Alice lists its images and follows the marker. Then page 1 holds `image-1` to `image-100` with a marker. And page 2 holds only `image-101`, with no marker.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-031
- Category: edge; scenario: SC-002; requirements: FR-NEW-003, FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `one.pdf` has 1 image with no caption, on page 2, converted. When Alice lists its images, then gets `image-1`. Then the list has one entry, `image-1`, page 2, empty caption. And getting it returns the picture with an empty caption.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-032
- Category: edge; scenario: SC-002; requirements: FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `cap.pdf` was converted and its only image has caption `Coût | Été 😀 "v2"`. When Alice gets `image-1`. Then the caption is returned exactly `Coût | Été 😀 "v2"`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-102
- Category: edge; scenario: SC-001; requirements: FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `logo.pdf` shows the same logo on pages 1 and 2. When Alice converts it. Then listing images shows `image-1` page 1 and `image-2` page 2. And their pictures are byte identical.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-103
- Category: failure; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice lists its images with continuation marker `zzz`. Then she receives `Invalid continuation marker: zzz`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-104
- Category: failure; scenario: SC-002; requirements: FR-NEW-028
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given folder `q1/` exists. When Alice lists the images of `q1`. Then she receives `not a file: /q1`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-115
- Category: edge; scenario: SC-002; requirements: FR-NEW-001, FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `memo.docx` holds 1 image captioned `Org chart` and the converter accepts `.docx`. When Alice converts it and lists its images. Then she gets `image-1`, `Org chart`, with an empty page.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-120
- Category: failure; scenario: SC-002; requirements: FR-NEW-032
- Level / Driver / Location: functional e2e / McpServer tool calls and the axum harness (pattern `api/dataplane.rs:2721-2820`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `notes.pdf` was never converted. When Alice lists its images as an HTTP API consumer and as an AI assistant. Then both answers carry the not found category and the text `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-122
- Category: failure; scenario: SC-002; requirements: FR-NEW-032
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice lists its images with marker `zzz`. Then the refusal carries the invalid argument category.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-123
- Category: failure; scenario: SC-003; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `atlas2.pdf` (101 images) and `big.pdf` (101 tables) were converted. When Alice lists the tables of `big.pdf` with the marker from page 1 of the images of `atlas2.pdf`. Then she receives `Invalid continuation marker: <that marker>`, the marker spelled as issued.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-127
- Category: happy; scenario: SC-001; requirements: FR-NEW-033
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `cap2.pdf` a picture reference described `Revenue chart`, then a line `Table 1: Costs`, then a 2 row pipe table. When Alice converts it and lists its images and tables. Then `image-1` has caption `Revenue chart`. And `table-1` has caption `Costs` and 2 data rows.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: image failures (US-0006), conversion paths (US-0007), view (US-0008).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0005: List and get images`, body `Spec: SPEC-0019` and the requirement ids.
