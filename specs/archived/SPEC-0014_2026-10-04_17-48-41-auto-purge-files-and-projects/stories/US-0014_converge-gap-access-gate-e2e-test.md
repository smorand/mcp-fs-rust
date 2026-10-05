# US-0014: Converge gap — end-to-end access-gate test through a real fs.*/REST call

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Depends On: US-0004
> min_tier: 2
> Files touched: 1-2 (new test(s) in `crates/core/src/tools/fs_read.rs` or wherever
> `fs.read`'s own test module lives, and/or `crates/core/src/api/dataplane.rs`)

## Why this story exists

Phase 4.6 CONVERGE found a gap after US-0004: FR-NEW-013 ("a soft-deleted project is
inaccessible to every `fs.*`/`git.*` tool and to the REST plane, with no bypass for
the platform admin") is unit-tested at the gate itself
(`storage/admin.rs::require_member_rejects_soft_deleted_project_as_not_found` and
`::require_member_re_reads_live_state_after_undelete`), but no test drives an actual
`fs.*` tool call or an actual `/api/fs/...` REST request against a soft-deleted
project and asserts the caller-visible rejection. The spec's own acceptance tests
E2E-NEW-040 ("fs.read blocked on soft-deleted project") and E2E-NEW-042 ("REST
inherits the gate automatically") describe exactly this, and neither exists in the
codebase under those names or under equivalent coverage (those two numeric IDs are
already used by unrelated tests elsewhere in the tree — a numbering collision from
upstream specs — so do not reuse the bare numbers; name the new tests descriptively
and reference the FR/requirement in the test name or doc comment instead).

## Objective

Add tests that:

1. Seed a soft-deleted project (via `AdminBackend::soft_delete_project`, the same
   primitive US-0006 added).
2. Call a real `fs.*` tool (e.g. `fs.read`, through the same dispatch path
   `mcp/server.rs`'s tests already use, or directly through
   `core::fs_ops`'s caller-facing entry point guarded by `state.authorize`) against
   that project. Assert it fails as not-found, for a plain member, for the owner,
   and for a platform admin (no bypass).
3. Call the REST plane (`api/dataplane.rs`'s existing test harness, which already
   drives real HTTP requests per other tests in that file) against the equivalent
   REST route for the same soft-deleted project. Assert the same not-found
   rejection, proving MCP and REST share the one gate rather than each having its
   own copy that happens to agree.

## Acceptance Tests

### E2E-NEW-provisional-4: fs.read blocked on soft-deleted project, member/owner/admin alike
Seed a soft-deleted project. Call the real `fs.read` tool path as a member, as the
owner, and as a platform admin. All three get the not-found rejection — no bypass.

### E2E-NEW-provisional-5: REST inherits the same gate
Seed a soft-deleted project. Call the equivalent REST `/api/fs/...` route. Assert
the same rejection the MCP tool call produced, proving the single shared
`state.authorize` → `require_member` path, not a REST-side duplicate check.

## Constraints

### Files Not to Touch
`storage/admin.rs`'s `require_member`/`live_project_exists` — unchanged, the gate
itself is correct and already tested at that layer; this story only adds the
missing end-to-end proof above it.

### Dependencies Not to Add
None.

## Non Regression
Full existing suite stays green and unmodified elsewhere.
