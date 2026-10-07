# US-0002: Export creation — `fs.export_zip` tool, REST route, URL construction

> Parent Spec: specs/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md
> Spec ID: SPEC-0012
> Epic: n/a
> Status: ready
> Priority: 2
> Depends On: US-0001
> Complexity: L
> min_tier: 1
> Files touched: 3

> **Sizing note:** this story carries 9 FRs and 21 tests, above the tier-2 nominal budget (2-4 FRs,
> 5-15 tests). It was kept whole rather than split because all 9 FRs are one function's exhaustive
> branch enumeration (one creation path, six rejection clauses, the happy-path persistence and URL
> build) — a story implementing only "reject unauthorized" has nothing to test without the rest of
> the creation function existing. `min_tier` is set to 1 to route it to a model that can absorb this
> decision density in one run. This was reviewed and confirmed with the user at the Phase 3 slicing
> gate.

## Objective
Give callers — both MCP tool users and REST clients — a way to select an arbitrary list of files
and/or directories from a project and get back a signed, single-use download URL for a zip of them.
This story owns the entire creation path: validating the request (empty selection, authorization,
path bounds, missing paths), building the zip in memory, persisting it (via US-0001's new accessor),
and constructing the URL the caller gets back.

## Technical Context

### Stack
Rust 2024, axum, `zip` crate (already a dependency), `uuid` crate (already a dependency).

### Relevant File Structure
```
crates/core/src/
  tools/
    export.rs        <- NEW: fs.export_zip tool
    trash.rs          <- existing tool family to mirror the module structure of
  api/
    dataplane.rs      <- add POST /api/fs/{mount_id}/export-zip here, alongside download_zip
  config.rs           <- add public_base_url: String to HttpConfig
  storage/
    volume.rs         <- read-only reference: VolumeClient::walk/read_bytes, VolumeClient.blob
    meta.rs           <- read-only reference: US-0001's new insert accessor (call it, don't modify it)
```

### Existing Patterns
- **The exact precedent to copy from:** `download_zip` at `crates/core/src/api/dataplane.rs:595-627`.
  It is a `guarded`/`guarded_json`-wrapped handler (`dataplane.rs:182-217`) that: normalizes one path
  (`r.norm(...)`), walks it via `r.client.walk(&root)` (the call is at `dataplane.rs:607`), reads each
  file via `r.client.read_bytes(&full)` (the call is at `dataplane.rs:614`), builds a `zip::ZipWriter`
  over an in-memory `Cursor<Vec<u8>>`, and returns it via `attachment()` (`dataplane.rs:635`). This
  story's creation logic is the same three calls (`walk`, `read_bytes`, `zip::ZipWriter`) generalized
  over a **list** of paths instead of one subtree, with each zip entry named by its full
  project-relative path (not relativized to a chosen root, per DEC-007) instead of relativized to
  `root`.
- `r.client` in that handler is a `VolumeClient`; its `walk` method is `storage/volume.rs:130`, its
  `read_bytes` method is `storage/volume.rs:92`. **Do not go through `core::fs_ops`** — the spec's own
  Phase 6 audit (round 2) found `fs_ops.rs` has no `walk` function and no plain-bytes `read_bytes`
  function (only `read_bytes_b64`), and corrected every FR below to cite `VolumeClient` directly,
  matching this real precedent.
- Authorization/normalization order: `state.authorize(mount_id, person)` first, then
  `state.safety.normalize_path` on every path, before any read — this is the project-wide convention
  for every `fs.*`/`git.*` handler (see AGENTS.md's "Conventions" section, and `download_zip` itself
  for the normalize-then-walk order).
- `tools/trash.rs` is the module-structure precedent for a new tool family file (`tools/export.rs`):
  one `pub(crate) async fn` per tool, a `#[cfg(test)] register()` for the shared test harness, and the
  real logic called identically from both the MCP `#[tool]` method (`mcp/server.rs`) and the REST
  handler — never two implementations of the same operation.
- Token generation: `uuid::Uuid::new_v4().to_string()`, the exact pattern used for CSRF tokens in
  `token_screen.rs`, `deleted_projects_screen.rs:66`, `trash_screen.rs:69`.
- Blob key: `format!("export:{token}")`, mirroring `git/odb.rs:42-44`'s `blob_key(sha) ->
  format!("git:{sha}")`. Write it via `client.blob.put("export:{token}", zip_bytes)` — `VolumeClient.blob`
  is a public `Arc<dyn BlobBackend>` field at `storage/volume.rs:15`, reached directly, no wrapper.
- `HttpConfig` struct: `crates/core/src/config.rs:140-145` (fields `host`, `port`, `mcp_path` today).
  It is reached as `ServerConfig.server: HttpConfig` (`config.rs:856`) — the YAML key is `server:`,
  **not** `http:`. Every existing deployment config (`config/local.yaml.template`,
  `config/minio.yaml.template`) uses `server:`.
- `attachment()` helper (`dataplane.rs:635`) is **not** needed by this story — creation returns JSON
  `{"url": ...}`, never zip bytes. (`attachment()`'s visibility promotion to `pub(crate)` is owned by
  US-0003, which is the story that actually serves bytes.)

### Data Model (excerpt)
Same `export_links` table US-0001 built (`token`, `volume_id`, `created_at`, `expires_at`). This
story only **inserts** rows, via US-0001's insert accessor — it does not read, delete, or sweep them.

### Decisions That Govern This Story
- **DEC-001:** Extend the existing zip capability (arbitrary file list + signed-URL mode) rather than
  build a parallel route family — reuse the proven in-memory zip-building code path. **Implemented
  by:** FR-NEW-001, FR-NEW-002.
- **DEC-002:** Fixed 5-minute expiry, not configurable per call. **Implemented by:** FR-NEW-007.
- **DEC-003:** Usable from both MCP tools and REST. **Implemented by:** FR-NEW-001, FR-NEW-002.
- **DEC-007:** Relative paths preserved in the zip — each entry named by its full project-relative
  path. Guarantees no name collisions across an arbitrary multi-path selection without needing a
  "common root" computation. Rejected flattening to basenames (collides, loses structure).
  **Implemented by:** FR-NEW-001.
- **DEC-008:** Opaque random token (`uuid::Uuid::new_v4()`) + server-side record. **Implemented by:**
  FR-NEW-007.
- **DEC-010:** Unlimited file count/total size, matching today's `download-zip`. No new limit. This is
  an NFR (§7.1), not a testable positive FR — do not add a size check that isn't in any FR below.
- **DEC-014 (Approach A):** store export bytes directly in the `BlobBackend` under key
  `export:{token}` (blob key namespacing). **Implemented by:** FR-NEW-007.
- **DEC-015:** New config key `server.public_base_url` (default `""`). **Implemented by:**
  FR-NEW-008, FR-NEW-016.
- **DEC-017:** Zip filename returned to the recipient is fixed `export.zip` (no caller-supplied
  name) — this is actually consumed by US-0003's download route, not this story, but do not add a
  caller-supplied-name parameter here either; none is in any FR below.

### Applicable NFRs
- **§7.1 Performance:** fully buffered in memory, no streaming, no new size or file-count limit
  (matches `download-zip`'s existing unlimited behavior — do not add a limit).
- **§7.2 Security:** `Security: internal finding` at the spec level (for the download route in
  US-0003, not this story's creation path, which is fully authenticated/authorized like every other
  `fs.*` tool). This story's only security-relevant NFR: traces/logs on export creation (§7.5) must
  not record the full token value, only truncated/hashed.
- **§7.5 Observability:** INFO on export creation: `mount_id`, file count. Never trace file contents
  or the full token.

### Bounded Context
**Export creation**: "Reading selected files via the existing filesystem engine and packaging them."
Key entity: `export_links` row (pending state, created here).

## Functional Requirements

### FR-NEW-001 [EARS-E]: MCP export creation
- **EARS:** WHEN a caller invokes tool `fs.export_zip(mount_id, paths: [String])` with a non-empty
  `paths` THE system SHALL authorize the caller for `mount_id`, normalize every path, resolve each
  entry (directories walked recursively) via `VolumeClient::walk` (`storage/volume.rs:130`), read
  every resulting file's bytes via `VolumeClient::read_bytes` (`storage/volume.rs:92`) — the same two
  calls the existing `download_zip` route already uses (`dataplane.rs:607,614`) — and build an
  in-memory zip with each entry named by its full project-relative path.
- **Inputs / Outputs:** `mount_id: String`, `paths: [String]` -> an in-memory `zip::ZipWriter` buffer
  (consumed by FR-NEW-007 below)
- **Business Rules:** a directory entry contributes every file found by recursively walking it; a
  file entry contributes itself; every resulting zip entry name is the full project-relative path
  (leading slash stripped), guaranteeing no collisions
- **Exact names:** tool `fs.export_zip`; params `mount_id: String`, `paths: [String]`

### FR-NEW-002 [EARS-E]: REST export creation
- **EARS:** WHEN a caller issues `POST /api/fs/{mount_id}/export-zip` body `{"paths": [...]}` THE
  system SHALL perform identical logic to FR-NEW-001 via the same `VolumeClient::walk`/`read_bytes`
  calls (no second implementation of path resolution, reading, or zipping).
- **Inputs / Outputs:** path param `mount_id`, JSON body `{"paths": ["..."]}` -> JSON `{"url": "..."}`
  on success
- **Exact names:** route `POST /api/fs/{mount_id}/export-zip`, body field `paths`, response field
  `url`

### FR-NEW-003 [EARS-O]: Empty selection rejected
- **EARS:** IF `paths` is empty THEN THE system SHALL reject the request with error code
  `ERR_INVALID_ARGUMENT` and SHALL NOT create an export record or blob.
- **Exact names:** error code `ERR_INVALID_ARGUMENT` (`crates/core/src/errors.rs:20`)

### FR-NEW-004 [EARS-O]: Unauthorized caller rejected
- **EARS:** IF the caller is not authorized for `mount_id` THEN THE system SHALL reject with error
  code `ERR_FORBIDDEN` before resolving any path.
- **Business Rules:** authorization check happens strictly before any path is touched.
- **Exact names:** error code `ERR_FORBIDDEN` (`errors.rs:10`)

### FR-NEW-005 [EARS-O]: Path escaping the project root rejected
- **EARS:** IF any path in `paths` normalizes outside the project root THEN THE system SHALL reject
  the entire request with error code `ERR_PATH_OUT_OF_BOUNDS`, creating nothing.
- **Business Rules:** all-or-nothing — one valid path in the list does not save a request containing
  one invalid path.
- **Exact names:** error code `ERR_PATH_OUT_OF_BOUNDS` (`errors.rs:13`)

### FR-NEW-006 [EARS-O]: Nonexistent path rejected
- **EARS:** IF any entry in `paths` does not resolve to an existing file or directory THEN THE system
  SHALL reject the entire request with error code `ERR_NOT_FOUND`, naming the offending path,
  creating nothing.
- **Business Rules:** all-or-nothing, same as FR-NEW-005.
- **Exact names:** error code `ERR_NOT_FOUND` (`errors.rs:16`); error message MUST contain the
  offending path's literal string

### FR-NEW-007 [EARS-E]: Export record and blob creation
- **EARS:** WHEN the zip archive has been built THE system SHALL generate `token =
  uuid::Uuid::new_v4()`, store the zip bytes at blob key `export:{token}` in the volume's configured
  blob backend, insert one `export_links` row (`token`, `volume_id`, `created_at`=now RFC3339,
  `expires_at`=now+300s RFC3339) via US-0001's insert accessor, and return `{"url": ...}`.
- **Business Rules:** token generation follows `uuid::Uuid::new_v4()`; blob key follows the
  `git:{sha}` precedent with prefix `export:` instead of `git:`.
- **Exact names:** blob key format `export:{token}`; table `export_links`; columns `token`,
  `volume_id`, `created_at`, `expires_at`

### FR-NEW-008 [EARS-U]: URL construction
- **EARS:** THE system SHALL build `url` as `{server.public_base_url}/exports/{token}` when
  `public_base_url` is non-empty, and as the relative path `/exports/{token}` when it is empty (the
  default).
- **Exact names:** config key `server.public_base_url`

### FR-NEW-016 [EARS-U]: Config
- **EARS:** THE system SHALL add `public_base_url: String` (default `""`) to the existing
  `HttpConfig` struct (`crates/core/src/config.rs:140-145`), reachable as
  `ServerConfig.server.public_base_url` (`ServerConfig.server: HttpConfig` at `config.rs:856`) — not
  a new top-level `http:` YAML section, and not a rename of the existing `server` field. Expandable
  via `${VAR}` like every other config value.
- **Exact names:** `HttpConfig::public_base_url: String`, default `""`; effective config path
  `server.public_base_url`

## Acceptance Tests

> **100% must pass.** Run through `cargo test --workspace`. Loop fix/run/check until zero failures.
> Never run test files directly.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| volume `proj` | owned by `owner@test.com` | test fixture | ready |
| `/src/main.rs` | `b"fn main() {}"` | test fixture, written via existing `core::fs_ops` write path | ready |
| `/docs/readme.md` | `b"# Hello"` | same | ready |
| `/docs/notes/todo.txt` | `b"buy milk"` | same | ready |
| `/docs/日本語 résumé (final)!.txt` | `b"content"` | same, unicode/special-char path | ready |
| `/data/report.bin` | 10,000 bytes from `StdRng::seed_from_u64(42)` | test-generated | ready |
| `stranger@test.com` | not a member of `proj` | test fixture | ready |

### E2E-NEW-001: fs.export_zip happy path, mixed file+directory selection, MCP tool
- **Category:** Happy path
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-008
- **Steps:** Given the fixture above / When `owner@test.com` calls `fs.export_zip` with
  `{"mount_id": "proj", "paths": ["/src/main.rs", "/docs"]}` / Then the call returns `Ok` with JSON
  `{"url": "<matches ^/exports/[0-9a-f-]{36}$>"}`
- **Cleanup:** none (export remains unconsumed, out of scope for this test)
- **Priority:** Critical

### E2E-NEW-002: POST /api/fs/{mount_id}/export-zip happy path, identical logic to tool
- **Category:** Happy path
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002
- **Steps:** Given the fixture / When `POST /api/fs/proj/export-zip` body `{"paths": ["/src/main.rs",
  "/docs"]}`, header `Authorization: Bearer <owner_token>` / Then status is `200 OK`, body is
  `{"url": "<same regex>"}`
- **Priority:** Critical

### E2E-NEW-003: empty paths rejected with ERR_INVALID_ARGUMENT, creates nothing
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Steps:** Given zero `export_links` rows, zero `export:*` blob keys / When `fs.export_zip` is
  called with `{"mount_id": "proj", "paths": []}` / Then the error code is exactly
  `ERR_INVALID_ARGUMENT` / And `SELECT COUNT(*) FROM export_links WHERE volume_id='proj'` is `0` /
  And no blob key matching `export:*` exists
- **Priority:** Critical

### E2E-NEW-004: unauthorized caller rejected with ERR_FORBIDDEN before path resolution
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-004
- **Steps:** Given `/src/main.rs` exists, `stranger@test.com` not a member / When
  `stranger@test.com` calls `fs.export_zip` with `{"mount_id": "proj", "paths": ["/src/main.rs"]}` /
  Then error code is exactly `ERR_FORBIDDEN` / And no `export_links` row or blob was created
- **Priority:** Critical

### E2E-NEW-005: path escaping project root rejected with ERR_PATH_OUT_OF_BOUNDS, all-or-nothing
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005
- **Steps:** Given `/src/main.rs` exists / When `fs.export_zip` is called with
  `{"mount_id": "proj", "paths": ["/src/main.rs", "/../../../etc/passwd"]}` / Then error code is
  exactly `ERR_PATH_OUT_OF_BOUNDS` / And no row/blob created, even though `/src/main.rs` alone was
  valid
- **Priority:** Critical

### E2E-NEW-006: one missing file entry rejects the whole request with ERR_NOT_FOUND
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-006
- **Steps:** Given `/src/main.rs` exists, `/src/missing.rs` does not / When `fs.export_zip` is called
  with `{"mount_id": "proj", "paths": ["/src/main.rs", "/src/missing.rs"]}` / Then error code is
  exactly `ERR_NOT_FOUND`, message contains `/src/missing.rs` / And no row/blob created
- **Priority:** Critical

### E2E-NEW-007: missing directory entry also rejects with ERR_NOT_FOUND
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-006
- **Steps:** Given `/docs` exists, `/nonexistent-dir` does not / When `fs.export_zip` is called with
  `{"mount_id": "proj", "paths": ["/docs", "/nonexistent-dir"]}` / Then error code is exactly
  `ERR_NOT_FOUND`, message contains `/nonexistent-dir` / And no row/blob created
- **Priority:** High

### E2E-NEW-008: data variation, unicode and special characters
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Steps:** Given `/docs/日本語 résumé (final)!.txt` = `b"content"` / When
  `fs.export_zip({"paths": ["/docs/日本語 résumé (final)!.txt"]})`, then the returned URL is
  downloaded (via US-0003's route — this test may be deferred to run after US-0003 merges, or the zip
  bytes may be asserted directly from the blob store if US-0003 is not yet available) / Then the zip
  has exactly one entry named exactly `docs/日本語 résumé (final)!.txt` with bytes exactly `b"content"`
- **Priority:** High

### E2E-NEW-009: edge, path traversal attempts rejected
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-005
- **Steps:** Given the fixture / When `fs.export_zip` is called with
  `{"paths": ["/src/../../outside.txt"]}` and separately with `{"paths":
  ["..\\..\\windows\\system32"]}` / Then both return error code exactly `ERR_PATH_OUT_OF_BOUNDS` /
  And neither creates a row/blob
- **Priority:** Critical

### E2E-NEW-010: boundary, single file, empty directory, mixed selection
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-001
- **Steps:** Given `/a.txt`=`b"A"`, empty directory `/empty_dir`, `/b/c.txt`=`b"C"` / When three
  calls: (a) `{"paths": ["/a.txt"]}`, (b) `{"paths": ["/empty_dir"]}`, (c) `{"paths": ["/a.txt",
  "/b"]}` / Then (a) zip has 1 entry `a.txt`=`b"A"`; (b) zip has 0 entries (succeeds, not
  `ERR_NOT_FOUND`); (c) zip has 2 entries `a.txt`=`b"A"`, `b/c.txt`=`b"C"` (assert via blob-store
  read of `export:{token}`, not necessarily via download)
- **Priority:** High

### E2E-NEW-011: side effect, export_links row created with correct columns
- **Category:** Side effect
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007, FR-NEW-015
- **Steps:** Given `t0 = Utc::now()` read just before the call / When
  `fs.export_zip({"paths": ["/src/main.rs"]})` returns `url` / Then a row exists with `token` = UUID
  from `url`, `volume_id = "proj"`, `created_at` in `[t0, t0+2s]`, `expires_at = created_at + 300s`
  (±1s)
- **Priority:** Critical

### E2E-NEW-012: side effect, zip bytes stored at blob key export:{token}
- **Category:** Side effect
- **Scenario:** SC-001
- **Requirements:** FR-NEW-007
- **Steps:** Given `/src/main.rs` = `b"fn main() {}"` / When
  `fs.export_zip({"paths": ["/src/main.rs"]})` returns token `T` / Then `blob_store.read("export:T")`
  returns bytes that, parsed as a zip, contain exactly one entry `src/main.rs` = `b"fn main() {}"`
- **Priority:** Critical

### E2E-NEW-013: URL uses public_base_url when configured
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008, FR-NEW-016
- **Steps:** Given `ServerConfig.server.public_base_url = "https://files.example.com"` / When
  `fs.export_zip({"paths": ["/src/main.rs"]})` / Then `url` is exactly
  `"https://files.example.com/exports/{token}"`
- **Priority:** High

### E2E-NEW-014: URL is relative when public_base_url is empty
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-008
- **Steps:** Given `ServerConfig.server.public_base_url = ""` (default) / When
  `fs.export_zip({"paths": ["/src/main.rs"]})` / Then `url` is exactly `"/exports/{token}"` and does
  not start with `http`
- **Priority:** High

### E2E-NEW-041: failure, unauthenticated MCP tool call rejected consistently with other tools
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-004
- **Steps:** Given no resolvable identity attached to the call context / When `fs.export_zip` is
  invoked with `{"mount_id": "proj", "paths": ["/src/main.rs"]}` / Then the error code matches
  exactly what an existing tool (e.g. `fs.read_text`) returns under the identical missing-identity
  condition (cross-checked, not hardcoded) / And no row/blob created
- **Priority:** High

### E2E-NEW-042: failure, malformed JSON body on REST route returns 400
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003
- **Steps:** Given a valid bearer token / When `POST /api/fs/proj/export-zip` body `{"paths":
  "not-an-array"}` / Then status is exactly `400`, no row/blob created
- **Priority:** Medium

### E2E-NEW-043: edge, empty-string path entry rejected, never silently skipped
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-003, FR-NEW-006
- **Steps:** Given `/src/main.rs` exists / When `fs.export_zip` is called with
  `{"mount_id": "proj", "paths": ["/src/main.rs", ""]}` / Then the call returns an error (exact code
  recorded: `ERR_NOT_FOUND` or `ERR_INVALID_ARGUMENT`, whichever the implementation actually
  produces — never a silent `200` with the entry dropped) / And no row/blob created
- **Priority:** Medium

### E2E-NEW-044: failure, unauthorized caller rejected via the REST route too
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-004
- **Steps:** Given `stranger@test.com` not a member of `proj`, `/src/main.rs` exists / When `POST
  /api/fs/proj/export-zip` body `{"paths": ["/src/main.rs"]}` with `stranger@test.com`'s bearer
  token / Then status is exactly the REST equivalent of `ERR_FORBIDDEN` (`403`), no row/blob created
- **Priority:** High

### E2E-NEW-045: failure, path escaping project root rejected via the REST route too
- **Category:** Failure
- **Scenario:** SC-001
- **Requirements:** FR-NEW-002, FR-NEW-005
- **Steps:** Given `/src/main.rs` exists, owner's bearer token / When `POST /api/fs/proj/export-zip`
  body `{"paths": ["/src/main.rs", "/../../../etc/passwd"]}` / Then status is the REST equivalent of
  `ERR_PATH_OUT_OF_BOUNDS` (`400`), no row/blob created
- **Priority:** High

### E2E-NEW-051: edge, public_base_url supports ${VAR} expansion like every other config value
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-016
- **Steps:** Given env var `MCPFS_PUBLIC_URL=https://files.example.com` and config
  `server.public_base_url: "${MCPFS_PUBLIC_URL}"` / When the config is loaded / Then
  `HttpConfig.public_base_url` resolves to exactly `"https://files.example.com"`
- **Priority:** Medium

### E2E-NEW-056: structural, public_base_url defaults to empty string when unset
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-NEW-016
- **Steps:** Given a config file with no `server.public_base_url` key at all / When the config is
  loaded / Then `HttpConfig.public_base_url` resolves to exactly `""` (the declared default)
- **Priority:** Medium

### E2E-NEW-022: zip-content integrity end to end, byte-for-byte round trip
- **Category:** Data integrity
- **Scenario:** SC-002 (FR-NEW-001's half; owned here since it's the creation-side byte-fidelity
  guarantee — the download-side assertion in US-0003 reuses this fixture)
- **Requirements:** FR-NEW-001, FR-NEW-009
- **Steps:** Given `/data/report.bin` written with 10,000 bytes from `StdRng::seed_from_u64(42)`,
  captured as `expected_bytes` / When `fs.export_zip({"paths": ["/data/report.bin"]})`, then assert
  directly against the blob store at `export:{token}` (parsed as a zip) that the single entry
  `data/report.bin` equals `expected_bytes` exactly (full comparison) — if US-0003 is already merged,
  additionally confirm the same via an actual `GET` of the URL
- **Priority:** Critical

### E2E-NEW-039: security, no invalidation surface exists (tool contract + CLI verbs)
- **Category:** Security (structural)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-018 (this test's half: verified once this story's `fs.export_zip` entry is
  in `TOOL_CONTRACT.txt`; the DELETE-route half of FR-NEW-018 is E2E-NEW-054, owned by US-0003)
- **Steps:** Given the full tool contract (`TOOL_CONTRACT.txt:1-620`) and the `mcp-fs` CLI verbs
  (`serve | keys | token | migrate | purge | version`, `crates/core/src/cli.rs`) / When searching for
  any export-invalidation entry / Then no tool name other than `fs.export_zip` contains `export`; no
  CLI verb other than `purge` touches `export_links`
- **Priority:** High

### E2E-NEW-053: security (structural), no admin.* tool exposes export invalidation
- **Category:** Security (structural)
- **Scenario:** SC-001
- **Requirements:** FR-NEW-018
- **Steps:** Given the 14 `admin.*` tool entries in `TOOL_CONTRACT.txt:1-66` / When searching for any
  entry naming `export` / Then none exists
- **Priority:** Medium

## Constraints

### Files Not to Touch
- `crates/core/src/storage/meta.rs` — call US-0001's insert accessor, do not modify its signature or
  add a second insert path.
- `crates/core/src/storage/volume.rs` — no change needed (confirmed in the spec's audit).
- `crates/core/src/exports.rs` — does not exist yet; that is US-0003's file.
- `crates/core/src/app.rs` — not touched by this story.

### Dependencies Not to Add
- No new crate. `zip` and `uuid` are already present.

### Patterns to Avoid
- Do not route reads through `core::fs_ops` — the spec's own audit found this to be wrong for this
  feature; use `VolumeClient::walk`/`read_bytes` directly, matching `download_zip`.
- Do not add a per-call configurable expiry or a file-count/size limit — neither is in any FR above
  (DEC-002, DEC-010).
- Do not add a caller-supplied zip filename parameter — the filename is fixed at the download side
  (DEC-017), not this story's concern.

### Scope Boundary
- This story does not implement `GET /exports/{token}` (US-0003) or the purge sweep (US-0004). Tests
  above that would ideally confirm via an actual download (E2E-NEW-008, E2E-NEW-010, E2E-NEW-022) may
  assert directly against the blob store instead if US-0003 is not yet merged when this story runs.

## Non Regression

### Existing Tests That Must Pass
- `download_zip`'s own existing tests, unmodified (`dataplane.rs`'s `mod tests`).
- Every existing `config.rs` test, notably the `HttpConfig`/`ServerConfig` default and override tests
  (`config.rs:1152-1155`, `config.rs:1328-1330`).

### Behaviors That Must Not Change
- `GET /api/fs/{mount_id}/download-zip` is completely unmodified.
- `ServerConfig.server`'s existing fields (`host`, `port`, `mcp_path`) and YAML key are unchanged.

### API Contracts to Preserve
- No existing route, tool, or config key changes shape. This story is purely additive.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
