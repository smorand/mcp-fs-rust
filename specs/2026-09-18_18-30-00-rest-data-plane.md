# mcp-fs REST Data Plane and OpenAPI — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

**ID convention.** S3 of eight. S3 owns the `3xx` block: `SC-3xx`, `FR-3xx`, `E2E-3xx`, `DEC-3xx`, `EXC-3xx`. S1 owns `0xx`, S2 owns `2xx`.

## 1. Executive Summary

This document specifies the **REST data plane** at `/api/fs` and the OpenAPI document that describes it. It is the server's second transport: 38 HTTP routes over the same engine the MCP tools call, plus a bytes plane (upload, download, zip) that HTTP can serve and the MCP protocol cannot.

Two properties define this layer, and both are structural rather than cosmetic. First, **parity**: every tool-equivalent route calls the same `core::fs_ops` function as its MCP counterpart, so the two surfaces cannot drift. Second, **single source of truth for documentation**: not one summary or parameter description is written on a REST endpoint; every one is copied at request time from the MCP tool schemas. Documenting a tool documents the REST API, and a blank description on the Swagger page is a blank description in the tool schema.

Retro-specification of shipped behaviour; every claim carries a `file:LINE` citation. Builds on S1 (identity, error mapping, authorization) and S2 (the engine and its semantics).

## 2. Current State Analysis

### 2.1 Project Overview

`api/dataplane.rs` is 2622 lines holding the router, one guard and 38 handlers. `api/openapi.rs` is 1724 lines generating the spec by joining a route table against the live tool registry. Both endpoints of the documentation pair are public, because a client that cannot yet authenticate still needs to read how to authenticate (`api/openapi.rs:26-29`).

### 2.2 Existing Specifications

- **S1** `2026-09-18_17-37-46-platform-foundation.md`: identity resolution (FR-007..FR-011), the error vocabulary and HTTP mapping (FR-020), the membership gate (FR-021, FR-022), path normalization (FR-032), the safety contract (FR-034..FR-037).
- **S2** `2026-09-18_18-05-00-filesystem-engine.md`: the engine and every filesystem semantic this plane exposes. FR-201 (engine is the only implementation) is the requirement this spec discharges on the REST side.

### 2.3 Relevant Architecture

- **Router**: `api/dataplane.rs:103-148`, 38 routes under `/api/fs`.
- **Route manifest**: `REST_ROUTES` (`api/dataplane.rs:45-91`), the `(method, last segment)` list the OpenAPI table is checked against.
- **Guard**: `guarded` (`api/dataplane.rs:182-202`) and `guarded_json` (`:205-216`).
- **Error mapping**: `error_response` (`api/dataplane.rs:225-232`), `unauthorized` (`:219-223`).
- **Documentation**: `api/openapi.rs`, with `REST_ONLY` (`:425`) and `REST_ONLY_PARAMS` (`:432`) covering the bytes plane, which has no tool to inherit from.

## 3. Scope

### 3.1 In Scope

- The 38 routes of `/api/fs`, their methods, path shapes and parameter names.
- The **bytes plane**: `roots`, `list`, `mkdir`, `delete`, `move`, `upload`, `download`, `download-zip`.
- The **tool-parity plane**: the 30 routes that call `core::fs_ops` directly.
- The shared guard: bearer verification, then membership, then the operation.
- Error mapping from `ToolError` to HTTP status and body shape.
- Body size limits for JSON and for multipart upload.
- The OpenAPI document at `/api/swagger.json`, its generation from the tool registry, the `REST_ONLY` overrides, and the Swagger UI at `/api/docs`.
- The route/table coverage invariant that prevents an undocumented route from shipping.

### 3.2 Out of Scope (Non-Goals)

- The filesystem semantics themselves (S2). This spec specifies that `/api/fs/{mount_id}/edit` calls `fs_ops::edit_unique` and maps its errors; what `edit_unique` does is S2 FR-214.
- Identity verification and the error vocabulary (S1).
- The **semantics** of the four document routes `extract-text`, `write-docx`, `write-bytes`'s `trigger_documentation_service` flag, and `documentize` (S5). S3 specifies their plumbing: route, method, guard, parameter names and error mapping.
- Search indexing triggered by writes through this plane (S7).
- Any authentication scheme other than the bearer token S1 specifies.

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **Web client** | A browser or application uploading and downloading files. The only consumer of the data plane that needs the bytes plane; multipart upload and zip download exist for it. |
| **REST integrator** | A program automating filesystem operations over HTTP rather than MCP. Reads the Swagger page to learn the surface. |
| **API documentation reader** | Anyone loading `/api/docs`, including before they hold a token. |
| **Project member** | The authenticated identity behind every call, subject to the same ACL and session rules as on the MCP surface. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Transport** | HTTP routing, status codes, content types, body limits. | route, status, Content-Disposition |
| **Bytes plane** | Operations that move raw bytes and have no MCP tool. | upload part, attachment, zip entry |
| **Documentation** | The generated OpenAPI document and the UI serving it. | operation, schema, REST_ONLY entry |
| **Volume**, **Session** (from S1) | Unchanged; this plane is another way in. | NodeRow, quota, audit entry |

A "route" in the Transport context maps to a "tool" in S1's Protocol context by name derivation; the two names differ by separator (`read-bytes` against `fs.read_bytes`) and that mapping is normative here.

## 5. Usage Scenarios

### SC-301: Web client uploads files into a project

**Actor:** Web client
**Preconditions:** the caller holds a valid bearer; the caller is a member of the project.
**Flow:**
1. Client POSTs `multipart/form-data` to `/api/fs/{mount_id}/upload`.
2. The guard verifies the bearer, then membership, then opens the volume client (`dataplane.rs:188-199`).
3. The whole form is read before files are paired with their relative paths, so part order does not matter (`dataplane.rs:~+12`).
4. Fields are interpreted: `directory` (default `/`), repeated `paths`, and `trigger_documentation_service`.
5. Each file is written through `fs_ops::write_bytes`, so the quota is charged and an audit entry recorded.

**Postconditions:** the files exist in the volume; the session's `bytes_written` has advanced; every write is auditable through `fs.audit_log`.
**Exceptions:**
- EXC-301a: no bearer → 401 with `{"error":"ERR_UNAUTHENTICATED","detail":"..."}` (`dataplane.rs:219-223`)
- EXC-301b: not a member → 403 with `ERR_FORBIDDEN`
- EXC-301c: unknown project → 404 with `ERR_PROJECT_NOT_FOUND`
- EXC-301d: malformed multipart → `ERR_INVALID_ARGUMENT`, `"invalid multipart body: {e}"`
- EXC-301e: body above 128 MiB → rejected by the body limit layer (`dataplane.rs:96,110-113`)
- EXC-301f: quota exhausted → 429 `ERR_WRITE_QUOTA_EXCEEDED`

### SC-302: Web client downloads a file and a subtree

