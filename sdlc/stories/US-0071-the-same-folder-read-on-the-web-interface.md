# US-0071: the same folder read on the web interface

> Spec: SPEC-0049 (sdlc/specs/SPEC-0049-project-folder-usage-breakdown/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0070
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Programs calling the web interface get the same folder read, with the same answers and refusals as the AI assistant tool, through `GET /api/admin/projects/{project_id}/folder-usage?folder=&limit=`.

## Requirements covered
- SPEC-0049/FR-NEW-012: The Administration context SHALL give AI assistants and web interface consumers the same result and the same refusals for the same request.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::folder_usage_e2e`: E2E-006 reads the root and `/video/` as Olivia through the tool and through the route and the two JSON bodies are equal; E2E-020 returns the tool's `folder '/nothing' not found` with status 404.

## Design excerpt
### Decisions
- DEC-009 (SPEC-0049): REST route `GET /api/admin/projects/{project_id}/folder-usage?folder=&limit=`, bearer only, same function, errors through `error_response`; the query is read raw so `folder=` present and empty is refused, absent is the root; a non numeric `limit` goes on as NaN.
- DEC-012 (SPEC-0049): acceptance tests are functional e2e through the real tool method or the data plane router.
### Data model
None.
### Contracts
`GET /api/admin/projects/{project_id}/folder-usage?folder=<path>&limit=<n>`: 200 with the tool's JSON; refusals with the status of their code (`ERR_FORBIDDEN` 403, `ERR_INVALID_ARGUMENT` 400, `ERR_NOT_FOUND` and `ERR_PROJECT_NOT_FOUND` 404) and the body `{"error": "ERR_NOT_FOUND", "detail": "ERR_NOT_FOUND: folder '/nothing' not found"}`. Register `("GET", "folder-usage")` in `REST_ROUTES` (dataplane.rs:98 area, next to `disk-usage`), bump the inventory test (48 to 49), and an `Op` in `api/openapi.rs` after the disk usage one with params `folder` (string, optional) and `limit` (number, optional, `Def::Int(100)`).
Handler (copy `project_disk_usage`, `dataplane.rs:424`), reading the query raw:
```rust
let folder = q.iter().find(|(k, _)| k == "folder").map(|(_, v)| v.as_str());
let limit = Q(q.clone()).opt("limit").map(|v| v.parse::<f64>().unwrap_or(f64::NAN));
```
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify (route table entry, handler, registration, inventory test) |
| `crates/core/src/api/openapi.rs` | modify (one `Op`) |
| `crates/core/src/tools/folder_usage_e2e.rs` | modify (router helper, tests) |
### Existing patterns
- Tool shape, gate and cap: `get_project_disk_usage`, `reader_project`, `parse_disk_usage_cap`, `crates/core/src/tools/admin.rs:672-760`; its `#[tool]` method `admin_get_project_disk_usage` in `crates/core/src/mcp/server.rs` (SPEC-0047, US-0064).
- Engine and lock: `RelationalMetaStore::largest_files` (`crates/core/src/storage/meta.rs:596-690`) and `quota::disk_usage` (`crates/core/src/quota.rs:179-195`).
- Test harness: `World`, `seed`, `read`, `ok_json`, `err_text`, `rest_usage` in `crates/core/src/tools/disk_usage_e2e.rs` (copy what you need into `folder_usage_e2e.rs`: modules are private).
- Router helper: `rest` and `rest_usage` at the end of `disk_usage_e2e.rs` (copy; add a `folder` query part).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec):project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores seven files, all active: `/video/demo.mp4` 600 000, `/video/clips/a.mp4` 200 000, `/data/export.csv` 250 000, `/data/old/x.csv` 50 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (1 225 500 in all). Olivia (`olivia@example.com`) owns `Atlas`, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Each member's session budget is 262 144 bytes. A test that starts with "`Atlas` stores ..." instead of "the shared setup" starts from a project holding only the files and folders it names (the seven shared files are absent), with the same limit of 2 MB unless it says otherwise. The answer fields are named in the design: `project_id`, `max_mb`, `used_bytes`, `folder` (`path`, `size`, `file_count`), `item_count`, `truncated`, `items` (`path`, `kind`, `size`, `file_count` for a folder only). Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0049/E2E-006: Assistants and the web interface agree
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_folder_usage` and the data plane router `oneshot` / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-005. **Requirements:** FR-NEW-012. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder breakdown of `Atlas` for no folder, then for `/video/`, each once through an AI assistant and once through the web interface
- **Then** for each of the two folders both interfaces carry exactly the same values, items and order

### SPEC-0049/E2E-019: The web interface refuses like the assistants
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_folder_usage` and the data plane router `oneshot` / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-005. **Requirements:** FR-NEW-008, FR-NEW-012. **Priority:** High
- **Given** the shared setup
- **When** Mia reads the folder breakdown of `Atlas` through the web interface and, separately, through an AI assistant
- **Then** both refuse with `'mia@example.com' is not the owner of 'Atlas'`

### SPEC-0049/E2E-020: The web interface reports a missing folder like the assistants
- Level / Driver / Location: functional e2e / tool `McpServer::admin_get_project_folder_usage` and the data plane router `oneshot` / `crates/core/src/tools/folder_usage_e2e.rs`
- **Scenario:** SC-005. **Requirements:** FR-NEW-009, FR-NEW-012. **Priority:** High
- **Given** the shared setup
- **When** Olivia reads the folder `/nothing` through the web interface and, separately, through an AI assistant
- **Then** both refuse with `folder '/nothing' not found`

## Constraints
- Files not to touch: the tool function, the engine, the other routes.
- Dependencies not to add: none.
- Patterns to avoid: a second implementation of the read in the handler; `Q::opt` for `folder` (it drops an empty value).
- Scope boundary: lock and state tests (US-0072), docs (US-0073).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (the route is absent, so each test fails on a 404 status assertion).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the other `/api/*` routes answer as before; the OpenAPI tests that list operations pass with the added `Op`.
- [ ] Docs updated: none in this story
- [ ] Self review: the answer shape matches DEC-003 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment.
- [ ] One commit `US-0071: the same folder read on the web interface`, body `Spec: SPEC-0049` and the requirement ids.
