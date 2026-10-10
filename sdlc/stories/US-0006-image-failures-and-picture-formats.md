# US-0006: Image failures and picture formats

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0005
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A conversion must report each image it could not keep, with the exact reason, never read a picture outside the converter's output, and keep everything else.

## Requirements covered
- SPEC-0019/FR-NEW-015: IF an image or a table the conversion text references fails THEN THE Documents context SHALL keep everything else and report each failed item as `<id>: <reason>`.

## Independent Test
`make check`; then convert `trav.pdf` whose text references `../../secret.png` with a real file `secret.png` placed one level above the sandbox; observe `image-1: picture unavailable` and that the stored blob set contains no blob with the secret's sha256.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0019): Picture targets resolve only to keys of `Conversion.files` after normalizing the relative path; URLs, absolute paths and `..` escapes are `picture unavailable`; the filesystem is never read by target. **Obliged by:** FR-NEW-015. **Rationale:** traversal impossible by construction. **Alternatives considered:** canonicalize then prefix check on disk (TOCTOU). **Source:** design. **Evidence:** n/a.
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
| `crates/core/src/docs/artifacts/capture.rs` | modify: failure reasons, target confinement, format list |
| `crates/core/src/docs/artifacts/parse.rs` | modify: picture reference targets |
| `crates/core/src/tools/artifacts.rs` | modify: e2e tests |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-005
- Category: failure; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `deck.pdf` has 5 images and the converter's output marks the caption of the 4th as failed. When Alice converts it. Then the result reports `5 images, 0 tables` and `image-4: caption unavailable`. And `image-4` is listed with an empty caption, images 1, 2, 3, 5 with theirs.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-007
- Category: failure; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `deck2.pdf` converts to text referencing 5 pictures and the 3rd referenced picture is missing from the converter's output. When Alice converts it. Then the result reports `4 images, 0 tables` and `image-3: picture unavailable`. And listing images shows `image-1`, `image-2`, `image-4`, `image-5`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-008
- Category: failure; scenario: SC-001; requirements: FR-NEW-015, FR-NEW-004
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `mixed.pdf` whose 1st figure is extracted as `.jpg` and 2nd as `.svg`. When Alice converts it and gets `image-1`. Then the result reports `image-2: unsupported picture format svg`. And `image-1` has format `jpeg`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-113
- Category: edge; scenario: SC-002; requirements: FR-NEW-004, FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `scan2.pdf` whose only figure is extracted as `.tif`. When Alice converts it and gets `image-1`. Then the result reports `1 images, 0 tables` and no missing item. And `image-1` has format `tiff`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-114
- Category: failure; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `mix2.pdf` converts to text referencing 3 pictures and holding 2 pipe tables, the 2nd referenced picture is missing from the converter's output and the 1st table's separator line does not match its header. When Alice converts it. Then the result reports `2 images, 1 tables`, then `image-2: picture unavailable`, then `table-1: malformed table`, in that order.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-129
- Category: failure; scenario: SC-001; requirements: FR-NEW-033, FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter's output for `blur.pdf` marks the caption of its only picture as failed. When Alice converts it. Then the result reports `1 images, 0 tables`, then `image-1: caption unavailable`. And `image-1` is listed with an empty caption.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-143
- Category: edge; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter silently drops the 2nd of 3 figures of `drop.pdf`, so its conversion text references only 2 pictures. When Alice converts it. Then the result reports `2 images, 0 tables` with no missing item. And listing images shows `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-151
- Category: failure; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `trav.pdf` the lines `![x](../../secret.png)` then `![y](figures/figure_1.png)` with that figure. When Alice converts it. Then the result reports `1 images, 0 tables` then `image-1: picture unavailable`. And listing images shows only `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-152
- Category: failure; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `url.pdf` the line `![x](https://example.com/a.png)`. When Alice converts it. Then the result reports `0 images, 0 tables` then `image-1: picture unavailable`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-153
- Category: failure; scenario: SC-001; requirements: FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `abs.pdf` the line `![x](/etc/hosts.png)`. When Alice converts it. Then the result reports `0 images, 0 tables` then `image-1: picture unavailable`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: every conversion path (US-0007).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0006: Image failures and picture formats`, body `Spec: SPEC-0019` and the requirement ids.
