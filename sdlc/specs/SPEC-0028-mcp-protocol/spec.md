# MCP protocol: Specification (as-built)

> Id: SPEC-0028
> Nature: FEAT
> Status: as-built
> Area: mcp-protocol
> Generated on: 2026-10-14 by /sdlc-retro-spec at bc2b51a
> Confidence: high (see Confidence notes)

## 1. Summary

An agent talks to the server over the Model Context Protocol on one HTTP endpoint. It opens a session
with a handshake, lists the tools this server serves, and calls them. Every exchange answers as a
server sent event stream. A tool that fails answers a normal result marked as an error, while a tool
the server does not serve answers a protocol error. Which tools are served follows the operator's
configuration.

## 2. Current State

### 2.1 How it works today
The caller authenticates on every request. The first request of a session must be the handshake; the
server answers with its identity and opens a session whose id the caller sends on every later request.
After acknowledging the handshake, the caller lists the tools and calls them by their exact names.

### 2.2 Existing specifications governing this area
- SPEC-0002 (platform foundation, as-built) FR-012 to FR-019 describe the protocol layer as it was
  before the migration to the official SDK; they are stale (SPEC-0002's own confidence notes say so).
  This spec describes the current protocol and supersedes them in fact; SPEC-0002 itself is not
  modified here (backlog BL-0028).
- SPEC-0016 (DEBT, implemented) declared the protocol changes of that migration.
- SPEC-0027 (BUG, implemented) made the served tool set follow the configuration.

### 2.3 Existing test coverage
16 requirements: 13 covered by tests at integration or end to end level, 3 untested (E2E-014 to
E2E-016). The baseline gate run on 2026-10-14 stopped at the dependency audit step before the tests
(backlog BL-0012); the same tests were green at the landing of US-0031 (2736 passed, 0 failed).

## 3. Scope
### 3.1 Covered
Authentication of protocol requests, the handshake, session use, tool listing, tool calls and their
result and error shapes, unknown tools and methods, malformed requests, the endpoint path setting.
### 3.2 Not covered
What each tool does (SPEC-0003, SPEC-0006 to SPEC-0009 and the tool contract), the REST data plane
(SPEC-0004), how the identity token is verified (SPEC-0002).

## 4. Actors
- **Agent**: an MCP client acting for a person, holding a bearer token.
- **Operator**: configures the endpoint path and which tool families are enabled.

## 5. Usage Scenarios

### SC-001: Open a session and call a tool
Agent, with a valid token. Sends the handshake, receives the server identity and a session id,
acknowledges the handshake, lists the tools, calls one with its arguments. Outcome: each answer is an
event stream carrying the JSON-RPC response with the request's id.

### SC-002: A call goes wrong
Agent. Calls a served tool that fails, calls a tool name the server does not serve, sends a body that
is not valid JSON-RPC, skips the handshake, or sends no token. Outcome: a distinct, documented answer
for each case.

### SC-003: The operator shapes the surface
Operator. Enables or disables tool families and sets the endpoint path. Outcome: the tool list and
the reachable path follow the configuration.

## 6. Functional Requirements

| ID | Requirement (EARS) | Scenario | Evidence |
|---|---|---|---|
| FR-001 | IF a protocol request carries no bearer token, or an invalid one, THEN THE system SHALL answer HTTP 401 with a JSON body whose `error` is `ERR_UNAUTHENTICATED` and whose `detail` explains why. | SC-002 | `crates/core/src/app.rs:301-325` |
| FR-002 | The system SHALL read the bearer token from the configured identity header (default `X-Forwarded-Authorization`), then from `Authorization`. | SC-001 | `crates/core/src/identity.rs:116-125`, `crates/core/src/config.rs:162` |
| FR-003 | IF the first request of a session is not the handshake THEN THE system SHALL refuse it with an HTTP client error (422) whose body mentions `initialize`. | SC-002 | `crates/core/src/app.rs:283-292` |
| FR-004 | WHEN the handshake succeeds THE system SHALL answer HTTP 200 with the server identity (name `mcp-fs`, the server version), the tools capability, and an `Mcp-Session-Id` response header. | SC-001 | `crates/core/src/mcp/server.rs:39`, `:3742-3747` |
| FR-005 | WHEN the caller sends the handshake acknowledgement (a notification, without id) THE system SHALL answer HTTP 202 with an empty body. | SC-001 | `crates/core/src/app.rs:283-292` |
| FR-006 | WHEN a request is answered within a session THE system SHALL answer `text/event-stream`, each JSON-RPC message in a `data:` line, carrying the request's `id` and `jsonrpc: "2.0"`. | SC-001 | `crates/core/src/app.rs:283-292` |
| FR-007 | WHEN a tool emits progress notifications before its result THE system SHALL send them as earlier events of the same stream, the result last. | SC-001 | `crates/core/src/app.rs:283-292` |
| FR-008 | WHEN the caller lists tools THE system SHALL return exactly the tools served for the current configuration: the file and admin tools always, git tools only with git enabled, search tools only with search enabled, each optional family only with its flag. | SC-003 | `crates/core/src/mcp/server.rs:137-163`, `:65-108` |
| FR-009 | WHEN a served tool succeeds THE system SHALL answer a tool result whose single text content is the tool's JSON output, not marked as an error. | SC-001 | `crates/core/src/mcp/server.rs:46-52` |
| FR-010 | IF a served tool fails THEN THE system SHALL answer a normal tool result marked as an error, whose text is `An error occurred invoking '<tool>': <ERR_CODE>: <message>`, and never a protocol error. | SC-002 | `crates/core/src/mcp/server.rs:46-56` |
| FR-011 | IF a call names a tool the server does not serve THEN THE system SHALL answer the JSON-RPC error code `-32602` with message `tool not found`. | SC-002 | `crates/core/src/mcp/server.rs:137-163` |
| FR-012 | The system SHALL resolve tool names by exact match only: a separator variant (for example an underscore instead of a dot) is a tool it does not serve. | SC-002 | `crates/core/src/mcp/server.rs:137-163` |
| FR-013 | WHEN the caller sends a protocol method outside tools that the server does not implement, such as listing resources, THE system SHALL answer HTTP 200 with the protocol's empty default result. | SC-002 | `crates/core/src/mcp/server.rs:3742-3747` |
| FR-014 | IF the body cannot be decoded as a JSON-RPC message THEN THE system SHALL answer HTTP 415 with a plain text body mentioning `deserialize`. | SC-002 | `crates/core/src/app.rs:283-292` |
| FR-015 | The operator SHALL be able to set the endpoint path (default `/mcp`); a path not starting with `/` stops the server at start. | SC-003 | `crates/core/src/app.rs:102-104`, `:132`, `crates/core/src/config.rs:135-137` |
| FR-016 | The system SHALL keep sessions in the memory of the serving process: a session id is valid only on the process that issued it, until that process restarts. | SC-001 | `crates/core/src/app.rs:292` |

## 7. Non-Functional Requirements
None observed beyond the requirements above.

## 8. End-to-End Tests

| ID | Covers | Test | Level | State |
|---|---|---|---|---|
| E2E-001 | FR-001 | `crates/core/src/app.rs::mcp_without_a_token_is_401_json` | integration | green at last full run |
| E2E-002 | FR-001 | `crates/core/src/app.rs::mcp_with_a_bad_token_is_401` | integration | green at last full run |
| E2E-003 | FR-003 | `crates/core/src/app.rs::mcp_bare_tools_call_without_initialize_is_rejected_mentioning_initialize` (asserts any 4xx, not 422) | integration | green at last full run |
| E2E-004 | FR-003 | `crates/core/src/app.rs::mcp_bare_tools_list_without_initialize_is_rejected_mentioning_initialize` | integration | green at last full run |
| E2E-005 | FR-004, FR-002 | `crates/core/src/app.rs::initialize_establishes_a_session_and_returns_server_identity` | integration | green at last full run |
| E2E-006 | FR-005 | `crates/core/src/app.rs::notifications_after_initialize_are_accepted_with_an_empty_body` | integration | green at last full run |
| E2E-007 | FR-006, FR-009 | `crates/core/src/app.rs::mcp_plain_tool_call_after_initialize_is_sse_framed`, `crates/core/src/app.rs::tools_list_over_an_established_session_is_sse_framed` | integration | green at last full run |
| E2E-008 | FR-007 | `crates/core/src/app.rs::mcp_tool_notification_before_response_yields_multiple_sse_frames` | integration | green at last full run |
| E2E-009 | FR-008 | `crates/core/src/tools/tool_surface_e2e.rs::bt_007_disabled_tools_are_not_listed`, `::bt_020_enabled_git_and_search_are_served_unchanged`, `::bt_010_each_optional_family_is_listed_with_its_catalog_schema` | e2e | green at last full run |
| E2E-010 | FR-010 | `crates/core/src/mcp/server.rs` tests asserting `is_error == Some(true)` (`:4360`) | unit | green at last full run |
| E2E-011 | FR-011, FR-012 | `crates/core/src/app.rs::unknown_tool_is_invalid_params`, `crates/core/src/tools/tool_surface_e2e.rs::bt_012_a_disabled_family_call_is_unknown` | integration | green at last full run |
| E2E-012 | FR-013 | `crates/core/src/app.rs::a_spec_defined_method_outside_tools_gets_rmcps_own_empty_default` | integration | green at last full run |
| E2E-013 | FR-014, FR-015 | `crates/core/src/app.rs::malformed_json_is_rejected_at_the_json_rpc_decode_step`, `::the_mcp_path_is_configurable`, `::a_relative_mcp_path_fails_the_boot` | integration | green at last full run |
| E2E-014 | FR-012 (a separator variant answers `tool not found`) | untested | | |
| E2E-015 | FR-016 (a session id from another or restarted process is refused) | untested | | |
| E2E-016 | FR-004 (protocol version and capability object in the handshake answer) | untested | | |

## 9. Glossary
- **Handshake**: the protocol's `initialize` request, followed by the `notifications/initialized` acknowledgement.
- **Served tool**: a tool the server lists and dispatches for its configuration.
- **Event stream**: an HTTP response of type `text/event-stream`.

## 10. Confidence notes
- FR-003 status: the code relies on the SDK's refusal (HTTP 422, `Unexpected message, expect initialize request`, `.agent_docs/architecture.md`); the local test only asserts a 4xx (E2E-003), so the exact status is from the SDK source, not a local test.
- FR-012 records the owner decision of 2026-10-14 (BL-0028): the separator variants of SPEC-0002/FR-019 were dropped by the migration and the drop is accepted. The CLI agent still resolves loose names on its own side (`crates/agent/src/mcp.rs:259`), which is client behavior, outside this spec.
- HTTP GET (server initiated stream) and DELETE (session end) are allowed by the SDK in this session mode, but no local code path or test exercises them; not written as requirements.
- The handshake answer sets no instructions text; not written as a requirement (absence).
- Several evidence cells cite the service construction (`app.rs:283-292`) because the wire behavior comes from the SDK configured there, not from code in this repository.
- Baseline gate red at the audit step (BL-0012), so test states read "green at last full run" (`make check` at the US-0031 landing, 2736 passed), not this run.
