# US-0012: Changed documents lose their artifacts

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0011
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Any change to a converted document, by any operation, must make its artifacts go away; a copy starts with none; an unchanged document keeps them, including across a version control checkout of an identical file.

## Requirements covered
- SPEC-0019/FR-NEW-011: WHEN a converted document is moved, renamed, deleted or has its bytes replaced, by any operation, THE Documents context SHALL discard its artifacts and its conversion text.
- SPEC-0019/FR-NEW-012: WHEN a member copies a converted document THE Documents context SHALL give the copy no artifacts and keep the source's artifacts.

## Independent Test
`make check`; then convert `report.pdf`, move it to `b.pdf` and back, and observe `No artifacts: document not extracted`.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0019): Validity token `nodes.rev`, a uuid v4 stamped on every node insert, path change and byte write (not on atime or mtime only touches); a set is served only while `set.node_rev = nodes.rev`. **Obliged by:** FR-NEW-011, FR-NEW-012, FR-NEW-035. **Rationale:** exact for a same bytes rewrite and a move away and back; one choke point. **Alternatives considered:** sha256 or mtime key (approach B), hooks (approach C). **Source:** design. **Evidence:** `storage/meta.rs:927-975`, `:1169`, `:1189-1221`.
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
| `crates/core/src/storage/meta.rs` | modify: `rev` stamping verified on every node write path |
| `crates/core/src/tools/git.rs` | modify only if needed: whole tree checkout skips blobs equal to the node's sha |
| `crates/core/src/tools/artifacts.rs` | modify: e2e tests per change path |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-086
- Category: happy; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice reads `report.pdf`, lists its folder and reads `report.md`. Then listing its images still returns `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-087
- Category: happy; scenario: SC-006; requirements: FR-NEW-011, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and moved to `archive/r.pdf`. When Alice converts `archive/r.pdf`. Then `archive/r.pdf` lists `image-1` and `image-2`. And before that conversion it answered `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-088
- Category: failure; scenario: SC-006; requirements: FR-NEW-011, FR-NEW-028
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice moves it to `b.pdf`, then lists the images of `b.pdf` and of `report.pdf`. Then `b.pdf` answers `No artifacts: document not extracted`. And `report.pdf` answers `not a file: /report.pdf`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-089
- Category: failure; scenario: SC-006; requirements: FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice copies it to `copy.pdf`. Then `copy.pdf` answers `No artifacts: document not extracted`. And `report.pdf` still lists `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-090
- Category: failure; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice deletes it to the trash, then restores it. Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-091
- Category: failure; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice writes the same bytes again to `report.pdf`. Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-092
- Category: failure; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given converted `report.pdf` is tracked in version control and a remote commit changes it. When Alice pulls. Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-093
- Category: failure; scenario: SC-006; requirements: FR-NEW-011, FR-NEW-012
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and `notes.pdf` never was. When Alice copies `notes.pdf` over `report.pdf` with overwrite. Then `report.pdf` answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-094
- Category: failure; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice extracts an archive in the same folder that holds a `report.pdf`, replacing it. Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-095
- Category: edge; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given folder `q1/` holds converted `report.pdf`. When Alice moves `q1/` to `archive/`. Then `archive/report.pdf` answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-096
- Category: edge; scenario: SC-006; requirements: FR-NEW-011, FR-NEW-022
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice rewrites `report.md` with the line `edited`. Then listing the images of `report.pdf` still returns `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-097
- Category: edge; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice moves it to `b.pdf` and back to `report.pdf`. Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-108
- Category: failure; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice edits `report.pdf` in place (any successful edit). Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-109
- Category: failure; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` is tracked in version control, its committed version differs from the current one, and the current one was converted. When Alice reverts `report.pdf` to its committed version. Then listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-137
- Category: edge; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` in Atlas was converted. When an admin deletes the project Atlas and then restores it. Then Alice listing its images gets `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-141
- Category: edge; scenario: SC-006; requirements: FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given converted `report.pdf` is tracked and identical on branches `main` and `dev`, and `other.txt` differs between them. When Alice checks out `dev`. Then listing the images of `report.pdf` returns `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: not files and sweep (US-0013).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0012: Changed documents lose their artifacts`, body `Spec: SPEC-0019` and the requirement ids.
