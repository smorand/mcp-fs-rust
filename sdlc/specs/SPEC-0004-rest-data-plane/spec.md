> Id: SPEC-0004
> Nature: FEAT
> Status: as-built
> Area: rest-data-plane
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# REST Data Plane and OpenAPI Documentation

## 1. Summary

The server exposes a second transport alongside MCP: a REST data plane of 40 HTTP routes under a project-scoped path, plus a self-describing OpenAPI document and an embedded Swagger UI. Thirty-six of the 40 routes call the exact same engine functions their MCP tool counterparts call, so an operation performed over HTTP and the same operation performed over MCP are observably identical: same result shape, same quota accounting, same audit trail, same search-index updates. Four routes (the project list, a browser-oriented directory listing, a single-file download, and a subtree-as-zip download) read through the storage layer directly rather than through the engine, by design, because they serve a browser rather than an agent and must not consume the engine's read-before-edit guard.

The OpenAPI document that describes this surface writes no description of its own: every route's summary and parameter description is copied, at the moment the document is requested, from the matching MCP tool's schema. Three routes with no tool equivalent carry an explicit, hand-written summary instead. A build-time test keeps the route table, the documented table and the router itself from disagreeing, so no route can ship without being reflected in the documentation.

## 2. Current State

### 2.1 Existing capability

None (greenfield relative to this area; this document retro-specifies shipped behaviour).

### 2.2 Existing specification debt

None.

### 2.3 Relevant prior specifications

This area builds on the platform foundation (identity resolution, the stable error vocabulary and its HTTP status mapping, project-membership authorization, path normalization and the write/read safety contract) and on the filesystem engine (every operation this plane exposes and its semantics). Those behaviours are referenced here by name, not restated; only the requirements that are specific to serving them over HTTP are claimed by this document.

## 3. Scope

### 3.1 In scope

- The 40 routes of the REST data plane: their methods, path shapes, and parameter names.
- The four routes that read through the storage layer directly: the project list, the browser directory listing, the single-file download and the subtree-zip download.
- The 36 routes that call the same engine function as their MCP tool counterpart, including the file upload route and the two routes shared with the export/extract-archive tool family.
- The shared request guard: identity verification, then project-membership authorization, then the operation.
- Mapping of engine errors to HTTP status codes and the two distinct JSON error body shapes used on this plane.
- Request body size ceilings for JSON bodies and for the multipart upload route.
- The generated OpenAPI document, its description-inheritance mechanism, the explicit-summary table for routes without a tool equivalent, and the embedded documentation UI.
- The build-time invariant that keeps the router, the route manifest and the documented route table from disagreeing.

### 3.2 Out of scope (non-goals)

- The filesystem semantics themselves (what an operation does): specified where the engine is specified.
- Identity verification and the stable error vocabulary: specified where the platform foundation is specified.
- The semantics of document-conversion operations (text extraction, document generation, the upload-time documentation flag, and the standalone documentation trigger): this document specifies only their routing, guard and parameter plumbing.
- Search-index maintenance triggered by writes through this plane: specified where search indexing is specified.
- Any authentication scheme other than the bearer-token scheme the platform foundation specifies.

## 4. Actors

| Actor | Description |
|---|---|
| **Web client** | A browser or application uploading and downloading files. The only consumer that needs the storage-layer-direct routes: multipart upload and zip/file download exist for it. |
| **REST integrator** | A program automating filesystem operations over HTTP instead of MCP, using the same parameter names the tools use. |
| **API documentation reader** | Anyone loading the documentation UI, including before they hold a token. |
| **Project member** | The authenticated identity behind every call, subject to the same membership and session rules as on the MCP surface. |

## 5. Usage Scenarios

### SC-001: Web client uploads files into a project
**Actor:** Web client
**Preconditions:** the caller holds a valid bearer; the caller is a member of the project.
**Flow:**
1. The client sends a multipart form to the upload route of a project.
2. The guard verifies the bearer, then membership, then opens the project volume.
3. The whole form is read before files are paired with their relative paths, so part order does not matter.
4. Form fields are interpreted: a destination directory (default the volume root), a repeated per-file relative path field, and an opt-in documentation-companion flag.
5. Every file is written through the engine's write operation, so the write quota is charged, an audit entry is recorded, and the search index is updated exactly as an MCP write would update it.

