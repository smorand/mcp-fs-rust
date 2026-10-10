# US-0013: Artifacts are not files, stale sets swept

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0012
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Artifacts must never appear in folder listings, searches, trash, exports or version control, and the storage of stale sets must be reclaimed by the purge cycle.

## Requirements covered
- SPEC-0019/FR-NEW-021: The Documents context SHALL NOT include artifacts in folder listings, file searches, trash listings, project exports, or version control history and pushes.

## Independent Test
`make check`; then convert `report.pdf`, move it, run `run_cycle`, and observe the stale set rows and their blob references gone from the store.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0019): Artifacts are rows in four new meta store tables plus blob bytes, never `nodes`; their queries live in `storage/meta.rs` beside `trash_entries` and `export_links`. **Obliged by:** FR-NEW-001, FR-NEW-021. **Rationale:** enumerators read `nodes` only, so exclusion is free. **Alternatives considered:** hidden side folder (approach D), a new node `mode`. **Source:** existing code. **Evidence:** `storage/volume.rs:119-149`, `storage/meta.rs:150-173`.
- DEC-011 (SPEC-0019): `purge::run_cycle` also deletes sets superseded for more than 10 minutes or whose `node_rev` no longer matches any node, decrementing their blobs. **Obliged by:** FR-NEW-011, FR-NEW-021. **Rationale:** one existing background loop and CLI verb. **Alternatives considered:** eager deletion inside every node mutation transaction (changes `PutFileResult.gc` to a list across the trait). **Source:** existing code. **Evidence:** `purge.rs:175`.
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
| `crates/core/src/purge.rs` | modify: sweep superseded and stale sets in `run_cycle` |
| `crates/core/src/storage/meta.rs` | modify: `sweep_stale_artifact_sets` |
| `crates/core/src/tools/artifacts.rs` | modify: e2e tests |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-019
- Category: side effect; scenario: SC-001; requirements: FR-NEW-021
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice lists its folder and pushes the project. Then the folder shows `report.pdf` and `report.md` only. And the pushed history contains no image or table of `report.pdf`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-022
- Category: side effect; scenario: SC-001; requirements: FR-NEW-021
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice searches the project for every file name and for files containing `Q1 sales`. Then no image or table of `report.pdf` appears as a file. And `report.pdf` and `report.md` do.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-023
- Category: side effect; scenario: SC-001; requirements: FR-NEW-021
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice exports the project root as a zip. Then the archive holds `report.pdf` and `report.md` only.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-098
- Category: side effect; scenario: SC-006; requirements: FR-NEW-021, FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted. When Alice deletes it to the trash and lists the trash. Then the trash shows `report.pdf` only, with no image or table entry.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-099
- Category: side effect; scenario: SC-006; requirements: FR-NEW-012, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given Alice has 10 000 bytes of session write quota left and converted `small.pdf` weighs 5 000 bytes with 200 000 bytes of artifacts. When she copies it to `c.pdf`, then writes a 4 000 byte file. Then both succeed, only the 5 000 file bytes having been charged for the copy.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: parity and docs (US-0014).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0013: Artifacts are not files, stale sets swept`, body `Spec: SPEC-0019` and the requirement ids.
