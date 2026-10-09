> Id: SPEC-0002
> Nature: FEAT
> Status: as-built
> Area: platform-foundation
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# Platform Foundation — Specification

## 1. Summary

The platform exposes a simulated multi-project filesystem to callers over a tool-calling protocol. This document specifies the **platform foundation**: the layer every other capability stands on. It covers startup and configuration, the error vocabulary, bearer-token identity, the tool-calling protocol surface, the storage seam (metadata tree, content-addressed bytes, project membership), the safety contract (path containment, read-before-write, write quota, audit, soft delete), and the project administration capabilities.

This is a **retro-specification**: the behaviour described is already implemented and shipping. Its purpose is to make the contract explicit, testable and auditable, so that later work has a written baseline instead of an oral tradition.

It is the first of a planned sequence covering the filesystem engine, a REST data plane, multi-backend storage, documents, source control integration, search/RAG, and optional integrations, all building on the contract fixed here.

## 2. Current State

### 2.1 How it works today

The system exposes a set of always-on tool-calling capabilities for filesystem operations and platform administration, plus additional capability families that can be toggled on. Server state (the metadata tree, access control, and related indexes) is held in a durable store; file bytes are held in a separate blob store, local or remote.

A project is a named container with its own simulated filesystem ("volume"). Projects are created and deleted by platform administrators; membership within a project is managed by that project's owner. Holding platform administration rights does not, by itself, grant access to any project's data — data access is governed strictly by project membership.

Callers authenticate with a bearer token carrying a verifiable identity claim. Identity comparison is caseless and consistent everywhere it is used (admin membership, project membership, ownership).

Every tool call that targets a project is gated, in order: identity resolution, then project membership authorization, then path containment and safety checks, then the underlying operation.

The safety contract governs every write: paths are normalized and confined to the project's root, a session must have read a path before writing to it (when enabled), each session has a byte quota for writes, every operation is recorded in a capped audit trail, and deletions are soft (moved to a recoverable location) unless hard deletion is explicitly allowed.

Content is stored once per project regardless of how many paths reference it; identical bytes written at a second path increase a reference count rather than being stored again, and bytes are only discarded once nothing references them anymore.

Ten project administration capabilities exist: creating and deleting a project, adding/removing/listing members, listing a caller's own projects, listing every project and every known identity (admin-only), and setting/reading a project's content-indexing mode.

### 2.2 Existing specifications governing this area

None. This is the first specification in the repository for this functional area. A de-facto contract existed previously as a frozen catalogue of capability names, descriptions and schemas, machine-checked on every test run; that catalogue remains the source of truth for exact parameter shapes and is not restated here.

### 2.3 Existing test coverage

Coverage exists today at two layers: focused tests exercising configuration parsing, identity verification, the safety contract, storage behaviour (content addressing, garbage collection, scoping), and the administration capabilities directly; and scenario-level tests exercising the administration lifecycle and authorization refusals end to end. See Section 8 for the consolidated end-to-end test catalogue and `## 8 Requirement to code map` in the companion design document for the underlying files.

## 3. Scope

### 3.1 In Scope

- Configuration schema for server, authentication, infrastructure stores and safety; environment-variable expansion; boot-time validation; the command-line verbs for serving, key generation, token minting and version reporting.
- The stable error-code vocabulary, its mapping to transport-level outcomes, and a transient/retryable flag.
- Bearer-token identity: header precedence, accepted signature families, issuer/audience/expiry validation, identity-claim extraction, caseless normalization.
- The tool-calling protocol surface: handshake, capability catalogue, successful and failed call framing, notification handling, unknown-tool/unknown-method handling, and tolerant tool-name resolution.
- The storage seam: the metadata, blob and administration contracts; content addressing, deduplication and reference-count based garbage collection; per-volume data isolation; safe subtree-pattern matching; volume provisioning and teardown.
- The project access-control model: project id validation, caseless membership, owner/member roles, content-indexing mode persistence.
- The safety contract: path normalization, path-length ceiling, read-before-write guard, per-session write quota, capped audit log, soft-delete path derivation.
- The ten project administration capabilities, their authorization gates and return shapes.

### 3.2 Out of Scope (Non-Goals)

- The filesystem operation capabilities themselves and the engine behind them (a later specification). **Consequence:** the safety layer and content addressing specified here are exercised end to end only through that later capability set; this specification verifies them at the boundary where they already have test coverage.
- The REST data plane and its API documentation surface (a later specification).
- Additional relational and blob storage engines beyond the default, and any offline migration capability (a later specification). The storage seam is in scope only as an abstraction boundary; no additional engine behaviour is specified here.
- Document extraction, source-control integration, search/RAG, and the optional non-filesystem capability families (later specifications).
- The interactive command-line client.
- Distributed tracing/telemetry export. It does not exist in the current system; see Section 7 and the backlog.

## 4. Actors

