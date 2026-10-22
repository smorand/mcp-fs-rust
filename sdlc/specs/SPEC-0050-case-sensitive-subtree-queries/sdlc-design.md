# SPEC-0050: Folder operations mix folders that differ only by letter case, design

> Spec: sdlc/specs/SPEC-0050-case-sensitive-subtree-queries/sdlc-spec.md
> Nature: BUG
> Depth: M
> Security: n/a
> Designed on: 2026-10-23
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, persistence | existing Rust workspace, `RelationalDb`/`Dialect` seam over SQLite, PostgreSQL, SQL Server | per `Cargo.toml` | DEC-001 |

No dependency added, so no context7 lookup. Skill loaded: `rust` (`make check` gate, library errors with
`ToolError`, real e2e on the tool methods, async code never blocks). Project state: existing. Binding earlier
decisions: SPEC-0049 DEC-004 (whole component match through `descendant_pattern`), the AGENTS.md rule "a LIKE
pattern is built with `descendant_pattern` / `Dialect::escape_like_literal`, never by hand".

## 2. Approaches

### Chosen: A, keep the escaped `LIKE` and add an exact, dialect rendered prefix equality

Every site that selects "a path and what lies below it" renders one clause from the dialect:
`(col LIKE <pattern> ESCAPE '\' AND substr(col, 1, n) = <prefix>)`. The `LIKE` keeps its job of narrowing
rows (and of using the primary key index on engines that do), the equality makes the decision case exactly.
Wildcard escaping stays as it is. One new helper in `Dialect`, one small value in `RelationalMetaStore`, and
seven call sites switch to it.

- Pros: one statement per operation as today (a permanent delete of 100 000 rows stays one `DELETE`); no schema
  change; the three engines share one clause shape with only the function name and the length unit changing;
  fixes reads, deletes, moves and listings with the same code.
- Cons and risks: the SQL Server rendering (`COLLATE` binary on the equality, UTF-16 length) cannot be run in
  the default suite; it is covered by a unit test of the rendered text and by the opt in SQL Server suite, and
  BL-0073 stays open on the whole key comparison there.
- Affected scope: `storage/rel/dialect.rs`, `storage/meta.rs`, `storage/conformance.rs`, tests.

### Rejected: B, `PRAGMA case_sensitive_like = ON` on SQLite connections

One line in `storage/sqlite.rs`. Rejected (DEC-002): SQLite documents the pragma as deprecated; it changes every
`LIKE` of the connection, including the git abbreviated hash lookup (`crates/core/src/git/db.rs:319`), where an
upper case abbreviation would stop matching; and it leaves PostgreSQL and SQL Server untouched, so the engines
would differ by design.

### Rejected: C, post filter the rows in Rust

Select with `LIKE`, drop wrong rows in Rust. Rejected (DEC-003): permanent removal would turn one `DELETE` into
one statement per row, `stale_files` would still lose the rows its `NOT LIKE` hides, and the paged trash listing
and its `total` would have to page in memory.

### Rejected: D, a range comparison `path >= '/video/' AND path < '/video0'`

Rejected (DEC-004): the order of text depends on the collation (PostgreSQL), so the range is not portable.

## 3. Architecture

```
 fs.delete / fs.move / fs.copy / fs.glob / fs.grep / fs.export_zip / fs.trash_list / purge
        |
        v
 core::fs_ops, storage::volume, purge            (unchanged: they call the meta store)
        |
        v
 RelationalMetaStore (storage/meta.rs)
   subtree(root)            \
   remove_subtree(path)      \   each builds   SubtreeMatch::new(dialect, path)
   tx_rename(.. pattern ..)   >  and renders    sm.clause(col, like_ph, prefix_ph)
   folder_items(folder, cap) /
   stale_files(.., exclude)  /   (NOT (clause))
   list/count trash entries /    (clause on original_path)
        |
        v
 Dialect (storage/rel/dialect.rs): substr_fn(), char_len(&str), exact_equals(lhs, rhs_ph)
```

The existing structure: `descendant_pattern` at `crates/core/src/storage/meta.rs:574`; the seven `LIKE` sites at
`meta.rs:719` (folder items), `1614` (rename), `1742` and `1783` (trash entries, count and page), `1936`
(subtree), `1996` (stale files), `2139` and `2164` (permanent removal); callers `volume.rs:179` (walk),
`fs_ops.rs:1358` (copy), `purge.rs:74`. The git hash `LIKE` (`crates/core/src/git/db.rs:319`) is a hexadecimal
abbreviation lookup and is out of scope.

## 4. Data model

No schema change, no migration, no new index. Reads and writes the same `nodes` and `trash_entries` rows.

## 5. Contracts and interfaces

No tool, route or answer changes. Internal signatures:

