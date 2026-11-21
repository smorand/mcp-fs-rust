# US-0273: fs.recalculate, docs and contract

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0268, US-0271
> min_tier: 1
> Files touched: 9
> Closes spec: yes

## Objective
Add the tool `fs.recalculate`: a member rebuilds the search entries of one file now, ignoring the delay; every refusal has its exact text and order; a call during a running rebuild supersedes it. Close the spec: regenerate the contract, update every doc, and run the full acceptance set.

## Requirements covered
- SPEC-0100/FR-NEW-008: WHEN a member of a project asks for the rebuild of a kind for a file of that project THE system SHALL cancel the waiting rebuild of that file, create a job at once, and answer with the job id, the path, the kind and the state.
- SPEC-0100/FR-NEW-009: IF the request is not allowed or not applicable THEN THE system SHALL refuse it, start nothing, and change no waiting rebuild.
- SPEC-0100/FR-NEW-013 (partial: a rebuild by hand asked during a running one): WHEN a running rebuild ends THE system SHALL discard its result and set its job to `superseded` if its file changed since it started or a rebuild by hand was asked for during it.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_` (all 71 spec tests) and `make check`. For example `fs_recalculate(proj, /notes.md, search_index)` while a rebuild waits answers a job in state `running` (or `done`), the waiting job is gone, and search finds the new content within 7 s without waiting the settle delay.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0100): At delay 0 a changed file gets a `hidden` waiting entry (due now, not listed, not counted as `waiting`); a manual rebuild queued behind a running one is a listed entry with `seconds_left` 0. **Obliged by:** SPEC-0100/FR-NEW-003, FR-NEW-008, FR-NEW-013. **Rationale:** the two cases differ by the spec. **Alternatives considered:** one flag for both (contradicts FR-NEW-008). **Source:** design. **Evidence:** n/a.
- DEC-011 (SPEC-0100): Kind is an enum with one value `search_index`; any other string is refused with the exact text of FR-NEW-009. **Obliged by:** SPEC-0100/FR-NEW-009. **Rationale:** later kinds (BL-0176, BL-0168) add a variant. **Alternatives considered:** free string. **Source:** design. **Evidence:** n/a.
- DEC-012 (SPEC-0100): Two tools in the `fs.*` family on the MCP surface only; no REST twin. **Obliged by:** SPEC-0100/FR-NEW-008, FR-NEW-010. **Rationale:** the spec names tools only; the metadata screen (BL-0175) will add REST. **Alternatives considered:** REST twins now (not asked). **Source:** spec. **Evidence:** `AGENTS.md` one implementation rule is respected: the engine is `tools::derived`.
- DEC-019 (SPEC-0100): Failure reason: an error whose text starts with `embedding request failed` or `embedding endpoint returned` becomes `embedding service unreachable`; any other error is `<ERR_CODE>: <message>`. **Obliged by:** SPEC-0100/FR-NEW-012. **Rationale:** matches the two failure texts of `embed`. **Alternatives considered:** a typed error (touches the whole `ToolError`). **Source:** existing code. **Evidence:** `search/embedding.rs:33-41`.
### Data model
No schema change.
### Contracts
Tool `fs.recalculate(mount_id: string, path: string, kind: string)` returns the job object of `fs.list_derived_jobs` plus nothing else; description for the golden: "Rebuild the search entries of one file now, without waiting for the settle delay." Refusal order and texts: design section 5.2. Each call creates a new job: no running job of that file: the job starts at once (`running`); a running job: it ends `superseded` and the answer is a `waiting` job with `seconds_left` 0 that starts when the running one ends; the waiting job of the file is removed without a record. `Scheduler::recalculate(project, path, kind, person)`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/derived.rs` | modify: `recalculate` engine with the refusal order |
| `crates/core/src/mcp/server.rs` | modify: `RecalculateArgs`, `#[tool(name = "fs.recalculate")]` |
| `crates/core/src/search/derived/scheduler.rs` | modify: `recalculate` |
| `crates/core/src/search/derived_e2e.rs` | modify: these tests |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | modify: regenerate, review; 117 to 118 tools, 47 `fs.*` |
| `README.md`, `AGENTS.md` | modify: counts, feature paragraph, key `search.settle_delay_seconds` |
| `.agent_docs/tools.md`, `.agent_docs/search.md`, `.agent_docs/architecture.md`, `.agent_docs/testing.md`, `.agent_docs/config.md` | modify: tool rows, scheduler, test command `cargo test -p fsa-core --lib -- e2e_0100_` |
| `sdlc/specs/SPEC-0008-*/` | do not touch (as built; the deviations are in SPEC-0100) |
### Existing patterns
Same shape as `fs.list_derived_jobs` from US-0266 (`authorize_only` is replaced by the explicit order in design 5.2: project, member, search enabled, kind, path, folder, mode).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-004: rebuild by hand, then list
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_004_*`)
- **Given** settle delay 30 s; Alice saved `/notes.md` with `beta` 5 s ago (a rebuild waits)
- **When** Alice asks `fs.recalculate` for kind `search_index` on `/notes.md`, then lists the jobs of `/notes.md`
- **Then** the answer names `/notes.md`, kind `search_index`, a job id and a state `running` (or `done` if it already ended); search finds `beta` within 7 s, without waiting the 30 s
- **And** the list shows one job of that kind, cause `manual`, and no `waiting` job
- **Priority:** Critical

### SPEC-0100/E2E-012: a non member cannot rebuild
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_012_*`)
- **Given** Dave (`dave@example.com`) is a member of project `other` only; a rebuild of `/notes.md` waits in `proj`
- **When** Dave asks `fs.recalculate` for `search_index` on `/notes.md` of `proj`
- **Then** the answer is `ERR_FORBIDDEN: 'dave@example.com' is not a member of 'proj'`
- **And** the waiting job is unchanged and no job started
- **Priority:** High