| Actor | Description |
|---|---|
| **Operator** | Deploys and runs the system. Owns configuration, the signing keypair and the process lifecycle. Never authenticated: acts through the filesystem and the command line. |
| **Platform admin** | A person listed in the platform's admin roster. Creates and deletes projects, lists every project and every known identity. Does **not** thereby gain file access. |
| **Project owner** | The person named as owner at project creation. Manages membership and the content-indexing mode of that project. |
| **Project member** | A person added to a project. May read the project's data and list its members. |
| **Caller** | Any program speaking the tool-calling protocol with a bearer token. |

### Bounded Contexts

| Context | Scope | Key concepts |
|---|---|---|
| **Platform** | Projects, membership, platform administration. | Project, Member, Person |
| **Volume** | One project's simulated filesystem: metadata tree plus content-addressed bytes. | Node, blob, project data scope |
| **Session** | Per-caller, per-project transient safety state. Lost on restart. | Recorded reads, audit entries, write quota |
| **Protocol** | The tool-calling wire surface: handshake, capability catalogue, call framing. | Tool schema, tool catalogue, call context |

A "person" in the Platform context is an identity string normalized caseless; in the Session context the same string is half of the session key. A "project id" in the Platform context is the access-control key; the same value names the project in every tool call and scopes every row of its data.

## 5. Usage Scenarios

### SC-001: Operator boots the system from a configuration file

**Actor:** Operator
**Preconditions:** a built system; a resolvable configuration file; a signing keypair generated ahead of time; every variable referenced by the configuration set in the environment.
**Flow:**
1. Operator starts the server, optionally naming a config path and any optional capability families to force on.
2. The configuration path resolves through a fixed precedence order.
3. The file is read, variable references are expanded, and the result is parsed into the server configuration.
4. Every configured store (metadata, administration, source-control index, token store) is validated, along with the document-conversion and search sections if present.
5. Any command-line capability flag force-enables that family regardless of the file's value.
6. The server assembles its internal state, registers its capabilities, and exposes a liveness endpoint and the tool-calling endpoint.
7. The listener binds and serves until a graceful shutdown signal.

**Postconditions:** the liveness endpoint answers unauthenticated with a status and version; the tool-calling endpoint answers at the configured path; the capability catalogue holds exactly the enabled families.
**Exceptions:**
- EXC-001a: configuration file absent → invalid-argument error naming the path
- EXC-001b: malformed configuration → invalid-argument error
- EXC-001c: a store section is misconfigured → a rejection naming the offending section
- EXC-001d: the tool-calling path is not absolute → boot aborts
- EXC-001e: the listening address is already in use → a bind error
- EXC-001f: no configuration found anywhere → the resolver reports the list of paths it tried

### SC-002: Caller authenticates with a bearer token and discovers the capability catalogue

**Actor:** Caller
**Preconditions:** server running; the public key matching the token's signing key configured; the token carries the configured issuer and identity claim.
**Flow:**
1. Caller obtains a signed token for a known identity.
2. Caller sends a handshake request carrying the bearer token in a configured header.
3. The server resolves identity before any dispatch.
4. The server answers the handshake with its protocol version and capabilities.
5. Caller requests the capability catalogue; the server returns it.
6. Caller invokes a capability it is entitled to.

**Postconditions:** the caller holds the capability catalogue; every subsequent call is attributed to the normalized identity from the token's claim.
**Exceptions:**
- EXC-002a: no bearer token present anywhere accepted → unauthenticated error, no dispatch
- EXC-002b: signature, issuer or expiry invalid → unauthenticated error
- EXC-002c: no verification key configured → unauthenticated error
- EXC-002d: identity claim missing from the token → unauthenticated error
- EXC-002e: identity claim present but blank → unauthenticated error
- EXC-002f: unknown method name → a protocol-level error distinct from a capability failure

### SC-003: Platform admin creates a project, which provisions its volume

**Actor:** Platform admin
**Preconditions:** caller is a platform admin; the chosen project id is free.
**Flow:**
1. Caller invokes the create-project capability with a project id and an owner identity.
2. The system requires platform-admin standing, validates the id format, and rejects a blank owner.
3. The access-control record is created with the owner's membership.
4. The project's volume is provisioned.
5. The capability returns the project id, owner and creation time.

**Postconditions:** the project exists with content-indexing mode "none"; the owner is a member with role owner; the volume exists with an empty root.
**Exceptions:**
- EXC-003a: caller not a platform admin → forbidden
- EXC-003b: id fails format validation → invalid-argument error naming the format rule
- EXC-003c: blank owner → invalid-argument error
- EXC-003d: id already taken → project-exists error
- EXC-003e: volume provisioning fails → the access-control record is rolled back and the provisioning error is returned

### SC-004: Owner manages project membership

