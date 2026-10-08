# US-0001: Foundation — `export_links` schema and storage primitives

> Parent Spec: specs/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md
> Spec ID: SPEC-0012
> Epic: n/a
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 1

## Objective
Give the rest of this feature the persistence primitives it needs: a new `export_links` table
(following the `trash_entries` precedent, not a separate sub-store) and three accessor functions —
insert a pending export record, atomically delete-and-fetch one record by token (the single-use
consume operation every later story's download/race/sweep behavior depends on), and select every
record whose `expires_at` has passed. No caller of these functions exists yet; this story only builds
and tests the storage layer itself.

## Technical Context

### Stack
Rust 2024, `rusqlite` (SQLite), optional `sqlx` (PostgreSQL) / `tiberius-ng` (SQL Server) behind the
`postgres`/`sqlserver` features. Dialect-neutral schema DSL in `crates/core/src/storage/rel/`.

### Relevant File Structure
```
crates/core/src/storage/
  meta.rs          <- add the export_links table + accessor fns here
  rel/
    schema.rs      <- Table/Column DSL reference (read, do not modify)
    dialect.rs      <- Dialect rendering reference (read, do not modify)
    sqlite.rs / postgres.rs / sqlserver.rs   <- already generic over the schema DSL, no changes needed
```

### Existing Patterns
- `trash_entries` is the table to mirror exactly: declared at `crates/core/src/storage/meta.rs:149`
  (and its index at `meta.rs:172-173`), with accessor functions in the same file — insert at
  `meta.rs:287-318`, delete at `meta.rs:380`/`meta.rs:640`, paged list at `meta.rs:273`. Follow this
  file's existing structure (one `mod tests` block, free functions taking `&mut dyn RelationalTx` or
  similar, not a new wrapper type).
- The `Table`/`Column` schema DSL: see `crates/core/src/git/db.rs:150-227`'s `schema()` function for
  the exact calling convention (`Table::new(name, vec![Column::required(...), ...], primary_key_cols)`).
  `export_links` is declared the same way, but **inside `meta.rs`'s existing schema list**, not as a
  new standalone `schema()` function — `git/db.rs` owns its own `RelationalGitDb`/migration path
  because git objects are a separate sub-store; `export_links` is core-engine state and belongs with
  `nodes`/`blob_refs`/`trash_entries`.
- `RelationalTx`/`RelationalDb::execute`/`RelationalTx::execute` return real affected-row counts on
  all three dialects: SQLite (`storage/rel/sqlite.rs:155`), PostgreSQL (`postgres.rs:238`/`:267` via
  `rows_affected()`), SQL Server (`sqlserver.rs:365-369`, summed `rows_affected()`). This is what makes
  an atomic "delete row X, tell me whether it existed" operation possible: `DELETE FROM export_links
  WHERE token=? AND expires_at>?` followed by a rows-affected check of exactly `1` *is* the atomic
  single-use/expiry primitive — no new dialect machinery is needed, this already works everywhere.
- `VOLUME_ID_LEN` is the existing constant bounding `volume_id` column width (see any `volume()`
  helper closure in `git/db.rs:164` for the pattern: `Column::required("volume_id",
  ColumnType::TextKey(VOLUME_ID_LEN))`).

### Data Model (excerpt)
New table `export_links`:

| Column | Type | Notes |
|---|---|---|
| `token` | `ColumnType::TextKey(36)` | primary key, UUIDv4 string |
| `volume_id` | `ColumnType::TextKey(VOLUME_ID_LEN)` | scopes which project/blob-backend instance owns this export |
| `created_at` | `ColumnType::Text` | RFC3339 |
| `expires_at` | `ColumnType::Text` | RFC3339, `created_at` + 300s |

Blob store: one object per export at key `export:{token}`, in the same backend (local dir / S3 bucket)
the owning `volume_id` already uses — not part of the content-addressed `nodes`/`blob_refs` graph,
exactly like git objects under `git:{sha}`. **This story does not touch the blob store at all** — it
is pure relational-layer plumbing. The blob reads/writes/deletes happen in later stories, directly via
`VolumeClient.blob` (`crates/core/src/storage/volume.rs:15`, a public `Arc<dyn BlobBackend>` field),
exactly as `git/odb.rs:42-44` already does with no `volume.rs` wrapper — nothing in `volume.rs` needs
to change for this or any later story.

