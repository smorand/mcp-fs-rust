# US-006: Annotate listing, metadata and read tool families

> Parent Spec: specs/2026-09-23_08-47-57-mcp-tool-annotation-hints.md
> Epic: n/a
> Status: ready
> Priority: 6
> Depends On: US-001
> Complexity: S
> min_tier: 2
> Files touched: 3

## Objective
Add the annotation builder chain to all 13 tool definitions in `listing.rs`,
`metadata.rs` and `read.rs`. Every tool in this story is "pure read", making
this the simplest of the family stories: one uniform annotation shape,
`.read_only(true).idempotent(true).open_world(false)`, no `.destructive(..)`
call.

## Technical Context

### Stack
Rust 2024. Builder methods from US-001.

### Relevant File Structure
```
crates/mcp-fs/src/tools/listing.rs
crates/mcp-fs/src/tools/metadata.rs
crates/mcp-fs/src/tools/read.rs
```

### Existing Patterns
Example, `fs.list_dir` (`listing.rs:15`):
```rust
ToolSchema::new("fs.list_dir", "Flat directory listing with kinds and optional sizes.")
    .req_str(...)
    ...
    .read_only(true)
    .idempotent(true)
    .open_world(false),
```
No `.destructive(..)` call anywhere in these three files, per DR-001.

### Data Model (excerpt)
None; builder calls only.

### Decisions That Govern This Story
None invented.

### Applicable NFRs
None beyond DR-007.

### Bounded Context
`fs.*` read-only surface: directory listing (`fs.list_dir`, `fs.tree`),
metadata probes (`fs.stat`, `fs.exists`, `fs.hash`), and content reads
(`fs.read`, `fs.read_bytes`, `fs.read_lines`, `fs.read_section`,
`fs.read_many`, `fs.head`, `fs.tail`, `fs.count_lines`).

## Functional Requirements

### DR-001 and DR-005: annotation classification rules (governing this story's table)
> DR-001: The `ToolSchema` for every tool classified "pure read" SHALL set
> `read_only_hint` to `Some(true)` and SHALL NOT set `destructive_hint`.
> DR-005: `idempotent_hint`/`open_world_hint` follow the row exactly, no
> family-level default.

(DR-002, DR-003, DR-004 do not apply: every tool in this story is pure read.)

### Per-tool table (13 tools, copied verbatim from spec Section 3.1, all Pure read)

| Tool | RO | D | I | OW |
|------|----|----|----|----|
| `fs.list_dir` | true | - | true | false |
| `fs.tree` | true | - | true | false |
| `fs.stat` | true | - | true | false |
| `fs.exists` | true | - | true | false |
| `fs.hash` | true | - | true | false |
| `fs.read` | true | - | true | false |
| `fs.read_bytes` | true | - | true | false |
| `fs.read_lines` | true | - | true | false |
| `fs.read_section` | true | - | true | false |
| `fs.read_many` | true | - | true | false |
| `fs.head` | true | - | true | false |
| `fs.tail` | true | - | true | false |
| `fs.count_lines` | true | - | true | false |

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `fs.*` registration in these three files | in-repo | ready |

### E2E-US006-01: every listing/metadata/read tool exposes `readOnlyHint=true`, `idempotentHint=true`, `openWorldHint=false`, no `destructiveHint`
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-005
- **Preconditions:** all 13 tools annotated per the table above.
- **Steps:** Given the registry with these three families registered / When
  `to_list_entry()` is read for each of the 13 names / Then each entry's
  `annotations` equals exactly
  `{"readOnlyHint": true, "idempotentHint": true, "openWorldHint": false}`,
  with no `destructiveHint` key.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file, in particular not `search.rs` (US-007), which
  also has pure-read `fs.*` tools (`fs.glob`, `fs.grep`,
  `fs.find_definition`, `fs.find_references`) but is a separate story.

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not call `.destructive(false)` on any tool in this story; DR-001 says
  pure-read tools SHALL NOT set `destructive_hint` at all.
- Do not change any `name`, `description`, param, or handler body.

### Scope Boundary
- Only `listing.rs`, `metadata.rs`, `read.rs`.

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
- Handler dispatch and runtime results for all 13 tools are unchanged (DR-007).

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 13 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
