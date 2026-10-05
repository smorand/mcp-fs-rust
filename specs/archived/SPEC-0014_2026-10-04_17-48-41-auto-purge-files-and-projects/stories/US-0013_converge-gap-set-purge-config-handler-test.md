# US-0013: Converge gap — test the production `admin.set_purge_config` handler

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Depends On: US-0009
> min_tier: 2
> Files touched: 1 (`crates/core/src/mcp/server.rs`)

## Why this story exists

Phase 4.6 CONVERGE found a gap after US-0009: the acceptance tests for FR-NEW-004,
FR-NEW-005 and FR-NEW-006 (`admin.set_purge_config` — enable flag, retention
rejection, authz/existence) call a `#[cfg(test)]`-gated duplicate function
`set_purge_config` in `crates/core/src/tools/admin.rs` (around line 536), not the
real dispatched tool handler `McpServer::admin_set_purge_config` in
`crates/core/src/mcp/server.rs` (around line 2395). The two implementations happen
to agree today, but nothing proves the production handler itself behaves per spec:
a future edit to one and not the other would pass the existing suite while the real
tool silently diverges.

## Objective

Add tests in `crates/core/src/mcp/server.rs`'s own `tests` module that call
`McpServer::admin_set_purge_config` directly (the established pattern already used
in that module — see other `#[tool]` methods tested there via `Fixture` and
`ok_json`/raw-result inspection), proving the production handler, not the test-only
duplicate, satisfies:

- FR-NEW-004: owner or platform admin can set `autopurge_enabled`,
  `use_internal_purge`, and either retention value independently (including both
  absent).
- FR-NEW-005: `file_retention_days == Some(0)` and `project_retention_days ==
  Some(0)` are both rejected as invalid arguments; the smallest positive value (1)
  succeeds.
- FR-NEW-006: a non-owner/non-admin caller is forbidden; an unknown `project_id` is
  not-found.

Do not delete or modify the existing `tools/admin.rs` test-only duplicate or its
tests — they stay as a cheap regression net at the storage/trait layer. This story
adds the missing production-handler proof on top, it does not replace what exists.

## Acceptance Tests

### E2E-NEW-provisional-1: production handler accepts valid config (owner)
Call `McpServer::admin_set_purge_config` directly (via `Fixture`) as the project
owner with a valid config. Assert success and that `storage::admin::get_purge_config`
reflects the written values.

### E2E-NEW-provisional-2: production handler rejects zero retention
Same call with `file_retention_days: Some(0)`. Assert the call returns the
`invalid_argument` error, through the real handler, not the test-only duplicate.
Repeat for `project_retention_days: Some(0)`.

### E2E-NEW-provisional-3: production handler enforces authz and existence
A non-member, non-admin caller is forbidden. An unknown `project_id` is not-found.
Both through `McpServer::admin_set_purge_config` directly.

## Constraints

### Files Not to Touch
`tools/admin.rs`'s `set_purge_config` (test-only duplicate) and its own tests —
unchanged. `storage/admin.rs`'s `set_purge_config` implementation — unchanged, it is
not the gap.

### Dependencies Not to Add
None.

## Non Regression
Full existing suite stays green and unmodified elsewhere.