**Actor:** Project owner (or platform admin)
**Preconditions:** the project exists; the caller owns it or is a platform admin.
**Flow:**
1. Caller adds a member by identity.
2. Caller lists members to confirm; a plain member may also do this.
3. Caller removes a member to revoke access.

**Postconditions:** an added person passes authorization for that project; a removed person no longer does.
**Exceptions:**
- EXC-004a: caller neither owner nor admin → forbidden
- EXC-004b: project missing → project-not-found
- EXC-004c: attempt to remove the owner → refused
- EXC-004d: platform admin acting on a missing project → project-not-found, not success

### SC-005: Caller lists projects; admin lists all projects and all identities

**Actor:** Member, or platform admin
**Flow:**
1. Any authenticated caller lists their own project memberships.
2. A platform admin lists every project and every known identity.

**Postconditions:** the caller holds the list they are entitled to; the caller's own-project listing marks which entries they own.
**Exceptions:**
- EXC-005a: a non-admin requesting the all-projects or all-identities listing → forbidden
- EXC-005b: a caller with no memberships → an empty list, not an error

### SC-006: A non-member is refused, and platform admin standing does not bypass the gate

**Actor:** Non-member
**Preconditions:** a project exists that the caller does not belong to.
**Flow:**
1. Caller invokes any project-scoped capability.
2. The system checks membership before any data access.
3. The call is refused.

**Postconditions:** no project data is read or written; the caller learns only whether the project exists.
**Exceptions:**
- EXC-006a: project exists, caller not a member → forbidden
- EXC-006b: project does not exist → project-not-found, distinct from forbidden
- EXC-006c: caller is a platform admin but not a member → still forbidden

### SC-007: Project content-indexing mode is set and read back

**Actor:** Owner or platform admin
**Preconditions:** the project exists; content indexing is enabled server-wide for writes.
**Flow:**
1. Caller sets the indexing mode to one of a fixed vocabulary.
2. The system checks owner-or-admin standing, validates the mode, and refuses a vector-capable mode with no embedding destination configured.
3. The new mode is stored before any index wipe begins.
4. Caller reads the mode back.

**Postconditions:** the mode persists across restart; the response reports whether a background reindex began.
**Exceptions:**
- EXC-007a: unknown mode value → invalid-argument error listing the accepted values
- EXC-007b: indexing not enabled server-wide → not-supported error naming the setting to enable
- EXC-007c: vector-capable mode with no embedding destination → invalid-argument error
- EXC-007d: project missing → project-not-found

### SC-008: Platform admin deletes a project, cascading members and volume

**Actor:** Platform admin or project owner
**Preconditions:** the project exists.
**Flow:**
1. Caller invokes delete-project.
2. The system checks owner-or-admin standing, tears down the volume, purges any source-control indexing state, then deletes the access-control record.
3. The capability returns the project id and a deleted flag.

**Postconditions:** the project, its memberships and its volume are gone; a project recreated under the same id inherits nothing.
**Exceptions:**
- EXC-008a: caller neither owner nor admin → forbidden
- EXC-008b: project missing → project-not-found
- EXC-008c: volume teardown fails → the error propagates and the access-control record survives, so the operation is retryable

### SC-009: The safety layer governs a write attempt

**Actor:** System-internal (invoked through the filesystem capability set; verified here at the safety boundary)
**Preconditions:** a safety manager configured from the safety settings; a session keyed by (person, project).
**Flow:**
1. A caller-supplied path is normalized.
2. When the read guard is on, a write to a path never read in this session is refused.
3. The byte count is charged against the session quota.
4. The operation is appended to the session's audit log.
5. A delete derives a recoverable destination rather than destroying data.

**Postconditions:** the stored path is absolute and normalized; the session's write-byte counter reflects the charge; the audit log holds the entry, oldest first.
**Exceptions:**
- EXC-009a: a null byte in the path → path-out-of-bounds error
- EXC-009b: path escaping the project root → path-out-of-bounds error
- EXC-009c: path beyond the active store's length ceiling → invalid-argument error naming the limit
- EXC-009d: write without a prior read → edit-without-prior-read error
- EXC-009e: quota exceeded → write-quota-exceeded error

### SC-010: Content is stored once and garbage-collected at zero references

**Actor:** System-internal (verified at the volume/metadata boundary)
**Preconditions:** a project volume with a metadata store and a blob store.
**Flow:**
1. Bytes are hashed; the content is stored under its hash; the path record is written.
2. Writing identical bytes at a second path increments a reference count rather than storing twice.
3. Deleting one of the two paths decrements the count and keeps the content.
4. Deleting the last referencing path removes the stored content.

**Postconditions:** content exists exactly once per distinct byte sequence per project; no orphaned content survives a delete.
**Exceptions:**
- EXC-010a: empty content → no content is stored; the path record reports zero size
- EXC-010b: overwriting a path with new content removes the old content once nothing else references it, preserving the path's creation time
- EXC-010c: rewriting a path with identical content removes nothing
- EXC-010d: two projects holding identical content are independent; neither delete affects the other

