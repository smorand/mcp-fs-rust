---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: US-0009
min_tier: 2
Complexity: S
Files touched: crates/core/src/api/dataplane.rs, crates/core/src/api/openapi.rs
---

# US-0011 — REST route + OpenAPI schema

## Objective

Add the REST door `POST /api/fs/{mount_id}/extract-archive`, mirroring the existing
`export-zip` route exactly, calling the same `tools::archive::extract_archive` function the MCP
tool calls, plus the corresponding OpenAPI `Op` entry and `ExtractArchiveBody` schema. Can run
parallel to US-0010.

## Technical Context

### Stack

Rust 2024, axum. No new dependency.

### Relevant File Structure

- `crates/core/src/api/dataplane.rs:149` — the existing `export-zip` route registration; this
  story adds `extract-archive` alongside it.
- `crates/core/src/api/dataplane.rs:633-648` — the `export_zip` handler, the structural
  template for the new `extract_archive` handler.
- `crates/core/src/api/dataplane.rs:87`-area — the `REST_ROUTES` list; this story adds
  `extract-archive` to it.
- `crates/core/src/api/openapi.rs:1016-1021,1259-1268` — `ExportZipBody`'s schema entry and the
  `export-zip` `Op` list entry; this story adds `ExtractArchiveBody` and the `extract-archive`
  `Op` entry in the same shape.

### Existing Patterns Referenced

The MCP surface and the REST plane MUST share one implementation (`AGENTS.md`, Conventions):
this story's handler parses the JSON body and calls `tools::archive::extract_archive` directly,
never reimplementing any check.

### Decisions That Govern This Story

None of §17's 11 decisions specifically govern the REST door; `FR-NEW-029` is the governing
requirement, following the `export_zip`/`export-zip` precedent referenced throughout the spec.

### Applicable NFRs

None beyond the existing REST-plane conventions (versioned route, sanitized 5xx body, per the
`rust` skill's HTTP APIs section).

### Bounded Context

Transport (`mcp::server` + `api::dataplane` + `api::openapi`, extended not owned, §4.5): this
story is entirely within this context, adding the REST half of the two doors `FR-NEW-001`
already names.

## Functional Requirements

**FR-NEW-029** [EARS-U] The system SHALL add the REST route `POST /api/fs/{mount_id}/
extract-archive` to `crates/core/src/api/dataplane.rs`'s router (alongside the existing
`export-zip` entry at `dataplane.rs:149`) and to its `REST_ROUTES` list (`dataplane.rs:87`-area),
backed by a handler that parses a JSON body `{"path": string, "destination": string|null,
"overwrite": boolean|null, "password": string|null}` and calls the same `tools::archive::
extract_archive` function as the MCP tool, named `ExtractArchiveBody` in
`crates/core/src/api/openapi.rs`'s schema list and `extract-archive` in its `Op` list, mirroring
`ExportZipBody` and the `export-zip` `Op` entry (`openapi.rs:1016-1021`, `openapi.rs:1259-1268`).

- Inputs: the JSON request body, the `mount_id` path parameter, the caller's auth context.
- Outputs: HTTP 200 with the `FR-NEW-024` response body on success; the mapped HTTP status for
  any `ToolError` on failure; HTTP 400 for a malformed body.
- Business Rules: the handler calls `tools::archive::extract_archive` directly, no
  reimplementation; unauthenticated/non-member requests get the same treatment `fs.read` gets
  under the same condition, verified through the real router.

## Acceptance Tests

### Test Data

| Request | expected |
|---|---|
| `{"path": "/uploads/report.tar.gz"}` | 200, `{destination, files_written:2, dirs_created:1, bytes_written:10}` |
| body missing `path` | 400 |
| no bearer token | same status `fs.read` returns under the same condition |
| bearer for non-member | `ERR_FORBIDDEN`'s status |

#### E2E-NEW-031 — REST `POST extract-archive` happy path

- Category: Happy. Requirements: FR-NEW-029.
- Driver: `crates/core/src/api/dataplane.rs`'s own REST test harness (the same `h.post`/fixture
  helpers the `export-zip` tests use).
- Preconditions: same fixture shape as E2E-NEW-001, served over the REST plane.
- Steps: Given the fixture, When `h.post(&u("extract-archive"), json!({"path":
  "/uploads/report.tar.gz"}))`, Then HTTP 200 And the JSON body matches `{"destination":
  "/uploads/report", "files_written": 2, "dirs_created": 1, "bytes_written": 10}`.
- Cleanup: none. Priority: P0.

#### E2E-NEW-032 — REST route: malformed body is 400; unauthenticated/stranger matches `fs.read`

- Category: Failure. Requirements: FR-NEW-029.
- Preconditions: as E2E-NEW-031.
- Steps: (a) Given a body missing `path`, When posted, Then HTTP 400. (b) Given no bearer
  token, When posted, Then the same status and `error` code `fs.read` would return under the
  same condition (cross-checked through the real router, not hardcoded, mirroring
  `e2e_new_041_unauthenticated_call_matches_other_tools` in `export.rs`). (c) Given a bearer for
  a person who is not a member of the project, When posted, Then `ERR_FORBIDDEN`'s status.
- Cleanup: none. Priority: P0.

#### E2E-NEW-047 — OpenAPI includes the `extract-archive` path and `ExtractArchiveBody` schema

- Category: Happy. Requirements: FR-NEW-029.
- Driver: a `#[test]` calling the OpenAPI spec-generation function directly (the same one `GET
  /api/swagger.json` serves).
- Steps: Given the generated spec, When inspected, Then it contains a path entry for
  `/api/fs/{mount_id}/extract-archive` with method `POST` And a schema named
  `ExtractArchiveBody` with required field `path` and optional fields `destination`,
  `overwrite`, `password`.
- Cleanup: none. Priority: P1.

## Constraints

- Files Not to Touch: `tools/archive.rs` logic (US-0004 through US-0010, already complete by
  this story's start); tool contract regeneration (US-0012).
- Dependencies Not to Add: none.
- Patterns to Avoid: do not reimplement any `extract_archive` check in the REST handler; parse
  the body and delegate.
- Scope Boundary: REST route, handler, OpenAPI schema/Op entry only.

## Non Regression

The existing `export-zip` route, handler, and OpenAPI entries are unmodified; this story adds a
new, parallel entry following the same shape.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
