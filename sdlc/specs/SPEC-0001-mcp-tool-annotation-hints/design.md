# SPEC-0001 — MCP Tool Annotation Hints: Design

## 1. Components

- `crates/mcp-fs/src/mcp/schema.rs`: `ToolSchema` struct, now carrying an optional `ToolAnnotations` (four optional bool fields: `destructive_hint`, `read_only_hint`, `idempotent_hint`, `open_world_hint`) plus four builder methods (`destructive`, `read_only`, `idempotent`, `open_world`); `to_list_entry()` serializes it.
- `crates/mcp-fs/src/tools/*.rs` (19 family files): each tool registration call site gains an annotation builder chain.
- `crates/mcp-fs/src/tools/contract_golden.rs`: `render()` and `assert_family()` extended to include/compare the `annotations` field.
- `crates/mcp-fs/src/tools/all.rs`: registry-wide invariant tests (DT-003, DT-004) iterating every registered tool.
- `tool-contract-golden.json`: regenerated frozen contract, every entry gaining an `annotations` key.
- `TOOL_CONTRACT.txt`, `.agent_docs/tools.md`: human-readable mirrors updated to list annotations per tool.

## 2. Flows

1. Client sends `tools/list`.
2. Registry's `list_payload()` calls `schema.to_list_entry()` per tool (unchanged call site).
3. `to_list_entry()` emits `name`, `description`, `inputSchema`, and now an optional `annotations` object, present only when at least one hint is set.
4. Client reads `annotations.destructiveHint`/`readOnlyHint`/`idempotentHint`/`openWorldHint` to decide whether to gate human confirmation, with no change to dispatch or runtime behavior.

## 3. Interfaces

- Wire format addition (`tools/list` response, per tool entry):
```json
{
  "name": "fs.delete",
  "description": "...",
  "inputSchema": { "...": "..." },
  "annotations": {
    "destructiveHint": true,
    "readOnlyHint": false,
    "idempotentHint": true
  }
}
Absent hints are omitted; the whole `annotations` key is omitted when no hint is set.

## 4. Data and state

No persistent state change. `ToolAnnotations` is pure in-memory schema metadata, constructed once at tool-registration time, with no database or config-driven variance.

### Data Model

```rust
struct ToolAnnotations {
    destructive_hint: Option<bool>,
    read_only_hint: Option<bool>,
    idempotent_hint: Option<bool>,
    open_world_hint: Option<bool>,
}
Added as `pub annotations: Option<ToolAnnotations>` on `ToolSchema`, after the existing `params` field.

## 5. Configuration

None. No config flag controls this behavior; annotations are static per tool regardless of runtime config (see Decisions, DEC-001, for the `doc_service` case specifically).

## 6. Observability

No new logging/tracing surface. This is a pure schema/wire-format change; existing request tracing for `tools/list` is unaffected.

## 7. Decisions