**Actor:** Web client
**Preconditions:** authenticated member; the path exists.
**Flow:**
1. Client GETs `/api/fs/{mount_id}/download?path=/report.pdf`.
2. The handler verifies the path is a file, reads the bytes, guesses the MIME type and returns an attachment (`dataplane.rs` `download`).
3. Client GETs `/api/fs/{mount_id}/download-zip?path=/src` for a subtree, whose entry names are relative to the requested root.

**Postconditions:** the client holds the bytes with a filename and content type; no read is recorded against the session, because the bytes plane talks to the volume client rather than the engine.
**Exceptions:**
- EXC-302a: the path is a directory → 404 with `{"detail":"not a file: {path}"}`
- EXC-302b: the path does not exist → 404
- EXC-302c: an unknown extension → `application/octet-stream`

### SC-303: REST integrator performs a tool-equivalent operation

**Actor:** REST integrator
**Preconditions:** authenticated member.
**Flow:**
1. Integrator GETs `/api/fs/{mount_id}/read?path=/a.txt&offset_lines=0&limit_lines=100`.
2. The guard runs, the path is normalized, and `fs_ops::read_window` is called, exactly as the MCP tool does.
3. The engine result is returned as JSON with the same keys the tool returns.
4. Integrator POSTs `/api/fs/{mount_id}/edit` with a JSON body using the same parameter names as the tool.

**Postconditions:** the observable effect is indistinguishable from the same operation performed over MCP: same result keys, same session accounting, same audit entry.
**Exceptions:**
- EXC-303a: a missing required parameter → 400 `ERR_INVALID_ARGUMENT`
- EXC-303b: any engine error → the status from S1's mapping, with the stable `ERR_*` code in the body
- EXC-303c: a JSON body above 30 MiB → rejected by the body limit (`dataplane.rs:92`)
- EXC-303d: editing without a prior read in this session → 428 `ERR_EDIT_WITHOUT_PRIOR_READ`

### SC-304: Integrator discovers the API through Swagger

**Actor:** API documentation reader
**Preconditions:** the server is running. **No token is required.**
**Flow:**
1. Reader GETs `/api/docs` and receives the Swagger UI page, served from assets embedded in the binary.
2. The UI fetches `/api/swagger.json`.
3. The document is generated by joining the route table against the live tool registry, deriving `fs.{sub_with_underscores}` from each route's last segment (`openapi.rs:6-9`).
4. Bytes-plane routes, having no tool, take their summaries from `REST_ONLY` and their parameter descriptions from `REST_ONLY_PARAMS` (`openapi.rs:425,432`).

**Postconditions:** the reader holds a complete description of the 38 routes, with descriptions identical to the LLM-facing tool documentation.
**Exceptions:**
- EXC-304a: a tool schema with a blank description → a blank description on the page, deliberately, because the page is an audit of how well the tools are documented
- EXC-304b: a route with no matching tool and no `REST_ONLY` entry → caught by the coverage test before it ships

### SC-305: Operator verifies the two surfaces cannot drift

**Actor:** Tool author, at build time
**Preconditions:** a change to a route or a tool.
**Flow:**
1. A new route is added to the router and to `REST_ROUTES`.
2. The OpenAPI table is checked against `REST_ROUTES` by a test (`dataplane.rs:41-44`).
3. Parameter names on the route are required to equal the tool's parameter names.

**Postconditions:** an undocumented route cannot ship; a renamed tool parameter surfaces as a documentation mismatch rather than as silent drift.
**Exceptions:**
- EXC-305a: a route added to the router but not to `REST_ROUTES` → the coverage test fails
- EXC-305b: a REST parameter renamed away from its tool name → the generated description is lost for that parameter

## 6. Functional Requirements

### Routing and transport

#### FR-301 [EARS-U]: The route manifest
> The server SHALL serve exactly 38 routes under `/api/fs`, and every route SHALL appear in the `REST_ROUTES` manifest.

- **Inputs:** an HTTP request under `/api/fs`.
- **Outputs:** the routed handler, or 404 from the router.
- **Business Rules:** the manifest lists `(method, last segment)` pairs (`dataplane.rs:45-91`); the OpenAPI table is keyed by the same pairs and a test asserts the two lists match, so a route cannot ship undocumented (`dataplane.rs:41-44`). The eight bytes-plane routes are `roots`, `list`, `mkdir`, `delete`, `move`, `upload`, `download`, `download-zip`; the other 30 are tool parity.
- **Priority:** Must-have

#### FR-302 [EARS-U]: Path shape and mount scoping
> Every route except `roots` SHALL carry the project as the path parameter `{mount_id}`, in the form `/api/fs/{mount_id}/{operation}`.

- **Inputs:** the request path.
- **Outputs:** the extracted mount id (`dataplane.rs:105-147`).
- **Business Rules:** `/api/fs/roots` is the single unscoped route, because it answers which mounts the caller can reach at all.
- **Priority:** Must-have

#### FR-303 [EARS-U]: REST parameter names equal tool parameter names
> Every query and body parameter SHALL be spelled exactly as the corresponding MCP tool parameter, in snake_case.

- **Inputs:** any request parameter.
- **Outputs:** the parsed argument.
- **Business Rules:** this is load-bearing rather than stylistic: the OpenAPI document inherits descriptions by matching these names against the tool schemas, so `/read-bytes` takes `offset_bytes` and `length_bytes` rather than shorter spellings (`dataplane.rs:17-20`). Route segments use hyphens while parameters use underscores.
- **Priority:** Must-have

#### FR-304 [EARS-E]: Body size limits
> WHEN a request body exceeds its limit THE server SHALL reject it, applying 30 MiB to JSON bodies and 128 MiB to the multipart upload route.

- **Inputs:** the request body.
- **Outputs:** the rejection from the body limit layer.
- **Business Rules:** `JSON_BODY_LIMIT` is 30 MiB (`dataplane.rs:92`); `UPLOAD_BODY_LIMIT` is 128 MiB and is applied as a per-route layer on `upload` only (`dataplane.rs:96,110-113`).
- **Priority:** Must-have

### The guard

#### FR-305 [EARS-E]: Guard order
> WHEN any `/api/fs` request is handled THE server SHALL verify the bearer first, then the project membership, then open the volume, and only then run the operation.

- **Inputs:** the request headers and the mount id.
- **Outputs:** the handler result, or the first failure encountered (`dataplane.rs:182-202`).
- **Business Rules:** the order matters for what a caller can learn: an unauthenticated caller never discovers whether a project exists. Identity resolution reuses S1's resolver against the same headers (`dataplane.rs:175-180`).
- **Priority:** Must-have

#### FR-306 [EARS-O]: Unauthenticated response shape
> IF bearer verification fails THEN the server SHALL answer 401 with the body `{"error":"<CODE>","detail":"<CODE>: <message>"}`.

- **Inputs:** the resolution failure.
- **Outputs:** the 401 JSON body (`dataplane.rs:219-223`).
- **Business Rules:** `detail` carries the full `CODE: message` rendering, so it repeats the code. This differs from the MCP surface's 401, where `detail` is the bare message (S1 FR-011); the difference is deliberate and verified against the reference server.
- **Priority:** Must-have

