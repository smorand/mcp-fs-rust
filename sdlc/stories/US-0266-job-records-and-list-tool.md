# US-0266: job records and the fs.list_derived_jobs tool

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 8
> Closes spec: no

## Objective
Create the persistent job record (table `derived_job`, its store, the job model) and expose it through the read only tool `fs.list_derived_jobs`, with the paging, ordering and refusals of the spec. Nothing writes jobs yet in production; the tests seed rows. A member can already ask "what is waiting or failed for this file" and get an honest empty answer.

## Requirements covered
- SPEC-0100/FR-NEW-010: WHEN a member of a project asks for its jobs THE system SHALL list, for one file or for the whole project, each job with its path, kind, state, cause (`change` or `manual`), creation time, the seconds left when `waiting`, its end time when finished, and its failure reason when `failed`.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_ derived::`. The tests seed rows through `JobStore` (10 000 finished jobs, then states waiting, running, failed), call `McpServer::fs_list_derived_jobs` as a member and as a non member and assert the answers listed under Acceptance tests.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0100): Jobs live in the admin store table `derived_job`, global to the deployment, foreign key to `project` with cascade. **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-017, FR-NEW-018. **Rationale:** one table scan recovers every project; hard delete cascades for free. **Alternatives considered:** per volume meta store (one SQLite file per volume), memory only. **Source:** existing code. **Evidence:** `storage/admin.rs:44-55` (cascade precedent), `storage/mod.rs:255-282`.
- DEC-005 (SPEC-0100): Only the job row is persisted, never `due`; after a restart every waiting row restarts with the full delay. Ids are uuid v4, times RFC 3339 UTC seconds (`chrono`). **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-010. **Rationale:** the spec says the full delay again; no clock arithmetic across restarts. **Alternatives considered:** persist due (needs wall clock math, not asked). **Source:** design. **Evidence:** n/a.
- DEC-006 (SPEC-0100): At the end of a job the earlier finished row of the same `(project, path, kind)` is deleted, so one finished job per key is kept. **Obliged by:** SPEC-0100/FR-NEW-010. **Rationale:** the spec keeps the last finished job per file and kind; bounded table. **Alternatives considered:** keep history and filter on read (unbounded growth). **Source:** design. **Evidence:** n/a.
- DEC-011 (SPEC-0100): Kind is an enum with one value `search_index`; any other string is refused with the exact text of FR-NEW-009. **Obliged by:** SPEC-0100/FR-NEW-009. **Rationale:** later kinds (BL-0176, BL-0168) add a variant. **Alternatives considered:** free string. **Source:** design. **Evidence:** n/a.
- DEC-012 (SPEC-0100): Two tools in the `fs.*` family on the MCP surface only; no REST twin. **Obliged by:** SPEC-0100/FR-NEW-008, FR-NEW-010. **Rationale:** the spec names tools only; the metadata screen (BL-0175) will add REST. **Alternatives considered:** REST twins now (not asked). **Source:** spec. **Evidence:** `AGENTS.md` one implementation rule is respected: the engine is `tools::derived`.
- DEC-022 (SPEC-0100): `list` reads waiting and running from memory and finished jobs from the table with a keyset query ordered `ended_at DESC, path ASC, kind ASC`; index `idx_derived_job_page`. **Obliged by:** SPEC-0100/FR-NEW-010 (10 000 jobs under 2 s). **Rationale:** no full scan per page. **Alternatives considered:** offset paging (marker drift). **Source:** design. **Evidence:** n/a.
### Data model
Table `derived_job` in `admin::schema()`: job_id Text pk; project_id Text not null fk project(id) cascade; path Text; kind Text; state Text; cause Text; hidden BigInt default 0; created_at Text; ended_at Text null; reason Text null; indexes `idx_derived_job_key (project_id, path, kind)` and `idx_derived_job_page (project_id, state, ended_at)`.
### Contracts
Tool `fs.list_derived_jobs(mount_id: string, path: string = "", marker: string = "")` returns `{"jobs":[{job_id,path,kind,state,cause,created_at,seconds_left?,ended_at?,reason?}], "marker"?: string}`; read only; description for the golden: "List the rebuild jobs of derived data (search entries) of a project or of one file: waiting, running and the last finished one per file." Refusals: project absent `ERR_PROJECT_NOT_FOUND`, non member `ERR_FORBIDDEN: '<person>' is not a member of '<mount_id>'`, search not enabled for the server answers `{"jobs":[]}`, bad marker `ERR_INVALID_ARGUMENT: marker is not valid`. Order: waiting (fewest `seconds_left` first), running, finished (newest `ended_at` first), ties by path then kind; 100 per answer; a `path` that names a folder lists the files under it. Marker: base64 url safe of JSON `[group, a, b, c]` (design section 5.3). Rust: `pub enum JobState`, `pub struct Job`, `pub struct JobStore` (`new(Arc<dyn RelationalDb>)`, `insert`, `set_state`, `finish`, `delete`, `list_finished_page`, `load_unfinished`, `delete_project`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/admin.rs` | modify: add the table and its indexes to `schema()` |
| `crates/core/src/storage/mod.rs` | modify: `build_derived_job_store(config, registry)` reusing the admin database target |
| `crates/core/src/search/derived/mod.rs` | create: `JobState`, `Job`, kind enum, paging marker |
| `crates/core/src/search/derived/store.rs` | create: `JobStore` |
| `crates/core/src/tools/derived.rs` | create: `list_derived_jobs(state, mount_id, path, marker)` engine |
| `crates/core/src/mcp/server.rs` | modify: `ListDerivedJobsArgs` and `#[tool(name = "fs.list_derived_jobs")]` |
| `crates/core/src/search/derived_e2e.rs` | create: fixture (admin store in memory, local volume) and these tests |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | modify: regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1`, review the diff; counts 116 to 117 in the tests that assert them |
### Existing patterns
Tool pattern: `mcp/server.rs:2318-2342` (`fs.trash_list`: `authorize_only`, engine in `tools/trash.rs`, `to_call_result`). Table declaration and cascade: `storage/admin.rs:23-95`. A keyset page and SQL through `Query::new(...).bind(...)`: `storage/admin.rs:244-262`. Store opened from the registry: `storage/mod.rs:255-282`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-016: no job yet
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_016_*`)
- **Given** project `proj` with no write since the server started
- **When** Alice lists the jobs of `proj`, then of `/notes.md`
- **Then** both lists are empty
- **And** no marker is returned
- **Priority:** Medium