**Postconditions:** the files exist in the volume; the quota has advanced; the write is auditable.
**Exceptions:**
- an absent or invalid bearer yields an unauthenticated response
- a caller who is not a member yields a forbidden response
- an unknown project yields a not-found response
- a malformed multipart body yields an invalid-argument response
- a body above the upload size ceiling is rejected before any handler runs
- an exhausted write quota yields a quota-exceeded response

### SC-002: Web client downloads a file and a subtree
**Actor:** Web client
**Preconditions:** authenticated member; the requested path exists.
**Flow:**
1. The client requests the single-file download route with a path.
2. The handler verifies the path names a file, reads the bytes directly from storage, guesses a content type from the extension, and returns the bytes as an attachment with a filename.
3. The client requests the subtree-zip route for a directory; entry names inside the returned archive are relative to the requested root, so extracting it reproduces the subtree rather than its absolute path.

**Postconditions:** the client holds the bytes with a filename and a content type. Neither download records a read against the caller's session, because both talk to storage directly rather than to the engine.
**Exceptions:**
- a path that names a directory, requested from the single-file route, yields a not-a-file response distinct in shape from an engine error
- a path that does not exist yields a not-found response
- an unrecognized extension yields a generic binary content type

### SC-003: REST integrator performs a tool-equivalent operation
**Actor:** REST integrator
**Preconditions:** authenticated member.
**Flow:**
1. The integrator requests a read-style route with the same parameter names its MCP tool counterpart accepts.
2. The guard runs, the path is normalized, and the identical engine function the tool calls is invoked.
3. The result is returned as JSON with the same keys the tool returns.
4. The integrator later submits a mutating operation (for example an edit) using a JSON body with the same parameter names as the tool.

**Postconditions:** the observable effect is indistinguishable from the same operation performed over MCP: same result keys, same session accounting, same audit entry, same index update.
**Exceptions:**
- a missing required parameter yields an invalid-argument response
- any engine refusal surfaces with the same stable error code as it would over MCP, at the HTTP status the platform's error mapping assigns it
- a JSON body above the size ceiling is rejected before any handler runs
- editing a path with no prior read in the current session yields the read-before-edit refusal

### SC-004: Integrator discovers the API through the documentation UI
**Actor:** API documentation reader
**Preconditions:** the server is running. **No token is required.**
**Flow:**
1. The reader loads the documentation page, served from assets embedded in the server binary, so it renders with no further network access.
2. The page fetches the generated OpenAPI document.
3. The document is built at request time by walking every documented route and, for each, deriving the name of its MCP tool counterpart and copying that tool's summary and parameter descriptions.
4. The three routes with no tool counterpart (upload, single-file download, subtree-zip download) take their summary and parameter descriptions from an explicit table maintained for that purpose.

**Postconditions:** the reader holds a description of all 40 routes, with descriptions identical to the tool documentation an MCP client would see.
**Exceptions:**
- a tool whose schema carries a blank description yields a blank description on the page, deliberately, since the page doubles as an audit of documentation quality
- a route with neither a matching tool nor an explicit-summary entry is prevented from shipping by a build-time check

### SC-005: Maintainer changes a route or a tool without the two surfaces drifting
**Actor:** Maintainer, at build time
**Preconditions:** a change to a route or to a tool's schema.
**Flow:**
1. A new route is added to the router and to the route manifest that documents the complete route set.
2. A build-time check compares the documented route table against that manifest.
3. For every documented route that has a tool counterpart, its parameter names are required to equal the tool's parameter names.

**Postconditions:** an undocumented route cannot ship; a renamed tool parameter is caught as a documentation mismatch rather than drifting silently.
**Exceptions:**
- a route added to the router but omitted from the manifest fails the build-time check
- a REST parameter renamed away from its tool counterpart's name loses its inherited description, which is itself detectable on the documentation page

## 6. Functional Requirements

### Routing and transport

#### FR-001: The route manifest is authoritative and exhaustive
> The server SHALL serve exactly 40 routes under the data plane path prefix, and every one of those routes SHALL appear in the authoritative route manifest.