#### FR-307 [EARS-E]: Error status mapping
> WHEN an operation returns a `ToolError` THE server SHALL answer with the status from S1's mapping and a body carrying the stable `ERR_*` code.

- **Inputs:** the `ToolError`.
- **Outputs:** `{"error":"<CODE>","detail":"<CODE>: <message>"}` at the mapped status (`dataplane.rs:225-232`).
- **Business Rules:** an unmappable status falls back to 500. The body keeps the code so a REST client can branch on the same vocabulary an MCP client sees.
- **Priority:** Must-have

#### FR-308 [EARS-U]: The membership gate applies identically on both surfaces
> The REST plane SHALL authorize by project membership through the same `AppState::authorize` the MCP tools use, and SHALL NOT grant a platform admin access on the basis of the admin role.

- **Inputs:** mount id, caller identity.
- **Outputs:** authorization, or 403/404 (`dataplane.rs:193`).
- **Business Rules:** this restates S1 FR-021 and FR-022 as binding on this transport. A second implementation of the gate is a second place to get it wrong, so there is exactly one.
- **Priority:** Must-have

### Parity plane

#### FR-309 [EARS-U]: Tool-parity routes call the engine
> Each of the 30 tool-parity routes SHALL call the same `core::fs_ops` function as its MCP counterpart and SHALL return the same result keys.

- **Inputs:** the route's parameters.
- **Outputs:** the engine's JSON value (`dataplane.rs:205-216`).
- **Business Rules:** this discharges S2 FR-201 on the REST side. The plane performs no filesystem logic of its own: a handler normalizes paths and forwards. The consequence is that session accounting, the read guard, the quota and the audit log behave identically whichever transport is used.
- **Priority:** Must-have

#### FR-310 [EARS-U]: Method assignment
> The server SHALL serve read-only operations over `GET` and mutating operations over `POST`.

- **Inputs:** the operation.
- **Outputs:** the route method (`dataplane.rs:105-147`).
- **Business Rules:** `read-many` is `POST` despite being read-only, because its `paths` array does not fit a query string. `mkdir`, `delete` and `move` are `POST` although they belong to the bytes plane.
- **Priority:** Must-have

### Bytes plane

#### FR-311 [EARS-E]: Multipart upload
> WHEN a multipart form is uploaded THE server SHALL read the whole form before pairing files with their relative paths, and SHALL write each file through the engine.

- **Inputs:** file parts; the fields `directory` (default `/`), repeated `paths`, and `trigger_documentation_service`.
- **Outputs:** the JSON upload report.
- **Business Rules:** reading the whole form first makes part order irrelevant. Writing through `fs_ops::write_bytes` rather than the volume client is what gives upload its quota charge and audit entry; the direct-to-client version charged nothing and left no trace (S2 FR-201, `core/fs_ops.rs:573-578`).
- **Priority:** Must-have

#### FR-312 [EARS-O]: The documentation flag is opt-in and strict
> IF the `trigger_documentation_service` form field holds exactly `true` or `1` after trimming THEN the server SHALL request document conversion; otherwise it SHALL NOT.

- **Inputs:** the form field text.
- **Outputs:** the boolean passed to the engine.
- **Business Rules:** anything other than an explicit yes is a no, because a form field is free text and a typo must not silently spend minutes of a converter's time. The conversion behaviour itself is S5's.
- **Priority:** Must-have

#### FR-313 [EARS-E]: File download
> WHEN `download` is called for a file THE server SHALL return the bytes as an attachment with the guessed MIME type and the basename as the filename.

- **Inputs:** `path`.
- **Outputs:** the attachment response.
- **Business Rules:** a path that is not a file yields 404 with `{"detail":"not a file: {path}"}`, the shape the bytes plane uses where it short-circuits instead of raising a tool error (`dataplane.rs:233-236`). MIME guessing here uses `docs::guess_mime` (`docs/mime.rs:10`), a **different** implementation from the one `fs.read_bytes` uses, though the audit confirmed the two tables currently hold identical content (S2 TBD-202).
- **Priority:** Must-have

#### FR-314 [EARS-E]: Subtree download as zip
> WHEN `download-zip` is called THE server SHALL return a zip archive whose entry names are relative to the requested root.

- **Inputs:** `path`.
- **Outputs:** the archive as an attachment.
- **Business Rules:** entry names are relative, so extracting the archive reproduces the subtree rather than the absolute volume path.
- **Priority:** Must-have

#### FR-315 [EARS-U]: Bytes-plane helpers talk to the volume client
> The `roots`, `list`, `download` and `download-zip` routes SHALL read through the volume client rather than the engine, and SHALL therefore record no session read.

- **Inputs:** the route parameters.
- **Outputs:** the C#-compatible shapes (`dataplane.rs:3-7`).
- **Business Rules:** this is a deliberate asymmetry with the parity plane: downloading a file does not satisfy the read guard for a later edit. Writes are the opposite: `upload` goes through the engine precisely so accounting applies.
- **Priority:** Must-have

### Documentation

#### FR-316 [EARS-E]: The OpenAPI document is generated from the tool registry
> WHEN `/api/swagger.json` is requested THE server SHALL build the document at request time by matching each route to `fs.{sub_with_underscores}` and copying that tool's summary and parameter descriptions.

- **Inputs:** the live `ToolRegistry`; the route table.
- **Outputs:** the OpenAPI JSON (`openapi.rs:5-10`).
- **Business Rules:** two name overrides exist: `list` maps to `fs.list_dir` and `roots` maps to `fs.list_allowed_roots` (`openapi.rs:468,476`). Not one description is written on a REST endpoint, so documenting a tool documents the API.
- **Priority:** Must-have

#### FR-317 [EARS-O]: Routes without a tool carry explicit summaries
> IF a route has no corresponding tool THEN its summary SHALL come from `REST_ONLY` and its parameter descriptions from `REST_ONLY_PARAMS`.

- **Inputs:** the route's last segment.
- **Outputs:** the summary and parameter text (`openapi.rs:170,179,425,432`).
- **Business Rules:** this covers the bytes plane, which has no MCP counterpart.
- **Priority:** Must-have

#### FR-318 [EARS-U]: The REST schema is not the tool schema
> The OpenAPI parameter list for a route SHALL come from the route table rather than from the tool's input schema.

- **Inputs:** the route table entry.
- **Outputs:** the documented parameters (`openapi.rs:21-24`).
- **Business Rules:** the surfaces genuinely differ: `/api/fs/{mount_id}/mkdir` takes only `path` while `fs.mkdir` also takes `parents` and `exist_ok`. Descriptions are inherited; parameter lists are not.
- **Priority:** Must-have

#### FR-319 [EARS-U]: The documentation endpoints are unauthenticated
> The server SHALL serve `/api/swagger.json` and `/api/docs` without requiring a bearer token.

- **Inputs:** an unauthenticated request.
- **Outputs:** the document and the UI page (`openapi.rs:26-29`).
- **Business Rules:** a client that cannot authenticate still needs to read how to authenticate. The Swagger UI assets are served from the copy embedded in `utoipa-swagger-ui`, so the page works with no network access.
- **Priority:** Must-have