## 6. Functional Requirements

EARS notation. Only MUST / SHALL / MUST NOT / SHALL NOT are used.

### Configuration and boot

| FR | Requirement | Scenario | Evidence |
|---|---|---|---|
| FR-001 | WHEN the server resolves its configuration file path THE server SHALL select, in order: an explicit path argument, then an environment variable, then an XDG config-home probe, then a default directory/name pair. An explicit path is authoritative even when the named file does not exist, so the resulting error names the path the operator asked for. A blank environment value is ignored. | SC-001 | `crates/core/src/cli.rs:255-262` |
| FR-002 | WHEN the server reads a configuration file THE server SHALL expand variable references (including defaulted references) before parsing the file content into the server configuration. | SC-001 | `crates/core/src/config.rs:802-806,1006` |
| FR-003 | IF any configured store section names a backend inconsistent with its connection string or with the compiled feature set THEN the server SHALL fail to boot with an invalid-argument error naming the offending section. | SC-001 | `crates/core/src/config.rs:874-915` |
| FR-004 | WHEN the server receives a capability-family force-enable flag THE server SHALL enable that family regardless of the configuration file's value. Flags force-enable only; none force-disables a family already enabled in the file. | SC-001 | `crates/core/src/cli.rs:57-74`, `crates/core/src/app.rs:73-81` |
| FR-005 | The server SHALL answer its liveness probe with a status and version, over HTTP 200, without requiring a bearer token and without touching a database. | SC-001 | `crates/core/src/app.rs:183-185` |
| FR-006 | IF the configured tool-calling path does not begin with a path separator THEN the server SHALL abort startup naming the offending value, before the listener binds. | SC-001 | `crates/core/src/app.rs:85-87` |
| FR-007 | WHEN a request arrives THE server SHALL look for a bearer token in a configured header first, then a standard authorization header, accepting a bearer scheme, a basic scheme where the password is the token, and a bare token with no scheme and no embedded space. | SC-002 | `crates/core/src/identity.rs:118-148` |
| FR-008 | WHEN a token is verified THE server SHALL validate its signature against the configured verification key, its issuer when one is configured, and its expiry/not-before claims with a defined clock-skew leeway. Audience is validated only when configured. Every configured signature algorithm is honoured. | SC-002 | `crates/core/src/identity.rs:161-175` |
| FR-009 | The server SHALL NOT accept a symmetric (shared-secret family) signature algorithm for bearer verification. | SC-002 | `crates/core/src/identity.rs:34-44,87-90` |
| FR-010 | WHEN a token passes verification THE server SHALL read the caller's identity from the configured claim and normalize it caseless. A missing or blank claim is rejected with a distinct message in each case. | SC-002 | `crates/core/src/identity.rs:180-187` |
| FR-011 | IF identity resolution fails THEN the server SHALL answer an unauthenticated error and SHALL NOT dispatch the request. Identity is resolved before body parsing and before any dispatch. | SC-002 | `crates/core/src/app.rs:193-202,271-280` |
| FR-012 | WHEN the server receives a handshake request THE server SHALL answer with its protocol version, its capability set, and server identification including its version. The handshake is accepted but not required for a call to proceed. | SC-002 | `crates/core/src/mcp/server.rs` |
| FR-013 | WHEN the server receives a capability-catalogue request THE server SHALL return every registered capability in registration order, with the parameter schema and description frozen against the published contract. | SC-002 | `crates/core/src/mcp/server.rs` |
| FR-014 | WHEN a capability call succeeds THE server SHALL answer with the result content framed as the protocol's success shape, carrying the call's correlation id. | SC-002 | `crates/core/src/mcp/server.rs` |
| FR-015 | WHEN a capability call fails with a domain error THE server SHALL answer a protocol-level success envelope marked as an error result, naming the failing capability and the error code and message, rather than a protocol-level error. Only an unknown capability name is a protocol-level error. | SC-002 | `crates/core/src/mcp/server.rs` |
| FR-016 | IF a call names a capability absent from the catalogue THEN the server SHALL answer a protocol-level error naming the unknown capability; IF a request names an unsupported method THEN the server SHALL answer a distinct protocol-level error naming the unsupported method. | SC-002 | `crates/core/src/app.rs` |
| FR-017 | WHEN a request carries no correlation id (a notification) THE server SHALL acknowledge it without a result body. | SC-002 | `crates/core/src/app.rs:223-227` |
| FR-018 | IF the request body cannot be parsed THEN the server SHALL answer an invalid-argument error distinct from a framed protocol-level parse error. | SC-002 | `crates/core/src/app.rs:204-220` |
| FR-019 | The server SHALL resolve a requested capability name by exact match first, and then by treating any separator variant of that name as equivalent, without changing the canonical name reported by the capability catalogue. | SC-002 | `crates/core/src/mcp/server.rs` |
| FR-020 | The server SHALL expose a fixed, stable set of error codes and SHALL map each to a fixed, consistent outcome category (authentication, authorization, not-found, conflict, validation, quota, not-supported, internal). A transient store failure carries a retryable indicator without introducing a new error code. | multiple | `crates/core/src/errors.rs` |
| FR-021 | The server SHALL authorize every project-scoped capability call by project membership alone. | SC-006 | `crates/core/src/state.rs:61-63` |
| FR-022 | The server SHALL NOT grant a platform admin access to a project's data on the basis of the admin role alone. | SC-006 | `crates/core/src/state.rs:61-63` |
| FR-023 | IF a platform admin invokes an owner-gated capability on a project that does not exist THEN the server SHALL answer project-not-found. The admin shortcut skips the ownership check but not the existence check. | SC-006 | `crates/core/src/state.rs:49-60` |
| FR-024 | The server SHALL compare identities caselessly for platform-admin standing, project membership and project ownership, using one shared normalization so the three comparisons cannot drift apart. | SC-006 | `crates/core/src/config.rs:866-869` |
| FR-025 | WHEN project creation succeeds THE server SHALL have created the access-control record, the owner's membership and the project's volume; WHEN volume provisioning fails THE server SHALL delete the access-control record before returning the error. | SC-003 | `crates/core/src/tools/admin.rs:177-180` |
| FR-026 | The server SHALL accept a project id of a bounded length composed only of lowercase letters, digits and hyphens, beginning and ending with a letter or digit, and SHALL reject any other form with an invalid-argument error naming the rule. | SC-003 | `crates/core/src/storage/admin.rs:345-357` |
| FR-027 | WHEN project deletion succeeds THE server SHALL have torn down the volume, purged the project's source-control indexing state when that capability is enabled, and deleted the access-control record and every membership, in that order, so a failure anywhere leaves the operation retryable. | SC-008 | `crates/core/src/tools/admin.rs:295-304` |
| FR-028 | WHEN a membership mutation is invoked THE server SHALL require the caller to be the project owner or a platform admin, and SHALL refuse to remove the owner. | SC-004 | `crates/core/src/storage/admin.rs:452,460` |
| FR-029 | WHEN a listing capability is invoked THE server SHALL apply its own gate: the caller's own project listing requires only authentication; member listing and indexing-mode reading require membership or platform-admin standing; the all-projects and all-identities listings require platform-admin standing. The caller's own listing marks ownership; the all-projects listing does not. | SC-005 | `crates/core/src/tools/admin.rs:213-262` |
| FR-030 | WHEN the indexing mode is changed THE server SHALL store the new mode before wiping or rebuilding any index, so a crash between the two steps leaves a state one more call repairs rather than one nothing repairs. | SC-007 | `crates/core/src/tools/admin.rs:322-325` |
| FR-031 | The server SHALL accept exactly a fixed vocabulary of indexing-mode values and SHALL reject any other value with an invalid-argument error. | SC-007 | `crates/core/src/storage/traits.rs:88-96` |
| FR-032 | WHEN a caller-supplied path is normalized THE server SHALL reject a path containing a null byte, make the path absolute, collapse relative segments, and reject any path that escapes the project's root. | SC-009 | `crates/core/src/safety.rs:89-106` |
| FR-033 | IF the active metadata store imposes a path-length ceiling THEN the server SHALL reject a longer normalized path with an invalid-argument error naming the store, the actual length and the limit, checked on the normalized form before any store access. | SC-009 | `crates/core/src/safety.rs:59-75` |
| FR-034 | WHILE the read-before-write guard is enabled THE server SHALL refuse a write to a path the session has not read, with a dedicated error. Session state is held in memory, keyed by caller and project, and is lost on restart. | SC-009 | `crates/core/src/safety.rs:116` |
| FR-035 | WHEN a byte charge would carry a session's running total past its configured write quota THE server SHALL refuse the charge atomically, leaving the session counter unchanged, with a dedicated error. | SC-009 | `crates/core/src/safety.rs:131-140` |
| FR-036 | The server SHALL retain at most a fixed number of audit entries per session, discarding the oldest first, and SHALL return them oldest first. | SC-009 | `crates/core/src/safety.rs:159-161` |
| FR-037 | The server SHALL derive a soft-delete destination from the original path and a timestamp, so repeated deletes of the same path produce distinct destinations; hard delete is disabled unless explicitly configured on. | SC-009 | `crates/core/src/safety.rs:168-172` |
| FR-038 | WHEN non-empty content is written THE server SHALL store the bytes once per project under a content hash and SHALL increment a reference count when the same content is written at another path, rather than storing it again. | SC-010 | `crates/core/src/storage/meta.rs:277-294` |
| FR-039 | WHEN the last path referencing a stored content is deleted or overwritten THE server SHALL delete the underlying bytes; WHILE any other path still references it THE server SHALL retain them. | SC-010 | `crates/core/src/storage/meta.rs:296-333` |
| FR-040 | IF the content written is empty THEN the server SHALL record a zero-size entry with no stored content and SHALL NOT store any bytes. | SC-010 | `crates/core/src/storage/meta.rs` |
| FR-041 | The server SHALL scope every stored record to its owning project, so one shared store can hold every project without one project's operation affecting another's rows. | SC-010 | `crates/core/src/storage/meta.rs:862-863,1194` |
| FR-042 | The server SHALL build every subtree-match pattern through a single escaping routine, so a literal match character in a path never matches an unrelated path. | SC-010 | `crates/core/src/storage/meta.rs:231-235` |
| FR-043 | WHEN a key-generation command is run THE server SHALL write a new asymmetric signing keypair to the target location; WHEN a token-minting command is run for a given identity THE server SHALL print a signed token, using defaults that work against a freshly generated keypair and a default configuration with no further alignment. | SC-001 | `crates/core/src/keys.rs:22-32,45,62,80` |
| FR-044 | The server SHALL access all persisted project and administration state through one consistent internal data-access boundary rather than directly through any specific store driver, so the same operation behaves identically across every supported backend. | multiple | `crates/core/src/storage/rel/mod.rs` |

