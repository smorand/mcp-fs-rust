> Id: SPEC-0001
> Nature: FEAT
> Status: implemented
> Migrated from: specs/archived/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md on 2026-10-09

# MCP Tool Annotation Hints

## 1. Summary

Every MCP tool the server exposes now declares, in its `tools/list` schema, a standard set of behavioral hints: whether it only reads, whether it can destroy data, whether it is safe to retry, and whether it reaches outside the sandboxed volume/database. A caller that gates human confirmation on tool calls can read these hints directly instead of hardcoding tool names.

## 2. Current State

### 2.1 Functional

Every registered tool (all `fs.*`, `admin.*`, `git.*`, `git.auth*`, `git.pr_*`, `search.*`, `sqlite.*`, `db.*`, `doc.*`, `context7.*`, `web.*` families) carries an explicit annotations object in its `tools/list` entry, composed of four optional hints: destructive, read-only, idempotent, and open-world. A tool with no hint set omits the annotations key entirely; a tool with some hints set emits only those, in camelCase. No tool's name, description, or input schema changed as part of this work.

### 2.2 Existing specs

SPEC-0002 platform-foundation (tool registration).

### 2.3 Test coverage

Unit tests verify the wire serialization omits absent hints and renders present ones in camelCase. An integration test iterates every registered tool and asserts each one has an explicit, non-null annotations object (catching any tool left unannotated by accident), plus a structural invariant that a read-only tool never also carries a destructive hint. The frozen tool contract (golden file) was regenerated and reviewed to confirm only annotations keys were added, with no change to any tool's name, description, or input schema. A follow-up audit (CONVERGE) found and fixed four misclassified hints after the initial rollout.

## 3. Scope

In scope: adding behavioral hints to every existing MCP tool's schema, and the wire format that exposes them. Out of scope: any change to tool names, descriptions, input schemas, JSON-RPC dispatch, handler behavior, or runtime results.

## 4. Actors

- **MCP client**: any caller of the `tools/list` method, including the bundled CLI agent and third-party MCP clients that gate human confirmation on tool risk.
- **Server maintainer**: classifies each tool against the five classification rules and keeps the frozen contract in sync.

## 5. Usage Scenarios

1. An MCP client calls `tools/list` and receives, for each tool, an optional annotations object describing its risk profile (destructive, read-only, idempotent, open-world), without needing to special-case tool names to decide whether to prompt a human before calling it.
2. An MCP client that does not understand the new field continues to operate unaffected, since the field is additive and optional.

## 6. Functional Requirements

| ID | Requirement | Evidence |
|---|---|---|
| DR-001 | Every tool classified "pure read" sets `readOnlyHint=true` and does not set `destructiveHint`. | crates/mcp-fs/src/tools/all.rs (registry-wide invariant test) |
| DR-002 | Every tool not classified "pure read" sets `readOnlyHint=false`. | crates/mcp-fs/src/tools/all.rs |
| DR-003 | Every "overwrite/delete" tool sets `destructiveHint=true`, reflecting its worst-case capability regardless of parameter defaults that make a specific call safe. | crates/mcp-fs/src/tools/*.rs (per-family annotation chains) |
| DR-004 | Every "additive" tool sets `destructiveHint=false`. | crates/mcp-fs/src/tools/*.rs |
| DR-005 | Every tool sets `idempotentHint` and `openWorldHint` per its explicit per-tool classification, with no family-level default inferred. | crates/mcp-fs/src/tools/*.rs |
| DR-006 | `to_list_entry()` serializes the annotations field as a camelCase JSON object (`destructiveHint`, `readOnlyHint`, `idempotentHint`, `openWorldHint`), omitting any unset hint, and omits the whole `annotations` key when every hint is unset. | crates/mcp-fs/src/mcp/schema.rs |
| DR-007 | The system does not change any existing tool's name, description, or input schema value, nor its JSON-RPC dispatch, handler signature, or runtime result, as a result of this change. | crates/mcp-fs/src/mcp/schema.rs; tool-contract-golden.json (regeneration diff reviewed) |
| DR-008 | The contract-golden renderer includes the annotations field (when present) in every rendered tool entry, and the golden-contract-currency test fails when a registered tool's annotations differ from the frozen contract. | crates/mcp-fs/src/tools/contract_golden.rs |

## 7. Non-Functional Requirements

- Additive-only wire change: no existing `tools/list` consumer breaks (clients ignoring unknown JSON keys are unaffected).
- No behavior change to any tool call: annotations are pure metadata, not a server-side enforcement mechanism.

## 8. E2E Tests

| Test | Level | State |
|---|---|---|
| `to_list_entry` omits `annotations` when unset | unit | done |
| `to_list_entry` serializes present hints in camelCase, omits absent ones | unit | done |
| Every registered tool has an explicit, non-null annotations object | integration | done |
| `readOnlyHint=true` tools never also carry `destructiveHint` | integration | done |
| Contract-golden regeneration is annotation-aware (fails on drift from the frozen contract) | integration | done |
| CONVERGE audit: 4 misclassified `destructiveHint` values (git.merge_resolve, git.rebase, git.rebase_continue, git.cherry_pick) found and fixed | integration | done |

## 9. Glossary

- **Annotation / hint**: one of four optional booleans (`destructiveHint`, `readOnlyHint`, `idempotentHint`, `openWorldHint`) attached to a tool's schema, read by MCP clients to decide whether to prompt a human before invoking the tool.
- **Open-world**: a tool that reaches outside the sandboxed volume/database (e.g. an external provider API or network fetch).
- **Golden contract**: the frozen, machine-checked snapshot of every tool's schema (`tool-contract-golden.json`), compared on every test run.

## 10. Confidence notes

All 11 stories (US-0001 through US-0011) plus one converge story (US-0012) are recorded as "done" in the legacy `stories/_index.md`. Per-story commit SHAs were not individually retrieved; `git log --oneline -3` against the legacy stories index file returned a single representative completion commit (`d0f22a3`, "Add MCP tool annotation hints (SPEC-0001)"), used here as the completion marker for the whole spec rather than resolving one SHA per story.
