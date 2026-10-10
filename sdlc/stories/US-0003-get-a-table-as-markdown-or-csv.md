# US-0003: Get a table as Markdown or CSV

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0002
> min_tier: 1
> Files touched: 4
> Closes spec: no

## Objective
A member can list tables but not read one; this story returns a table as Markdown or CSV with the exact byte rules of FR-NEW-002.

## Requirements covered
- SPEC-0019/FR-NEW-002: The Documents context SHALL offer every table as Markdown and as CSV, each table carrying a CSV quality label.
- SPEC-0019/FR-NEW-006: WHEN a member gets a table by id THE Documents context SHALL return it in the requested format, `markdown` when no format is given.

## Independent Test
`make check`; then extract `names.xlsx` and get `table-1` as `csv` through MCP; parse the CSV back with a CSV reader and compare with the original cells.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0019): Five new tools and five REST routes, one implementation in `tools/artifacts.rs` called by both. **Obliged by:** SPEC-0019/FR-NEW-003, 004, 005, 006, 007, 020. **Rationale:** the project rule "one engine, two doors". **Alternatives considered:** one generic `fs.artifacts` tool with a `kind` argument (harder for an LLM, no gain). **Source:** constitution, AGENTS.md. **Evidence:** `api/dataplane.rs:634-650`, `tools/export.rs:5-7`.
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
| `crates/core/src/docs/artifacts/render.rs` | create: Markdown and CSV rendering, cell normalization |
| `crates/core/src/tools/artifacts.rs` | modify: `get_table` |
| `crates/core/src/mcp/server.rs` | modify: `fs.get_table`, counts 103 to 104 |
| `TOOL_CONTRACT.txt, tool-contract-golden.json, crates/core/src/tools/contract_golden.rs` | regenerate (`MCPFS_REWRITE_TOOL_CONTRACT=1`) and bump the five count assertions; one mechanical unit |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-034
- Category: happy; scenario: SC-003; requirements: FR-NEW-005, FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice lists its tables. Then she gets `table-1` `Q1 sales` 3 rows 4 columns and `table-2` `Costs` 2 rows 2 columns. And each is labelled `CSV quality: approximate`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-035
- Category: happy; scenario: SC-003; requirements: FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` with no format. Then she receives exactly the 5 lines `| Region | Jan | Feb | Mar |`, `| --- | --- | --- | --- |`, `| North | 10 | 11 | 12 |`, `| South | 20 | 21 | 22 |`, `| West | 30 | 31 | 32 |`, with no trailing newline.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-036
- Category: happy; scenario: SC-003; requirements: FR-NEW-006, FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` as `csv`. Then she receives 4 lines (header and 3 rows) of 4 comma separated fields.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-037
- Category: failure; scenario: SC-003; requirements: FR-NEW-019
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-9`. Then she receives `Not found: table-9`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-039
- Category: failure; scenario: SC-003; requirements: FR-NEW-024
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` as `xml`. Then she receives `Unsupported format: xml (use markdown or csv)`. And no table content is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-040
- Category: failure; scenario: SC-003; requirements: FR-NEW-024
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` as `CSV`. Then she receives `Unsupported format: CSV (use markdown or csv)`. And no table content is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-041
- Category: failure; scenario: SC-003; requirements: FR-NEW-024
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` as `md`. Then she receives `Unsupported format: md (use markdown or csv)`. And no table content is returned.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-044
- Category: edge; scenario: SC-003; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the text of `names.xlsx` was extracted, holding one table with cells `a|b`, `Dupont, "Jr"` and `Zoë 🚀`. When Alice gets it as `markdown`, then as `csv`. Then the Markdown holds `a\|b` and keeps 3 columns. And the CSV holds `"Dupont, ""Jr"""` and `Zoë 🚀`, and reading it back gives the three original cells exactly.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-046
- Category: edge; scenario: SC-003; requirements: FR-NEW-005, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the text of `empty.xlsx` was extracted, holding one table with a header and 0 data rows. When Alice lists its tables, then gets `table-1` as `csv`. Then the list shows 0 rows. And the CSV is the header line only.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-047
- Category: edge; scenario: SC-003; requirements: FR-NEW-002, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured and `ledger.xlsx` holds one table of 401 data rows by 3 columns. When Alice extracts its text, lists its tables and gets `table-1` as `csv`. Then the list shows 401 rows. And the CSV has 402 lines (header and 401 rows).
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-048
- Category: edge; scenario: SC-003; requirements: FR-NEW-002, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-2` as `markdown`. Then she receives exactly 4 lines, `| Item | EUR |`, `| --- | --- |`, `| Rent | 900 |`, `| Power | 120 |`, with no trailing newline.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-112
- Category: edge; scenario: SC-003; requirements: FR-NEW-002, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-2` as `csv`. Then she receives exactly the 3 lines `Item,EUR`, `Rent,900`, `Power,120`, separated by a single LF, with no trailing newline. And no field is quoted.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-140
- Category: edge; scenario: SC-003; requirements: FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the text of `empty.xlsx` (header `A`, `B`, 0 data rows) was extracted. When Alice gets `table-1` as `markdown`. Then she receives exactly the 2 lines `| A | B |` and `| --- | --- |`, with no trailing newline.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-147
- Category: edge; scenario: SC-003; requirements: FR-NEW-002, FR-NEW-006
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `ledger.csv` holds a header and 401 data rows of 3 columns. When Alice extracts its text, lists its tables and gets `table-1` as `csv`. Then the list shows 401 rows. And the CSV has 402 lines.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-154
- Category: edge; scenario: SC-003; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `crlf.csv` holds header `k`, `v` and one row `x` and a `v` cell made of `a`, CR LF, `b`. When Alice extracts its text and gets `table-1` as `markdown`, then as `csv`. Then the Markdown's last line is exactly `| x | a b |`. And in the CSV the cell reads `"a`, CR LF, `b"`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-158
- Category: edge; scenario: SC-003; requirements: FR-NEW-017
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `gaps.xlsx` holds header `a`, `b`, a row `1`, `2`, a fully blank row, then a row `3`, `4`. When Alice extracts its text, lists its tables and gets `table-1` as `csv`. Then the list shows 2 data rows. And the CSV is exactly `a,b`, `1,2`, `3,4`, separated by one LF.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: table refusals and REST routes (US-0004).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0003: Get a table as Markdown or CSV`, body `Spec: SPEC-0019` and the requirement ids.
