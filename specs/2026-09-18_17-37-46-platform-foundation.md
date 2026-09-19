# mcp-fs Platform Foundation — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

## 1. Executive Summary

`mcp-fs` is a streamable-HTTP MCP server exposing a simulated multi-project filesystem. This document specifies its **platform foundation**: the layer every other capability stands on. It covers configuration and boot, the error code vocabulary, bearer-token identity, the hand-rolled MCP protocol layer, the storage seam (metadata tree, content-addressed blobs, project ACL), the safety contract (path normalization, read-before-write, write quota, audit, trash), and the 10 `admin.*` tools.

This is a **retro-specification**: the behaviour described is already implemented and shipping. Its purpose is to make the contract explicit, testable and auditable, so that later work has a written baseline instead of an oral tradition. Every statement about the current code carries a `file:LINE` citation; anything uncited is labelled `ASSUMED:`.

It is the first of a planned sequence. Specs S2 through S8 (filesystem engine and `fs.*` tools, REST data plane, multi-backend storage, documents, git, search and RAG, optional integrations) build on the contract fixed here.

## 2. Current State Analysis

### 2.1 Project Overview

A Rust 2024 workspace (`crates/mcp-fs`, `crates/agent`) built on axum + tokio. The server exposes 45 always-on MCP tools (35 `fs.*`, 10 `admin.*`) and up to 34 more behind config flags. Server state lives in a relational store, SQLite by default; blob bytes live in `infra.blob`, local or S3. The MCP wire format is hand-rolled rather than taken from the `rmcp` SDK, a decision recorded at `crates/mcp-fs/src/mcp/mod.rs:3-8`.

### 2.2 Existing Specifications

None. `specs/` did not exist before this document; this is the first specification in the repository. The de-facto contract to date has been `TOOL_CONTRACT.txt` (human readable) and `tool-contract-golden.json` (machine checked on every test run), plus `.agent_docs/*.md`.

### 2.3 Relevant Architecture

- **Composition root**: `crates/mcp-fs/src/app.rs:43` assembles `AppState` and the axum `Router`; `app.rs:114-115` mounts `/health` and the MCP path.
- **Shared state**: `crates/mcp-fs/src/state.rs:17-29`, cheap to clone, everything behind `Arc`.
- **Authorization ladder**: `state.rs:38` (`require_admin`), `state.rs:49-60` (`require_owner_or_admin`), `state.rs:61-63` (`authorize`, membership only).
- **Storage contracts**: `crates/mcp-fs/src/storage/traits.rs:126-187`, three traits: `MetaBackend`, `BlobBackend`, `AdminBackend`.
- **Safety**: `crates/mcp-fs/src/safety.rs:31-172`, session state keyed by `(person, project)`.

## 3. Scope

### 3.1 In Scope

- Configuration schema for `server`, `auth`, `infra`, `safety`; `${VAR}` expansion; boot validation; config path resolution; the `serve`, `keys`, `token`, `version` CLI verbs.
- The 14 `ERR_*` codes, their HTTP status mapping and the `retryable` flag.
- Bearer JWT identity: header precedence, accepted algorithms, issuer/audience/expiry validation, claim extraction, caseless normalization.
- The MCP protocol layer: JSON-RPC 2.0 framing over SSE, `initialize`, `tools/list`, `tools/call`, notification handling, error shapes, the tool registry and its name resolution.
- The storage seam: `MetaBackend`, `BlobBackend`, `AdminBackend` contracts; content addressing, dedup and refcount GC; `volume_id` scoping; `LIKE` pattern escaping; volume provisioning and teardown.
- The project ACL: project id validation, caseless membership, the owner/member roles, `IndexMode` persistence.
- The safety contract: path normalization, path length ceiling, read-before-write guard, per-session write quota, capped audit log, trash path derivation.
- The 10 `admin.*` tools, their schemas, authorization gates and return shapes.

### 3.2 Out of Scope (Non-Goals)

- The 35 `fs.*` tools and `core/fs_ops.rs` (spec S2). **Consequence:** the safety layer and content addressing specified here are exercised end to end only through S2. This spec verifies them at the `SafetyManager` / `MetaBackend` / `VolumeClient` level, which is where their existing tests already live (`safety.rs:186-290`, `meta.rs:902-943`).
- The REST data plane and OpenAPI (S3).
- PostgreSQL, SQL Server, S3/MinIO blob storage, and the `migrate` verb (S4). The `Dialect` seam is in scope only as the abstraction boundary; no engine beyond SQLite is specified here. `storage/conformance.rs` is one suite run against every engine and will be **extended**, not replaced, by S4.
- Document extraction and `doc_service` (S5), git (S6), search and RAG (S7), `web`/`context7`/`sqlite`/`db` tool families (S8).
- The `crates/agent` CLI client.
- OpenTelemetry. It does not exist in this codebase; see §7.5 and `specs/BACKLOG.md` entry BL-001.

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **Operator** | Deploys and runs the server. Owns the YAML config, the RSA keypair and the process lifecycle. Never authenticated: acts through the filesystem and the CLI. |
| **Platform admin** | A person listed in `auth.admins` (`config.rs:190-195`). Creates and deletes projects, lists every project and every person. Does **not** thereby gain file access. |
| **Project owner** | The person named as `owner` at project creation. Manages membership and the index mode of that project. |
| **Project member** | A person added to a project. May read the project's data and list its members. |
| **MCP client** | Any program speaking JSON-RPC 2.0 over HTTP POST to the MCP path with a bearer token. The `crates/agent` CLI is one such client. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Platform** | Projects, membership, platform administration. The ACL registry. | Project, Member, Person |
| **Volume** | One project's simulated filesystem: metadata tree plus content-addressed bytes. | NodeRow, blob, volume_id |
| **Session** | Per-caller, per-project transient safety state. In memory, lost on restart. | SessionState, AuditEntry, write quota |
| **Protocol** | The MCP wire surface: JSON-RPC framing, tool registry, schemas. | ToolSchema, ToolRegistry, ToolCtx |

A "person" in the Platform context is an identity string normalized caseless; in the Session context the same string is half of the session key. A "project id" in the Platform context is the ACL primary key; the same value is the `mount_id` parameter in the Protocol context and the `volume_id` column in the Volume context.

## 5. Usage Scenarios

### SC-001: Operator boots the server from a YAML config

**Actor:** Operator
**Preconditions:** binary built; a resolvable YAML config; an RS256 keypair (`mcp-fs keys --dir .keys`, `cli.rs:76-80`); every `${VAR}` referenced in the YAML set in the environment.
**Flow:**
1. Operator runs `mcp-fs serve`, optionally `-c/--config PATH` and any of `--git --web --context7 --sqlite --db --doc` (`cli.rs:52-74`).
2. The config path resolves: explicit `--config` wins even when absent, then `MCP_FS_CONFIG`, then an `XDG_CONFIG_HOME` probe, then the dir/name pair (`cli.rs:242-270`).
3. The file is read, `${VAR}` and `${VAR:-default}` are expanded (`config.rs:1006`), and parsed into `ServerConfig` (`config.rs:802-806`).
4. `validate()` checks `infra.meta`, `infra.admin`, `infra.git`, `infra.oauth`, `doc_service` and `search` (`config.rs:813-821`).
5. CLI feature flags force their family on over the YAML (`cli.rs:57-74`).
6. `app::build` assembles state, registers tools, mounts `/health` and the MCP path (`app.rs:43,73-82,114-115`).
7. The listener binds and serves with graceful shutdown (`app.rs:145-148`).

**Postconditions:** `GET /health` answers `{"status":"ok","version":VERSION}` unauthenticated (`app.rs:183-185`); the MCP endpoint answers POST at the configured path; the registry holds exactly the enabled families.
**Exceptions:**
- EXC-001a: config file absent → `ERR_INVALID_ARGUMENT`, `"configuration file not found: {path} ({e})"` (`config.rs:794-798`)
- EXC-001b: malformed YAML → `ERR_INVALID_ARGUMENT`, `"invalid config: {e}"` (`config.rs:805`)
- EXC-001c: store block misconfigured → `validate_store` rejection naming the section (`config.rs:874-915`)
- EXC-001d: `server.mcp_path` not starting with `/` → boot aborts (`app.rs:85-87`)
- EXC-001e: address in use → `"cannot bind {addr}: {e}"` (`app.rs:146-147`)
- EXC-001f: no config found anywhere → the resolver returns the list of paths tried (`cli.rs:233-235`)

**Cross-scenario notes:** step 4 validates `doc_service` and `search`, owned by S5 and S7; this spec requires only that boot fails on an invalid block.

### SC-002: Client authenticates with a bearer JWT and discovers the tools

**Actor:** MCP client
**Preconditions:** server running; `auth.jwt.public_key_path` points at the public key matching the token's signing key; the token carries the configured issuer and claim.
**Flow:**
1. Client mints or obtains a token (`mcp-fs token <email>`, `cli.rs:82-99`).
2. Client POSTs `{"jsonrpc":"2.0","id":1,"method":"initialize"}` with `X-Forwarded-Authorization: Bearer <token>`.
3. The endpoint resolves identity before dispatch (`app.rs:193-202`).
4. The server answers `initialize` with protocol version `2024-11-05` and its capabilities (`mcp/mod.rs:29,79-85`).
5. Client POSTs `tools/list`; the server returns the registry payload (`app.rs:232`).
6. Client POSTs `tools/call` for a tool it is entitled to.

**Postconditions:** the client holds the tool catalogue; every subsequent call is attributed to the normalized identity from the token's claim.
**Exceptions:**
- EXC-002a: no bearer in either header → 401, `{"error":"ERR_UNAUTHENTICATED","detail":"no bearer token in request headers"}` (`identity.rs:190`, `app.rs:271-280`)
- EXC-002b: signature, issuer, `exp` or `nbf` invalid → 401, `"invalid token: {e}"` (`identity.rs:177`)
- EXC-002c: no public key configured → 401, `"no JWT public key configured"` (`identity.rs:153`)
- EXC-002d: claim missing → 401, `"token has no '{claim}' claim"` (`identity.rs:181-183`)
- EXC-002e: claim present but blank → 401, `"token identity claim is empty"` (`identity.rs:185`)
- EXC-002f: unknown method → `-32601`, `"Method '{other}' is not available."` (`app.rs:236-240`)

### SC-003: Platform admin creates a project, which provisions its volume

**Actor:** Platform admin
**Preconditions:** caller listed in `auth.admins`; the project id is free.
**Flow:**
1. Caller invokes `admin.create_project` with `project_id` and `owner`.
2. The handler requires platform admin (`tools/admin.rs:171`), validates the id (`:172`) and rejects a blank owner (`:173-175`).
3. The ACL row is created with the owner's membership (`storage/admin.rs:123`).
4. The volume is provisioned (`tools/admin.rs:177`, `storage/mod.rs:421`).
5. The tool returns `{"project_id","owner","created_at"}` (`tools/admin.rs:181-185`).

**Postconditions:** the project exists in the ACL with `index_mode` `none`; the owner is a member with role `owner`; the volume exists with a root directory (`meta.rs:185`).
**Exceptions:**
- EXC-003a: caller not a platform admin → `ERR_FORBIDDEN` (`state.rs:42-44`)
- EXC-003b: id fails validation → `ERR_INVALID_ARGUMENT`, `"project_id must be 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds"` (`storage/admin.rs:355-357`)
- EXC-003c: blank owner → `ERR_INVALID_ARGUMENT`, `"owner is required"` (`tools/admin.rs:174`)
- EXC-003d: id already taken → `ERR_PROJECT_EXISTS` (`storage/admin.rs:383`)
- EXC-003e: volume provisioning fails → the ACL row is rolled back and the provisioning error is returned (`tools/admin.rs:177-180`)

### SC-004: Owner manages project membership

**Actor:** Project owner (or platform admin)
**Preconditions:** the project exists; the caller owns it or is a platform admin.
**Flow:**
1. Caller invokes `admin.add_member` with `project_id` and `person`.
2. The handler gates on owner-or-admin (`tools/admin.rs:266`) and inserts the membership (`storage/admin.rs:166`).
3. Caller invokes `admin.list_members` to confirm; a plain member may also call this (`tools/admin.rs:283`).
4. Caller invokes `admin.remove_member` to revoke.

**Postconditions:** the added person passes `authorize` on that project; the removed person no longer does.
**Exceptions:**
- EXC-004a: caller neither owner nor admin → `ERR_FORBIDDEN` (`storage/admin.rs:305`)
- EXC-004b: project missing → `ERR_PROJECT_NOT_FOUND` (`storage/admin.rs:460`)
- EXC-004c: attempt to remove the owner → refused (`storage/admin.rs:452`)
- EXC-004d: platform admin acting on a missing project → `ERR_PROJECT_NOT_FOUND`, not success (`state.rs:51-54`)

### SC-005: Caller lists projects; admin lists all projects and all users

**Actor:** Member, or platform admin
**Preconditions:** server running; caller authenticated.
**Flow:**
1. Any caller invokes `admin.list_projects`, scoped to their own memberships (`tools/admin.rs:213-224`).
2. A platform admin invokes `admin.list_all_projects` (`tools/admin.rs:233-241`) and `admin.list_users` (`tools/admin.rs:259-262`).

**Postconditions:** the caller holds the project list they are entitled to; `list_projects` entries carry `is_owner`.
**Exceptions:**
- EXC-005a: non-admin calls `admin.list_all_projects` or `admin.list_users` → `ERR_FORBIDDEN` (`state.rs:42-44`)
- EXC-005b: caller with no memberships → an empty `projects` array, not an error

### SC-006: A non-member is refused, and a platform admin does not bypass the gate

**Actor:** Non-member
**Preconditions:** a project exists that the caller does not belong to.
**Flow:**
1. Caller invokes any tool taking `mount_id` for that project.
2. The handler calls `authorize` before any storage access (`state.rs:61-63`).
3. The call is refused.

