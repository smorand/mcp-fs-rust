# US-0010: Re-conversion replaces the set

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0009
> min_tier: 1
> Files touched: 4
> Closes spec: no

## Objective
Re-converting must replace the whole set atomically, renumber, never show a mix to a concurrent reader, keep the last committed conversion consistent with its sibling, and keep the set when the conversion fails.

## Requirements covered
- SPEC-0019/FR-NEW-010: WHEN a member re-converts a document THE Documents context SHALL replace its whole artifact set and its conversion text, numbering the new items from 1.
- SPEC-0019/FR-NEW-026: WHEN text extraction rewrites the Markdown sibling of a document THE Documents context SHALL replace its artifact set with the set the built-in conversion produces (FR-NEW-010).
- SPEC-0019/FR-NEW-027: IF a convert action is refused because the Markdown sibling exists and overwrite was not requested THEN THE Documents context SHALL leave the artifact set unchanged.

## Independent Test
`make check`; then run two concurrent re-conversions with a fake converter blocking on a barrier and observe exactly one complete set numbered from 1, read from the store.

## Design excerpt
### Decisions
- DEC-008 (SPEC-0019): Replacement is pointer based: commit inserts the new set rows, then upserts `doc_artifact_current` in one transaction that also checks `nodes.rev` equals the captured rev; readers resolve the pointer once and read rows by `set_id`; replaced sets get `superseded_at` and are deleted by the sweep after 10 minutes; a per `(volume, path)` in process async lock serializes sibling write and commit so set, text and sibling come from the same conversion. **Obliged by:** FR-NEW-010, FR-NEW-035. **Rationale:** never a mix (E2E-081), last commit wins (E2E-157). **Alternatives considered:** delete and insert in place (readers see partial), a multi statement read transaction (PostgreSQL read committed). **Source:** design. **Evidence:** owned transactions in `storage/rel/`.
### Data model
See SPEC-0019 design Section 4, copied here for what this story touches:
All tables in `storage::meta::schema()` (DEC-002), every row and every `WHERE` scoped by
`volume_id` (AGENTS.md rule).

**`nodes` (modified):** add column `rev TextKey(36) NULL` via `SchemaSet::column_migration`
(`storage/rel/schema.rs:164-178`). Every statement that inserts a node or changes its path or bytes
sets `rev` to a fresh uuid v4: `put_file` insert and update (`meta.rs:906-975`), directory inserts,
the `rename` per row update (`meta.rs:1169`), copy inserts. `atime`/`mtime` only touches
(`meta.rs:1189-1221`) do NOT change `rev`. Existing rows keep `NULL`; a conversion of a `NULL` rev
node first stamps it (DEC-003). Rollback: older code ignores the column; no data loss.

**`doc_artifact_current`** PK `(volume_id, path)`: `volume_id TextKey`, `path TextKey(path len)`,
`set_id TextKey(36)`, `node_rev TextKey(36)`, `updated_at BigInt`. The pointer a reader follows.

**`doc_artifact_sets`** PK `(volume_id, set_id)`: `path TextKey`, `node_rev TextKey(36)`,
`text_sha TextKey(64) NULL` (conversion text blob, NULL when empty), `text_len BigInt`,
`report Text` (the conversion report lines), `superseded_at BigInt NULL`, `created_at BigInt`.
Index `(volume_id, path)`.

**`doc_images`** PK `(volume_id, set_id, seq)`: `seq BigInt` (the n of `image-n`), `caption Text`,
`page BigInt NULL`, `format TextKey(8)`, `sha256 TextKey(64)`, `size BigInt`, `line_start BigInt`,
`line_count BigInt`, `caption_line BigInt NULL`.

**`doc_tables`** PK `(volume_id, set_id, seq)`: `caption Text`, `rows BigInt`, `cols BigInt`,
`quality TextKey(16)` (`exact` or `approximate`), `cells Text` (JSON array of arrays of strings,
normalized per FR-NEW-002), `line_start BigInt`, `line_count BigInt`.

Bytes: pictures and conversion text are stored with `BlobBackend::put` under their sha256, the same
key layout as file content (`{root}/{sha[..2]}/{sha}`), and counted in `blob_refs`
(`meta.rs:140-148`) with `tx_incref` on set commit and `tx_decref` on set deletion, GC at zero
(DEC-004). Failed ids (gaps) have no row; the report keeps them.