- **Inputs:** an HTTP request under the data plane prefix.
- **Outputs:** the routed handler, or a not-found response from the router itself when no route matches.
- **Business Rules:** the manifest lists every route as a (method, operation-name) pair; a build-time test asserts the documented route table matches this manifest exactly, so a route cannot ship undocumented. Of the 40 routes, 4 read directly from storage and 36 call an engine function shared with an MCP tool.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:45-91` (manifest), `crates/core/src/api/dataplane.rs:108-152` (router), `crates/core/src/api/dataplane.rs:2424` (count assertion).

#### FR-002: Path shape and project scoping
> Every route except the project-list route SHALL carry the target project as a path segment, in the form `/{prefix}/{project}/{operation}`.

- **Inputs:** the request path.
- **Outputs:** the extracted project identifier.
- **Business Rules:** the project-list route is the single unscoped route in the set, because it answers which projects the caller can reach at all, before any project identifier is known.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:108` (unscoped route) against `crates/core/src/api/dataplane.rs:109-152` (scoped routes).

#### FR-003: REST parameter names equal tool parameter names
> Every query and body parameter on a route with a tool counterpart SHALL be spelled exactly as that tool's corresponding parameter, in snake_case.

- **Inputs:** any request parameter.
- **Outputs:** the parsed argument.
- **Business Rules:** this is load-bearing, not stylistic: the documentation document inherits descriptions by matching these names against the tool schema. Route-segment words are hyphen-separated while parameter names are underscore-separated.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:17-20`.

#### FR-004: Request body size ceilings
> WHEN a request body exceeds its ceiling THE server SHALL reject it, applying one ceiling to JSON bodies on every route and a separate, larger ceiling to the multipart upload route alone.

- **Inputs:** the request body.
- **Outputs:** the rejection from the body-size limiting layer, before any handler logic runs.
- **Business Rules:** the JSON ceiling is 30 MiB and applies to the whole router; the upload ceiling is 128 MiB and applies only to the upload route as a per-route override.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:95,99,113-116,154` (`JSON_BODY_LIMIT`, `UPLOAD_BODY_LIMIT`).

### The guard

#### FR-005: Guard order
> WHEN any data-plane request is handled THE server SHALL verify the bearer first, then project membership, then open the project's storage handle, and only then run the operation.

- **Inputs:** the request headers and the target project.
- **Outputs:** the handler's result, or the first failure encountered, in that order.
- **Business Rules:** the order matters for what a caller can learn: an unauthenticated caller never learns whether a project exists, because membership is checked only after identity succeeds.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:186-206` (`guarded`).

#### FR-006: Unauthenticated response shape
> IF bearer verification fails THEN the server SHALL answer with an unauthenticated status and a body whose detail field repeats the stable error code followed by the message.

- **Inputs:** the identity-resolution failure.
- **Outputs:** the error body.
- **Business Rules:** this differs deliberately from the equivalent MCP-surface response, whose detail field carries the bare message without repeating the code; both shapes are frozen as independently correct.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:222-226`.

#### FR-007: Error status mapping
> WHEN an operation returns an engine error THE server SHALL answer at the HTTP status the platform's error-to-status mapping assigns that error's stable code, with a body carrying that same stable code.

- **Inputs:** the engine error.
- **Outputs:** an error body at the mapped status; an unmappable status falls back to an internal-error status.
- **Business Rules:** the body's detail field repeats the stable code followed by the message, matching the unauthenticated-response convention. The body keeps the same stable code vocabulary a caller on the MCP surface would see for the identical failure.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:228-235`.

#### FR-008: The membership gate is shared with the MCP surface
> The data plane SHALL authorize every scoped route by the same project-membership check the MCP tools use, and SHALL NOT grant access on the basis of a platform-administrator role alone.

