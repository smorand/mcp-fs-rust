# SPEC-0046: choose the context size of a file comparison, design

> Spec: sdlc/specs/SPEC-0046-diff-context-lines/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-20
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust, edition 2024 (existing) | `rust-toolchain.toml` | DEC-010 |
| MCP | rmcp `#[tool]` on `McpServer` (existing) | 3.5.0 (`Cargo.toml`) | DEC-002 |
| HTTP | axum data plane (existing) | `Cargo.lock` | DEC-004 |
| Diff | `similar` Myers (existing), `core::diff::unified_labeled(.., context: usize)` | `Cargo.toml:86` | DEC-007 |

No dependency is added or changed, so no context7 lookup. Skills loaded: `rust` (via `load_dynamic_context`).
Effect: functional e2e through the real handlers, no `unwrap()` on fallible paths, CPU work stays in
`spawn_blocking`, `make check` is the gate. Where the project's `AGENTS.md` differs (own `./test.sh`, `cargo fmt`,
`cargo clippy --all-targets --all-features -- -D warnings`), the project wins.

## 2. Approaches
### Chosen: one validator in the shared orchestrator, an untyped optional argument
`tools/diff_files.rs::diff_files` gains `context_lines: Option<&Value>`, validates it first (before `normalize_path`),
and hands the resulting `usize` to the engine that already exists (`unified_labeled`, `core/diff.rs:62`). Both doors
(MCP and REST) pass the raw value; neither parses. Pros: one implementation (AGENTS.md convention), one place for
the check order (FR-NEW-007). Cons: the argument is `Option<Value>`, so the published schema carries no type.
Affected scope: `tools/diff_files.rs`, `mcp/server.rs`, `tools/registry_support/schema.rs`, `api/dataplane.rs`,
`api/openapi.rs`, the two contract files, docs.

### Rejected: a typed `Option<u32>` argument
Simple, but rmcp rejects `-1`, `abc`, `2.5`, `true` at deserialization, with a protocol error instead of the
FR-NEW-003 message, and the spec pins an open schema (Section 14.4). Rejected (DEC-002).

### Rejected: parse in each door
Two parsers drift (the AGENTS.md rule against it). Rejected (DEC-001).

## 3. Architecture
```
McpServer::fs_diff_files --\
                            >-- tools::diff_files::diff_files(state, person, a, b, context_lines: Option<&Value>)
REST GET /api/fs/diff -----/        1. parse_context_lines  (refusal first, DEC-001)
                                    2. normalize paths, authorize, read, touch (unchanged, SPEC-0045)
                                    3. spawn_blocking: unified_labeled(.., context)  (DEC-007, DEC-008)
```
Bounded contexts: file storage (engine, validator), access control (unchanged), programmatic access (REST).

## 4. Data model
None. No persisted concept changes.

## 5. Contracts and interfaces
- Tool `fs.diff_files`: new optional property `context_lines`, last in `properties`, no `type`, `"default": null`,
  description: `Optional. Number of unchanged lines shown around each change, a whole number from 0 to 10000000. Default 3.`
  Tool description: `Compare two text files (same or different project) and return a unified diff. Shows 3 lines of context by default; context_lines (0 to 10000000) chooses another number of unchanged lines around each change.`
  `required` stays `["mount_id_a","path_a","mount_id_b","path_b"]`.
- REST `GET /api/fs/diff?...&context_lines=<text>`: same answer, same refusal body shape as other
  `ERR_INVALID_ARGUMENT` refusals (400).
- Refusal message (single constant): `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`.
- Rust: `pub(crate) const MAX_CONTEXT_LINES: usize = 10_000_000;`
  `fn parse_context_lines(raw: Option<&Value>) -> Result<usize>`; `diff_files(state, person, a, b, context_lines: Option<&Value>)`.
