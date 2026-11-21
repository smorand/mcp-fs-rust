# SPEC-0100: search entries are rebuilt after the file stops changing, design

> Spec: sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/spec.md
> Nature: FEAT
> Depth: L
> Security: n/a
> Designed on: 2026-11-20
> Target tier: 2 (default, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024 | toolchain 1.99.0 (rustup stable; repo pin 1.94.1 is stale, see AGENTS.md quirks) | existing |
| Runtime and web | tokio, axum, rmcp | `Cargo.toml` workspace (tokio 1, rmcp 3.5.0) | existing |
| Relational layer | `storage::rel` (SQLite default, PostgreSQL optional) | in tree | DEC-004 |
| Search engines | tantivy (BM25), sqlx + pgvector, sqlite-vec | in tree | DEC-008, DEC-009 |
| Ids and times | `uuid` v4, `chrono` | workspace manifest (`uuid` 1, `chrono` 0.4) | DEC-005 |
| Config | serde_norway YAML with `${VAR}` expansion | in tree | DEC-013 |

No dependency is added, so no context7 lookup was needed (versions are the manifest's). Skill loaded: `rust` (already in context). Effect: `make check` is the gate (format, clippy `-D warnings` with `--all-features`, tests), tests under `crates/core/src/**` modules, comments explain WHY, no dashes as punctuation, `tracing` for logs, `ToolError` for errors.

## 2. Approaches
### Chosen: a persisted job table plus one in memory scheduler, cut in at the indexer hooks
`search::indexer` already is the single place every write, edit, patch, upload, copy, move, delete and restore passes through (`crates/core/src/search/indexer.rs:199-376`). Its `after_*` functions stop spawning an immediate task and call a `Scheduler` (new module `search/derived/`). The scheduler keeps the waiting and running jobs in memory behind one async mutex, drives them with one dispatcher task, and mirrors every job in a table of the admin store so a restart loses nothing. Pros: callers (about 40 call sites in `mcp/server.rs`, `api/dataplane.rs`, `tools/*`) do not change; one scheduler, no timer per write. Cons: a hook gets one extra DB write on the first change of a quiet period. Scope: `search/`, `storage/admin.rs`, `app.rs`, two tools.
### Rejected: a timer task per write
Simple, but a restart forgets it, a burst of saves leaks timers and there is nothing to list. Rejected (DEC-003).
### Rejected: jobs only in memory
Fails FR-NEW-007 (restart keeps waiting rebuilds) and FR-NEW-017. Rejected (DEC-004).
### Rejected: jobs in the per volume meta store
SQLite meta is one file per volume (`storage/mod.rs:255-264`), so recovery at boot would open every volume and a project delete would need extra cleanup. The admin store is global and already cascades on project delete (`storage/admin.rs:44-55`). Rejected (DEC-004).

## 3. Architecture
```
 write/edit/patch/upload/copy/restore/move/delete tools and REST routes (unchanged)
                     |
          search::indexer::after_*   (hooks, now thin)
                     |
        ScheduledBackend (decorator of the SearchBackend, DEC-014)
                     |  .scheduler()
               derived::Scheduler  --- derived::JobStore ---> admin DB table derived_job
         (Mutex<Inner>, dispatcher)
                     |  index_path_checked(..., guard)
               SearchBackend (Tantivy | PG | sqlite-vec | Combined)

 fs.recalculate, fs.list_derived_jobs  --->  tools::derived  --->  Scheduler
```
Bounded contexts: Derived data (scheduler, job store, tools), Search (backends, index mode). Existing structure cited: hooks `search/indexer.rs`, backend trait `search/mod.rs:52-100`, factory `search/mod.rs:120-215`, admin schema `storage/admin.rs:23-95`, composition root `app.rs:60-160`, tool pattern `mcp/server.rs:2318-2342`.

### 3.1 Scheduler behavior (the heart)
State, per key `(project_id, path, kind)` (kind is always `search_index` now):
- `waiting: Option<Waiting { job_id, created_at, due: tokio::time::Instant, hidden: bool, cause }>`
- `running: Option<Running { job_id, started_at, stale: Arc<AtomicBool>, removed: Arc<AtomicBool> }>`
Rules (each is exercised by the stories):
1. `schedule(project, path, cause=change)`: when the project's mode is active (checked by the hook, `active_backend`). If `waiting` exists: `due = now + delay`, no DB write. Else create `Waiting` (insert row, state `waiting`), `hidden = (delay == 0)`. If `running` exists: set its `stale`. Wake the dispatcher.
2. Dispatcher: sleeps until the earliest `due` among waiting entries whose key has no `running`; on due it moves the entry to `running` (row state `running`), spawns the rebuild task. When a running task ends it wakes the dispatcher (a waiting entry blocked by it is then eligible).
3. Rebuild task: read the file as UTF 8 (skip silently when not text, job still `done`), call `backend.index_path_checked(..., &|| !stale.load())`. The guard is evaluated after embedding and inside each engine's commit step, so a stale result is never stored. Outcome: guard refused or `stale` set at end: job `superseded`; `Err`: job `failed` with the reason of DEC-019; else `done`. At the end of any outcome the earlier finished row of the same key is deleted (DEC-006) and the key's blocked waiting entry is woken. If `removed` is set (delete or project removal during the run) the row is deleted instead of finished and `backend.delete_path` is called once more, because a commit may have slipped between the guard and the delete hook (DEC-010).
4. Delay 0 (FR-NEW-003): `hidden` entries are not listed and are due at once; with no running job the dispatcher starts them in the same wake.
5. Persistence: only the row (state, cause, times, reason). `due` is never stored: after a restart every `waiting` row gets `due = now + delay` (DEC-005).
6. All state changes and row writes happen under one `tokio::sync::Mutex<Inner>`; the lock is never held while a rebuild runs.

## 4. Data model
New table `derived_job` in the admin schema (`storage/admin.rs::schema()`, rendered per dialect by `storage/rel/schema.rs`, created by `admin.connect()`):

| Column | Type | Constraint |
|---|---|---|
| job_id | Text | primary key (uuid v4) |
| project_id | Text | not null, foreign key to `project(id)` on delete cascade |
| path | Text | not null |
| kind | Text | not null (`search_index`) |
| state | Text | not null (`waiting`, `running`, `done`, `failed`, `superseded`) |
| cause | Text | not null (`change`, `manual`) |
| hidden | BigInt | not null default 0 (1 only for delay 0 waiting entries) |
| created_at | Text | not null, RFC 3339 UTC seconds |
| ended_at | Text | null until finished |
| reason | Text | null unless `failed` |

Indexes: `idx_derived_job_key (project_id, path, kind)`, `idx_derived_job_page (project_id, state, ended_at)`. Migration: new table, `CREATE TABLE IF NOT EXISTS`, additive; rollback is dropping the table (no other table references it). No column change on existing tables. Project removal: the cascade covers a hard delete; a soft delete (`soft_delete_project`) and an undelete call `Scheduler::drop_project` (DEC-016).

## 5. Contracts and interfaces
### 5.1 Tools (both in the `fs.*` family, so 45 becomes 47 `fs.*` and 116 becomes 118 tools; `TOOL_CONTRACT.txt` and `tool-contract-golden.json` regenerate)
`fs.recalculate(mount_id: string, path: string, kind: string)` returns `{job_id, path, kind, state, cause, created_at, seconds_left?, ended_at?, reason?}`. Annotations: not read only, not destructive, idempotent false.
`fs.list_derived_jobs(mount_id: string, path: string = "", marker: string = "")` returns `{jobs: [Job], marker?: string}`. Read only.
`Job` = `{job_id, path, kind, state, cause, created_at, seconds_left?, ended_at?, reason?}` with the field rules of FR-NEW-010 (`seconds_left` only when `waiting`, whole seconds rounded up; `ended_at` only when finished; `reason` only when `failed`; times RFC 3339 UTC with seconds, `2026-11-20T10:01:10Z`).
### 5.2 Exact refusals (asserted by tests)
Order for `fs.recalculate`: project absent or deleted `ERR_PROJECT_NOT_FOUND` (existing text, `storage/admin.rs:380-398`), non member `ERR_FORBIDDEN: '<person>' is not a member of '<mount_id>'` (existing), search off `ERR_NOT_SUPPORTED: search is not enabled; set search.enabled: true in config` (existing text, `tools/search_semantic.rs:48`), kind `ERR_INVALID_ARGUMENT: kind must be 'search_index'`, path absent `ERR_NOT_FOUND: file not found: <path>`, path is a folder `ERR_INVALID_ARGUMENT: path must be a file`, indexing off `ERR_NOT_SUPPORTED: search indexing is not active for project <mount_id>`.
`fs.list_derived_jobs`: the first two as above; search off answers `{"jobs": []}`; an unusable marker `ERR_INVALID_ARGUMENT: marker is not valid`.
### 5.3 Marker
Opaque base64 (URL safe, no padding) of a JSON array `[group, a, b, c]`: group 0 waiting `[0, seconds_left, path, kind]`, group 1 running `[1, created_at, path, kind]`, group 2 finished `[2, ended_at, path, kind]`. A page continues strictly after that tuple in the list order of FR-NEW-010. Any decoding or shape failure is `marker is not valid`.
### 5.4 Configuration
`search.settle_delay_seconds` (default 30, whole seconds, 0 or more). Precedence as every key: defaults < file < env (`${VAR}` expansion in the YAML), no CLI flag. Boot failure text: `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`.
### 5.5 Internal signatures (Rust)
```rust
// search/mod.rs, trait SearchBackend (provided method, default ignores the guard)
async fn index_path_checked(&self, volume_id: &str, path: &str, text: &str,
    chunk_size: usize, chunk_overlap: usize,
    still_current: &(dyn Fn() -> bool + Send + Sync)) -> Result<Option<usize>>;   // None = guard refused, nothing stored
fn scheduler(&self) -> Option<Arc<derived::Scheduler>> { None }
// search/derived/mod.rs
pub enum JobState { Waiting, Running, Done, Failed, Superseded }
pub struct Scheduler;  // new(backend, store, stores, admin, delay_secs) / schedule / reschedule_now / move_path / forget_path / forget_project / list / recalculate / recover
// search/indexer.rs (hooks keep their names; after_move gains `src`, after_restore is new)
pub async fn after_move(state, volume_id, src: &str, sources: &[String], destination: &str, client)
pub async fn after_restore(state, volume_id, trash_path: &str, restored_path: &str, client)
```

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/search/mod.rs` | modify: trait gains `index_path_checked` and `scheduler()`; Combined indexes vector first then BM25; `ScheduledBackend` wiring | FR-NEW-011, FR-NEW-012, FR-NEW-020 | US-0264, US-0268 |
| `crates/core/src/search/bm25_sqlite.rs` | modify: guard under the write lock | FR-NEW-011, FR-NEW-013 | US-0264 |
| `crates/core/src/search/bm25_pg.rs`, `vector_pg.rs`, `vector_sqlite.rs` | modify: guard, then one transaction for delete plus inserts | FR-NEW-011, FR-NEW-012, FR-NEW-020 | US-0264, US-0265 |
| `crates/core/src/config.rs` | modify: `SearchConfig.settle_delay_seconds`, scalar deserializer, boot check | FR-NEW-014, FR-NEW-016 | US-0267 |
| `config/local.yaml.template`, `.agent_docs/config.md` | modify: document the key | FR-NEW-014 | US-0267 |
| `crates/core/src/storage/admin.rs` | modify: `derived_job` table in `schema()` | FR-NEW-007, FR-NEW-010 | US-0266 |
| `crates/core/src/storage/mod.rs` | modify: `build_derived_job_store` | FR-NEW-007 | US-0266 |
| `crates/core/src/search/derived/{mod,store,scheduler}.rs` | create | FR-NEW-001..FR-NEW-020 | US-0266, US-0268 |
| `crates/core/src/tools/derived.rs` | create: list and recalculate engines | FR-NEW-008, FR-NEW-009, FR-NEW-010 | US-0266, US-0273 |
| `crates/core/src/mcp/server.rs` | modify: two `#[tool]` methods and their args | FR-NEW-008, FR-NEW-010 | US-0266, US-0273 |
| `crates/core/src/search/indexer.rs` | modify: hooks call the scheduler; `after_move`, `after_restore`, `after_copy`, `after_delete*` | FR-NEW-001, FR-NEW-005, FR-NEW-006 | US-0268..US-0272 |
| `crates/core/src/tools/trash.rs`, `api/dataplane.rs` | modify: pass `src` to `after_move`, call `after_restore` | FR-NEW-001, FR-NEW-006 | US-0269, US-0271 |
| `crates/core/src/app.rs` | modify: build the scheduler, recover at boot, drain at shutdown | FR-NEW-007, FR-NEW-017 | US-0268, US-0270 |
| `crates/core/src/search/derived_e2e.rs` | create (test module, all spec E2E) | all | US-0266..US-0273 |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | modify (regenerate) | FR-NEW-008, FR-NEW-010 | US-0266, US-0273 |
| `README.md`, `AGENTS.md`, `.agent_docs/{tools,search,architecture,testing}.md` | modify | all | US-0273 |
Affected existing tests: `search/indexer.rs` hook tests (kept, run with delay 0 and a scheduler, or keep a backend with no scheduler: legacy immediate path, DEC-014), `search/e2e.rs` auto index tests (keep, same rule), `tools/trash_search_e2e.rs` and `tools/project_removal_e2e.rs` (keep, backend without scheduler), `tools/tool_surface_e2e.rs` and the tool contract golden tests (modify: counts 116 to 118), SPEC-0008 E2E-023 equivalents (modified by E2E-MOD-001/002).

## 7. Compatibility and risks
- Observable change: a write no longer indexes at once; with the default 30 s the content becomes searchable about 30 s later. Operators wanting the old behavior set `search.settle_delay_seconds: 0` (E2E-MOD-002).
- `search.index` and `search.status` are unchanged. Companion indexing (`after_companion`) keeps its immediate path (spec 14.3, BL-0176).
- Risk 1, entries across engines: in `both` mode BM25 and vector commit one after the other, so a single search can see one engine updated first for a few milliseconds. Accepted; FR-NEW-020 is met per engine. Mitigation: the vector commit (the one that can fail) is first.
- Risk 2, a commit slipping between the guard and a delete: handled by DEC-010.
- Risk 3, virtual time in tests: the scheduler uses only `tokio::time`; tests run with `tokio::time::pause()`. SQLite offload through `spawn_blocking` can make auto advance fire early; tests advance by explicit steps and `yield_now`.
- Crash (not a clean stop) can lose a job whose insert did not commit; the insert is awaited inside the hook, so the window is the hook itself.
- Rollback: revert the branch; the table is additive and ignored by older code.

## 9. Test strategy
Every spec test is a functional e2e: it calls the real `McpServer` handler (`fs_write`, `fs_edit`, `fs_move`, `fs_recalculate`, `fs_list_derived_jobs`...) on a fixture with an in memory admin store, a local volume and a stub embedding endpoint (a tiny axum server with a settable delay and a down switch), clock paused. Pattern: `tools/trash_search_e2e.rs` and `mcp/server.rs` tests (`Fixture::new`). All in `crates/core/src/search/derived_e2e.rs`, named `e2e_0100_<id>_<slug>` so `cargo test -p fsa-core --lib -- e2e_0100_` runs them. Story local tests (backend atomicity, config) live in the touched modules. Exploit level: none (Security n/a).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-016 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0266 |
| E2E-020 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0266 |
| E2E-027 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0266 |
| E2E-070 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0266 |
| E2E-025 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0267 |
| E2E-071 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0267 |
| E2E-074 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0267 |
| E2E-001 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-002 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-005 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-007 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-008 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-009 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-018 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-019 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-021 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-023 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-029 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-036 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-040 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-041 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-043 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-044 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-045 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-051 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-052 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-053 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-073 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-077 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-MOD-001 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-MOD-002 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0268 |
| E2E-049 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0269 |
| E2E-063 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0269 |
| E2E-064 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0269 |
| E2E-068 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0269 |
| E2E-011 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0270 |
| E2E-034 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0270 |
| E2E-035 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0270 |
| E2E-054 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0270 |
| E2E-067 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0270 |
| E2E-072 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0270 |
| E2E-010 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-024 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-030 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-031 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-032 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-033 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-042 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-075 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0271 |
| E2E-046 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-047 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-048 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-060 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-061 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-062 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-065 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-066 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0272 |
| E2E-004 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-012 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-013 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-014 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-015 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-017 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-026 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-028 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-039 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-056 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-057 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-059 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-069 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |
| E2E-076 | functional e2e | `McpServer` handler, clock paused | `search/derived_e2e.rs` | US-0273 |

Story local tests (US-0264, US-0265) are unit and integration tests in the touched backend modules.
Test data: generated (stub endpoint, in memory stores); nothing user provided.

## 10. Decisions log
- **DEC-001:** No new dependency; stack is the repository's. **Obliged by:** n/a (no code impact): the spec names no technology. **Rationale:** every needed capability (uuid, chrono, tokio time, sqlx) is in the manifest. **Alternatives considered:** a job queue crate (rejected: a table plus one task is enough). **Source:** existing code. **Evidence:** `Cargo.toml:126-127`.
- **DEC-002:** The cut in point is `search::indexer::after_*`; no caller of a hook changes. **Obliged by:** SPEC-0100/FR-NEW-001, FR-NEW-004. **Rationale:** about 40 call sites reach the index through these hooks. **Alternatives considered:** wrapping each tool (rejected, drift). **Source:** existing code. **Evidence:** `search/indexer.rs:199-376`, `mcp/server.rs:1757-2261`.
- **DEC-003:** One scheduler with one dispatcher task and an async mutex over its state, not one timer per write. **Obliged by:** SPEC-0100/FR-NEW-002, FR-NEW-003, FR-NEW-013. **Rationale:** restart of a wait is a field update; one rebuild per key at a time is a map check. **Alternatives considered:** timer per write, `tokio_util::DelayQueue` (new dependency). **Source:** design. **Evidence:** n/a.
- **DEC-004:** Jobs live in the admin store table `derived_job`, global to the deployment, foreign key to `project` with cascade. **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-017, FR-NEW-018. **Rationale:** one table scan recovers every project; hard delete cascades for free. **Alternatives considered:** per volume meta store (one SQLite file per volume), memory only. **Source:** existing code. **Evidence:** `storage/admin.rs:44-55` (cascade precedent), `storage/mod.rs:255-282`.
- **DEC-005:** Only the job row is persisted, never `due`; after a restart every waiting row restarts with the full delay. Ids are uuid v4, times RFC 3339 UTC seconds (`chrono`). **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-010. **Rationale:** the spec says the full delay again; no clock arithmetic across restarts. **Alternatives considered:** persist due (needs wall clock math, not asked). **Source:** design. **Evidence:** n/a.
- **DEC-006:** At the end of a job the earlier finished row of the same `(project, path, kind)` is deleted, so one finished job per key is kept. **Obliged by:** SPEC-0100/FR-NEW-010. **Rationale:** the spec keeps the last finished job per file and kind; bounded table. **Alternatives considered:** keep history and filter on read (unbounded growth). **Source:** design. **Evidence:** n/a.
- **DEC-007:** At delay 0 a changed file gets a `hidden` waiting entry (due now, not listed, not counted as `waiting`); a manual rebuild queued behind a running one is a listed entry with `seconds_left` 0. **Obliged by:** SPEC-0100/FR-NEW-003, FR-NEW-008, FR-NEW-013. **Rationale:** the two cases differ by the spec. **Alternatives considered:** one flag for both (contradicts FR-NEW-008). **Source:** design. **Evidence:** n/a.
- **DEC-008:** Add `SearchBackend::index_path_checked(..., still_current)` as a provided method (default ignores the guard and calls `index_path`); the four engines and Combined override it. `index_path` keeps its signature. **Obliged by:** SPEC-0100/FR-NEW-011, FR-NEW-013, FR-NEW-020. **Rationale:** the guard must run after embedding and before the commit; callers and the test `RecordingBackend` stay untouched. **Alternatives considered:** change `index_path` (touches `search.index`, `full_index`, tests), a prepare/commit split (more surface). **Source:** existing code. **Evidence:** `search/mod.rs:52-62`, `search/indexer.rs:455`.
- **DEC-009:** Atomic replacement per engine: Tantivy already deletes and adds in one commit (`bm25_sqlite.rs:109-140`); PostgreSQL BM25, pgvector and sqlite-vec wrap delete plus inserts in one transaction; Combined runs the vector engine (embedding, can fail) first and BM25 second. **Obliged by:** SPEC-0100/FR-NEW-011, FR-NEW-012, FR-NEW-020. **Rationale:** a failure before any commit leaves every engine on the old entries. **Alternatives considered:** two phase across engines (needs staging tables). **Source:** existing code. **Evidence:** `search/bm25_pg.rs:56-90`, `search/vector_pg.rs:80-125`, `search/vector_sqlite.rs:93-170`, `search/mod.rs:252-262`.
- **DEC-010:** Delete or project removal during a run sets the key's `removed` flag before the hook deletes the entries, and the rebuild epilogue calls `delete_path` again when it finds the flag. **Obliged by:** SPEC-0100/FR-NEW-005, FR-NEW-018. **Rationale:** closes the window between the guard and the delete. **Alternatives considered:** a per path lock in every engine. **Source:** design. **Evidence:** n/a.
- **DEC-011:** Kind is an enum with one value `search_index`; any other string is refused with the exact text of FR-NEW-009. **Obliged by:** SPEC-0100/FR-NEW-009. **Rationale:** later kinds (BL-0176, BL-0168) add a variant. **Alternatives considered:** free string. **Source:** design. **Evidence:** n/a.
- **DEC-012:** Two tools in the `fs.*` family on the MCP surface only; no REST twin. **Obliged by:** SPEC-0100/FR-NEW-008, FR-NEW-010. **Rationale:** the spec names tools only; the metadata screen (BL-0175) will add REST. **Alternatives considered:** REST twins now (not asked). **Source:** spec. **Evidence:** `AGENTS.md` one implementation rule is respected: the engine is `tools::derived`.
- **DEC-013:** `SearchConfig.settle_delay_seconds` is a `String` filled by a deserializer that accepts a YAML integer, float or string, and is parsed in `validate()` before the `enabled` early return. **Obliged by:** SPEC-0100/FR-NEW-014, FR-NEW-016. **Rationale:** `2.5` and `abc` must reach the spec's message, not a serde error; the check applies when search is disabled. **Alternatives considered:** `u64` field (serde error text). **Source:** existing code. **Evidence:** `config.rs:428` (`serde_norway::Value` deserialize precedent), `config.rs:1173-1175`.
- **DEC-014:** The scheduler reaches the hooks through a `ScheduledBackend` decorator built in `app.rs` (`SearchBackend::scheduler()` default `None`); a backend without scheduler keeps today's immediate indexing, used only by tests that build an `AppState` by hand. **Obliged by:** SPEC-0100/FR-NEW-004. **Rationale:** about 25 test sites build `AppState { search: ... }`; adding a field would touch them all. **Alternatives considered:** `AppState.derived` field, a global. **Source:** existing code. **Evidence:** `tools/trash_search_e2e.rs:78`, `tools/project_removal_e2e.rs:159`, `state.rs:16-28`.
- **DEC-015:** `search.index`, `search.status` and `after_companion` are not changed. **Obliged by:** SPEC-0100/FR-NEW-001 (business rule). **Rationale:** the spec leaves them out. **Alternatives considered:** none. **Source:** spec. **Evidence:** `search/indexer.rs:213-244`.
- **DEC-016:** A soft delete and an undelete of a project call `Scheduler::forget_project`; a hard delete relies on the cascade. **Obliged by:** SPEC-0100/FR-NEW-018, FR-NEW-019. **Rationale:** the soft delete keeps the `project` row, so no cascade fires. **Alternatives considered:** listing filters deleted projects only. **Source:** existing code. **Evidence:** `storage/admin.rs:510-540`.
- **DEC-017:** A restore from the trash calls a new `after_restore` (clear the trash path, schedule the destination) instead of `after_move`. **Obliged by:** SPEC-0100/FR-NEW-001, FR-NEW-006. **Rationale:** a move moves jobs and re-indexes at once; a restore is a change. **Alternatives considered:** a flag on `after_move`. **Source:** existing code. **Evidence:** `tools/trash.rs:206-218`.
- **DEC-018:** A move keeps indexing the destination immediately and without a job (as today) and moves jobs by `src`: `after_move` gains the `src` argument. **Obliged by:** SPEC-0100/FR-NEW-006. **Rationale:** the spec says entries follow the move within 5 s without a job. **Alternatives considered:** schedule the destination (contradicts FR-NEW-006). **Source:** existing code. **Evidence:** `search/indexer.rs:322-345`.
- **DEC-019:** Failure reason: an error whose text starts with `embedding request failed` or `embedding endpoint returned` becomes `embedding service unreachable`; any other error is `<ERR_CODE>: <message>`. **Obliged by:** SPEC-0100/FR-NEW-012. **Rationale:** matches the two failure texts of `embed`. **Alternatives considered:** a typed error (touches the whole `ToolError`). **Source:** existing code. **Evidence:** `search/embedding.rs:33-41`.
- **DEC-020:** Boot recovery runs in `app.rs` after the stores are built, before the router serves: `Scheduler::recover()` ends `running` rows as `failed` (`interrupted by a server restart`) and re queues each, and re queues `waiting` rows with the full delay. Shutdown stops the dispatcher and leaves rows as they are. **Obliged by:** SPEC-0100/FR-NEW-007, FR-NEW-017. **Rationale:** rows are the truth; the dispatcher is stateless. **Alternatives considered:** lazy recovery on first request. **Source:** existing code. **Evidence:** `app.rs:160-190`.
- **DEC-021:** Rebuild reads the file at start, not at schedule time; the hooks pass only a path. **Obliged by:** SPEC-0100/FR-NEW-001. **Rationale:** the content of the last save is what must be indexed. **Alternatives considered:** carry the text (stale, large). **Source:** design. **Evidence:** `search/indexer.rs:185-196` (`after_write_reread`).
- **DEC-022:** `list` reads waiting and running from memory and finished jobs from the table with a keyset query ordered `ended_at DESC, path ASC, kind ASC`; index `idx_derived_job_page`. **Obliged by:** SPEC-0100/FR-NEW-010 (10 000 jobs under 2 s). **Rationale:** no full scan per page. **Alternatives considered:** offset paging (marker drift). **Source:** design. **Evidence:** n/a.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0264 | an index update refused or failed keeps the old entries | P1 | FR-NEW-011, FR-NEW-012, FR-NEW-020 (partial) | main | 2 | no |
| US-0265 | PostgreSQL and sqlite vec replace a file's entries in one transaction | P1 | FR-NEW-011, FR-NEW-012, FR-NEW-020 (partial) | US-0264 | 2 | no |
| US-0266 | job records and `fs.list_derived_jobs` | P1 | FR-NEW-010 | main | 1 | no |
| US-0267 | the settle delay setting | P1 | FR-NEW-014, FR-NEW-016 | main | 2 | no |
| US-0268 | a saved file is rebuilt once after it settles | P1 | FR-NEW-001 to FR-NEW-004, FR-NEW-011 to FR-NEW-014, FR-NEW-020, FR-MOD-001 | US-0264, US-0265, US-0267, US-0266 | 1 | no |
| US-0269 | a copy and a restore schedule their destination | P2 | FR-NEW-001 (partial) | US-0268 | 2 | no |
| US-0270 | waiting rebuilds survive a restart | P2 | FR-NEW-007, FR-NEW-017 | US-0268 | 2 | no |
| US-0271 | delete and move follow the jobs | P2 | FR-NEW-005, FR-NEW-006 | US-0268 | 1 | no |
| US-0272 | index mode change and project removal cancel jobs | P2 | FR-NEW-015, FR-NEW-018, FR-NEW-019 | US-0268 | 2 | no |
| US-0273 | `fs.recalculate`, docs and contract | P1 | FR-NEW-008, FR-NEW-009, FR-NEW-013 (partial) | US-0268, US-0271 | 2 | yes |