#### FR-320 [EARS-UB]: No route ships undocumented
> The server SHALL NOT expose a route under `/api/fs` that is absent from the OpenAPI table.

- **Inputs:** the router, `REST_ROUTES`, the OpenAPI table.
- **Outputs:** a failing test when the three disagree (`dataplane.rs:41-44`).
- **Business Rules:** the invariant is enforced at build time rather than reviewed by hand, which is what makes it hold.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Downloads stream from the blob store through the volume client; the ranged read used by `fs.read_bytes` is available to avoid materializing a whole file (S2 FR-207).
- Body limits bound worst-case memory per request: 30 MiB JSON, 128 MiB upload (FR-304).
- The OpenAPI document is generated per request rather than cached; the cost is bounded by the registry size, at most 86 tools.

### 7.2 Security
- The guard order gives an unauthenticated caller no information about project existence (FR-305).
- Platform administration confers no data access on this plane either (FR-308).
- Every path parameter is normalized before use, so the traversal protection of S1 FR-032 covers this transport.
- The documentation endpoints are deliberately public (FR-319); they expose the shape of the API, never data.
- Error bodies carry a stable code and a message, never a stack trace.

### 7.3 Usability
- Parameter names match the tool names exactly, so knowledge transfers between the two surfaces in both directions (FR-303).
- The Swagger page doubles as an audit of LLM-facing documentation quality: a blank description is visible rather than hidden (EXC-304a).
- Downloads carry a filename and a content type, so a browser saves them sensibly.

### 7.4 Reliability
- The plane holds no state of its own; every operation is the engine's, so reliability is inherited from S2 §7.4.
- The route/table coverage test prevents the most likely regression, an endpoint added without documentation.

### 7.5 Observability
Unchanged from S1 §7.5. REST failures map to HTTP statuses, and S1's expected-error classification keeps client-caused 4xx responses at INFO. Writes through this plane produce the same audit entries as MCP writes, which is the point of routing upload through the engine.

### 7.6 Deployment
Served by the same binary and the same listener as the MCP endpoint; no separate port, process or configuration. The `api` config section governs it (`config.rs:420`).

### 7.7 Scalability
Stateless per request apart from the session state S1 owns, so the same replication caveat applies: the read guard and the write quota are per process, and a client whose read landed on one replica can be refused an edit by another (S1 TBD-001).

## 8. Data Model

No persisted entity of its own. Transport-level shapes:

| Shape | Fields | Home |
|---|---|---|
| **Error body** | `{error: "ERR_*", detail: "ERR_*: message"}` | `dataplane.rs:225-232` |
| **Bytes-plane 404** | `{detail: "<reason>"}` | `dataplane.rs:233-236` |
| **Attachment** | body plus `Content-Type` and `Content-Disposition` | `download`, `download-zip` |
| **Upload form** | file parts, `directory`, repeated `paths`, `trigger_documentation_service` | `upload` |
| **OpenAPI operation** | summary and parameter descriptions inherited from a tool | `openapi.rs:160-190` |

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/api/dataplane.rs` | Specified, unchanged | FR-301..FR-315 |
| `crates/mcp-fs/src/api/openapi.rs` | Specified, unchanged | FR-316..FR-320 |
| `crates/mcp-fs/src/core/fs_ops.rs` | Referenced | The implementation every parity route calls |
| `crates/mcp-fs/src/docs/mime.rs` | Referenced | `docs::guess_mime`, used by `download` |
| `tests/functional/scenarios/` | Extended | New REST scenarios per §12 |

### 9.2 Affected Requirements

S1 FR-011 and S3 FR-306 describe two different 401 bodies on two different transports. That is not a contradiction: the MCP 401 carries the bare message in `detail`, the REST 401 carries `CODE: message`. Both are verified against the reference server and both are frozen. Recorded in §13 so the audit does not read it as drift.

S2 FR-201 is discharged here by FR-309.

### 9.3 Affected Tests

| Test location | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/api/dataplane.rs` `#[cfg(test)]` (from line 1339) | guard behaviour, handlers, owner/stranger fixtures | Keep; annotate with ids |
| `crates/mcp-fs/src/api/openapi.rs` `#[cfg(test)]` | route/table coverage, description inheritance | Keep; annotate |
| `tests/functional/scenarios/10_security.sh` | authorization refusals | Extend with REST cases |

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | Documentation index | Reference this spec |
| `.agent_docs/api.md` | REST plane | Cross-reference §6 rather than restate |

### 9.5 Dependencies & Risks

No new dependency, no code change. Risks:

1. **Two MIME implementations remain in use.** `download` calls `docs::guess_mime` while `fs.read_bytes` calls `core::fs_ops::mime_guess` (`core/fs_ops.rs:1286`). The cross-spec audit compared the two tables: both hold exactly 32 extensions with identical values, so a file is reported with the same content type on either route today. The duplication is a regression risk, not a live inconsistency. Recorded as TBD-301.
2. **Parity is asserted, not yet proven test-by-test.** FR-309 claims 30 routes call the same engine functions as their tools. §12 tests a representative subset; a route-by-route parity test is specified as E2E-318 but is the single most expensive test in this spec.

## 10. Documentation Requirements

### 10.1 README.md
Mention `/api/docs` as the discoverable entry point for the REST surface, if it does not already.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: add this spec to the index.
- `.agent_docs/api.md`: point at §6; keep its "OpenAPI is the single source of truth" statement, which §6 FR-316 now formalizes.

### 10.3 docs/*
None required.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-301 | FR-304, FR-311, FR-312 | E2E-301, E2E-302 | E2E-303, E2E-304, E2E-305, E2E-306 | E2E-307, E2E-308, E2E-309 |
| SC-302 | FR-313, FR-314, FR-315 | E2E-310, E2E-311 | E2E-312, E2E-313, E2E-314 | E2E-315, E2E-316, E2E-317 |
| SC-303 | FR-303, FR-309, FR-310 | E2E-318, E2E-319 | E2E-320, E2E-321, E2E-322 | E2E-323, E2E-324, E2E-325 |
| SC-304 | FR-316, FR-317, FR-318, FR-319 | E2E-326, E2E-327 | E2E-328, E2E-329 | E2E-330, E2E-331, E2E-332 |
| SC-305 | FR-301, FR-302, FR-320 | E2E-333 | E2E-334, E2E-335 | E2E-336, E2E-337 |
| SC-303 (guard) | FR-305, FR-306, FR-307, FR-308 | E2E-338 | E2E-339, E2E-340, E2E-341, E2E-342, E2E-343 | E2E-344, E2E-345 |

