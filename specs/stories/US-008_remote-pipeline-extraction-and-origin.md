# US-008: Extract git/remote.rs, validate the URL, and record origin at clone

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 8
> Depends On: US-002, US-005
> Complexity: L
> min_tier: 1
> Files touched: 3

## Objective

One module owns host resolution, URL validation, credential supply, timeout and the four remote operations, so the security properties are proved once rather than four times. Clone migrates onto it and finally records its URL as remote `origin`, which is the only source push, fetch and pull will have for that URL.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # the single pipeline; clone_to_temp moves here from tools/git.rs:1003-1020
crates/mcp-fs/src/tools/git.rs  # remote_clone :715-830 becomes a thin adapter; RemoteCallbacks at :1009
crates/mcp-fs/src/git/db.rs  # add_remote :281, list_remotes :298, upsert-by-name test :429-442
```

### Existing Patterns
- `git2::RemoteCallbacks` is constructed exactly once in the whole tree today, at `crates/mcp-fs/src/tools/git.rs:1009` inside `clone_to_temp`. It must still be constructed in exactly one file after this story.
- The credential is `git2::Cred::userpass_plaintext("oauth2", &t)` (`crates/mcp-fs/src/tools/git.rs:1014`) — a plain HTTPS credential with no OAuth semantics, which is why a PAT works identically to a device-flow token. Do not change its shape (DEC-009).
- `add_remote` (`crates/mcp-fs/src/git/db.rs:281`) already exists, is tested for upsert-by-name (`:429-442`), and has **no production caller**. This story supplies its first one.
- Remote work runs off the request thread on the existing `on_git_thread` path (`crates/mcp-fs/src/tools/git.rs:724`).
- The audit entry for clone already records the URL (`crates/mcp-fs/src/tools/git.rs:794-800`), which stays correct only because this story rejects userinfo-bearing URLs.

### Data Model (excerpt)
- `git_remotes` row — unchanged schema `(volume_id, name, url)` (`crates/mcp-fs/src/git/db.rs:69-75`), keyed `(volume_id, name)`. **Newly written**: FR-NEW-020 supplies its first production writer.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-011:** Scope grows to full bidirectional remote sync. **Rationale:** a token that can only clone cannot serve the write path. **Alternatives considered:** push only; deferring both. **Implemented by:** FR-NEW-021 to FR-NEW-036. **Round:** 2a. **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1009` is the only `RemoteCallbacks` in the tree
- **DEC-021:** Push, fetch and pull take no `url`; they resolve the stored `origin`. No remote-management tools. **Implemented by:** FR-NEW-021. **Round:** 2b. **Code evidence:** `crates/mcp-fs/src/git/db.rs:298`
- **DEC-022:** `git.remote_clone` must persist the clone URL as `origin`. **Rationale:** push, fetch and pull have no other source for the URL, and the table plus its API already exist unused. **Implemented by:** FR-NEW-020. **Round:** 2b. **Code evidence:** table `crates/mcp-fs/src/git/db.rs:69-75`, API `:281-305`, test `:429-442`, and no call anywhere in `crates/mcp-fs/src/tools/git.rs:715-830`

### Applicable NFRs
- **Transport restricted to HTTPS** (§7.2, FR-NEW-041): `ssh`, `git`, `file` and `http` all rejected. `file://` in particular would expose the server's filesystem.
- **No credentials in URLs** (§7.2, FR-NEW-042): userinfo-bearing URLs rejected before they can be stored as `origin` or emitted in an audit entry.
- **One pipeline** (§7.2): every security property in this specification rests on there being one implementation rather than four.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-054 [EARS-U]: One module owns the remote pipeline
> The mcp-fs server SHALL implement host resolution, URL validation, credential supply, timeout and the four remote operations exactly once, in `crates/mcp-fs/src/git/remote.rs`, and `crates/mcp-fs/src/tools/git.rs` SHALL call into it without reimplementing any of those five steps.

- **Inputs:** The source tree after implementation.
- **Outputs:** `git2::RemoteCallbacks` is constructed in exactly one file, as it is today; `git.hosts` is read in exactly one file.
- **Business Rules:** Mirrors the standing rule that `core::fs_ops` is the single implementation behind both the MCP and REST layers.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-008. DEC-034 was recorded as a decision but obliged no numbered requirement, while Section 7.2 rests every security property on there being one pipeline rather than four.

