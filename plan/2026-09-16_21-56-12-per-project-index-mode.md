# Per-project search index mode

> Generated: 2026-09-16
> Request: Auto-index per project with four modes: none, bm25, rag, both. Setting a mode triggers
> an initial full index of existing files; unsetting it (back to none) wipes the index.
> Every subsequent write/delete on the volume is automatically reflected in the index.

## Summary

A new `index_mode` column is added to the `project` table (default `none`). Two new admin
tools let owners/admins set and read it. Changing the mode from `none` to any active mode
triggers a background full-index pass over the volume's existing files. Changing to `none`
wipes the index. Every `fs.write` / `fs.edit` / REST upload on an active-mode project
auto-indexes the file after the write; every delete auto-removes it. Auto-indexing is
fire-and-forget: a slow or absent embedding endpoint never fails a write.

## Scope

In scope:
- `index_mode` column on `project` table, schema migration via `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`
- `IndexMode` enum: `None`, `Bm25`, `Rag`, `Both`
- `AdminBackend` trait: `set_index_mode`, `get_index_mode`
- Two new MCP tools: `admin.set_index_mode`, `admin.get_index_mode`
- `admin.list_projects` and `admin.list_all_projects` return shape gains `index_mode` field
- `SearchBackend` trait: new `delete_all(volume_id)` method for wipe-on-disable
- Side-effect engine: `ProjectIndexer` (in `search/indexer.rs`) — wipe + full-index background task + write/delete hooks
- Auto-index hooks in tool handlers (`fs.write`, `fs.edit`, `fs.multi_edit`, `fs.search_replace`, `fs.insert_at_line`, `fs.apply_patch`, `fs.append`, `fs.write_bytes`, `fs.write_docx`) and in the REST upload handler (`api/dataplane.rs`)
- Auto-delete hook in the `fs.delete` tool handler
- TOOL_CONTRACT.txt + golden JSON updated (2 new tools)
- `.agent_docs/search.md` updated
- AGENTS.md updated (tool count)
- Unit + integration tests for the new tools and the indexer logic

Out of scope:
- Automatic re-chunking when `chunk_size` / `chunk_overlap` config changes (documented, not automated)
- Progress reporting for the initial full-index pass (background task, no streaming)
- `admin.reindex_project` (full forced reindex without mode change) — deferred

## Approach

### Where index_mode lives

The mode is stored in the `project` row, not in a separate table and not in server config.
It is a project property, owned by the project owner, visible to members via `admin.get_index_mode`.

Alternative considered: a separate `project_search` table. Rejected: the mode is a single
scalar on the project; a second table adds a join on every project read for no benefit.

### Schema migration

`SchemaSet::render` uses `CREATE TABLE IF NOT EXISTS` — it handles new tables idempotently
but cannot add a column to an existing table. Adding `index_mode` to the `project` table
requires `ALTER TABLE project ADD COLUMN index_mode TEXT NOT NULL DEFAULT 'none'`.

The existing pattern for this: run an explicit `ALTER TABLE` in `AdminBackend::connect()`
after the `migrate(&schema())` call. Each engine has a different guard:
- SQLite: `ALTER TABLE project ADD COLUMN index_mode TEXT NOT NULL DEFAULT 'none'` fails
  silently if the column already exists (no `IF NOT EXISTS` in older SQLite). Wrap in a
  `PRAGMA table_info(project)` check, or simply catch the "duplicate column name" error.
  Actually SQLite 3.37+ supports `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`. The bundled
  `rusqlite` ships SQLite 3.45+, so this is safe to use directly.
- PostgreSQL: `ALTER TABLE project ADD COLUMN IF NOT EXISTS index_mode TEXT NOT NULL DEFAULT 'none'`
  is standard and idempotent.
- SQL Server: `IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID(N'project')
  AND name = N'index_mode') ALTER TABLE project ADD index_mode NVARCHAR(MAX) NOT NULL DEFAULT 'none'`

A new `RelationalDb::add_column_if_missing(table, column, type_ddl, default)` method
would make this cross-dialect. But that adds a new trait method for one use case.
Simpler: add a new `SchemaSet::column_migrations: Vec<ColumnMigration>` field alongside
the existing `tables` field, with per-dialect rendering. `migrate()` runs them after tables.
This stays inside the existing abstraction without a new trait method.