```rust
// storage/rel/dialect.rs
impl Dialect {
    /// `substr` (SQLite, PostgreSQL) or `SUBSTRING` (SQL Server).
    pub fn substr_fn(self) -> &'static str;
    /// Characters of `s` as the engine counts them: Unicode scalar values for SQLite and PostgreSQL,
    /// UTF-16 code units for SQL Server.
    pub fn char_len(self, s: &str) -> usize;
    /// `lhs = rhs` compared by code, case included: plain `=` on SQLite and PostgreSQL,
    /// `lhs = rhs COLLATE Latin1_General_100_BIN2` on SQL Server.
    pub fn exact_equals(self, lhs: &str, rhs: &str) -> String;
}

// storage/meta.rs
/// The two bound values and the length of one "path and below" selection.
struct SubtreeMatch { like: String, prefix: String, prefix_len: usize }
impl SubtreeMatch {
    fn new(dialect: Dialect, path: &str) -> Self;      // like = descendant_pattern, prefix = "<path>/" (root: "/")
    /// `(col LIKE ?L ESCAPE '\' AND substr(col, 1, n) = ?P)` with the given placeholder numbers.
    fn clause(&self, dialect: Dialect, col: &str, like_ph: usize, prefix_ph: usize) -> String;
}
```

`prefix` is the folder path with exactly one trailing `/` (root: `/`), `n` is `dialect.char_len(&prefix)`, written
into the SQL as an integer literal (never user text). The equality row `path = <root>` of each query is kept
as it is.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/rel/dialect.rs` | modify: `substr_fn`, `char_len`, `exact_equals`, unit tests of the rendered text | BR-001 to BR-007 | US-0074 |
| `crates/core/src/storage/meta.rs` | modify: `SubtreeMatch`; sites `remove_subtree` (US-0074), `tx_rename` (US-0075), `subtree` and `folder_items` (US-0076), trash entries and `stale_files` (US-0077) | BR-001 to BR-007 | US-0074 to US-0077 |
| `crates/core/src/tools/case_paths_e2e.rs`, `crates/core/src/tools/mod.rs` | create / modify | all BT | US-0074 to US-0077 |
| `crates/core/src/purge.rs` | modify: one test in its test module | BR-007 | US-0077 |
| `crates/core/src/storage/conformance.rs` | modify: a case collision case in `meta_like_escaping` | BR-001 to BR-004 | US-0077 |
| `.agent_docs/architecture.md`, `AGENTS.md` | modify: the rule "a path and below is selected by `SubtreeMatch`, case exact" | docs | US-0077 |

