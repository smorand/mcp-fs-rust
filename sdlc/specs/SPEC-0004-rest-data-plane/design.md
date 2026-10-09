> Id: SPEC-0004
> Nature: FEAT
> Status: as-built
> Area: rest-data-plane
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# REST Data Plane and OpenAPI — Design

## 1. Components

| Component | Responsibility | Location |
|---|---|---|
| Route manifest | The authoritative `(method, operation)` list every data-plane route must appear in; checked against the router and the documentation table by a build-time test. | `crates/core/src/api/dataplane.rs:45-91` |
| Router | Builds the axum `Router` for the 40 data-plane routes, applies the JSON body-size layer globally and the upload-size layer per-route. | `crates/core/src/api/dataplane.rs:105-155` |
| Guard (`guarded`, `guarded_json`) | Shared identity-then-membership-then-operation wrapper every handler runs through. | `crates/core/src/api/dataplane.rs:186-220` |
| Error response builders (`unauthorized`, `error_response`) | Render the two frozen 401/error JSON body shapes from an engine error or an identity-resolution failure. | `crates/core/src/api/dataplane.rs:222-236` |
| Storage-direct handlers | `roots`, `list`, `download`, `download_zip`: read through `VolumeClient` directly, bypassing the engine. | `crates/core/src/api/dataplane.rs:324-363`, `579-628` |
| Engine-parity handlers | The remaining 36 handlers (including `mkdir`, `delete`, `move_path`, `upload`, `export_zip`, `extract_archive`): call a `core::fs_ops` function or a shared tool-family function identical to the one their MCP tool counterpart calls. | `crates/core/src/api/dataplane.rs:370-472, 630-682` and the read/write handlers elsewhere in the same file |
| OpenAPI generator | Builds the document at request time by joining the documented operation table against the live tool registry. | `crates/core/src/api/openapi.rs` |
| Documented operation table (`OPERATIONS`) | The per-route documentation metadata: HTTP method, path template, tool-name mapping (or override), and the route's own parameter list. | `crates/core/src/api/openapi.rs:519-...` |
| Explicit-summary tables (`REST_ONLY`, `REST_ONLY_PARAMS`) | Hand-written summaries and parameter descriptions for the 3 routes with no tool counterpart. | `crates/core/src/api/openapi.rs:484-512` |
| Swagger UI server | Serves the embedded documentation UI assets and the initializer that points it at the generated document. | `crates/core/src/api/openapi.rs:1372-1410` |

## 2. Flows

### 2.1 Request through the guard (every scoped route)

1. `person_of` resolves identity from the request headers via the platform's resolver.
2. `AppState::authorize(mount, person)` checks project membership; a platform-admin role grants nothing extra here.
3. `StoreManager::client(mount)` opens the project's storage handle.
4. The handler closure runs with a `Req` bundling state, person, mount and the open client.
5. Any error at any step short-circuits to `unauthorized` (step 1) or `error_response` (steps 2-4 and the handler itself).

### 2.2 Mutating engine-parity route (e.g. `delete`)

1. Guard runs (§2.1).
2. Request body parsed into typed args via the shared `Args` helper.
3. Path normalized through the project's `SafetyManager`.
4. For `delete` and `move_path` specifically: the set of affected paths is collected from the search indexer **before** the engine call, because a recursive delete leaves nothing to enumerate afterward.
5. The shared `core::fs_ops` function is called (`delete_path`, `move_path`, `mkdir`) with the same parameters its MCP tool accepts.
6. The search indexer's `after_delete_many` / `after_move` hook runs with the pre-collected path set, mirroring exactly what the MCP tool triggers.
7. The engine's result is wrapped as JSON and returned.

### 2.3 Multipart upload

1. Guard runs (§2.1).
2. The entire multipart form is buffered before any file is matched to a destination path, so `directory` and `paths` fields may appear before or after the file parts.
3. If the documentation flag is set, eligibility of every file in the batch is checked before any file is written; a mixed batch with one ineligible file is refused wholesale.
4. Each file is written via the engine's write function (shared with the write tool), charging the quota and writing an audit entry per file.
5. When the documentation flag is honored, a companion conversion is requested per eligible file after the write.

### 2.4 Storage-direct download

1. Guard runs (§2.1), but the resulting `client` is used directly rather than passed to `core::fs_ops`.
2. `download`: existence/file-type check via `client.is_file`, then `client.read_bytes`; content type via `docs::guess_mime`; filename via `PosixPath::basename`.
3. `download_zip`: `client.walk` enumerates the subtree; each file's bytes are read via `client.read_bytes` and written into an in-memory zip with entry names relativized against the requested root.
4. Neither path touches the engine's session-read bookkeeping, so neither satisfies the read-before-edit guard.

