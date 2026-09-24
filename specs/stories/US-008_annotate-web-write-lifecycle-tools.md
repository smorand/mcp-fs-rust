# US-008: Annotate web, write and lifecycle tool families

> Parent Spec: specs/2026-09-23_08-47-57-mcp-tool-annotation-hints.md
> Epic: n/a
> Status: ready
> Priority: 8
> Depends On: US-001
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective
Add the annotation builder chain to all 15 tool definitions in `web.rs`,
`write.rs` and `lifecycle.rs`, per the exact per-tool table below.

## Technical Context

### Stack
Rust 2024. Builder methods from US-001.

### Relevant File Structure
```
crates/mcp-fs/src/tools/web.rs
crates/mcp-fs/src/tools/write.rs
crates/mcp-fs/src/tools/lifecycle.rs
```

### Existing Patterns
Example, `web.download` (`web.rs`), overwrite/delete, open-world:
```rust
ToolSchema::new("web.download", ..)
    .req_str(...)
    .destructive(true)
    .read_only(false)
    .idempotent(true)
    .open_world(true),
```
`fs.delete` (`lifecycle.rs:39`), overwrite/delete, idempotent (trashing an
already-trashed path is a safe no-op):
```rust
ToolSchema::new("fs.delete", "Delete a path (moves to trash by default).")
    .req_str(...)
    .destructive(true)
    .read_only(false)
    .idempotent(true)
    .open_world(false),
```

### Data Model (excerpt)
None; builder calls only.

### Decisions That Govern This Story
- **DDEC-004** (quoted): "`fs.write`, `fs.write_bytes`, `fs.create_empty`,
  `fs.mkdir` and every other tool whose default parameters make a specific
  call non-destructive (e.g. `overwrite=false`, `exist_ok=false`) are still
  classified by their worst-case capability, not their default-argument
  behavior." Applies to `fs.write`, `fs.write_bytes`, `fs.copy`, `fs.move` in
  this story's table (all `destructiveHint=true` despite a no-clobber
  default).

### Applicable NFRs
None beyond DR-007.

### Bounded Context
`web.*` (external search/fetch/download via DuckDuckGo and raw HTTP),
`fs.write`/`fs.append`/`fs.create_empty`/`fs.write_bytes` (file creation and
content writes), `fs.mkdir`/`fs.delete`/`fs.move`/`fs.copy`/
`fs.list_allowed_roots`/`fs.audit_log` (path lifecycle and session
introspection).

## Functional Requirements

### DR-001 through DR-005: annotation classification rules (governing this story's table)
> DR-001: pure read ⇒ `read_only_hint=Some(true)`, no `destructive_hint`.
> DR-002: not pure read ⇒ `read_only_hint=Some(false)`.
> DR-003: Overwrite/delete ⇒ `destructive_hint=Some(true)` (worst case, per
> DDEC-004 for tools with a safe default argument).
> DR-004: Additive ⇒ `destructive_hint=Some(false)`.
> DR-005: `idempotent_hint`/`open_world_hint` follow the row exactly, no
> family-level default.

### Per-tool table (15 tools, copied verbatim from spec Section 3.1)

| Tool | Class | RO | D | I | OW |
|------|-------|----|----|----|----|
| `web.search` | Pure read | true | - | true | true |
| `web.news` | Pure read | true | - | true | true |
| `web.fetch` | Pure read | true | - | true | true |
| `web.suggestions` | Pure read | true | - | true | true |
| `web.download` | Overwrite/delete | false | true | true | true |
| `fs.write` | Overwrite/delete | false | true | false | false |
| `fs.append` | Additive | false | false | false | false |
| `fs.create_empty` | Additive | false | false | true | false |
| `fs.write_bytes` | Overwrite/delete | false | true | false | false |
| `fs.mkdir` | Additive | false | false | true | false |
| `fs.delete` | Overwrite/delete | false | true | true | false |
| `fs.move` | Overwrite/delete | false | true | false | false |
| `fs.copy` | Overwrite/delete | false | true | false | false |
| `fs.list_allowed_roots` | Pure read | true | - | true | false |
| `fs.audit_log` | Pure read | true | - | true | false |

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `web::register` (config-gated), `write`'s registration site, `lifecycle`'s registration site | in-repo | ready |

### E2E-US008-01: every web/write/lifecycle tool exposes the exact declared annotations
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-002, DR-003, DR-004, DR-005
- **Preconditions:** all 15 tools annotated per the table above.
- **Steps:** Given the registry with these three families registered (web
  feature enabled) / When `to_list_entry()` is read for each of the 15 names /
  Then each entry's `annotations` object matches its table row exactly.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file.

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not classify `fs.write`/`fs.write_bytes`/`fs.copy`/`fs.move` as
  `destructiveHint=false` on the reasoning that their default is no-clobber;
  DDEC-004 mandates the worst-case classification.
- Do not change any `name`, `description`, param, or handler body.

### Scope Boundary
- Only `web.rs`, `write.rs`, `lifecycle.rs`.

## Non Regression

### Existing Tests That Must Pass
- Every existing `#[test]` in these three files and in
  `crates/mcp-fs/src/tools/all.rs`, except `tool_contract_golden_is_current`,
  which stays **expected to fail** (per spec Section 7) until US-011.
- **The existing test suite passes unmodified**, except for the one
  documented, expected exception above. Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- Handler dispatch and runtime results for all 15 tools are unchanged (DR-007).

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 15 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
