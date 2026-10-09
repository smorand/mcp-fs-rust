> Id: SPEC-0007
> Nature: FEAT
> Status: as-built
> Area: git
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# mcp-fs Git — Design Document (as-built)

## 1. Components

| Component | File (current) | Role |
|---|---|---|
| Object database | `crates/core/src/git/odb.rs` (507 lines) | `BlobObjectDb`: read/write/exists/prefix lookup over the blob store plus the SQLite/relational index; `export_to_repo`/`import_from_repo` hydrate and drain the on-disk cache. |
| Index | `crates/core/src/git/db.rs` (830 lines; 464 at spec time) | `RelationalGitDb`, tables `git_objects`, `git_refs`, `git_remotes` (all `volume_id`-scoped). Grew post-spec to add `git_operations`, owned by the dev-process spec. |
| Repository store | `crates/core/src/git/repo.rs` (357 lines) | Per-project `git2::Repository` handle, in-process cache, per-repository write lock, lazy open-from-disk, purge-on-delete. |
| Smart HTTP | `crates/core/src/git/http/mod.rs` (1370 lines; 1372 at spec time) + `http/pktline.rs` | Axum routes for ref advertisement, upload-pack, receive-pack; pkt-line framing; membership + bearer auth; body-size enforcement. |
| OAuth device flow | `crates/core/src/git/oauth/device_flow.rs`, `store.rs`, `cipher.rs`, `persistence.rs` | RFC 8628 device grant against GitHub/GitLab; AES-256-GCM encryption at rest; conditional persistence. |
| Tools | `crates/core/src/tools/git.rs` (original 11 of 39 tools today), `tools/git_auth.rs` (original 3 of 4 tools today) | Typed async functions called by `mcp::server::McpServer`'s `#[tool]` methods; never reimplement the engine logic. |

## 2. Flows

### 2.1 Commit and read (US-001, US-002)
`git.commit` → engine writes via the cached `git2::Repository` handle (write lock held) → `import_from_repo` copies new objects into the blob store + `git_objects` index → response. A subsequent `git.log`/`git.show`/`git.diff`/`git.blame` → `export_to_repo` hydrates the on-disk ODB from the blob store (idempotent; cheap if objects are already present) → `git2` answers the read.

### 2.2 Clone/push over HTTP (US-003)
`GET info/refs?service=...` → membership + bearer check → `export_to_repo` → ref advertisement with capability string (`agent=mcp-fs/0.1.0`, `multi_ack_detailed` on upload-pack). `POST git-upload-pack` → revwalk from every "want" tip → full pack built → NAK. `POST git-receive-pack` → body size checked twice (body-limit middleware layer + explicit check, same config value) → pack indexed → `import_from_repo` → refs updated → report-status reply, band 1, beginning with `unpack ok`.

### 2.3 Device flow login (US-004)
`git.auth` → request device code from provider → return user code immediately → spawn background poll task. Poll loop classifies provider responses: `authorization_pending`/`slow_down` → continue (with fallback interval if `interval` is absent/zero); `access_denied`/expiry → terminal, no token stored; success → token encrypted (if persistence key set) and stored keyed by `(person, provider)`. `git.auth_status`/`git.auth_revoke` read/remove from the process-singleton store.

### 2.4 Deletion and migration (US-005)
Project delete → volume teardown → git row purge (`git_objects`/`git_refs`/`git_remotes` for that `volume_id`) → on-disk `state/git/{id}.db` + `state/git-repos/{id}/` removed (`purge_repo`) → ACL row removed. Deployment migration copies the three git tables row-for-row, explicitly skipping the OAuth token table.

## 3. Interfaces

- **MCP tools:** 11 `git.*` + 3 `git.auth*`, all requiring `mount_id` except the auth tools (which require only authentication). Error vocabulary: stable `ERR_*` codes via `ToolError`.
- **HTTP:** `GET /git/{mount_id}/info/refs?service={git-upload-pack|git-receive-pack}`, `POST /git/{mount_id}/git-upload-pack`, `POST /git/{mount_id}/git-receive-pack`. Auth: `Authorization: Bearer`, with `Basic` accepted (password = token) to satisfy the git CLI's credential-helper prompt behavior. 401 challenge: `WWW-Authenticate: Bearer realm="mcp-fs"`.
- **Internal:** `BlobObjectDb` (odb.rs) is the single read/write surface onto git object bytes; no caller touches the blob store directly for `git:` keys.