`ColumnMigration`:
```rust
pub struct ColumnMigration {
    pub table: &'static str,
    pub column: &'static str,
    pub ty: ColumnType,
    pub not_null: bool,
    pub default: &'static str,   // SQL literal, e.g. "'none'"
}
```

Rendered per dialect:
- SQLite: `ALTER TABLE "{table}" ADD COLUMN IF NOT EXISTS "{col}" {type} NOT NULL DEFAULT {default}`
- PostgreSQL: same
- SQL Server: `IF NOT EXISTS (...) ALTER TABLE [{table}] ADD [{col}] {type} NOT NULL DEFAULT {default}`

### Auto-indexing architecture

A new `search/indexer.rs` module holds `ProjectIndexer`, the single place that knows how
to map a `(project_id, IndexMode, SearchBackend)` to a set of file operations. It is
called from:
1. `admin.set_index_mode` tool handler (the mode change side effect)
2. Every write tool handler (fire-and-forget hook)
3. The `fs.delete` tool handler (fire-and-forget hook)
4. The REST upload handler in `api/dataplane.rs`

The `ProjectIndexer` is **not** stored in `AppState`. The state already holds
`Option<Arc<dyn SearchBackend>>`. The indexer is a stateless struct that borrows state:

```rust
pub struct ProjectIndexer<'a> {
    backend: &'a Arc<dyn SearchBackend>,
    admin: &'a Arc<dyn AdminBackend>,
    config: &'a SearchConfig,
}
```

Key methods:
- `async fn on_mode_change(project_id, new_mode, old_mode, volume_client)` — wipe if needed,
  spawn background full-index task if new_mode != none
- `async fn on_write(project_id, path, text)` — fire-and-forget index_path
- `async fn on_delete(project_id, path)` — fire-and-forget delete_path
- `async fn full_index(project_id, mode, volume_client)` — iterate all files, index each

**Fire-and-forget pattern**: `on_write` and `on_delete` call `tokio::task::spawn` with the
backend operation. Errors are logged at WARN level. The outer write never waits for the
index operation and never fails because of it.

**Initial full-index**: spawned as a `tokio::task::spawn` from `on_mode_change`. The task
iterates all files in the volume via `fs_ops::iter_files`, reads each one, chunks and
indexes it. Large volumes may take minutes; this is acceptable for an explicit admin
action. No progress reporting in this iteration (deferred).

### Auto-index hook placement

The hooks go in the **tool handlers**, not in `fs_ops`. Rationale: `fs_ops::commit` is the
storage engine; policy (search indexing) belongs in the presentation layer. This is the
same separation the doc service follows: `write_bytes_documented` is a tool-layer function,
not a new `commit` variant.

In every write tool handler, after the `fs_ops::*` call returns `Ok`, add:

```rust
// Fire-and-forget: auto-index the written file if the project has an active mode.
if let Some(backend) = &ctx.state.search {
    let mode = ctx.state.admin.get_index_mode(&mount).await.unwrap_or(IndexMode::None);
    if mode != IndexMode::None {
        let indexer = ProjectIndexer::new(backend, &ctx.state.config.search);
        indexer.on_write(&mount, &norm, &new_text).await;  // spawn internally
    }
}
```

The `get_index_mode` call is one admin DB row read per write. On SQLite this is a
single-mutex serialized read; on PostgreSQL it hits the connection pool. To avoid the DB
round-trip on every write, add a `tokio::sync::RwLock<HashMap<String, IndexMode>>`
in-memory cache on `AppState` that is populated lazily and invalidated when
`admin.set_index_mode` is called. But this is an optimization: start without the cache,
add it if profiling shows the DB read is a bottleneck. The admin store is already fast
for single-row reads.

For the **delete** hook, the tool handler for `fs.delete` already has the normalized path
and the volume_id. Same pattern: check mode, call `indexer.on_delete()`.

### `SearchBackend` trait addition

Add `delete_all(volume_id: &str) -> Result<usize>` to the trait. Each backend deletes all
rows/chunks for the given volume_id. Returns the count of deleted chunks. This is called
on `set_index_mode(..., None)` or when the mode changes to a different active mode (wipe
then re-index with the new config).

