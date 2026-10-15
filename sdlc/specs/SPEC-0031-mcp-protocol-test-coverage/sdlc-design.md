# Cover the untested MCP protocol requirements: technical debt design

> Id: SPEC-0031
> Nature: DEBT
> Depth: S
> Depth evidence: one debt item, one file (`crates/core/src/app.rs`, test module only), no public contract, no persisted data
> Status: implemented
> Generated on: 2026-10-15
> Behavior change: none
> From backlog: BL-0052
> Depends on: SPEC-0028 (as-built baseline)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt

### DBT-001: Four SPEC-0028 protocol requirements are not pinned by a test
- **Current structure:** the MCP route tests live in `crates/core/src/app.rs` (test module, helpers `test_setup` `:337`, `sse_json` `:427`, `rpc` `:439`, `INITIALIZE_BODY` `:455`, `establish_session` `:461`). Gaps against SPEC-0028:
  - FR-012 (exact names only): `unknown_tool_is_invalid_params` (`:664`) checks a name that resembles nothing; no test sends a separator variant of a served tool.
  - FR-016 (sessions local to the process): no test sends a session id the serving instance did not issue. rmcp answers `404` with body `Not Found: Session not found` (`rmcp-3.5.0/src/transport/streamable_http_server/tower.rs:2054-2057`).
  - FR-004 (handshake answer): `initialize_establishes_a_session_and_returns_server_identity` (`:567`) asserts only `serverInfo`. The request carries `protocolVersion: "2025-03-26"` (`:455`); rmcp echoes a supported requested version (`rmcp-3.5.0/src/service/server.rs:491-492`); the server declares the tools capability only (`crates/core/src/mcp/server.rs:3743-3746`).
  - FR-003 (pre handshake refusal is 422): the two tests `mcp_bare_tools_call_without_initialize_is_rejected_mentioning_initialize` (`:535`) and `mcp_bare_tools_list_without_initialize_is_rejected_mentioning_initialize` (`:553`) assert `r.status().is_client_error()` (`:547`, `:561`); rmcp answers `UNPROCESSABLE_ENTITY` (`rmcp-3.5.0/src/transport/common/server_side_http.rs:200`).