### Note on appearance-order renumbering

FR-043 and FR-044 above correspond to the legacy document's FR-044 (key generation / token minting) and FR-043 (single data-access boundary) respectively; they are renumbered here in the order they appear in this document.

## 7. Non-Functional Requirements

Only requirements whose effect is observable to an operator or caller are listed here. Mechanism-only requirements (how the system achieves them internally) are in the companion design document.

### 7.1 Performance
- The liveness probe answers without any store access.
- No caller-facing request is blocked indefinitely on a store operation.
- A large file's contents can be read in a sub-range rather than requiring the whole file to be read into memory first.

### 7.2 Security
- Every capability call requires a verified bearer token; only the liveness probe is unauthenticated.
- Only asymmetric signature algorithms are accepted for token verification (FR-009).
- Secrets are supplied only through the environment; connection strings never print their credentials in logs or diagnostics.
- Platform administration confers no data access (FR-022).
- Authorization is checked before path normalization, and path normalization before any store access (FR-021, FR-032).

### 7.3 Usability
- Error messages name the offending value and, where a remedy exists, state it.
- Generated credentials (tokens) are emitted separately from diagnostic output, so a caller can capture just the credential.

### 7.4 Reliability
- Project creation is atomic with respect to the access-control record: a failed volume provisioning rolls that record back (FR-025).
- The indexing mode is persisted before the index itself is mutated, so the recoverable failure mode is the one a retry repairs (FR-030).
- A transient store failure is marked retryable without inventing a new error code.
- The server shuts down gracefully.