### Decisions That Govern This Story
- **DEC-008:** Opaque random token (`uuid::Uuid::new_v4()`) + a server-side `export_links` record,
  not a stateless signed (HMAC/JWT) token. A record is needed regardless for cleanup bookkeeping, so
  statelessness buys nothing. **Implemented by:** FR-NEW-007, FR-NEW-015 (this story owns FR-NEW-015).
- **DEC-013:** Grace period for a never-downloaded export equals the link's own expiry — no separate
  grace window. The sweep's criterion is simply `expires_at < now`. This is why the "select expired"
  accessor function needs no second timestamp parameter.
- **DEC-014 (Approach A):** store export bytes directly in the existing `BlobBackend` under key
  `export:{token}` (blob key namespacing, mirroring `git:{sha}`), bypassing content-addressing
  entirely. Rejected writing the zip into the visible filesystem tree because it would pollute
  `fs.list`/`fs.grep`/search/quota/trash with ephemeral system-internal artifacts.
- **DEC-016:** `export_links` declared in the same schema module as `trash_entries`
  (`storage/meta.rs`), not a separate sub-store like `git/db.rs`, because it is core-engine state, not
  git-specific.

### Applicable NFRs
- **§7.6 Deployment:** no new infrastructure; reuses the existing relational store (SQLite/
  PostgreSQL/SQL Server) already configured for the project.
- **§7.4 Reliability:** this story has no direct reliability requirement of its own; it is the
  substrate the later purge-cleanup story's reliability note depends on.

### Bounded Context
**Lifecycle sweep** (partially) and the shared substrate for **Export creation** and **Signed
delivery**: this story builds the one place all three contexts' `export_links` row reads/writes go
through.

## Functional Requirements

### FR-NEW-015 [EARS-U]: Schema
- **EARS:** THE system SHALL declare the `export_links` table in the same schema module as
  `trash_entries` (`crates/core/src/storage/meta.rs`), with columns `token` (`TextKey(36)`, primary
  key), `volume_id` (`TextKey(VOLUME_ID_LEN)`), `created_at` (`Text`), `expires_at` (`Text`), migrated
  identically across SQLite, PostgreSQL and SQL Server via the store's existing migration path.
- **Inputs / Outputs:** n/a (structural)
- **Business Rules:** the table lives in `meta.rs`'s existing schema list, not a new `schema()`
  function; column types exactly as declared above.
- **Exact names:** table `export_links`; column types `ColumnType::TextKey(36)`,
  `ColumnType::TextKey(VOLUME_ID_LEN)`, `ColumnType::Text` (x2)