Implementations:
- `TantivyBm25Backend`: delete all Tantivy documents for the volume (can be done by
  deleting the entire index directory for that volume, which is the cleanest option for
  Tantivy)
- `PostgresBm25Backend`: `DELETE FROM search_fts WHERE volume_id = $1`
- `SqliteVecBackend`: `DELETE FROM search_vec_meta WHERE volume_id = ?1` + corresponding
  `search_vec` rows
- `PostgresVectorBackend`: `DELETE FROM search_chunks WHERE volume_id = $1`
- `CombinedBackend`: call both inner `delete_all`

### New MCP tools

```
admin.set_index_mode
  desc: Set the search index mode for a project (owner or platform admin).
        none=no index, bm25=full-text only, rag=vector only, both=full-text and vector.
        Switching to an active mode triggers an initial full index of existing files in
        the background. Switching to none wipes the index immediately.
  params: project_id:string, mode:string
  required: [project_id, mode]
  returns: {"project_id": "p", "index_mode": "bm25", "previous_mode": "none",
            "reindex_started": true}

admin.get_index_mode
  desc: Get the current search index mode for a project (member or platform admin).
  params: project_id:string
  required: [project_id]
  returns: {"project_id": "p", "index_mode": "bm25"}
```

Authorization:
- `set_index_mode`: `require_owner_or_admin` (same gate as `delete_project`)
- `get_index_mode`: `require_member` (any member can read the mode)

Validation in `set_index_mode`:
- Mode must be one of `none`, `bm25`, `rag`, `both`
- If mode is `rag` or `both` and `search.embedding.endpoint` is empty: `ERR_INVALID_ARGUMENT`
  (cannot index vectors without an embedding endpoint)
- If search backend is `None` (search disabled): `ERR_NOT_SUPPORTED`

## Changes

### New files

| Path | Purpose |
|---|---|
| `crates/mcp-fs/src/search/indexer.rs` | `ProjectIndexer`: mode-change side effects, write/delete hooks, full-index background task |

### Modified files

| Path | Change |
|---|---|
| `crates/mcp-fs/src/storage/rel/schema.rs` | Add `ColumnMigration` struct and `SchemaSet::column_migrations` field; extend `render()` to emit `ALTER TABLE ADD COLUMN IF NOT EXISTS` per dialect |
| `crates/mcp-fs/src/storage/rel/dialect.rs` | Add `render_column_migration(dialect, migration) -> String` |
| `crates/mcp-fs/src/storage/rel/mod.rs` | Export `ColumnMigration` |
| `crates/mcp-fs/src/storage/admin.rs` | Add `index_mode` column to `project` schema; add `ColumnMigration` for the `ALTER TABLE`; add `set_index_mode` / `get_index_mode` to `RelationalAdminStore`; update row-reading code to include `index_mode` |
| `crates/mcp-fs/src/storage/traits.rs` | Add `IndexMode` enum (derives `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`); add `set_index_mode` / `get_index_mode` to `AdminBackend` trait; add `index_mode` field to `Project` struct |
| `crates/mcp-fs/src/search/mod.rs` | Add `pub mod indexer;`; add `delete_all` to `SearchBackend` trait; implement it on `CombinedBackend` |
| `crates/mcp-fs/src/search/bm25_sqlite.rs` | Implement `delete_all`: delete the Tantivy index directory for the volume |
| `crates/mcp-fs/src/search/bm25_pg.rs` | Implement `delete_all`: `DELETE FROM search_fts WHERE volume_id = $1` |
| `crates/mcp-fs/src/search/vector_sqlite.rs` | Implement `delete_all`: delete from `search_vec_meta` + `search_vec` |
| `crates/mcp-fs/src/search/vector_pg.rs` | Implement `delete_all`: `DELETE FROM search_chunks WHERE volume_id = $1` |
| `crates/mcp-fs/src/tools/admin.rs` | Add `admin.set_index_mode` and `admin.get_index_mode` tools |
| `crates/mcp-fs/src/tools/write.rs` | Add fire-and-forget `on_write` hook to `fs.write`, `fs.edit`, `fs.multi_edit`, `fs.search_replace`, `fs.insert_at_line`, `fs.apply_patch`, `fs.append`, `fs.write_bytes`, `fs.write_docx` handlers |
| `crates/mcp-fs/src/tools/lifecycle.rs` | Add fire-and-forget `on_delete` hook to `fs.delete` handler |
| `crates/mcp-fs/src/api/dataplane.rs` | Add `on_write` hook to the upload handler; `search: None` in test state updated |
| `TOOL_CONTRACT.txt` | Append `admin.set_index_mode` and `admin.get_index_mode` entries |
| `tool-contract-golden.json` | Regenerate |
| `AGENTS.md` | Update tool count (61 + 2 = 63 when search enabled); update documentation index |
| `.agent_docs/search.md` | Document per-project index modes, the auto-indexing behavior, latency trade-offs, and the `delete_all` wipe semantics |