Per-FR coverage:

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-301 | E2E-333, E2E-334, E2E-336 | FR-311 | E2E-301, E2E-305, E2E-307 |
| FR-302 | E2E-333, E2E-335, E2E-337 | FR-312 | E2E-302, E2E-306, E2E-308 |
| FR-303 | E2E-319, E2E-320, E2E-323 | FR-313 | E2E-310, E2E-312, E2E-315 |
| FR-304 | E2E-303, E2E-304, E2E-309 | FR-314 | E2E-311, E2E-313, E2E-316 |
| FR-305 | E2E-338, E2E-339, E2E-340, E2E-344 | FR-315 | E2E-314, E2E-317, E2E-310 |
| FR-306 | E2E-339, E2E-341, E2E-344 | FR-316 | E2E-326, E2E-328, E2E-330 |
| FR-307 | E2E-342, E2E-343, E2E-345 | FR-317 | E2E-327, E2E-329, E2E-331 |
| FR-308 | E2E-340, E2E-342, E2E-345 | FR-318 | E2E-326, E2E-330, E2E-332 |
| FR-309 | E2E-318, E2E-321, E2E-324 | FR-319 | E2E-327, E2E-329, E2E-332 |
| FR-310 | E2E-319, E2E-322, E2E-325 | FR-320 | E2E-333, E2E-335, E2E-336 |

## 12. End-to-End Test Suite

**Placement.** Handler and guard tests are Rust tests in the `#[cfg(test)]` modules of `api/dataplane.rs` and `api/openapi.rs`, which already hold the `OWNER` / `STRANGER` / `MOUNT` fixtures (`dataplane.rs:1339-1341`). Full-stack tests that need a listening server are shell scenarios under `tests/functional/scenarios/`.

**Fixtures:** project `spec-rest`; `OWNER` = `owner@test.com`, a member; `STRANGER` = `stranger@test.com`, not a member; a seeded volume with `/a.txt` (3 lines), `/report.pdf` (8 bytes), `/src/app.py`.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-301 | Existing | Core Journey | SC-301 | FR-311 | Critical |
| E2E-302 | New | Feature | SC-301 | FR-312 | High |
| E2E-303 | New | Error | SC-301 | FR-304 | High |
| E2E-304 | New | Error | SC-301 | FR-304 | Medium |
| E2E-305 | Existing | Error | SC-301 | FR-311 | High |
| E2E-306 | New | Error | SC-301 | FR-312 | High |
| E2E-307 | New | Side Effect | SC-301 | FR-311 | Critical |
| E2E-308 | New | Edge | SC-301 | FR-312 | Medium |
| E2E-309 | New | Edge | SC-301 | FR-304 | Medium |
| E2E-310 | Existing | Core Journey | SC-302 | FR-313, FR-315 | Critical |
| E2E-311 | Existing | Feature | SC-302 | FR-314 | Critical |
| E2E-312 | Existing | Error | SC-302 | FR-313 | High |
| E2E-313 | New | Error | SC-302 | FR-314 | Medium |
| E2E-314 | New | Security | SC-302 | FR-315 | Critical |
| E2E-315 | New | Edge | SC-302 | FR-313 | High |
| E2E-316 | New | Edge | SC-302 | FR-314 | High |
| E2E-317 | New | Edge | SC-302 | FR-315 | High |
| E2E-318 | New | Core Journey | SC-303 | FR-309 | Critical |
| E2E-319 | Existing | Feature | SC-303 | FR-303, FR-310 | Critical |
| E2E-320 | Existing | Error | SC-303 | FR-303 | High |
| E2E-321 | New | Error | SC-303 | FR-309 | Critical |
| E2E-322 | New | Error | SC-303 | FR-310 | Medium |
| E2E-323 | New | Edge | SC-303 | FR-303 | High |
| E2E-324 | New | Edge | SC-303 | FR-309 | Critical |
| E2E-325 | New | Edge | SC-303 | FR-310 | Medium |
| E2E-326 | Existing | Feature | SC-304 | FR-316, FR-318 | Critical |
| E2E-327 | Existing | Feature | SC-304 | FR-317, FR-319 | High |
| E2E-328 | New | Error | SC-304 | FR-316 | High |
| E2E-329 | New | Error | SC-304 | FR-317, FR-319 | High |
| E2E-330 | New | Edge | SC-304 | FR-316, FR-318 | High |
| E2E-331 | New | Edge | SC-304 | FR-317 | Medium |
| E2E-332 | New | Edge | SC-304 | FR-318, FR-319 | Medium |
| E2E-333 | Existing | Feature | SC-305 | FR-301, FR-302, FR-320 | Critical |
| E2E-334 | New | Error | SC-305 | FR-301 | High |
| E2E-335 | New | Error | SC-305 | FR-302, FR-320 | High |
| E2E-336 | New | Edge | SC-305 | FR-301, FR-320 | High |
| E2E-337 | New | Edge | SC-305 | FR-302 | Medium |
| E2E-338 | Existing | Core Journey | SC-303 | FR-305 | Critical |
| E2E-339 | Existing | Security | SC-303 | FR-305, FR-306 | Critical |
| E2E-340 | Existing | Security | SC-303 | FR-305, FR-308 | Critical |
| E2E-341 | New | Error | SC-303 | FR-306 | High |
| E2E-342 | Existing | Error | SC-303 | FR-307, FR-308 | High |
| E2E-343 | New | Error | SC-303 | FR-307 | High |
| E2E-344 | New | Security | SC-303 | FR-305, FR-306 | Critical |
| E2E-345 | New | Edge | SC-303 | FR-307, FR-308 | High |

**Coverage Statistics** (45 tests):
- Happy path (Core Journey + Feature): 12
- Failure/error (Error + Security): 24
- Side effects: 1
- Edge cases: 15 (of which 7 double as failure-adjacent boundary checks)
- Happy:Failure ratio: 1:2.0

### 12.2 New Test Specifications

#### E2E-302: The upload documentation flag is honoured only on an explicit yes
- **Category:** Feature | **Scenario:** SC-301 | **Requirements:** FR-312
- **Preconditions:** a server with `doc_service` enabled and pointed at the fake converter (`scripts/doc_service_fake.py`).
- **Steps:**
  - Given a multipart upload of `report.pdf` with the field `trigger_documentation_service` set to `true`
  - When the request completes
  - Then the response reports the documentation companion
  - And `/api/fs/spec-rest/list?path=/` shows both `report.pdf` and `report.md`
- **Cleanup:** delete both files
- **Priority:** High

#### E2E-303: A JSON body above the limit is rejected
- **Category:** Error | **Scenario:** SC-301 | **Requirements:** FR-304
- **Preconditions:** authenticated member.
- **Steps:**
  - Given a POST to `/api/fs/spec-rest/write` with a JSON body of 31 MiB
  - When the request is sent
  - Then the response status is 413
  - And no file is created, confirmed by `exists` reporting absence
- **Cleanup:** none
- **Priority:** High

#### E2E-304: An upload above the multipart limit is rejected
- **Category:** Error | **Scenario:** SC-301 | **Requirements:** FR-304
- **Preconditions:** authenticated member.
- **Steps:**
  - Given a multipart upload whose total body exceeds 128 MiB
  - When the request is sent
  - Then the response status is 413
  - And the volume contains no partial file
- **Cleanup:** none
- **Priority:** Medium