Migration: additive (`CREATE TABLE IF NOT EXISTS`, one `ALTER TABLE ADD COLUMN`). Rollback: drop the
four tables and the column; artifacts are derived data and re-conversion rebuilds them.
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
| `crates/core/src/docs/artifacts/capture.rs` | modify: per path commit lock, pointer swap, superseded sets |
| `crates/core/src/storage/meta.rs` | modify: `superseded_at`, pointer swap query |
| `crates/core/src/core/fs_ops.rs` | modify: refresh path replacement |
| `crates/core/src/tools/artifacts.rs` | modify: marker past end, e2e tests |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-069
- Category: happy; scenario: SC-005; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images) and the converter now returns 3 images for it, captioned `A`, `B`, `C`. When Alice re-converts it with overwrite. Then listing images shows exactly `image-1` `A`, `image-2` `B`, `image-3` `C`. And no caption `Schéma réseau 🌐` remains.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-070
- Category: happy; scenario: SC-005; requirements: FR-NEW-026, FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images, 2 tables). When Alice extracts its text with refresh. Then listing its images returns an empty list. And listing its tables returns an empty list (the built-in conversion finds no table in a PDF).
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-072
- Category: failure; scenario: SC-005; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and the converter is unreachable. When Alice re-converts it with overwrite. Then she receives the converter's existing failure message, unchanged from today. And its 2 images, 2 tables and `report.md` are unchanged.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-073
- Category: failure; scenario: SC-005; requirements: FR-NEW-027
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images) and the converter now returns 3 images for it. When Alice uses the convert action without overwrite. Then she receives `'/report.md' exists (pass overwrite=true)`. And listing images shows `image-1` and `image-2` with their former captions.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-074
- Category: failure; scenario: SC-005; requirements: FR-NEW-016
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Olga re-converts it with overwrite. Then she receives the refusal she receives today for any operation on Atlas. And the 2 images and 2 tables are unchanged.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-079
- Category: edge; scenario: SC-005; requirements: FR-NEW-010, FR-NEW-019
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted with 2 images and the converter now returns 1 image for it. When Alice re-converts it with overwrite, then gets `image-2`. Then she receives `Not found: image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-080
- Category: edge; scenario: SC-005; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images). When Alice and Bob re-convert it with overwrite at the same moment and both finish, then Alice lists its images. Then she gets exactly `image-1` and `image-2`, once each.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-081
- Category: edge; scenario: SC-005; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images) and the converter now returns 5 images for it. When Alice re-converts it with overwrite while Bob lists its images 20 times. Then every answer Bob gets holds exactly 2 or exactly 5 images, never another count or a mix of captions.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-082
- Category: edge; scenario: SC-005; requirements: FR-NEW-026
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images). When Alice extracts its text without refresh and the existing Markdown sibling answers it. Then listing its images still returns `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-083
- Category: edge; scenario: SC-005; requirements: FR-NEW-027
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and the converter now returns 1 image for it. When Alice uses the convert action with overwrite. Then the result reports `1 images, 2 tables`. And listing images shows only `image-1`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-084
- Category: edge; scenario: SC-005; requirements: FR-NEW-010, FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images). When Alice uploads a new `report.pdf` with 1 image requesting documentation. Then listing its images shows only `image-1`, with the new caption. And `Schéma réseau 🌐` is not listed.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-126
- Category: edge; scenario: SC-005; requirements: FR-NEW-030
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice extracts its text without refresh and the existing sibling answers it. Then the result carries no conversion report.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-136
- Category: edge; scenario: SC-002; requirements: FR-NEW-003
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `atlas2.pdf` (101 images) was converted, Alice holds the page 1 marker, and the converter now returns 3 images for it. When Alice re-converts it with overwrite, then lists its images with that marker. Then she receives an empty list with no continuation marker.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-157
- Category: edge; scenario: SC-005; requirements: FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, and the converter returns 3 images for Alice's request and 4 for Bob's, Bob's finishing last. When both re-convert it with overwrite at the same moment. Then Alice's result reports `3 images, 2 tables` and Bob's reports `4 images, 2 tables`. And listing images shows `image-1` to `image-4`, and `report.md` is the sibling of Bob's conversion.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: refused re-conversions (US-0011).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0010: Re-conversion replaces the set`, body `Spec: SPEC-0019` and the requirement ids.
