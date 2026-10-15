# US-0034: Pin the untested MCP protocol behaviors

> Spec: SPEC-0031 (sdlc/specs/SPEC-0031-mcp-protocol-test-coverage/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
Four SPEC-0028 protocol behaviors are not pinned by a test, so an SDK upgrade could change them silently. Add three tests and tighten two assertions in the MCP route test module; no production code changes.

## Requirements covered
- SPEC-0031/DR-001: The test module SHALL contain a test `a_separator_variant_of_a_served_tool_is_not_served` that, over an established session, calls `fs_read` and `fs-read` and asserts for each the JSON-RPC error `code == -32602` and `message == "tool not found"`.
- SPEC-0031/DR-002: The test module SHALL contain a test `a_session_id_from_another_instance_is_not_found` that establishes a session on one app instance, sends a `tools/list` with that `Mcp-Session-Id` to a second app instance built from the same configuration, and asserts status `404` and a body containing `Session not found`.
- SPEC-0031/DR-003: The test module SHALL contain a test `initialize_answers_the_requested_protocol_version_and_the_tools_capability` asserting `result.protocolVersion == "2025-03-26"` and that `result.capabilities.tools` is a JSON object.
- SPEC-0031/DR-004: The two pre handshake tests SHALL assert `r.status() == StatusCode::UNPROCESSABLE_ENTITY` in place of `r.status().is_client_error()`, keeping their `body.contains("initialize")` assertion.
- SPEC-0031/DR-005: The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib app::tests` passes, including the three new test names; each new or tightened assertion was seen red under its mutation (see below).

## Design excerpt
### Decisions
- DEC-001 (SPEC-0031): RED evidence by mutation: change one input temporarily, observe the named failure, restore.
- DEC-002 (SPEC-0031): the foreign session uses a second app instance built from a clone of the same configuration (same keys, token stays valid), not a random id.
- DEC-003 (SPEC-0031): test both `fs_read` and `fs-read`.
### Contracts (observed, from rmcp 3.5.0)
- Unknown session: `404`, body `Not Found: Session not found` (`rmcp-3.5.0/src/transport/streamable_http_server/tower.rs:2054-2057`).
- Pre handshake: `422` (`rmcp-3.5.0/src/transport/common/server_side_http.rs:200`).
- Handshake echoes a supported requested version (`rmcp-3.5.0/src/service/server.rs:491-492`); request uses `"2025-03-26"` (`crates/core/src/app.rs:455`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/app.rs` | add 3 tests after `unknown_tool_is_invalid_params` (`:664`); replace the assertion at `:547` and `:561` |
### Existing patterns
- `test_setup` (`app.rs:337`) returns `(ServerConfig, token)`; `build(c)` builds the router; `rpc(token, session, payload)` (`:439`); `sse_json` (`:427`); `establish_session(&app, &t)` (`:461`).
- `unknown_tool_is_invalid_params` (`app.rs:664`) for the `-32602` assertion shape; `initialize_establishes_a_session_and_returns_server_identity` (`:567`) for reading the initialize answer.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0031/DT-010: Separator variant is not served
- Given an established session / When `tools/call` names `fs_read`, then `fs-read` (arguments `{"mount_id":"p","path":"/x"}`) / Then each answer has `error.code == -32602`, `error.message == "tool not found"`.
- RED by mutation: name `fs.read`; the answer carries `result`, the code assertion fails.

### SPEC-0031/DT-011: A foreign session id is not found
- Given a session established on instance A / When `tools/list` with that id goes to instance B (`build(c.clone())`) / Then status `404` and body contains `Session not found`.
- RED by mutation: send to instance A; status `200`, the status assertion fails.

### SPEC-0031/DT-012: Handshake answers version and capability
- Then `v["result"]["protocolVersion"] == "2025-03-26"` and `v["result"]["capabilities"]["tools"].is_object()`.
- RED by mutation: assert `"2025-06-18"`; the equality fails.

### SPEC-0031/DT-013: Pre handshake refusal is exactly 422
- Then both tests assert `assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);`.
- RED by mutation: assert `StatusCode::BAD_REQUEST`; both fail with `left: 422` / `right: 400`.

## Constraints
- Only `crates/core/src/app.rs` changes outside `sdlc/`; no production code.
- No new dependency.
- If an observed value differs from the Contracts above, stop and report BLOCKED (the as-built spec would be wrong), do not adapt the assertion silently.

## Definition of done
- [ ] RED evidence: each mutation run recorded in `.implement/US-0034-red.log`, then restored.
- [ ] DT-010..DT-013 green; `make check` green with `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`, no `skipping` DB suite.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 2 (the two pre handshake tests, assertion tightened only).
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: DT-010..DT-013 and 8.1 green; DR-001..DR-005 cited here.
- [ ] One commit `US-0034: Pin the untested MCP protocol behaviors`, body `Spec: SPEC-0031` and the DR ids.