### 2.5 OpenAPI document generation

1. `swagger_json` iterates the `OPERATIONS` table.
2. For each entry, the tool name (explicit on the entry, with two name overrides: `roots` → `fs.list_allowed_roots`, `list` → `fs.list_dir`) is looked up in the live `ToolRegistry`.
3. If found, the tool's top-level description becomes the route's summary, and per-parameter descriptions are matched by name against the tool's input schema.
4. If not found (the 3 routes with no tool), the summary and parameter descriptions come from `REST_ONLY` / `REST_ONLY_PARAMS`.
5. The assembled document is returned as JSON with no caching.

## 3. Interfaces

| Interface | Shape | Consumed by |
|---|---|---|
| Error body | `{"error": "<CODE>", "detail": "<CODE>: <message>"}` | Every handler's failure path via `error_response` / `unauthorized` |
| Storage-direct 404 | `{"detail": "<reason>"}` | `download` (not-a-file case) |
| Attachment response | Body bytes plus `Content-Type` and `Content-Disposition` (ASCII filename + RFC 5987 `filename*`) | `download`, `download_zip` |
| Upload multipart form | File parts; `directory` (optional); repeated `paths`; `trigger_documentation_service` (optional) | `upload` |
| Generated OpenAPI document | Standard OpenAPI JSON, summaries/descriptions inherited per §2.5 | `swagger_json`, consumed by the embedded Swagger UI |

## 4. Data and state

No persisted entity of its own. This plane holds no state beyond:
- the per-session read-before-edit and write-quota bookkeeping owned by the platform foundation and shared with the MCP surface
- the search index, updated through the same hooks the MCP-surface mutating tools use

## 5. Configuration

Served by the same listener and the same `api` configuration section as the rest of the HTTP surface; no plane-specific configuration keys. The multipart upload body ceiling (128 MiB) and the JSON body ceiling (30 MiB) are compile-time constants, not configuration.

## 6. Observability

- Writes through this plane produce the identical audit-log entries a matching MCP write would produce, since both paths call the same `core::fs_ops` function.
- Expected 4xx-class failures (unauthenticated, forbidden, invalid-argument, etc.) are classified the same way the platform foundation classifies them for the MCP surface.
- No plane-specific metrics or spans beyond what the shared engine and guard emit.

## 7. Decisions

| ID | Decision | Evidence | Rationale |
|---|---|---|---|
| DEC-001 | This document specifies the plumbing of document-conversion routes (text extraction, document writing, the upload documentation flag, standalone documentation) but not their semantics. | `crates/core/src/api/dataplane.rs:147-150` | Those routes' behaviour belongs to the document-conversion area; specifying it here would duplicate that area and risk the two disagreeing. |
| DEC-002 | The REST 401 body and the MCP 401 body use two different shapes, specified as-built rather than unified. | `crates/core/src/api/dataplane.rs:222-226` | Both are frozen wire contracts in independent use; unifying them is a breaking change to at least one client and is a product decision, not a specification one. |
| DEC-003 | Engine parity is specified as a testable property with representative tests across the mutating routes rather than a per-route parity-test pair for every one of the 36 parity routes. | `crates/core/src/api/dataplane.rs:2424-2430` | Parity is the design's whole justification; asserting it in prose without any test is how it quietly stops being true, but a full per-route pairing roughly doubles the test count for little extra signal over a representative walk. |
| DEC-004 | The storage-direct vs. engine-parity split is specified as a deliberate asymmetry (writes go through the engine for accounting; downloads do not go through the engine because they are not agent reads) rather than smoothed into one uniform rule. | `crates/core/src/api/dataplane.rs:3-10`, `579-628` | Hiding the asymmetry would make the read-guard behavior on download look like an undiscovered bug rather than a design choice with a reason on each side. |
| DEC-005 | The documentation-inheritance mechanism (descriptions copied from tool schemas at request time) is specified as a functional requirement rather than described as an implementation detail of the generator. | `crates/core/src/api/openapi.rs:1-24` | It produces an observable, relied-upon property: a poorly described tool yields a poorly described route, which the team uses as a documentation-quality audit. |
| DEC-006 | `mkdir`, `delete` and `move` were migrated from a direct-storage-client implementation to routing through the shared engine function, carried here as a design decision rather than silently absorbed into FR-009's general statement. | `crates/core/src/api/dataplane.rs:365-369, 394-420, 432-461` (in-code rationale comments) | The direct-storage-client version skipped the trash, the hard-delete guard, `recursive` semantics and the audit log; the same logical operation behaving differently depending on which door it came through was the defect this migration fixed. |

## 8. Requirement to code map

