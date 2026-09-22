# US-018: Every remote operation is audited and traced, and no token is ever emitted

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 18
> Depends On: US-008, US-009, US-010, US-011
> Complexity: M
> min_tier: 1
> Files touched: 2

## Objective

Every remote operation, successful or not, leaves one audit entry and one tracing span naming the operation, host, branch and outcome. And the redaction discipline is made total: no token value reaches a tool response, a tracing record, an audit entry, an error message, or an HTTP body.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/remote.rs  # one span and one audit entry per operation
crates/mcp-fs/src/safety.rs  # record_audit, and the capped audit log
```

### Existing Patterns
- `safety.record_audit` is the existing audit path; the clone entry already records the URL (`crates/mcp-fs/src/tools/git.rs:794-800`), which stays correct only because US-008 rejects userinfo-bearing URLs.
- The redaction discipline already covers `Debug` output (`crates/mcp-fs/src/git/oauth/store.rs:41`, with its test at `:285`). This story extends it to every emitted surface.
- Span naming follows the existing tracing conventions in the tree.

### Data Model (excerpt)
- No new entity.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a

### Applicable NFRs
- **What to never trace** (§7.5): token values, the `Authorization` header, userinfo from any URL, and any credential surfaced in a libgit2 error string.
- **Audit** (§7.5): every remote operation records an entry naming the operation, host, branch and outcome.

### Bounded Context
**Remote Operations** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-057 [EARS-E]: Every remote operation records an audit entry
> WHEN `git.remote_clone`, `git.remote_push`, `git.remote_fetch` or `git.remote_pull` completes, successfully or not, THE mcp-fs server SHALL record one entry through `safety.record_audit` naming the tool, the host, the branch when one is named, and the outcome.

- **Inputs:** A successful push of `main` to `github.ibm.com`, then a push refused as non-fast-forward.
- **Outputs:** Two audit entries, the second recording the refusal.
- **Business Rules:** The entry never contains a token or a userinfo-bearing URL (FR-NEW-042, FR-NEW-043).
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-013. A persisted, user-visible side effect stated only in a non-functional prose bullet, with no requirement and no test behind it.

#### FR-NEW-072 [EARS-E]: Every remote operation emits one tracing span
> WHEN `git.remote_clone`, `git.remote_push`, `git.remote_fetch` or `git.remote_pull` runs, THE mcp-fs server SHALL emit exactly one tracing span named `git.remote`, carrying the fields `operation`, `host`, `provider`, `branch` when the tool names one, `outcome` of `ok` or `error`, and `duration_ms`.

- **Inputs:** A successful push of `main` to `github.ibm.com`; a clone refused for an undeclared host.
- **Outputs:** One span per call, the second with `outcome=error`. Neither span carries a token, an `Authorization` value, or URL userinfo.
- **Business Rules:** The span is emitted for a failure resolved before any network call as well as for a completed operation, so the refusal paths of FR-NEW-007, FR-NEW-049 and FR-NEW-070 are all observable. Content is constrained by FR-NEW-043.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F5 of round 3. The span was required only in a non-functional prose bullet, the same shape as the audit-entry bullet that became FR-NEW-057.

#### FR-NEW-043 [EARS-UB]: A token never appears on any observable surface
> The mcp-fs server SHALL NOT include a token value in a tool response, a tracing record, an audit entry, an error message, or an HTTP response body.

- **Inputs:** Every operation in this specification, including failures.
- **Outputs:** No emitted string contains a stored token.
- **Business Rules:** Extends the existing redaction discipline, which already covers `Debug` output (`crates/mcp-fs/src/git/oauth/store.rs:41`, test at `:285`).
- **Priority:** Must-have

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

### The 9 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-056** — Clone audit records the operation without the credential
- **Category:** Side Effect · **Scenario:** SC-004 · **Requirements:** FR-NEW-043 · **Priority:** Critical
- When a clone succeeds using a stored token
- Then an audit entry exists naming the operation, the mount and the URL
- And the entry contains no substring of the token

**E2E-NEW-151** — Tracing output never contains a token
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** Critical
- Given a tracing subscriber capturing every event at TRACE level
- When clone, push, fetch and pull each run successfully with a stored token
- Then no captured event contains any substring of length 8 or more from that token

**E2E-NEW-152** — Error messages never contain a token
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** Critical
- When each failure mode in this specification is triggered with a token present
- Then no error message contains any substring of the token, including messages originating in libgit2

**E2E-NEW-153** — Audit entries never contain a token
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** Critical
- When every remote operation runs
- Then each records an audit entry naming the operation, host, branch and outcome
- And none contains a token value

**E2E-NEW-154** — `Debug` output stays redacted under the new key
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-043 · **Priority:** High
- When an `OAuthSession` is formatted with `{:?}`
- Then the token is redacted, preserving the behaviour at `crates/mcp-fs/src/git/oauth/store.rs:41`

**E2E-NEW-200** — Each remote operation records one audit entry
- **Category:** Side Effect · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-057 · **Priority:** Critical
- When a push of `main` to `github.ibm.com` succeeds, then a second push is refused as non-fast-forward
- Then two audit entries exist, each naming the tool, host, branch and outcome, the second recording the refusal

**E2E-NEW-201** — Fetch and pull record audit entries too
- **Category:** Error · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-057 · **Priority:** High
- When a fetch succeeds and a pull is refused as dirty
- Then both are recorded, the refusal included

**E2E-NEW-202** — No audit entry carries a credential
- **Category:** Security · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-057 · **Priority:** Critical
- When every remote operation runs with a stored token
- Then no audit entry contains any substring of the token or a userinfo-bearing URL

**E2E-NEW-247** — Every remote operation emits exactly one span
- **Category:** Side Effect · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-072 · **Priority:** Critical
- Given a tracing subscriber capturing spans
- When a push succeeds and a clone is refused for an undeclared host
- Then exactly two `git.remote` spans exist, carrying `operation`, `host`, `provider`, `outcome` and `duration_ms`, the second with `outcome=error`
- And neither span contains a token, an `Authorization` value or URL userinfo

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Never emit a span or audit entry containing a token or a userinfo-bearing URL (FR-NEW-042, FR-NEW-043).
- Never skip the span on a failure resolved before any network call: the refusal paths of FR-NEW-007, FR-NEW-049 and FR-NEW-070 must all be observable (FR-NEW-072).

### Scope Boundary
Audit entries, the tracing span, and total token redaction across every surface.

## Non Regression

### Existing Tests That Must Pass
- The existing clone audit entry keeps recording what it records today, plus the fields this story fixes.
- The `Debug` redaction test at `crates/mcp-fs/src/git/oauth/store.rs:285` still passes.
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
