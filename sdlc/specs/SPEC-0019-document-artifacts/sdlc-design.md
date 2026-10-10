# SPEC-0019: Document artifacts, design

> Spec: sdlc/specs/SPEC-0019-document-artifacts/sdlc-spec.md
> Nature: FEAT
> Depth: L
> Security: n/a
> Designed on: 2026-10-10
> Target tier: 2 (default, no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust, edition 2024 | workspace `Cargo.toml` | DEC-001 |
| MCP | rmcp `#[tool]` methods on `McpServer` | manifest | DEC-001 |
| REST | axum routes in `api/dataplane.rs`, OpenAPI `Op` table | manifest | DEC-001 |
| Persistence | `RelationalMetaStore` schema (`storage/meta.rs:124-194`) + blob store | manifest | DEC-002 |
| Ids and tokens | `uuid` v4, already a dependency (`Cargo.toml:127`) | 1 | DEC-003 |
| Markers | `base64` URL safe no pad, already a dependency (`Cargo.toml:106`) | 0.22 | DEC-010 |
| JSON | `serde_json`, already a dependency (`Cargo.toml:24`) | 1 | DEC-006 |

No dependency is added or removed, so no context7 lookup is needed. Skills loaded: `rust`
(binding: `make check` gate, thiserror in core, tracing on boundaries, typed `Parameters<T>` tools,
two channel errors, e2e tests on real entry points).

## 2. Approaches

### Chosen: A, artifact rows beside the node tree, validity by node revision

Artifacts live in new relational tables of the meta store (not as `nodes`); picture and text bytes
live in the blob store, content addressed and reference counted through `blob_refs`. A new nullable
`nodes.rev` column gets a fresh value on every node mutation; an artifact set records the `rev` it
was built from and is served only while it still equals the node's current `rev`.

- Pros: nothing new appears in `nodes`, so folder listing, glob, grep, export, trash and git tree
  building exclude artifacts with no change (`storage/volume.rs:119-149` enumerate `nodes` only);
  every mutation path invalidates with no hook per tool, because every byte write funnels through
  `VolumeClient::write_bytes_atomic` (`storage/volume.rs:157`) and every move through the meta
  `rename` (`storage/meta.rs:1160-1180`); concurrent readers stay consistent (DEC-008).
- Cons and risks: one column migration on `nodes`; stale sets occupy storage until swept (DEC-011).
- Affected scope: storage/meta, docs, core/fs_ops, tools, mcp, api, purge.
- Best suited when: invalidation must be exact (same bytes rewrite, move away and back).

### Rejected: B, validity by content hash or mtime

Key artifact sets by `(path, sha256)` or `(path, mtime)`. Rejected (DEC-003): a same bytes rewrite
keeps path and sha256 (`storage/meta.rs:927-950`) and a move away and back restores them, so
E2E-091 and E2E-097 would keep artifacts; `mtime` has one second resolution.

### Rejected: C, eager invalidation hooks in every mutating tool

Delete artifact rows from each fs, git, archive and trash operation. Rejected (DEC-003): more than
twenty call sites; a new mutation path would silently serve stale artifacts and nothing detects a
missed hook.

### Rejected: D, artifacts as hidden files in a side folder

Store `report.pdf.artifacts/` files. Rejected (DEC-002): every enumerator would need a filter
(FR-NEW-021), and the spec forbids artifacts appearing as files.

## 3. Architecture

```
            MCP #[tool] (mcp/server.rs)        REST (api/dataplane.rs)
                       \                         /
                        tools/artifacts.rs  (list_images, get_image, list_tables,
                        |                    get_table, document_view: the only implementation)
                        v
   docs/artifacts/ ---- mod.rs     (model: ArtifactSet, ImageArtifact, TableArtifact, CsvQuality,
        |                           Report; continuation markers)
        |               parse.rs   (conversion text -> picture refs + pipe tables, FR-NEW-033)
        |               render.rs  (Markdown/CSV tables FR-NEW-002, view FR-NEW-007/036/037)
        |               capture.rs (build a set from a conversion, charge, commit with rev check)
        ^
   storage/meta.rs: artifact set queries (sets, images, tables, current pointer) and nodes.rev
   core/fs_ops.rs: documentize / write_bytes_documented / extract_document call capture
   docs/service.rs: DocService::convert -> Conversion { markdown, files, manifest }
   docs/extract.rs: built-in conversion also returns table grids with their line spans
   purge.rs: run_cycle also sweeps stale and superseded artifact sets
```

Bounded contexts: Documents = `docs/artifacts/*`, `tools/artifacts.rs` and the artifact queries in
`storage/meta.rs`; Files = `storage/*` (only the `rev` stamp); Quota =
`SafetyManager::charge_write` (`safety.rs:130-136`), unchanged; Access = `state.authorize`
(`state.rs:59-60`), unchanged.

## 4. Data model

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

## 5. Contracts and interfaces

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

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/meta.rs` | modify | FR-NEW-001, FR-NEW-005, FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-017, FR-NEW-021, FR-NEW-023, FR-NEW-026, FR-NEW-027, FR-NEW-034 | US-0001, US-0010, US-0012, US-0013 |
| `crates/core/src/storage/conformance.rs` | modify | FR-NEW-001, FR-NEW-005, FR-NEW-017, FR-NEW-023, FR-NEW-034 | US-0001 |
| `crates/core/src/docs/artifacts/mod.rs` | create | FR-NEW-001, FR-NEW-005, FR-NEW-017, FR-NEW-023, FR-NEW-034 | US-0001 |
| `crates/core/src/docs/extract.rs` | modify | FR-NEW-001, FR-NEW-005, FR-NEW-017, FR-NEW-023, FR-NEW-034 | US-0001 |
| `crates/core/src/core/fs_ops.rs` | modify | FR-MOD-001, FR-MOD-002, FR-NEW-001, FR-NEW-005, FR-NEW-010, FR-NEW-013, FR-NEW-014, FR-NEW-016, FR-NEW-017, FR-NEW-018, FR-NEW-023, FR-NEW-026, FR-NEW-027, FR-NEW-029, FR-NEW-030, FR-NEW-033, FR-NEW-034, FR-NEW-035 | US-0001, US-0002, US-0007, US-0010, US-0011 |
| `crates/core/src/tools/artifacts.rs` | create | FR-MOD-001, FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-013, FR-NEW-014, FR-NEW-015, FR-NEW-016, FR-NEW-017, FR-NEW-018, FR-NEW-019, FR-NEW-020, FR-NEW-021, FR-NEW-022, FR-NEW-023, FR-NEW-024, FR-NEW-025, FR-NEW-026, FR-NEW-027, FR-NEW-028, FR-NEW-029, FR-NEW-031, FR-NEW-032, FR-NEW-034, FR-NEW-035, FR-NEW-036, FR-NEW-037 | US-0001, US-0003, US-0004, US-0005, US-0006, US-0007, US-0008, US-0009, US-0010, US-0011, US-0012, US-0013, US-0014 |
| `crates/core/src/mcp/server.rs` | modify | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-017, FR-NEW-022, FR-NEW-023, FR-NEW-025, FR-NEW-034 | US-0001, US-0003, US-0005, US-0008 |
| `TOOL_CONTRACT.txt, tool-contract-golden.json, crates/core/src/tools/contract_golden.rs` | modify | FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-017, FR-NEW-022, FR-NEW-023, FR-NEW-025, FR-NEW-034 | US-0001, US-0003, US-0005, US-0008 |
| `crates/core/src/docs/service.rs` | modify | FR-MOD-002, FR-NEW-001, FR-NEW-013, FR-NEW-030, FR-NEW-033 | US-0002 |
| `crates/core/src/config.rs` | modify | FR-MOD-002, FR-NEW-001, FR-NEW-013, FR-NEW-030, FR-NEW-033 | US-0002 |
| `crates/core/src/docs/artifacts/parse.rs` | create | FR-MOD-002, FR-NEW-001, FR-NEW-013, FR-NEW-015, FR-NEW-030, FR-NEW-033 | US-0002, US-0006 |
| `crates/core/src/docs/artifacts/capture.rs` | create | FR-MOD-001, FR-MOD-002, FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-010, FR-NEW-013, FR-NEW-014, FR-NEW-015, FR-NEW-016, FR-NEW-017, FR-NEW-018, FR-NEW-026, FR-NEW-027, FR-NEW-029, FR-NEW-030, FR-NEW-033, FR-NEW-035 | US-0002, US-0005, US-0006, US-0007, US-0010, US-0011 |
| `scripts/doc_service_fake.py` | modify | FR-MOD-002, FR-NEW-001, FR-NEW-013, FR-NEW-030, FR-NEW-033 | US-0002 |
| `crates/core/src/docs/artifacts/render.rs` | create | FR-NEW-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-022, FR-NEW-025, FR-NEW-036, FR-NEW-037 | US-0003, US-0008, US-0009 |
| `crates/core/src/api/dataplane.rs` | modify | FR-NEW-003, FR-NEW-004, FR-NEW-007, FR-NEW-008, FR-NEW-019, FR-NEW-022, FR-NEW-024, FR-NEW-025, FR-NEW-028, FR-NEW-031, FR-NEW-032 | US-0004, US-0005, US-0008 |
| `crates/core/src/api/openapi.rs` | modify | FR-NEW-003, FR-NEW-004, FR-NEW-007, FR-NEW-008, FR-NEW-019, FR-NEW-022, FR-NEW-024, FR-NEW-025, FR-NEW-028, FR-NEW-031, FR-NEW-032 | US-0004, US-0005, US-0008 |
| `crates/core/src/tools/git.rs` | modify | FR-NEW-011, FR-NEW-012 | US-0012 |
| `crates/core/src/purge.rs` | modify | FR-NEW-021 | US-0013 |
| `README.md, AGENTS.md` | modify | FR-NEW-003, FR-NEW-020 | US-0014 |
| `.agent_docs/tools.md, .agent_docs/api.md, .agent_docs/architecture.md` | modify | FR-NEW-003, FR-NEW-020 | US-0014 |

Affected existing tests: keep all. The tool count assertions (`tools/contract_golden.rs:142`,
`:249`, `:305`, `:319`, `mcp/server.rs:3921`, `:3942-3943`) are bumped by each tool story (102 to
103, 104, 106, 107); that is the declared contract change. Existing extraction tests keep passing:
the sibling bytes are unchanged (FR-NEW-022).
Affected documentation: `README.md`, `AGENTS.md` (102 to 107 tools), `.agent_docs/tools.md`,
`.agent_docs/api.md`, `.agent_docs/architecture.md` (US-0014).

## 7. Compatibility and risks

- Existing results gain one key (`artifacts_report`); no key removed or changed (FR-MOD-002).
- Text extraction checks the quota before writing the sibling (FR-MOD-001): a refused extraction
  no longer leaves a sibling. Declared in the spec.
- The default CLI command changes to output folder mode; a deployment with a custom command and no
  `{outdir}` keeps today's behavior (no pictures). The real `doc-convert` lacks `artifacts.json`
  (page, caption failure marker), keeps PNG only and deduplicates figures (spec 14.3): with the real
  converter pages are empty and `caption unavailable` never appears until docling-scripts ships the
  manifest. Tests drive a fake converter (DEC-012).
- Ordering: US-0001 lands the `nodes.rev` migration before any reader depends on it.
- Risk: `tools/git.rs:7245 write_one`, if a whole tree checkout calls it for unchanged files, would
  discard their artifacts (E2E-141); US-0012 makes it skip blobs whose sha equals the node's.
- Risk: a small window between the rev check and the sibling write (DEC-008), closed for concurrent
  conversions by the per path lock, open only to a non conversion write racing it.
- Single replica: the per path commit lock is in process (BL-0015 covers multi replica).
- Dependencies added or removed: none.

## 9. Test strategy

All acceptance tests are functional e2e, driven through the real `McpServer` tool methods (pattern
`mcp/server.rs:4631`) or, for parity and REST tests, the in process axum harness (pattern
`api/dataplane.rs:2721-2820`), with an injected fake `DocService` returning a `Conversion`. Store
logic gets unit tests in `storage/meta.rs` and `docs/artifacts/*` and one conformance case
(`storage/conformance.rs:728-744`) run on every engine. `make check` runs them; PostgreSQL and SQL
Server run under `make test-e2e-full`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 to E2E-167 | functional e2e | `McpServer` tool calls; axum harness where the test names an HTTP API consumer | `crates/core/src/tools/artifacts.rs` `#[cfg(test)] mod e2e` | the story file listing `### SPEC-0019/E2E-NNN` (each test in exactly one) |
| `doc_artifacts_round_trip` | integration | `RelationalDb` | `crates/core/src/storage/conformance.rs` | US-0001 |

Test data: generated in the tests (fake converter outputs, xlsx and csv built in memory as existing
extraction tests do). Captions and pages come from the fake converter's manifest. No user provided
data.

## 10. Decisions log

- **DEC-001:** Five new tools and five REST routes, one implementation in `tools/artifacts.rs`
  called by both. **Obliged by:** SPEC-0019/FR-NEW-003, 004, 005, 006, 007, 020. **Rationale:** the
  project rule "one engine, two doors". **Alternatives considered:** one generic `fs.artifacts` tool
  with a `kind` argument (harder for an LLM, no gain). **Source:** constitution, AGENTS.md.
  **Evidence:** `api/dataplane.rs:634-650`, `tools/export.rs:5-7`.
- **DEC-002:** Artifacts are rows in four new meta store tables plus blob bytes, never `nodes`;
  their queries live in `storage/meta.rs` beside `trash_entries` and `export_links`.
  **Obliged by:** FR-NEW-001, FR-NEW-021. **Rationale:** enumerators read `nodes` only, so exclusion
  is free. **Alternatives considered:** hidden side folder (approach D), a new node `mode`.
  **Source:** existing code. **Evidence:** `storage/volume.rs:119-149`, `storage/meta.rs:150-173`.
- **DEC-003:** Validity token `nodes.rev`, a uuid v4 stamped on every node insert, path change and
  byte write (not on atime or mtime only touches); a set is served only while
  `set.node_rev = nodes.rev`. **Obliged by:** FR-NEW-011, FR-NEW-012, FR-NEW-035. **Rationale:**
  exact for a same bytes rewrite and a move away and back; one choke point. **Alternatives
  considered:** sha256 or mtime key (approach B), hooks (approach C). **Source:** design.
  **Evidence:** `storage/meta.rs:927-975`, `:1169`, `:1189-1221`.
- **DEC-004:** Picture and conversion text bytes are content addressed blobs counted in
  `blob_refs`, incref on set commit, decref on set deletion, GC at zero. **Obliged by:**
  FR-NEW-004, FR-NEW-009. **Rationale:** reuses layout, refcount and GC. **Alternatives considered:**
  BLOB columns (three dialect typings, large rows). **Source:** existing code. **Evidence:**
  `storage/meta.rs:140-148`, `:436`, `:455`.
- **DEC-005:** One charge per conversion of (set bytes + sibling UTF-8 bytes + conversion text UTF-8
  bytes) through `charge_write` before any write; set bytes as FR-NEW-013 defines; a documented
  upload keeps its source charge and reports the quota message as its documentation outcome.
  **Obliged by:** FR-NEW-013, FR-NEW-014, FR-MOD-001. **Rationale:** a refusal leaves nothing.
  **Alternatives considered:** charging after commit (today's extraction order, refused by
  FR-MOD-001). **Source:** spec. **Evidence:** `safety.rs:130-136`, `fs_ops.rs:1553-1555`,
  `fs_ops.rs:651-666`.
- **DEC-006:** `DocService::convert` returns a `Conversion` bundle; CLI `{outdir}` placeholder and
  API JSON bundle; optional `artifacts.json` manifest; `to_markdown` kept as a wrapper.
  **Obliged by:** FR-NEW-001, FR-NEW-033, constraint 3.3. **Rationale:** same observable result in
  both modes. **Alternatives considered:** parsing the converter's `images.md` (internal format, no
  failure marker). **Source:** design. **Evidence:** `docs/service.rs:60-70`, `:139-227`, `:278-315`,
  `config.rs:765-794`, `:1063`.
- **DEC-007:** Picture targets resolve only to keys of `Conversion.files` after normalizing the
  relative path; URLs, absolute paths and `..` escapes are `picture unavailable`; the filesystem is
  never read by target. **Obliged by:** FR-NEW-015. **Rationale:** traversal impossible by
  construction. **Alternatives considered:** canonicalize then prefix check on disk (TOCTOU).
  **Source:** design. **Evidence:** n/a.
- **DEC-008:** Replacement is pointer based: commit inserts the new set rows, then upserts
  `doc_artifact_current` in one transaction that also checks `nodes.rev` equals the captured rev;
  readers resolve the pointer once and read rows by `set_id`; replaced sets get `superseded_at` and
  are deleted by the sweep after 10 minutes; a per `(volume, path)` in process async lock
  serializes sibling write and commit so set, text and sibling come from the same conversion.
  **Obliged by:** FR-NEW-010, FR-NEW-035. **Rationale:** never a mix (E2E-081), last commit wins
  (E2E-157). **Alternatives considered:** delete and insert in place (readers see partial), a multi
  statement read transaction (PostgreSQL read committed). **Source:** design. **Evidence:** owned
  transactions in `storage/rel/`.
- **DEC-009:** Built-in conversion exposes uncapped grids with line spans; converter text is parsed
  by `parse.rs` with the FR-NEW-033 rules (picture reference line, pipe table block, fenced code).
  **Obliged by:** FR-NEW-002, FR-NEW-017, FR-NEW-033, FR-NEW-034. **Rationale:** exact CSV needs the
  grid; converter tables only exist as Markdown. **Alternatives considered:** parsing the built-in
  Markdown too (loses rows past 400 and escapes). **Source:** existing code. **Evidence:**
  `docs/extract.rs:288-312`, `:614-617`, `:905-913`, `:1038-1041`.
- **DEC-010:** Continuation marker = base64url (no pad) of `v1\n<project>\n<kind>\n<path>\n<offset>`;
  any decode or field mismatch is invalid; an offset past the end gives an empty page with no marker.
  **Obliged by:** FR-NEW-003. **Rationale:** stateless, scoped, person independent. **Alternatives
  considered:** plain offset (cannot detect cross list use), a server side cursor table.
  **Source:** design. **Evidence:** offset paging precedent `tools/trash.rs:79-116`.
- **DEC-011:** `purge::run_cycle` also deletes sets superseded for more than 10 minutes or whose
  `node_rev` no longer matches any node, decrementing their blobs. **Obliged by:** FR-NEW-011,
  FR-NEW-021. **Rationale:** one existing background loop and CLI verb. **Alternatives considered:**
  eager deletion inside every node mutation transaction (changes `PutFileResult.gc` to a list across
  the trait). **Source:** existing code. **Evidence:** `purge.rs:175`.
- **DEC-012:** Tests drive a fake `DocService` in Rust; `scripts/doc_service_fake.py` gains a JSON
  bundle mode for manual api mode runs; no test needs the real `doc-convert`. **Obliged by:**
  FR-NEW-033, FR-NEW-015. **Rationale:** the real converter lacks the manifest today.
  **Alternatives considered:** gating tests on an installed `doc-convert`. **Source:** design.
  **Evidence:** spec Section 14.3.
- **DEC-013:** Stories carry `min_tier: 1` where the vertical slice exceeds the tier 2 budget (the
  tool contract trio and the two doors are fixed overhead), `min_tier: 2` otherwise.
  **Obliged by:** n/a (no code impact): routing only. **Rationale:** a thinner slice would not be
  independently testable through a real entry point. **Alternatives considered:** horizontal
  store, then tools, then REST (forbidden). **Source:** design. **Evidence:** n/a.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0001 | Table artifacts from text extraction | P1 | FR-NEW-001, FR-NEW-005, FR-NEW-017, FR-NEW-023, FR-NEW-034 | main | 1 | no |
| US-0002 | Converter bundle and conversion capture | P1 | FR-NEW-001, FR-NEW-013, FR-NEW-030, FR-NEW-033, FR-MOD-002 | US-0001 | 1 | no |
| US-0003 | Get a table as Markdown or CSV | P1 | FR-NEW-002, FR-NEW-006 | US-0002 | 1 | no |
| US-0004 | Table refusals and table routes | P1 | FR-NEW-019, FR-NEW-024, FR-NEW-028, FR-NEW-031, FR-NEW-032 | US-0003 | 1 | no |
| US-0005 | List and get images | P1 | FR-NEW-003, FR-NEW-004 | US-0004 | 1 | no |
| US-0006 | Image failures and picture formats | P1 | FR-NEW-015 | US-0005 | 2 | no |
| US-0007 | Images through every conversion path | P1 | FR-NEW-014, FR-NEW-016, FR-NEW-017, FR-NEW-018, FR-NEW-029 | US-0006 | 1 | no |
| US-0008 | Full document view | P1 | FR-NEW-007, FR-NEW-008, FR-NEW-022, FR-NEW-025 | US-0007 | 1 | no |
| US-0009 | View line rules and sibling independence | P1 | FR-NEW-009, FR-NEW-036, FR-NEW-037 | US-0008 | 2 | no |
| US-0010 | Re-conversion replaces the set | P2 | FR-NEW-010, FR-NEW-026, FR-NEW-027 | US-0009 | 1 | no |
| US-0011 | Refused and overtaken re-conversions | P2 | FR-NEW-035, FR-NEW-013, FR-MOD-001 | US-0010 | 1 | no |
| US-0012 | Changed documents lose their artifacts | P2 | FR-NEW-011, FR-NEW-012 | US-0011 | 1 | no |
| US-0013 | Artifacts are not files, stale sets swept | P2 | FR-NEW-021 | US-0012 | 2 | no |
| US-0014 | Consumer parity, marker scope and docs | P2 | FR-NEW-020, FR-NEW-003 | US-0013 | 1 | yes |
