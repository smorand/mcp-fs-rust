# US-0051: send the Accept header on the full-stack e2e MCP calls

> Spec: SPEC-0043 (sdlc/specs/SPEC-0043-full-stack-e2e-accept-header/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
`make test-e2e-full` dies at the MCP `initialize` call with `http 406 Not Acceptable` because the test's `/mcp` requests carry no `Accept` header. Make the test send it so the only PostgreSQL + S3 + git end to end test runs again.

## Requirements covered
- SPEC-0043/BR-001: WHEN a developer runs `make test-e2e-full` with PostgreSQL and an S3 compatible store reachable and the four environment variables of `.agent_docs/testing.md` set, THE full-stack e2e test SHALL complete the MCP handshake and run its whole scenario to a pass, printing `test result: ok. 1 passed`.
- SPEC-0043/BR-002: The full-stack e2e test SHALL send the header `Accept: application/json, text/event-stream` on every request it makes to the `/mcp` endpoint, the `initialize`, `notifications/initialized` and `tools/call` requests included.
- SPEC-0043/BR-003: The fix SHALL NOT change the server's treatment of a request to `/mcp` that lacks the `Accept` header, and SHALL NOT remove the `#[ignore]` attribute of `full_stack_lifecycle`.

## Independent Test
Start infra (below), run `make test-e2e-full`: output has `test full_stack_lifecycle ... ok` and `test result: ok. 1 passed; 0 failed`, exit 0. Then `make check` stays green (the test is ignored there).

## Design excerpt
### Decisions
- DEC-002 (SPEC-0043): set the header per `/mcp` request through a constant, not as a client wide default (REST calls keep their own headers).
- DEC-003 (SPEC-0043): leave the server and its 406 as is.
### Contracts
`const MCP_ACCEPT: &str = "application/json, text/event-stream";` private, in `crates/core/tests/full_stack_e2e.rs` after `const ADMIN` (`:64`).
### Paths
| Path | Action |
|---|---|
| `crates/core/tests/full_stack_e2e.rs` | add `MCP_ACCEPT`; add `.header("Accept", MCP_ACCEPT)` right after `.bearer_auth(token)` on the three `/mcp` POSTs (`:169-172` initialize, `:185-189` notifications/initialized, `:222-227` tools/call) |
| `.agent_docs/testing.md` | one dated line in the full-stack section (`:224-249`): the `/mcp` client must send `Accept: application/json, text/event-stream`, else the server answers 406 (2026-10-19) |
### Existing patterns
`crates/agent/src/mcp.rs:116`: `.header("Accept", "application/json, text/event-stream")`.

## Acceptance tests
> Run through `make` targets. The e2e needs live infra; set it up as below.

Infra: `docker compose -f docker-compose.test.yml up -d postgres`; S3 on `http://127.0.0.1:9000` (MinIO if an image is reachable, else `uvx --from 'moto[server]' moto_server -H 127.0.0.1 -p 9000 &`, see `.agent_docs/testing.md:206-214`). Env: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_MINIO_SECRET_KEY=any MCPFS_MINIO_ENDPOINT=http://127.0.0.1:9000 MCPFS_MINIO_ACCESS_KEY=admin` (with real MinIO use its real secret). Tear down after.

### SPEC-0043/BT-001: the lifecycle command reaches past the handshake
- Level / Driver / Location: functional e2e / CLI `make test-e2e-full` / `full_stack_lifecycle`
- Red before the fix because: `MCP initialize: http 406 Not Acceptable` at `full_stack_e2e.rs:175` (record this output before editing the production lines).
- Given the infra above / When `make test-e2e-full` runs / Then output has neither `MCP initialize: http 406 Not Acceptable` nor `panicked`, exit 0.

### SPEC-0043/BT-010: the full lifecycle passes
- Level / Driver / Location: functional e2e / CLI / same test
- Given the same infra / When `make test-e2e-full` runs / Then output contains `test full_stack_lifecycle ... ok` and `test result: ok. 1 passed; 0 failed`.

### SPEC-0043/BT-020: the default suite is unchanged
- Level / Driver / Location: integration / CLI `make test` / whole workspace
- Given no live infra / When `make test` runs / Then 0 failed and `full_stack_lifecycle` is reported ignored.

## Constraints
- Files not to touch: anything under `crates/core/src/` (server), `Cargo.toml`, `Cargo.lock`.
- Dependencies not to add: none.
- Patterns to avoid: a client wide default header; removing `#[ignore]` (`full_stack_e2e.rs:316`).
- Scope boundary: no other e2e changes; MinIO specific behavior not covered by moto is out of scope.

## Definition of done
- [ ] RED evidence recorded for BT-001 before the production edit.
- [ ] BT-001, BT-010, BT-020 green; `make check` green; main stays green after the squash.
- [ ] Non regression: `full_stack_lifecycle` keeps its name, scenario and `#[ignore]`; server untouched.
- [ ] Docs updated: `.agent_docs/testing.md` dated gotcha line.
- [ ] Self review: only the two listed files changed; header value exact and in that order.
- [ ] Closes spec: BT-001, BT-010, BT-020 green; BR-001..BR-003 cited by this story.
- [ ] One commit `US-0051: send the Accept header on the full-stack e2e MCP calls`, body `Spec: SPEC-0043` and BR-001, BR-002, BR-003.