## 4. Data and State

| Table | Key | Fields | Home |
|---|---|---|---|
| `git_objects` | `(volume_id, hash)` | `hash`, `type`, `size` | `git/db.rs` |
| `git_refs` | `(volume_id, name)` | ref name, target | `git/db.rs` |
| `git_remotes` | `(volume_id, name)` | remote name, URL | `git/db.rs` |
| OAuth session | `(person, provider)` | encrypted token, clear metadata, `expires_at` | `git/oauth/persistence.rs` |

Stored git object: blob key `git:{sha}`, content `{type} {len}\0{payload}`. On-disk layout: `state/git/{project_id}.db` (index), `state/git-repos/{project_id}/` (bare repo, rebuildable cache — safe to delete entirely at any time).

## 5. Configuration

| Setting | Effect |
|---|---|
| `git.enabled` (+ `--git` CLI override) | Gates registration of all 14 original tools and the HTTP routes. |
| `git.max_pack_size_mb` | Bounds push body size; enforced by both a body-limit layer and an explicit check against the same value. |
| `git.anonymous_read` | Relaxes the HTTP route auth requirement for reads. |
| `MCPFS_TOKEN_KEY` | 32-byte key (base64/hex), gates whether the OAuth token store persists (encrypted) or is memory-only. |
| `{provider}_client_secret_env` | Names the env var read at call time for the OAuth client secret; never cached in a struct field. |

## 6. Observability

None specific to git: no per-operation timing, no pack-size metric, no export/import volume counter. This is accurately-described current behaviour, not a gap introduced by this document.

## 7. Decisions Log

- **DEC-001:** No custom libgit2 ODB backend is registered; the export/import pair is the permanent design, not a stopgap. **Rationale:** `git2` exposes only `add_disk_alternate`/`add_new_mempack_backend`; a real `git_odb_backend` requires hand-rolled unsafe FFI with manual lifetime management, which is untestable and against the project's `unsafe_code = "forbid"` stance. **Alternatives considered:** writing the unsafe backend (rejected). **Code evidence:** `crates/core/src/git/odb.rs:9-24` (module doc).
- **DEC-002:** Three deliberate divergences from the prior reference implementation are treated as requirements, not notes: membership enforced on git HTTP routes, `max_pack_size_mb` actually enforced, and the `unpack ok` report line sent. **Rationale:** matching the reference in these three spots would have been actively wrong (a security hole, an unenforced limit, and a protocol bug that makes real `git push` report failure on success). **Code evidence:** `crates/core/src/git/http/mod.rs:579` (unpack ok), route auth checks, body-size checks.
- **DEC-003:** Full packs (no have/want negotiation) are current behaviour with an open question attached, not a defect to silently fix in this spec. **Rationale:** negotiation is a feature to build, not a bug; stating it as a TBD keeps the spec honest about what exists today.
- **DEC-004:** The most valuable tests for this layer drive a real `git` CLI against a running server rather than asserting on recorded byte sequences. **Rationale:** unit tests over pkt-line framing prove framing, not interoperability with an actual client.
- **DEC-005:** Deleting the on-disk object cache and proving reads still succeed is the load-bearing test pattern for the "blob store is the source of truth" claim. **Rationale:** the claim is only meaningful if it's directly falsifiable by deletion.

## 8. Requirement to Code Map