- **Inputs:** the project identifier, the caller's identity.
- **Outputs:** authorization, or a forbidden/not-found refusal.
- **Business Rules:** there is exactly one implementation of this gate, shared by both transports, so it cannot be gotten wrong twice.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:197`.

### Engine-parity plane

#### FR-009: Engine-parity routes call the shared engine function
> Each of the routes that has an MCP tool counterpart SHALL call the same underlying function that tool calls, and SHALL return the same result keys that tool returns.

- **Inputs:** the route's parameters.
- **Outputs:** the engine's result value, returned as the response body.
- **Business Rules:** this plane performs no filesystem logic of its own beyond normalizing paths and forwarding; session accounting, the read-before-edit guard, the write quota, the audit log and search-index maintenance all behave identically whichever transport is used. This includes directory creation, deletion, and move/rename, each of which now routes through the same function its tool calls, carrying the same search-index update hooks the MCP-surface operation triggers.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:365-461` (create-directory, delete, move all calling the shared engine functions and the shared search-index hooks); `crates/core/src/api/openapi.rs:547-565` (each mapped to its tool name).

#### FR-010: Method assignment
> The server SHALL serve read-only operations over GET and mutating operations over POST, except where a read-only operation's parameter shape (an array that does not fit a query string) requires POST.

- **Inputs:** the operation.
- **Outputs:** the route's HTTP method.
- **Business Rules:** the multi-file read operation is the one read-only route served over POST, because its input is an array of paths.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:109-152` (method per route); `crates/core/src/api/dataplane.rs:137` (the multi-file read route under POST).

### Storage-direct plane

#### FR-011: Multipart upload writes through the engine
> WHEN a multipart form is uploaded THE server SHALL read the whole form before pairing files with their relative paths, and SHALL write each file through the same engine write function the write tool calls.

- **Inputs:** file parts; a destination-directory field (default the volume root); a repeated per-file relative-path field; a documentation-companion flag field.
- **Outputs:** a JSON upload report.
- **Business Rules:** reading the whole form before acting makes part order irrelevant; routing the write through the engine rather than directly to storage is what gives upload its quota charge and audit entry.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:473-and-following` (upload handler); engine write function shared with the write tool.

#### FR-012: The upload documentation flag is opt-in and strict
> IF the documentation-companion form field holds exactly `true` or `1` after trimming THEN the server SHALL request a Markdown companion for every eligible file in the form; otherwise it SHALL NOT, and the entire upload SHALL be refused before any byte is written if any file in a triggered upload is ineligible.

- **Inputs:** the form field's text value.
- **Outputs:** the boolean passed to the engine, applied uniformly to the whole form.
- **Business Rules:** anything other than an explicit affirmative is a negative, because a free-text form field with a typo must not silently invoke a potentially slow conversion. The conversion behaviour itself belongs to the document-conversion area.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/openapi.rs:503-510` (the flag's documented parsing rule).

#### FR-013: File download reads directly from storage
> WHEN the single-file download route is called for a path that names a file THE server SHALL return the bytes as an attachment, with a guessed content type and the basename as filename, read directly from storage rather than through the engine.

- **Inputs:** the path.
- **Outputs:** the attachment response.
- **Business Rules:** a path that does not name a file yields a not-found response whose body shape is `{"detail": "..."}`, distinct from the engine-error shape the rest of the plane uses, because this route short-circuits before any engine call. Content-type guessing on this route uses a separate implementation from the one the byte-range read tool uses; the two tables are functionally equivalent today but are two call sites.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:579-596`; `crates/core/src/docs/mod.rs:23` (the download route's mime lookup) against `crates/core/src/core/fs_ops.rs:1391` (the byte-range read tool's separate mime lookup).

#### FR-014: Subtree download as zip reads directly from storage
> WHEN the subtree-zip download route is called THE server SHALL return a zip archive, built by walking storage directly, whose entry names are relative to the requested root.

- **Inputs:** the root path (default the volume root).
- **Outputs:** the archive as an attachment, named after the root's basename or, for the volume root, after the project itself.
- **Business Rules:** entry names are relative to the requested root, so extracting the archive reproduces the subtree rather than the absolute volume path.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:600-628`.

#### FR-015: The project-list and browser-listing routes read directly from storage
> The project-list route and the browser-oriented directory-listing route SHALL read through storage directly rather than through the engine, and SHALL therefore record no session read.

- **Inputs:** the caller's identity (for the project list); the target path (for the listing).
- **Outputs:** the project list, or a directory listing sorted directories-first then caselessly by name.
- **Business Rules:** the directory-listing route is deliberately distinct from its nearest tool counterpart's shape: it always includes size and modification time and offers no hidden-file filter or sort choice, because it serves a browser rather than an agent. Listing a path that names a file yields an error rather than an empty listing that would invent a directory.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:324-363`.

