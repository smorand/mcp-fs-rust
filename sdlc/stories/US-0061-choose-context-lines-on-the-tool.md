# US-0061: choose the context size of a comparison

> Spec: SPEC-0046 (sdlc/specs/SPEC-0046-diff-context-lines/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 6
> Closes spec: no

## Objective
A member can pass an optional `context_lines` to `fs.diff_files` and get that many unchanged lines around each change; absent means 3, byte identical to today; invalid values are refused first, with one message. Note: 6 files, two of them contract artifacts regenerated mechanically, so the decision density stays at T2.

## Requirements covered
- SPEC-0046/FR-NEW-001: WHEN a member asks to compare two text files and gives a context size N THE product SHALL show, around each change, at most N unchanged lines before it and at most N after it.
- SPEC-0046/FR-NEW-002: IF no context size is given THEN THE product SHALL behave as if 3 were given.
- SPEC-0046/FR-NEW-003: IF the context size is not a whole number from 0 to 10000000 THEN THE product SHALL refuse the comparison.
- SPEC-0046/FR-NEW-004: WHEN the context size is 0 THE product SHALL show only the removed and added lines of each change, with the two header lines.
- SPEC-0046/FR-NEW-005: WHEN at most 2 times the context size unchanged lines separate two changes THE product SHALL show them in one block.
- SPEC-0046/FR-NEW-006: IF both files have identical content and the context size is valid THEN THE product SHALL answer that they are identical, with an empty diff.
- SPEC-0046/FR-NEW-007: The product SHALL check the context size before examining any project, path or file, and on the web interface right after the check for missing values.
- SPEC-0046/FR-NEW-010: WHEN more than 2 times the context size unchanged lines separate two changes THE product SHALL show them in two blocks.
- SPEC-0046/FR-MOD-001: WHEN a member asks to compare two text files THE product SHALL return the differences from the first file to the second as a unified diff with the requested number of unchanged lines around each change, 3 when none is requested.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::diff_files_e2e`: the 48 existing tests and the new ones pass; E2E-003 returns exactly `--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -6 +6 @@\n-l6\n+L6\n` for size 0 and E2E-011 is refused with the pinned message while the same call without a size returns the 3 line diff.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0046): Validate `context_lines` inside `tools::diff_files::diff_files`, as its first statement, via `parse_context_lines`. **Obliged by:** FR-NEW-003, FR-NEW-007, FR-NEW-009. **Rationale:** one implementation for both doors, refusal before any project or file is examined.
- DEC-002 (SPEC-0046): `DiffFilesArgs.context_lines: Option<serde_json::Value>` with `#[serde(default)]` and the frozen description as doc comment, so `-1`, `abc`, `2.5`, `true`, `[1]` reach the handler's own message instead of a protocol error.
- DEC-003 (SPEC-0046): `None` and `Value::Null` mean `DEFAULT_CONTEXT` (3). `Value::Number` is accepted only when `n.is_u64()` and <= `MAX_CONTEXT_LINES` (serde_json reads `-0` as an f64, `de.rs:517-523`, so it is refused; `3.0`, `1e3`, `-1`, `2.5` are f64/i64 and refused). `Value::String` is accepted only when non empty, every byte `0` to `9`, and it parses and is <= max (leading zeros allowed, overflow refused). `Bool`, `Array`, `Object` refused.
- DEC-005 (SPEC-0046): add `ParamType::Any` and `ToolSchema::opt_any(name, desc)` rendering `description` then `default: null`, no `type`, for the test registry glue (precedent: `opt_flexible_str_array`, `registry_support/schema.rs:165`).
- DEC-006 (SPEC-0046): one message built from `MAX_CONTEXT_LINES`: `'context_lines' must be a whole number from 0 to 10000000`, via `ToolError::invalid_argument`.
- DEC-007 (SPEC-0046): the engine call becomes `unified_labeled(&old, &new, &la, &lb, context)`; `DEFAULT_CONTEXT` and `unified()` stay for write/edit/multi_edit (`core/diff.rs:11, 48-70`).
- DEC-008 (SPEC-0046): the context value is moved into the existing `spawn_blocking` closure.
- DEC-009 (SPEC-0046): frozen texts copied verbatim into `#[tool(description)]`, the test glue, the arg doc comment and `TOOL_CONTRACT.txt`; `tool-contract-golden.json` is regenerated, never hand edited.
### Data model
None.
### Contracts
Tool `fs.diff_files`, new optional property `context_lines`, last in `properties`, no `type`, `"default": null`.
Property description (verbatim): `Optional. Number of unchanged lines shown around each change, a whole number from 0 to 10000000. Default 3.`
Tool description (verbatim): `Compare two text files (same or different project) and return a unified diff. Shows 3 lines of context by default; context_lines (0 to 10000000) chooses another number of unchanged lines around each change.`
Rust: `pub(crate) const MAX_CONTEXT_LINES: usize = 10_000_000;` `fn parse_context_lines(raw: Option<&Value>) -> Result<usize>`; `diff_files(state, person, a, b, context_lines: Option<&Value>)` (the REST handler, `api/dataplane.rs:383`, passes `None` in this story and is changed in US-0062).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/diff_files.rs` | modify |
| `crates/core/src/mcp/server.rs` | modify |
| `crates/core/src/tools/registry_support/schema.rs` | modify |
| `tool-contract-golden.json` | regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, review the diff |
| `TOOL_CONTRACT.txt` | modify (the `fs.diff_files` block near line 181: description and params line, imitate how `exclude_patterns` is written) |
| `crates/core/src/tools/diff_files_e2e.rs` | modify |
### Existing patterns
- Test style: copy `spec_0045_e2e_001_two_files_of_one_project` (`tools/diff_files_e2e.rs:76`): `let f = world().await; let v = ok_json(diff(&f, ALICE, ("alpha","/notes/a.txt"), ("alpha","/notes/b.txt")).await);`. The helper `diff()` builds `DiffFilesArgs`; extend it (or add `diff_with(.., context_lines: Option<Value>)`) so a test can pass a size; existing callers pass `None`.
- Untyped optional arg precedent: `ExcludePatterns`, `mcp/server.rs:461-488` (custom schema `{}` plus `default: null`). For `Option<Value>` check the rendered schema with the structural golden test `ninety_five_tool_schemas_match_the_golden_contract_structurally` (`mcp/server.rs:4217`).
- Do not touch `dataplane.rs` beyond the one call site: the REST handler passes `None`.

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions (copied from the spec). `|` below stands for a line feed in expected diffs; the real answer separates lines with line feeds. Expected values in the form `--- alpha:/a.txt|+++ ...` are the exact `diff` text. The identical indicator is the answer's `identical` field. Each project and file named below is created by the test itself with `put()`; Alice is a member of alpha and beta, Mallory is not a member of alpha (add members with `f.state.admin.add_member`, as `world()` does).

Fixtures (every line is `l<number>` followed by a line feed unless stated):
- **P1:** project `alpha`: `/a.txt` = `l1`..`l10`; `/b.txt` = the same with line 6 replaced by `L6`; `/same.txt` = identical to `/a.txt`.
- **P2:** project `alpha`: `/c.txt` = `l1`..`l14`; `/d.txt` = the same with line 3 replaced by `L3` and line 12 by `L12`.
- **P3:** project `alpha`: `/e.txt` = `l1`..`l10`; `/f.txt` = same with line 1 replaced by `L1`.
- **P5:** alpha: `/g.txt` = `l1`..`l15`; `/h.txt` = same with line 3 replaced by `L3` and line 10 by `L10`.
- **P6:** alpha: `/i.txt` = `l1`..`l16`; `/j.txt` = same with line 3 replaced by `L3` and line 11 by `L11`.
- **P4:** project `beta`: `/b.txt` = same content as alpha `/b.txt`.
- Actor: `Alice`, member of `alpha` and `beta`; `Mallory` is not a member of `alpha`.
A diff is shown below with `|` as line separator; the real answer separates lines with a line feed.

Refusal message everywhere: `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000` (no value echoed).

### SPEC-0046/E2E-001: size 1
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-MOD-001
- **Given** P1, Alice
- **When** Alice compares alpha `/a.txt` with alpha `/b.txt` with context size 1
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -5,3 +5,3 @@| l5|-l6|+L6| l7`
- **And** the identical indicator is false

### SPEC-0046/E2E-002: no size given
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002, FR-NEW-001, FR-MOD-001
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` without a size
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -3,7 +3,7 @@| l3| l4| l5|-l6|+L6| l7| l8| l9`

### SPEC-0046/E2E-003: size 0
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-002. **Requirements:** FR-NEW-004, FR-NEW-001
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` with size 0
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -6 +6 @@|-l6|+L6`

### SPEC-0046/E2E-004: two projects, size 1
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** P1, P4, Alice
- **When** Alice compares alpha `/a.txt` with beta `/b.txt` with size 1
- **Then** the diff is `--- alpha:/a.txt|+++ beta:/b.txt|@@ -5,3 +5,3 @@| l5|-l6|+L6| l7`

### SPEC-0046/E2E-006: largest accepted size
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001, SC-003. **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-005
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` with size 10000000
- **Then** the diff is `--- alpha:/a.txt|+++ alpha:/b.txt|@@ -1,10 +1,10 @@| l1| l2| l3| l4| l5|-l6|+L6| l7| l8| l9| l10`

### SPEC-0046/E2E-007: changes 8 lines apart, size 3
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 3 (8 unchanged lines between the changes, more than 2 times 3)
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -1,6 +1,6 @@| l1| l2|-l3|+L3| l4| l5| l6|@@ -9,6 +9,6 @@| l9| l10| l11|-l12|+L12| l13| l14` (two blocks)

### SPEC-0046/E2E-008: same changes, size 4
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-001
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 4 (8 equals 2 times 4)
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -1,14 +1,14 @@| l1| l2|-l3|+L3| l4| l5| l6| l7| l8| l9| l10| l11|-l12|+L12| l13| l14` (one block)

### SPEC-0046/E2E-009: change on the first line
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** P3, Alice
- **When** Alice compares `/e.txt` with `/f.txt` with size 3
- **Then** the diff is `--- alpha:/e.txt|+++ alpha:/f.txt|@@ -1,4 +1,4 @@|-l1|+L1| l2| l3| l4`

### SPEC-0046/E2E-010: identical files, size 0
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-002, SC-005. **Requirements:** FR-NEW-006, FR-NEW-004
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` with size 0
- **Then** the identical indicator is true and the diff is empty

### SPEC-0046/E2E-011: negative size
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-002, SC-003. **Requirements:** FR-NEW-003, FR-NEW-004, FR-NEW-002
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` with size `-1`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`
- **And** no diff is returned, in particular not the 3 line one

### SPEC-0046/E2E-012: not a number
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice
- **When** size is `abc`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

### SPEC-0046/E2E-013: decimal
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice
- **When** size is `2.5`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

### SPEC-0046/E2E-014: just above the maximum
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** P1, Alice
- **When** size is `10000001`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

### SPEC-0046/E2E-016: invalid size and missing file
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007, FR-NEW-003
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/missing.txt` with size `-1`
- **Then** refused with the size message, not a missing file message

### SPEC-0046/E2E-017: invalid size by a non member
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-007, FR-NEW-003
- **Given** P1, Mallory (not a member of alpha)
- **When** Mallory compares alpha `/a.txt` with alpha `/b.txt` with size `abc`
- **Then** refused with the size message, the same text Alice gets, revealing nothing about alpha

### SPEC-0046/E2E-019: refusal touches nothing
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** side effect. **Scenario:** SC-003. **Requirements:** FR-NEW-007, FR-NEW-003
- **Given** P1, last access dates of `/a.txt` and `/b.txt` noted, Alice
- **When** Alice compares them with size `-1`
- **Then** refused as in E2E-011
- **And** both files, used storage of alpha and both last access dates are unchanged; a following comparison with size 1 does count as access (SPEC-0045 behavior)

### SPEC-0046/E2E-021: identical files, invalid size
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-006, FR-NEW-003
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` with size `-1`
- **Then** refused with the size message, not the identical answer

### SPEC-0046/E2E-022: identical files, no size
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-006, FR-NEW-002
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` without a size
- **Then** the identical indicator is true and the diff is empty

### SPEC-0046/E2E-026: leading plus sign
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice
- **When** size is `+3`
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`

### SPEC-0046/E2E-027: leading zeros
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** P1 with `/a.txt` and `/b.txt`, Alice
- **When** Alice compares them with size `001`
- **Then** the diff is the one of E2E-001 (size 1)

### SPEC-0046/E2E-028: size given as null on the tool surface
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/b.txt` giving the size as `null`
- **Then** the diff is the one of E2E-002 (3 lines)

### SPEC-0046/E2E-029: size is the number `3.0` written without quotes
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the JSON number `3.0`
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-030: size is the number `1e3` written without quotes
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the JSON number `1e3`
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-032: size is the text ` 3` (leading space)
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the text ` 3` (leading space)
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-033: size is the text `3 ` (trailing space)
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the text `3 ` (trailing space)
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-034: size is the text `0x3`
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the text `0x3`
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-035: size is the boolean `true`
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the boolean `true`
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-036: size is the list `[1]`
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** size is the list `[1]`
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-037: Alice compares `/a.txt` with `/b.txt` on
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-003, FR-NEW-001
- **Given** P1, Alice (or her program client)
- **When** Alice compares `/a.txt` with `/b.txt` once with the number `1` and once with the text `"1"`
- **Then** both diffs are the one of E2E-001

### SPEC-0046/E2E-040: separate changes, size 0
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001, SC-002. **Requirements:** FR-NEW-010, FR-NEW-004
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 0
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -3 +3 @@|-l3|+L3|@@ -12 +12 @@|-l12|+L12`

### SPEC-0046/E2E-041: separate changes, size 1
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** P2, Alice
- **When** Alice compares `/c.txt` with `/d.txt` with size 1
- **Then** the diff is `--- alpha:/c.txt|+++ alpha:/d.txt|@@ -2,3 +2,3 @@| l2|-l3|+L3| l4|@@ -11,3 +11,3 @@| l11|-l12|+L12| l13`

### SPEC-0046/E2E-043: 6 unchanged lines between changes, size 3
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-001
- **Given** P5, Alice
- **When** Alice compares `/g.txt` with `/h.txt` with size 3
- **Then** one block: `--- alpha:/g.txt|+++ alpha:/h.txt|@@ -1,13 +1,13 @@| l1| l2|-l3|+L3| l4| l5| l6| l7| l8| l9|-l10|+L10| l11| l12| l13`

### SPEC-0046/E2E-044: 7 unchanged lines between changes, size 3
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-010, FR-NEW-001
- **Given** P6, Alice
- **When** Alice compares `/i.txt` with `/j.txt` with size 3
- **Then** two blocks: `--- alpha:/i.txt|+++ alpha:/j.txt|@@ -1,6 +1,6 @@| l1| l2|-l3|+L3| l4| l5| l6|@@ -8,7 +8,7 @@| l8| l9| l10|-l11|+L11| l12| l13| l14`

### SPEC-0046/E2E-045: identical files, size 3
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** happy. **Scenario:** SC-005. **Requirements:** FR-NEW-006
- **Given** P1, Alice
- **When** Alice compares `/a.txt` with `/same.txt` with size 3
- **Then** the identical indicator is true and the diff is empty

### SPEC-0046/E2E-046: adjacent changes, size 0
- Level / Driver / Location: `McpServer::fs_diff_files` over real SQLite and local blobs, in `crates/core/src/tools/diff_files_e2e.rs` (helpers `world()`, `put()`, `diff()`, `ok_json()`, `err_text()` exist there)
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-004
- **Given** P7: alpha `/k.txt` = `l1`..`l5`; `/m.txt` = same with lines 2 and 3 replaced by `L2` and `L3`; Alice
- **When** Alice compares `/k.txt` with `/m.txt` with size 0
- **Then** the diff is `--- alpha:/k.txt|+++ alpha:/m.txt|@@ -2,2 +2,2 @@|-l2|-l3|+L2|+L3`

### SPEC-0046/E2E-MOD-001: existing comparison tests without a size
- Level / Driver / Location: functional e2e / unchanged
- **Validated before:** diff with always 3 lines.
- **Validates now:** the same, because no size is given (FR-NEW-002).
- **Given / When / Then:** as the existing tests; no expected value changes. The existing tests pass unmodified.

## Constraints
- Files not to touch: `core/diff.rs` (the engine already takes a size), `api/dataplane.rs` beyond the one call site argument, `api/openapi.rs`, README and `.agent_docs`.
- Dependencies not to add: none.
- Patterns to avoid: parsing in the MCP method; echoing the value in the message; editing an existing test.
- Scope boundary: the web interface (US-0062), write/edit non regression tests, scale test and docs (US-0063).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the 48 existing tests of `crates/core/src/tools/diff_files_e2e.rs` and every other existing test pass unmodified; write, edit and multi edit diffs still show 3 unchanged lines.
- [ ] `rg -n "3 lines of context" crates TOOL_CONTRACT.txt tool-contract-golden.json` shows no stale description in the tool surface files of this story.
- [ ] Tool count unchanged (109); the golden tests and `every_fs_schema_matches_the_frozen_tool_contract` pass without editing test source.
- [ ] Self review: every EARS statement above traced to a test; no `unwrap()` on a fallible path; no dash as punctuation in comments or docs; comments explain why.
- [ ] One commit `US-0061: choose the context size of a comparison`, body `Spec: SPEC-0046` and the requirement ids.