- **Cost paid today:** an SDK upgrade or a router change can bring back separator aliasing (silently dropped once, BL-0028), change session or handshake semantics, or move 422 to 400, and the suite stays green.
- **Target structure:** three new tests and two tightened assertions in the same test module, reusing the existing helpers.
- **Observable behavior:** unchanged (test code only).

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/app.rs` (test module) | add 3 tests (DT-010, DT-011, DT-012); tighten 2 assertions (DT-013, lines `:547`, `:561`) | 5 |

**Non source occurrences:** none.
**Contracts touched:** none; tests only.
**Guardrail coverage:** the 15 route and boot tests in `crates/core/src/app.rs` (`:485` to `:757`) plus the full suite. **Test command:** see 8.1.
**Total:** 1 file, 5 changes, recounted.

## 3. Class sweep
Not a security lot.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-U]: Separator variant pinned
> The test module SHALL contain a test `a_separator_variant_of_a_served_tool_is_not_served` that, over an established session, calls `fs_read` and `fs-read` and asserts for each the JSON-RPC error `code == -32602` and `message == "tool not found"`.

#### DR-002 [EARS-U]: Foreign session pinned
> The test module SHALL contain a test `a_session_id_from_another_instance_is_not_found` that establishes a session on one app instance, sends a `tools/list` with that `Mcp-Session-Id` to a second app instance built from the same configuration, and asserts status `404` and a body containing `Session not found`.

#### DR-003 [EARS-U]: Handshake answer pinned
> The test module SHALL contain a test `initialize_answers_the_requested_protocol_version_and_the_tools_capability` asserting `result.protocolVersion == "2025-03-26"` and that `result.capabilities.tools` is a JSON object.

#### DR-004 [EARS-U]: Pre handshake status pinned
> The two pre handshake tests SHALL assert `r.status() == StatusCode::UNPROCESSABLE_ENTITY` in place of `r.status().is_client_error()`, keeping their `body.contains("initialize")` assertion.
- **Old form:** `assert!(r.status().is_client_error(), "expected a 4xx, got {}", r.status());`
- **New form:** `assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);`
- **Applies to:** `app.rs:547`, `app.rs:561`

### 4.2 Invariance
#### DR-005 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

## 6. Decisions
- **DEC-001:** RED evidence by mutation, since the tests pin behavior that already holds and would pass on first run. For each test, change one input temporarily, observe the named failure, restore. **Alternatives considered:** accepting green on first run, rejected because a test never seen red proves nothing; breaking production code to see red, rejected as out of scope for a test only lot. **Implemented by:** DR-001..DR-004. **Evidence:** n/a.
- **DEC-002:** The foreign session is proven with a second app instance built from a clone of the same configuration (same keys, so the token stays valid and only the session differs), not with a random id. **Alternatives considered:** a random id, rejected because it does not show the restart semantics FR-016 states; restarting one instance in place, rejected as heavier with the same signal. **Implemented by:** DR-002. **Evidence:** `crates/core/src/app.rs:337` (`test_setup` returns the config), `:461` (`establish_session`).
- **DEC-003:** `fs-read` is tested alongside `fs_read`: both separator variants a client could plausibly send. **Alternatives considered:** `fs_read` only, rejected as narrower than FR-012's wording. **Implemented by:** DR-001. **Evidence:** n/a.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the rows in 8.4:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None.

### 8.3 Structural tests
#### DT-010: Separator variant is not served
- **Validates:** DR-001, SPEC-0028/FR-012.
- **Given** an established session / **When** `tools/call` names `fs_read`, then `fs-read` / **Then** each answer is `error.code == -32602`, `error.message == "tool not found"`.
- **RED by mutation:** name `fs.read` with valid arguments; the answer carries `result`, not `error`, so the code assertion fails.

#### DT-011: A foreign session id is not found
- **Validates:** DR-002, SPEC-0028/FR-016.
- **Given** a session established on instance A / **When** `tools/list` with that id is sent to instance B (same configuration) / **Then** status `404`, body contains `Session not found`.
- **RED by mutation:** send it to instance A; status `200`, so the status assertion fails.

#### DT-012: Handshake answers version and capability
- **Validates:** DR-003, SPEC-0028/FR-004.
- **Given** the standard initialize request (`protocolVersion "2025-03-26"`) / **Then** `result.protocolVersion == "2025-03-26"` and `result.capabilities.tools.is_object()`.
- **RED by mutation:** assert `"2025-06-18"`; the equality fails.

#### DT-013: Pre handshake refusal is exactly 422
- **Validates:** DR-004, SPEC-0028/FR-003.
- **Then** both tests assert `StatusCode::UNPROCESSABLE_ENTITY`.
- **RED by mutation:** assert `StatusCode::BAD_REQUEST`; both fail with `left: 422, right: 400`.

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/app.rs` | `mcp_bare_tools_call_without_initialize_is_rejected_mentioning_initialize` | `is_client_error()` to `== UNPROCESSABLE_ENTITY` (`:547`) | strengthening only (DR-004), BL-0052 named the weak assertion |
| `crates/core/src/app.rs` | `mcp_bare_tools_list_without_initialize_is_rejected_mentioning_initialize` | same (`:561`) | same |

## 9. Implementation order
1. Add DT-012 and DT-013 (single instance tests), run them, record the mutation REDs.
2. Add DT-010 and DT-011, same.
3. 8.1.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 1 | IMPLEMENTABLE |

Inline checklist (depth S): 1 PASS (exact values for every assertion), 2 PASS (test names and both assertion forms spelled), 3 n/a, 4 PASS, 5 n/a, 6 PASS, 7 PASS, 8 PASS (tests only), 9 PASS (lines recounted), 10 PASS (two instance build uses existing helpers), 11 PASS, 12 PASS, 13 n/a, 14 PASS (two declared, strengthening only).

**Amendments applied:** A (count): guardrail test count in Section 2 recounted to 15.

## Design additions (2026-10-15)

### File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/app.rs` (test module) | add 3 tests, tighten 2 assertions | DR-001..DR-004 | US-0034 |

Docs: none (tests only).

### Test strategy

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-010 | integration | axum `oneshot` over the real router | `crates/core/src/app.rs::tests::a_separator_variant_of_a_served_tool_is_not_served` | US-0034 |
| DT-011 | integration | two routers from one config | `crates/core/src/app.rs::tests::a_session_id_from_another_instance_is_not_found` | US-0034 |
| DT-012 | integration | axum `oneshot` | `crates/core/src/app.rs::tests::initialize_answers_the_requested_protocol_version_and_the_tools_capability` | US-0034 |
| DT-013 | integration | axum `oneshot` | the two `mcp_bare_tools_*_without_initialize_*` tests | US-0034 |
| 8.1 | non regression | `make check` with DB DSNs | workspace | US-0034 |

- **DEC-004:** One story, `min_tier: 2`. **Obliged by:** SPEC-0031/DR-001..DR-005. **Rationale:** one file, helpers exist, five small changes that share setup; splitting would add four stories for one test module. **Alternatives considered:** one story per test, rejected as fragmentation of one file. **Source:** design. **Evidence:** `crates/core/src/app.rs:337`, `:439`, `:461`.

### Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0034 | Pin the untested MCP protocol behaviors | P1 | DR-001..DR-005 | main | 2 | yes |