### SPEC-0100/E2E-013: unknown kind
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_013_*`)
- **Given** Alice, a member of `proj`
- **When** she asks for kind `chunks`
- **Then** the answer is `ERR_INVALID_ARGUMENT: kind must be 'search_index'`
- **And** no job started
- **Priority:** High

### SPEC-0100/E2E-014: unknown file
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_014_*`)
- **Given** Alice; `/nope.md` does not exist
- **When** she asks for `search_index` on `/nope.md`
- **Then** the answer is `ERR_NOT_FOUND: file not found: /nope.md`
- **And** no job started
- **Priority:** High

### SPEC-0100/E2E-015: a kind of a later increment is refused
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_015_*`)
- **Given** Alice, a member of `proj`
- **When** she asks for kind `conversion`
- **Then** the answer is `ERR_INVALID_ARGUMENT: kind must be 'search_index'`
- **And** no job started
- **Priority:** High

### SPEC-0100/E2E-017: two rebuilds by hand a second apart
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_017_*`)
- **Given** `/notes.md` indexed; the embedding service answers in 5 s; Carol (`carol@example.com`) is a member
- **When** Alice and Carol each ask for `search_index` on `/notes.md`, Carol 1 s after Alice
- **Then** the two answers name two different job ids
- **And** at no time are two jobs of that file and kind `running`; between 5 s and 8 s the list shows Alice's job `superseded` and Carol's `waiting` or `running`; after 15 s the list shows one finished job, Carol's, `done`, and Alice's job is no longer listed
- **Priority:** High

### SPEC-0100/E2E-026: a rebuild by hand replaces the waiting one
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_026_*`)
- **Given** `/notes.md` has a waiting job (cause `change`)
- **When** Alice rebuilds by hand
- **Then** the waiting job is gone; one job exists with cause `manual`
- **And** after the delay no extra job starts
- **Priority:** High

### SPEC-0100/E2E-028: search_index where indexing is not active
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_028_*`)
- **Given** project `plain` (index mode `none`); `/x.md` exists
- **When** Alice asks `search_index` on `/x.md`
- **Then** the answer is `ERR_NOT_SUPPORTED: search indexing is not active for project plain`
- **And** no job started
- **Priority:** High

### SPEC-0100/E2E-039: a rebuild by hand while one runs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_039_*`)
- **Given** a rebuild of `/notes.md` is running (the embedding service answers in 10 s)
- **When** Alice asks for a rebuild by hand of `search_index` on `/notes.md`
- **Then** the answer names a new job in state `waiting` with `seconds_left` 0; the running job ends `superseded`; the new job (cause `manual`) starts as soon as it has ended
- **And** at no time are two jobs of that file and kind `running`
- **Priority:** High

### SPEC-0100/E2E-056: a failed manual rebuild is reported in the answer
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_056_*`)
- **Given** the embedding service is down; `/notes.md` indexed
- **When** Alice asks for a rebuild by hand and waits for the job to end
- **Then** the first answer is not an error and its state is `running`, or `failed` with the reason `embedding service unreachable` if it already failed; within 5 s the job list shows `failed` with that reason
- **And** the previous entries are still searchable
- **Priority:** High

### SPEC-0100/E2E-057: a second rebuild by hand while the first runs
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_057_*`)
- **Given** `/notes.md` indexed; the embedding service answers in 10 s
- **When** Alice asks for `search_index` on `/notes.md`; Carol asks 2 s later
- **Then** the first job ends `superseded`; Carol's answer names a different job id with state `waiting` and `seconds_left` 0, which then runs
- **And** at no time are two jobs of that file and kind `running`
- **Priority:** High

### SPEC-0100/E2E-059: the project does not exist
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_059_*`)
- **Given** Alice; project `ghost` does not exist
- **When** she asks `fs.recalculate` for `search_index` on `/a.md` of `ghost`, then lists the jobs of `ghost`
- **Then** both answers are `ERR_PROJECT_NOT_FOUND`
- **And** no job started
- **Priority:** High

### SPEC-0100/E2E-069: a rebuild by hand when search is not enabled
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_069_*`)
- **Given** the server configuration has search disabled; Alice is a member of `proj`
- **When** she asks `fs.recalculate` for `search_index` on `/notes.md`
- **Then** the answer is `ERR_NOT_SUPPORTED: search is not enabled; set search.enabled: true in config`
- **And** no job started
- **Priority:** High

### SPEC-0100/E2E-076: a rebuild by hand on a folder
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_076_*`)
- **Given** `/docs` is a folder; Alice is a member
- **When** she asks `fs.recalculate` for `search_index` on `/docs`
- **Then** the answer is `ERR_INVALID_ARGUMENT: path must be a file`
- **And** no job started
- **Priority:** High

## Constraints
- Do not add a REST route.
- `search.index` is unchanged.
- Tool counts must match everywhere (rg for `116` and `45 ` in docs and tests).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: All of `README.md`, `AGENTS.md`, `.agent_docs/*` named in Paths; no dashes as punctuation.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.
- [ ] Closes spec: the spec full acceptance set green (every E2E-001 to E2E-077 and E2E-MOD-001, E2E-MOD-002) and every requirement of SPEC-0100 cited by at least one story with status done or landed; regenerate the tool contract and review the diff.
- [ ] One commit `US-0273: fs.recalculate, docs and contract` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