**Postconditions:** no volume data is read or written; the caller learns only whether the project exists.
**Exceptions:**
- EXC-006a: project exists, caller not a member → `ERR_FORBIDDEN` (`storage/admin.rs:293-303`)
- EXC-006b: project does not exist → `ERR_PROJECT_NOT_FOUND`, distinct from forbidden (`storage/admin.rs:401`)
- EXC-006c: caller is a platform admin but not a member → still `ERR_FORBIDDEN` (`state.rs:61-63`)

### SC-007: Project index mode is set and read back

**Actor:** Owner or platform admin
**Preconditions:** the project exists; `search.enabled` is true for writes.
**Flow:**
1. Caller invokes `admin.set_index_mode` with a mode in `none|bm25|rag|both`.
2. The handler gates on owner-or-admin, parses the mode, and refuses a vector mode with no embedding endpoint (`tools/admin.rs:306-319`).
3. The new mode is stored **before** the index wipe (`tools/admin.rs:325`), then the indexer reacts.
4. Caller invokes `admin.get_index_mode` to read it back.

**Postconditions:** the mode is persisted on the project row and survives restart; `reindex_started` reports whether a background pass began.
**Exceptions:**
- EXC-007a: unknown mode string → `ERR_INVALID_ARGUMENT`, `"unknown index mode '{other}', expected none, bm25, rag, or both"` (`traits.rs:93-95`)
- EXC-007b: `search.enabled` false → `ERR_NOT_SUPPORTED`, `"search is not enabled; set search.enabled: true in config"` (`tools/admin.rs:310-312`)
- EXC-007c: vector mode with no embedding endpoint → `ERR_INVALID_ARGUMENT` (`tools/admin.rs:316-318`)
- EXC-007d: project missing → `ERR_PROJECT_NOT_FOUND` (`storage/admin.rs:499`)

**Cross-scenario notes:** the indexing behaviour behind `reindex_started` belongs to S7. This spec fixes only the gate, the persistence and the return shape.

### SC-008: Platform admin deletes a project, cascading members and volume

**Actor:** Platform admin or project owner
**Preconditions:** the project exists.
**Flow:**
1. Caller invokes `admin.delete_project`.
2. The handler gates on owner-or-admin (`tools/admin.rs:294`), tears down the volume (`:295`), purges git rows when git is enabled (`:297-303`), then deletes the ACL row (`:304`).
3. The tool returns `{"project_id","deleted":true}` (`tools/admin.rs:305`).

**Postconditions:** the project, its memberships and its volume are gone; a project recreated under the same id inherits nothing.
**Exceptions:**
- EXC-008a: caller neither owner nor admin → `ERR_FORBIDDEN`
- EXC-008b: project missing → `ERR_PROJECT_NOT_FOUND`
- EXC-008c: volume teardown fails → the error propagates and the ACL row survives, so the operation is retryable

### SC-009: The safety layer governs a write attempt

