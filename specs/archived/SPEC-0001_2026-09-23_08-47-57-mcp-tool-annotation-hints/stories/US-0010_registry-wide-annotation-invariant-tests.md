# US-0010: Registry-wide annotation invariant tests

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 10
> Depends On: US-0003, US-0004, US-0005, US-0006, US-0007, US-0008, US-0009
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
Add DT-003 and DT-004 to `crates/mcp-fs/src/tools/all.rs`, once every tool
family is annotated (US-0003 through US-0009 all merged), so these two
structural gates pass immediately rather than failing on partial coverage:
every registered tool has an explicit annotation, and no tool is both
`readOnlyHint=true` and carries a `destructiveHint`.

## Technical Context

### Stack
Rust 2024, `serde_json::Value`.

### Relevant File Structure
```
crates/mcp-fs/src/tools/all.rs
```

### Existing Patterns
`all.rs`'s existing tests iterate a freshly built `ToolRegistry` and assert on
`reg.len()` / `reg.resolve(name)` (e.g. `the_git_families_add_forty_nine_tools`,
`all_features_enabled_count`). New tests follow the same shape, but iterate
every registered name via `reg.names()`:
```rust
#[test]
fn every_registered_tool_has_an_explicit_annotation() {
    let mut reg = ToolRegistry::new();
    let features = EnabledFeatures {
        git: true, web: true, context7: true, sqlite: true, db: true, doc: true, search: true,
    };
    let config = crate::config::ServerConfig::default();
    super::register_all(&mut reg, &features, &config);
    for name in reg.names() {
        let tool = reg.resolve(name).unwrap();
        let entry = tool.schema.to_list_entry();
        assert!(
            entry.get("annotations").is_some_and(|a| a.is_object()),
            "{name} has no annotations set"
        );
    }
}

#[test]
fn read_only_tools_never_carry_a_destructive_hint() {
    let mut reg = ToolRegistry::new();
    let features = EnabledFeatures {
        git: true, web: true, context7: true, sqlite: true, db: true, doc: true, search: true,
    };
    let config = crate::config::ServerConfig::default();
    super::register_all(&mut reg, &features, &config);
    for name in reg.names() {
        let tool = reg.resolve(name).unwrap();
        let entry = tool.schema.to_list_entry();
        let ann = &entry["annotations"];
        if ann["readOnlyHint"] == serde_json::json!(true) {
            assert!(
                ann.get("destructiveHint").is_none(),
                "{name} is readOnlyHint=true but also carries destructiveHint"
            );
        }
    }
}
```

### Data Model (excerpt)
None; test-only, iterating the existing registry.

### Decisions That Govern This Story
None invented; DT-003 and DT-004 are copied verbatim from the spec's Section
6.3.

### Applicable NFRs
None beyond DR-007.

### Bounded Context
The composition root's registration entry point
(`crates/mcp-fs/src/tools/all.rs`), all features enabled, to cover every
optional family (including `doc.to_docx`/`doc.to_pptx`, which register
conditionally on `pandoc` being in `PATH`, per the existing
`all_features_enabled_count` test's `doc_count` pattern).

## Functional Requirements

### DT-003: every registered tool has an explicit annotation
- **EARS:** for every tool in `crate::tools::all::register_all` with every
  feature flag on, `to_list_entry()["annotations"]` is present and is not
  `Value::Null` (i.e., no tool was left with all four hints as `None`, which
  would indicate a forgotten registration).
- **Method:** integration test in `crates/mcp-fs/src/tools/all.rs`, iterating
  `reg.names()` and asserting `resolve(name).schema.to_list_entry()["annotations"].is_object()`.

### DT-004: `readOnlyHint=true` tools never carry `destructiveHint`
- **EARS:** no tool has both `readOnlyHint: Some(true)` and
  `destructiveHint: Some(_)` set (structural consistency with MCP convention).
- **Method:** integration test in `crates/mcp-fs/src/tools/all.rs`, same
  iteration as DT-003, checking the invariant on the parsed JSON.

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| fully annotated registry, all features on | `register_all` with every `EnabledFeatures` flag `true` | in-repo, after US-0003..US-0009 | ready |

### DT-003 (as an executable test)
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DT-003
- **Preconditions:** every tool family annotated (US-0003 through US-0009 done).
- **Steps:** Given `register_all` called with every feature flag `true` / When
  every registered tool's `to_list_entry()` is inspected / Then every one has
  an `"annotations"` key whose value is a JSON object (never absent, never
  `null`).
- **Cleanup:** none.
- **Priority:** Critical

### DT-004 (as an executable test)
- **Category:** edge
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DT-004
- **Preconditions:** same as DT-003.
- **Steps:** Given the same fully annotated registry / When a tool's
  `annotations["readOnlyHint"]` is `true` / Then that same tool's
  `annotations` object has no `"destructiveHint"` key.
- **Cleanup:** none.
- **Priority:** Critical

## Constraints

### Files Not to Touch
- No `tools/<family>.rs` file: annotation values are not modified here, only
  asserted on.
- `contract_golden.rs`: unchanged (already correct since US-0002).

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not special-case any tool name in the two new tests; both must iterate
  every registered tool uniformly, matching the spec's DT-003/DT-004 wording
  ("for every tool").

### Scope Boundary
- Only `all.rs`, adding exactly two `#[test]` functions.

## Non Regression

### Existing Tests That Must Pass
- Every existing `#[test]` in `all.rs`, unmodified.
- `tool_contract_golden_is_current` (in `contract_golden.rs`) is **still
  expected to fail** at the end of this story: the golden file is not
  regenerated until US-0011.
- **The existing test suite passes unmodified**, except for the one
  documented, expected exception above. Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- No production code path is touched in this story; only two test functions
  are added.

### API Contracts to Preserve
- N/A, test-only story.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