#### FR-NEW-041 [EARS-O]: Non-HTTPS remote URLs are rejected
> IF a remote URL's scheme is not `https`, THEN THE mcp-fs server SHALL reject the operation with `ERR_INVALID_ARGUMENT`, naming the scheme.

- **Inputs:** `ssh://git@github.com/o/r.git`, `git://github.com/o/r.git`, `file:///tmp/r`, `http://github.com/o/r.git`.
- **Outputs:** `ERR_INVALID_ARGUMENT` for each. No network call, no filesystem access.
- **Business Rules:** SSH is a non-goal; `file://` would let a caller read the server's filesystem.
- **Priority:** Must-have

#### FR-NEW-042 [EARS-O]: Remote URLs carrying userinfo are rejected
> IF a remote URL contains a userinfo component, THEN THE mcp-fs server SHALL reject the operation with `ERR_INVALID_ARGUMENT` and SHALL NOT include the URL in the error message.

- **Inputs:** `https://alice:ghp_secret@github.com/o/r.git`.
- **Outputs:** `ERR_INVALID_ARGUMENT` naming the host only. The credential appears in no log, trace or audit entry, and the URL is not stored as `origin`.
- **Business Rules:** Such a URL would otherwise be recorded as `origin` and later emitted in audit entries, which record the URL today (`git.rs:794-800`).
- **Priority:** Must-have

#### FR-NEW-020 [EARS-E]: Clone records the origin remote
> WHEN `git.remote_clone` completes successfully, THE mcp-fs server SHALL record the clone URL as the remote named `origin` for that volume.

- **Inputs:** A clone of `https://github.ibm.com/org/repo.git` into mount `proj-a`.
- **Outputs:** `list_remotes` for `proj-a` returns `[("origin", "https://github.ibm.com/org/repo.git")]`.
- **Business Rules:** Uses the existing `add_remote` (`crates/mcp-fs/src/git/db.rs:281`). Re-cloning the same volume from a different URL replaces the row, since `add_remote` upserts by name (`db.rs:429-442`).
- **Priority:** Must-have
- **Rationale:** `git_remotes` is live schema with a tested API and, today, no writer at all: `git.remote_clone` (`git.rs:715-830`) never calls it. Push, fetch and pull have no other source for the URL.

#### FR-NEW-021 [EARS-O]: A volume without an origin cannot reach a remote
> IF `git.remote_push`, `git.remote_fetch` or `git.remote_pull` is called on a volume with no recorded `origin`, THEN THE mcp-fs server SHALL reject the call, stating that the volume has no origin remote.

- **Inputs:** A volume created by `git.init`, then `git.remote_push`.
- **Outputs:** `ERR_INVALID_ARGUMENT` stating the volume has no `origin`. No network call.
- **Business Rules:** These tools take no `url` parameter. Remote management is out of scope.
- **Priority:** Must-have

#### FR-NEW-053 [EARS-U]: `auth` always reports the resolved provider
> The mcp-fs server SHALL set the `auth` field of every `git.remote_clone`, `git.remote_push`, `git.remote_fetch` and `git.remote_pull` response to the resolved provider string, one of `github`, `gitlab`, `generic` or `anonymous`.

- **Inputs:** A clone of `https://git.acme.internal/o/r.git`, declared `generic`, with a seeded token.
- **Outputs:** `"auth": "generic"`.
- **Business Rules:** `anonymous` is reported only for a host declared `anonymous`, never as a fallback (FR-NEW-007).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-007. The value was specified for `anonymous` and `github` only.

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run the suite through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly or with an ad hoc command. `--all-features` matters: without it the two optional relational drivers are never compiled.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| `alice@test.com` | Primary person fixture, used by every test below | fixture | ready |
| `bob@test.com` | Second person, for isolation assertions | fixture | ready |
| `proj-a` | Mount id used by every volume-scoped test | fixture | ready |
| `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH` | GitHub-shaped token fixture | fixture | ready |
| `glpat_1111222233334444555566667777` | GitLab-shaped token fixture | fixture | ready |
| Reference host map | `github.com: github`, `github.ibm.com: github`, `gitlab.acme.corp: gitlab`, `git.acme.internal: generic`, `public.example.org: anonymous` | fixture | ready |
| `git.unknown.test` | Host deliberately absent from the map | fixture | ready |

