# US-0004: Table refusals and table routes

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0003
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Table requests must refuse in a fixed order with fixed categories, and the two table operations must be reachable over the REST plane through the same functions.

## Requirements covered
- SPEC-0019/FR-NEW-019: IF a member requests an artifact id the document does not have THEN THE Documents context SHALL answer `Not found: <id>`.
- SPEC-0019/FR-NEW-024: IF a member requests a table format other than `markdown` or `csv` THEN THE Documents context SHALL refuse with `Unsupported format: <value> (use markdown or csv)` and return no content.
- SPEC-0019/FR-NEW-028: IF a member requests artifacts at a path that holds no file THEN THE Documents context SHALL answer `not a file: <path>`.
- SPEC-0019/FR-NEW-031: The Documents context SHALL check an artifact request in this order and answer the first refusal that applies: the non member refusal; `not a file: <path>`; `Unsupported format: <value> (use markdown or csv)`, `Unsupported table mode: <value> (use markdown, csv-reference or both)` or `Invalid continuation marker: <value>`; `No artifacts: document not extracted`; `Not found: <id>`.
- SPEC-0019/FR-NEW-032: The Documents context SHALL class `Not found: <id>`, `not a file: <path>` and `No artifacts: document not extracted` as not found refusals, and `Unsupported format: <value> (use markdown or csv)`, `Unsupported table mode: <value> (use markdown, csv-reference or both)` and `Invalid continuation marker: <value>` and `Document changed during conversion: <path>` as invalid argument refusals.

## Independent Test
`make check`; then `GET /api/fs/{mount}/table?path=/report.pdf&id=table-9&format=xml` answers `ERR_INVALID_ARGUMENT` `Unsupported format: xml (use markdown or csv)`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0019): Five new tools and five REST routes, one implementation in `tools/artifacts.rs` called by both. **Obliged by:** SPEC-0019/FR-NEW-003, 004, 005, 006, 007, 020. **Rationale:** the project rule "one engine, two doors". **Alternatives considered:** one generic `fs.artifacts` tool with a `kind` argument (harder for an LLM, no gain). **Source:** constitution, AGENTS.md. **Evidence:** `api/dataplane.rs:634-650`, `tools/export.rs:5-7`.
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
| `crates/core/src/tools/artifacts.rs` | modify: refusal order, categories |
| `crates/core/src/api/dataplane.rs` | modify: `GET tables`, `GET table`, `REST_ROUTES` |
| `crates/core/src/api/openapi.rs` | modify: two `Op` entries |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-038
- Category: failure; scenario: SC-003; requirements: FR-NEW-016
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Olga lists its tables, then gets `table-1` as `markdown` and as `csv`. Then each answer is the refusal she receives today for any operation on Atlas.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-042
- Category: failure; scenario: SC-003; requirements: FR-NEW-023
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `notes.pdf` was never converted. When Alice lists its tables, then gets `table-1` as `csv`. Then both answers are `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-105
- Category: failure; scenario: SC-002; requirements: FR-NEW-028
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no file `ghost.pdf` exists. When Alice lists the tables of `ghost.pdf`. Then she receives `not a file: /ghost.pdf`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-117
- Category: failure; scenario: SC-003; requirements: FR-NEW-031
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `notes.pdf` was never converted. When Alice gets `table-9` of `notes.pdf` as `xml`. Then she receives `Unsupported format: xml (use markdown or csv)`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-118
- Category: failure; scenario: SC-003; requirements: FR-NEW-031
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-9` as `xml`. Then she receives `Unsupported format: xml (use markdown or csv)`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-121
- Category: failure; scenario: SC-003; requirements: FR-NEW-032
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` as `xml`. Then the refusal carries the invalid argument category.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-142
- Category: edge; scenario: SC-003; requirements: FR-NEW-002
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `esc.pdf` the pipe table `| k | v |`, `| --- | --- |`, `|  a\|b  | **x** |`. When Alice converts it and gets `table-1` as `csv`, then as `markdown`. Then the CSV is exactly the 2 lines `k,v` and `a|b,**x**`, separated by one LF. And the Markdown's last line is exactly `| a\|b | **x** |`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: images (US-0005).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0004: Table refusals and table routes`, body `Spec: SPEC-0019` and the requirement ids.