| FR | Code |
|---|---|
| FR-001, FR-002 | `crates/core/src/git/odb.rs` (key layout, write path); `crates/core/src/git/db.rs:49-53` type area (object index row) |
| FR-003 | `crates/core/src/git/odb.rs:9-24` (module doc, no custom ODB backend) |
| FR-004, FR-005 | `crates/core/src/git/odb.rs` (`export_to_repo`/`import_from_repo`) |
| FR-006 | `crates/core/src/git/odb.rs` (prefix lookup against the index) |
| FR-007, FR-008 | `crates/core/src/git/repo.rs` (lazy open, in-process cache) |
| FR-009 | `crates/core/src/git/repo.rs` (per-repository write lock) |
| FR-010 | `crates/core/src/git/repo.rs` (`purge_repo`); `crates/core/src/tools/admin.rs` (delete cascade) |
| FR-011 | `crates/core/src/tools/all.rs` (conditional registration) |
| FR-012 | `crates/core/src/tools/git.rs` (membership check before engine call) |
| FR-013 | `crates/core/src/tools/git_auth.rs` (authentication-only gate) |
| FR-014 | `crates/core/src/tools/git.rs` (history tools call into odb/repo; `checkout_file` routes through the write-accounting engine) |
| FR-015, FR-018, FR-023 | `crates/core/src/git/http/mod.rs` (routes, advertisement, pktline framing) |
| FR-016, FR-017 | `crates/core/src/git/http/mod.rs` (membership + bearer/401 challenge) |
| FR-019 | `crates/core/src/git/http/mod.rs` (revwalk, full pack, NAK) |
| FR-020, FR-021 | `crates/core/src/git/http/mod.rs:579` (unpack ok); import into blob store on receive-pack |
| FR-022 | `crates/core/src/git/http/mod.rs` (body-limit layer + explicit check against `max_pack_size_mb`) |
| FR-024, FR-025 | `crates/core/src/git/oauth/device_flow.rs` (device code request, poll classification) |
| FR-026 | `crates/core/src/git/oauth/device_flow.rs` (secret read at call time, not held in a field) |
| FR-027 | `crates/core/src/git/oauth/cipher.rs` (AES-256-GCM) |
| FR-028 | `crates/core/src/git/oauth/persistence.rs` (conditional on `MCPFS_TOKEN_KEY`) |
| FR-029 | `crates/core/src/tools/git_auth.rs` (`auth_status`/`auth_revoke`) |

## 9. Legacy Mapping

Source: `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md` (pre-move).

| Pre-move reference | Current location | Note |
|---|---|---|
| `crates/mcp-fs/src/git/odb.rs` | `crates/core/src/git/odb.rs` | Crate root moved from `crates/mcp-fs` to `crates/core` per the workspace's current layout (`crates/mcp-fs` now holds only `main.rs`); content and line-level claims still hold. |
| `crates/mcp-fs/src/git/db.rs` | `crates/core/src/git/db.rs` | Same crate-root move; grew 464 → 830 lines (the `git_operations` table, out of scope here, added by the dev-process spec). |
| `crates/mcp-fs/src/git/repo.rs` | `crates/core/src/git/repo.rs` | Same crate-root move; unchanged content, 357 lines both before and after. |
| `crates/mcp-fs/src/git/http/mod.rs` | `crates/core/src/git/http/mod.rs` | Same crate-root move; 1372 → 1370 lines, `unpack ok` still present at line 579. |
| `crates/mcp-fs/src/git/oauth/*` | `crates/core/src/git/oauth/*` | Same crate-root move; `store.rs` grew (PR credential scope gate, out of scope here). |
| `crates/mcp-fs/src/tools/git.rs`, `git_auth.rs` | `crates/core/src/tools/git.rs`, `git_auth.rs` | Same crate-root move; both files now hold 39/4 tools respectively (28 and 1 added by the dev-process spec), of which this document specifies only the original 11/3. |
| S1 (membership gate, error vocabulary, catalogue gating) | Platform/filesystem foundation spec (migrated) | Reused, not restated. |
| S4 (relational seam, migration FR-420/FR-424) | Storage/backends spec (migrated) | Reused, not restated. |
| `specs/archived/SPEC-0011_...-full-git-dev-process/spec.md` | SPEC-0012, migrated | Owns the 35 additive tools, `git_operations`, the merge engine, named remotes, pull requests. Exact migrated spec number to be confirmed. |
| GitHub Enterprise host map / per-host token store | SPEC-0011, migrated | Owns `git.hosts`, host-to-provider resolution, the per-host token store screen. Exact migrated spec number to be confirmed. |
