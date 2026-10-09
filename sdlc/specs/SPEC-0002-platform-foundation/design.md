# Platform Foundation — Design

Companion to `sdlc/specs/SPEC-0002-platform-foundation/spec.md`. This document carries every mechanism, file reference, schema and API-shape detail stripped from the functional specification.

## 1. Components

- **Composition root** (`crates/core/src/app.rs`): assembles shared state and the HTTP router; mounts `/health` and the MCP path.
- **Shared state** (`crates/core/src/state.rs`): cheap to clone, everything behind `Arc`; authorization ladder — `require_admin`, `require_owner_or_admin`, `authorize` (membership only).
- **Storage contracts** (`crates/core/src/storage/traits.rs`): `MetaBackend`, `BlobBackend`, `AdminBackend`.
- **Safety** (`crates/core/src/safety.rs`): session state keyed by `(person, project)`; path normalization, read-before-write, write quota, capped audit, trash path derivation.
- **Config** (`crates/core/src/config.rs`): `ServerConfig`, `${VAR}` expansion, `Dsn` (redacts in Debug/Display), boot validation of every `infra.*` store.
- **Errors** (`crates/core/src/errors.rs`): the stable `ERR_*` codes, `ToolError`, HTTP status mapping, `retryable` flag.
- **Identity** (`crates/core/src/identity.rs`): RS256-family verification, 30s clock skew, forwarded-header-then-`Authorization` precedence.
- **MCP protocol surface** (`crates/core/src/mcp/server.rs`): one `#[tool]` method per catalogue entry, backed by `rmcp`. **Note:** this replaces the hand-rolled JSON-RPC/SSE layer the legacy spec (`crates/mcp-fs/src/mcp/mod.rs`, now removed) described; see Section 9 and the findings below.
- **Storage** (`crates/core/src/storage/`): `rel/` (the relational seam: `RelationalDb`/`RelationalTx`, `dialect.rs`, `schema.rs`, per-engine drivers, `run_retrying`), `meta.rs` (nodes + blob_refs, content addressing, refcount GC), `admin.rs` (project/project_member, caseless ACL, project id validation), `blob/local.rs` + `blob/s3.rs`, `volume.rs` (VolumeClient).
- **Admin tools** (`crates/core/src/tools/admin.rs`): the ten `admin.*` capabilities.
- **Keys/tokens** (`crates/core/src/keys.rs`): keypair generation, token minting.

## 2. Flows

See `spec.md` Section 5 for the functional flow of each scenario (SC-001 through SC-010). Mechanism-level notes:

- **Boot (SC-001):** config path resolution precedence is explicit `--config`/`-c` → `$MCP_FS_CONFIG` → `$XDG_CONFIG_HOME` probe → dir/name default (`cli.rs:242-270`). `${VAR}` and `${VAR:-default}` expansion precedes YAML parse (`config.rs:1006`, `config.rs:802-806`). CLI feature flags (`--git --web --context7 --sqlite --db --doc`) force-enable only (`cli.rs:57-74`, `app.rs:73-81`).
- **Identity (SC-002):** header precedence is `auth.jwt.header` (default `X-Forwarded-Authorization`) then `Authorization`, accepting `Bearer`/`Basic`/bare token (`identity.rs:118-148`). 30s leeway on `exp`/`nbf` (`identity.rs:167`). Accepted algorithms: `RS256`, `RS384`, `RS512`, `PS256`, `PS384`, `PS512`, `ES256`, `ES384` (`identity.rs:34-44`); `HS*` is dropped at construction (`identity.rs:87-90`).
- **Admin lifecycle (SC-003, SC-004, SC-005, SC-007, SC-008):** gates live in `tools/admin.rs`, data operations in `storage/admin.rs`. Project id format: 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds (`storage/admin.rs:345-357`). Index mode persisted before index wipe (`tools/admin.rs:322-325`).
- **Authorization (SC-006):** `authorize` delegates to `AdminBackend::require_member`, no admin check (`state.rs:61-63`). Owner-or-admin gate skips ownership but not existence (`state.rs:49-60`).
- **Safety (SC-009):** `PosixPath::normpath` is the single normalization implementation (`util/posix.rs`); path-length ceiling counted in UTF-16 code units for SQL Server compatibility (`safety.rs:64-65`), SQLite/PostgreSQL have none (`meta.rs:89-100`). `AUDIT_CAP = 500` (`safety.rs:13`), ring drops from the front.
- **Content addressing (SC-010):** sha256 computed before put (`volume.rs:123`); `blob_refs` keyed `(volume_id, sha256)` (`meta.rs:862-863`); local blob layout `{dir}/{bucket}/{sha[..2]}/{sha}` is part of the on-disk contract (`storage/blob/local.rs:4-6,24,33`).