### SPEC-0100/E2E-020: a long job history is paged
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_020_*`)
- **Given** 10 000 files each with a finished job
- **When** Alice lists the jobs of the project, then asks for the next page with the returned marker
- **Then** the first answer holds 100 jobs, newest end first, and a marker; the second holds the next 100
- **And** the first answer arrives in under 2 s
- **Priority:** Medium

### SPEC-0100/E2E-027: a non member cannot list jobs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_027_*`)
- **Given** Dave is a member of `other` only; `proj` has jobs
- **When** Dave lists the jobs of `proj`
- **Then** the answer is `ERR_FORBIDDEN: 'dave@example.com' is not a member of 'proj'`
- **And** listing the jobs of `other` still works and shows only `other` jobs
- **Priority:** High

### SPEC-0100/E2E-070: listing jobs when search is not enabled
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_070_*`)
- **Given** the server configuration has search disabled
- **When** Alice lists the jobs of `proj`
- **Then** the list is empty
- **And** no error is returned
- **Priority:** Medium

## Constraints
- Do not add REST routes.
- Do not write jobs from any hook yet (US-0268).
- Reads of waiting and running jobs come from memory in later stories; here the list reads only the table for every state.
- No new dependency.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: `.agent_docs/tools.md` gets the row for the tool at US-0273 together with its sibling; here only the contract files.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0266: job records and the fs.list_derived_jobs tool` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