**Actor:** Engine-level (invoked by S2's tools; verified here at the `SafetyManager` boundary)
**Preconditions:** a `SafetyManager` built from `SafetyConfig`; a session keyed by `(person, project)`.
**Flow:**
1. A caller-supplied path is normalized (`safety.rs:89`).
2. When `read_guard` is on, a write to a path never read in this session is refused (`safety.rs:116`).
3. The byte count is charged against the session quota (`safety.rs:131`).
4. The operation is appended to the session audit log (`safety.rs:144`).
5. A delete derives a trash destination rather than destroying data (`safety.rs:168`).

**Postconditions:** the stored path is absolute and normalized; the session's `bytes_written` reflects the charge; the audit log holds the entry, oldest first.
**Exceptions:**
- EXC-009a: NUL byte in path → `ERR_PATH_OUT_OF_BOUNDS`, `"path contains a NUL byte"` (`safety.rs:90-92`)
- EXC-009b: path escaping the root → `ERR_PATH_OUT_OF_BOUNDS`, `"path escapes the volume root: {path}"` (`safety.rs:95-99`)
- EXC-009c: path beyond the backend ceiling → `ERR_INVALID_ARGUMENT` naming the limit (`safety.rs:69-74`)
- EXC-009d: write without a prior read → `ERR_EDIT_WITHOUT_PRIOR_READ` (`safety.rs:116`)
- EXC-009e: quota exceeded → `ERR_WRITE_QUOTA_EXCEEDED`, `"session write quota of {quota} bytes exceeded"` (`safety.rs:135-137`)

### SC-010: Content is stored once and garbage-collected at refcount zero

**Actor:** Engine-level (verified at the `VolumeClient` / `MetaBackend` boundary)
**Preconditions:** a volume with a metadata store and a blob store.
**Flow:**
1. Bytes are hashed; the blob is put under its sha256; the node row is written (`volume.rs:119-131`).
2. Writing identical bytes at a second path increments the refcount rather than storing twice (`meta.rs:277-294`).
3. Deleting one of the two paths decrements the refcount and keeps the blob (`meta.rs:296-333`).
4. Deleting the last referencing path returns the sha for GC, and the blob is deleted (`volume.rs:127-130`).

**Postconditions:** blob bytes exist exactly once per distinct content per volume; no orphan blob survives a delete.
**Exceptions:**
- EXC-010a: empty content → no blob is stored, `sha256` is `NULL`, size 0 (`volume.rs:120-121`)
- EXC-010b: overwriting a path with new content GCs the old blob and preserves `ctime` (`meta.rs:930`)
- EXC-010c: rewriting a path with identical content GCs nothing (`meta.rs:943`)
- EXC-010d: two volumes holding identical content are independent; neither delete affects the other (`meta.rs:1194`)

## 6. Functional Requirements

**Format:** EARS notation is mandatory. Forbidden modals (`should`, `may`, `could`, `might`, `would`) are not used. Only `MUST` / `SHALL` / `MUST NOT` / `SHALL NOT`.

| Tag | Pattern |
|-----|---------|
| `[EARS-U]` | Ubiquitous: The `<system>` SHALL `<response>` |
| `[EARS-E]` | Event-Driven: WHEN `<trigger>` THE `<system>` SHALL `<response>` |
| `[EARS-S]` | State-Driven: WHILE `<state>` THE `<system>` SHALL `<response>` |
| `[EARS-O]` | Optional/Conditional: IF `<condition>` THEN THE `<system>` SHALL `<response>` |
| `[EARS-UB]` | Unwanted Behaviour: The `<system>` SHALL NOT `<unwanted behaviour>` |
| `[EARS-X]` | Free-form escape |

All requirements are `FR-XXX`: this is a retro-specification of as-built behaviour, so nothing is a delta against a prior spec.

### Configuration and boot

#### FR-001 [EARS-E]: Config path resolution order
> WHEN the server resolves its configuration file path THE server SHALL select, in order: the explicit `--config`/`-c` path, then `$MCP_FS_CONFIG`, then a probe of `$XDG_CONFIG_HOME`, then the dir/name pair.

- **Inputs:** optional `--config PATH`; environment variables `MCP_FS_CONFIG`, `XDG_CONFIG_HOME`.
- **Outputs:** a single `PathBuf`, or the ordered list of paths tried.
- **Business Rules:** an explicit path is authoritative **even when the file does not exist**, so the resulting error names the path the operator asked for rather than silently falling through (`cli.rs:255-262`). A blank or whitespace-only environment value is ignored (`cli.rs:250-253`).
- **Priority:** Must-have

#### FR-002 [EARS-E]: Environment expansion and YAML parsing
> WHEN the server reads a configuration file THE server SHALL expand `${VAR}` and `${VAR:-default}` before parsing the YAML into `ServerConfig`.

- **Inputs:** the raw file contents; the process environment.
- **Outputs:** a populated `ServerConfig` (`config.rs:772-786`), or `ERR_INVALID_ARGUMENT`.
- **Business Rules:** expansion precedes parsing (`config.rs:802-806`). An unset variable with no default expands per `expand_env` (`config.rs:1006`). Secrets are supplied only through the environment; a `Dsn` redacts itself in `Debug` and `Display`.
- **Priority:** Must-have

#### FR-003 [EARS-O]: Boot validation of every store block
> IF any of `infra.meta`, `infra.admin`, `infra.git` or `infra.oauth` names a backend inconsistent with its DSN or with the compiled feature set THEN the server SHALL fail to boot with `ERR_INVALID_ARGUMENT` naming the offending section.

- **Inputs:** the four `infra.*` blocks, plus `doc_service` and `search`.
- **Outputs:** `Ok(())` or `ERR_INVALID_ARGUMENT`.
- **Business Rules:** three failures are rejected because each is otherwise a silent misconfiguration: a DSN that is never read, a backend that needs a DSN and has none, and a backend whose driver was not compiled in (`config.rs:874-915`). Validation covers all four sections (`config.rs:814-817`).
- **Priority:** Must-have

#### FR-004 [EARS-E]: CLI feature flags override the YAML
> WHEN `mcp-fs serve` receives any of `--git`, `--web`, `--context7`, `--sqlite`, `--db` or `--doc` THE server SHALL enable that tool family regardless of the YAML value.

- **Inputs:** the six boolean flags (`cli.rs:57-74`).
- **Outputs:** the `EnabledFeatures` used for registration (`app.rs:73-81`).
- **Business Rules:** the flags force-enable only; there is no flag that force-disables a family enabled in YAML.
- **Priority:** Must-have

#### FR-005 [EARS-U]: Unauthenticated liveness probe
> The server SHALL answer `GET /health` with `{"status":"ok","version":"<CARGO_PKG_VERSION>"}` and HTTP 200 without requiring a bearer token.

- **Inputs:** none.
- **Outputs:** JSON body with keys `status` and `version` (`app.rs:183-185`).
- **Business Rules:** the probe never touches a database, so it stays a liveness check rather than a readiness check.
- **Priority:** Must-have

#### FR-006 [EARS-O]: MCP path must be absolute
> IF `server.mcp_path` does not begin with `/` THEN the server SHALL abort startup with a message naming the offending value.

- **Inputs:** `server.mcp_path`, default from `d_mcp_path()` (`config.rs:141-150`).
- **Outputs:** a startup error (`app.rs:85-87`).
- **Business Rules:** the check runs during router assembly, before the listener binds.
- **Priority:** Must-have

### Identity

#### FR-007 [EARS-E]: Bearer extraction and header precedence
> WHEN a request arrives THE server SHALL look for a token in the configured header (default `X-Forwarded-Authorization`) and then in `Authorization`, accepting the `Bearer` scheme, the `Basic` scheme where the password is the token, and a bare token with no scheme.

- **Inputs:** the request headers; `auth.jwt.header` (`config.rs:152-154`).
- **Outputs:** an optional token string (`identity.rs:118-148`).
- **Business Rules:** the scheme prefix match is case-insensitive for both `Bearer` and `Basic` (`identity.rs:122-127`). `Basic` support exists for git CLI compatibility. A bare value is accepted only when it contains no space (`identity.rs:144-146`).
- **Priority:** Must-have

#### FR-008 [EARS-E]: Token verification
> WHEN a token is verified THE server SHALL validate its signature against the configured public key, its issuer when one is configured, and its `exp` and `nbf` claims with 30 seconds of leeway.

- **Inputs:** the token; `auth.jwt.public_key_path`, `issuer`, `audience`, `algorithms`.
- **Outputs:** the normalized identity, or `ERR_UNAUTHENTICATED`.
- **Business Rules:** leeway is exactly 30 seconds (`identity.rs:167`). Audience is validated only when configured; otherwise audience validation is disabled (`identity.rs:173-175`). Every configured algorithm is honoured, not just the first (`identity.rs:161-163`).
- **Priority:** Must-have

#### FR-009 [EARS-UB]: Symmetric algorithms are refused
> The server SHALL NOT accept an HMAC-family (`HS*`) signature algorithm for bearer verification.

- **Inputs:** `auth.jwt.algorithms` entries.
- **Outputs:** unsupported names are dropped at construction and reported by `unsupported_algorithms()` (`identity.rs:87-90`).
- **Business Rules:** accepted names are exactly `RS256`, `RS384`, `RS512`, `PS256`, `PS384`, `PS512`, `ES256`, `ES384` (`identity.rs:34-44`). An `HS*` algorithm with a public key file lets anyone holding that public key mint tokens.
- **Priority:** Must-have

#### FR-010 [EARS-E]: Identity claim extraction and normalization
> WHEN a token passes verification THE server SHALL read the identity from the claim named by `auth.jwt.username_claim` (default `email`) and normalize it caseless.

- **Inputs:** the decoded claims; `username_claim`.
- **Outputs:** the normalized identity string (`identity.rs:180-187`).
- **Business Rules:** a missing claim yields `"token has no '{claim}' claim"`; a blank claim yields `"token identity claim is empty"` (`identity.rs:181-186`). Normalization is `util::normalize_identity`, the same function `is_admin` uses (`config.rs:866-869`), so admin matching and membership matching agree.
- **Priority:** Must-have

#### FR-011 [EARS-O]: Unauthenticated request rejection
> IF identity resolution fails THEN the server SHALL answer HTTP 401 with the JSON body `{"error":"ERR_UNAUTHENTICATED","detail":"<message>"}` and SHALL NOT dispatch the request.

- **Inputs:** the failed resolution error.
- **Outputs:** a 401 with `Content-Type: application/json`, not SSE (`app.rs:271-280`).
- **Business Rules:** identity is resolved before body parsing and before dispatch (`app.rs:193-202`), so a bad bearer never reaches a tool.
- **Priority:** Must-have

### MCP protocol

#### FR-012 [EARS-E]: Initialize handshake
> WHEN the server receives `initialize` THE server SHALL answer with `protocolVersion` `2024-11-05`, capabilities `{"logging":{},"tools":{"listChanged":true}}` and `serverInfo` naming `mcp-fs` and the crate version.

- **Inputs:** a JSON-RPC request with method `initialize`.
- **Outputs:** the result object (`mcp/mod.rs:79-85`).
- **Business Rules:** `initialize` is accepted but not required: a bare `tools/call` is answered without a prior handshake, which is why the MCP layer is hand-rolled rather than `rmcp`-based (`mcp/mod.rs:3-8`).
- **Priority:** Must-have

#### FR-013 [EARS-E]: Tool catalogue
> WHEN the server receives `tools/list` THE server SHALL return the registry payload in registration order.

- **Inputs:** a JSON-RPC request with method `tools/list`.
- **Outputs:** the payload from `ToolRegistry::list_payload` (`app.rs:232`).
- **Business Rules:** registration order is `fs.*`, then `admin.*`, then the optional families (`tools/all.rs:4-6`). Each entry's `name`, `description` and `inputSchema` are frozen against `tool-contract-golden.json`.
- **Priority:** Must-have

#### FR-014 [EARS-E]: Successful tool call framing
> WHEN a tool call succeeds THE server SHALL answer `{"result":{"content":[{"type":"text","text":"<json>"}]},"id":<id>,"jsonrpc":"2.0"}` framed as one SSE event.

- **Inputs:** `tools/call` params with `name` and `arguments`.
- **Outputs:** the framed response; headers `Content-Type: text/event-stream` and `Cache-Control: no-cache,no-store`; body `event: message\ndata: {json}\n\n` (`mcp/mod.rs:10-13,41-44`).
- **Business Rules:** the JSON key order is `result`, `id`, `jsonrpc` and is part of the frozen contract (`mcp/mod.rs:46-52`). The tool's return value is serialized as JSON **text** inside a single text content block (`mcp/mod.rs:65-67`).
- **Priority:** Must-have

#### FR-015 [EARS-E]: Tool failure is a result, not a protocol error
> WHEN a tool handler returns an error THE server SHALL answer a successful JSON-RPC result carrying `"isError":true` and the text `An error occurred invoking '<tool>': <CODE>: <message>`.

- **Inputs:** the `ToolError` returned by the handler.
- **Outputs:** the result object (`mcp/mod.rs:71-77`); the failure is logged through `logging::log_tool_failure` (`app.rs:258`).
- **Business Rules:** only an unknown tool is a protocol-level error; every domain failure is a result (`app.rs:246-247`). Expected client-facing failures log at INFO without a backtrace; genuine 5xx failures log at ERROR (`logging.rs:9-13`).
- **Priority:** Must-have

#### FR-016 [EARS-O]: Unknown tool and unknown method
> IF a `tools/call` names a tool absent from the registry THEN the server SHALL answer JSON-RPC error `-32602` with message `Unknown tool: '<name>'`; IF a request names an unsupported method THEN the server SHALL answer `-32601` with message `Method '<other>' is not available.`

- **Inputs:** the tool name or method name.
- **Outputs:** the framed JSON-RPC error (`app.rs:236-240,251-253`).
- **Business Rules:** the two codes are distinct and are part of the frozen wire contract (`mcp/mod.rs:34-38`).
- **Priority:** Must-have

#### FR-017 [EARS-E]: Notifications are acknowledged without a body
> WHEN a request arrives with no `id`, or with a null `id`, THE server SHALL answer HTTP 202 with an empty body.

- **Inputs:** a JSON-RPC notification such as `notifications/initialized`.
- **Outputs:** HTTP 202, no body (`app.rs:223-227`).
- **Business Rules:** this covers every notification uniformly; the method name is not inspected.
- **Priority:** Must-have

#### FR-018 [EARS-O]: Malformed request body
> IF the request body is not valid JSON THEN the server SHALL answer HTTP 500 with `Content-Type: application/json` and the body `{"error":"ERR_INVALID_ARGUMENT","detail":"invalid JSON-RPC request: <e>"}`.

- **Inputs:** the raw request bytes.
- **Outputs:** the 500 response (`app.rs:204-220`).
- **Business Rules:** a malformed body is deliberately **not** a framed JSON-RPC parse error; this shape is frozen.
- **Priority:** Must-have

#### FR-019 [EARS-U]: Tolerant tool name resolution
> The registry SHALL resolve a requested tool name by exact match first and then by treating `.` and `_` as equivalent separators.

- **Inputs:** the requested name.
- **Outputs:** the registered tool, or none (`mcp/registry.rs:70-78`).
- **Business Rules:** the canonical names remain dotted; `admin_create_project` resolves to `admin.create_project`. Resolution never changes the name reported by `tools/list`.
- **Priority:** Must-have

### Errors

#### FR-020 [EARS-U]: The error vocabulary and its HTTP mapping
> The server SHALL expose exactly 14 stable error codes and SHALL map each to a fixed HTTP status.

- **Inputs:** a `ToolError`.
- **Outputs:** `code`, `message`, `http_status()`, `is_client_error()`, `retryable`.
- **Business Rules:** the codes are `ERR_UNAUTHENTICATED`, `ERR_FORBIDDEN`, `ERR_PROJECT_NOT_FOUND`, `ERR_PROJECT_EXISTS`, `ERR_PATH_OUT_OF_BOUNDS`, `ERR_EDIT_WITHOUT_PRIOR_READ`, `ERR_NO_CLOBBER`, `ERR_NOT_FOUND`, `ERR_AMBIGUOUS_MATCH`, `ERR_NO_MATCH`, `ERR_WRITE_QUOTA_EXCEEDED`, `ERR_INVALID_ARGUMENT`, `ERR_NOT_SUPPORTED`, `ERR_INTERNAL_ERROR` (`errors.rs:9-22`). The mapping is: 401, 403, 404, 409, 400, 428, 409, 404, 409, 422, 429, 400, 501, 500 respectively (`errors.rs:105-128`). `is_client_error()` is true exactly when the status is in 400..500 (`errors.rs:132-134`). A transient database failure carries `retryable` without a new code (`errors.rs:48`).
- **Priority:** Must-have

### Authorization

#### FR-021 [EARS-U]: Membership is the gate for volume access
> The server SHALL authorize every volume-scoped tool call by project membership alone.

- **Inputs:** `mount_id`, the caller identity.
- **Outputs:** `Ok(())` or an error (`state.rs:61-63`).
- **Business Rules:** `authorize` delegates to `AdminBackend::require_member` and performs no admin check. Authorization runs before path normalization and before any storage access, so a non-member receives `ERR_FORBIDDEN` rather than a path error.
- **Priority:** Must-have

#### FR-022 [EARS-UB]: Platform administration does not confer data access
> The server SHALL NOT grant a platform admin access to a project's volume on the basis of the admin role.

- **Inputs:** a platform admin identity that is not a member of the target project.
- **Outputs:** `ERR_FORBIDDEN` (`state.rs:61-63`).
- **Business Rules:** managing the platform and reading a project's data are separate authorities. This is the single most consequential invariant of the ACL design.
- **Priority:** Must-have

#### FR-023 [EARS-O]: Owner-or-admin gate requires the project to exist
> IF a platform admin invokes an owner-gated operation on a project that does not exist THEN the server SHALL answer `ERR_PROJECT_NOT_FOUND`.

- **Inputs:** `project_id`, the caller identity.
- **Outputs:** `Ok(())`, `ERR_PROJECT_NOT_FOUND`, or `ERR_FORBIDDEN` (`state.rs:49-60`).
- **Business Rules:** the admin shortcut skips the ownership check but not the existence check (`state.rs:51-54`). A non-admin falls through to `require_owner` (`state.rs:59`).
- **Priority:** Must-have

#### FR-024 [EARS-U]: Identity matching is caseless everywhere
> The server SHALL compare identities caselessly for platform-admin membership, project membership and ownership.

- **Inputs:** any identity string.
- **Outputs:** a normalized comparison result.
- **Business Rules:** `is_admin` normalizes both sides (`config.rs:866-869`); project membership is caseless (`storage/admin.rs:391`); the token claim is normalized at extraction (`identity.rs:187`). One function, `util::normalize_identity` (`util/mod.rs:10`), is used throughout, so the three comparisons cannot drift apart.
- **Priority:** Must-have

### Admin tools

#### FR-025 [EARS-E]: Project creation provisions a volume atomically
> WHEN `admin.create_project` succeeds THE server SHALL have created the ACL row, the owner's membership and the volume; and WHEN volume provisioning fails THE server SHALL delete the ACL row before returning the error.

- **Inputs:** `project_id: string`, `owner: string` (both required).
- **Outputs:** `{"project_id","owner","created_at"}` (`tools/admin.rs:181-185`).
- **Business Rules:** platform admin only (`tools/admin.rs:171`). The id is validated (`:172`) and a blank owner is refused with `"owner is required"` (`:173-175`). The rollback exists because a project whose volume does not exist fails every later `fs.*` call with a confusing storage error (`tools/admin.rs:177-180`). New projects default to `index_mode` `none` (`storage/admin.rs:476`).
- **Priority:** Must-have

#### FR-026 [EARS-U]: Project id format
> The server SHALL accept a project id of 3 to 32 characters composed only of lowercase ASCII letters, digits and hyphens, beginning and ending with a letter or digit.

- **Inputs:** the candidate id.
- **Outputs:** `Ok(())` or `ERR_INVALID_ARGUMENT` with the message `"project_id must be 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds"` (`storage/admin.rs:345-357`).
- **Business Rules:** the same rule governs the volume identifier, so an id that passes is safe as a database key and as a bucket-name component.
- **Priority:** Must-have

#### FR-027 [EARS-E]: Project deletion cascades
> WHEN `admin.delete_project` succeeds THE server SHALL have torn down the volume, purged the project's git rows when git is enabled, and deleted the ACL row and every membership.

- **Inputs:** `project_id: string`.
- **Outputs:** `{"project_id","deleted":true}` (`tools/admin.rs:305`).
- **Business Rules:** owner-or-admin gate (`tools/admin.rs:294`). The order is volume teardown, then git purge, then ACL delete (`:295-304`), so a failure leaves the ACL row and the operation stays retryable. The git purge exists because a project recreated under the same id otherwise inherits stale refs (`tools/admin.rs:297-299`).
- **Priority:** Must-have

#### FR-028 [EARS-E]: Membership mutation
> WHEN `admin.add_member` or `admin.remove_member` is invoked THE server SHALL require the caller to be the project owner or a platform admin, and SHALL refuse to remove the owner.

- **Inputs:** `project_id: string`, `person: string` (both required).
- **Outputs:** `{"project_id","person","role","added_by"}` for add (`tools/admin.rs:268-272`); `{"project_id","person","removed":true}` for remove (`:279`).
- **Business Rules:** adding to a missing project is `ERR_PROJECT_NOT_FOUND` (`storage/admin.rs:460`). Removing the owner is refused (`storage/admin.rs:452`).
- **Priority:** Must-have

#### FR-029 [EARS-E]: Listing tools and their gates
> WHEN a listing tool is invoked THE server SHALL apply its own gate: `admin.list_projects` requires only authentication, `admin.list_members` and `admin.get_index_mode` require membership or platform admin, and `admin.list_all_projects` and `admin.list_users` require platform admin.

- **Inputs:** none, or `project_id` for the member-scoped tools.
- **Outputs:** `{"projects":[{"project_id","owner","created_at","index_mode","is_owner"}]}`; `{"projects":[{"project_id","owner","created_at","index_mode"}]}`; `{"users":[{"person","is_admin"}]}`; `{"project_id","members":[{"person","role","added_by"}]}` (`TOOL_CONTRACT.txt:293-296`).
- **Business Rules:** `admin.list_projects` is scoped to the caller's own memberships and carries `is_owner`, which the all-projects listing does not. `admin.list_members` deliberately uses the membership gate, not owner-or-admin: a plain member is entitled to list (`tools/admin.rs:283`). `admin.list_users` returns a distinct, sorted person list (`storage/admin.rs:467`).
- **Priority:** Must-have

#### FR-030 [EARS-E]: Index mode is persisted before the index is touched
> WHEN `admin.set_index_mode` is invoked THE server SHALL store the new mode before wiping or rebuilding any index.

- **Inputs:** `project_id: string`, `mode: string` in `none|bm25|rag|both`.
- **Outputs:** `{"project_id","index_mode","previous_mode","reindex_started"}` (`tools/admin.rs:331-336`).
- **Business Rules:** the ordering is deliberate: a crash between the two leaves the mode recorded and the index stale, which one more call repairs; the reverse leaves a wiped index that nothing rebuilds (`tools/admin.rs:322-325`). `reindex_started: true` means a background pass is running, not that the volume is searchable (`tools/admin.rs:300-304`). A vector mode with no `search.embedding.endpoint` is refused at the gate (`tools/admin.rs:316-318`); `search.enabled` false yields `ERR_NOT_SUPPORTED` (`:310-312`).
- **Priority:** Must-have

#### FR-031 [EARS-U]: Index mode vocabulary
> The server SHALL accept exactly the index mode values `none`, `bm25`, `rag` and `both`, and SHALL reject any other value with `ERR_INVALID_ARGUMENT`.

- **Inputs:** the mode string.
- **Outputs:** an `IndexMode`, or `"unknown index mode '{other}', expected none, bm25, rag, or both"` (`traits.rs:88-96`).
- **Business Rules:** the stored form equals the wire form (`traits.rs:58-66`); the default is `none` (`traits.rs:47-49`).
- **Priority:** Must-have

### Safety

#### FR-032 [EARS-E]: Path normalization
> WHEN a caller-supplied path is normalized THE server SHALL reject a path containing a NUL byte, make the path absolute, collapse `.` and `..` segments, and reject any path that escapes the volume root.

- **Inputs:** the raw path string.
- **Outputs:** the normalized absolute path, or `ERR_PATH_OUT_OF_BOUNDS` (`safety.rs:89-106`).
- **Business Rules:** a relative path is made absolute by prefixing `/` (`safety.rs:93`); `/a/./b/../c.txt` normalizes to `/a/c.txt`; `/` normalizes to `/` (`safety.rs:186-189`). The NUL check precedes normalization (`safety.rs:90-92`). The collapsing itself is `PosixPath::normpath` (`util/posix.rs`), the single implementation in the tree.
- **Priority:** Must-have

#### FR-033 [EARS-O]: Backend path-length ceiling
> IF the active metadata backend imposes a path-length ceiling THEN the server SHALL reject a longer normalized path with `ERR_INVALID_ARGUMENT` naming the backend, the actual length and the limit.

- **Inputs:** the normalized path; the backend's ceiling (`meta.rs:80-100`).
- **Outputs:** `Ok(())` or the error (`safety.rs:59-75`).
- **Business Rules:** length is counted in characters, not bytes, because the ceiling comes from `NVARCHAR(n)` counting UTF-16 code units (`safety.rs:64-65`). The ceiling applies to SQL Server only; SQLite and PostgreSQL have none (`meta.rs:89-100`). The check runs on the normalized form, before any database call, so the caller gets a deterministic error rather than a driver failure (`safety.rs:101-104`). `parent` shares the column width but is always a prefix, so a path that fits guarantees a parent that fits.
- **Priority:** Must-have

#### FR-034 [EARS-S]: Read-before-write guard
> WHILE `safety.read_guard` is true THE server SHALL refuse a write to a path the session has not read, with `ERR_EDIT_WITHOUT_PRIOR_READ`.

- **Inputs:** `(person, project, path)`; the session's recorded reads.
- **Outputs:** `Ok(())` or the error (`safety.rs:116`).
- **Business Rules:** reads are recorded by `record_read` (`safety.rs:108`). Session state is in memory, keyed by `(person, project_id)` (`safety.rs:2-4,79-86`), so it is lost on restart. `read_guard` defaults to true (`config.rs:370`).
- **Priority:** Must-have

#### FR-035 [EARS-E]: Per-session write quota
> WHEN a byte charge that carries a session total past `safety.write_quota_bytes` is attempted THE server SHALL refuse it with `ERR_WRITE_QUOTA_EXCEEDED` and leave the session counter unchanged.

- **Inputs:** `(person, project, num_bytes)`.
- **Outputs:** `Ok(())` or `"session write quota of {quota} bytes exceeded"` (`safety.rs:131-140`).
- **Business Rules:** the quota is per session, not per project or per process, and the counter is readable through `bytes_written` (`safety.rs:163`). The charge is refused atomically: a rejected charge does not advance the counter.
- **Priority:** Must-have

#### FR-036 [EARS-U]: Capped session audit log
> The server SHALL retain at most 500 audit entries per session, discarding the oldest first, and SHALL return them oldest first.

- **Inputs:** `(person, project, op, path, detail)`.
- **Outputs:** the ordered entry list (`safety.rs:159-161`).
- **Business Rules:** `AUDIT_CAP` is 500 (`safety.rs:13`); the ring drops from the front (`safety.rs:152-154`). Entries are per session, so they do not survive a restart.
- **Priority:** Must-have

#### FR-037 [EARS-U]: Trash path derivation
> The server SHALL derive a soft-delete destination of the form `/{trash_dir}/{epoch_ms}__{flattened_path}`, where the flattened path is the original with leading and trailing `/` stripped and every remaining `/` replaced by `__`.

- **Inputs:** the normalized source path; `safety.trash_dir`.
- **Outputs:** the trash path (`safety.rs:168-172`).
- **Business Rules:** the millisecond stamp makes repeated deletes of the same path distinct. `allow_hard_delete` defaults to false (`config.rs:371`), so soft delete is the default behaviour.
- **Priority:** Must-have

### Storage

#### FR-038 [EARS-E]: Content-addressed storage with dedup
> WHEN non-empty content is written THE server SHALL store the bytes once per volume under their sha256 and SHALL increment the reference count when the same content is written at another path.

- **Inputs:** the path and the byte content.
- **Outputs:** a blob under its sha256 plus a node row carrying that sha (`volume.rs:119-131`, `meta.rs:277-294`).
- **Business Rules:** the sha is computed before the put (`volume.rs:123`). `blob_refs` is keyed by `(volume_id, sha256)` (`meta.rs:862-863`). Copy is a metadata-only operation. The local backend's on-disk layout is `{dir}/{bucket}/{sha[..2]}/{sha}`, and the two-character shard directory is part of the on-disk contract rather than an implementation detail (`storage/blob/local.rs:4-6,24,33`).
- **Priority:** Must-have

#### FR-039 [EARS-E]: Garbage collection at refcount zero
> WHEN the last node referencing a blob is deleted or overwritten THE server SHALL delete the blob bytes; and WHILE any other node still references it THE server SHALL retain them.

- **Inputs:** a delete or an overwrite.
- **Outputs:** the sha to collect, or none (`meta.rs:296-333`); the blob deletion (`volume.rs:127-130`).
- **Business Rules:** an overwrite GCs the old blob and preserves `ctime` (`meta.rs:930`); rewriting a path with identical content GCs nothing (`meta.rs:943`).
- **Priority:** Must-have

#### FR-040 [EARS-O]: Empty files store no blob
> IF the content written is empty THEN the server SHALL store a node with `sha256` NULL and size 0 and SHALL NOT put any blob.

- **Inputs:** an empty byte slice.
- **Outputs:** the node row only (`volume.rs:120-121`).
- **Business Rules:** `create_empty` is defined as writing empty bytes (`volume.rs:138-140`), so the two paths cannot diverge.
- **Priority:** Must-have

#### FR-041 [EARS-U]: Volume scoping of every row
> The server SHALL include `volume_id` in the key and in every predicate of every `nodes` and `blob_refs` access.

- **Inputs:** any metadata operation.
- **Outputs:** rows belonging to exactly one volume.
- **Business Rules:** one relational database holds every volume, so omitting `volume_id` leaks or corrupts another volume's rows. `blob_refs` is keyed `(volume_id, sha256)` (`meta.rs:862-863`); refcounts do not leak across volumes (`meta.rs:1194`).
- **Priority:** Must-have

#### FR-042 [EARS-U]: LIKE patterns are escaped
> The server SHALL build every subtree `LIKE` pattern through `descendant_pattern`, escaping `_` and `%` as literals.

- **Inputs:** a directory path.
- **Outputs:** an escaped pattern (`meta.rs:231-235`).
- **Business Rules:** an unescaped prefix made `a_b` match `axb`, so a subtree delete removed unrelated rows. The escaping is verified by `meta.rs:871`.
- **Priority:** Must-have

#### FR-044 [EARS-E]: Key generation and token minting
> WHEN `mcp-fs keys` is run THE server SHALL write an RSA keypair as `jwt.key` and `jwt.pub` in the target directory, and WHEN `mcp-fs token <email>` is run THE server SHALL print a signed token on stdout.

- **Inputs:** `--dir` defaulting to `.keys`; for the token, the email, `--key`, `--issuer`, `--claim` and `--ttl`.
- **Outputs:** the two key files, or the token on stdout (`keys.rs:22-32,45,62,80`).
- **Business Rules:** the file names are `jwt.key` for the private key and `jwt.pub` for the public key (`keys.rs:24-26`); the directory defaults to `.keys` (`keys.rs:22`) and is created when missing. The token defaults are issuer `web-a2a`, claim `email` and a TTL of 3600 seconds (`keys.rs:28-32`), which are exactly the defaults `auth.jwt` expects (`config.rs:158-163`) and are documented as matching them at the constants themselves (`keys.rs:27-31`), so a freshly generated keypair and a freshly minted token work against a default config with no further alignment. The token is printed on stdout while logs go to stderr, so it can be piped straight into a file (`logging.rs:26-28`).
- **Priority:** Must-have

#### FR-043 [EARS-U]: Stores speak the relational seam, never a driver
> Every store SHALL access its data through `storage::rel::RelationalDb` and `RelationalTx` rather than through any driver type.

- **Inputs:** any store operation.
- **Outputs:** engine-independent SQL rendered per `Dialect`.
- **Business Rules:** a transaction is an owned handle from `begin()`, not a closure. `run_retrying` re-runs its closure, so calling it asserts the work is idempotent; `put_file` documents exactly why it qualifies (`meta.rs:480-484`). Work touching state outside the transaction uses `begin()` instead.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- `GET /health` performs no database access and SHALL answer without I/O beyond the socket (`app.rs:183-185`).
- No request thread blocks on the database: the relational seam is async throughout (`storage/rel/mod.rs:1-7`).
- Blob reads support ranged access so a large file is not materialized whole when a range suffices (`volume.rs:66`).

### 7.2 Security
- Bearer verification is mandatory for every MCP request; only `/health` is unauthenticated (`app.rs:114-115,193-202`).
- Only asymmetric signature algorithms are accepted (FR-009).
- Secrets come from the environment only; a `Dsn` redacts itself in `Debug` and `Display`. No token or key is ever logged.
- Platform administration confers no data access (FR-022).
- Path normalization precedes every storage access, and authorization precedes normalization (FR-021, FR-032).

### 7.3 Usability
- Error messages name the offending value and, where a remedy exists, state it: the path-length error suggests shortening a directory name and shows an example (`safety.rs:70-74`); the `ERR_NOT_SUPPORTED` for search names the config key to set (`tools/admin.rs:311`).
- The CLI prints tokens on stdout and logs on stderr, so `mcp-fs token` can be piped straight into a file (`logging.rs:26-28`).

### 7.4 Reliability
- Project creation is atomic with respect to the ACL: a failed volume provisioning rolls the ACL row back (FR-025).
- Index mode is persisted before the index is mutated, so the recoverable failure mode is the one that a retry repairs (FR-030).
- Transient database failures are marked `retryable` without inventing a new error code (`errors.rs:48`).
- The server shuts down gracefully (`app.rs:148`).

### 7.5 Observability

**As-built, and deliberately modest.** The stack is `tracing` + `tracing-subscriber` only.

- **Collector:** none. A `fmt` subscriber writes to **stderr** (`logging.rs:29-37`).
- **Filter:** the `RUST_LOG` environment variable, defaulting to `info` (`logging.rs:20-22`).
- **Tool failures:** classified by `is_expected`; client-facing 4xx failures log one concise INFO line with no backtrace, genuine 5xx failures log at ERROR (`logging.rs:9-13`, `app.rs:258`).
- **Unauthenticated requests:** logged with the MCP path and the reason (`app.rs:199`).
- **Never logged:** tokens, keys, DSN contents.
- **OpenTelemetry is NOT implemented.** There is no `opentelemetry`, `opentelemetry-otlp` or `tracing-appender` dependency anywhere in the workspace; the only telemetry dependency is `tracing-subscriber` (`Cargo.toml:98`, `crates/mcp-fs/Cargo.toml:79`). There is no OTLP exporter, no span export, and no rolling file log. Adding one is deferred: see `specs/BACKLOG.md` BL-001.

### 7.6 Deployment
- **Project context:** personal.
- **Compute:** a single self-contained binary; `docker-compose.test.yml` exists only to supply PostgreSQL and SQL Server to the opt-in test suites.
- **Data stores:** SQLite by default for relational state, local filesystem for blobs. Other engines are S4.
- **Secret management:** environment variables, expanded into the YAML at load time (FR-002).
- **Observability collector:** stderr (see 7.5).
- **CI/CD:** the quality gate is `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check`, all of which MUST be clean before any commit.

### 7.7 Scalability
- One relational database holds every volume, distinguished by `volume_id` (FR-041), so adding a project adds rows rather than a database, except under the SQLite default where each volume is its own file (`config.rs:826-828`).
- Session state is in memory and per process, so the write quota and read guard are **not** shared across replicas. Horizontal scaling changes their semantics; that limitation is recorded here rather than solved.

## 8. Data Model

| Entity | Key | Fields | Home |
|---|---|---|---|
| **Project** | `id` | `id`, `owner`, `created_at`, `index_mode` | `project` table (`storage/admin.rs:27-40`), types `traits.rs:100-108` |
| **Member** | `(project_id, person)` | `project_id`, `person`, `role` (`owner`/`member`), `added_by`, `added_at` | `project_member` table (`storage/admin.rs:41-50`), types `traits.rs:110-119` |
| **NodeRow** | `(volume_id, path)` | `path`, `parent`, `name`, `kind` (`dir`/`file`), `size`, `mode`, `mtime`, `ctime`, `atime`, `sha256` | `nodes` table (`meta.rs:117-138`), type `traits.rs:12-25` |
| **BlobRef** | `(volume_id, sha256)` | `volume_id`, `sha256`, `refcount`, `size` | `blob_refs` table (`meta.rs:139-145`) |
| **Blob** | `sha256` | raw bytes | blob backend; local layout `{dir}/{bucket}/{sha[..2]}/{sha}` (`storage/blob/local.rs:4,24,33`) |
| **SessionState** | `(person, project)` | recorded reads, `bytes_written`, `audit` ring | in memory (`safety.rs:28`) |

Default POSIX modes are `MODE_DIR = 0o040755` and `MODE_FILE = 0o100644` (`traits.rs:36-38`).

## 9. Impact Analysis

This specification documents shipped behaviour; it requires no code change. The tables below record what it covers and what the E2E suite adds.

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/config.rs` | Specified, unchanged | FR-001..FR-004, FR-006 |
| `crates/mcp-fs/src/errors.rs` | Specified, unchanged | FR-020 |
| `crates/mcp-fs/src/identity.rs` | Specified, unchanged | FR-007..FR-011 |
| `crates/mcp-fs/src/safety.rs` | Specified, unchanged | FR-032..FR-037 |
| `crates/mcp-fs/src/state.rs` | Specified, unchanged | FR-021..FR-024 |
| `crates/mcp-fs/src/app.rs` | Specified, unchanged | FR-005, FR-011..FR-018 |
| `crates/mcp-fs/src/mcp/` | Specified, unchanged | FR-012..FR-019 |
| `crates/mcp-fs/src/storage/` | Specified, unchanged | FR-038..FR-043 |
| `crates/mcp-fs/src/tools/admin.rs` | Specified, unchanged | FR-025..FR-031 |
| `tests/functional/scenarios/` | Extended | New scenario scripts for the gaps in §12 |

### 9.2 Affected Requirements

No prior specification exists, so no requirement is superseded. `TOOL_CONTRACT.txt` and `tool-contract-golden.json` remain the authoritative schema artifacts; this document references them and does not restate the schemas tool by tool.

### 9.3 Affected Tests

| Test File | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/config.rs` (99 test fns) | config parsing, expansion, resolution, validation | Keep; referenced by §12 |
| `crates/mcp-fs/src/identity.rs` (17) | verification, header precedence, algorithms | Keep |
| `crates/mcp-fs/src/safety.rs` (20) | normalization, ceiling, guard | Keep |
| `crates/mcp-fs/src/storage/meta.rs` (36) | content addressing, GC, scoping, patterns | Keep |
| `crates/mcp-fs/src/storage/admin.rs` (17) | ACL, caseless membership, index mode | Keep |
| `crates/mcp-fs/src/tools/admin.rs` (37) | the 10 tools and their gates | Keep |
| `crates/mcp-fs/src/storage/conformance.rs` | one suite per engine | Keep; S4 extends |
| `tests/functional/scenarios/01_admin_lifecycle.sh` | admin lifecycle over HTTP | Extend |
| `tests/functional/scenarios/10_security.sh` | authorization refusals over HTTP | Extend |

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | Documentation index | Add a pointer to `specs/` |
| `.agent_docs/architecture.md` | Storage model, request lifecycle | Cross-reference this spec |
| `README.md` | — | No change required |

### 9.5 Dependencies & Risks

No new dependency, no breaking change, no migration. The single risk is **specification drift**: this document asserts behaviour at specific line numbers, and line numbers move. The mitigation is that every assertion is also a test in §12, so drift surfaces as a red test rather than as a stale sentence.

## 10. Documentation Requirements

### 10.1 README.md
Unchanged by this spec. It already documents purpose, prerequisites, build/run/test and configuration.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: add `specs/` to the documentation index, naming this file as the foundation contract.
- `.agent_docs/architecture.md`: cross-reference §5 and §6 rather than restating them.
- This project uses `AGENTS.md`, not `CLAUDE.md`; the template's naming is adapted accordingly.

### 10.3 docs/*
No `docs/` tree exists and none is required. Operational guidance lives in `.agent_docs/`.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-001 | FR-001, FR-002, FR-003, FR-004, FR-005, FR-006, FR-044 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-115 | E2E-005, E2E-006, E2E-007, E2E-008, E2E-009, E2E-010, E2E-011, E2E-116 | E2E-012, E2E-013, E2E-014, E2E-117 |
| SC-002 | FR-007, FR-008, FR-009, FR-010, FR-011, FR-012, FR-013 | E2E-015, E2E-016, E2E-017 | E2E-018, E2E-019, E2E-020, E2E-021, E2E-022, E2E-023, E2E-024 | E2E-025, E2E-026, E2E-027, E2E-028 |
| SC-002 (protocol) | FR-014, FR-015, FR-016, FR-017, FR-018, FR-019 | E2E-029, E2E-030, E2E-031 | E2E-032, E2E-033, E2E-034, E2E-035, E2E-036 | E2E-037, E2E-038, E2E-039, E2E-040, E2E-041, E2E-042 |
| SC-003 | FR-020, FR-025, FR-026 | E2E-043, E2E-044 | E2E-045, E2E-046, E2E-047, E2E-048, E2E-049, E2E-050 | E2E-051, E2E-052, E2E-053 |
| SC-004 | FR-028 | E2E-054, E2E-055 | E2E-056, E2E-057, E2E-058 | E2E-059, E2E-060 |
| SC-005 | FR-029 | E2E-061, E2E-062, E2E-063 | E2E-064, E2E-065 | E2E-066, E2E-067 |
| SC-006 | FR-021, FR-022, FR-023, FR-024 | E2E-068 | E2E-069, E2E-070, E2E-071, E2E-072, E2E-073 | E2E-074, E2E-075, E2E-076 |
| SC-007 | FR-030, FR-031 | E2E-077, E2E-078 | E2E-079, E2E-080, E2E-081, E2E-082 | E2E-083, E2E-084 |
| SC-008 | FR-027 | E2E-085 | E2E-086, E2E-087, E2E-088 | E2E-089 |
| SC-009 | FR-032, FR-033, FR-034, FR-035, FR-036, FR-037 | E2E-090, E2E-091, E2E-092 | E2E-093, E2E-094, E2E-095, E2E-096, E2E-097, E2E-098, E2E-099 | E2E-100, E2E-101, E2E-102, E2E-103 |
| SC-010 | FR-038, FR-039, FR-040, FR-041, FR-042, FR-043 | E2E-104, E2E-105 | E2E-106, E2E-107, E2E-108 | E2E-109, E2E-110, E2E-111, E2E-112, E2E-113, E2E-114 |

Per-FR coverage (every FR carries at least three tests):

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-001 | E2E-001, E2E-005, E2E-012, E2E-013 | FR-023 | E2E-071, E2E-074, E2E-088 |
| FR-002 | E2E-002, E2E-006, E2E-014 | FR-024 | E2E-072, E2E-075, E2E-076 |
| FR-003 | E2E-003, E2E-007, E2E-008, E2E-009 | FR-025 | E2E-043, E2E-045, E2E-046, E2E-050, E2E-051 |
| FR-004 | E2E-004, E2E-010, E2E-053 | FR-026 | E2E-044, E2E-047, E2E-048, E2E-052 |
| FR-005 | E2E-001, E2E-011, E2E-026 | FR-027 | E2E-085, E2E-086, E2E-087, E2E-089 |
| FR-006 | E2E-011, E2E-012, E2E-013 | FR-028 | E2E-054, E2E-055, E2E-056, E2E-057, E2E-058, E2E-059, E2E-060 |
| FR-007 | E2E-015, E2E-018, E2E-025, E2E-027 | FR-029 | E2E-061..E2E-067 |
| FR-008 | E2E-016, E2E-019, E2E-020, E2E-028 | FR-030 | E2E-077, E2E-079, E2E-080, E2E-081, E2E-083 |
| FR-009 | E2E-021, E2E-026, E2E-028 | FR-031 | E2E-078, E2E-082, E2E-084 |
| FR-010 | E2E-017, E2E-022, E2E-023 | FR-032 | E2E-090, E2E-093, E2E-094, E2E-100, E2E-101 |
| FR-011 | E2E-018, E2E-023, E2E-024 | FR-033 | E2E-095, E2E-102, E2E-103 |
| FR-012 | E2E-015, E2E-029, E2E-037 | FR-034 | E2E-091, E2E-096, E2E-097 |
| FR-013 | E2E-017, E2E-030, E2E-038 | FR-035 | E2E-092, E2E-098, E2E-099 |
| FR-014 | E2E-029, E2E-031, E2E-039 | FR-036 | E2E-092, E2E-099, E2E-100 |
| FR-015 | E2E-032, E2E-033, E2E-040 | FR-037 | E2E-090, E2E-098, E2E-103 |
| FR-016 | E2E-034, E2E-035, E2E-041 | FR-038 | E2E-104, E2E-106, E2E-109, E2E-113 |
| FR-017 | E2E-030, E2E-036, E2E-042 | FR-039 | E2E-105, E2E-107, E2E-110, E2E-111 |
| FR-018 | E2E-033, E2E-035, E2E-036 | FR-040 | E2E-108, E2E-109, E2E-112 |
| FR-019 | E2E-031, E2E-038, E2E-042 | FR-041 | E2E-106, E2E-113, E2E-114 |
| FR-020 | E2E-043, E2E-049, E2E-069, E2E-094 | FR-042 | E2E-107, E2E-111, E2E-114 |
| FR-021 | E2E-068, E2E-069, E2E-070 | FR-043 | E2E-104, E2E-110, E2E-112 |
| FR-022 | E2E-070, E2E-073, E2E-076 | FR-044 | E2E-115, E2E-116, E2E-117 |

## 12. End-to-End Test Suite

> E2E tests are the contract. The implementation is correct when all of them pass.

**Placement.** Protocol-level and tool-level tests (E2E-001 through E2E-089) are shell scenarios under `tests/functional/scenarios/`, driving a running server over HTTP, following the existing convention of `01_admin_lifecycle.sh` and `10_security.sh`. Engine-level tests (E2E-090 through E2E-114) are Rust tests against `SafetyManager`, `VolumeClient` and `MetaBackend`, following the convention of `safety.rs:186-290` and `meta.rs:902-943`. Where a listed test already exists at the cited location, the action is **Existing** and the requirement is to keep it green and named; the rest are **New**.

**Fixtures, used by every tool-level test unless stated otherwise:**
- `ADMIN` = `admin@example.com`, listed in `auth.admins`.
- `ALICE` = `alice@example.com`, `BOB` = `bob@example.com`, neither an admin.
- `PROJ` = `spec-foundation`, a valid project id.
- Tokens minted with `mcp-fs token <email>`, issuer `web-a2a`, claim `email`.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | Core Journey | SC-001 | FR-001, FR-005 | Critical |
| E2E-002 | Existing | Feature | SC-001 | FR-002 | Critical |
| E2E-003 | Existing | Feature | SC-001 | FR-003 | Critical |
| E2E-004 | New | Feature | SC-001 | FR-004 | High |
| E2E-005 | Existing | Error | SC-001 | FR-001 | High |
| E2E-006 | Existing | Error | SC-001 | FR-002 | High |
| E2E-007 | Existing | Error | SC-001 | FR-003 | Critical |
| E2E-008 | Existing | Error | SC-001 | FR-003 | Critical |
| E2E-009 | Existing | Error | SC-001 | FR-003 | Critical |
| E2E-010 | New | Error | SC-001 | FR-004 | Medium |
| E2E-011 | New | Error | SC-001 | FR-005, FR-006 | High |
| E2E-012 | Existing | Edge | SC-001 | FR-001, FR-006 | High |
| E2E-013 | Existing | Edge | SC-001 | FR-001, FR-006 | Medium |
| E2E-014 | Existing | Edge | SC-001 | FR-002 | Medium |
| E2E-015 | New | Core Journey | SC-002 | FR-007, FR-012 | Critical |
| E2E-016 | Existing | Feature | SC-002 | FR-008 | Critical |
| E2E-017 | New | Feature | SC-002 | FR-010, FR-013 | Critical |
| E2E-018 | New | Security | SC-002 | FR-007, FR-011 | Critical |
| E2E-019 | Existing | Security | SC-002 | FR-008 | Critical |
| E2E-020 | Existing | Security | SC-002 | FR-008 | Critical |
| E2E-021 | Existing | Error | SC-002 | FR-009 | High |
| E2E-022 | Existing | Error | SC-002 | FR-010 | High |
| E2E-023 | New | Error | SC-002 | FR-010, FR-011 | High |
| E2E-024 | New | Security | SC-002 | FR-011 | Critical |
| E2E-025 | Existing | Edge | SC-002 | FR-007 | Medium |
| E2E-026 | New | Edge | SC-002 | FR-005, FR-009 | Medium |
| E2E-027 | Existing | Edge | SC-002 | FR-007 | Medium |
| E2E-028 | Existing | Edge | SC-002 | FR-008, FR-009 | High |
| E2E-029 | Existing | Feature | SC-002 | FR-012, FR-014 | Critical |
| E2E-030 | New | Feature | SC-002 | FR-013, FR-017 | Critical |
| E2E-031 | New | Feature | SC-002 | FR-014, FR-019 | High |
| E2E-032 | New | Error | SC-002 | FR-015 | Critical |
| E2E-033 | Existing | Error | SC-002 | FR-015, FR-018 | High |
| E2E-034 | Existing | Error | SC-002 | FR-016 | Critical |
| E2E-035 | New | Error | SC-002 | FR-016, FR-018 | High |
| E2E-036 | New | Error | SC-002 | FR-017, FR-018 | High |
| E2E-037 | Existing | Edge | SC-002 | FR-012 | Medium |
| E2E-038 | Existing | Edge | SC-002 | FR-013, FR-019 | High |
| E2E-039 | Existing | Edge | SC-002 | FR-014 | High |
| E2E-040 | New | Edge | SC-002 | FR-015 | Medium |
| E2E-041 | New | Edge | SC-002 | FR-016 | Medium |
| E2E-042 | New | Edge | SC-002 | FR-017, FR-019 | Medium |
| E2E-043 | Existing | Core Journey | SC-003 | FR-020, FR-025 | Critical |
| E2E-044 | Existing | Feature | SC-003 | FR-026 | High |
| E2E-045 | New | Side Effect | SC-003 | FR-025 | Critical |
| E2E-046 | New | Data Integrity | SC-003 | FR-025 | Critical |
| E2E-047 | Existing | Error | SC-003 | FR-026 | High |
| E2E-048 | Existing | Error | SC-003 | FR-026 | High |
| E2E-049 | Existing | Error | SC-003 | FR-020 | High |
| E2E-050 | Existing | Security | SC-003 | FR-025 | Critical |
| E2E-051 | New | Edge | SC-003 | FR-025 | Medium |
| E2E-052 | New | Edge | SC-003 | FR-026 | Medium |
| E2E-053 | New | Edge | SC-003 | FR-004 | Low |
| E2E-054 | Existing | Core Journey | SC-004 | FR-028 | Critical |
| E2E-055 | New | Side Effect | SC-004 | FR-028 | Critical |
| E2E-056 | Existing | Error | SC-004 | FR-028 | High |
| E2E-057 | Existing | Error | SC-004 | FR-028 | High |
| E2E-058 | Existing | Error | SC-004 | FR-028 | High |
| E2E-059 | New | Edge | SC-004 | FR-028 | Medium |
| E2E-060 | New | Edge | SC-004 | FR-028 | Medium |
| E2E-061 | Existing | Feature | SC-005 | FR-029 | High |
| E2E-062 | Existing | Feature | SC-005 | FR-029 | High |
| E2E-063 | Existing | Feature | SC-005 | FR-029 | High |
| E2E-064 | Existing | Security | SC-005 | FR-029 | Critical |
| E2E-065 | Existing | Security | SC-005 | FR-029 | Critical |
| E2E-066 | New | Edge | SC-005 | FR-029 | Medium |
| E2E-067 | Existing | Edge | SC-005 | FR-029 | Medium |
| E2E-068 | New | Core Journey | SC-006 | FR-021 | Critical |
| E2E-069 | Existing | Security | SC-006 | FR-020, FR-021 | Critical |
| E2E-070 | Existing | Security | SC-006 | FR-021, FR-022 | Critical |
| E2E-071 | Existing | Error | SC-006 | FR-023 | High |
| E2E-072 | Existing | Error | SC-006 | FR-024 | High |
| E2E-073 | New | Security | SC-006 | FR-022 | Critical |
| E2E-074 | Existing | Edge | SC-006 | FR-023 | High |
| E2E-075 | Existing | Edge | SC-006 | FR-024 | High |
| E2E-076 | New | Edge | SC-006 | FR-022, FR-024 | High |
| E2E-077 | Existing | Feature | SC-007 | FR-030 | High |
| E2E-078 | Existing | Feature | SC-007 | FR-031 | High |
| E2E-079 | New | Side Effect | SC-007 | FR-030 | High |
| E2E-080 | Existing | Error | SC-007 | FR-030 | High |
| E2E-081 | Existing | Error | SC-007 | FR-030 | High |
| E2E-082 | Existing | Error | SC-007 | FR-031 | High |
| E2E-083 | Existing | Edge | SC-007 | FR-030 | Medium |
| E2E-084 | Existing | Edge | SC-007 | FR-031 | Medium |
| E2E-085 | Existing | Core Journey | SC-008 | FR-027 | Critical |
| E2E-086 | Existing | Error | SC-008 | FR-027 | High |
| E2E-087 | New | Side Effect | SC-008 | FR-027 | Critical |
| E2E-088 | New | Error | SC-008 | FR-023, FR-027 | High |
| E2E-089 | New | Edge | SC-008 | FR-027 | High |
| E2E-090 | Existing | Feature | SC-009 | FR-032, FR-037 | Critical |
| E2E-091 | Existing | Feature | SC-009 | FR-034 | Critical |
| E2E-092 | New | Feature | SC-009 | FR-035, FR-036 | High |
| E2E-093 | Existing | Error | SC-009 | FR-032 | Critical |
| E2E-094 | Existing | Error | SC-009 | FR-020, FR-032 | Critical |
| E2E-095 | Existing | Error | SC-009 | FR-033 | High |
| E2E-096 | Existing | Error | SC-009 | FR-034 | Critical |
| E2E-097 | New | Data Integrity | SC-009 | FR-034 | High |
| E2E-098 | New | Error | SC-009 | FR-035, FR-037 | Medium |
| E2E-099 | New | Edge | SC-009 | FR-035, FR-036 | High |
| E2E-100 | Existing | Edge | SC-009 | FR-032, FR-036 | Medium |
| E2E-101 | Existing | Edge | SC-009 | FR-032 | High |
| E2E-102 | Existing | Edge | SC-009 | FR-033 | High |
| E2E-103 | New | Edge | SC-009 | FR-033, FR-037 | Medium |
| E2E-104 | Existing | Core Journey | SC-010 | FR-038, FR-043 | Critical |
| E2E-105 | Existing | Feature | SC-010 | FR-039 | Critical |
| E2E-106 | Existing | Data Integrity | SC-010 | FR-038, FR-041 | Critical |
| E2E-107 | Existing | Error | SC-010 | FR-039, FR-042 | Critical |
| E2E-108 | Existing | Error | SC-010 | FR-040 | High |
| E2E-109 | Existing | Edge | SC-010 | FR-038, FR-040 | High |
| E2E-110 | Existing | Edge | SC-010 | FR-039, FR-043 | High |
| E2E-111 | Existing | Edge | SC-010 | FR-039, FR-042 | High |
| E2E-112 | New | Edge | SC-010 | FR-040, FR-043 | Medium |
| E2E-113 | Existing | Edge | SC-010 | FR-038, FR-041 | Critical |
| E2E-114 | New | Edge | SC-010 | FR-041, FR-042 | High |
| E2E-115 | New | Feature | SC-001 | FR-044 | Critical |
| E2E-116 | New | Error | SC-001 | FR-044 | High |
| E2E-117 | New | Edge | SC-001 | FR-044 | High |

**Coverage Statistics** (117 tests):
- Happy path (Core Journey + Feature): 32
- Failure/error (Error + Security): 48
- Side effects: 4
- Edge cases: 29
- Data integrity: 4
- State transitions: 0 (no state machine in this layer)
- Performance: 0 (see §7.1; no performance test is specified at this layer)
- Happy:Failure ratio: 1:1.5 (48 failure vs 32 happy, failure-dominant as required)

### 12.2 New Test Specifications

Only tests whose **Action is New** are specified in full below. Tests marked **Existing** are already implemented at the locations given in §9.3 and their contract is the assertion they already make; they are listed in §12.1 and in the matrix so that no requirement is untraced, and §14 requires that each be renamed to carry its `E2E-XXX` id in a comment.

#### E2E-001: Server boots from an explicit config and answers /health
- **Category:** Core Journey
- **Scenario:** SC-001
- **Requirements:** FR-001, FR-005
- **Preconditions:** a config file at `/tmp/spec/foundation.yaml` with `server.host: 127.0.0.1`, `server.port: 0`, `auth.jwt.public_key_path` pointing at a generated `jwt.pub`; no `MCP_FS_CONFIG` set.
- **Steps:**
  - Given the binary built and the config written
  - When `mcp-fs serve -c /tmp/spec/foundation.yaml` is started and `GET /health` is issued
  - Then the response status is 200 and the body equals `{"status":"ok","version":"<crate version>"}` with exactly those two keys
  - And no `Authorization` header was sent, proving the probe is unauthenticated
- **Cleanup:** stop the server, remove `/tmp/spec`
- **Priority:** Critical

#### E2E-004: A CLI feature flag enables a family the YAML disables
- **Category:** Feature
- **Scenario:** SC-001
- **Requirements:** FR-004
- **Preconditions:** a config with `web.enabled: false`.
- **Steps:**
  - Given the server started with `mcp-fs serve -c <cfg> --web`
  - When `tools/list` is called with a valid admin token
  - Then the returned names include `web.search`, `web.news`, `web.suggestions`, `web.fetch` and `web.download`
  - And the total tool count is exactly 50 (35 `fs.*` + 10 `admin.*` + 5 `web.*`)
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-010: Absence of a CLI flag leaves the YAML value untouched
- **Category:** Error
- **Scenario:** SC-001
- **Requirements:** FR-004
- **Preconditions:** a config with `web.enabled: false`.
- **Steps:**
  - Given the server started with `mcp-fs serve -c <cfg>` and no feature flags
  - When `tools/list` is called
  - Then no returned name starts with `web.`
  - And the total tool count is exactly 45
  - And calling `tools/call` for `web.search` returns JSON-RPC error `-32602` with message `Unknown tool: 'web.search'`
- **Cleanup:** stop the server
- **Priority:** Medium

#### E2E-011: A relative mcp_path aborts startup
- **Category:** Error
- **Scenario:** SC-001
- **Requirements:** FR-005, FR-006
- **Preconditions:** a config with `server.mcp_path: mcp` (no leading slash).
- **Steps:**
  - Given that config
  - When `mcp-fs serve -c <cfg>` is run
  - Then the process exits non-zero within 10 seconds
  - And stderr contains `server.mcp_path must start with '/' (got 'mcp')`
  - And no socket is listening on the configured port
- **Cleanup:** none
- **Priority:** High

#### E2E-015: Initialize returns the frozen handshake
- **Category:** Core Journey
- **Scenario:** SC-002
- **Requirements:** FR-007, FR-012
- **Preconditions:** server running; a valid token for `ADMIN`.
- **Steps:**
  - Given the header `X-Forwarded-Authorization: Bearer <token>`
  - When `POST <mcp_path>` is issued with body `{"jsonrpc":"2.0","id":1,"method":"initialize"}`
  - Then the response `Content-Type` is `text/event-stream` and `Cache-Control` is `no-cache,no-store`
  - And the body starts with `event: message\ndata: ` and ends with `\n\n`
  - And the decoded `result.protocolVersion` equals `2024-11-05`
  - And `result.capabilities` equals `{"logging":{},"tools":{"listChanged":true}}`
  - And `result.serverInfo.name` equals `mcp-fs`
- **Cleanup:** none
- **Priority:** Critical

#### E2E-017: A successful tool call is framed with the frozen key order
- **Category:** Feature
- **Scenario:** SC-002
- **Requirements:** FR-010, FR-013
- **Preconditions:** server running; `ADMIN` token; project `PROJ` exists.
- **Steps:**
  - Given a valid bearer for `ADMIN`
  - When `tools/call` is issued with `{"name":"admin.list_projects","arguments":{}}` and `id: 7`
  - Then the raw decoded JSON object's keys appear in the order `result`, `id`, `jsonrpc`
  - And `result.content` is an array of exactly one object with `type` equal to `text`
  - And `result.content[0].text` parses as JSON containing a `projects` array
  - And `result` carries no `isError` key
- **Cleanup:** none
- **Priority:** Critical

#### E2E-018: A request with no bearer is refused before dispatch
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-007, FR-011
- **Preconditions:** server running.
- **Steps:**
  - Given no `X-Forwarded-Authorization` and no `Authorization` header
  - When `POST <mcp_path>` is issued with body `{"jsonrpc":"2.0","id":1,"method":"tools/list"}`
  - Then the response status is 401
  - And `Content-Type` is `application/json`, not `text/event-stream`
  - And the body equals `{"error":"ERR_UNAUTHENTICATED","detail":"no bearer token in request headers"}`
- **Cleanup:** none
- **Priority:** Critical

#### E2E-023: A token whose identity claim is blank is refused
- **Category:** Error
- **Scenario:** SC-002
- **Requirements:** FR-010, FR-011
- **Preconditions:** server running with `username_claim: email`; a token signed by the right key with claims `{"email":"   ","iss":"web-a2a","exp":<future>}`.
- **Steps:**
  - Given that token as the bearer
  - When `tools/list` is called
  - Then the response status is 401
  - And the body equals `{"error":"ERR_UNAUTHENTICATED","detail":"token identity claim is empty"}`
- **Cleanup:** none
- **Priority:** High

#### E2E-024: No public key configured refuses every token
- **Category:** Security
- **Scenario:** SC-002
- **Requirements:** FR-011
- **Preconditions:** a server started with `auth.jwt.public_key_path: ""`.
- **Steps:**
  - Given an otherwise perfectly valid token for `ADMIN`
  - When `tools/list` is called
  - Then the response status is 401
  - And the body equals `{"error":"ERR_UNAUTHENTICATED","detail":"no JWT public key configured"}`
- **Cleanup:** stop the server
- **Priority:** Critical

#### E2E-026: /health stays reachable while authentication is failing
- **Category:** Edge
- **Scenario:** SC-002
- **Requirements:** FR-005, FR-009
- **Preconditions:** a server started with `auth.jwt.public_key_path: ""`.
- **Steps:**
  - Given every MCP call is returning 401
  - When `GET /health` is issued with no headers
  - Then the response status is 200 and the body equals `{"status":"ok","version":"<crate version>"}`
- **Cleanup:** stop the server
- **Priority:** Medium

#### E2E-030: A notification is acknowledged with 202 and no body
- **Category:** Feature
- **Scenario:** SC-002
- **Requirements:** FR-013, FR-017
- **Preconditions:** server running; valid `ADMIN` token.
- **Steps:**
  - Given a valid bearer
  - When `POST <mcp_path>` is issued with body `{"jsonrpc":"2.0","method":"notifications/initialized"}`
  - Then the response status is 202
  - And the response body is empty
  - And a subsequent `tools/list` still succeeds, proving no session state was required
- **Cleanup:** none
- **Priority:** Critical

#### E2E-031: A tool name with underscores resolves to the dotted tool
- **Category:** Feature
- **Scenario:** SC-002
- **Requirements:** FR-014, FR-019
- **Preconditions:** server running; `ADMIN` token.
- **Steps:**
  - Given a valid bearer
  - When `tools/call` is issued with `{"name":"admin_list_projects","arguments":{}}`
  - Then the response is a success result whose `content[0].text` parses to an object containing `projects`
  - And the response carries no `error` key
  - And `tools/list` still reports the tool as `admin.list_projects`, not `admin_list_projects`
- **Cleanup:** none
- **Priority:** High

#### E2E-032: A domain failure is a result carrying isError, not a JSON-RPC error
- **Category:** Error
- **Scenario:** SC-002
- **Requirements:** FR-015
- **Preconditions:** server running; token for `ALICE`, who is not a platform admin.
- **Steps:**
  - Given `ALICE`'s bearer
  - When `tools/call` is issued with `{"name":"admin.list_all_projects","arguments":{}}` and `id: 11`
  - Then the decoded response has a `result` key and no `error` key
  - And `result.isError` is `true`
  - And `result.content[0].text` equals `An error occurred invoking 'admin.list_all_projects': ERR_FORBIDDEN: 'alice@example.com' is not a platform admin`
  - And the HTTP status is 200, because the transport succeeded
- **Cleanup:** none
- **Priority:** Critical

#### E2E-035: An unknown method and an unknown tool use different codes
- **Category:** Error
- **Scenario:** SC-002
- **Requirements:** FR-016, FR-018
- **Preconditions:** server running; `ADMIN` token.
- **Steps:**
  - Given a valid bearer
  - When `POST` is issued with `{"jsonrpc":"2.0","id":3,"method":"resources/list"}`
  - Then the decoded response equals `{"error":{"code":-32601,"message":"Method 'resources/list' is not available."},"id":3,"jsonrpc":"2.0"}`
  - And when `tools/call` is issued with `{"name":"admin.nope","arguments":{}}` and `id: 4`
  - Then the decoded response equals `{"error":{"code":-32602,"message":"Unknown tool: 'admin.nope'"},"id":4,"jsonrpc":"2.0"}`
- **Cleanup:** none
- **Priority:** High

#### E2E-036: A malformed body answers 500 with a JSON body, not an SSE frame
- **Category:** Error
- **Scenario:** SC-002
- **Requirements:** FR-017, FR-018
- **Preconditions:** server running; `ADMIN` token.
- **Steps:**
  - Given a valid bearer
  - When `POST <mcp_path>` is issued with the raw body `{not json`
  - Then the response status is 500
  - And `Content-Type` is `application/json`
  - And the body parses to an object whose `error` equals `ERR_INVALID_ARGUMENT` and whose `detail` starts with `invalid JSON-RPC request: `
  - And the body does not start with `event: message`
- **Cleanup:** none
- **Priority:** High

#### E2E-040: A tool failure message embeds the code and the message verbatim
- **Category:** Edge
- **Scenario:** SC-002
- **Requirements:** FR-015
- **Preconditions:** server running; `ADMIN` token; no project named `no-such-project`.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `tools/call` is issued with `{"name":"admin.list_members","arguments":{"project_id":"no-such-project"}}`
  - Then `result.isError` is `true`
  - And `result.content[0].text` equals `An error occurred invoking 'admin.list_members': ERR_PROJECT_NOT_FOUND: project 'no-such-project' not found`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-041: An unknown tool is a protocol error even for an admin
- **Category:** Edge
- **Scenario:** SC-002
- **Requirements:** FR-016
- **Preconditions:** server running; `ADMIN` token.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `tools/call` is issued with `{"name":"fs.definitely_not_a_tool","arguments":{"mount_id":"x"}}`
  - Then the decoded response has an `error` key and no `result` key
  - And `error.code` equals `-32602`
  - And `error.message` equals `Unknown tool: 'fs.definitely_not_a_tool'`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-042: A null id is treated as a notification
- **Category:** Edge
- **Scenario:** SC-002
- **Requirements:** FR-017, FR-019
- **Preconditions:** server running; `ADMIN` token.
- **Steps:**
  - Given a valid bearer
  - When `POST` is issued with `{"jsonrpc":"2.0","id":null,"method":"tools/list"}`
  - Then the response status is 202 and the body is empty
  - And no SSE frame is returned
- **Cleanup:** none
- **Priority:** Medium

#### E2E-045: Project creation provisions the volume and the owner membership
- **Category:** Side Effect
- **Scenario:** SC-003
- **Requirements:** FR-025
- **Preconditions:** server running; `ADMIN` token; no project `spec-foundation`.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `admin.create_project` is called with `{"project_id":"spec-foundation","owner":"alice@example.com"}`
  - Then the result parses to `{"project_id":"spec-foundation","owner":"alice@example.com","created_at":<string>}`
  - And `admin.list_members` for that project returns exactly one member `{"person":"alice@example.com","role":"owner","added_by":"alice@example.com"}`
  - And `admin.get_index_mode` returns `{"project_id":"spec-foundation","index_mode":"none"}`
  - And a subsequent `fs.list_dir` by `ALICE` on `/` succeeds, proving the volume exists with a root
- **Cleanup:** `admin.delete_project` for `spec-foundation`
- **Priority:** Critical

#### E2E-046: Failed volume provisioning leaves no ACL row behind
- **Category:** Data Integrity
- **Scenario:** SC-003
- **Requirements:** FR-025
- **Preconditions:** a server configured so volume provisioning fails deterministically, for example `infra.meta.dir` pointing at a path that exists as a read-only file rather than a directory; `ADMIN` token.
- **Steps:**
  - Given provisioning is guaranteed to fail
  - When `admin.create_project` is called with `{"project_id":"rollback-probe","owner":"alice@example.com"}`
  - Then the result carries `isError` true and the text names the provisioning failure
  - And `admin.list_all_projects` does not contain `rollback-probe`
  - And a second `admin.create_project` with the same id does **not** fail with `ERR_PROJECT_EXISTS`, proving the first attempt left no row
- **Cleanup:** restore the directory permissions
- **Priority:** Critical

#### E2E-051: An id at each length bound is accepted
- **Category:** Edge
- **Scenario:** SC-003
- **Requirements:** FR-025
- **Preconditions:** `ADMIN` token.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `admin.create_project` is called with `project_id` `abc` (3 chars) and then with a 32-character id `a0123456789012345678901234567890`
  - Then both calls succeed and both ids appear in `admin.list_all_projects`
- **Cleanup:** delete both projects
- **Priority:** Medium

#### E2E-052: An id one character past each bound is rejected
- **Category:** Edge
- **Scenario:** SC-003
- **Requirements:** FR-026
- **Preconditions:** `ADMIN` token.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `admin.create_project` is called with `project_id` `ab` (2 chars)
  - Then the result carries `isError` true and the text ends with `ERR_INVALID_ARGUMENT: project_id must be 3-32 chars, lowercase letters/digits/hyphens, alphanumeric bounds`
  - And the same assertion holds for a 33-character id
  - And the same assertion holds for `-abc`, `abc-`, `Abc` and `a_b`
  - And `admin.list_all_projects` contains none of them
- **Cleanup:** none
- **Priority:** Medium

#### E2E-053: Every feature flag enabled yields the full tool count
- **Category:** Edge
- **Scenario:** SC-003
- **Requirements:** FR-004
- **Preconditions:** a server started with `--git --web --context7 --sqlite --db` and `search.enabled: true` in YAML; pandoc absent from `PATH`.
- **Steps:**
  - Given that server
  - When `tools/list` is called
  - Then the count equals 86: 35 `fs.*` + 10 `admin.*` + 14 git + 5 web + 2 context7 + 8 sqlite + 5 db + 3 editor + 4 search, with the 2 pandoc tools absent
  - And the names include `git.auth_revoke`, `search.status` and `doc.open_editor`
- **Cleanup:** stop the server
- **Priority:** Low

#### E2E-055: Adding a member grants access; removing revokes it
- **Category:** Side Effect
- **Scenario:** SC-004
- **Requirements:** FR-028
- **Preconditions:** project `PROJ` owned by `ALICE`; `BOB` is not a member.
- **Steps:**
  - Given `BOB`'s bearer, and `fs.list_dir` on `PROJ` currently failing for `BOB`
  - When `ALICE` calls `admin.add_member` with `{"project_id":"spec-foundation","person":"bob@example.com"}`
  - Then the result equals `{"project_id":"spec-foundation","person":"bob@example.com","role":"member","added_by":"alice@example.com"}`
  - And `BOB`'s `fs.list_dir` on `/` now succeeds
  - And when `ALICE` calls `admin.remove_member` for `BOB`, the result equals `{"project_id":"spec-foundation","person":"bob@example.com","removed":true}`
  - And `BOB`'s `fs.list_dir` fails again with `ERR_FORBIDDEN`
- **Cleanup:** none
- **Priority:** Critical

#### E2E-059: Adding an existing member is not an error and does not duplicate
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-028
- **Preconditions:** `BOB` is already a member of `PROJ`.
- **Steps:**
  - Given `ALICE`'s bearer
  - When `admin.add_member` is called a second time for `BOB`
  - Then `admin.list_members` returns `BOB` exactly once
  - And the member count is unchanged
- **Cleanup:** none
- **Priority:** Medium

#### E2E-060: Membership is matched caselessly on add and on access
- **Category:** Edge
- **Scenario:** SC-004
- **Requirements:** FR-028
- **Preconditions:** project `PROJ` owned by `ALICE`.
- **Steps:**
  - Given `ALICE`'s bearer
  - When `admin.add_member` is called with `{"project_id":"spec-foundation","person":"BOB@Example.COM"}`
  - Then a token minted for `bob@example.com` passes `authorize` on `PROJ`
  - And `admin.list_members` shows one entry for that person, not two
- **Cleanup:** remove the member
- **Priority:** Medium

#### E2E-066: A caller with no memberships gets an empty list, not an error
- **Category:** Edge
- **Scenario:** SC-005
- **Requirements:** FR-029
- **Preconditions:** a valid token for `carol@example.com`, who belongs to no project and is not an admin.
- **Steps:**
  - Given that bearer
  - When `admin.list_projects` is called
  - Then the result parses to `{"projects":[]}`
  - And the response carries no `isError`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-068: A member passes the gate on their own project
- **Category:** Core Journey
- **Scenario:** SC-006
- **Requirements:** FR-021
- **Preconditions:** `BOB` is a member of `PROJ`.
- **Steps:**
  - Given `BOB`'s bearer
  - When `fs.list_dir` is called with `{"mount_id":"spec-foundation","path":"/"}`
  - Then the call succeeds and returns an entry list
  - And no error is returned
- **Cleanup:** none
- **Priority:** Critical

#### E2E-073: A platform admin who is not a member is refused volume access
- **Category:** Security
- **Scenario:** SC-006
- **Requirements:** FR-022
- **Preconditions:** project `PROJ` owned by `ALICE`; `ADMIN` is a platform admin and is **not** a member of `PROJ`.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `fs.list_dir` is called with `{"mount_id":"spec-foundation","path":"/"}`
  - Then the result carries `isError` true
  - And the text ends with `ERR_FORBIDDEN: 'admin@example.com' is not a member of 'spec-foundation'`
  - And the same `ADMIN` can still call `admin.list_all_projects` successfully, proving the platform authority is intact and merely does not extend to data
- **Cleanup:** none
- **Priority:** Critical

#### E2E-076: Admin status is matched caselessly
- **Category:** Edge
- **Scenario:** SC-006
- **Requirements:** FR-022, FR-024
- **Preconditions:** a server whose `auth.admins` contains `Admin@Example.com`.
- **Steps:**
  - Given a token minted for `admin@example.com`
  - When `admin.list_users` is called
  - Then the call succeeds
  - And the returned entry for that person carries `"is_admin": true`
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-079: Setting a mode persists it across a restart
- **Category:** Side Effect
- **Scenario:** SC-007
- **Requirements:** FR-030
- **Preconditions:** `search.enabled: true`; project `PROJ` with mode `none`.
- **Steps:**
  - Given `ALICE`'s bearer
  - When `admin.set_index_mode` is called with `{"project_id":"spec-foundation","mode":"bm25"}`
  - Then the result equals `{"project_id":"spec-foundation","index_mode":"bm25","previous_mode":"none","reindex_started":true}`
  - And after the server is restarted, `admin.get_index_mode` still returns `{"project_id":"spec-foundation","index_mode":"bm25"}`
- **Cleanup:** set the mode back to `none`
- **Priority:** High

#### E2E-087: Deleting a project removes its members and its volume
- **Category:** Side Effect
- **Scenario:** SC-008
- **Requirements:** FR-027
- **Preconditions:** project `PROJ` with `ALICE` as owner and `BOB` as member, holding at least one file.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `admin.delete_project` is called with `{"project_id":"spec-foundation"}`
  - Then the result equals `{"project_id":"spec-foundation","deleted":true}`
  - And `admin.list_all_projects` no longer contains it
  - And `BOB`'s `fs.list_dir` on it now fails with `ERR_PROJECT_NOT_FOUND`, not `ERR_FORBIDDEN`
  - And recreating a project with the same id yields an empty volume, with `fs.list_dir` on `/` returning no entries
- **Cleanup:** delete the recreated project
- **Priority:** Critical

#### E2E-088: Deleting a missing project is not found, even for an admin
- **Category:** Error
- **Scenario:** SC-008
- **Requirements:** FR-023, FR-027
- **Preconditions:** no project named `ghost-project`.
- **Steps:**
  - Given `ADMIN`'s bearer
  - When `admin.delete_project` is called with `{"project_id":"ghost-project"}`
  - Then the result carries `isError` true
  - And the text ends with `ERR_PROJECT_NOT_FOUND: project 'ghost-project' not found`
- **Cleanup:** none
- **Priority:** High

#### E2E-089: An owner who is not a platform admin can delete their own project
- **Category:** Edge
- **Scenario:** SC-008
- **Requirements:** FR-027
- **Preconditions:** project `owned-by-alice` created with `ALICE` as owner; `ALICE` is not in `auth.admins`.
- **Steps:**
  - Given `ALICE`'s bearer
  - When `admin.delete_project` is called with `{"project_id":"owned-by-alice"}`
  - Then the result equals `{"project_id":"owned-by-alice","deleted":true}`
  - And `BOB`, a plain member of another project, receives `ERR_FORBIDDEN` when attempting the same call on that other project
- **Cleanup:** none
- **Priority:** High

#### E2E-092: The audit log records an operation and returns entries oldest first
- **Category:** Feature
- **Scenario:** SC-009
- **Requirements:** FR-035, FR-036
- **Preconditions:** a `SafetyManager` built from `SafetyConfig::default()`.
- **Steps:**
  - Given a fresh manager and the session `("alice@example.com", "spec-foundation")`
  - When `record_audit` is called three times in order with ops `write`, `edit`, `delete` on paths `/a.txt`, `/a.txt`, `/b.txt`
  - Then `audit` returns exactly three entries
  - And their `op` values in order are `write`, `edit`, `delete`
  - And a different session key returns zero entries, proving per-session isolation
- **Cleanup:** none
- **Priority:** High

#### E2E-097: A refused quota charge does not advance the counter
- **Category:** Data Integrity
- **Scenario:** SC-009
- **Requirements:** FR-034
- **Preconditions:** a `SafetyManager` with `write_quota_bytes: 100`.
- **Steps:**
  - Given a session that has already been charged 90 bytes
  - When `charge_write` is called with 20 bytes
  - Then the call returns `ERR_WRITE_QUOTA_EXCEEDED` with the message `session write quota of 100 bytes exceeded`
  - And `bytes_written` still returns exactly 90, not 110
  - And a subsequent charge of 10 bytes succeeds and brings the total to exactly 100
- **Cleanup:** none
- **Priority:** High

#### E2E-098: The trash path flattens separators and carries a millisecond stamp
- **Category:** Error
- **Scenario:** SC-009
- **Requirements:** FR-035, FR-037
- **Preconditions:** a `SafetyManager` with `trash_dir: .mcp_trash`.
- **Steps:**
  - Given the normalized path `/src/nested/file.txt`
  - When `trash_path` is called
  - Then the result matches `^/\.mcp_trash/\d{13}__src__nested__file\.txt$`
  - And calling it twice for the same path yields two different strings when the millisecond clock has advanced
  - And for the path `/top.txt` the result matches `^/\.mcp_trash/\d{13}__top\.txt$`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-099: The audit log discards the oldest entry past the cap
- **Category:** Edge
- **Scenario:** SC-009
- **Requirements:** FR-035, FR-036
- **Preconditions:** a `SafetyManager` from defaults.
- **Steps:**
  - Given a session with 500 audit entries recorded, the first carrying path `/entry-0.txt`
  - When one more entry is recorded with path `/entry-500.txt`
  - Then `audit` returns exactly 500 entries
  - And no entry has path `/entry-0.txt`
  - And the last entry has path `/entry-500.txt`
  - And the first entry has path `/entry-1.txt`
- **Cleanup:** none
- **Priority:** High

#### E2E-103: A path at the ceiling passes and its trash path is derived from the normalized form
- **Category:** Edge
- **Scenario:** SC-009
- **Requirements:** FR-033, FR-037
- **Preconditions:** a `SafetyManager` constructed with the SQL Server ceiling.
- **Steps:**
  - Given a path whose normalized length equals the ceiling exactly
  - When `normalize_path` is called
  - Then it returns that path unchanged
  - And when the same path is one character longer, `normalize_path` returns `ERR_INVALID_ARGUMENT` whose message contains both the actual character count and the limit
  - And a path that only exceeds the ceiling **before** normalization, such as one padded with `/./` segments that collapse away, is accepted
- **Cleanup:** none
- **Priority:** Medium

#### E2E-112: create_empty and an empty write are indistinguishable
- **Category:** Edge
- **Scenario:** SC-010
- **Requirements:** FR-040, FR-043
- **Preconditions:** an in-memory volume.
- **Steps:**
  - Given a fresh volume
  - When `create_empty("/a.txt")` is called and `write_bytes_atomic("/b.txt", &[])` is called
  - Then `stat` on both returns `size` 0 and `sha256` `None`
  - And the blob store contains zero objects
  - And reading either path returns an empty byte vector
- **Cleanup:** none
- **Priority:** Medium

#### E2E-114: A subtree delete does not touch a sibling whose name differs by a LIKE metacharacter
- **Category:** Edge
- **Scenario:** SC-010
- **Requirements:** FR-041, FR-042
- **Preconditions:** an in-memory volume.
- **Steps:**
  - Given files at `/a_b/one.txt` and `/axb/two.txt`, both non-empty and distinct
  - When the subtree `/a_b` is removed
  - Then `/a_b/one.txt` no longer exists
  - And `/axb/two.txt` still exists with its original content
  - And the blob backing `/axb/two.txt` is still present
  - And a second volume holding a file at `/a_b/one.txt` is unaffected, proving `volume_id` scoping
- **Cleanup:** none
- **Priority:** High

#### E2E-115: Generated keys and a minted token authenticate against a default config
- **Category:** Feature
- **Scenario:** SC-001
- **Requirements:** FR-044
- **Preconditions:** an empty working directory; a config whose `auth.jwt` block is left at its defaults, with `public_key_path` pointing at `.keys/jwt.pub`.
- **Steps:**
  - Given no `.keys` directory
  - When `mcp-fs keys` is run with no `--dir`
  - Then the directory `.keys` exists and contains exactly `jwt.key` and `jwt.pub`
  - And when `mcp-fs token alice@example.com` is run with no other flags, a token is printed on stdout and nothing else is written to stdout
  - And that token authenticates successfully against a server started with that config, proving the `keys` defaults and the `auth.jwt` defaults agree
- **Cleanup:** remove `.keys`
- **Priority:** Critical

#### E2E-116: A token minted with mismatched parameters is refused
- **Category:** Error
- **Scenario:** SC-001
- **Requirements:** FR-044
- **Preconditions:** a generated keypair; a server with `auth.jwt` at its defaults.
- **Steps:**
  - Given a token minted with `--issuer wrong-issuer`
  - When it is used against the server
  - Then the response status is 401 with `ERR_UNAUTHENTICATED`
  - And a token minted with `--claim sub` is refused with the detail `token has no 'email' claim`
  - And a token minted with `--ttl 1`, used two seconds later, is refused for expiry
- **Cleanup:** remove the generated keys
- **Priority:** High

#### E2E-117: Key generation creates its target directory
- **Category:** Edge
- **Scenario:** SC-001
- **Requirements:** FR-044
- **Preconditions:** a temporary path whose parent does not exist.
- **Steps:**
  - Given `mcp-fs keys --dir /tmp/spec-keys/nested`
  - When the command completes
  - Then the directory exists even though its parent did not
  - And it contains exactly `jwt.key` and `jwt.pub`
  - And a server started with `public_key_path` pointing at that `jwt.pub` accepts a token minted from the matching `jwt.key`
- **Cleanup:** remove `/tmp/spec-keys`
- **Priority:** High

### 12.3 Modified Test Specifications

None. No existing test changes behaviour; §14 requires only that existing tests be annotated with the `E2E-XXX` id they satisfy.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

No prior specification exists, so there is nothing to contradict. Two consistency observations against the non-spec artifacts:

1. **`TOOL_CONTRACT.txt` remains authoritative for tool schemas.** This document specifies gates, side effects and return shapes; it does not restate parameter descriptions, which are frozen in `TOOL_CONTRACT.txt:1-40` and machine-checked against `tool-contract-golden.json`. If the two ever disagree, the golden file wins and this document is the one to amend.
2. **`AGENTS.md` describes 59 tools** in its overview, counting the always-on families plus git. The registry assertions in `tools/all.rs:63-140` give 45 always-on (35 + 10), 14 for the two git families, and up to 86 with every flag set. The numbers are reconcilable but the phrasing in `AGENTS.md` is easy to misread; §14 notes the documentation touch-up.

## 14. Migration & Implementation Notes

No production code changes. The work this spec obliges is test and documentation work, in this order:

1. **Annotate existing tests first.** Add the `E2E-XXX` id as a comment on each test listed as **Existing** in §12.1, at the locations in §9.3. Doing this before writing new tests prevents duplicate coverage.
2. **Add the new engine-level tests** (E2E-092, E2E-097, E2E-098, E2E-099, E2E-103, E2E-112, E2E-114). These are pure Rust unit tests with no fixture dependency and no server, so they can land independently and first.
3. **Add the new protocol and tool scenarios** (the remaining New tests). These need a running server and the fixture identities from §12; extend `tests/functional/scenarios/01_admin_lifecycle.sh` and `10_security.sh` rather than creating parallel scripts where the subject matches.
4. **E2E-046 requires a deliberate provisioning failure.** Introduce it by configuration (a read-only `infra.meta.dir`), never by adding a test-only failure hook to production code.
5. **E2E-053 asserts a tool count that depends on pandoc.** Guard it the way `tools/all.rs:117` already does, skipping or adjusting when `which::which("pandoc")` succeeds.
6. **Documentation last**, once the ids are stable: update `AGENTS.md` per §10.2 and §13.

Rollback is trivial: every change is additive test or documentation content.

## 15. Open Questions & TBDs

- **TBD-001:** The write quota and read guard are per process and in memory (`safety.rs:2-4`). Under more than one replica their semantics change silently. This spec records the limitation; whether to make session state shared is a product decision deferred to a later spec.
- **TBD-002:** `admin.list_users` returns every person known to the ACL (`storage/admin.rs:259`). Whether a platform admin listing every identity in the deployment is acceptable under the intended privacy posture has not been decided; the behaviour is specified as-built.
- **TBD-003:** `E2E-053`'s tool count of 86 assumes `search.enabled: true` with a configured embedding endpoint is not required for registration. Registration is gated only on `search.enabled` (`app.rs:80`), so the count holds, but the interaction with S7 should be re-verified when S7 is written.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Volume** | One project's simulated filesystem: a metadata tree plus content-addressed bytes. Provisioned at project creation, torn down at deletion. | Volume |
| **mount_id** | The tool parameter naming the target project. Equal to the project id and to the `volume_id` column. | Protocol |
| **volume_id** | The column scoping every `nodes` and `blob_refs` row to one volume. | Volume |
| **Platform admin** | A person listed in `auth.admins`. Holds project lifecycle authority and no data access. | Platform |
| **Member** | A person on a project's membership list. Passes `authorize` for that project. | Platform |
| **Owner** | The member with role `owner`, named at creation. Cannot be removed from the project. | Platform |
| **Session** | Transient per-`(person, project)` state: recorded reads, bytes written, audit ring. In memory. | Session |
| **Read guard** | The rule that a write to a path not read in this session is refused. | Session |
| **Write quota** | The per-session byte allowance enforced by `charge_write`. | Session |
| **Trash path** | The soft-delete destination derived from the source path and a millisecond stamp. | Session |
| **Content addressing** | Storing bytes under their sha256 so identical content is stored once per volume. | Volume |
| **Refcount** | The per-volume count of nodes referencing one blob. Reaching zero is what triggers collection of the bytes. | Volume |
| **Relational seam** | `RelationalDb`/`RelationalTx`, the engine-independent surface every store speaks instead of a driver. | Volume |
| **Dialect** | The per-engine renderer turning canonical `?N` SQL into engine SQL. | Volume |
| **Index mode** | The per-project setting in `none|bm25|rag|both` controlling how much content is indexed. | Platform |
| **Tool registry** | The name-to-schema-and-handler map rendered by `tools/list`. | Protocol |
| **SSE event** | The `event: message\ndata: {json}\n\n` envelope carrying one JSON-RPC message. | Protocol |
| **Retryable** | A flag on an error marking a transient database failure, carried without a distinct error code. | Protocol |

## 17. Interview Decisions Log

This specification was produced non-interactively from the code at the user's instruction, so this log records the scoping decisions actually taken rather than a live interview transcript.

- **DEC-001:** Split the retro-specification into eight sequenced specs along the dependency spine rather than one monolith or one spec per tool family. **Rationale:** one spec would carry several hundred FRs and over a thousand tests, which nobody reviews; one per family would be twenty interviews with most specs too thin to justify the ceremony. **Alternatives considered:** single spec; per-family specs; per-crate specs. **Implemented by:** n/a — no code impact. **Round:** 1. **Code evidence:** tool family registration order and gating at `crates/mcp-fs/src/tools/all.rs:20-46`.
- **DEC-002:** S1 owns the MCP protocol layer (`mcp/`), not S2. **Rationale:** the 10 `admin.*` tools are MCP tools; without the protocol layer they have no callable surface and S1's tests could not run end to end. **Alternatives considered:** deferring `mcp/` to S2 and testing admin tools through direct handler calls. **Implemented by:** FR-012..FR-019. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/app.rs:228-247`.
- **DEC-003:** S1 covers SQLite and the local blob backend only; PostgreSQL, SQL Server, S3 and the `migrate` verb defer to S4. **Rationale:** the default build carries neither optional driver, so the default posture is the honest baseline. **Alternatives considered:** specifying all backends at once. **Implemented by:** FR-043 (the seam), with engines out of scope per §3.2. **Round:** 1. **Code evidence:** `crates/mcp-fs/Cargo.toml:31-44`.
- **DEC-004:** S1 specifies only the config sections it owns (`server`, `auth`, `infra`, `safety`); the other ten `ServerConfig` sections defer to their owning specs. **Rationale:** otherwise S1 swallows git, web, search and doc configuration it does not implement. **Alternatives considered:** specifying the whole config file in S1. **Implemented by:** FR-002, FR-003. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/config.rs:772-786`.
- **DEC-005:** §7.5 describes the as-built `tracing` setup and declares OpenTelemetry absent, sending it to the backlog rather than writing OTel requirements. **Rationale:** a retro-specification must describe what exists; inventing an OTel requirement would silently convert this from a specification into an implementation project. **Alternatives considered:** requiring OTel as new work in this spec. **Implemented by:** n/a — no code impact; deferred as BL-001. **Round:** 1. **Code evidence:** only `tracing-subscriber` is present at `Cargo.toml:98` and `crates/mcp-fs/Cargo.toml:79`; the subscriber is a plain stderr `fmt` layer at `crates/mcp-fs/src/logging.rs:29-37`.
- **DEC-006:** Keep the safety layer and content addressing in S1 (option A), specified at the engine boundary, rather than moving them to S2 where they become tool-observable. **Rationale:** they are the invariants every other layer rests on; moving them would leave S2 owning both the fs surface and the safety engine, and S2 is the larger spec already. **Alternatives considered:** option B, deferring both to S2. **Implemented by:** FR-032..FR-043. **Round:** 2. **Code evidence:** `core/fs_ops.rs` is the only caller of the safety manager for tool paths; the existing engine-level tests sit at `crates/mcp-fs/src/safety.rs:186-290` and `crates/mcp-fs/src/storage/meta.rs:902-943`.
- **DEC-007:** §12 places tool-level tests in `tests/functional/scenarios/` and engine-level tests in Rust `#[cfg(test)]` modules, rather than creating a new top-level `tests/` tree. **Rationale:** those are the two layers that already exist and already run; a third layout would fragment the suite. **Alternatives considered:** a workspace-level `tests/` directory per the template's default. **Implemented by:** §12 placement rules; §14 step 1. **Round:** 1. **Code evidence:** `tests/functional/scenarios/` holds 16 scenario scripts; no crate under `crates/` has a `tests/` directory.
- **DEC-008:** Existing tests are referenced rather than re-specified, and are required only to carry their `E2E-XXX` id. **Rationale:** roughly 240 unit tests already cover this layer; re-specifying them would pad the document without adding a single assertion. **Alternatives considered:** writing full Given/When/Then for all 114 tests. **Implemented by:** §12.2 preamble; §14 step 1. **Round:** 1. **Code evidence:** test function counts per file in §9.3.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 1 (closed: FR-044 added for `keys.rs`) | 3 (A-1 state.rs citation, A-2 blob layout, A-5 normalize_identity citation) | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** FR-044 added with E2E-115, E2E-116, E2E-117; citations corrected; test totals recomputed to 117
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
