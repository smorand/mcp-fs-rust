# US-0011: Refused and overtaken re-conversions

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0010
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
A re-conversion refused by the quota, by a missing converter, or overtaken by a document change must leave the previous set and the sibling untouched and charge nothing.

## Requirements covered
- SPEC-0019/FR-MOD-001: IF a text extraction is refused for the session write quota THEN THE Documents context SHALL leave the Markdown sibling as it was before the request.
- SPEC-0019/FR-NEW-013: IF a conversion (artifacts and Markdown sibling together) exceeds the member's session write quota THEN THE Documents context SHALL refuse it, charge nothing for it, and leave every existing artifact and the Markdown sibling unchanged.
- SPEC-0019/FR-NEW-035: WHEN a document changes while its conversion is running THE Documents context SHALL keep no artifact and no conversion text from that conversion and report `Document changed during conversion: <path>`.

## Independent Test
`make check`; then with a fake converter that waits on a channel, write new bytes to `report.pdf` during the conversion and observe `Document changed during conversion: /report.pdf` and the sibling bytes unchanged.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0019): Validity token `nodes.rev`, a uuid v4 stamped on every node insert, path change and byte write (not on atime or mtime only touches); a set is served only while `set.node_rev = nodes.rev`. **Obliged by:** FR-NEW-011, FR-NEW-012, FR-NEW-035. **Rationale:** exact for a same bytes rewrite and a move away and back; one choke point. **Alternatives considered:** sha256 or mtime key (approach B), hooks (approach C). **Source:** design. **Evidence:** `storage/meta.rs:927-975`, `:1169`, `:1189-1221`.
- DEC-005 (SPEC-0019): One charge per conversion of (set bytes + sibling UTF-8 bytes + conversion text UTF-8 bytes) through `charge_write` before any write; set bytes as FR-NEW-013 defines; a documented upload keeps its source charge and reports the quota message as its documentation outcome. **Obliged by:** FR-NEW-013, FR-NEW-014, FR-MOD-001. **Rationale:** a refusal leaves nothing. **Alternatives considered:** charging after commit (today's extraction order, refused by FR-MOD-001). **Source:** spec. **Evidence:** `safety.rs:130-136`, `fs_ops.rs:1553-1555`, `fs_ops.rs:651-666`.
- DEC-008 (SPEC-0019): Replacement is pointer based: commit inserts the new set rows, then upserts `doc_artifact_current` in one transaction that also checks `nodes.rev` equals the captured rev; readers resolve the pointer once and read rows by `set_id`; replaced sets get `superseded_at` and are deleted by the sweep after 10 minutes; a per `(volume, path)` in process async lock serializes sibling write and commit so set, text and sibling come from the same conversion. **Obliged by:** FR-NEW-010, FR-NEW-035. **Rationale:** never a mix (E2E-081), last commit wins (E2E-157). **Alternatives considered:** delete and insert in place (readers see partial), a multi statement read transaction (PostgreSQL read committed). **Source:** design. **Evidence:** owned transactions in `storage/rel/`.
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
| `crates/core/src/docs/artifacts/capture.rs` | modify: rev check refusal message |
| `crates/core/src/core/fs_ops.rs` | modify: charge before writes on every path |
| `crates/core/src/tools/artifacts.rs` | modify: e2e tests |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-071
- Category: failure; scenario: SC-005; requirements: FR-NEW-013, FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, Alice has 10 000 bytes of session write quota left and the new set weighs 50 000 bytes. When Alice re-converts it with overwrite. Then she receives `session write quota of 262144 bytes exceeded`. And its 2 images, 2 tables and `report.md` are unchanged byte for byte.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-075
- Category: failure; scenario: SC-005; requirements: FR-MOD-001, FR-NEW-013, FR-NEW-026
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted (2 images) and Alice has 10 bytes of session write quota left; the new sibling and set
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-076
- Category: failure; scenario: SC-005; requirements: FR-MOD-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, `report.md` is kept as reference R, and Alice has 10 bytes of session write quota left; the new sibling and set
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-077
- Category: failure; scenario: SC-005; requirements: FR-NEW-013, FR-NEW-022
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted with `report.md` kept as reference R, and Alice has 10 bytes of session write quota left; the new sibling and set
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-078
- Category: failure; scenario: SC-005; requirements: FR-NEW-014, FR-NEW-011
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted with `report.md` kept as reference R, and Alice's session write quota is 600 000 bytes with 300 000 left. When she uploads a new 290 000 byte `report.pdf` requesting documentation, whose new artifacts, Markdown
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-085
- Category: side effect; scenario: SC-005; requirements: FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, the converter now returns a different set for it, and Alice has 60 000 bytes of session write quota left; the new set, sibling and conversion text weigh 50 000 bytes. When she re-converts it with overwrite, then writes a 20 000 byte file. Then the re-conversion succeeds. And the write is refused with `session write quota of 262144 bytes exceeded`, the old set's bytes not having been given back.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-107
- Category: failure; scenario: SC-005; requirements: FR-NEW-029, FR-NEW-010
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted while a converter was configured, then the operator removed the converter. When Alice uses the convert action with overwrite. Then she receives `document service is not configured`. And listing its images still returns `image-1` and `image-2`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-133
- Category: failure; scenario: SC-005; requirements: FR-NEW-035
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and the converter takes 5 seconds. When Alice re-converts it with overwrite and Bob writes new bytes to `report.pdf` 1 second later. Then Alice receives `Document changed during conversion: /report.pdf`. And listing its images answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-134
- Category: failure; scenario: SC-006; requirements: FR-NEW-035
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `fresh.pdf` was never converted and the converter takes 5 seconds. When Alice converts it and Bob moves it to `moved.pdf` 1 second later. Then Alice receives `Document changed during conversion: /fresh.pdf`. And `moved.pdf` answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-135
- Category: edge; scenario: SC-005; requirements: FR-NEW-035
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and the converter takes 5 seconds. When Alice re-converts it with overwrite and Bob only reads `report.pdf` meanwhile. Then the re-conversion succeeds and lists its images.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-138
- Category: failure; scenario: SC-005; requirements: FR-NEW-035, FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted, `report.md` is kept as reference R, and Alice has written 0 bytes this session; the converter takes 5 seconds. When Alice re-converts it with overwrite and Bob writes new bytes to `report.pdf` 1 second later, and then Alice writes a 262 144 byte file `n.txt`. Then Alice's re-conversion is refused with `Document changed during conversion: /report.pdf` in the invalid argument category. And `report.md` is byte identical to R, and the write of `n.txt` succeeds.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: change paths (US-0012).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0011: Refused and overtaken re-conversions`, body `Spec: SPEC-0019` and the requirement ids.
