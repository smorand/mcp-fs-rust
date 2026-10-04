# US-0009: Delete the old hand-rolled MCP layer + final suite/clippy/fmt gate

> Parent Spec: specs/SPEC-0013_2026-10-03_00-17-37-rmcp-3x-migration-prep/spec.md
> Spec ID: SPEC-0013
> Epic: n/a
> Status: ready
> Priority: 9
> Depends On: US-0008
> Complexity: M
> min_tier: 2
> Files touched: 4

## Objective
Delete `crates/core/src/mcp/{mod,registry,schema,args}.rs` now that US-0008's transport swap is
green against the new `rmcp`-based server, remove the 16 tests whose only purpose was pinning the
deleted module's internals, and run the full non-regression gate: `cargo test --workspace`,
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`. This is
the last story in the migration; after it, the system's only MCP transport is `rmcp`.

## Technical Context

### Stack
Rust 2024, full workspace.

### Relevant File Structure
```
crates/core/src/mcp/mod.rs        # deleted (145 lines: DEC-107 doc, sse_frame, rpc_result, rpc_error, initialize_result, 7 tests)
crates/core/src/mcp/registry.rs   # deleted (166 lines: ToolRegistry, ToolHandler, ToolCtx, 3 tests)
crates/core/src/mcp/schema.rs     # deleted (329 lines: ToolSchema builder, 6 tests)
crates/core/src/mcp/args.rs       # deleted (220 lines: Args, typed/tolerant accessors)
```

### Existing Patterns
None to follow — this story removes code, it does not add any.

### Decisions That Govern This Story
- **DR-003** (spec Section 3.1): "`crates/core/src/mcp/mod.rs`, `registry.rs`, `schema.rs` and
  `args.rs` SHALL be deleted in full once DR-002 is complete; no `ToolRegistry`, `ToolSchema`,
  `Args` or `ToolHandler` symbol SHALL remain referenced outside test code documenting the old
  contract for historical comparison (none is required; none SHALL be added)."
- **Implementation Order, Section 7, step 4** (spec): "delete `crates/core/src/mcp/{mod,registry,
  schema,args}.rs` only once step 3's tests (DT-001 through DT-004) are green against the new
  transport; deleting first would leave the build broken with no way to bisect a transport bug
  from a removed-code bug."
- **Implementation Order, Section 7, step 6** (spec): "Run `cargo test --workspace` (Section 6.1)
  and confirm the only failures before the fix were the ones Section 6.4 names; confirm `cargo
  clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` are clean."

### Applicable NFRs
DR-006, DR-007 (restated per Invariant 3, see US-0003) — this story deletes dead code only; it
must not change any observable behavior beyond what US-0008 already made observable.

## Functional Requirements

### DR-003: Delete the old mcp module in full
- **EARS:** see above (verbatim).
- **Inputs / Outputs:** `crates/core/src/mcp/{mod,registry,schema,args}.rs` no longer exist.
  `rg -n "ToolRegistry|ToolSchema::new|mcp::args::Args" crates/ --type rust` returns zero matches
  outside `target/`.
- **Business Rules:** no symbol from the deleted module is referenced anywhere in source after
  deletion; none is re-added for "historical comparison".

## Acceptance Tests

> **100% must pass.** Run through `make check` (format-check + lint + typecheck + security + test).

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| none | deletion + whole-suite run, no new fixtures | n/a | ready |

### DT-006: Old symbols fully removed
- **Category:** happy
- **Requirements:** DR-003
- **Preconditions:** `crates/core/src/mcp/{mod,registry,schema,args}.rs` deleted.
- **Steps:** Given the migrated codebase; When running `rg -n
  "ToolRegistry|ToolSchema::new|mcp::args::Args" crates/ --type rust`; Then the only matches are
  inside this document's own history (none expected in source), verified by the grep returning
  zero matches outside `target/`.
- **Cleanup:** none.
- **Priority:** Critical.

### Existing test verdicts carried into this story (spec Section 6.4)
- **7 tests, `mcp/mod.rs:93-145`** — `removed`. The module and its `sse_frame`/`rpc_result`/
  `rpc_error`/`initialize_result` functions are deleted whole under DR-003; these tests exist only
  to pin that module's internals.
- **3 tests, `mcp/registry.rs`** — `removed`. `ToolRegistry` deleted under DR-003.
- **6 tests, `mcp/schema.rs`** — `removed`. `ToolSchema` deleted under DR-003.

### DT-FINAL-001: Full non-regression gate is green
- **Category:** happy
- **Requirements:** Section 7 step 6 (spec)
- **Preconditions:** deletion complete, all prior stories (US-0001..0008) landed.
- **Steps:** Given the fully migrated codebase; When running `cargo test --workspace`, `cargo
  clippy --all-targets --all-features -- -D warnings`, and `cargo fmt --all -- --check` in
  sequence; Then all three succeed with zero failures and zero warnings, and the only test
  differences from the pre-migration suite are exactly the 33 (+2 agent, +1 full_stack_e2e)
  verdicts named across US-0008 and this story's own 16 removed tests — no unaccounted-for test
  was silently added, removed, or changed.
- **Cleanup:** none.
- **Priority:** Critical.

### DT-008 (acknowledged, no new work owed)
Per the spec: "already covered by 6.1, listed here only to make explicit that no new per-tool test
is owed by this migration, because DR-007 forbids per-tool behavior change." This story's
DT-FINAL-001 run of `cargo test --workspace` is what exercises it; no separate test is written.

## Constraints

### Files Not to Touch
`crates/core/src/mcp/server.rs` (the new, live `McpServer` — not touched by this story);
`crates/core/src/app.rs` (route wiring already done in US-0008).

### Dependencies Not to Add
None. This story may remove the dependency on anything the old module alone needed (e.g. if any
crate was pulled in solely for the hand-rolled SSE framing) — check `cargo machete` or equivalent
before declaring the dependency list final, but do not remove a dependency still used elsewhere.

### Patterns to Avoid
Re-adding any `ToolRegistry`/`ToolSchema`/`Args`/`ToolHandler` symbol "just in case" or "for
historical comparison" — DR-003 explicitly forbids this.

### Scope Boundary
Deletion and final verification only. No new functionality.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace` minus exactly the 16 removed tests named above, with every remaining test
green, including the 18 modified in US-0008.

### Behaviors That Must Not Change
DR-006, DR-007 (restated per Invariant 3, see US-0003), with the same explicit exclusions US-0008
named (initialize requirement, JSON/SSE default framing, JSON-RPC key order).

### API Contracts to Preserve
`TOOL_CONTRACT.txt`, `tool-contract-golden.json` for all 94 tools (closed out in US-0007, must
still hold after this deletion).

## Self-Review Checklist
Full 4-axis self-review per /implement Phase 3.3 Step 5. In addition: confirm the `rg` grep in
DT-006 was actually run and its zero-match output is quoted in the implementation report, not
assumed.