| ID | Decision | Evidence | Rationale |
|---|---|---|---|
| DEC-001 | `openWorldHint` for `doc.to_docx`, `doc.to_pptx`, `fs.documentize`, `fs.extract_text` is declared `true` unconditionally, regardless of whether `doc_service` is configured as a local CLI or an HTTP endpoint. | spec Section 8, DDEC-001 | A hint describes a tool's worst-case capability across all valid server configurations, not the current deployment's config; MCP hints are static per tool, not dynamic per-instance. |
| DEC-002 | `sqlite.import_csv` is classified `destructiveHint=false`, `idempotentHint=false`. | crates/mcp-fs/src/tools/sqlite.rs:365-420 | Handler only ever executes `CREATE TABLE IF NOT EXISTS` and `INSERT INTO`, never destructive SQL; but repeating it duplicates rows, so not safe to retry. |
| DEC-003 | `doc.open_editor` is classified `idempotentHint=false`. | crates/mcp-fs/src/tools/editor.rs:106-131 | Each call returns a new, unique `editor_id` rather than reusing an existing session. |
| DEC-004 | Tools whose default parameters make a specific call non-destructive (e.g. `overwrite=false`) are still classified by worst-case capability, not default-argument behavior. | spec Section 8, DDEC-004 | MCP hints describe what a tool CAN do, not what a specific call with default arguments does; a client cannot inspect not-yet-chosen arguments. |
| DEC-005 | `git.merge_abort`, `git.rebase_abort`, `git.cherry_pick_abort`, `git.revert_abort` are classified `destructiveHint=false` uniformly, despite each discarding uncommitted conflict-resolution work. | crates/mcp-fs/src/tools/git.rs (per-handler schema description) | Their entire documented purpose is restoring the repo to its pre-operation state and preventing data loss; marking them destructive would gate the recovery action itself, inverting the hint's intent. |
| DEC-006 | Post-rollout CONVERGE audit reclassified `git.merge_resolve`, `git.rebase`, `git.rebase_continue` to `destructiveHint=true` and `git.cherry_pick` to `destructiveHint=false`. | legacy stories/_index.md, US-0012 | Audit found these four tools' initial classification did not match the per-tool table in spec Section 3.1; fixed to converge with the documented classification. |

## 8. Requirement to code map

| Requirement | Code |
|---|---|
| DR-001..DR-005 (classification) | Per-tool annotation builder chains across all 19 `crates/mcp-fs/src/tools/*.rs` family files |
| DR-006 (wire serialization) | `crates/mcp-fs/src/mcp/schema.rs`, `to_list_entry()` |
| DR-007 (invariance) | `crates/mcp-fs/src/mcp/schema.rs` (no change to `name`/`description`/`inputSchema` fields or dispatch) |
| DR-008 (golden contract awareness) | `crates/mcp-fs/src/tools/contract_golden.rs`, `render()`/`assert_family()`; `tool-contract-golden.json` |

## 9. Legacy mapping

Source: specs/archived/SPEC-0001_2026-09-23_08-47-57-mcp-tool-annotation-hints/spec.md (pre-move)

| Old id | New id | Note |
|---|---|---|
| US-0001 Tool annotations schema infrastructure | SPEC-0001 FR DR-006/DR-007 | done, implemented, not re-tracked in v2 queue |
| US-0002 Contract golden annotation awareness | SPEC-0001 FR DR-008 | done, implemented, not re-tracked in v2 queue |
| US-0003 Annotate admin, context7, db tools (17) | SPEC-0001 FR DR-001..005 (subset) | done, implemented, not re-tracked in v2 queue |
| US-0004 Annotate doc, document, edit tools (10) | SPEC-0001 FR DR-001..005 (subset) | done, implemented, not re-tracked in v2 queue |
| US-0005 Annotate editor, git_auth, git_pr tools (13) | SPEC-0001 FR DR-001..005 (subset) | done, implemented, not re-tracked in v2 queue |
| US-0006 Annotate listing, metadata, read tools (13) | SPEC-0001 FR DR-001, DR-005 | done, implemented, not re-tracked in v2 queue |
| US-0007 Annotate search, search_semantic, sqlite tools (16) | SPEC-0001 FR DR-001..005 (subset) | done, implemented, not re-tracked in v2 queue |
| US-0008 Annotate web, write, lifecycle tools (15) | SPEC-0001 FR DR-001..005 (subset) | done, implemented, not re-tracked in v2 queue |
| US-0009 Annotate git.rs tools (39) | SPEC-0001 FR DR-001..005 (subset) | done, implemented, not re-tracked in v2 queue |
| US-0010 Registry-wide annotation invariant tests | SPEC-0001 Test DT-003/DT-004 | done, implemented, not re-tracked in v2 queue |
| US-0011 Regenerate golden contract and update docs | SPEC-0001 FR DR-008 (completion) | done, implemented, not re-tracked in v2 queue |
| US-0012 Fix 4 destructiveHint misclassifications (CONVERGE) | SPEC-0001 DEC-006 | done, implemented, not re-tracked in v2 queue |
