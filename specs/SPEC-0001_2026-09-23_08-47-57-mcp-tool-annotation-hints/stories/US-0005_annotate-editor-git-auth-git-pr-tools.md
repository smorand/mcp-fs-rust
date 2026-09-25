# US-0005: Annotate editor, git_auth and git_pr tool families

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 5
> Depends On: US-0001
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective
Add the annotation builder chain to all 13 tool definitions in `editor.rs`,
`git_auth.rs` and `git_pr.rs`, per the exact per-tool table below.

## Technical Context

### Stack
Rust 2024. Builder methods from US-0001.

### Relevant File Structure
```
crates/mcp-fs/src/tools/editor.rs
crates/mcp-fs/src/tools/git_auth.rs
crates/mcp-fs/src/tools/git_pr.rs
```

### Existing Patterns
Same `reg.add(ToolSchema::new(...).chain(...), handler(...))` pattern. Example,
`doc.list_editors` (`editor.rs:153`), a no-param pure-read tool:
```rust
ToolSchema::new("doc.list_editors", "List all active HTML editors and their URLs.")
    .read_only(true)
    .idempotent(true)
    .open_world(false),
```
`git.pr_merge` (`git_pr.rs:223-285`), open-world, non-idempotent:
```rust
ToolSchema::new("git.pr_merge", ..)
    .req_str(...)
    .destructive(true)
    .read_only(false)
    .idempotent(false)
    .open_world(true),
```

### Data Model (excerpt)
None; builder calls only.

### Decisions That Govern This Story
None invented beyond what the table encodes directly from Section 3.1's
per-tool citations (e.g. `git.auth_status` is local-only, `OW=false`, unlike
the three `git.pr_*` read tools which reach the provider's REST API,
`OW=true`).

### Applicable NFRs
None beyond DR-007.

### Bounded Context
`doc.open_editor`/`doc.close_editor`/`doc.list_editors` (HTML editor sessions,
local), `git.auth*` (OAuth device flow and token store, per-host), `git.pr_*`
(the normalized pull-request surface, reaches a provider API).

## Functional Requirements

### DR-001 through DR-005: annotation classification rules (governing this story's table)
> DR-001: pure read ⇒ `read_only_hint=Some(true)`, no `destructive_hint`.
> DR-002: not pure read ⇒ `read_only_hint=Some(false)`.
> DR-003: Overwrite/delete ⇒ `destructive_hint=Some(true)` (worst case).
> DR-004: Additive ⇒ `destructive_hint=Some(false)`.
> DR-005: `idempotent_hint`/`open_world_hint` follow the row exactly, no
> family-level default.

### Per-tool table (13 tools, copied verbatim from spec Section 3.1)

| Tool | Class | RO | D | I | OW |
|------|-------|----|----|----|----|
| `doc.open_editor` | Additive | false | false | false | false |
| `doc.close_editor` | Additive | false | false | true | false |
| `doc.list_editors` | Pure read | true | - | true | false |
| `git.auth` | Additive | false | false | false | true |
| `git.auth_status` | Pure read (local) | true | - | true | false |
| `git.auth_revoke` | Overwrite/delete | false | true | true | false |
| `git.token_set` | Additive | false | false | true | false |
| `git.pr_create` | Additive | false | false | false | true |
| `git.pr_list` | Pure read | true | - | true | true |
| `git.pr_get` | Pure read | true | - | true | true |
| `git.pr_diff` | Pure read | true | - | true | true |
| `git.pr_merge` | Overwrite/delete | false | true | false | true |
| `git.pr_review` | Additive | false | false | false | true |

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `editor::register`, `git_auth::register`, `git_pr::register` | in-repo | ready |

### E2E-US005-01: every editor/git_auth/git_pr tool exposes the exact declared annotations
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-002, DR-003, DR-004, DR-005
- **Preconditions:** all 13 tools annotated per the table above.
- **Steps:** Given the registry with these families registered / When
  `to_list_entry()` is read for each of the 13 names / Then each entry's
  `annotations` object matches its table row exactly.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file, in particular not `git.rs` (its own story,
  US-0009).

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not classify `git.auth_status` as `OW=true` by analogy with the other
  `git.pr_*`/`git.auth` network calls; it reads only the local token store
  (Section 3.1: "10 git, local").
- Do not change any `name`, `description`, param, or handler body.

### Scope Boundary
- Only `editor.rs`, `git_auth.rs`, `git_pr.rs`.

## Non Regression

### Existing Tests That Must Pass
- Every existing `#[test]` in these three files and in
  `crates/mcp-fs/src/tools/all.rs`, except `tool_contract_golden_is_current`,
  which stays **expected to fail** (per spec Section 7) until US-0011.
- **The existing test suite passes unmodified**, except for the one
  documented, expected exception above. Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- Handler dispatch and runtime results for all 13 tools are unchanged (DR-007).

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 13 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
