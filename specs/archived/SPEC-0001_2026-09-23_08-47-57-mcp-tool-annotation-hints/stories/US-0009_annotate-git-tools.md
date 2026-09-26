# US-0009: Annotate the git.rs tool family (39 tools)

> Parent Spec: specs/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md
> Spec ID: SPEC-0001
> Epic: n/a
> Status: ready
> Priority: 9
> Depends On: US-0001
> Complexity: L
> min_tier: 2
> Files touched: 1

## Objective
Add the annotation builder chain to all 39 tool definitions in `git.rs`,
including the loop-registered `git.stash_pop`/`git.stash_apply` pair, per the
exact per-tool table below. This is deliberately its own story, kept to one
file, because `git.rs` is large (39 registrations) and its per-tool
classification carries the most nuanced decisions in the spec (worst-case
push/reset semantics, the `*_abort` non-destructive policy).

## Technical Context

### Stack
Rust 2024. Builder methods from US-0001.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
Example, `git.status` (pure read, local):
```rust
ToolSchema::new("git.status", ..)
    .req_str(...)
    .read_only(true)
    .idempotent(true)
    .open_world(false),
```
`git.remote_push` (`git.rs:648-706`), worst-case destructive (force mode can
overwrite the remote branch), open-world, idempotent (identical repeat push
converges):
```rust
ToolSchema::new("git.remote_push", ..)
    .req_str(...)
    .destructive(true)
    .read_only(false)
    .idempotent(true)
    .open_world(true),
```
`git.merge_abort` (one of the four `*_abort` tools), additive per DDEC-005:
```rust
ToolSchema::new("git.merge_abort", ..)
    .req_str(...)
    .destructive(false)
    .read_only(false)
    .idempotent(false)
    .open_world(false),
```
The loop-registered pair (`git.rs:751-786`, `for pop in [false, true] { ... }`)
produces `git.stash_pop` and `git.stash_apply` from one call site; both rows
in the table below apply to the single `ToolSchema::new(name, description)`
call inside the loop body, since both share the same annotation chain
(`destructive(true).read_only(false).idempotent(false).open_world(false)`).

### Data Model (excerpt)
None; builder calls only.

### Decisions That Govern This Story
- **DDEC-004** (quoted): worst-case capability classification, not
  default-argument behavior. Governs `git.reset` (hard mode rewrites the
  volume, soft mode moves the pointer only; classified `destructiveHint=true`
  by worst case) and `git.remote_push` (force mode can overwrite the remote
  branch; classified `destructiveHint=true` by worst case).
- **DDEC-005** (quoted): "`git.merge_abort`, `git.rebase_abort`,
  `git.cherry_pick_abort` and `git.revert_abort` are classified
  `destructiveHint=false`, uniformly, even though each discards uncommitted
  conflict-resolution work recorded so far... these four tools' entire
  documented purpose... is restoring the repository to its pre-operation state
  and preventing data loss from a stuck conflict, never committed data is
  destroyed by any of them, only in-progress, not-yet-committed resolution
  state. Marking them destructive would tell a HITL-aware client to gate
  exactly the recovery action a caller reaches for after a conflict, which
  inverts the intent of the hint." All four are Additive
  (`destructiveHint=false`), never Overwrite/delete, despite superficially
  resembling a delete.

### Applicable NFRs
None beyond DR-007.

### Bounded Context
The full `git.*` local-repository and remote-transport surface: status/log/
diff/blame (read), branch/stash/reset/rebase/merge/cherry-pick/revert
(local mutation), remote add/remove/clone/fetch/push/pull (network-touching
mutation).

## Functional Requirements

### DR-001 through DR-005: annotation classification rules (governing this story's table)
> DR-001: pure read ⇒ `read_only_hint=Some(true)`, no `destructive_hint`.
> DR-002: not pure read ⇒ `read_only_hint=Some(false)`.
> DR-003: Overwrite/delete ⇒ `destructive_hint=Some(true)` (worst case, per
> DDEC-004).
> DR-004: Additive ⇒ `destructive_hint=Some(false)` (per DDEC-005 for the four
> `*_abort` tools specifically).
> DR-005: `idempotent_hint`/`open_world_hint` follow the row exactly, no
> family-level default.

### Per-tool table (39 tools, copied verbatim from spec Section 3.1)

**Pure read (RO=true, D=-):**

| Tool | I | OW |
|------|---|----|
| `git.status` | true | false |
| `git.branches` | true | false |
| `git.tags` | true | false |
| `git.log` | true | false |
| `git.show` | true | false |
| `git.diff` | true | false |
| `git.blame` | true | false |
| `git.remote_list` | true | false |
| `git.stash_list` | true | false |

