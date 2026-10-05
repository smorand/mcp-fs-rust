# US-0002: Schema & configuration foundation

> Parent Spec: specs/SPEC-0014_2026-10-04_17-48-41-auto-purge-files-and-projects/spec.md
> Spec ID: SPEC-0014
> Epic: n/a
> Status: ready
> Priority: 2
> Depends On: none
> Complexity: S
> min_tier: 2
> Files touched: 2

## Objective

Adds the persisted schema and global configuration every other story in this spec builds on:
the nullable `project.deleted_at` column, the `project_purge_config` table, and two new
`SafetyConfig` keys. No behavior change yet — every new column defaults to inactive, so existing
projects are unaffected until a later story wires something to read/write them.

## Technical Context

### Stack
Rust 2024, the dialect-abstracted relational layer (`storage::rel`), SQLite/PostgreSQL/SQL Server.

### Relevant File Structure
```
crates/core/src/
  storage/
    rel/
      schema.rs       (SchemaSet/Table/Column declarations — add here)
    admin.rs          (project table definition lives here, read for reference)
  config.rs           (SafetyConfig — add the two new fields)
```

### Existing Patterns
- Adding a nullable column to an already-deployed table: `project.index_mode` was added this way,
  via `column_migration()` (`storage/admin.rs:63-70`). `project.deleted_at` follows the identical
  pattern.
- `project` and `project_member` tables (`storage/admin.rs:28-68`): `project(id, owner,
  created_at)` has **no `volume_id` column** — the admin registry is global to the deployment
  (`storage/admin.rs:1-6`). `project_purge_config` follows the same convention: keyed on
  `project_id` alone, no `volume_id`.
- Every new persisted column/table goes through the dialect checklist in
  `.agent_docs/backends.md`: declare once in `storage/rel/schema.rs` via `SchemaSet`/`Table`/
  `Column`/`ColumnType`, never hand-build per-dialect DDL.
- `SafetyConfig` (`config.rs:359-377`) is a plain struct with `#[serde(default = "...")]`-style
  defaults; follow that exact shape for the two new fields.

### Data Model (excerpt)
- `project.deleted_at: Option<Timestamp>` — new nullable column on the existing `project` table.
- `project_purge_config` — new table: `project_id` (PK, FK → `project.id`),
  `autopurge_enabled: bool`, `use_internal_purge: bool`, `file_retention_days: Option<u32>`,
  `project_retention_days: Option<u32>`.
- `SafetyConfig.purge_interval_secs: i64` (default `3600`).
- `SafetyConfig.project_purge_grace_days: i64` (default `30`).

### Decisions That Govern This Story
- **DEC-008** (parent spec Section 17): the grace period is one global config value, not
  per-project.
- Parent spec Section 8 (corrected during the implementability audit, round 2→3): the
  `volume_id`-in-every-key convention applies only to the per-volume metadata tree (`nodes`,
  `blob_refs`, `git_*`), never to `project` or `project_purge_config` — the admin registry has no
  `volume_id` column at all.

### Applicable NFRs
None beyond the dialect checklist (Section 9.5: no new crate dependencies, additive-only schema
change, no data migration of existing rows).

## Functional Requirements

### FR-NEW-018 [EARS-U]: New global configuration
> The system SHALL add `safety.purge_interval_secs` (default `3600`) and
> `safety.project_purge_grace_days` (default `30`) to `SafetyConfig`.

- **Inputs / Outputs:** YAML keys under `safety:`; struct fields `purge_interval_secs: i64`,
  `project_purge_grace_days: i64`.
- **Business Rules:** both default when absent from YAML; both load correctly when present and
  overridden.

## Acceptance Tests

> **100% must pass.** Run via `make test` / `cargo test --workspace`.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| YAML config fixture | A config file setting `safety.purge_interval_secs: 60` and `safety.project_purge_grace_days: 7` | fixture, test-only | ready |

### E2E-NEW-074: Config defaults
- **Category:** happy
- **Scenario:** cross-cutting (feeds SC-002/003/004/007 elsewhere)
- **Requirements:** FR-NEW-018
- **Preconditions:** no config override present.
- **Steps:**
  - Given no YAML override
  - When `SafetyConfig::default()` is constructed
  - Then `purge_interval_secs == 3600` and `project_purge_grace_days == 30`
- **Priority:** Critical

### E2E-NEW-075: Config override
- **Category:** happy
- **Scenario:** cross-cutting
- **Requirements:** FR-NEW-018
- **Preconditions:** YAML sets both keys to non-default values.
- **Steps:**
  - Given YAML config sets `safety.purge_interval_secs: 60` and
    `safety.project_purge_grace_days: 7`
  - When the server/CLI loads config
  - Then both values are reflected exactly in the loaded `ServerConfig`
- **Priority:** Critical

### E2E-NEW-076: Config boundary value
- **Category:** edge
- **Scenario:** cross-cutting
- **Requirements:** FR-NEW-018
- **Preconditions:** YAML sets `purge_interval_secs: 0`.
- **Steps:**
  - Given `safety.purge_interval_secs: 0` in YAML
  - When config loads
  - Then either boot-time validation rejects it with a clear error, or it loads and the loop
    degrades to a tight-but-non-panicking cycle — pick one explicit behavior and assert it; do not
    leave this silently undefined
- **Priority:** Medium

## Constraints

### Files Not to Touch
`tools/admin.rs`, `mcp/server.rs`, `cli.rs`, `app.rs` — no tool, CLI, or loop wiring in this
story; it is schema and config only.

### Dependencies Not to Add
None.

### Patterns to Avoid
Do not add a `volume_id` column or predicate to `project_purge_config` or `project.deleted_at` —
see the Decisions section above; this was a corrected implementability finding in the parent
spec, not a stylistic choice.

### Scope Boundary
Create the schema and config fields only. No code anywhere else reads or writes them yet — that
starts in US-0003 (atime) through US-0012.

## Non Regression

### Existing Tests That Must Pass
`cargo test --workspace`, with particular attention to `storage/conformance.rs` (the cross-dialect
suite) continuing to pass unmodified on SQLite — the new table/column must not change any existing
row's shape.

### Behaviors That Must Not Change
Existing `project` rows: `index_mode`'s column-migration precedent must not be disturbed; this
story adds `deleted_at` the same way, not by touching `index_mode`'s migration code.

### API Contracts to Preserve
None affected — no tool or route yet reads/writes the new columns.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