## 3. Interfaces

- **Transport:** MCP over streamable HTTP, built on `rmcp`'s `StreamableHttpService` with `LocalSessionManager` and `legacy_session_mode: true`, mounted alongside `/health`. (Historical note: the legacy spec described a hand-rolled JSON-RPC/SSE framing layer at `mcp/mod.rs`; that module and its framing rules no longer exist in the current tree — see Findings.)
- **Wire contract:** `TOOL_CONTRACT.txt` (human-readable) and `tool-contract-golden.json` (machine-checked every test run) remain the single source of truth for exact tool names, descriptions and `inputSchema`. This design document, like the spec, does not restate them.
- **Error shape:** `ToolError` carries `code`, `message`, `http_status()`, `is_client_error()`, `retryable` (`errors.rs`).

## 4. Data and state

| Entity | Key | Fields | Home |
|---|---|---|---|
| **Project** | `id` | `id`, `owner`, `created_at`, `index_mode` | `project` table (`storage/admin.rs:27-40`), types `traits.rs:100-108` |
| **Member** | `(project_id, person)` | `project_id`, `person`, `role` (`owner`/`member`), `added_by`, `added_at` | `project_member` table (`storage/admin.rs:41-50`), types `traits.rs:110-119` |
| **NodeRow** | `(volume_id, path)` | `path`, `parent`, `name`, `kind` (`dir`/`file`), `size`, `mode`, `mtime`, `ctime`, `atime`, `sha256` | `nodes` table (`meta.rs:117-138`), type `traits.rs:12-25` |
| **BlobRef** | `(volume_id, sha256)` | `volume_id`, `sha256`, `refcount`, `size` | `blob_refs` table (`meta.rs:139-145`) |
| **Blob** | `sha256` | raw bytes | blob backend; local layout `{dir}/{bucket}/{sha[..2]}/{sha}` |
| **SessionState** | `(person, project)` | recorded reads, `bytes_written`, `audit` ring | in memory (`safety.rs:28`) |

Default POSIX modes: `MODE_DIR = 0o040755`, `MODE_FILE = 0o100644` (`traits.rs:36-38`).

## 5. Configuration

Config sections owned by this area: `server`, `auth`, `infra`, `safety`. `${VAR}`/`${VAR:-default}` expansion precedes parse. Boot validates `infra.meta`, `infra.admin`, `infra.git`, `infra.oauth`, plus `doc_service` and `search` presence (owned by later specs, validated here only for "fails to boot on invalid block"). `server.mcp_path` must start with `/`. `safety.read_guard` defaults true; `safety.allow_hard_delete` defaults false (`config.rs:370-371`).

## 6. Observability (mechanism)

`tracing` + `tracing-subscriber` only, `fmt` layer to stderr, filter from `RUST_LOG` (default `info`). Tool failures classified by `is_expected`: 4xx-equivalent logs INFO with no backtrace, 5xx-equivalent logs ERROR (`logging.rs:9-13`, `app.rs:258`). No OpenTelemetry dependency exists in the workspace as of the legacy document's writing; this is tracked as backlog item BL-001. **This should be re-verified**, since the dependency surface may have changed since SPEC-0013 (rmcp migration) — see Findings.