### Public API changes

**`Project` struct gains `index_mode: IndexMode`** — a breaking change to the struct.
All sites that construct `Project` (in `admin.rs`) and all sites that read `project_id` or
`owner` from a `Project` must include `index_mode`. There are exactly 2 construction sites
and the golden contract test covers the JSON return shape, so drift is caught automatically.

**`admin.list_projects` and `admin.list_all_projects` JSON** gain `"index_mode": "none"` in
every project entry. Additive change, backwards compatible for consumers that ignore unknown fields.

**`AdminBackend` trait** gains two new async methods. Every `impl AdminBackend` must be
updated. There are two: `RelationalAdminStore` (the real one) and the in-memory test stub.

**`SearchBackend` trait** gains `delete_all`. Every impl must be updated (5 backends +
`CombinedBackend`).

## IndexMode values and transitions

| From \ To | none | bm25 | rag | both |
|---|---|---|---|---|
| **none** | no-op | wipe (noop) + full BM25 index | wipe + full RAG index | wipe + full BM25+RAG index |
| **bm25** | wipe BM25 | no-op (same mode) | wipe BM25 + full RAG | wipe BM25 + full BM25+RAG |
| **rag** | wipe RAG | wipe RAG + full BM25 | no-op | wipe RAG + full BM25+RAG |
| **both** | wipe both | wipe both + full BM25 | wipe both + full RAG | no-op (same mode) |

"Wipe" = call `backend.delete_all(volume_id)`.
"Full index" = background `tokio::spawn` iterating all volume files.
"no-op (same mode)" = return early, no wipe, no reindex (`reindex_started: false`).

## Auto-index write hook details

All write hooks follow this pattern (pseudocode):

```rust
// After the fs_ops call succeeds:
if let Some(backend) = &ctx.state.search {
    if let Ok(mode) = ctx.state.admin.get_index_mode(&mount).await {
        if mode != IndexMode::None {
            let b = backend.clone();
            let vol = mount.clone();
            let p = norm.clone();
            let t = text.to_string();  // already computed for the write
            let cfg = ctx.state.config.search.clone();
            tokio::task::spawn(async move {
                if let Err(e) = b.index_path(&vol, &p, &t,
                    cfg.chunk_size, cfg.chunk_overlap).await {
                    tracing::warn!(path=%p, error=%e, "auto-index write failed");
                }
            });
        }
    }
}
```

For tools that don't already have the new text as a string (e.g. `fs.append`, `fs.insert_at_line`),
read the file back from the volume after the write: `client.read_text(&norm).await` — the
just-written content is immediately available. This is an extra read per write for those
tools; acceptable given it only fires when the project has an active index mode.

The `write_bytes` path (binary uploads) skips text reading — if `from_utf8` fails, log
and skip silently, same as `search.index` already does.

## Assumptions and defaults

- `IndexMode::None` is the default for all existing projects (column `DEFAULT 'none'`).
  No existing project is indexed automatically without an explicit `set_index_mode` call.
- `chunk_size` and `chunk_overlap` for auto-indexing come from `config.search` (same values
  as the `search.index` tool defaults: 1000 / 100). No per-project chunk config in this iteration.
- The full-index background task runs on the tokio thread pool with no rate limiting or
  concurrency cap. For large volumes this may saturate the embedding endpoint. A concurrency
  limiter (`tokio::sync::Semaphore`) can be added in `indexer.rs` without changing the API.
