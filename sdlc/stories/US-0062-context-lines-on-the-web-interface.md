# US-0062: the same context size on the web interface

> Spec: SPEC-0046 (sdlc/specs/SPEC-0046-diff-context-lines/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0061
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A program client can add `context_lines` to `GET /api/fs/diff` and gets the same answers and the same refusals as the tool surface; the first occurrence of a repeated parameter counts and an empty value is refused.

## Requirements covered
- SPEC-0046/FR-NEW-003: IF the context size is not a whole number from 0 to 10000000 THEN THE product SHALL refuse the comparison.
- SPEC-0046/FR-NEW-007: The product SHALL check the context size before examining any project, path or file, and on the web interface right after the check for missing values.
- SPEC-0046/FR-NEW-009: The product SHALL give a program client, for the same files and the same context size, the same answer and the same refusals as the tool surface.

## Independent Test
Run `make check`; `GET /api/fs/diff` for alpha `/a.txt` and `/b.txt` with `context_lines=1` returns the same body as the tool (E2E-005) and `context_lines=` returns 400 with the pinned message (E2E-015).

## Design excerpt
### Decisions
- DEC-001 (SPEC-0046): the validator lives in `tools::diff_files::diff_files` (US-0061); this story only passes the raw value.
- DEC-004 (SPEC-0046): the REST handler reads the FIRST `context_lines` pair raw from the query vector `Vec<(String,String)>`; absent means `None`; present, even empty, means `Some(Value::String(raw))`. It runs after the existing missing value check. `Q::opt` skips empty values (`api/dataplane.rs:272-274`) and must not be used.
- DEC-009 (SPEC-0046): OpenAPI lists the new optional query parameter with the frozen property description.
### Data model
None.
### Contracts
`GET /api/fs/diff?mount_id_a=&path_a=&mount_id_b=&path_b=[&context_lines=<text>]`: same answer `{identical, diff}`, refusals as 400 `ERR_INVALID_ARGUMENT` with the message `'context_lines' must be a whole number from 0 to 10000000`. A missing value among the four still wins, named first in the order `mount_id_a`, `path_a`, `mount_id_b`, `path_b` (`'<name>' is required`). Percent decoding is done by axum's `Query<Vec<(String,String)>>`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (`diff` handler `:365-387`, tests) |
| `crates/core/src/api/openapi.rs` | modify (`Op` `sub: "diff"` near `:543`, add a `Param` for `context_lines`; keep its description equal to the frozen one) |
| `crates/core/src/tools/diff_files_e2e.rs` | modify (E2E-042 only) |
### Existing patterns
- Handler today: `let value = |name: &str| q.opt(name).filter(|v| !v.trim().is_empty()); ... crate::tools::diff_files::diff_files(&state, &person, a, b, None).await` (after US-0061). Add: `let context = q.0.iter().find(|(k, _)| k == "context_lines").map(|(_, v)| serde_json::Value::String(v.clone()));` and pass `context.as_ref()` after the missing value check.
- OpenAPI `Param { name, required, ty, format, default }` with `Def::Null` default: follow the neighbouring `Param`s; if the OpenAPI tests compare params to the tool schema, make them agree (the tool schema has no type: use the closest accepted `ty`).

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

### SPEC-0046/E2E-005: web interface, size 1
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** happy. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-001
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison of alpha `/a.txt` and alpha `/b.txt` with size `1`
- **Then** the answer equals that of E2E-001, field for field

### SPEC-0046/E2E-015: web interface, empty size
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-003, FR-NEW-009, FR-NEW-002
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison with all four values and an empty size
- **Then** refused with `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`, and no 3 line diff

### SPEC-0046/E2E-018: web interface, invalid size
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-003
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison with size `abc`
- **Then** the refusal is the same as E2E-012

### SPEC-0046/E2E-023: missing value wins over invalid size
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-007, FR-NEW-009
- **Given** P1, a program client acting for Alice
- **When** it requests the comparison omitting the second path and giving size `abc`
- **Then** refused naming the second path as required (SPEC-0045 FR-NEW-018 text), not the size message

### SPEC-0046/E2E-038: a program client requests the comparison
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-003, FR-NEW-009
- **Given** P1, Alice (or her program client)
- **When** a program client requests the comparison with the size given twice, first empty then `2`
- **Then** refused with the message of E2E-011 (the first occurrence, empty, counts)

### SPEC-0046/E2E-039: a program client requests the comparison
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-003
- **Given** P1, Alice (or her program client)
- **When** a program client requests the comparison with the size ` ` (one space)
- **Then** refused with the message of E2E-011

### SPEC-0046/E2E-042: negative zero
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-003, FR-NEW-009
- **Given** P1, Alice and a program client acting for her
- **When** Alice gives the number `-0` without quotes, and the client gives the text `-0`
- **Then** both are refused with the size message (message of E2E-011)

### SPEC-0046/E2E-047: web interface, size 0
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-004
- **Given** P1, a program client acting for Alice
- **When** it requests `/a.txt` against `/b.txt` with size `0`
- **Then** the answer equals that of E2E-003

### SPEC-0046/E2E-048: web interface, size 10000000
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-001
- **Given** P1, a program client acting for Alice
- **When** it requests `/a.txt` against `/b.txt` with size `10000000`
- **Then** the answer equals that of E2E-006

### SPEC-0046/E2E-049: web interface, leading zeros
- Level / Driver / Location: the real axum router built as the existing `api/dataplane.rs` tests do (see the `diff` tests around `crates/core/src/api/dataplane.rs:1902` and `:2243` for the request helper and Bearer setup); E2E-042 also drives `McpServer::fs_diff_files` in `tools/diff_files_e2e.rs`
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009, FR-NEW-003
- **Given** P1, a program client acting for Alice
- **When** it requests `/a.txt` against `/b.txt` with size `001`
- **Then** the answer equals that of E2E-001

## Constraints
- Files not to touch: `tools/diff_files.rs` (validator), `mcp/server.rs`, contract files.
- Dependencies not to add: none.
- Patterns to avoid: `Q::opt` or `Q::int_or` for this value; a second parser.
- Scope boundary: docs and the closing audit are US-0063.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the 48 existing tests of `crates/core/src/tools/diff_files_e2e.rs` and every other existing test pass unmodified; write, edit and multi edit diffs still show 3 unchanged lines.
- [ ] `api/openapi.rs` tests pass and the OpenAPI document lists `context_lines` for `/api/fs/diff`.
- [ ] Self review: every EARS statement above traced to a test; no `unwrap()` on a fallible path; no dash as punctuation in comments or docs; comments explain why.
- [ ] One commit `US-0062: the same context size on the web interface`, body `Spec: SPEC-0046` and the requirement ids.
