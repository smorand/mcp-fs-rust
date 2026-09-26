# US-0007: Annotate search, search_semantic and sqlite tool families

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 7
> Depends On: US-0001
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective
Add the annotation builder chain to all 16 tool definitions in `search.rs`
(the `fs.glob`/`fs.grep`/`fs.find_definition`/`fs.find_references` family),
`search_semantic.rs` (the `search.*` family) and `sqlite.rs`, per the exact
per-tool table below.

## Technical Context

### Stack
Rust 2024. Builder methods from US-0001.

### Relevant File Structure
```
crates/mcp-fs/src/tools/search.rs
crates/mcp-fs/src/tools/search_semantic.rs
crates/mcp-fs/src/tools/sqlite.rs
```

### Existing Patterns
Example, `fs.glob` (`search.rs:20`), pure read, local:
```rust
ToolSchema::new("fs.glob", "Find files by glob pattern, newest first (cap 100).")
    .req_str(...)
    ...
    .read_only(true)
    .idempotent(true)
    .open_world(false),
```
`sqlite.import_csv` (`sqlite.rs:365-420`), additive, non-idempotent (confirmed
`CREATE TABLE IF NOT EXISTS` + `INSERT INTO`, never `DROP`/`DELETE`/
`TRUNCATE`/`REPLACE`, but a repeat call duplicates rows):
```rust
ToolSchema::new("sqlite.import_csv", ..)
    .req_str(...)
    .destructive(false)
    .read_only(false)
    .idempotent(false)
    .open_world(false),
```

### Data Model (excerpt)
None; builder calls only.

### Decisions That Govern This Story
- **DDEC-002** (quoted): `sqlite.import_csv` is classified
  `destructiveHint=false`, `idempotentHint=false`. "confirmed at
  `crates/mcp-fs/src/tools/sqlite.rs:365-420` the handler only ever executes
  `CREATE TABLE IF NOT EXISTS` and `INSERT INTO`, never
  `DROP`/`DELETE`/`TRUNCATE`/`REPLACE`; it cannot destroy pre-existing rows,
  but repeating it duplicates rows so it is not safe to retry blindly."

### Applicable NFRs
None beyond DR-007.

### Bounded Context
`fs.glob`/`fs.grep`/`fs.find_definition`/`fs.find_references` (code search,
local, in `search.rs`), `search.index`/`search.query`/`search.delete`/
`search.status` (the BM25/RAG index, in `search_semantic.rs`), `sqlite.*`
(SQLite database operations against a volume-stored file).

## Functional Requirements

### DR-001 through DR-005: annotation classification rules (governing this story's table)
> DR-001: pure read ⇒ `read_only_hint=Some(true)`, no `destructive_hint`.
> DR-002: not pure read ⇒ `read_only_hint=Some(false)`.
> DR-003: Overwrite/delete ⇒ `destructive_hint=Some(true)` (worst case).
> DR-004: Additive ⇒ `destructive_hint=Some(false)`.
> DR-005: `idempotent_hint`/`open_world_hint` follow the row exactly, no
> family-level default.

### Per-tool table (16 tools, copied verbatim from spec Section 3.1)

| Tool | Class | RO | D | I | OW |
|------|-------|----|----|----|----|
| `fs.glob` | Pure read | true | - | true | false |
| `fs.grep` | Pure read | true | - | true | false |
| `fs.find_definition` | Pure read | true | - | true | false |
| `fs.find_references` | Pure read | true | - | true | false |
| `search.index` | Additive | false | false | true | false |
| `search.query` | Pure read | true | - | true | false |
| `search.delete` | Overwrite/delete | false | true | true | false |
| `search.status` | Pure read | true | - | true | false |
| `sqlite.query` | Pure read | true | - | true | false |
| `sqlite.execute` | Overwrite/delete | false | true | false | false |
| `sqlite.list_tables` | Pure read | true | - | true | false |
| `sqlite.describe_table` | Pure read | true | - | true | false |
| `sqlite.list_indexes` | Pure read | true | - | true | false |
| `sqlite.vacuum` | Overwrite/delete | false | true | true | false |
| `sqlite.import_csv` | Additive | false | false | false | false |
| `sqlite.export_csv` | Pure read | true | - | true | false |

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `search::register` (config-gated), `search_semantic::register` (config-gated), `sqlite::register` (config-gated) | in-repo | ready |

### E2E-US007-01: every search/search_semantic/sqlite tool exposes the exact declared annotations
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-002, DR-003, DR-004, DR-005
- **Preconditions:** all 16 tools annotated per the table above.
- **Steps:** Given the registry with these three families registered (feature
  flags enabled) / When `to_list_entry()` is read for each of the 16 names /
  Then each entry's `annotations` object matches its table row exactly.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file.

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not classify `sqlite.import_csv` as `destructive_hint=true` because it
  writes data; DDEC-002 explicitly rules it non-destructive (never destroys
  pre-existing rows) but non-idempotent (duplicates on retry).
- Do not change any `name`, `description`, param, or handler body.

### Scope Boundary
- Only `search.rs`, `search_semantic.rs`, `sqlite.rs`.

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
- Handler dispatch and runtime results for all 16 tools are unchanged (DR-007).

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 16 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
