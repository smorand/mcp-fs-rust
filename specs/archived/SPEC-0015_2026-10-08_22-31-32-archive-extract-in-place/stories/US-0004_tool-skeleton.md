---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: none
Complexity: S
min_tier: 2
Files touched: crates/core/src/tools/archive.rs (new), crates/core/src/tools/mod.rs, crates/core/src/mcp/server.rs
---

# US-0004 — Tool skeleton: registration, authorize, path resolve, directory rejection

## Objective

Create the new `crates/core/src/tools/archive.rs` module with the `extract_archive` function
signature, register it in `tools/mod.rs`, and implement the first three steps every `fs.*` tool
follows: `state.authorize(mount_id, person)`, path normalization via
`SafetyManager::normalize_path` plus a `client.meta.get` not-found check, and a directory
rejection. This is the skeleton: format detection and decode land in later stories.

## Technical Context

### Stack

Rust 2024, no new dependency in this story (format-detection and archive-decode crates land in
US-0006). This story is pure plumbing over existing `AppState`/`SafetyManager`/`VolumeClient`
primitives.

### Relevant File Structure

- `crates/core/src/tools/export.rs:1-9,58-104` — the structural template: one typed
  `pub(crate) async fn export_zip(state: &AppState, mount_id: &str, paths: &[String]) ->
  Result<Value>`, called by both the MCP tool and the REST route.
- `crates/core/src/tools/mod.rs` — registers the `export` module; this story adds `archive`
  alongside it.
- `crates/core/src/safety.rs:89-106` — `SafetyManager::normalize_path` (clamps, does not reject,
  an escaping path).
- `crates/core/src/core/fs_ops.rs` — other `fs.*` engine functions follow the
  authorize-then-normalize-then-engine shape this story mirrors.

### Existing Patterns Referenced

Every `fs.*`/`git.*` handler: `state.authorize(mount_id, person)` first, then normalize every
path through `state.safety.normalize_path`, then call the engine (`AGENTS.md`, Conventions
section). This story is the first half of that shape for `fs.extract_archive`; the engine call
in this story is only the not-found/directory checks, with format detection deferred to US-0005.

### Data Model

No new persisted schema (spec §8). This story reads existing `nodes` rows via
`client.meta.get`, writes nothing.

### Decisions That Govern This Story

None of the 11 decisions in §17 specifically govern the skeleton; `FR-NEW-001` through
`FR-NEW-004` are the governing requirements.

### Applicable NFRs

None beyond the existing `fs.*` authorization convention (§6, implicit in every tool).

### Bounded Context

Transport (`mcp::server` + the new `tools::archive` module boundary) for the registration shape;
the not-found/directory checks themselves belong to no single bounded context listed in §4.5 —
they are the shared precondition every later context assumes has already run.

## Functional Requirements