| FR | Evidence |
|---|---|
| FR-001 | `crates/core/src/api/dataplane.rs:45-91`, `:108-152`, `:2424` |
| FR-002 | `crates/core/src/api/dataplane.rs:108` vs `:109-152` |
| FR-003 | `crates/core/src/api/dataplane.rs:17-20` |
| FR-004 | `crates/core/src/api/dataplane.rs:95,99,113-116,154` |
| FR-005 | `crates/core/src/api/dataplane.rs:186-206` |
| FR-006 | `crates/core/src/api/dataplane.rs:222-226` |
| FR-007 | `crates/core/src/api/dataplane.rs:228-235` |
| FR-008 | `crates/core/src/api/dataplane.rs:197` |
| FR-009 | `crates/core/src/api/dataplane.rs:365-461`, `crates/core/src/api/openapi.rs:547-565` |
| FR-010 | `crates/core/src/api/dataplane.rs:109-152, 137` |
| FR-011 | `crates/core/src/api/dataplane.rs:473-...` |
| FR-012 | `crates/core/src/api/openapi.rs:503-510` |
| FR-013 | `crates/core/src/api/dataplane.rs:579-596`, `crates/core/src/docs/mod.rs:23`, `crates/core/src/core/fs_ops.rs:1391` |
| FR-014 | `crates/core/src/api/dataplane.rs:600-628` |
| FR-015 | `crates/core/src/api/dataplane.rs:324-363` |
| FR-016 | `crates/core/src/api/dataplane.rs:579-628` |
| FR-017 | `crates/core/src/api/openapi.rs:1-24, 527, 535` |
| FR-018 | `crates/core/src/api/openapi.rs:484-512` |
| FR-019 | `crates/core/src/api/openapi.rs:21-24` |
| FR-020 | `crates/core/src/api/openapi.rs:50-54` |
| FR-021 | `crates/core/src/api/dataplane.rs:2424-2430` |

## 9. Legacy mapping

Source: specs/SPEC-0004_2026-09-18_18-30-00-rest-data-plane/spec.md (pre-move)

| Legacy ID | Current ID | Note |
|---|---|---|
| FR-301 | FR-001 | Route count corrected from 38 to 40 (see Confidence notes in spec.md). |
| FR-302 | FR-002 | Unchanged in substance. |
| FR-303 | FR-003 | Unchanged in substance. |
| FR-304 | FR-004 | Unchanged in substance. |
| FR-305 | FR-005 | Unchanged in substance. |
| FR-306 | FR-006 | Unchanged in substance. |
| FR-307 | FR-007 | Unchanged in substance. |
| FR-308 | FR-008 | Unchanged in substance. |
| FR-309 | FR-009 | Scope widened: `mkdir`/`delete`/`move` now confirmed as engine-parity, not a separate "bytes plane" exception as the legacy draft implied. |
| FR-310 | FR-010 | Unchanged in substance. |
| FR-311 | FR-011 | Unchanged in substance. |
| FR-312 | FR-012 | Unchanged in substance. |
| FR-313 | FR-013 | Unchanged in substance. |
| FR-314 | FR-014 | Unchanged in substance. |
| FR-315 | FR-015 + FR-016 | Split: FR-315's two claims (helpers read via storage client; downloads don't satisfy the read guard) are now two requirements, and the "helpers" set narrowed from 4 (roots/list/download/download-zip) plus mkdir/delete/move to just the first 4. |
| FR-316 | FR-017 | Unchanged in substance. |
| FR-317 | FR-018 | `REST_ONLY` entry count corrected from implying 8 bytes-plane routes to the actual 3. |
| FR-318 | FR-019 | Unchanged in substance. |
| FR-319 | FR-020 | Unchanged in substance. |
| FR-320 | FR-021 | Unchanged in substance. |
| DEC-301 | DEC-001 | Unchanged in substance. |
| DEC-302 | DEC-002 | Unchanged in substance. |
| DEC-303 | DEC-003 | Unchanged in substance. |
| DEC-304 | DEC-004 | Unchanged in substance. |
| DEC-305 | DEC-005 | Unchanged in substance. |
| (new) | DEC-006 | New: the legacy draft's "bytes plane" classification of `mkdir`/`delete`/`move` is now recorded as a resolved migration, not a live classification. |
| TBD-301 (legacy) | Design note, §6 of this document (two MIME implementations) | Re-verified current; not reopened as a TBD since no code change was found that would alter the prior audit's conclusion. |
| TBD-302 (legacy) | FR-016 | Promoted from an open question to a stated, deliberate requirement. |
| TBD-303 (legacy) | Not carried forward | Unmeasured performance cost of per-request OpenAPI generation; no evidence found that this was ever measured or became a problem; dropped rather than carried as a stale TBD. |
