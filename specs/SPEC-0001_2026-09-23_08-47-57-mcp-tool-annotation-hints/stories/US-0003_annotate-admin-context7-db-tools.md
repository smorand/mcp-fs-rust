# US-0003: Annotate admin, context7 and db tool families

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 3
> Depends On: US-0001
> Complexity: M
> min_tier: 2
> Files touched: 3

## Objective
Add the annotation builder chain to all 17 tool definitions in
`admin.rs`, `context7.rs` and `db.rs`, per the exact per-tool table below. No
`name`, `description` or `inputSchema` value changes; only new builder calls
are appended to each existing `ToolSchema::new(...)` chain.

## Technical Context

### Stack
Rust 2024. Uses the four builder methods added in US-0001
(`crates/mcp-fs/src/mcp/schema.rs`): `.read_only(bool)`, `.destructive(bool)`,
`.idempotent(bool)`, `.open_world(bool)`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/admin.rs
crates/mcp-fs/src/tools/context7.rs
crates/mcp-fs/src/tools/db.rs
```

### Existing Patterns
Every tool is registered with `reg.add(ToolSchema::new(name, desc).req_str(...)....(chain), handler(...))`.
Example, `admin.create_project` (`admin.rs:44-58`):
```rust
reg.add(
    ToolSchema::new(
        "admin.create_project",
        "Create a project for a designated owner and provision its volume (platform admin only).",
    )
    .req_str(
        "project_id",
        "New project id: 3 to 32 chars, lowercase letters, digits, hyphens, alphanumeric bounds.",
    )
    .req_str("owner", "Person id who owns the new project."),
    handler(|ctx: ToolCtx, a| async move {
        ...
    }),
);
```
Append the annotation chain immediately after the last existing builder call,
before the trailing comma that separates the schema from the `handler(...)`
argument:
```rust
    .req_str("owner", "Person id who owns the new project.")
    .read_only(false)
    .destructive(false)
    .idempotent(false)
    .open_world(false),
    handler(|ctx: ToolCtx, a| async move {
```
For a pure-read tool with no params (`admin.list_projects`,
`admin.rs:76`), append the chain directly to `ToolSchema::new(...)`:
```rust
ToolSchema::new("admin.list_projects", "List projects the caller can access.")
    .read_only(true)
    .idempotent(true)
    .open_world(false),
```
(no `.destructive(..)` call for pure-read tools, per DR-001.)

### Data Model (excerpt)
No data model change; this story only calls builder methods added in US-0001.

### Decisions That Govern This Story
None invented. DDEC-001 (open-world is worst-case, static per tool, not
per-config) governs `db.*`'s `open_world(true)` classification below, quoted
from the spec: "a hint describes the tool's worst-case capability across all
valid server configurations, not the specific deployment's current config."

### Applicable NFRs
None beyond DR-007 (behavior invariance).

### Bounded Context
`admin.*` (project lifecycle and membership), `context7.*` (external docs
lookup), `db.*` (CSV/Parquet/JSON data-file operations). No cross-family
interaction; annotate independently.

## Functional Requirements

### DR-001 through DR-005: annotation classification rules (governing this story's table)
> DR-001: The `ToolSchema` for every tool classified "pure read" SHALL set
> `read_only_hint` to `Some(true)` and SHALL NOT set `destructive_hint`.
> DR-002: The `ToolSchema` for every tool NOT classified "pure read" SHALL set
> `read_only_hint` to `Some(false)`.
> DR-003: The `ToolSchema` for every tool classified "Overwrite/delete" SHALL
> set `destructive_hint` to `Some(true)`, reflecting worst-case capability.
> DR-004: The `ToolSchema` for every tool classified "Additive" SHALL set
> `destructive_hint` to `Some(false)`.
> DR-005: The `ToolSchema` for every tool SHALL set `idempotent_hint` and
> `open_world_hint` to the exact `Some(bool)` value given in its row below, with
> no default inferred from its family.

### Per-tool table (17 tools, copied verbatim from spec Section 3.1)

| Tool | Class | RO | D | I | OW |
|------|-------|----|----|----|----|
| `admin.create_project` | Additive | false | false | false | false |
| `admin.delete_project` | Overwrite/delete | false | true | true | false |
| `admin.list_projects` | Pure read | true | - | true | false |
| `admin.list_all_projects` | Pure read | true | - | true | false |
| `admin.list_users` | Pure read | true | - | true | false |
| `admin.add_member` | Additive | false | false | true | false |
| `admin.remove_member` | Overwrite/delete | false | true | true | false |
| `admin.list_members` | Pure read | true | - | true | false |
| `admin.set_index_mode` | Overwrite/delete | false | true | true | false |
| `admin.get_index_mode` | Pure read | true | - | true | false |
| `context7.resolve_library_id` | Pure read | true | - | true | true |
| `context7.get_library_docs` | Pure read | true | - | true | true |
| `db.query` | Pure read | true | - | true | true |
| `db.schema` | Pure read | true | - | true | true |
| `db.profile` | Pure read | true | - | true | true |
| `db.sample` | Pure read | true | - | true | true |
| `db.convert` | Overwrite/delete | false | true | true | true |

"-" means the `destructive_hint` builder is not called at all (per DR-001).

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `admin::register`, `context7::register`, `db::register` | in-repo | ready |

### E2E-US003-01: every admin/context7/db tool exposes the exact declared annotations
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-002, DR-003, DR-004, DR-005
- **Preconditions:** all 17 tools annotated per the table above.
- **Steps:** Given a `ToolRegistry` with `admin::register`, `context7::register`
  (config default), `db::register` (config default) called / When
  `to_list_entry()` is read for each of the 17 tool names / Then each entry's
  `annotations` object matches the table exactly (e.g.
  `reg.resolve("admin.delete_project").unwrap().schema.to_list_entry()["annotations"]
  == json!({"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false})`).
- **Cleanup:** none.
- **Priority:** Critical

This is verified manually per-tool during implementation (compile + read the
generated JSON via a scratch `cargo test -p mcp-fs --lib tools::admin` run);
the durable, committed assertion of "every tool has some annotation" is DT-003
in US-0010, and the durable per-tool value assertion is the regenerated golden
file in US-0011. This story's own gate is: it compiles, and
`tool_contract_golden_is_current` legitimately turns red (expected, see Non
Regression) because these 17 tools now carry annotations the golden file does
not yet have.

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file.
- `contract_golden.rs`, `all.rs`: untouched (already prepared by US-0001/US-0002).

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not change any `name`, `description`, `req_str`/`opt_*` param call, or
  handler body in these three files.
- Do not infer `open_world_hint` from "this tool touches an external service"
  as a blanket rule; always follow the table row.

### Scope Boundary
- Only `admin.rs`, `context7.rs`, `db.rs`. No other tool family.

## Non Regression

### Existing Tests That Must Pass
- Every existing `#[test]` in `admin.rs`, `context7.rs`, `db.rs` and in
  `crates/mcp-fs/src/tools/all.rs` except `tool_contract_golden_is_current`,
  which is **expected to fail** starting with this story (per spec Section 7:
  "the registry has annotations the golden file does not yet have") until
  US-0011 regenerates the golden file.
- **The existing test suite passes unmodified**, except for the one
  documented, expected exception above. Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- Handler dispatch, authorization checks, and runtime results for all 17
  tools are unchanged (DR-007).

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 17 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
