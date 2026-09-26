# US-0001: Tool annotations schema infrastructure

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
Add the `ToolAnnotations` struct and the four builder methods (`destructive`,
`read_only`, `idempotent`, `open_world`) to `ToolSchema`, and wire
`to_list_entry()` to serialize them. This is pure infrastructure: no tool
registration site calls the new methods yet, so every existing behavior stays
byte-identical until later stories opt individual tools in.

## Technical Context

### Stack
Rust 2024, `serde_json` (`Map`, `Value`, `json!`). No new dependency.

### Relevant File Structure
```
crates/mcp-fs/src/mcp/schema.rs
```

### Existing Patterns
`ToolSchema` (`crates/mcp-fs/src/mcp/schema.rs:51-55`) is a builder: consuming
methods that take `self` and return `Self`, matching the existing `req_str`/
`opt_bool` style (`schema.rs:74-116`):
```rust
pub fn opt_bool(self, n: &'static str, def: bool, d: &'static str) -> Self {
    self.push(n, d, ParamType::Bool, false, Some(json!(def)))
}
```
`to_list_entry()` today (`schema.rs:173-179`):
```rust
pub fn to_list_entry(&self) -> Value {
    json!({
        "name": self.name,
        "description": self.description,
        "inputSchema": self.input_schema(),
    })
}
```

### Data Model (excerpt)
New struct, four `Option<bool>` fields, no `Default` needed beyond
`#[derive(Default)]` (all `None`):
```rust
#[derive(Debug, Clone, Default)]
pub struct ToolAnnotations {
    pub destructive_hint: Option<bool>,
    pub read_only_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}
```
`ToolSchema` gains one field, placed after `params:` per the spec's exact
placement instruction:
```rust
pub struct ToolSchema {
    pub name: &'static str,
    pub description: String,
    params: Vec<Param>,
    annotations: ToolAnnotations,
}
```

### Decisions That Govern This Story
None beyond DR-006 and DR-007 below; this story invents no technical
decision.

### Applicable NFRs
None declared in the spec beyond the behavior-invariance requirement (DR-007).

### Bounded Context
The MCP wire layer (`crates/mcp-fs/src/mcp/`): schema construction and
`tools/list` serialization. No other module is touched by this story.

## Functional Requirements

### DR-006: `to_list_entry` serializes present annotations
- **EARS:** The `to_list_entry()` method SHALL serialize the `annotations`
  field as a camelCase JSON object (`destructiveHint`, `readOnlyHint`,
  `idempotentHint`, `openWorldHint`) omitting any hint whose value is `None`,
  and SHALL omit the `annotations` key entirely from the rendered `Value` when
  every hint is `None`.
- **Inputs / Outputs:** A `ToolSchema` with no annotation builder calls ⇒
  `to_list_entry()` has no `"annotations"` key. A `ToolSchema` with
  `.destructive(true).read_only(false)` called and nothing else ⇒
  `to_list_entry()["annotations"] == {"destructiveHint":true,"readOnlyHint":false}`.
- **Business Rules:** Key order in the emitted object does not matter here
  (unlike `inputSchema`, `annotations` is not covered by the property-key-order
  golden assertion in this story; that assertion is added in US-0002/US-0011 once
  the golden file itself carries `annotations`). Add the four builder methods:
  ```rust
  pub fn destructive(mut self, v: bool) -> Self {
      self.annotations.destructive_hint = Some(v);
      self
  }
  pub fn read_only(mut self, v: bool) -> Self {
      self.annotations.read_only_hint = Some(v);
      self
  }
  pub fn idempotent(mut self, v: bool) -> Self {
      self.annotations.idempotent_hint = Some(v);
      self
  }
  pub fn open_world(mut self, v: bool) -> Self {
      self.annotations.open_world_hint = Some(v);
      self
  }
  ```
  `to_list_entry()` becomes:
  ```rust
  pub fn to_list_entry(&self) -> Value {
      let mut entry = Map::new();
      entry.insert("name".into(), json!(self.name));
      entry.insert("description".into(), json!(self.description));
      entry.insert("inputSchema".into(), self.input_schema());
      let a = &self.annotations;
      if a.destructive_hint.is_some()
          || a.read_only_hint.is_some()
          || a.idempotent_hint.is_some()
          || a.open_world_hint.is_some()
      {
          let mut ann = Map::new();
          if let Some(v) = a.destructive_hint {
              ann.insert("destructiveHint".into(), json!(v));
          }
          if let Some(v) = a.read_only_hint {
              ann.insert("readOnlyHint".into(), json!(v));
          }
          if let Some(v) = a.idempotent_hint {
              ann.insert("idempotentHint".into(), json!(v));
          }
          if let Some(v) = a.open_world_hint {
              ann.insert("openWorldHint".into(), json!(v));
          }
          entry.insert("annotations".into(), Value::Object(ann));
      }
      Value::Object(entry)
  }
  ```