### The 19 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-051** — Clone records origin
- **Category:** Side Effect · **Scenario:** SC-004 · **Requirements:** FR-NEW-020 · **Priority:** Critical
- When `git.remote_clone` succeeds for `https://github.ibm.com/org/repo.git` into `proj-a`
- Then `list_remotes` for `proj-a` returns exactly `[("origin", "https://github.ibm.com/org/repo.git")]`
- *Today no production code calls `add_remote` (`crates/mcp-fs/src/git/db.rs:281`); this test fails before the change.*

**E2E-NEW-052** — Re-cloning replaces the origin row
- **Category:** State Transition · **Scenario:** SC-004 · **Requirements:** FR-NEW-020 · **Priority:** High
- Given `proj-a` cloned from `https://github.ibm.com/org/a.git`
- When it is cloned again from `https://github.ibm.com/org/b.git`
- Then `list_remotes` returns one row, `("origin", "https://github.ibm.com/org/b.git")`

**E2E-NEW-075** — Push without an origin is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-021 · **Priority:** Critical
- Given a volume created by `git.init` with a commit and no recorded remote
- When `git.remote_push` is called
- Then it fails stating the volume has no `origin` remote
- And no network call occurs

**E2E-NEW-080** — Push on a non-git volume is rejected
- **Category:** Error · **Scenario:** SC-009 · **Requirements:** FR-NEW-021 · **Priority:** High
- Given a volume never initialised as a repository
- When `git.remote_push` is called
- Then it fails cleanly, with no panic and no partial state

**E2E-NEW-089** — Fetch without an origin is rejected
- **Category:** Error · **Scenario:** SC-011 · **Requirements:** FR-NEW-021 · **Priority:** Critical
- Given a volume from `git.init`
- When `git.remote_fetch` is called
- Then it fails stating the volume has no `origin`

**E2E-NEW-124** — Pull without an origin is rejected
- **Category:** Error · **Scenario:** SC-012 · **Requirements:** FR-NEW-021 · **Priority:** Critical
- Given a volume from `git.init`
- When a pull is attempted
- Then it fails stating the volume has no `origin`

**E2E-NEW-142** — An HTTPS URL is accepted
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When a clone of `https://github.ibm.com/o/r.git` runs against a declared host
- Then the scheme check passes and the operation proceeds

**E2E-NEW-143** — An `ssh://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When a clone of `ssh://git@github.com/o/r.git` is attempted
- Then it fails with `ERR_INVALID_ARGUMENT` naming the scheme, with no network call

**E2E-NEW-144** — A `git://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When `git://github.com/o/r.git` is attempted
- Then it fails naming the scheme

**E2E-NEW-145** — A `file://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When `file:///etc/passwd` and `file:///tmp/repo` are attempted
- Then both fail, and no path on the server's filesystem is read
- *A `file://` remote would let a caller read the server's own disk.*

**E2E-NEW-146** — A plain `http://` URL is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** Critical
- When `http://github.com/o/r.git` is attempted
- Then it fails, because a token must never travel unencrypted

**E2E-NEW-147** — The `scp`-style shorthand is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-041 · **Priority:** High
- When `git@github.com:org/repo.git` is attempted
- Then it fails as not HTTPS, and is not silently reinterpreted as a hostname

**E2E-NEW-148** — A URL carrying userinfo is rejected
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-042 · **Priority:** Critical
- When `https://alice:ghp_secret@github.com/o/r.git` is attempted
- Then it fails with `ERR_INVALID_ARGUMENT`
- And the error message contains neither `ghp_secret` nor the full URL

**E2E-NEW-149** — A rejected userinfo URL is never stored as origin
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-042 · **Priority:** Critical
- When the clone of E2E-NEW-148 is attempted
- Then `list_remotes` for the volume returns no row containing `ghp_secret`

