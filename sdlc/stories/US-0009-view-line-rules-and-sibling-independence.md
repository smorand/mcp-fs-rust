# US-0009: View line rules and sibling independence

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0008
> min_tier: 2
> Files touched: 2
> Closes spec: no

## Objective
The view must replace exactly the right lines, normalize empty lines around image lines, leave references it did not keep untouched, and ignore later edits to the Markdown sibling.

## Requirements covered
- SPEC-0019/FR-NEW-009: WHEN a member gets the full document after the Markdown sibling was rewritten or deleted THE Documents context SHALL build the view from the conversion text.
- SPEC-0019/FR-NEW-036: WHEN the Documents context places an image line in a view THE Documents context SHALL remove the picture reference line, the matched caption line (FR-NEW-007) and every empty line directly before, between or after those removed lines, then write exactly one empty line before and one after the image line.
- SPEC-0019/FR-NEW-037: WHEN a picture reference yields no kept image (picture unavailable or unsupported picture format) THE Documents context SHALL leave its line in the view exactly as written in the conversion text and render no image line for its id.

## Independent Test
`make check`; then convert `fig.pdf`, delete `report.md`-style sibling, call `fs.get_document_view` and compare the text with the exact 5 expected lines.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0019): Picture and conversion text bytes are content addressed blobs counted in `blob_refs`, incref on set commit, decref on set deletion, GC at zero. **Obliged by:** FR-NEW-004, FR-NEW-009. **Rationale:** reuses layout, refcount and GC. **Alternatives considered:** BLOB columns (three dialect typings, large rows). **Source:** existing code. **Evidence:** `storage/meta.rs:140-148`, `:436`, `:455`.
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
| `crates/core/src/docs/artifacts/render.rs` | modify: line replacement and empty line rule |
| `crates/core/src/tools/artifacts.rs` | modify: e2e tests |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-064
- Category: edge; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document with default options. Then no image reference or figure path from the conversion text remains. And the text outside image and table positions equals the conversion text.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-065
- Category: edge; scenario: SC-004; requirements: FR-NEW-009
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, then Alice replaced `report.md` with the single line `edited`. When Alice gets the full document with default options. Then the text contains `[Image image-1: Figure 1: Revenue 2024]`. And it does not contain the line `edited`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-066
- Category: edge; scenario: SC-004; requirements: FR-NEW-009
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, then Alice deleted `report.md`. When Alice gets the full document in `csv-reference` mode. Then the text contains `[Table table-1: Q1 sales (CSV)]`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-139
- Category: edge; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `fig.pdf` the lines `Intro`, the picture reference, `Figure 1: Revenue 2024`, `Body text`, and describes the picture `Figure 1: Revenue 2024`. When Alice converts it and gets the full document with default options. Then the text reads exactly `Intro`, an empty line, `[Image image-1: Figure 1: Revenue 2024]`, an empty line, `Body text`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-144
- Category: edge; scenario: SC-001; requirements: FR-NEW-033, FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `inline.pdf` the line `See ![chart](figures/figure_1.png) here` and that picture. When Alice converts it and gets the full document with default options. Then the result reports `0 images, 0 tables`. And the view holds the line `See ![chart](figures/figure_1.png) here` unchanged.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-159
- Category: edge; scenario: SC-004; requirements: FR-NEW-036, FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `gap.pdf` the lines `Intro`, an empty line, the picture reference, an empty line, `Figure 1: Revenue 2024`, an empty line, `Body text`, and describes the picture `Figure 1: Revenue 2024`. When Alice converts it and gets the full document with default options. Then the text is exactly the 5 lines `Intro`, an empty line, `[Image image-1: Figure 1: Revenue 2024]`, an empty line, `Body text`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-160
- Category: edge; scenario: SC-004; requirements: FR-NEW-036
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `top.pdf` the picture reference as its first line, then an empty line, then `Body text`, with no description. When Alice converts it and gets the full document. Then the text is exactly the 3 lines `[Image image-1]`, an empty line, `Body text`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-161
- Category: edge; scenario: SC-004; requirements: FR-NEW-036
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `end.pdf` the lines `Intro`, two empty lines, the picture reference, with no description. When Alice converts it and gets the full document. Then the text is exactly the 3 lines `Intro`, an empty line, `[Image image-1]`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-162
- Category: failure; scenario: SC-004; requirements: FR-NEW-037
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `trav.pdf` as in E2E-151 was converted. When Alice gets its full document with default options. Then the text holds the line `![x](../../secret.png)` unchanged and the line `[Image image-2]`. And it holds no line starting with `[Image image-1`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-163
- Category: failure; scenario: SC-004; requirements: FR-NEW-037
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `mixed.pdf` as in E2E-008 was converted. When Alice gets its full document. Then the reference line of its `.svg` figure stands unchanged. And no line starting with `[Image image-2` appears.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-164
- Category: edge; scenario: SC-004; requirements: FR-NEW-037
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `url.pdf` as in E2E-152 was converted. When Alice gets its full document with captions off. Then the text holds `![x](https://example.com/a.png)` unchanged.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: re-conversion (US-0010).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0009: View line rules and sibling independence`, body `Spec: SPEC-0019` and the requirement ids.