**FR-NEW-001** [EARS-U] The system SHALL expose a tool named `fs.extract_archive` taking
`mount_id:string` (required), `path:string` (required, the archive's absolute POSIX path),
`destination:string` (optional), `overwrite:boolean` (optional, default `false`), and
`password:string` (optional), served identically by the MCP `#[tool]` method
`McpServer::fs_extract_archive` and the REST route `POST /api/fs/{mount_id}/extract-archive`,
both calling one `pub(crate) async fn extract_archive(state: &AppState, mount_id: &str, path:
&str, destination: Option<&str>, overwrite: bool, password: Option<&str>) -> Result<Value>` in a
new module `crates/core/src/tools/archive.rs`, mirroring `tools::export::export_zip`
(`export.rs:58-104`).

- Inputs: `mount_id`, `path` (required); `destination`, `overwrite`, `password` (optional).
- Outputs: the function signature only, in this story (no return value shape yet — that is
  `FR-NEW-024`, in US-0009). This story's acceptance is the signature compiling and being
  callable.
- Business Rules: the MCP tool and the REST route must call the exact same function, never
  reimplementing the operation in either transport layer.

**FR-NEW-002** [EARS-E] WHEN `fs.extract_archive` is called THE system SHALL call
`state.authorize(mount_id, person)` (or the REST plane's equivalent `authorize_only`) before any
other step, exactly as every other `fs.*` tool does.

- Inputs: `mount_id`, the calling `person`.
- Outputs: `Err` propagated from `authorize` on failure (e.g. `ERR_FORBIDDEN`), before any other
  step runs.
- Business Rules: authorization happens before path normalization, before the not-found check,
  before everything else.

**FR-NEW-003** [EARS-E] WHEN `path` is normalized THE system SHALL use
`SafetyManager::normalize_path`, then look up the node via `client.meta.get`; IF the node does not
exist THEN THE system SHALL fail with `ERR_NOT_FOUND`, message `"'{path}' not found"`.

- Inputs: raw `path`.
- Outputs: the normalized path, or `Err(ToolError::not_found(...))`.
- Business Rules: normalization happens after authorization, before the directory check.

**FR-NEW-004** [EARS-O] IF the node at the normalized `path` is a directory THEN THE system SHALL
fail with `ERR_INVALID_ARGUMENT`, message `"'{path}' is a directory, not an archive file"`.

- Inputs: the normalized path's node kind.
- Outputs: `Err(ToolError::invalid_argument(...))` when the node is a directory.
- Business Rules: this check runs after the not-found check and before format detection
  (US-0005). It applies uniformly including to the volume root `/` (no special-casing).

## Acceptance Tests

### Test Data

| path | node exists | node kind | expected |
|---|---|---|---|
| `/uploads/missing.zip` | no | n/a | `ERR_NOT_FOUND` |
| `/uploads/adir` | yes | directory | `ERR_INVALID_ARGUMENT` |
| `/uploads/a/b` | yes | directory (nested) | `ERR_INVALID_ARGUMENT` |
| `/` | yes | directory (root) | `ERR_INVALID_ARGUMENT` |

#### E2E-NEW-025 — `path` points at a directory, not a file

- Category: Failure
- Scenario: (FR-only)
- Requirements: FR-NEW-004
- Preconditions: `/uploads/adir` exists as a directory
- Steps:
  - Given: the fixture
  - When: `extract_archive(.., path="/uploads/adir", ..)`
  - Then: `Err` with `code == "ERR_INVALID_ARGUMENT"`
  - And: `message` contains `"'/uploads/adir' is a directory"`
- Cleanup: fixture dropped at end of test
- Priority: P1

#### E2E-NEW-059 — `path` points at a directory nested two levels deep

- Category: Failure
- Scenario: (FR-only)
- Requirements: FR-NEW-004
- Preconditions: `/uploads/a/b` exists as a directory, two levels deep
- Steps:
  - Given: the fixture
  - When: `extract_archive(.., path="/uploads/a/b", ..)`
  - Then: `Err` with `code == "ERR_INVALID_ARGUMENT"`
  - And: `message` contains `"'/uploads/a/b' is a directory"`
- Cleanup: fixture dropped at end of test
- Priority: P2

#### E2E-NEW-060 — `path` is `/` itself (the volume root), rejected the same way

- Category: Edge
- Scenario: (FR-only)
- Requirements: FR-NEW-004
- Preconditions: the project volume's root `/` (always a directory)
- Steps:
  - Given: the fixture
  - When: `extract_archive(.., path="/", ..)`
  - Then: `Err` with `code == "ERR_INVALID_ARGUMENT"`, the same rejection as any other directory
    path, proving the root is not special-cased
- Cleanup: none
- Priority: P2

## Constraints

- Files Not to Touch: no format-detection logic, no decode crates (US-0005/US-0006). No REST
  route, no OpenAPI schema (US-0011).
- Dependencies Not to Add: none of the five archive crates yet.
- Patterns to Avoid: do not reimplement `authorize`/`normalize_path` locally; call the existing
  `AppState`/`SafetyManager` methods directly.
- Scope Boundary: authorize, normalize, not-found, directory-rejection only.

## Non Regression

No existing `fs.*` tool's behavior changes. The MCP tool registration must not shift any existing
tool's position in a way that breaks the running tool-count test (that bump itself is US-0012's
job, at the end).

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