## 7. Decisions

| ID | Decision | Evidence | Rationale |
|---|---|---|---|
| DEC-001 | Split the retro-specification into eight sequenced specs along the dependency spine rather than one monolith or one spec per tool family. | `crates/mcp-fs/src/tools/all.rs:20-46` (legacy path) | A single spec would carry hundreds of FRs and over a thousand tests; one per family would be twenty near-empty specs. |
| DEC-002 | This spec owns the MCP protocol layer, not the filesystem-engine spec. | `crates/mcp-fs/src/app.rs:228-247` (legacy path) | The admin tools are MCP tools; without the protocol layer they have no callable surface and this spec's tests could not run end to end. |
| DEC-003 | This spec covers the default relational and local blob backends only; additional engines and the migrate verb defer to a later spec. | `crates/mcp-fs/Cargo.toml:31-44` (legacy path) | The default build carries neither optional driver, so the default posture is the honest baseline. |
| DEC-004 | This spec specifies only the config sections it owns (`server`, `auth`, `infra`, `safety`); other sections defer to their owning specs. | `config.rs:772-786` | Otherwise this spec would swallow configuration it does not implement (git, web, search, doc). |
| DEC-005 | Describe the as-built `tracing` setup and declare distributed tracing/telemetry export absent, deferred to backlog rather than specified as new work. | `Cargo.toml:98`, `crates/mcp-fs/Cargo.toml:79` (legacy paths); `logging.rs:29-37` | A retro-specification must describe what exists; inventing a new requirement would convert it into an implementation project. |
| DEC-006 | Keep the safety layer and content addressing specified at the engine boundary in this spec, rather than moving them to the filesystem-engine spec where they become tool-observable. | `core/fs_ops.rs` is the only caller of the safety manager for tool paths; tests at `safety.rs:186-290`, `storage/meta.rs:902-943` | They are invariants every other layer rests on; moving them would leave the filesystem spec owning both the fs surface and the safety engine. |
| DEC-007 | Place tool-level tests in `tests/functional/scenarios/` and engine-level tests in Rust `#[cfg(test)]` modules, rather than a new top-level `tests/` tree. | `tests/functional/scenarios/` (16 scenario scripts); no crate under `crates/` has its own `tests/` directory | Those are the two layers that already exist and already run. |
| DEC-008 | Reference existing tests rather than re-specify them; require only that each carry its `E2E-XXX` id. | test function counts per file, recorded per legacy document §9.3 | Roughly 240 unit tests already cover this layer; re-specifying them would pad the document without adding an assertion. |

### Implementability audit (carried forward as supporting notes, not re-run)

Audited as part of a cross-spec audit on 2026-09-18 (see legacy `specs/AUDIT.md`). Sub-agent execution was unavailable at that time (provider budget error); mechanical verification plus a targeted reading pass substituted for it. Round 1 result: 1 functional finding (closed — key-generation/token-minting requirement added), 3 drift findings (a `state.rs` citation, a blob-layout citation, a `normalize_identity` citation), verdict IMPLEMENTABLE-WITH-DRIFT, amendments applied in place. The drift register itself was left empty by design: every finding was corrected in place rather than logged as debt.

## 8. Requirement to code map