### 7.5 Observability
- Operational events (unauthenticated requests, capability failures) are recorded with enough detail to diagnose them, with client-facing failures recorded at a lower severity than internal failures.
- Tokens, signing keys and connection-string credentials are never recorded.
- No distributed tracing/telemetry export exists today; this is a known gap (see backlog).

### 7.6 Deployment
- The system is a single self-contained deployable unit.
- The default store configuration requires no external database or object store.
- Secret management is via the environment, expanded at load time (FR-002).

### 7.7 Scalability
- One shared store can hold every project's data, distinguished by project scope (FR-041).
- Per-session safety state (read guard, write quota) is held in memory per running process, so it is **not** shared across multiple running instances of the server. Running more than one instance changes that state's effective meaning; this limitation is recorded rather than solved here.

## 8. End-to-End Tests

| ID | Covers | Test | Level | State |
|---|---|---|---|---|
| E2E-001 | SC-001, FR-001, FR-005 | Server boots from an explicit config and answers the liveness probe | e2e | green |
| E2E-002 | SC-001, FR-002 | Config parsing exercises variable expansion | unit | green |
| E2E-003 | SC-001, FR-003 | Boot validation rejects a misconfigured store section | unit | green |
| E2E-004 | SC-001, FR-004 | A capability-family flag enables a family the config disables | e2e | green |
| E2E-005..009 | SC-001, FR-001-003 | Config resolution and validation error paths | unit | green |
| E2E-010 | SC-001, FR-004 | Absence of a flag leaves the config value untouched | e2e | green |
| E2E-011 | SC-001, FR-005, FR-006 | A relative tool-calling path aborts startup | e2e | green |
| E2E-012..014 | SC-001, FR-001, FR-002, FR-006 | Config resolution/path edge cases | unit | green |
| E2E-015 | SC-002, FR-007, FR-012 | Handshake returns the frozen protocol response | e2e | green |
| E2E-016 | SC-002, FR-008 | Token verification success/failure paths | unit | green |
| E2E-017 | SC-002, FR-010, FR-013 | A successful capability call is framed with the frozen key order | e2e | green |
| E2E-018 | SC-002, FR-007, FR-011 | A request with no bearer is refused before dispatch | e2e | green |
| E2E-019..020 | SC-002, FR-008 | Token verification security paths | unit | green |
| E2E-021 | SC-002, FR-009 | A symmetric-algorithm token is refused | unit | green |
| E2E-022..023 | SC-002, FR-010, FR-011 | Identity-claim extraction error paths | unit/e2e | green |
| E2E-024 | SC-002, FR-011 | Unauthenticated error shape | e2e | green |
| E2E-025..028 | SC-002, FR-007-009 | Header precedence and algorithm edge cases | unit/e2e | green |
| E2E-029 | SC-002, FR-012, FR-014 | Handshake plus successful call framing | e2e | green |
| E2E-030 | SC-002, FR-013, FR-017 | Capability catalogue plus notification acknowledgment | e2e | green |
| E2E-031 | SC-002, FR-014, FR-019 | Successful call framing plus tolerant name resolution | e2e | green |
| E2E-032 | SC-002, FR-015 | A domain failure is framed as an error result, not a protocol error | e2e | green |
| E2E-033 | SC-002, FR-015, FR-018 | Malformed body distinct from a domain error | unit/e2e | green |
| E2E-034 | SC-002, FR-016 | Unknown capability name | e2e | green |
| E2E-035 | SC-002, FR-016, FR-018 | Unknown method vs malformed body | e2e | green |
| E2E-036 | SC-002, FR-017, FR-018 | Notification acknowledgment vs malformed body | e2e | green |
| E2E-037..042 | SC-002, FR-012-019 | Protocol edge cases | unit/e2e | green |
| E2E-043 | SC-003, FR-020, FR-025 | Project creation happy path | e2e | green |
| E2E-044 | SC-003, FR-026 | Project id format accepted | unit | green |
| E2E-045 | SC-003, FR-025 | Side effects of successful creation | e2e | green |
| E2E-046 | SC-003, FR-025 | Rollback on provisioning failure | e2e | green |
| E2E-047..049 | SC-003, FR-020, FR-026 | Project id and error-code validation | unit | green |
| E2E-050 | SC-003, FR-025 | Authorization gate on creation | e2e | green |
| E2E-051..053 | SC-003, FR-025, FR-026, FR-004 | Creation edge cases | unit/e2e | green |
| E2E-054 | SC-004, FR-028 | Membership mutation happy path | e2e | green |
| E2E-055 | SC-004, FR-028 | Side effects of membership mutation | e2e | green |
| E2E-056..058 | SC-004, FR-028 | Membership mutation error paths | unit | green |
| E2E-059..060 | SC-004, FR-028 | Membership mutation edge cases | unit | green |
| E2E-061..063 | SC-005, FR-029 | Listing capability happy paths | unit | green |
| E2E-064..065 | SC-005, FR-029 | Listing capability authorization gates | unit | green |
| E2E-066..067 | SC-005, FR-029 | Listing capability edge cases | unit/e2e | green |
| E2E-068 | SC-006, FR-021 | Non-member refusal happy-path check | e2e | green |
| E2E-069 | SC-006, FR-020, FR-021 | Non-member refusal error code | unit | green |
| E2E-070 | SC-006, FR-021, FR-022 | Admin standing does not bypass membership gate | unit | green |
| E2E-071 | SC-006, FR-023 | Owner-gate existence check | unit | green |
| E2E-072 | SC-006, FR-024 | Caseless identity matching | unit | green |
| E2E-073 | SC-006, FR-022 | Admin-without-membership refusal | e2e | green |
| E2E-074..076 | SC-006, FR-022-024 | Authorization edge cases | unit | green |
| E2E-077..078 | SC-007, FR-030, FR-031 | Indexing-mode happy paths | unit | green |
| E2E-079 | SC-007, FR-030 | Persist-before-wipe side effect | e2e | green |
| E2E-080..082 | SC-007, FR-030, FR-031 | Indexing-mode error paths | unit | green |
| E2E-083..084 | SC-007, FR-030, FR-031 | Indexing-mode edge cases | unit | green |
| E2E-085 | SC-008, FR-027 | Project deletion happy path | e2e | green |
| E2E-086 | SC-008, FR-027 | Deletion error path | unit | green |
| E2E-087 | SC-008, FR-027 | Deletion side effects | e2e | green |
| E2E-088 | SC-008, FR-023, FR-027 | Deletion on a missing project | unit | green |
| E2E-089 | SC-008, FR-027 | Deletion edge case (teardown failure retry) | e2e | green |
| E2E-090..091 | SC-009, FR-032, FR-034, FR-037 | Safety contract happy paths | unit | green |
| E2E-092 | SC-009, FR-035, FR-036 | Write quota and audit log happy path | unit | green |
| E2E-093..096 | SC-009, FR-020, FR-032-034 | Safety contract error paths | unit | green |
| E2E-097 | SC-009, FR-034 | Read-guard data-integrity check | unit | green |
| E2E-098..099 | SC-009, FR-035-037 | Write-quota/audit error and edge cases | unit | green |
| E2E-100..103 | SC-009, FR-032, FR-033, FR-036, FR-037 | Safety contract edge cases | unit | green |
| E2E-104 | SC-010, FR-038, FR-044 | Content addressing happy path | unit | green |
| E2E-105 | SC-010, FR-039 | Garbage collection happy path | unit | green |
| E2E-106 | SC-010, FR-038, FR-041 | Data-integrity: dedup and scoping | unit | green |
| E2E-107 | SC-010, FR-039, FR-042 | GC and safe pattern matching | unit | green |
| E2E-108 | SC-010, FR-040 | Empty-content error path | unit | green |
| E2E-109 | SC-010, FR-038, FR-040 | Content-addressing edge case | unit | green |
| E2E-110..111 | SC-010, FR-039, FR-042, FR-044 | GC and data-boundary edge cases | unit | green |
| E2E-112 | SC-010, FR-040, FR-044 | Empty-content edge case | unit | green |
| E2E-113 | SC-010, FR-038, FR-041 | Cross-project isolation data integrity | unit | green |
| E2E-114 | SC-010, FR-041, FR-042 | Scoping and pattern-matching edge case | unit | green |
| E2E-115 | SC-001, FR-043 | Key generation and token minting happy path | e2e | green |
| E2E-116 | SC-001, FR-043 | Key/token error path | e2e | green |
| E2E-117 | SC-001, FR-043 | Key/token edge case | e2e | green |

