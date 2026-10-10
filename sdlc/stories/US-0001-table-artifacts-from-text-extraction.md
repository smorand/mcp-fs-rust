# US-0001: Table artifacts from text extraction

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 8
> Closes spec: no

## Objective
A member who extracts the text of a Word, Excel or CSV document today gets only Markdown; this story keeps every table the built-in conversion finds as a table artifact and lets the member list them. It lands the persistence every later story builds on: the artifact tables, the `nodes.rev` validity token and content addressed bytes.

## Requirements covered
- SPEC-0019/FR-NEW-001: WHEN a member converts a document THE Documents context SHALL keep each image (picture, caption, page, reading position) and each table found, as artifacts of that document.
- SPEC-0019/FR-NEW-005: WHEN a member lists a document's tables THE Documents context SHALL return, in reading order, each table's id, caption, data row count, column count and CSV quality, 100 per page.
- SPEC-0019/FR-NEW-017: WHEN a member extracts a document's text THE Documents context SHALL keep the tables the built-in conversion finds in Word, Excel and CSV documents, labelled `CSV quality: exact`, and no images.
- SPEC-0019/FR-NEW-023: IF a member requests artifacts of a document that was never converted, or that changed since its last conversion, THEN THE Documents context SHALL answer `No artifacts: document not extracted`.
- SPEC-0019/FR-NEW-034: WHEN text extraction cuts the conversion text at the requested character limit THE Documents context SHALL keep only the tables whose whole Markdown lies before the cut.

## Independent Test
`make check`; then through the MCP server extract the text of an in memory `budget.xlsx` with sheets `Q1` and `Q2`, call `fs.list_tables`, observe `table-1` caption `Q1` and `table-2` caption `Q2`, both `CSV quality: exact`; rewrite the file and observe `No artifacts: document not extracted` (the rev check runs in the store, not in the tool's own output).

## Design excerpt
### Decisions
- DEC-002 (SPEC-0019): Artifacts are rows in four new meta store tables plus blob bytes, never `nodes`; their queries live in `storage/meta.rs` beside `trash_entries` and `export_links`. **Obliged by:** FR-NEW-001, FR-NEW-021. **Rationale:** enumerators read `nodes` only, so exclusion is free. **Alternatives considered:** hidden side folder (approach D), a new node `mode`. **Source:** existing code. **Evidence:** `storage/volume.rs:119-149`, `storage/meta.rs:150-173`.
- DEC-003 (SPEC-0019): Validity token `nodes.rev`, a uuid v4 stamped on every node insert, path change and byte write (not on atime or mtime only touches); a set is served only while `set.node_rev = nodes.rev`. **Obliged by:** FR-NEW-011, FR-NEW-012, FR-NEW-035. **Rationale:** exact for a same bytes rewrite and a move away and back; one choke point. **Alternatives considered:** sha256 or mtime key (approach B), hooks (approach C). **Source:** design. **Evidence:** `storage/meta.rs:927-975`, `:1169`, `:1189-1221`.
- DEC-004 (SPEC-0019): Picture and conversion text bytes are content addressed blobs counted in `blob_refs`, incref on set commit, decref on set deletion, GC at zero. **Obliged by:** FR-NEW-004, FR-NEW-009. **Rationale:** reuses layout, refcount and GC. **Alternatives considered:** BLOB columns (three dialect typings, large rows). **Source:** existing code. **Evidence:** `storage/meta.rs:140-148`, `:436`, `:455`.
- DEC-005 (SPEC-0019): One charge per conversion of (set bytes + sibling UTF-8 bytes + conversion text UTF-8 bytes) through `charge_write` before any write; set bytes as FR-NEW-013 defines; a documented upload keeps its source charge and reports the quota message as its documentation outcome. **Obliged by:** FR-NEW-013, FR-NEW-014, FR-MOD-001. **Rationale:** a refusal leaves nothing. **Alternatives considered:** charging after commit (today's extraction order, refused by FR-MOD-001). **Source:** spec. **Evidence:** `safety.rs:130-136`, `fs_ops.rs:1553-1555`, `fs_ops.rs:651-666`.
- DEC-009 (SPEC-0019): Built-in conversion exposes uncapped grids with line spans; converter text is parsed by `parse.rs` with the FR-NEW-033 rules (picture reference line, pipe table block, fenced code). **Obliged by:** FR-NEW-002, FR-NEW-017, FR-NEW-033, FR-NEW-034. **Rationale:** exact CSV needs the grid; converter tables only exist as Markdown. **Alternatives considered:** parsing the built-in Markdown too (loses rows past 400 and escapes). **Source:** existing code. **Evidence:** `docs/extract.rs:288-312`, `:614-617`, `:905-913`, `:1038-1041`.
- DEC-010 (SPEC-0019): Continuation marker = base64url (no pad) of `v1\n<project>\n<kind>\n<path>\n<offset>`; any decode or field mismatch is invalid; an offset past the end gives an empty page with no marker. **Obliged by:** FR-NEW-003. **Rationale:** stateless, scoped, person independent. **Alternatives considered:** plain offset (cannot detect cross list use), a server side cursor table. **Source:** design. **Evidence:** offset paging precedent `tools/trash.rs:79-116`.
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
| `crates/core/src/storage/meta.rs` | modify: `nodes.rev` migration and stamping; the four artifact tables; set insert, pointer upsert, current set read, all scoped by `volume_id` |
| `crates/core/src/storage/conformance.rs` | modify: case `doc_artifacts_round_trip` appended to `run_suite` |
| `crates/core/src/docs/artifacts/mod.rs` | create: model, marker, built-in capture |
| `crates/core/src/docs/extract.rs` | modify: uncapped grids with line spans and captions; text returned before the sibling write |
| `crates/core/src/core/fs_ops.rs` | modify: `extract_document` charges once before writing, then captures |
| `crates/core/src/tools/artifacts.rs` | create: `list_tables` |
| `crates/core/src/mcp/server.rs` | modify: `fs.list_tables`, counts 102 to 103 |
| `TOOL_CONTRACT.txt, tool-contract-golden.json, crates/core/src/tools/contract_golden.rs` | regenerate (`MCPFS_REWRITE_TOOL_CONTRACT=1`) and bump the five count assertions; one mechanical unit |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-014
- Category: failure; scenario: SC-001; requirements: FR-MOD-001, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured, Alice has written 262 000 bytes this session and `budget.xlsx` has no Markdown sibling; its Markdown sibling, tables and conversion text weigh 5 000 bytes together. When Alice extracts its text. Then she receives `session write quota of 262144 bytes exceeded`. And `budget.md` does not exist, and listing its tables answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-101
- Category: edge; scenario: SC-003; requirements: FR-NEW-017
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `budget.xlsx` has sheets `Q1` and `Q2`, one table each. When Alice extracts its text and lists its tables. Then `table-1` has caption `Q1` and `table-2` has caption `Q2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-130
- Category: edge; scenario: SC-003; requirements: FR-NEW-034
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `two.xlsx` holds sheet `A` (3 rows) then sheet `B` (3 rows), and the character limit Alice requests cuts its text inside sheet `B`'s table. When Alice extracts its text and lists its tables. Then exactly `table-1`, caption `A`, is listed.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-131
- Category: failure; scenario: SC-003; requirements: FR-NEW-034
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `two.xlsx` as in E2E-130 and a character limit cutting its text inside sheet `A`'s table. When Alice extracts its text and lists its tables. Then the list is empty.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: converter bundle (US-0002), get table (US-0003), images (US-0005), view (US-0008).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0001: Table artifacts from text extraction`, body `Spec: SPEC-0019` and the requirement ids.
