# US-0002: Converter bundle and conversion capture

> Spec: SPEC-0019 (sdlc/specs/SPEC-0019-document-artifacts/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0001
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
Conversions through the external converter keep only stdout today; this story makes the converter return a bundle in CLI and API mode, parses the conversion text into picture references and pipe tables, commits the set under one quota charge, and adds the conversion report to every conversion result.

## Requirements covered
- SPEC-0019/FR-MOD-002: WHEN a text extraction, convert action or documentation write converts a document THE Documents context SHALL add the conversion report of FR-NEW-030 to its result.
- SPEC-0019/FR-NEW-001: WHEN a member converts a document THE Documents context SHALL keep each image (picture, caption, page, reading position) and each table found, as artifacts of that document.
- SPEC-0019/FR-NEW-013: IF a conversion (artifacts and Markdown sibling together) exceeds the member's session write quota THEN THE Documents context SHALL refuse it, charge nothing for it, and leave every existing artifact and the Markdown sibling unchanged.
- SPEC-0019/FR-NEW-030: WHEN a conversion completes THE Documents context SHALL return, in addition to every item the operation returns today and with those items unchanged, a conversion report.
- SPEC-0019/FR-NEW-033: WHEN the external converter returns its output THE Documents context SHALL take as images the picture references of the conversion text and as tables its pipe table blocks, in reading order.

## Independent Test
`make check`; then with an injected fake converter returning `| a | b |`, `| --- | --- |`, `| 1 | 2 |` for `t.pdf`, call `fs.documentize`, observe `artifacts_report` `0 images, 1 tables`, then `fs.list_tables` lists `table-1` with 1 data row read back from the store.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0019): One charge per conversion of (set bytes + sibling UTF-8 bytes + conversion text UTF-8 bytes) through `charge_write` before any write; set bytes as FR-NEW-013 defines; a documented upload keeps its source charge and reports the quota message as its documentation outcome. **Obliged by:** FR-NEW-013, FR-NEW-014, FR-MOD-001. **Rationale:** a refusal leaves nothing. **Alternatives considered:** charging after commit (today's extraction order, refused by FR-MOD-001). **Source:** spec. **Evidence:** `safety.rs:130-136`, `fs_ops.rs:1553-1555`, `fs_ops.rs:651-666`.
- DEC-006 (SPEC-0019): `DocService::convert` returns a `Conversion` bundle; CLI `{outdir}` placeholder and API JSON bundle; optional `artifacts.json` manifest; `to_markdown` kept as a wrapper. **Obliged by:** FR-NEW-001, FR-NEW-033, constraint 3.3. **Rationale:** same observable result in both modes. **Alternatives considered:** parsing the converter's `images.md` (internal format, no failure marker). **Source:** design. **Evidence:** `docs/service.rs:60-70`, `:139-227`, `:278-315`, `config.rs:765-794`, `:1063`.
- DEC-007 (SPEC-0019): Picture targets resolve only to keys of `Conversion.files` after normalizing the relative path; URLs, absolute paths and `..` escapes are `picture unavailable`; the filesystem is never read by target. **Obliged by:** FR-NEW-015. **Rationale:** traversal impossible by construction. **Alternatives considered:** canonicalize then prefix check on disk (TOCTOU). **Source:** design. **Evidence:** n/a.
- DEC-008 (SPEC-0019): Replacement is pointer based: commit inserts the new set rows, then upserts `doc_artifact_current` in one transaction that also checks `nodes.rev` equals the captured rev; readers resolve the pointer once and read rows by `set_id`; replaced sets get `superseded_at` and are deleted by the sweep after 10 minutes; a per `(volume, path)` in process async lock serializes sibling write and commit so set, text and sibling come from the same conversion. **Obliged by:** FR-NEW-010, FR-NEW-035. **Rationale:** never a mix (E2E-081), last commit wins (E2E-157). **Alternatives considered:** delete and insert in place (readers see partial), a multi statement read transaction (PostgreSQL read committed). **Source:** design. **Evidence:** owned transactions in `storage/rel/`.
- DEC-009 (SPEC-0019): Built-in conversion exposes uncapped grids with line spans; converter text is parsed by `parse.rs` with the FR-NEW-033 rules (picture reference line, pipe table block, fenced code). **Obliged by:** FR-NEW-002, FR-NEW-017, FR-NEW-033, FR-NEW-034. **Rationale:** exact CSV needs the grid; converter tables only exist as Markdown. **Alternatives considered:** parsing the built-in Markdown too (loses rows past 400 and escapes). **Source:** existing code. **Evidence:** `docs/extract.rs:288-312`, `:614-617`, `:905-913`, `:1038-1041`.
- DEC-012 (SPEC-0019): Tests drive a fake `DocService` in Rust; `scripts/doc_service_fake.py` gains a JSON bundle mode for manual api mode runs; no test needs the real `doc-convert`. **Obliged by:** FR-NEW-033, FR-NEW-015. **Rationale:** the real converter lacks the manifest today. **Alternatives considered:** gating tests on an installed `doc-convert`. **Source:** design. **Evidence:** spec Section 14.3.
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
| `crates/core/src/docs/service.rs` | modify: `DocService::convert`, `Conversion`, CLI `{outdir}`, API JSON bundle |
| `crates/core/src/config.rs` | modify: optional `{outdir}` placeholder, default command `doc-convert --quiet -o {outdir} {document}` |
| `crates/core/src/docs/artifacts/parse.rs` | create: FR-NEW-033 rules |
| `crates/core/src/docs/artifacts/capture.rs` | create: converter capture, report, rev check at commit (moved out of `mod.rs`) |
| `crates/core/src/core/fs_ops.rs` | modify: `documentize`, `write_bytes_documented` capture and `artifacts_report` |
| `scripts/doc_service_fake.py` | modify: JSON bundle response mode |
### Existing patterns
- Tool method: `fs_trash_list` (`crates/core/src/mcp/server.rs:2053-2069`), args struct `TrashListArgs` (`:1070-1082`), typed function `crates/core/src/tools/trash.rs:79-116`.
- REST route: export-zip (`crates/core/src/api/dataplane.rs:87`, `:151`, `:636-653`), OpenAPI `Op` (`crates/core/src/api/openapi.rs:1016-1022`).
- Meta table: `trash_entries` / `export_links` (`crates/core/src/storage/meta.rs:150-173`), conformance case `export_links_round_trip` (`crates/core/src/storage/conformance.rs:690-725`).
- Tool e2e: `e2e_new_407_trash_list_on_unknown_project_is_not_found` (`crates/core/src/mcp/server.rs:4631`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.

### SPEC-0019/E2E-006
- Category: failure; scenario: SC-001; requirements: FR-NEW-015, FR-NEW-001
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `three.pdf` converts to text holding 3 pipe tables, the 2nd with a 2 cell header and a 3 cell separator line. When Alice converts it. Then the result reports `0 images, 2 tables` and `table-2: malformed table`. And listing tables shows exactly `table-1` and `table-3`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-010
- Category: failure; scenario: SC-001; requirements: FR-NEW-018
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter accepts only `.pdf`. When Alice uses the convert action on `budget.xlsx`. Then she receives `'/budget.xlsx' cannot be documented, the document service accepts: .pdf`. And listing its tables answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-011
- Category: failure; scenario: SC-001; requirements: FR-NEW-029
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured. When Alice uses the convert action on `budget.xlsx`. Then she receives `document service is not configured`. And listing its tables answers `No artifacts: document not extracted`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-020
- Category: side effect; scenario: SC-001; requirements: FR-NEW-022
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the Markdown sibling of `report.pdf` produced before this spec is kept as reference R. When Alice converts `report.pdf` with overwrite after this spec. Then `report.md` exists at the same place. And its content is byte identical to R.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-021
- Category: side effect; scenario: SC-001; requirements: FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given Alice has written 0 bytes this session and the artifacts, Markdown sibling and conversion text of `report.pdf` weigh 200 000 bytes together. When she converts it, then writes a 70 000 byte file `notes.txt`. Then the conversion succeeds. And the write of `notes.txt` is refused with `session write quota of 262144 bytes exceeded`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-043
- Category: edge; scenario: SC-003; requirements: FR-NEW-002, FR-NEW-017
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `report.pdf` was converted and the text of `budget.xlsx` was extracted. When Alice lists the tables of both. Then those of `report.pdf` read `CSV quality: approximate`. And those of `budget.xlsx` read `CSV quality: exact`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-045
- Category: edge; scenario: SC-003; requirements: FR-NEW-005
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given `big.pdf` was converted with 101 tables. When Alice lists its tables and follows the marker. Then page 1 holds `table-1` to `table-100`. And page 2 holds only `table-101`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-106
- Category: failure; scenario: SC-001; requirements: FR-NEW-029
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given no external converter is configured. When Alice uploads `report.pdf` requesting documentation. Then she receives `document service is not configured`. And `report.pdf` does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-110
- Category: failure; scenario: SC-001; requirements: FR-NEW-018
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter accepts only `.pdf`. When Alice uploads `budget.xlsx` requesting documentation. Then she receives `'/budget.xlsx' cannot be documented, the document service accepts: .pdf`. And `budget.xlsx` does not exist.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-128
- Category: edge; scenario: SC-003; requirements: FR-NEW-033
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `nocap.pdf` a pipe table preceded by the line `Summary`. When Alice converts it and lists its tables. Then `table-1` has an empty caption.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-145
- Category: edge; scenario: SC-001; requirements: FR-NEW-033
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `code.pdf` a fenced code block holding `| a | b |` then `| --- | --- |`. When Alice converts it and lists its tables. Then the list is empty and the result reports `0 images, 0 tables`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-146
- Category: edge; scenario: SC-001; requirements: FR-NEW-033
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `sep.pdf` the lines `| a | b |` then `| c | d |`. When Alice converts it. Then the result reports `0 images, 0 tables` with no missing item.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-148
- Category: edge; scenario: SC-003; requirements: FR-NEW-033
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `bare.pdf` the lines `a | b` then `--- | ---` then `1 | 2`. When Alice converts it. Then the result reports `0 images, 0 tables` with no missing item.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-149
- Category: failure; scenario: SC-001; requirements: FR-NEW-033, FR-NEW-015
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `esc2.pdf` the lines `| a\|b | c |` then `| --- | --- | --- |`. When Alice converts it. Then the result reports `0 images, 0 tables` then `table-1: malformed table`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-150
- Category: edge; scenario: SC-003; requirements: FR-NEW-033
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given the converter returns for `trail.pdf` the lines `| a | b` then `| --- | ---` then `| 1 | 2`. When Alice converts it and lists its tables. Then `table-1` shows 1 data row and 2 columns.
- Cleanup: the test project and its temporary volume are dropped with the harness.

### SPEC-0019/E2E-156
- Category: failure; scenario: SC-001; requirements: FR-NEW-013
- Level / Driver / Location: functional e2e / McpServer tool calls (pattern `mcp/server.rs:4631`) / crates/core/src/tools/artifacts.rs `#[cfg(test)] mod e2e`
- Given Alice has written 0 bytes this session, and for `t.pdf` the set weighs 100 000 bytes, the Markdown sibling 80 000 bytes and the conversion text 80 000 bytes. When she converts it, then writes a 10 000 byte file `n.txt`. Then the conversion succeeds. And the write of `n.txt` is refused with `session write quota of 262144 bytes exceeded`.
- Cleanup: the test project and its temporary volume are dropped with the harness.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*` engine files (use `RelationalDb`/`Dialect` only), existing acceptance tests.
- Dependencies not to add: any crate; `uuid`, `base64`, `serde_json`, `sha2` are already present.
- Patterns to avoid: a second implementation in `api/dataplane.rs`; artifacts as `nodes` rows; a `WHERE` without `volume_id`; dashes as punctuation in code or messages.
- Scope boundary: get table (US-0003), image tools (US-0005), view (US-0008).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing test passes; the Markdown sibling bytes are unchanged (SPEC-0019/FR-NEW-022); existing result keys unchanged.
- [ ] Docs updated: TOOL_CONTRACT.txt when a tool is added; nothing else.
- [ ] Self review: full.
- [ ] Closes spec only: n/a.
- [ ] One commit `US-0002: Converter bundle and conversion capture`, body `Spec: SPEC-0019` and the requirement ids.