#### FR-016: Download routes do not satisfy the read-before-edit guard
> Downloading a file or a subtree through the storage-direct routes SHALL NOT satisfy the engine's read-before-edit requirement for a subsequent edit of the same path.

- **Inputs:** a prior download of a path, a subsequent edit request for that path in the same session.
- **Outputs:** the edit is refused exactly as if no read had occurred.
- **Business Rules:** this is a deliberate asymmetry with the write side: uploads go through the engine so writes are accounted for, but downloads exist to serve bytes to a browser, not to satisfy an agent's read-then-edit workflow.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:579-628` (download handlers use the storage client, not the engine's read path).

### Documentation

#### FR-017: The documentation document is generated from the tool registry at request time
> WHEN the documentation document is requested THE server SHALL build it at that moment by matching each documented route to its MCP tool counterpart and copying that tool's summary and parameter descriptions.

- **Inputs:** the live tool registry; the documented route table.
- **Outputs:** the generated document.
- **Business Rules:** two routes carry a name override because their nearest tool has a different name than their route segment would otherwise imply (the project-list route and the browser-listing route). Not one description is authored directly on a documented route; documenting a tool documents the matching route.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/openapi.rs:1-24` (module intent), `crates/core/src/api/openapi.rs:527,535` (the two name overrides).

#### FR-018: Routes without a tool counterpart carry an explicit summary
> IF a documented route has no corresponding tool THEN its summary and its parameter descriptions SHALL come from an explicit table maintained for that purpose.

- **Inputs:** the route's operation name.
- **Outputs:** the summary and the per-parameter description text.
- **Business Rules:** exactly three routes fall into this case: upload, single-file download, and subtree-zip download. The project-create, delete and move routes, despite also being storage- or engine-adjacent, each have a tool counterpart and inherit from it rather than from this table.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/openapi.rs:484-512` (the explicit table, three entries).

#### FR-019: The documented parameter list is the route's own list, not the tool's
> The documentation document's parameter list for a route SHALL come from that route's own documented parameter table rather than from its tool counterpart's full input schema.

- **Inputs:** the route's documented parameter table.
- **Outputs:** the parameters shown on the documentation page.
- **Business Rules:** the two surfaces genuinely differ in shape for some operations: a route may accept fewer parameters than its tool counterpart accepts (for example, a route exposing only the target path where its tool counterpart also accepts optional behavioural flags). Descriptions are inherited by matching on name; the set of parameters shown is not inherited.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/openapi.rs:21-24`.

#### FR-020: The documentation endpoints require no authentication
> The server SHALL serve the generated documentation document and the documentation UI without requiring a bearer token.

- **Inputs:** an unauthenticated request.
- **Outputs:** the document, or the UI page and its embedded assets.
- **Business Rules:** a caller who cannot yet authenticate still needs to learn how to authenticate; the UI's assets are served from a copy embedded in the server binary, so the page renders with no further network access.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/openapi.rs:50-54` (routes mounted without a guard).

#### FR-021: No route ships undocumented
> The server SHALL NOT expose a data-plane route that is absent from the documented route table.