### DR-007: Behavior invariance
- **EARS:** The system SHALL NOT change the `name`, `description`, or
  `inputSchema` value of any existing tool, and SHALL NOT change the JSON-RPC
  method dispatch, handler signature, or runtime result of any tool call, as a
  result of this change.
- **Inputs / Outputs:** Every existing `schema.rs` test that asserts on
  `name`/`description`/`input_schema()` output continues to pass unmodified.
- **Business Rules:** `input_schema()` itself (`schema.rs:118-165`) is not
  touched by this story. `to_list_entry()` is rewritten but must still emit the
  exact same `name`, `description`, `inputSchema` values and keys it emitted
  before, for every `ToolSchema` that never calls an annotation builder.

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| `ToolSchema::new("t.probe", "d")` | Bare schema, no annotations, no params | test-local | ready |
| `ToolSchema::new("t.probe", "d").destructive(true).read_only(false)` | Two hints set, two absent | test-local | ready |

### DT-001: `to_list_entry` omits `annotations` when unset
- **Category:** happy
- **Scenario:** n/a (structural test, DEBT spec)
- **Requirements:** DR-006
- **Preconditions:** a `ToolSchema` built with no annotation builder call.
- **Steps:** Given `let s = ToolSchema::new("t.probe", "d");` / When
  `let v = s.to_list_entry();` / Then `assert!(v.get("annotations").is_none())`.
- **Cleanup:** none.
- **Priority:** Critical

### DT-002: `to_list_entry` serializes present hints in camelCase, omits absent ones
- **Category:** happy
- **Scenario:** n/a (structural test, DEBT spec)
- **Requirements:** DR-006
- **Preconditions:** a `ToolSchema` built with `.destructive(true).read_only(false)`
  and no idempotent/open-world call.
- **Steps:** Given `let s = ToolSchema::new("t.probe", "d").destructive(true).read_only(false);`
  / When `let v = s.to_list_entry();` / Then
  `assert_eq!(v["annotations"], json!({"destructiveHint": true, "readOnlyHint": false}))`
  and `assert!(v["annotations"].get("idempotentHint").is_none())` and
  `assert!(v["annotations"].get("openWorldHint").is_none())`.
- **Cleanup:** none.
- **Priority:** Critical

Test Implementation (added to `#[cfg(test)] mod tests` in `schema.rs`):
```rust
#[test]
fn to_list_entry_omits_annotations_when_unset() {
    let s = ToolSchema::new("t.probe", "d");
    let v = s.to_list_entry();
    assert!(v.get("annotations").is_none());
}

#[test]
fn to_list_entry_serializes_present_hints_camel_case() {
    let s = ToolSchema::new("t.probe", "d").destructive(true).read_only(false);
    let v = s.to_list_entry();
    assert_eq!(v["annotations"], json!({"destructiveHint": true, "readOnlyHint": false}));
    assert!(v["annotations"].get("idempotentHint").is_none());
    assert!(v["annotations"].get("openWorldHint").is_none());
}
```

Signatures to implement:
```rust
pub struct ToolAnnotations { .. } // #[derive(Debug, Clone, Default)]
impl ToolSchema {
    pub fn destructive(self, v: bool) -> Self;
    pub fn read_only(self, v: bool) -> Self;
    pub fn idempotent(self, v: bool) -> Self;
    pub fn open_world(self, v: bool) -> Self;
    pub fn to_list_entry(&self) -> Value; // rewritten, same external shape plus optional annotations
}
```

## Constraints

### Files Not to Touch
- No file under `crates/mcp-fs/src/tools/` in this story; no tool registration
  site calls the new methods yet.
- `crates/mcp-fs/src/mcp/registry.rs` and `crates/mcp-fs/src/app.rs`: unchanged,
  they call `to_list_entry()` unmodified and need no edit.

### Dependencies Not to Add
- No new crate. `serde_json` is already a dependency.

### Patterns to Avoid
- Do not change `input_schema()`'s key order or the `name`/`description`
  emission order inside `to_list_entry()`.

### Scope Boundary
- This story does not annotate any tool. It only adds the capability.

## Non Regression

### Existing Tests That Must Pass
- All 5 existing `#[test]` functions in `crates/mcp-fs/src/mcp/schema.rs`.
- **The existing test suite passes unmodified.** Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- Every `to_list_entry()` call for a `ToolSchema` built before this story
  (i.e. without an annotation builder call) renders byte-identical JSON to
  before this change (verified: no `"annotations"` key appears).

### API Contracts to Preserve
- `tools/list` JSON-RPC response shape: additive only, per Section 4 of the
  spec.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