#### E2E-306: A non-affirmative documentation flag is treated as false
- **Category:** Error | **Scenario:** SC-301 | **Requirements:** FR-312
- **Preconditions:** `doc_service` enabled.
- **Steps:**
  - Given an upload of `report.pdf` with `trigger_documentation_service` set to `yes`
  - When the request completes
  - Then the upload succeeds
  - And no `report.md` companion exists, because only `true` and `1` are affirmative
  - And the same holds for the values `TRUE`, `y` and the empty string
- **Cleanup:** delete `report.pdf`
- **Priority:** High

#### E2E-307: An upload charges the quota and leaves an audit entry
- **Category:** Side Effect | **Scenario:** SC-301 | **Requirements:** FR-311
- **Preconditions:** a fresh session for `OWNER`.
- **Steps:**
  - Given a multipart upload of one file containing exactly 11 bytes to `directory` `/`
  - When the request completes
  - Then `GET /api/fs/spec-rest/audit-log` contains an entry with `op` `write` and `detail` `11 bytes`
  - And a second upload that would exceed `safety.write_quota_bytes` is refused with status 429 and `ERR_WRITE_QUOTA_EXCEEDED`
- **Cleanup:** delete the uploaded files
- **Priority:** Critical

#### E2E-308: Form part order does not matter
- **Category:** Edge | **Scenario:** SC-301 | **Requirements:** FR-312
- **Preconditions:** authenticated member.
- **Steps:**
  - Given a multipart body where the `directory` field appears **after** the file part
  - When the upload completes
  - Then the file is written under the directory named by that field, not under `/`
  - And the same result is obtained when the field precedes the file part
- **Cleanup:** delete the uploaded files
- **Priority:** Medium

#### E2E-309: A body just under the limit is accepted
- **Category:** Edge | **Scenario:** SC-301 | **Requirements:** FR-304
- **Preconditions:** a raised write quota so the quota is not the limiting factor.
- **Steps:**
  - Given a POST to `/write` with a JSON body of 29 MiB
  - When the request is sent
  - Then the response status is 200
  - And the written file's size matches the content sent
- **Cleanup:** delete the file
- **Priority:** Medium

#### E2E-313: Zipping a missing path fails cleanly
- **Category:** Error | **Scenario:** SC-302 | **Requirements:** FR-314
- **Steps:**
  - Given no path `/nodir`
  - When `GET /api/fs/spec-rest/download-zip?path=/nodir` is issued
  - Then the response status is 404
  - And the body is JSON carrying a `detail` field
- **Cleanup:** none
- **Priority:** Medium