Affected existing tests: keep all (the spec's 4.4 table); the SPEC-0049 test
`spec_0049_us_0073_folders_differing_only_by_case_do_not_mix` stays and stays green.

## 7. Compatibility and risks

- Consumers: no contract changes; the only visible change is that a folder operation touches fewer rows when
  another folder differs from it only by letter case.
- Risk: SQL Server rendering unrun in the default suite. Mitigation: the unit test of the rendered text for all
  three dialects (E2E: `dialect` unit tests), the opt in suite (`MCPFS_TEST_MSSQL_DSN`), BL-0073.
- Risk: a wrong `n` for non ASCII names (an `École` folder). Mitigation: BT-008 uses accented capitals; `char_len`
  counts characters per engine.
- Ordering between stories: US-0074 introduces the helper and fixes the permanent delete only; until US-0075 to
  US-0077 land, the move, copy, reads, trash listing and purge still have the defect (no release between them).
- Rollback: revert the commits; nothing is persisted differently.

## 9. Test strategy

BT-001 to BT-006 and BT-008, BT-020 to BT-022 are functional e2e through the real `McpServer` tool methods over
real storage (as `disk_usage_e2e.rs` does), in `crates/core/src/tools/case_paths_e2e.rs`. BT-007 is an
integration test over the real stores in the test module of `crates/core/src/purge.rs` (the file purge is not a
tool). Extras beyond the acceptance set: a unit test of the three rendered clauses in `dialect.rs`, and a case
collision case in the cross engine conformance suite (`conformance.rs`, runs on SQLite by default and on
PostgreSQL and SQL Server with their opt in DSNs).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001, BT-020, BT-021, BT-022 | functional e2e | `fs_delete`, `fs_read`, `fs_list_dir`, `fs_move`, the usage read | `crates/core/src/tools/case_paths_e2e.rs` | US-0074 |
| BT-002, BT-003, BT-008 | functional e2e | `fs_delete` (trash), `fs_trash_list`, `fs_move`, `fs_read` | same file | US-0075 |
| BT-004, BT-005 | functional e2e | `fs_copy`, `fs_glob`, `fs_grep`, `fs_export_zip` | same file | US-0076 |
| BT-006 | functional e2e | `fs_delete` (trash), `fs_trash_list` | same file | US-0077 |
| BT-007 | integration | `purge::sweep_project_files` over the real stores | `crates/core/src/purge.rs` tests | US-0077 |

BUG rule: every reproduction test is RED on the current code; the expected failing assertion is stated in each
story. Test data is generated with the volume client; BT-007 sets file access times through the store, as the
existing purge tests do.

## 10. Decisions log

- **DEC-001:** No stack change; the fix lives in the existing storage seam. **Obliged by:** n/a (no code impact):
  nothing new is chosen. **Rationale:** conventions fix the stack. **Alternatives considered:** none viable.
  **Source:** existing code `Cargo.toml`, `AGENTS.md`. **Evidence:** n/a
- **DEC-002:** Do not use `PRAGMA case_sensitive_like`. **Obliged by:** SPEC-0050/BR-001, BR-006. **Rationale:**
  deprecated by SQLite, connection wide (it would change the git hash lookup), SQLite only. **Alternatives
  considered:** the pragma on every connection. **Source:** design. **Evidence:**
  `crates/core/src/git/db.rs:319`, `crates/core/src/storage/sqlite.rs:57-67`
- **DEC-003:** Decide the match in SQL, not in Rust. **Obliged by:** SPEC-0050/BR-001, BR-006, BR-007.
  **Rationale:** keeps one `DELETE`, keeps the paged `total` exact, and lets `stale_files` stop hiding rows.
  **Alternatives considered:** Rust post filter. **Source:** design. **Evidence:**
  `crates/core/src/storage/meta.rs:1996`, `2139`, `2164`, `1742`
- **DEC-004:** The exact test is a substring equality, not a range. **Obliged by:** SPEC-0050/BR-001 to BR-007.
  **Rationale:** portable on the three engines. **Alternatives considered:** `>=`/`<` range (collation
  dependent). **Source:** design. **Evidence:** `crates/core/src/storage/rel/dialect.rs:255-262` (precedent of a
  per engine function)
- **DEC-005:** `Dialect` renders the engine specific parts (`substr_fn`, `char_len`, `exact_equals`); the length
  is written as an integer literal. **Obliged by:** SPEC-0050/BR-001 to BR-007, BR-008. **Rationale:** the
  project rule that dialect differences live in `dialect.rs`; an integer literal is not injectable and avoids a
  placeholder in a function argument that SQL Server types differently. **Alternatives considered:** a bound
  length. **Source:** `AGENTS.md` conventions. **Evidence:** `crates/core/src/storage/rel/dialect.rs:267-273`
- **DEC-006:** On SQL Server the equality carries `COLLATE Latin1_General_100_BIN2`; the length counts UTF-16
  units. **Obliged by:** SPEC-0050/BR-001. **Rationale:** a binary collation compares by code unit whatever the
  database collation; `SUBSTRING` counts UTF-16 units. **Alternatives considered:** `Latin1_General_BIN2`
  (older, same effect for this use). **Source:** design; unverified on a live server (BL-0073). **Evidence:** n/a
- **DEC-007:** `stale_files` excludes the trash folder with `NOT (clause)`, so only the exact trash folder is
  exempt. **Obliged by:** SPEC-0050/BR-007. **Rationale:** same helper. **Alternatives considered:** exclusion in
  Rust (DEC-003). **Source:** design. **Evidence:** `crates/core/src/storage/meta.rs:1996`,
  `crates/core/src/purge.rs:74`
- **DEC-008:** The trash entry filter applies the clause to `original_path`; the equality `original_path = ?`
  of the existing predicate stays. **Obliged by:** SPEC-0050/BR-006. **Rationale:** the filter is paged and
  counted in SQL. **Alternatives considered:** in memory paging. **Source:** existing code. **Evidence:**
  `crates/core/src/storage/meta.rs:1726-1790`
- **DEC-009:** `folder_items` (SPEC-0049) switches to the helper and keeps its Rust `strip_prefix`, which
  buckets the rows anyway. **Obliged by:** SPEC-0050/BR-005. **Rationale:** one rule, and no wrong row read from
  the database. **Alternatives considered:** leave the Rust only check. **Source:** SPEC-0049. **Evidence:**
  `crates/core/src/storage/meta.rs:719`, `743`
- **DEC-010:** Tests: functional e2e on the tool methods for the user visible operations, an integration test for
  the purge, a dialect unit test and a conformance case as extras. **Obliged by:** SPEC-0050/all BT. **Rationale:**
  skill `rust` layering. **Alternatives considered:** unit tests of the store only. **Source:** skill.
  **Evidence:** `crates/core/src/tools/disk_usage_e2e.rs`, `crates/core/src/storage/conformance.rs:190`

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0074 | exact case for the permanent delete | P1 | BR-001, BR-008 | main | 2 | no |
| US-0075 | exact case for move, rename and the trash | P1 | BR-002, BR-003 | US-0074 | 2 | no |
| US-0076 | exact case for copy and recursive reads | P2 | BR-004, BR-005 | US-0074 | 2 | no |
| US-0077 | exact case for the trash listing and the purge, spec close | P2 | BR-006, BR-007 | US-0075, US-0076 | 2 | yes |