- `delete_all` on `TantivyBm25Backend` removes the entire index directory (`state/search/{volume_id}/`).
  This is the cleanest approach for Tantivy (avoids segment fragmentation) and is safe because
  the directory is always rebuildable.
- `get_index_mode` reads directly from the admin store on every call (no cache). If profiling
  reveals this is a bottleneck on high-write workloads, a `tokio::sync::RwLock<HashMap>` cache
  on `AppState` is the next step (not in this plan).
- Both new tools are in the `admin.*` family (not `search.*`) because index mode is a project
  administration concern, not a search query concern.

## Test plan

| Test | Location | What it proves |
|---|---|---|
| `set_index_mode_owner_can_set` | `tools/admin.rs` | Owner sets mode, returns correct JSON |
| `set_index_mode_member_is_forbidden` | same | Non-owner gets `ERR_FORBIDDEN` |
| `set_index_mode_unknown_mode_is_invalid` | same | `"fancy"` → `ERR_INVALID_ARGUMENT` |
| `set_index_mode_rag_without_endpoint_is_invalid` | same | mode=rag + empty endpoint → `ERR_INVALID_ARGUMENT` |
| `get_index_mode_member_can_read` | same | Member reads the mode set by owner |
| `list_projects_includes_index_mode` | same | `admin.list_projects` JSON has `index_mode` field |
| `delete_all_removes_all_chunks_bm25_sqlite` | `search/bm25_sqlite.rs` | After delete_all, stats returns 0 |
| `delete_all_removes_all_chunks_vector_sqlite` | `search/vector_sqlite.rs` | Same |
| `delete_all_removes_all_chunks_bm25_pg` | `search/bm25_pg.rs` | Same, live PG |
| `delete_all_removes_all_chunks_vector_pg` | `search/vector_pg.rs` | Same |
| `column_migration_adds_index_mode` | `storage/admin.rs` | After connect() on existing DB, column exists with default 'none' |
| `on_write_hook_indexes_file` | `search/indexer.rs` | Write hook calls index_path on a mock backend |
| `on_delete_hook_removes_file` | same | Delete hook calls delete_path |
| `on_mode_change_none_wipes_index` | same | Mode → none calls delete_all |
| `on_mode_change_active_starts_reindex` | same | Mode → bm25 spawns background task |
| `auto_index_e2e_write_then_query` | `search/e2e.rs` | Write a file via `fs.write`, set mode to bm25, query finds it |
| `auto_index_e2e_delete_then_query_empty` | same | Delete file, query returns empty |
| `auto_index_e2e_mode_change_wipes_then_rebuilds` | same | Change mode bm25→rag: old BM25 gone, new RAG populated |
| `golden_contract_covers_two_new_admin_tools` | `tools/contract_golden.rs` | Schema + description frozen |

## Open risks

1. **Write latency on RAG mode**: every `fs.write` fires a `tokio::spawn` that eventually
   calls the embedding HTTP endpoint. The spawn is non-blocking but the embedding call
   itself takes 50–300ms. The write returns immediately; the index lags. For workloads
   that write and immediately query, there is a race window. Document this clearly.

2. **Full-index task is unobservable**: no tool reports progress or completion of the
   background reindex. An operator who sets mode to `rag` on a 10k-file volume has no way
   to know when it finishes (other than polling `search.status` and watching `vector_chunks`
   grow). `admin.reindex_project` with a progress token is the right follow-up.

3. **`delete_all` on `TantivyBm25Backend` removes the index directory**: if the server
   crashes between `delete_all` and the completion of the full-index pass, the volume has
   no index and the project mode says `bm25`. The state is consistent (index is empty, not
   corrupt), but the user must call `admin.set_index_mode` again to restart the rebuild.
   Documented in `.agent_docs/search.md`.

4. **`column_migration` is new schema machinery**: the `ColumnMigration` / `ALTER TABLE ADD
   COLUMN IF NOT EXISTS` path is new. It will be the second site that runs DDL on the admin
   store (after `SchemaSet`). Every `match` on `Dialect` stays exhaustive, so a missing
   rendering case fails to compile — same safety net as the rest of the dialect layer.