#### E2E-314: A download does not satisfy the read guard
- **Category:** Security | **Scenario:** SC-302 | **Requirements:** FR-315
- **Preconditions:** a fresh session; `/a.txt` exists.
- **Steps:**
  - Given a fresh session
  - When `GET /api/fs/spec-rest/download?path=/a.txt` succeeds
  - Then a subsequent `POST /api/fs/spec-rest/edit` on `/a.txt` fails with status 428 and `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And after `GET /api/fs/spec-rest/read?path=/a.txt` the same edit succeeds
- **Cleanup:** restore `/a.txt`
- **Priority:** Critical

#### E2E-315: A download of a directory is a bytes-plane 404
- **Category:** Edge | **Scenario:** SC-302 | **Requirements:** FR-313
- **Preconditions:** `/src` is a directory.
- **Steps:**
  - Given that directory
  - When `GET /api/fs/spec-rest/download?path=/src` is issued
  - Then the response status is 404
  - And the body equals `{"detail":"not a file: /src"}`, the bytes-plane shape rather than the `{error, detail}` tool-error shape
- **Cleanup:** none
- **Priority:** High

#### E2E-316: Zip entry names are relative to the requested root
- **Category:** Edge | **Scenario:** SC-302 | **Requirements:** FR-314
- **Preconditions:** `/src/app.py` and `/src/nested/deep.py` exist.
- **Steps:**
  - Given `GET /api/fs/spec-rest/download-zip?path=/src`
  - When the archive is listed
  - Then it contains the entries `app.py` and `nested/deep.py`
  - And it contains no entry beginning with `src/` or `/`
- **Cleanup:** none
- **Priority:** High

#### E2E-317: Download reports a content type and a filename
- **Category:** Edge | **Scenario:** SC-302 | **Requirements:** FR-315
- **Preconditions:** `/report.pdf` and `/a.txt` exist.
- **Steps:**
  - Given `GET /api/fs/spec-rest/download?path=/report.pdf`
  - Then the `Content-Disposition` header names the file `report.pdf`
  - And the `Content-Type` is the PDF type rather than `application/octet-stream`
  - And for a file with an unknown extension the `Content-Type` is `application/octet-stream`
- **Cleanup:** none
- **Priority:** High

#### E2E-318: Every parity route calls the engine, proven by session accounting
- **Category:** Core Journey | **Scenario:** SC-303 | **Requirements:** FR-309
- **Preconditions:** a fresh session; a seeded volume.
- **Steps:**
  - Given a fresh session
  - When each mutating parity route is exercised once over REST: `write`, `append`, `create-empty`, `copy`, `edit`, `multi-edit`, `search-replace`, `insert-at-line`, `apply-patch`, `write-bytes`
  - Then `GET /api/fs/spec-rest/audit-log` contains one entry per mutation, proving each went through the engine's shared commit path rather than the volume client
  - And for each of the read routes `read`, `read-lines`, `head`, `tail`, `stat`, `exists`, `hash`, `count-lines`, `glob`, `grep`, `tree`, the JSON body carries exactly the keys the corresponding MCP tool returns
- **Cleanup:** reset the volume
- **Priority:** Critical

#### E2E-321: An engine refusal surfaces with the same code on both transports
- **Category:** Error | **Scenario:** SC-303 | **Requirements:** FR-309
- **Preconditions:** `/a.txt` exists and contains three occurrences of `x`; read in this session.
- **Steps:**
  - Given an ambiguous edit
  - When `POST /api/fs/spec-rest/edit` is issued with `old_string` `x` and `replace_all` false
  - Then the status is 409 and the body `error` equals `ERR_AMBIGUOUS_MATCH`
  - And the same operation over the MCP surface returns the identical code and message text
- **Cleanup:** none
- **Priority:** Critical

#### E2E-322: A mutating route refuses GET
- **Category:** Error | **Scenario:** SC-303 | **Requirements:** FR-310
- **Steps:**
  - Given `GET /api/fs/spec-rest/write`
  - When the request is sent
  - Then the response status is 405
  - And `POST` to the same path with a valid body succeeds
- **Cleanup:** delete the written file
- **Priority:** Medium

#### E2E-323: Parameter names match the tool names exactly
- **Category:** Edge | **Scenario:** SC-303 | **Requirements:** FR-303
- **Preconditions:** `/a.txt` exists.
- **Steps:**
  - Given `GET /api/fs/spec-rest/read-bytes?path=/a.txt&offset_bytes=0&length_bytes=4`
  - Then the response carries `length` 4
  - And the same call using `offset` and `length` instead is treated as if the parameters were absent, falling back to the documented defaults
- **Cleanup:** none
- **Priority:** High

#### E2E-324: read-many is POST and isolates per-file errors identically to the tool
- **Category:** Edge | **Scenario:** SC-303 | **Requirements:** FR-309
- **Preconditions:** `/a.txt` exists; `/nope.txt` does not.
- **Steps:**
  - Given `POST /api/fs/spec-rest/read-many` with body `{"paths":["/a.txt","/nope.txt"]}`
  - Then the status is 200
  - And `files[1].error` equals `not found: /nope.txt`, matching S2 FR-206 exactly
- **Cleanup:** none
- **Priority:** Critical

#### E2E-325: Read routes are GET and mutating routes are POST throughout
- **Category:** Edge | **Scenario:** SC-303 | **Requirements:** FR-310
- **Steps:**
  - Given the 38-route manifest
  - When each route is probed with the wrong method
  - Then every one answers 405
  - And `read-many` is confirmed to be POST despite being read-only
- **Cleanup:** none
- **Priority:** Medium

#### E2E-328: A route with no matching tool still carries a summary
- **Category:** Error | **Scenario:** SC-304 | **Requirements:** FR-316
- **Steps:**
  - Given `GET /api/swagger.json`
  - When the operations for `upload`, `download`, `download-zip` and `roots` are inspected
  - Then each has a non-empty summary
  - And none of those summaries is the empty string that a missing tool lookup would produce
- **Cleanup:** none
- **Priority:** High

#### E2E-329: The documentation endpoints need no token
- **Category:** Error | **Scenario:** SC-304 | **Requirements:** FR-317, FR-319
- **Steps:**
  - Given no `Authorization` and no `X-Forwarded-Authorization` header
  - When `GET /api/swagger.json` and `GET /api/docs` are issued
  - Then both return status 200
  - And `GET /api/fs/roots` with the same absent headers returns 401, showing the data plane is still guarded
- **Cleanup:** none
- **Priority:** High

#### E2E-330: Descriptions are inherited from the tool schemas
- **Category:** Edge | **Scenario:** SC-304 | **Requirements:** FR-316, FR-318
- **Steps:**
  - Given `GET /api/swagger.json`
  - When the `path` parameter of the `read` operation is compared against the `path` parameter description of the `fs.read` tool from `tools/list`
  - Then the two strings are identical
  - And the `list` operation's summary equals the description of `fs.list_dir`, confirming the name override
  - And the `roots` operation's summary equals the description of `fs.list_allowed_roots`
- **Cleanup:** none
- **Priority:** High

#### E2E-331: Bytes-plane parameters carry their own descriptions
- **Category:** Edge | **Scenario:** SC-304 | **Requirements:** FR-317
- **Steps:**
  - Given `GET /api/swagger.json`
  - When the parameters of `download-zip` are inspected
  - Then the `path` parameter has a non-empty description sourced from the REST-only parameter table
- **Cleanup:** none
- **Priority:** Medium

#### E2E-332: The documented parameter list is the REST list, not the tool list
- **Category:** Edge | **Scenario:** SC-304 | **Requirements:** FR-318, FR-319
- **Steps:**
  - Given `GET /api/swagger.json`
  - When the `mkdir` operation is inspected
  - Then its documented parameters are exactly `mount_id` and `path`
  - And they do not include `parents` or `exist_ok`, which the `fs.mkdir` tool accepts but the route does not
- **Cleanup:** none
- **Priority:** Medium

#### E2E-334: The router and the manifest agree
- **Category:** Error | **Scenario:** SC-305 | **Requirements:** FR-301
- **Steps:**
  - Given the manifest of 38 `(method, segment)` pairs
  - When each pair is requested with a valid token against a real server
  - Then none answers 404 from the router
  - And the manifest length is exactly 38
- **Cleanup:** none
- **Priority:** High

#### E2E-335: An unknown operation segment is a router 404
- **Category:** Error | **Scenario:** SC-305 | **Requirements:** FR-302, FR-320
- **Steps:**
  - Given `GET /api/fs/spec-rest/not-a-real-operation` with a valid token
  - When the request is sent
  - Then the response status is 404
  - And the body is not the `{error, detail}` tool-error shape, because no handler ran
- **Cleanup:** none
- **Priority:** High

#### E2E-336: Every manifest route appears in the OpenAPI document
- **Category:** Edge | **Scenario:** SC-305 | **Requirements:** FR-301, FR-320
- **Steps:**
  - Given the generated document from `GET /api/swagger.json`
  - When its path and method pairs are collected
  - Then the set equals the 38-entry manifest exactly, with no extra and no missing entry
- **Cleanup:** none
- **Priority:** High

#### E2E-337: Only roots is unscoped by mount
- **Category:** Edge | **Scenario:** SC-305 | **Requirements:** FR-302
- **Steps:**
  - Given the manifest
  - When each route's path template is inspected
  - Then `roots` is served at `/api/fs/roots` with no mount segment
  - And the other 37 are served at `/api/fs/{mount_id}/{operation}`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-341: The REST 401 body repeats the code in detail
- **Category:** Error | **Scenario:** SC-303 | **Requirements:** FR-306
- **Steps:**
  - Given no bearer token
  - When `GET /api/fs/roots` is issued
  - Then the status is 401
  - And the body `error` equals `ERR_UNAUTHENTICATED`
  - And the body `detail` begins with `ERR_UNAUTHENTICATED: `, unlike the MCP 401 whose detail is the bare message
- **Cleanup:** none
- **Priority:** High

#### E2E-343: Every mapped status is reachable through this plane
- **Category:** Error | **Scenario:** SC-303 | **Requirements:** FR-307
- **Preconditions:** a seeded volume and a member session.
- **Steps:**
  - Given operations chosen to raise each error
  - When a no-clobber write, an unread overwrite, an ambiguous edit, a non-matching edit, a missing path read and a hard delete with hard delete disabled are each issued
  - Then the statuses are 409, 428, 409, 422, 404 and 501 respectively
  - And each body's `error` field carries the matching `ERR_*` code
- **Cleanup:** reset the volume
- **Priority:** High

#### E2E-344: An unauthenticated caller learns nothing about project existence
- **Category:** Security | **Scenario:** SC-303 | **Requirements:** FR-305, FR-306
- **Steps:**
  - Given no bearer token
  - When `GET /api/fs/spec-rest/list?path=/` and `GET /api/fs/does-not-exist/list?path=/` are both issued
  - Then both answer 401 with identical bodies
  - And neither answers 404, proving the identity check precedes the membership check
- **Cleanup:** none
- **Priority:** Critical

#### E2E-345: A member and a stranger are distinguished from a missing project
- **Category:** Edge | **Scenario:** SC-303 | **Requirements:** FR-307, FR-308
- **Preconditions:** `spec-rest` exists with `OWNER` as member; `STRANGER` is authenticated but not a member.
- **Steps:**
  - Given `STRANGER`'s token
  - When `GET /api/fs/spec-rest/list?path=/` is issued
  - Then the status is 403 with `error` `ERR_FORBIDDEN`
  - And with the same token `GET /api/fs/no-such-project/list?path=/` returns 404 with `error` `ERR_PROJECT_NOT_FOUND`
  - And a platform admin who is not a member also receives 403, not 200
- **Cleanup:** none
- **Priority:** High

### 12.3 Modified Test Specifications

None.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

1. **Two different 401 bodies, both correct.** S1 FR-011 specifies the MCP 401 as `{"error":"ERR_UNAUTHENTICATED","detail":"<message>"}`; S3 FR-306 specifies the REST 401 as `{"error":"<CODE>","detail":"<CODE>: <message>"}`. The difference is real, deliberate and verified against the reference server (`app.rs:271-280` against `api/dataplane.rs:219-223`). E2E-341 pins it.
2. **The bytes plane uses a different 404 shape** from the tool-error shape: `{"detail":"..."}` rather than `{"error":..., "detail":...}` (`dataplane.rs:233-236`). E2E-315 pins it.
3. **Tool count in the OpenAPI document.** The document covers 38 REST routes, not the 45 always-on tools; the two surfaces are not in one-to-one correspondence, as FR-318 states.
4. **MIME duplication**, carried forward from S2 TBD-202 and confirmed to have two live call sites: `docs::guess_mime` in `download`, `core::fs_ops::mime_guess` in `fs.read_bytes`. The audit verified the two tables are identical in both keys and values, so the surfaces agree. Recorded as TBD-301.

## 14. Migration & Implementation Notes

No production code change. Test work, in order:

1. **Annotate existing tests** in `api/dataplane.rs` and `api/openapi.rs` with their `E2E-3xx` ids.
2. **Add the document-level tests first** (E2E-328, E2E-330, E2E-331, E2E-332, E2E-336, E2E-337): they need only the generated JSON and the registry, so they run in-process with no server.
3. **Add the guard tests** (E2E-341, E2E-343, E2E-344, E2E-345) using the existing `OWNER` / `STRANGER` fixtures.
4. **E2E-318 is the expensive one** and belongs in its own functional scenario script: it exercises every mutating parity route and then reads the audit log. Write it after the cheaper tests are green, because it is the one most likely to need fixture tuning.
5. **E2E-303, E2E-304 and E2E-309 need large bodies.** Generate them in the test rather than committing fixture files, and run them last so a failure does not slow the rest of the suite.
6. **E2E-302 and E2E-306 need the fake document service** (`scripts/doc_service_fake.py`); they are the only tests here that depend on S5's subject and can be deferred until S5 lands.

## 15. Open Questions & TBDs

- **TBD-301:** Two MIME implementations serve two routes for the same file. `download` uses `docs::guess_mime`, `fs.read_bytes` uses `core::fs_ops::mime_guess` (`core/fs_ops.rs:1286`). The audit verified they agree on every extension, 32 entries each with identical values, so there is no live inconsistency. What remains open is whether to collapse them; until that happens E2E-548 in S5 is what detects divergence.
- **TBD-302:** `download` and `download-zip` record no session read (FR-315), while `read-bytes` does (S2 FR-207). An integrator who downloads a file and then tries to edit it is refused. This is as-built and defensible, but it is a sharp edge worth a deliberate decision.
- **TBD-303:** The OpenAPI document is regenerated per request. At 86 tools the cost is small, but it is unmeasured.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Data plane** | The `/api/fs` REST surface, as distinct from the MCP endpoint. | Transport |
| **Bytes plane** | The eight routes that move raw bytes and have no MCP tool. | Bytes plane |
| **Tool parity** | The property that a REST route and its MCP tool call the same engine function and return the same keys. | Transport |
| **Guard** | The shared bearer-then-membership-then-operation wrapper every route passes through. | Transport |
| **Route manifest** | `REST_ROUTES`, the authoritative list of `(method, segment)` pairs. | Transport |
| **Attachment** | A download response carrying `Content-Type` and `Content-Disposition`. | Bytes plane |
| **Upload form** | The multipart body carrying files plus the `directory`, `paths` and documentation fields. | Bytes plane |
| **REST_ONLY** | The table supplying summaries for routes that have no tool to inherit from. | Documentation |
| **Name override** | The two exceptions to name derivation: `list` to `fs.list_dir`, `roots` to `fs.list_allowed_roots`. | Documentation |
| **Swagger UI** | The documentation page at `/api/docs`, served from assets embedded in the binary. | Documentation |
| **Body limit** | The per-transport ceiling on request size: 30 MiB JSON, 128 MiB upload. | Transport |

## 17. Interview Decisions Log

Produced non-interactively from the code.

- **DEC-301:** S3 specifies the plumbing of the four document routes but not their semantics. **Rationale:** `extract-text`, `write-docx`, `documentize` and the upload documentation flag all depend on S5's engines; specifying their behaviour here would duplicate S5 and risk the two disagreeing. **Alternatives considered:** deferring the routes entirely, which would leave four of the 38 undocumented. **Implemented by:** FR-301 (they are in the manifest), FR-312 (the flag's parsing rule only). **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/api/dataplane.rs:144-147`.
- **DEC-302:** The two different 401 body shapes are specified as-built rather than unified. **Rationale:** both are frozen wire contracts verified against the reference server; unifying them would be a breaking change to at least one client, which is a product decision and not a specification one. **Alternatives considered:** specifying one shape and registering the other as drift. **Implemented by:** FR-306, E2E-341, §13 item 1. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/app.rs:271-280` against `crates/mcp-fs/src/api/dataplane.rs:219-223`.
- **DEC-303:** Parity is specified as a testable property (FR-309) with one expensive test that walks every mutating route. **Rationale:** parity is the whole justification for the layer's design, and asserting it in prose without a test is how it would quietly stop being true. **Alternatives considered:** a per-route parity test pair, which would roughly double this spec's test count for little extra signal. **Implemented by:** FR-309, E2E-318, E2E-321, E2E-324. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/api/dataplane.rs:7-10`.
- **DEC-304:** The bytes plane's deliberate asymmetry (writes go through the engine, reads do not) is specified rather than smoothed over. **Rationale:** the asymmetry has a reason on each side: writes need accounting, downloads are not agent reads. Hiding it would make the read-guard behaviour in E2E-314 look like a bug. **Alternatives considered:** routing downloads through the engine so they record reads. **Implemented by:** FR-311, FR-315, TBD-302. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/api/dataplane.rs:3-10` and `crates/mcp-fs/src/core/fs_ops.rs:573-578`.
- **DEC-305:** The documentation-inheritance mechanism is specified as a requirement rather than described as an implementation detail. **Rationale:** it produces an observable property, that a poorly described tool yields a poorly described API, which the team relies on as an audit. **Alternatives considered:** treating the Swagger document as generated output outside the spec. **Implemented by:** FR-316, FR-317, FR-318, E2E-330. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/api/openapi.rs:5-20`.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 0 | 1 (A-6 MIME duplication resolved with evidence) | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** TBD-301 rewritten from an open question into a stated fact
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
