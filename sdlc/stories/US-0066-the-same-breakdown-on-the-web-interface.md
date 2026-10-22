# US-0066: the same breakdown on the web interface

> Spec: SPEC-0047 (sdlc/specs/SPEC-0047-project-disk-usage-breakdown/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0064
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Programs calling the web interface get the breakdown with the same answer and the same refusals as the AI assistant tool, through `GET /api/admin/projects/{project_id}/disk-usage?limit=`. The route calls the tool's own function and is listed in the OpenAPI document.

## Requirements covered
- SPEC-0047/FR-NEW-011: The Administration context SHALL give AI assistants and web interface consumers the same result and the same refusals for the same request.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::disk_usage_e2e`: E2E-004 reads `Atlas` as Olivia once through the tool and once through `GET /api/admin/projects/Atlas/disk-usage` and the two JSON bodies are equal; E2E-034 with `?limit=2` returns two entries and `truncated` true.

## Design excerpt
### Decisions
- DEC-009 (SPEC-0047): REST route `GET /api/admin/projects/{project_id}/disk-usage?limit=`, bearer only, calling `tools::admin::get_project_disk_usage`, errors mapped with `error_response` (body `{"error": <code>, "detail": "<code>: <message>"}`), mirroring the quota route.
- DEC-006 (SPEC-0047): a missing `limit` is the default; an unparsable `limit` is passed on as `Some(f64::NAN)` so the function refuses it with `limit must be between 1 and 500`.
### Data model
None.
### Contracts
`GET /api/admin/projects/{project_id}/disk-usage?limit=<n>`: 200 with the tool's JSON body; refusals with the status of their code (`ERR_FORBIDDEN` 403, `ERR_INVALID_ARGUMENT` 400, `ERR_PROJECT_NOT_FOUND` 404) and the body `{"error": "ERR_FORBIDDEN", "detail": "ERR_FORBIDDEN: 'mia@example.com' is not the owner of 'Atlas'"}`. Register `("GET", "disk-usage")` in the sub route table at `api/dataplane.rs:98` (next to `("GET", "quota")`) and an `Op { method: "GET", sub: "disk-usage", path: "/api/admin/projects/{project_id}/disk-usage", tool: "admin.get_project_disk_usage", params: [limit, optional number], body: "" }` in `api/openapi.rs` next to line 1178.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (route table entry, handler, router registration beside the quota route) |
| `crates/core/src/api/openapi.rs` | modify (one `Op`) |
| `crates/core/src/tools/disk_usage_e2e.rs` | modify (router helper, tests) |
### Existing patterns
- Handler to mirror: `project_quota`, `crates/core/src/api/dataplane.rs:398-412`:
```rust
async fn project_quota(State(state): State<Arc<AppState>>, Path(project_id): Path<String>, headers: HeaderMap) -> Response {
    let person = match person_of(&state, &headers) { Ok(p) => p, Err(e) => return unauthorized(&e) };
    let ctx = crate::tools::registry_support::ToolCtx { person, state };
    match crate::tools::admin::get_project_quota(&ctx, &project_id).await { Ok(v) => Json(v).into_response(), Err(e) => error_response(&e) }
}
```
- Query parsing: `Query(q): Query<Vec<(String, String)>>` and the `Q` helper used by `list` at `dataplane.rs:421-430`.
- Router test helper: `rest(f, person)` and `rest_diff` at `crates/core/src/tools/diff_files_e2e.rs:709-750` (copy into `disk_usage_e2e.rs`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec): project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores five files, all `Active`: `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (975 500 in all). Olivia (`olivia@example.com`) owns Atlas, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Every session budget is 262 144 bytes. "The standard answer" means: project `Atlas`, limit 2, bytes used 975 500, 5 files, list complete, entries in the order of the table above, each `Active`. Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0047/E2E-004: Assistants and the web interface agree
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_disk_usage` and the data plane router `oneshot` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-011
- **Given** the shared setup
- **When** Olivia reads the breakdown of `Atlas` once through an AI assistant and once through the web
  interface
- **Then** both answers carry exactly the same values, entries and order

### SPEC-0047/E2E-015: The web interface refuses like the assistants
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_disk_usage` and the data plane router `oneshot` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-003. **Requirements:** FR-NEW-007, 011
- **Given** the shared setup
- **When** Mia reads the breakdown of `Atlas` through the web interface and, separately, through an AI
  assistant
- **Then** both refuse with `'mia@example.com' is not the owner of 'Atlas'`

### SPEC-0047/E2E-034: The cap through the web interface
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_disk_usage` and the data plane router `oneshot` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004, 011
- **Given** the shared setup
- **When** Olivia reads the breakdown through the web interface with a cap of 2
- **Then** two entries (`/video/demo.mp4`, `/data/export.csv`), 5 files counted, the list cut, and
  with a cap of 0 the refusal `limit must be between 1 and 500`

### SPEC-0047/E2E-035: The default cap through the web interface
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_disk_usage` and the data plane router `oneshot` / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005, 011
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads the breakdown through the web interface with no cap
- **Then** 100 entries shown, 120 files counted, the list cut, identical to the assistant answer

## Constraints
- Files not to touch: the tool function, the engine, the quota route.
- Dependencies not to add: none.
- Patterns to avoid: a second implementation of the read in the handler.
- Scope boundary: the lock and the docs (US-0067).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (the route is absent, so each test fails on a 404 status assertion).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the quota route and the other `/api/*` routes answer as before; the OpenAPI test that lists operations passes with the added `Op`.
- [ ] Docs updated: none in this story (US-0067 writes the docs)
- [ ] Self review: the answer shape matches DEC-004 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment, not a WHAT comment.
- [ ] One commit `US-0066: the same breakdown on the web interface`, body `Spec: SPEC-0047` and the requirement ids.
