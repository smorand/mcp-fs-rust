# US-0004: Annotate doc, document and edit tool families

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 4
> Depends On: US-0001
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective
Add the annotation builder chain to all 10 tool definitions in `doc.rs`,
`document.rs` and `edit.rs`, per the exact per-tool table below.

## Technical Context

### Stack
Rust 2024. Builder methods from US-0001.

### Relevant File Structure
```
crates/mcp-fs/src/tools/doc.rs
crates/mcp-fs/src/tools/document.rs
crates/mcp-fs/src/tools/edit.rs
```

### Existing Patterns
Same `reg.add(ToolSchema::new(...).chain(...), handler(...))` pattern as
US-0003. Example, `doc.to_docx` (`doc.rs:56`):
```rust
ToolSchema::new("doc.to_docx", DOCX_DESC)
    .req_str(...)
    ...
    .destructive(true)
    .read_only(false)
    .idempotent(true)
    .open_world(true),
```
`fs.edit` (`edit.rs:24`), an overwrite/delete tool with `I=false`:
```rust
ToolSchema::new("fs.edit", "Replace a unique string; dry_run returns the diff.")
    .req_str(...)
    ...
    .destructive(true)
    .read_only(false)
    .idempotent(false)
    .open_world(false),
```

### Data Model (excerpt)
None; builder calls only.

### Decisions That Govern This Story
- **DDEC-001** (quoted): `openWorldHint` for `doc.to_docx`, `doc.to_pptx`,
  `fs.documentize`, `fs.extract_text` is declared `true` unconditionally, even
  though the underlying `doc_service` may be configured as a local CLI
  (closed-world) rather than an HTTP endpoint (open-world). "A hint describes
  the tool's worst-case capability across all valid server configurations, not
  the specific deployment's current config." This applies to all four tools in
  this story's table below marked `OW=true`.

### Applicable NFRs
None beyond DR-007.

### Bounded Context
`doc.*` document conversion (docx/pptx), `fs.write_docx`/`fs.documentize`/
`fs.extract_text` (document generation and extraction), `fs.edit`/
`fs.apply_patch`/`fs.multi_edit`/`fs.insert_at_line`/`fs.search_replace`
(text-editing family).

## Functional Requirements

### DR-001 through DR-005: annotation classification rules (governing this story's table)
> DR-001: pure read ⇒ `read_only_hint=Some(true)`, no `destructive_hint`.
> DR-002: not pure read ⇒ `read_only_hint=Some(false)`.
> DR-003: Overwrite/delete ⇒ `destructive_hint=Some(true)` (worst case).
> DR-004: Additive ⇒ `destructive_hint=Some(false)`.
> DR-005: `idempotent_hint`/`open_world_hint` follow the row exactly, no
> family-level default.

### Per-tool table (10 tools, copied verbatim from spec Section 3.1)

| Tool | Class | RO | D | I | OW |
|------|-------|----|----|----|----|
| `doc.to_docx` | Overwrite/delete | false | true | true | true (DDEC-001) |
| `doc.to_pptx` | Overwrite/delete | false | true | true | true (DDEC-001) |
| `fs.write_docx` | Overwrite/delete | false | true | false | false |
| `fs.documentize` | Additive | false | false | true | true (DDEC-001) |
| `fs.extract_text` | Additive | false | false | true | true (DDEC-001) |
| `fs.edit` | Overwrite/delete | false | true | false | false |
| `fs.multi_edit` | Overwrite/delete | false | true | false | false |
| `fs.search_replace` | Overwrite/delete | false | true | false | false |
| `fs.insert_at_line` | Overwrite/delete | false | true | false | false |
| `fs.apply_patch` | Overwrite/delete | false | true | false | false |

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `doc::register`, `document`'s registration site, `edit`'s registration site | in-repo | ready |

### E2E-US004-01: every doc/document/edit tool exposes the exact declared annotations
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-002, DR-003, DR-004, DR-005
- **Preconditions:** all 10 tools annotated per the table above.
- **Steps:** Given the registry with these families registered / When
  `to_list_entry()` is read for each of the 10 names / Then each entry's
  `annotations` object matches its table row exactly.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file.

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not classify `fs.write_docx` as Additive by analogy with `fs.documentize`;
  it overwrites an existing document (Overwrite/delete per the table), unlike
  the two DDEC-001 additive extraction tools.
- Do not change any `name`, `description`, param, or handler body.

### Scope Boundary
- Only `doc.rs`, `document.rs`, `edit.rs`.

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
- Handler dispatch and runtime results for all 10 tools are unchanged (DR-007).

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 10 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