- **Inputs:** the router, the route manifest, the documented route table.
- **Outputs:** a failing build-time test when the three disagree.
- **Business Rules:** this invariant is enforced mechanically rather than by review, which is what makes it hold as the route set grows.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/api/dataplane.rs:2424-2430` (the route-count and route-membership assertions).

## 7. Non-Functional Requirements

### 7.1 Performance
- Downloads read through the storage layer directly; a byte-range read is available on the engine's parity surface to avoid materializing a whole file when only part of it is needed.
- Body ceilings bound worst-case per-request memory: a JSON ceiling and a separate, larger upload ceiling (FR-004).
- The documentation document is generated per request rather than cached; its cost is bounded by the size of the tool registry.

### 7.2 Security
- The guard order gives an unauthenticated caller no information about whether a project exists (FR-005).
- A platform-administrator role confers no data access on this plane either (FR-008).
- Every path parameter is normalized before use, inheriting the platform's traversal protection.
- The documentation endpoints are deliberately public (FR-020); they expose the shape of the API, never data.
- Error bodies carry a stable code and a human-readable message, never an internal stack trace.

### 7.3 Usability
- Parameter names match tool names exactly, so knowledge transfers between the two surfaces in both directions (FR-003).
- The documentation page doubles as an audit of tool-documentation quality: a blank description is visible rather than silently hidden (SC-004 exception).
- Downloads carry a filename and a content type, so a browser saves them sensibly.

### 7.4 Reliability
- The plane holds no state of its own beyond what the underlying engine and session layer own; reliability is inherited from them.
- The route/table coverage check prevents the most likely regression class: an endpoint shipped without documentation (FR-021).

### 7.5 Observability
Writes through this plane produce the same audit entries as the equivalent MCP writes, which is the structural point of routing mutating routes through the shared engine.

### 7.6 Deployment
Served by the same process and listener as the MCP endpoint; no separate port or process.

### 7.7 Scalability
Stateless per request apart from the session state the platform foundation owns; the same per-process caveat applies to the read-before-edit guard and the write quota on this plane as on the MCP surface.

## 8. End-to-End Tests

Fixtures: a seeded project with a member and a non-member identity, and a seeded volume containing a small text file, a small binary file, and a nested directory of source files.

- **E2E-001**: A multipart upload with a documentation-companion flag set to an exact affirmative value produces both the uploaded file and its Markdown companion; the same upload with any non-exact value (`yes`, `TRUE`, `y`, empty) produces only the uploaded file. — FR-011, FR-012
- **E2E-002**: A JSON body above the JSON ceiling is rejected before any handler runs, and no partial write occurs. — FR-004
- **E2E-003**: A multipart body above the upload ceiling is rejected, and the volume contains no partial file. — FR-004
- **E2E-004**: An upload with the directory field positioned after the file part in the multipart body writes to the same place as the same upload with the field positioned before it. — FR-011
- **E2E-005**: A single-file-record upload advances the write quota and leaves one matching audit entry; a second upload that would exceed the quota is refused with the quota-exceeded response. — FR-011
- **E2E-006**: Requesting the single-file download route for a path that names a directory yields a not-found response shaped `{"detail": "..."}`, distinct from the engine-error shape. — FR-013
- **E2E-007**: Requesting the subtree-zip route for a nested directory yields an archive whose entries are relative to the requested root and contain no entry beginning with the root's own name or a leading separator. — FR-014
- **E2E-008**: Downloading a file through the single-file route does not satisfy the read-before-edit guard for a subsequent edit of that path in the same session; reading it through the engine-parity read route does satisfy it. — FR-016
- **E2E-009**: Every mutating engine-parity route (including directory creation, deletion and move) is exercised once; the audit log shows one entry per mutation, proving each went through the shared engine commit path rather than a direct storage write. — FR-009
- **E2E-010**: An ambiguous edit submitted through this plane surfaces the identical stable error code and HTTP status as the equivalent refusal raised through the MCP surface. — FR-009, FR-007
- **E2E-011**: Issuing a GET against a mutating route's path yields a method-not-allowed response; the same path accepts POST with a valid body. — FR-010
- **E2E-012**: Requesting the documentation document and the documentation UI with no bearer token both succeed, while requesting the project-list route with the same absent credentials fails with the unauthenticated response, showing the data plane itself is still guarded. — FR-020
- **E2E-013**: In the documentation document, a route's documented parameter description matches, character for character, the description of the same-named parameter in its tool counterpart's schema; the two name-override routes' summaries match their respective overridden tool's description. — FR-017
- **E2E-014**: In the documentation document, the parameter list for a route whose tool counterpart accepts additional optional parameters is limited to the route's own narrower documented set. — FR-019
- **E2E-015**: Requesting the project-list route and an unknown project's scoped route with no bearer token both yield the identical unauthenticated response, and neither yields a not-found response, proving identity verification precedes membership checking. — FR-005, FR-006
- **E2E-016**: A non-member's request to a scoped route yields a forbidden response, while the same caller's request to a genuinely unknown project yields a not-found response, and a platform administrator who is not a project member also receives the forbidden response rather than success. — FR-007, FR-008
- **E2E-017**: Every route in the manifest, probed with its documented method against a running server, resolves to its handler rather than the router's own not-found fallback; probed with the wrong method, every one yields a method-not-allowed response. — FR-001, FR-010

## 9. Glossary

| Term | Definition |
|---|---|
| **Data plane** | The REST surface at the project-scoped path prefix, distinct from the MCP endpoint. |
| **Storage-direct route** | One of the 4 routes (project list, browser listing, single-file download, subtree-zip download) that read through the storage layer rather than through the engine. |
| **Engine parity** | The property that a route and its MCP tool counterpart call the same engine function and return the same result keys. |
| **Guard** | The shared identity-then-membership-then-operation wrapper every scoped route passes through. |
| **Route manifest** | The authoritative list of (method, operation) pairs every data-plane route must appear in. |
| **Explicit-summary table** | The table supplying summaries and parameter descriptions for the three routes that have no tool counterpart. |
| **Name override** | An exception to deriving a route's tool counterpart from its route name, used where the nearest tool has a differently-shaped name. |
| **Documentation UI** | The browsable documentation page, served from assets embedded in the server binary. |
| **Attachment** | A download response carrying a content type and a filename for the browser to save. |
| **Body ceiling** | The per-route-class limit on request size: a JSON ceiling and a larger multipart-upload ceiling. |

## 10. Confidence notes

- Confidence: **high**. Line citations in §6 were spot-checked directly against `crates/core/src/api/dataplane.rs` and `crates/core/src/api/openapi.rs` at the stated base commit; the route manifest, the body-size constants, the guard function, the three handlers quoted (download, download-zip, mkdir/delete/move) and the OpenAPI name-override and explicit-summary tables all matched.
- **Material drift found and corrected relative to the pre-move draft spec** (`specs/SPEC-0004_2026-09-18_18-30-00-rest-data-plane/spec.md`), which this document supersedes with current, verified behaviour:
  1. **Route count is 40, not 38.** Two routes (`export-zip`, `extract-archive`) were added after the draft was written (SPEC-0012, SPEC-0015), confirmed by the route-count assertion at `dataplane.rs:2424` and the manifest itself. A third route, `read-section`, is also present in the current manifest and router but absent from the draft's narrative description, though it does not affect the count claimed here since the draft's own count (38) was already stale before this addition; the count of 40 reflects the full current manifest including `read-section`.
  2. **`mkdir`, `delete` and `move` no longer read/write through the storage client directly.** The draft classified them as "bytes plane" helpers that call the storage client. The current code routes all three through the same engine function their MCP tool counterpart calls, each carrying explicit code comments describing this as a fix (a prior direct-storage-client implementation skipped the trash, the hard-delete guard, the quota and the audit log) and each wired into the shared search-index-update hooks. This narrows the storage-direct route set from the draft's implied 8 down to 4: project list, browser listing, single-file download, subtree-zip download.
  3. **The explicit-summary table (`REST_ONLY`) holds 3 entries, not 8.** Only upload, single-file download and subtree-zip download use it; `roots`, `list`, `mkdir`, `delete` and `move` each have genuine tool counterparts (`fs.list_allowed_roots`, `fs.list_dir`, `fs.mkdir`, `fs.delete`, `fs.move`) and inherit descriptions normally.
  4. The project's physical layout moved from `crates/mcp-fs/src/...` (as the draft cited) to `crates/core/src/...`, consistent with the current `AGENTS.md`; all citations in this document use the current path.
- The two-MIME-implementation note (download route vs. the byte-range read tool) was re-verified against current code and still holds as stated (`docs::guess_mime` vs. `fs_ops::mime_guess`), carried forward as a design note rather than an open question, since the two tables were previously audited as equivalent and nothing in the current diff touches either.
- E2E test identifiers were renumbered as a flat `E2E-NNN` sequence per the task's instruction; they are not a 1:1 mapping of the draft's `E2E-3xx` identifiers, several of which were merged where they tested the same observable property from slightly different angles (e.g., the draft's five separate guard-ordering tests collapse into E2E-015/E2E-016 here).