## 9. Glossary

| Term | Definition | Context |
|---|---|---|
| **Volume** | One project's simulated filesystem: a metadata tree plus content-addressed bytes. Provisioned at project creation, torn down at deletion. | Volume |
| **Project id / mount id** | The identifier naming the target project in every call and scoping its data. | Protocol / Volume |
| **Platform admin** | A person listed in the platform's admin roster. Holds project lifecycle authority and no data access. | Platform |
| **Member** | A person on a project's membership list. Passes authorization for that project. | Platform |
| **Owner** | The member with role owner, named at creation. Cannot be removed from the project. | Platform |
| **Session** | Transient per-(person, project) state: recorded reads, bytes written, audit ring. Held in memory. | Session |
| **Read guard** | The rule that a write to a path not read in this session is refused. | Session |
| **Write quota** | The per-session byte allowance for writes. | Session |
| **Trash path / soft delete** | The recoverable destination derived from the source path and a timestamp. | Session |
| **Content addressing** | Storing bytes under a hash of their content so identical content is stored once per project. | Volume |
| **Reference count** | The per-project count of paths referencing one stored content. Reaching zero triggers removal. | Volume |
| **Index mode** | The per-project setting controlling how much content is indexed for search. | Platform |
| **Capability catalogue** | The set of callable operations the system advertises. | Protocol |
| **Retryable** | A flag on an error marking a transient store failure, carried without a distinct error code. | Protocol |

