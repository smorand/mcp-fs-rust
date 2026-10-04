# US-0006: Migrate `git.auth*` (4) + `git.pr_*` (6) tools to `#[tool]` methods

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 6
> Depends On: US-0002
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective
Re-express the 4 `git.auth*` (OAuth/PAT token store) tools and the 6 `git.pr_*` (pull request)
tools as `#[tool]` methods on `McpServer`, calling the same `git/oauth/` and `git/provider/` code
the old handlers called.

## Technical Context

### Stack
Rust 2024, `rmcp` 3.5.0 `#[tool]` macros, `jsonwebtoken` + `rsa` + `aes-gcm` (OAuth store,
unchanged).

### Relevant File Structure
```
crates/core/src/mcp/server.rs       # extended with git.auth* and git.pr_* #[tool] methods
crates/core/src/tools/git_auth.rs   # reference only, not modified
crates/core/src/tools/git_pr.rs
crates/core/src/git/oauth/          # store, cipher, device flow — called, not modified
crates/core/src/git/provider/       # host->provider resolution, ProviderClient — called, not modified
```

### Existing Patterns
Follow `crates/core/src/tools/git_auth.rs` and `git_pr.rs`'s existing `register()` functions. The
`git.pr_*` tools return the ONE normalized pull request object from `git/provider/model.rs`
(project convention, `AGENTS.md`) — preserve that exactly, never a provider-specific shape.

### Decisions That Govern This Story
- **DR-002** (git.auth*+git.pr_* subset, same requirement text as the prior migration stories,
  scoped to these 10 tools).
- **DR-007**: no individual tool's behavior, authorization check, token handling, or return shape
  changes.

### Applicable NFRs
DR-006 (restated, per Invariant 3): byte-identical observable output once wired by US-0008.
Secrets come from the environment only; never log a token or a key (project convention,
`AGENTS.md`) — this applies unchanged to the new dispatch wrapper.

### Bounded Context
`git.auth*`: per-person+host OAuth/PAT token store (`git/oauth/`). `git.pr_*`: the pull-request
seam (`git/provider/`), host-to-provider resolution, injectable `ProviderClient`.

## Functional Requirements

### DR-002 (git.auth*+git.pr_* subset)
- **EARS:** see US-0003's DR-002 text, scoped here to the 4 `git.auth*` and 6 `git.pr_*` tool
  names in `TOOL_CONTRACT.txt`.
- **Inputs / Outputs:** same parameter sets and return shapes as the existing `tools/git_auth.rs`
  and `tools/git_pr.rs` handlers, including the normalized pull request object shape for every
  `git.pr_*` tool.
- **Business Rules:** dialect binary typing for the OAuth token (project convention, unchanged);
  no token or key ever logged.

## Acceptance Tests

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Existing OAuth/provider fixtures | test token store entries, mocked `ProviderClient` responses already used by `tools/git_auth.rs`/`git_pr.rs` tests | existing | ready |

### E2E-GITAUTH-001: git.auth* and git.pr_* `#[tool]` methods dispatch identically to the old handlers
- **Category:** happy
- **Requirements:** DR-002, DR-007
- **Preconditions:** `McpServer` compiled with the 10 new methods present.
- **Steps:** Given the same fixtures an existing `tools/git_auth.rs`/`git_pr.rs` test uses; When
  the new `#[tool]` method is called directly with the same arguments; Then the result matches the
  old handler's existing asserted result exactly, including the normalized PR object shape.
- **Cleanup:** existing fixture teardown.
- **Priority:** Critical.

### E2E-GITAUTH-002: No secret is logged by the new dispatch wrapper
- **Category:** edge
- **Requirements:** DR-002, DR-007
- **Preconditions:** a test OAuth token value distinguishable in logs if leaked.
- **Steps:** Given a `git.auth*` `#[tool]` method call with a real token in its arguments; When the
  call completes (success or error); Then no log line emitted during the call contains the token
  value, verified by grepping captured tracing output for the literal token string.
- **Cleanup:** none.
- **Priority:** High.

### E2E-GITAUTH-003: Tool names and descriptions preserved verbatim
- **Category:** happy
- **Requirements:** DR-002
- **Preconditions:** `McpServer`'s router introspectable.
- **Steps:** Given the router; When queried for the 4 `git.auth*` and 6 `git.pr_*` entries; Then
  each `name` and `description` matches `TOOL_CONTRACT.txt` exactly.
- **Cleanup:** none.
- **Priority:** Critical.

## Constraints

### Files Not to Touch
`crates/core/src/tools/git_auth.rs`, `git_pr.rs`, `crates/core/src/git/oauth/*`,
`crates/core/src/git/provider/*` (reference only); `crates/core/src/mcp/{mod,registry,schema,args}.rs`;
`crates/core/src/app.rs`.

### Dependencies Not to Add
None.

### Patterns to Avoid
Returning a provider-specific pull-request shape instead of the normalized
`git/provider/model.rs` object; logging any token or key value.

### Scope Boundary
No transport change, no deletion.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` — unaffected.

### Behaviors That Must Not Change
DR-006, DR-007 (restated per Invariant 3, see US-0003).

### API Contracts to Preserve
`TOOL_CONTRACT.txt` entries for the 4 `git.auth*` and 6 `git.pr_*` tools.

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5.