**E2E-NEW-150** — A rejected userinfo URL never reaches the audit log
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-042, FR-NEW-043 · **Priority:** Critical
- When the clone of E2E-NEW-148 is attempted
- Then no audit entry contains `ghp_secret`
- *The clone audit entry records the URL today (`crates/mcp-fs/src/tools/git.rs:794-800`), which is why this rejection must happen first.*

**E2E-NEW-194** — `auth` reports `generic`
- **Category:** Feature · **Scenario:** SC-004 · **Requirements:** FR-NEW-053 · **Priority:** Critical
- When a clone of `https://git.acme.internal/o/r.git` succeeds with a seeded token
- Then the response reports `"auth": "generic"`

**E2E-NEW-195** — `auth` is reported identically by all four operations
- **Category:** Edge · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-053 · **Priority:** High
- When clone, push, fetch and pull each run against `gitlab.acme.corp`
- Then all four report `"auth": "gitlab"`

**E2E-NEW-196** — `anonymous` is never reported as a fallback
- **Category:** Error · **Scenario:** SC-005 · **Requirements:** FR-NEW-053 · **Priority:** Critical
- When an operation targets an undeclared host, and separately a declared host with no token
- Then both fail, and neither response reports `"auth": "anonymous"`

**E2E-NEW-197** — The remote pipeline exists once
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-054 · **Priority:** Critical
- When the source tree is scanned after implementation
- Then `git2::RemoteCallbacks` is constructed in exactly one file, `crates/mcp-fs/src/git/remote.rs`
- And the entries of `git.hosts` are interpreted in exactly that one file, `config.rs` holding only the field declaration and a single call to `git::remote::validate_hosts` (FR-NEW-065)

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Do not leave a second copy of host resolution, URL validation or credential supply in `tools/git.rs` (FR-NEW-054).
- Do not include the URL in the error message when it carries userinfo (FR-NEW-042); name the host only.

### Scope Boundary
The module, URL validation, clone's migration onto it, `origin` persistence, and the no-origin guard. Do NOT implement push, fetch or pull (US-009 to US-013) and do NOT add the timeout (US-014).

## Non Regression

### Existing Tests That Must Pass
- A clone that succeeds today from a declared host still succeeds, still imports objects and refs, still sets `HEAD` and `refs/heads/{branch}`, and still reports `skipped` for per-file failures (`crates/mcp-fs/src/tools/git.rs:785-791`).
- Per-repository write serialization (`FR-609` of the git spec) is preserved and relied upon.
- The clone write-quota charge stays the whole tree (`crates/mcp-fs/src/tools/git.rs:778-782`): on a clone every file genuinely is new. Only pull uses the delta basis (US-012).
- The whole suite stays green: `cargo test --workspace` with `--all-features`, plus `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check`.

### Behaviors That Must Not Change
- Platform admin manages projects and membership and gains **no** implicit file or token access.
- `volume_id` scopes every row of `nodes`, `blob_refs` and `git_*`; it belongs in EVERY `WHERE` clause.
- Secrets come from the environment only. Never log a token or a key.
- A store never speaks a driver: it speaks `storage::rel::RelationalDb`. Never block a request thread on the database.

### API Contracts to Preserve
- `mount_id` stays required on every `fs.*` and `git.*` tool.
- Errors stay `ToolError::<code>` carrying a stable `ERR_*` from the closed set at `crates/mcp-fs/src/errors.rs:9-22`.
- Tool parameter names stay snake_case; parameter descriptions are frozen LLM-facing docs.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5. In addition, before declaring this story done:

1. **Spec compliance.** Every FR above is satisfied by code you can point at, and every test above passes by its own assertion, not by a weakened one.
2. **One implementation.** You added no second copy of host resolution, URL validation, credential supply or timeout. `git2::RemoteCallbacks` is still constructed in exactly one file; `git.hosts` is still read in exactly one file.
3. **No invented decision.** Every choice you made traces to a `DEC-` entry quoted above or to an explicit FR. If you had to decide something this story does not settle, you stopped and said so rather than choosing.
4. **No token anywhere.** No token value reaches a response, a log line, a tracing record, an audit entry, an error message or an HTTP body.