| Requirement (SPEC-0002/FR-00x) | Files |
|---|---|
| FR-001, FR-002, FR-003, FR-004, FR-006 | `crates/core/src/config.rs`, `crates/core/src/cli.rs` |
| FR-005, FR-011..FR-019 | `crates/core/src/app.rs` |
| FR-007..FR-011 | `crates/core/src/identity.rs` |
| FR-012..FR-019 | `crates/core/src/mcp/server.rs` (historically `crates/mcp-fs/src/mcp/mod.rs`, removed — see Findings) |
| FR-020 | `crates/core/src/errors.rs` |
| FR-021..FR-024 | `crates/core/src/state.rs` |
| FR-025..FR-031 | `crates/core/src/tools/admin.rs`, `crates/core/src/storage/admin.rs` |
| FR-032..FR-037 | `crates/core/src/safety.rs` |
| FR-038..FR-042, FR-044 | `crates/core/src/storage/meta.rs`, `crates/core/src/storage/volume.rs`, `crates/core/src/storage/rel/` |
| FR-043 | `crates/core/src/keys.rs` |

### Affected components (impact, as of the legacy document; no code change required by this spec)

| File/Module | Impact Type | Description |
|---|---|---|
| `config.rs` | Specified, unchanged | FR-001..FR-004, FR-006 |
| `errors.rs` | Specified, unchanged | FR-020 |
| `identity.rs` | Specified, unchanged | FR-007..FR-011 |
| `safety.rs` | Specified, unchanged | FR-032..FR-037 |
| `state.rs` | Specified, unchanged | FR-021..FR-024 |
| `app.rs` | Specified, unchanged | FR-005, FR-011..FR-018 |
| `mcp/` | Specified, **superseded** (see Findings) | FR-012..FR-019 |
| `storage/` | Specified, unchanged | FR-038..FR-044 |
| `tools/admin.rs` | Specified, unchanged | FR-025..FR-031 |
| `tests/functional/scenarios/` | Extended | New scenario scripts for the legacy gap list |

### Documentation requirements

- `AGENTS.md`: add a pointer to `sdlc/specs/`, naming this spec as the platform-foundation contract. (Current `AGENTS.md` already documents a different, more current architecture; reconcile rather than duplicate.)
- `.agent_docs/architecture.md`: cross-reference the spec's scenarios and requirements rather than restating them.
- `README.md`: no change required.
- Consistency notes carried from the legacy document: `TOOL_CONTRACT.txt`/`tool-contract-golden.json` remain authoritative for tool schemas; if this spec and the golden file ever disagree, the golden file wins.

### Migration & implementation notes (carried forward, historical)

No production code change was required by the legacy document. Its ordered test/doc work: (1) annotate existing tests with their `E2E-XXX` id before writing new ones; (2) add new engine-level tests first, since they need no server or fixtures; (3) add new protocol/tool scenarios extending the existing scenario scripts; (4) a deliberate provisioning-failure test must be induced by configuration (e.g. a read-only metadata directory), never by a test-only hook in production code; (5) a tool-count assertion depending on an optional external tool (pandoc) must guard/skip consistently with existing conditional registration; (6) documentation updates last, once ids are stable. Given the protocol-layer supersession noted in Findings, step (3) in particular needs re-scoping against the current `rmcp`-based transport before it is executed.

## 9. Legacy mapping

Source: specs/SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md (pre-move)

| Old id | New id | Note |
|---|---|---|
| DEC-001 | DEC-001 | carried forward as-is |
| DEC-002 | DEC-002 | carried forward as-is |
| DEC-003 | DEC-003 | carried forward as-is |
| DEC-004 | DEC-004 | carried forward as-is |
| DEC-005 | DEC-005 | carried forward as-is; BL-001 (OTel) still open, see Findings |
| DEC-006 | DEC-006 | carried forward as-is |
| DEC-007 | DEC-007 | carried forward as-is |
| DEC-008 | DEC-008 | carried forward as-is |
| TBD-001 (open question: session state not shared across replicas) | — | restated verbatim in spec.md §7.7 as a scalability limitation, not resolved |
| TBD-002 (open question: `admin.list_users` privacy posture) | — | restated as an open question; not resolved here |
| TBD-003 (open question: tool-count assumption re: search embedding endpoint) | — | superseded in practice by the current 102-tool contract; needs re-verification, see Findings |
| Implementation Drift Register | — | empty in the legacy document; no entries to map |
