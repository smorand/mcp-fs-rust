# US-0008: Full document view

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0007
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
A member gets the whole document with image lines and tables in the chosen mode, built from the conversion text.

## Requirements covered
- SPEC-0019/FR-NEW-007: WHEN a member gets the full document THE Documents context SHALL return the conversion text with an image line at each image's reading position and each table rendered in the chosen table mode at its reading position.
- SPEC-0019/FR-NEW-008: WHEN a member gets the full document with captions turned off THE Documents context SHALL render every image line as `[Image image-<n>]`.
- SPEC-0019/FR-NEW-022: The Documents context SHALL keep producing the Markdown sibling exactly as before this spec.
- SPEC-0019/FR-NEW-025: IF a member requests a table mode other than `markdown`, `csv-reference` or `both` THEN THE Documents context SHALL refuse with `Unsupported table mode: <value> (use markdown, csv-reference or both)` and return no content.

## Independent Test
`make check`; then convert `report.pdf` with the fake converter and compare `fs.get_document_view` in `csv-reference` mode with the expected reference lines.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0019): Picture and conversion text bytes are content addressed blobs counted in `blob_refs`, incref on set commit, decref on set deletion, GC at zero. **Obliged by:** FR-NEW-004, FR-NEW-009. **Rationale:** reuses layout, refcount and GC. **Alternatives considered:** BLOB columns (three dialect typings, large rows). **Source:** existing code. **Evidence:** `storage/meta.rs:140-148`, `:436`, `:455`.
- DEC-009 (SPEC-0019): Built-in conversion exposes uncapped grids with line spans; converter text is parsed by `parse.rs` with the FR-NEW-033 rules (picture reference line, pipe table block, fenced code). **Obliged by:** FR-NEW-002, FR-NEW-017, FR-NEW-033, FR-NEW-034. **Rationale:** exact CSV needs the grid; converter tables only exist as Markdown. **Alternatives considered:** parsing the built-in Markdown too (loses rows past 400 and escapes). **Source:** existing code. **Evidence:** `docs/extract.rs:288-312`, `:614-617`, `:905-913`, `:1038-1041`.
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
| `crates/core/src/docs/artifacts/render.rs` | modify: view assembly, table modes, captions |
| `crates/core/src/tools/artifacts.rs` | modify: `document_view` |
| `crates/core/src/mcp/server.rs` | modify: `fs.get_document_view`, counts 106 to 107 |
| `crates/core/src/api/dataplane.rs` | modify: `GET document-view` |
| `crates/core/src/api/openapi.rs` | modify: one `Op` entry |
| `TOOL_CONTRACT.txt, tool-contract-golden.json, crates/core/src/tools/contract_golden.rs` | regenerate (`MCPFS_REWRITE_TOOL_CONTRACT=1`) and bump the five count assertions; one mechanical unit |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-050
- Category: happy; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document with default options. Then the line `[Image image-1: Figure 1: Revenue 2024]` stands at the first image's place. And the 5 lines of E2E-035 stand right after the line `Table 1: Q1 sales`, and the 4 lines of E2E-048 right after `Table 2: Costs`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-051
- Category: happy; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document with table mode `csv-reference`. Then the first table reads exactly `[Table table-1: Q1 sales (CSV)]`. And no Markdown table appears.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-052
- Category: happy; scenario: SC-004; requirements: FR-NEW-007, FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document with table mode `both` and captions off. Then each Markdown table is followed by its reference line, the first being `[Table table-1: Q1 sales (CSV)]`. And the image lines read `[Image image-1]` and `[Image image-2]`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-053
- Category: happy; scenario: SC-004; requirements: FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document with captions off in `markdown` mode. Then the text holds the lines `[Image image-1]` and `[Image image-2]`, and neither `Figure 1: Revenue 2024` nor `Schéma réseau 🌐` appears. And both tables appear as Markdown tables.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-054
- Category: failure; scenario: SC-004; requirements: FR-NEW-025
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice asks for table mode `html`. Then she receives `Unsupported table mode: html (use markdown, csv-reference or both)`. And no document text is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-055
- Category: failure; scenario: SC-004; requirements: FR-NEW-025
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice asks for table mode `CSV-REFERENCE`. Then she receives `Unsupported table mode: CSV-REFERENCE (use markdown, csv-reference or both)`. And no document text is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-056
- Category: failure; scenario: SC-004; requirements: FR-NEW-025
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice asks for table mode `markdown,both`. Then she receives `Unsupported table mode: markdown,both (use markdown, csv-reference or both)`. And no document text is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-057
- Category: failure; scenario: SC-004; requirements: FR-NEW-023
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `notes.pdf` was never converted. When Alice asks for its full document view. Then she receives `No artifacts: document not extracted`. And no `notes.md` is created.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-058
- Category: failure; scenario: SC-004; requirements: FR-NEW-016
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Olga asks for its full document view in `both` mode. Then she receives the refusal she receives today for any operation on Atlas. And no text is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-059
- Category: failure; scenario: SC-004; requirements: FR-NEW-009, FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, then Alice deleted `report.md`, then overwrote `report.pdf`. When Alice asks for its full document view. Then she receives `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-060
- Category: edge; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report3.pdf`, with tables `Q1 sales`, `Costs` and a third table with no caption, was converted. When Alice gets the full document in `csv-reference`. Then the third table reads exactly `[Table table-3 (CSV)]`. And the first reads `[Table table-1: Q1 sales (CSV)]`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-061
- Category: edge; scenario: SC-004; requirements: FR-NEW-008
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `plain.pdf` (no image) was converted. When Alice gets its full document with captions off, then with captions on. Then both texts are byte identical.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-062
- Category: edge; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `one.pdf` (1 image, no caption) was converted. When Alice gets its full document with default options. Then the text holds the line `[Image image-1]`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-063
- Category: edge; scenario: SC-004; requirements: FR-NEW-007
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document with default options. Then the line `[Image image-1: Figure 1: Revenue 2024]` appears exactly once, and `[Image image-2: Schéma réseau 🌐]` exactly once after it. And each stands between empty lines.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-068
- Category: side effect; scenario: SC-004; requirements: FR-NEW-022
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document in each of the three table modes, then reads `report.md`. Then `report.md` is byte identical to its content before the views.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-116
- Category: side effect; scenario: SC-005; requirements: FR-NEW-030, FR-MOD-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice extracts its text with refresh. Then the result reports `0 images, 0 tables`. And the full document view in `markdown` mode with captions on equals the content of `report.md` byte for byte.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-119
- Category: failure; scenario: SC-004; requirements: FR-NEW-031
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no file `ghost.pdf` exists. When Alice asks for the full document of `ghost.pdf` with mode `html`. Then she receives `not a file: /ghost.pdf`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-132
- Category: edge; scenario: SC-004; requirements: FR-NEW-034
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `two.xlsx` was extracted as in E2E-130. When Alice gets its full document. Then the text ends where the cut text ends, and `table-1` appears once.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: view line rules (US-0009).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0008: Full document view`, body `Spec: SPEC-0019` and the requirement ids.