**Overwrite/delete (RO=false, D=true):**

| Tool | I | OW |
|------|---|----|
| `git.checkout_file` | true | false |
| `git.branch_switch` | true | false |
| `git.branch_delete` | true | false |
| `git.branch_reset` | true | false |
| `git.reset` | true | false |
| `git.remote_remove` | false | false |
| `git.remote_push` | true | true |
| `git.stash_drop` | true | false |
| `git.stash_pop` | false | false |
| `git.stash_apply` | false | false |
| `git.merge` | true | false |
| `git.merge_resolve` | false | false |
| `git.rebase` | false | false |
| `git.rebase_continue` | false | false |
| `git.remote_pull` | true | true |

**Additive (RO=false, D=false):**

| Tool | I | OW |
|------|---|----|
| `git.init` | true | false |
| `git.commit` | false | false |
| `git.branch_create` | false | false |
| `git.remote_add` | false | false |
| `git.remote_clone` | false | true |
| `git.remote_fetch` | true | true |
| `git.stash_save` | false | false |
| `git.merge_abort` | false | false |
| `git.rebase_abort` | false | false |
| `git.cherry_pick_abort` | false | false |
| `git.revert_abort` | false | false |
| `git.cherry_pick` | true | false |
| `git.cherry_pick_continue` | false | false |
| `git.revert` | false | false |
| `git.revert_continue` | false | false |

**Total check:** 9 (pure read) + 15 (overwrite/delete) + 15 (additive) = 39.

## Acceptance Tests

> **100% must pass.** Run tests through `make test`. Never run test files
> directly or with ad hoc commands.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| live registry after annotating | `git::register` (git feature enabled) | in-repo | ready |

### E2E-US009-01: every git.rs tool exposes the exact declared annotations
- **Category:** happy
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-001, DR-002, DR-003, DR-004, DR-005
- **Preconditions:** all 39 tools annotated per the table above.
- **Steps:** Given the registry with `git::register` called / When
  `to_list_entry()` is read for each of the 39 names (including both
  `git.stash_pop` and `git.stash_apply` from the loop registration) / Then
  each entry's `annotations` object matches its table row exactly.
- **Cleanup:** none.
- **Priority:** Critical

### E2E-US009-02: the four `*_abort` tools are never destructive
- **Category:** edge
- **Scenario:** n/a (structural, DEBT spec)
- **Requirements:** DR-004 (per DDEC-005)
- **Preconditions:** `git.merge_abort`, `git.rebase_abort`,
  `git.cherry_pick_abort`, `git.revert_abort` all annotated.
- **Steps:** Given the registry with `git::register` called / When each of the
  four `*_abort` tool schemas is read / Then
  `annotations["destructiveHint"] == false` for all four, never `true`.
- **Cleanup:** none.
- **Priority:** High

## Constraints

### Files Not to Touch
- No other `tools/*.rs` file, in particular not `git_auth.rs` or `git_pr.rs`
  (US-0005).

### Dependencies Not to Add
None.

### Patterns to Avoid
- Do not mark the four `*_abort` tools destructive by analogy with a "delete"
  sounding name; DDEC-005 rules them Additive.
- Do not mark `git.reset` or `git.remote_push` non-destructive on the
  reasoning that their default mode (soft reset, non-force push) is safe;
  DDEC-004 mandates worst-case classification.
- Do not touch the loop body's control flow (`for pop in [false, true]`) beyond
  appending the annotation chain to the single `ToolSchema::new(name, description)`
  call inside it.
- Do not change any `name`, `description`, param, or handler body.

### Scope Boundary
- Only `git.rs`.

## Non Regression

### Existing Tests That Must Pass
- Every existing `#[test]` in `git.rs` and in `crates/mcp-fs/src/tools/all.rs`,
  except `tool_contract_golden_is_current`, which stays **expected to fail**
  (per spec Section 7) until US-0011. `the_git_families_add_forty_nine_tools`
  in `all.rs` must keep asserting `reg.len() == 10 + 39 + 4 + 6` unchanged.
- **The existing test suite passes unmodified**, except for the one
  documented, expected exception above. Run:
  ```
  cargo test --workspace
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  ```

### Behaviors That Must Not Change
- Handler dispatch and runtime results for all 39 tools are unchanged (DR-007).
- The loop registration still produces exactly two tools,
  `git.stash_pop` and `git.stash_apply`, in the same order.

### API Contracts to Preserve
- `name`, `description`, `inputSchema` for all 39 tools: byte-identical.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