Also implement, in the same file, the three accessor functions `FR-NEW-007`, `FR-NEW-009` and
`FR-NEW-014` need later (not separately numbered FRs — they are the mechanical consequence of
FR-NEW-015 existing, and this story's whole point is to make them available):
1. **Insert** one pending row: `(token: &str, volume_id: &str, created_at: &str, expires_at: &str) -> Result<()>`.
2. **Atomic delete-and-fetch** by token: given `token` and "now" (RFC3339 string), delete the row
   `WHERE token=? AND expires_at>?` in one statement, return whether exactly one row was deleted (the
   `rows_affected()` pattern cited above). This single primitive is what FR-NEW-009 (happy path),
   FR-NEW-010 (replay/unknown), FR-NEW-011 (expired) and FR-NEW-013 (concurrency) all build on in
   later stories — get its signature right here, since every later story's floor check depends on it.
3. **Select expired**: given `volume_id` and "now", return every row (or just its tokens) with
   `expires_at <= now`, for the sweep to iterate and delete (FR-NEW-014, later story).

## Acceptance Tests

> **100% must pass.** Run through `cargo test --workspace` (the project's canonical test command,
> `test.sh:5`/`Makefile:16`). Never run test files directly.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| fresh SQLite/Postgres/SQL Server stores | one per dialect, no prior `export_links` data | the existing `storage::conformance` suite's fixture setup | ready |
| sample row | `token` a UUIDv4 string, `volume_id="proj"`, `created_at`/`expires_at` RFC3339 | test-generated | ready |

### E2E-NEW-040: structural, export_links schema declared alongside trash_entries, migrated identically
- **Category:** Structural
- **Scenario:** SC-001 (cross-cutting, assigned here per the matrix's convention for structural/
  negative requirements — this is the story that actually builds the schema)
- **Requirements:** FR-NEW-015
- **Preconditions:** `crates/core/src/storage/meta.rs`'s schema module
- **Steps:**
  - Given `crates/core/src/storage/meta.rs`'s schema module
  - When checking the module `export_links` is declared in
  - Then it's the same module/list as `trash_entries`, and the conformance suite's create/read/delete
    cycle passes identically across SQLite, PostgreSQL, SQL Server
- **Cleanup:** none (structural check)
- **Priority:** High

### E2E-NEW-050: structural, export_links migration runs cleanly on a fresh database, all three dialects
- **Category:** Structural
- **Scenario:** SC-001
- **Requirements:** FR-NEW-015
- **Preconditions:** a brand-new SQLite file, a fresh PostgreSQL schema, a fresh SQL Server database
  (no prior `export_links` data)
- **Steps:**
  - Given the three fresh databases above
  - When the store's migration path runs
  - Then `export_links` exists with the exact declared columns on all three, and a subsequent
    insert/read/delete cycle (using this story's three accessor functions) succeeds identically on
    each
- **Cleanup:** drop the three test databases/schemas
- **Priority:** Medium

**Additional unit coverage this story must also provide** (not matrix E2E ids, but required by the
floor check — this story must be independently verifiable without any consumer existing yet):
- a direct test of the insert accessor: insert one row, read it back, assert every column matches.
- a direct test of the atomic delete-and-fetch accessor: insert a row with `expires_at` in the
  future, call delete-and-fetch with "now" before expiry, assert it reports success and the row is
  gone; call it again on the same token, assert it reports "not found" (this is the exact mechanism
  E2E-NEW-016/017/020/021/024/025 in US-0003 will later assert through the HTTP layer — get the
  not-found-on-replay behavior right here, at the storage layer, first).
- a direct test of the atomic delete-and-fetch accessor with `expires_at` already in the past: assert
  it reports "not found" even though the row exists (this is FR-NEW-011's storage-layer mechanism).
- a direct test of the select-expired accessor: insert one expired row and one live row, assert only
  the expired one is returned.

## Constraints

### Files Not to Touch
- `crates/core/src/storage/volume.rs` — confirmed in the spec's Phase 6 audit that no change is
  needed here; `VolumeClient.blob` is already public. Do not add a wrapper method "for symmetry."
- `crates/core/src/git/db.rs` — do not extend or reuse this; it is git's own sub-store, a different
  precedent (DEC-016 explicitly rejected following it).

### Dependencies Not to Add
- No new crate. `uuid` and the dialect crates are already present.

### Patterns to Avoid
- Do not write per-dialect `CREATE TABLE` SQL strings by hand anywhere. The `Table`/`Column` DSL
  renders them generically; a hand-written DDL string is exactly the mistake `git/db.rs`'s own
  `schema()` function demonstrates how to avoid.

### Scope Boundary
- This story does not call the new accessor functions from any tool, route, or sweep. It only builds
  and unit-tests them. US-0002, US-0003 and US-0004 are the consumers.

## Non Regression

### Existing Tests That Must Pass
- The full existing `storage::conformance` suite, unmodified, on all three dialects.
- Every existing `meta.rs` test (notably the `trash_entries` tests), unmodified.

### Behaviors That Must Not Change
- `trash_entries`'s own schema, accessors and tests are untouched.

### API Contracts to Preserve
- No public API changes outside the new (additive) accessor functions.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
