# US-0002: Contract golden annotation awareness

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 2
> Depends On: US-0001
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
Make the frozen tool contract machinery (`render()` and `assert_family()` in
`contract_golden.rs`) aware of the new `annotations` field, so that once tool
families start declaring hints (starting at US-0003), the existing
`tool_contract_golden_is_current` test drifts honestly instead of silently
ignoring the new data. At the end of this story, no tool yet has annotations,
so the golden file is still unchanged and the test still passes.

## Technical Context

### Stack
Rust 2024, `serde_json`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/contract_golden.rs
```

### Existing Patterns
`render()` builds one JSON object per tool (`contract_golden.rs`, function
`render`):
```rust
fn render(reg: &ToolRegistry) -> String {
    let mut names: Vec<&str> = reg.names().to_vec();
    names.sort_unstable();
    let tools: Vec<Value> = names
        .iter()
        .map(|name| {
            let tool = reg.resolve(name).expect("a listed name resolves");
            let mut entry = Map::new();
            entry.insert("name".into(), Value::String((*name).to_string()));
            entry.insert("description".into(), Value::String(tool.schema.description.clone()));
            entry.insert("inputSchema".into(), tool.schema.input_schema());
            Value::Object(entry)
        })
        .collect();
    ...
}
```
`assert_family()` compares each frozen entry to the live registry field by
field (`description`, `input_schema()`, then the serialized string for key
order):
```rust
assert_eq!(mine.schema.description, tool["description"].as_str().unwrap(), "description drift on {name}");
assert_eq!(mine.schema.input_schema(), tool["inputSchema"], "schema drift on {name}");
assert_eq!(
    serde_json::to_string(&mine.schema.input_schema()).unwrap(),
    serde_json::to_string(&tool["inputSchema"]).unwrap(),
    "property key order drift on {name}"
);
```

### Data Model (excerpt)
`render()`'s per-tool `Map` gains one more optional key, using
`ToolSchema::to_list_entry()` added in US-0001 as the source of truth rather
than re-deriving the shape by hand:
```rust
if let Some(ann) = tool.schema.to_list_entry().get("annotations") {
    entry.insert("annotations".into(), ann.clone());
}
```

### Decisions That Govern This Story
None beyond DR-008.

### Applicable NFRs
None beyond DR-007 (behavior invariance), restated in Non Regression.

### Bounded Context
Test-only access to the frozen contract file
(`crates/mcp-fs/src/tools/contract_golden.rs`). No production dispatch path is
touched.

## Functional Requirements

### DR-008: Contract regeneration covers annotations
- **EARS:** The `render()` function in `contract_golden.rs` SHALL include the
  `annotations` field (when present) in every rendered tool entry, and the
  `tool_contract_golden_is_current` test SHALL fail when a registered tool's
  annotations differ from the frozen `tool-contract-golden.json` entry.
- **Inputs / Outputs:** Before any tool declares annotations (this story's
  end state): `render()`'s output is byte-identical to before this change,
  because `to_list_entry()["annotations"]` is `None` for every tool. After a
  later story adds `.destructive(true)` to one tool: `render()`'s output for
  that tool gains an `"annotations"` key, and `assert_family()` must fail with
  a message identifying that the golden file is stale for that tool, until the
  golden file is regenerated (US-0011).
- **Business Rules:** `assert_family()` must add exactly one more `assert_eq!`
  comparing the annotations `Value` (or its absence) between the live registry
  and the frozen entry, following the existing pattern of one `assert_eq!` per
  compared aspect:
  ```rust
  let mine_entry = mine.schema.to_list_entry();
  assert_eq!(
      mine_entry.get("annotations"),
      tool.get("annotations"),
      "annotations drift on {name}"
  );
  ```

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| existing `tool-contract-golden.json` | Frozen contract, 94 entries, no `annotations` key yet | repo fixture | ready |

### DT-005: contract golden regeneration is annotation-aware
- **Category:** happy
- **Scenario:** n/a (structural test, DEBT spec)
- **Requirements:** DR-008
- **Preconditions:** `render()` and `assert_family()` updated per this story;
  no tool has annotations set yet (US-0003 through US-0009 not yet run).
- **Steps:** Given the current registry with zero annotated tools / When
  `cargo test -p mcp-fs --lib tool_contract_golden_is_current` runs / Then the
  test still passes, because `render()` emits no `annotations` key for any
  tool and the frozen file has none either.
- **Cleanup:** none.
- **Priority:** Critical

This is the existing `tool_contract_golden_is_current` test, exercised
naturally once `render()` is updated; no new test function is added in this
story (per the spec's Section 6.3 DT-005 method note). The assertion that
proves this story's own change is correct is the pre-existing test staying
green immediately after the edit, before any tool gains an annotation.

## Constraints

### Files Not to Touch
- No `tools/*.rs` family file (admin.rs, git.rs, etc.) in this story.
- `tool-contract-golden.json` itself: not regenerated in this story (there is
  nothing to regenerate yet, since no tool has annotations).

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not hand-construct the annotations JSON shape in `render()`; reuse
  `ToolSchema::to_list_entry()` from US-0001 as the single source of truth, so
  the golden file and the live `tools/list` response can never diverge in
  shape.

### Scope Boundary
- This story only prepares the golden-contract machinery. It does not
  annotate any tool and does not touch the golden JSON file.

## Non Regression

### Existing Tests That Must Pass
- `tool_contract_golden_is_current` (in `contract_golden.rs`) stays green at
  the end of this story specifically because no tool yet has annotations.
- **The existing test suite passes unmodified.** Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- `render()`'s output for every currently-unannotated tool is byte-identical
  to its pre-story output.

### API Contracts to Preserve
- `tool-contract-golden.json` schema: additive only (Section 4 of the spec).

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