## 10. Confidence notes

- **Verified against current code (sampled):** `crates/core/src/app.rs` (liveness/shutdown region), `crates/core/src/safety.rs` (`normalize_path`), `crates/core/src/errors.rs` (error-code constants and `http_status`/`is_client_error`). Content and intent matched the legacy citations; exact line numbers shifted slightly against the legacy document's line references, consistent with ordinary code drift.
- **Not individually re-verified, due to budget:** the remaining `file:LINE` citations carried forward from the legacy document (`config.rs`, `identity.rs`, `storage/*`, `tools/admin.rs`, `keys.rs`). These are preserved as best-available pointers; treat exact line numbers as approximate.
- **Significant known staleness:** the legacy document describes a hand-rolled JSON-RPC/SSE protocol layer (FR-012 through FR-019 above, and the corresponding E2E-015 through E2E-042). The current repository's own index (`AGENTS.md`) states the MCP transport was migrated to the official SDK (`rmcp`), replacing that hand-rolled layer, and that the file this spec's citations point at (`mcp/mod.rs`) was removed in favor of `mcp/server.rs`. FR-012 through FR-019 are therefore retained here as the historical, as-built behaviour the legacy document captured, but they should be treated as **likely superseded** rather than current. See the companion design document's findings section.
- **Tool/family counts** quoted in the legacy document (e.g. specific tool totals in end-to-end tests) were not re-verified against the current 102-tool contract stated in `AGENTS.md`, and are likely stale for the same reason.
- Paraphrased throughout: all prose was rewritten to remove file paths, schema and API-shape detail per the functional-only constraint on this document; no factual claim was invented beyond what the legacy document stated.
