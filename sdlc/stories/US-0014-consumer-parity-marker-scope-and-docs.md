# US-0014: Consumer parity, marker scope and docs

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0013
> min_tier: 1
> Files touched: 6
> Closes spec: yes

## Objective
Closes SPEC-0019: proves AI assistants and HTTP API consumers get byte identical results, that continuation markers are scoped to project, kind and path but not to the person, and updates the documentation.

## Requirements covered
- SPEC-0019/FR-NEW-003: WHEN a member lists a document's images THE Documents context SHALL return, in reading order, each image's id, caption and page, 100 per page.
- SPEC-0019/FR-NEW-020: The Documents context SHALL give byte identical results to AI assistants and HTTP API consumers.

## Independent Test
`make check`; then call each of the five artifact operations through MCP and through REST on the same converted document and compare the payloads byte for byte.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0019): Five new tools and five REST routes, one implementation in `tools/artifacts.rs` called by both. **Obliged by:** SPEC-0019/FR-NEW-003, 004, 005, 006, 007, 020. **Rationale:** the project rule "one engine, two doors". **Alternatives considered:** one generic `fs.artifacts` tool with a `kind` argument (harder for an LLM, no gain). **Source:** constitution, AGENTS.md. **Evidence:** `api/dataplane.rs:634-650`, `tools/export.rs:5-7`.
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
| `crates/core/src/tools/artifacts.rs` | modify: parity and marker e2e tests |
| `README.md, AGENTS.md` | modify: 107 tools, artifact tools, `{outdir}` |
| `.agent_docs/tools.md, .agent_docs/api.md, .agent_docs/architecture.md` | modify: tools, routes, artifact store, rev token |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-033
- Category: side effect; scenario: SC-002; requirements: FR-NEW-020
- Level / Driver / Location: functional e2e / McpServer tool calls and the axum harness (pattern `api/dataplane.rs:2721-2820`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice lists images and gets `image-1` once as an AI assistant and once as an HTTP API consumer. Then ids, captions, pages, picture bytes and format are identical.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-049
- Category: side effect; scenario: SC-003; requirements: FR-NEW-020
- Level / Driver / Location: functional e2e / McpServer tool calls and the axum harness (pattern `api/dataplane.rs:2721-2820`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets `table-1` as `markdown` and as `csv`, once as an AI assistant and once as an HTTP API consumer. Then both pairs are byte identical.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-067
- Category: side effect; scenario: SC-004; requirements: FR-NEW-020
- Level / Driver / Location: functional e2e / McpServer tool calls and the axum harness (pattern `api/dataplane.rs:2721-2820`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice gets the full document in `both` mode once as an AI assistant and once as an HTTP API consumer. Then the two texts are byte identical.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-155
- Category: edge; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `atlas2.pdf` (101 images) was converted and Alice holds the page 1 marker. When Bob lists its images with that marker. Then he gets only `image-101`, with no marker.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-165
- Category: failure; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given project `Borealis`, where Alice is a member, holds a converted `/atlas2.pdf` with 101 images, and Alice holds the page 1 marker of `/atlas2.pdf` in Atlas. When she lists the images of `/atlas2.pdf` in Borealis with that marker. Then she receives `Invalid continuation marker: <that marker>`, the marker spelled as issued.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-166
- Category: edge; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the marker of E2E-165. When Alice uses it in Atlas on `/atlas2.pdf`. Then she gets only `image-101`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-167
- Category: failure; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the marker of E2E-165. When Alice lists the images of `/big.pdf` in Borealis with it. Then she receives `Invalid continuation marker: <that marker>`, the marker spelled as issued.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: nothing: closes the spec.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: README.md, AGENTS.md, .agent_docs/tools.md, .agent_docs/api.md, .agent_docs/architecture.md.
- [ ] Self review: full.
- [ ] Closes spec only: the spec's full acceptance set green (every E2E-001 to E2E-167), and every requirement of SPEC-0019 cited by at least one story with status done or landed.
- [ ] One commit `US-0014: Consumer parity, marker scope and docs`, body `Spec: SPEC-0019` and the requirement ids.
