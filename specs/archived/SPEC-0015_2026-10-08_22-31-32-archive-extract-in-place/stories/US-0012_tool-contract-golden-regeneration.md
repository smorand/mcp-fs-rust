---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P1
Depends On: US-0011
min_tier: 2
Complexity: XS
Files touched: TOOL_CONTRACT.txt, tool-contract-golden.json (generated, not hand-written)
---

# US-0012 — `TOOL_CONTRACT.txt` + golden regeneration

## Objective

Regenerate `TOOL_CONTRACT.txt` and `tool-contract-golden.json` to include
`fs.extract_archive`, bringing the tool count from 149 to 150, and bump the running tool-count
assertion in `mcp/server.rs`. Last story: every other change (US-0001 through US-0011) must be
complete and the tool fully defined before this regeneration runs.

## Technical Context

### Stack

Rust 2024. No code change beyond the generated files and one assertion bump; the regeneration
is driven by the test harness itself.

### Relevant File Structure

- `TOOL_CONTRACT.txt` — the 149 (soon 150) tool schemas, human readable, authoritative per
  `AGENTS.md`.
- `tool-contract-golden.json` — the machine-checked twin; three tests compare every name,
  description and `inputSchema` against it, serialized.
- `crates/core/src/tools/contract_golden.rs` — owns the regeneration logic, triggered by
  `MCPFS_REWRITE_TOOL_CONTRACT=1`.
- `crates/core/src/mcp/server.rs:3526`/`3539`-area — the running tool-count assertion, already
  tracking prior bumps (e.g. 36→37) with dated comments; this story adds the 149→150 bump in
  the same style.

### Existing Patterns Referenced

`AGENTS.md`'s own instruction: regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p
mcp-fs --lib tool_contract_golden_is_current`, then review the diff before committing. Never
hand-edit either generated file.

### Decisions That Govern This Story

**DEC-009** (process deviation note, informational only — not actionable by this story's
scope): the test design for this spec was authored without a fresh-context sub-agent; not
relevant to the mechanical regeneration this story performs.

### Applicable NFRs

None beyond the existing tool-contract golden-check convention.

### Bounded Context

Transport (`mcp::server`, extended not owned, §4.5): the tool contract is the frozen
description of every transport-exposed tool, including this spec's new one.

## Functional Requirements

**FR-NEW-030** [EARS-U] The system SHALL regenerate `TOOL_CONTRACT.txt` and
`tool-contract-golden.json` to include `fs.extract_archive` (bringing the tool count from 149 to
150), via `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib
tool_contract_golden_is_current`, and SHALL review the diff before committing, per `AGENTS.md`'s
own instruction for that command.

- Inputs: the fully-defined `fs.extract_archive` tool (from US-0004 through US-0011).
- Outputs: both generated files updated; the running tool-count assertion bumped 149→150.
- Business Rules: never hand-edit either generated file; run the regeneration command and
  review its diff.

## Acceptance Tests

### Test Data

| File | expected after regeneration |
|---|---|
| `TOOL_CONTRACT.txt` | contains `fs.extract_archive` with `params: mount_id:string, path:string, destination:string=null, overwrite:boolean=false, password:string=null` |
| `tool-contract-golden.json` | contains tool `fs.extract_archive` with `inputSchema.required == ["mount_id", "path"]` |
| `mcp/server.rs` tool-count assertion | `150` |

#### E2E-NEW-033 — `TOOL_CONTRACT.txt` lists `fs.extract_archive` with the exact schema

- Category: Happy (structural). Requirements: FR-NEW-030.
- Driver: a `#[test]` reading `TOOL_CONTRACT.txt`.
- Steps: Given the regenerated file, When searched for the line `fs.extract_archive`, Then it
  is present, with a `params:` line listing `mount_id:string, path:string,
  destination:string=null, overwrite:boolean=false, password:string=null` (exact parameter
  list and defaults matching `FR-NEW-001`).
- Cleanup: none. Priority: P1.

#### E2E-NEW-048 — `tool-contract-golden.json` also includes `fs.extract_archive`

- Category: Happy. Requirements: FR-NEW-030.
- Driver: a `#[test]` reading `tool-contract-golden.json` (the machine-checked twin of
  `TOOL_CONTRACT.txt`).
- Steps: Given the regenerated file, When parsed as JSON and searched for a tool named
  `fs.extract_archive`, Then it is present with an `inputSchema` whose required array is
  `["mount_id", "path"]`.
- Cleanup: none. Priority: P1.

#### E2E-NEW-061 — The running tool-count assertion in `mcp/server.rs` is bumped 149→150

- Category: Happy. Requirements: FR-NEW-030.
- Driver: `crates/core/src/mcp/server.rs`'s own running tool-count test (the one whose comments
  already track prior bumps, e.g. `server.rs:3526`/`3539`).
- Steps: Given the test's existing assertion of the total tool count, When
  `fs.extract_archive` is added, Then the asserted total is `150`, with a comment dated to this
  spec mirroring the existing comment style for prior bumps.
- Cleanup: none. Priority: P2.

## Constraints

- Files Not to Touch: no further logic changes to `tools/archive.rs`, `dataplane.rs`,
  `openapi.rs` — those are all complete by the time this story runs.
- Dependencies Not to Add: none.
- Patterns to Avoid: never hand-edit `TOOL_CONTRACT.txt` or `tool-contract-golden.json`
  directly; always regenerate via the documented command and review the diff.
- Scope Boundary: contract regeneration and the one tool-count assertion bump only.

## Non Regression

The existing 149 tool entries in both files must remain byte-for-byte identical except for the
new entry's insertion; the three golden-comparison tests must still pass for every pre-existing
tool.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