- Acceptance rule (DEC-003): `None` or `Value::Null` gives `DEFAULT_CONTEXT` (3). `Value::Number` accepted only when
  `n.is_u64()` and value <= max (so `3.0`, `1e3`, `-1`, `2.5`, `-0` are refused: serde_json reads a negative zero as an f64,
  `serde_json-1.x/src/de.rs:517-523`).
  `Value::String` accepted only when non empty, every byte `b'0'..=b'9'`, and the value parses and is <= max (leading zeros
  allowed; overflow refused). `Bool`, `Array`, `Object` refused.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/diff_files.rs` | modify (validator, param, description in test glue) | FR-NEW-001..007, FR-MOD-001 | US-0061 |
| `crates/core/src/mcp/server.rs` | modify (`DiffFilesArgs.context_lines`, tool description, pass raw value) | FR-NEW-003, FR-MOD-001 | US-0061 |
| `crates/core/src/tools/registry_support/schema.rs` | modify (`ParamType::Any`, `opt_any`) | DEC-005 | US-0061 |
| `tool-contract-golden.json` | regenerate | FR-MOD-001 | US-0061 |
| `TOOL_CONTRACT.txt` | modify (line 181 to 185 block) | FR-MOD-001 | US-0061 |
| `crates/core/src/tools/diff_files_e2e.rs` | modify (new tests) | all | US-0061, US-0062, US-0063 |
| `crates/core/src/api/dataplane.rs` | modify (`diff` handler) | FR-NEW-009 | US-0062 |
| `crates/core/src/api/openapi.rs` | modify (`Op` for `diff`, new `Param`) | FR-NEW-009 | US-0062 |
| `README.md`, `.agent_docs/tools.md`, `.agent_docs/api.md` | modify | FR-MOD-001 | US-0063 |

Affected existing tests: `ninety_five_tool_schemas_match_the_golden_contract_structurally` (`mcp/server.rs:4217`),
`every_fs_schema_matches_the_frozen_tool_contract` (`tools/mod.rs:505`) and the golden tests in
`tools/contract_golden.rs` change expectations only through the regenerated golden file; no test source is edited.
The 48 tests of `tools/diff_files_e2e.rs` pass unmodified (E2E-MOD-001). Documentation: README.md:276,
`.agent_docs/tools.md:102`, `.agent_docs/api.md:82,90`. `AGENTS.md` counts do not change (no new tool).

## 7. Compatibility and risks
Additive: omitting the argument gives today's bytes (FR-NEW-002). The published schema and description change, so a
client caching the schema sees a new optional property. Risk 1: `-0` and large literals depend on serde_json number
parsing; covered by E2E-042 and E2E-014. Risk 2: the schemars output for `Option<Value>` must equal the registry glue
rendering (`description`, then `default: null`, no `type`); signal: the structural golden test. Rollback: revert the
commit; no data touched.

## 8. Security design
Not applicable (`Security: n/a`).

## 9. Test strategy
Every acceptance test is a functional e2e through `McpServer::fs_diff_files` (the real handler) over real SQLite and
local blobs, in `crates/core/src/tools/diff_files_e2e.rs`; web tests drive the real axum router the way the existing
dataplane tests do (`api/dataplane.rs` test harness, see its `diff` tests near line 1902 and 2243). Level and story
per test:

| Tests | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001..004, 006..014, 016, 017, 019, 021, 022, 026..030, 032..037, 040, 041, 043..046, E2E-MOD-001 | functional e2e | `McpServer::fs_diff_files` | `tools/diff_files_e2e.rs` | US-0061 |
| E2E-005, 015, 018, 023, 038, 039, 042, 047, 048, 049 | functional e2e | REST router (`GET /api/fs/diff`) and `fs_diff_files` for 042 | `api/dataplane.rs` tests and `tools/diff_files_e2e.rs` | US-0062 |
| E2E-020, 024, 025, 031 | functional e2e | `McpServer` write/edit/multi_edit and `fs_diff_files` | `tools/diff_files_e2e.rs` | US-0063 |

E2E-MOD-001 is the existing 48 tests, run unmodified, and is asserted in US-0061 (first story that can break them).
Test data: generated in test code (fixtures P1..P7 of the spec). E2E-031 builds two 5242880 byte files in memory.

## 10. Decisions log
- **DEC-001:** Validate `context_lines` inside `tools::diff_files::diff_files`, as its first statement, via `parse_context_lines`. **Obliged by:** SPEC-0046/FR-NEW-003, FR-NEW-007, FR-NEW-009. **Rationale:** one implementation for both doors, refusal before any project or file is examined. **Alternatives considered:** parse in each door (drift). **Source:** AGENTS.md Conventions (one implementation). **Evidence:** `crates/core/src/tools/diff_files.rs:26-34`.
- **DEC-002:** `DiffFilesArgs.context_lines: Option<serde_json::Value>` with `#[serde(default)]` and the frozen description as doc comment. **Obliged by:** SPEC-0046/FR-NEW-003, Section 14.4. **Rationale:** an open schema lets `-1`, `abc`, `2.5`, `true`, `[1]` reach the handler's own message. **Alternatives considered:** `Option<u32>` (protocol level error), `Option<String>` (a JSON number is refused by rmcp). **Source:** spec Section 14.4. **Evidence:** `crates/core/src/mcp/server.rs:625-634`; precedent for an untyped optional: `exclude_patterns`, `mcp/server.rs:461-488`.
- **DEC-003:** Acceptance rule as in Section 5: only `Value::Number` with `is_u64()` or a digit only non empty `Value::String`, both <= `MAX_CONTEXT_LINES`; `None` and `Null` mean the default. `-0` is read by serde_json as an f64 (`de.rs:517-523`), so `is_u64` already refuses it; E2E-042 pins it. **Obliged by:** FR-NEW-003. **Rationale:** the message does not echo the value, so only accept/refuse matters. **Alternatives considered:** `as_i64` plus range (accepts `-0`), `str::parse::<usize>` on strings (accepts `+3`). **Source:** design. **Evidence:** `Cargo.toml:24` (no `arbitrary_precision`).
- **DEC-004:** The REST handler reads the first `context_lines` pair raw from the query vector; absent means `None`; present (even empty) means `Some(Value::String(raw))`. It runs after the existing missing value check. **Obliged by:** FR-NEW-003, FR-NEW-007, FR-NEW-009. **Rationale:** `Q::opt` skips empty values (`api/dataplane.rs:272-274`), which would turn `context_lines=` into "not given". **Alternatives considered:** `Q::int_or` (other message, empty ignored). **Source:** existing code. **Evidence:** `api/dataplane.rs:272-274, 291, 365-385`.
- **DEC-005:** Add `ParamType::Any` and `ToolSchema::opt_any(name, desc)` rendering `description` then `default: null`, no `type`, for the test registry glue and the frozen contract. **Obliged by:** FR-MOD-001 (published contract). **Rationale:** the glue must equal the golden and the rmcp router schema. **Alternatives considered:** reuse `opt_flexible_str_array` (renders identically but misnamed). **Source:** design. **Evidence:** `tools/registry_support/schema.rs:165-168, 200`.
- **DEC-006:** One message constant `'context_lines' must be a whole number from 0 to 10000000` built from `MAX_CONTEXT_LINES`, no echo of the value. **Obliged by:** FR-NEW-003. **Rationale:** the spec pins it and removed the echo (tool surface cannot quote a number as written). **Alternatives considered:** echo (rejected in SPEC-0046 round 4). **Source:** spec. **Evidence:** `errors.rs` `ToolError::invalid_argument`.
- **DEC-007:** The engine call becomes `unified_labeled(&old, &new, &la, &lb, context)`; `DEFAULT_CONTEXT` and `unified()` stay for write/edit/multi_edit. **Obliged by:** FR-NEW-001, FR-NEW-008. **Rationale:** the engine already takes a size; write/edit diffs must not move. **Alternatives considered:** changing `unified()` (breaks FR-NEW-008). **Source:** existing code. **Evidence:** `core/diff.rs:11, 48-51, 62-70`.
- **DEC-008:** The context value is moved into the existing `spawn_blocking` closure. **Obliged by:** FR-NEW-001. **Rationale:** unchanged threading model. **Alternatives considered:** none. **Source:** rust skill. **Evidence:** `tools/diff_files.rs:61-66`.
- **DEC-009:** Frozen text lives once as constants where possible and is copied verbatim into `#[tool(description)]`, the test glue, the arg doc comment, `TOOL_CONTRACT.txt`, README and `.agent_docs`; `tool-contract-golden.json` is regenerated, never hand edited. **Obliged by:** FR-MOD-001. **Rationale:** AGENTS.md: descriptions are frozen, a contract change is deliberate. **Alternatives considered:** none. **Source:** AGENTS.md Conventions. **Evidence:** `AGENTS.md` ("tool-contract-golden.json").
- **DEC-010:** No new dependency, no toolchain change. **Obliged by:** n/a (no code impact): records the absence of a category 2 decision. **Rationale:** the engine and `serde_json` already suffice. **Alternatives considered:** a numeric parsing crate (needless). **Source:** manifests. **Evidence:** `Cargo.toml:24, 86`.
- **DEC-011:** Three stories: tool surface (US-0061), REST (US-0062), write/edit non regression, scale and docs, closing (US-0063). **Obliged by:** FR-NEW-009, FR-NEW-008. **Rationale:** the REST door depends on the validator; the contract regeneration must land with the schema change to keep the golden tests green. **Alternatives considered:** one story (about 9 files, over budget); contract files in their own story (red golden tests in between). **Source:** design. **Evidence:** Section 6.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0061 | choose the context size of a comparison | P1 | FR-NEW-001..007, FR-NEW-010, FR-MOD-001 | main | 2 | no |
| US-0062 | the same context size on the web interface | P1 | FR-NEW-003, FR-NEW-007, FR-NEW-009 | US-0061 | 2 | no |
| US-0063 | other diffs unchanged, scale, docs, spec close | P2 | FR-NEW-008, closing audit | US-0061, US-0062 | 2 | yes |
