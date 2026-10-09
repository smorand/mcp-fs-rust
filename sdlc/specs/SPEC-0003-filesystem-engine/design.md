# SPEC-0003 — Filesystem Engine: Design

## 1. Components

- **Filesystem engine** (`core/fs_ops.rs`): the single implementation of every `fs.*` operation — paged reads, byte reads, line-oriented helpers, writes, edits, multi-edit, fuzzy search-replace, the multi-file patch applier, listing, tree, metadata probes, glob, grep, symbol search, lifecycle (mkdir/delete/move/copy), and the two session-introspection tools.
- **Unified diff producer** (`core/diff.rs`): builds the `--- / +++ / @@` text returned by every editing tool.
- **Text utilities** (`util/text.rs`): line splitting and the shared `fnmatch`-semantics glob matcher (`Fnmatch`) used by `fs.glob` and `fs.grep`.
- **Tool adapters** (`tools/{read,write,edit,listing,metadata,lifecycle,search}.rs`): one module per family; each handler authorizes, normalizes every path, then calls the engine. No operation is implemented twice.
- **Frozen contract** (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`): the 32 tools' schemas, descriptions and (for this spec) result shapes.

## 2. Flows

### 2.1 Shared write/commit path
Every text mutation: charge quota → write bytes atomically → record audit entry, in that order (so a rejected charge writes nothing, and the log reflects what happened). Confirmed at `crates/core/src/core/fs_ops.rs:608-611` for `fs.write`'s path; the same ordering recurs at every other commit site (append, edit, multi_edit, search_replace, insert_at_line, patch update).

### 2.2 Read guard interaction
A write to an existing path requires a prior read of that exact path in the current session. Reads recorded by `fs.read`, `fs.read_bytes`, `fs.read_many` satisfy the guard; listing/glob/grep/stat/exists/hash never do. After any successful write, the path is recorded as read, so a chained overwrite in the same call sequence succeeds without an extra read.

### 2.3 Paging flow
`fs.read` computes `total_lines`, slices `[offset_lines, offset_lines + min(limit_lines, max_read_lines))`, numbers lines from `offset_lines + 1`, and sets `next_offset` to the end of the slice when more remains, else null.

### 2.4 Edit flow (unique match)
Read guard → read current text → count `old_string` occurrences → require exactly one (or any count under `replace_all`) → build diff of old vs new → commit unless `dry_run`.

### 2.5 Multi-edit flow
Read guard → fold `edits` sequentially over the text (each edit's `old_string` is searched in the output of the previous edit) → diff is original vs final → single commit, or none if any edit fails.

### 2.6 Fuzzy replace flow
Slide a window sized to `search_block`'s line count across the file; score each window by an LCS-based similarity ratio in `0.0..=1.0`; keep the best; accept only at ratio ≥ `FUZZY_THRESHOLD` (0.6); normalize the replacement to be newline-terminated; commit.

### 2.7 Patch flow
Parse `*** Begin Patch` / `*** End Patch` body into ordered `FileOp`s (Add/Update/Delete, optional Move). For each op in sequence: normalize its own path independently → Add: charge+write, no read-guard check; Update: read-guard check, apply hunks, charge+write, optional rename, **two audit entries** (one inside the update branch, one in the loop tail — deliberate, not a bug); Delete: read-guard check, remove. No rollback on a later failure: earlier ops in the same patch stand.

### 2.8 Walk flow (glob/grep/symbol search/tree)
Single bounded traversal skipping `DEFAULT_EXCLUDES` (`.git`, `node_modules`, `target`, `dist`, `.build`, `coverage`, trash dir), capped at `MAX_FILES` (5000). `fs.tree` uses a separate `TREE_EXCLUDES` list that omits the trash directory deliberately, capped at `TREE_CAP` (2000) nodes. `fs.glob` matches both full path and bare name via `Fnmatch`, sorts by mtime descending (stable), caps at `GLOB_CAP` (100). `fs.grep` shapes output by `output_mode`, with `content` as both the explicit and the fall-through default.

## 3. Interfaces

- 32 `fs.*` MCP tools, one `#[tool]` method per entry in `TOOL_CONTRACT.txt`, each a thin authorize/normalize/call-engine adapter.
- The same engine functions are called by the REST data plane (`api/dataplane.rs`) — never a second implementation.
- Result shapes are part of the interface contract (frozen per tool), independent of and additional to the schema freeze already enforced by `tool-contract-golden.json`.

## 4. Data and state

No new persisted entity. Operates on the platform's `NodeRow`, `blob_refs`, and per-session `SessionState` (read set, write-quota counter, capped audit ring). Transient engine-level shapes:

| Structure | Shape | Home |
|---|---|---|
| **Window** | `{content, total_lines, truncated, next_offset}` | `fs_ops.rs` (read path) |
| **Diff** | unified text, `--- path` / `+++ path` / `@@` hunks | `core/diff.rs` |
| **FileOp** | `{kind: Add\|Update\|Delete, path, add_content, move_to, hunks}` | `fs_ops.rs` (patch parser) |
| **Hunk** | context, removed and added line groups | `fs_ops.rs` (patch parser) |
| **Match** | `{path, line, text, context}` | grep result shape |

## 5. Configuration

No configuration of its own beyond what the platform foundation exposes (`safety.max_read_lines`, `safety.write_quota_bytes`, `safety.allow_hard_delete`). Engine-internal ceilings are hardcoded constants, treated as contract rather than tunables: `MAX_FILES = 5000`, `GLOB_CAP = 100`, `TREE_CAP = 2000`, `FUZZY_THRESHOLD = 0.6`, `ALLOWED_ALGOS = [md5, sha1, sha256, sha512]`, `STAT_UID = STAT_GID = 1000` (verified at `crates/core/src/core/fs_ops.rs:25-45`).

## 6. Observability

Every mutation leaves one (or, for patched updates, two) session audit entries readable through `fs.audit_log`: `{timestamp, op, path, detail}`. No engine-specific tracing/metrics beyond the platform's `tracing` setup. OpenTelemetry remains absent at this layer.

## 7. Decisions

| ID | Decision | Evidence | Rationale |
|---|---|---|---|
| DEC-001 | Scope covers 32 `fs.*` tools; the three document tools are excluded. | `crates/core/src/tools/document.rs` | They depend on the extraction engine, the docx writer and the external document service, none of which belongs to this engine; splitting on dependency rather than name prefix keeps the spec self-contained. |
| DEC-002 | The symbol tools (`fs.find_definition`/`fs.find_references`) are specified here; their matcher is out of scope. | `crates/core/src/core/fs_ops.rs` calls into `docs::symbols` | They are walk-based fs operations whose gates and result shapes belong with the other search tools; the tree-sitter grammars and lexical fallback are a separate document-domain engine. |
| DEC-003 | The duplicate audit entry on patched updates is specified and frozen, not treated as a defect. | `crates/core/src/core/fs_ops.rs` (patch update branch + loop tail) | `fs.audit_log` output is observable contract; silently fixing it would change what an agent sees. |
| DEC-004 | `fs.apply_patch`'s non-transactional behaviour is specified as-built and pinned by test, not corrected. | `crates/core/src/core/fs_ops.rs` (sequential loop, no rollback) | This is a retro-specification; changing the semantics is a product decision, not a drift to silently resolve. |
| DEC-005 | Engine ceilings (5000 files, 100 globs, 2000 tree nodes, 0.6 similarity) are specified as contract, not implementation detail. | `crates/core/src/core/fs_ops.rs:25-45` | They are observable through `truncated` flags and shape what an agent receives; changing one changes the product. |

## 8. Requirement to code map

| Requirement | Code |
|---|---|
| FR-001, FR-002 | `crates/core/src/core/fs_ops.rs` shared write/commit path (`:601-611`) |
| FR-003 | `TOOL_CONTRACT.txt` |
| FR-004..FR-008 | `crates/core/src/core/fs_ops.rs` read family |
| FR-009, FR-010, FR-011 | `crates/core/src/core/fs_ops.rs:601-609` (`fs.write` no-clobber / read-guard / parent creation) |
| FR-012, FR-013 | `crates/core/src/core/fs_ops.rs` append / create_empty |
| FR-014, FR-015 | `crates/core/src/core/fs_ops.rs:1294,1298` (unique-match edit + dry run) |
| FR-016 | `crates/core/src/core/fs_ops.rs` multi_edit |
| FR-017 | `crates/core/src/core/fs_ops.rs:41,1330` (fuzzy threshold + acceptance) |
| FR-018 | `crates/core/src/core/fs_ops.rs` insert_at_line |
| FR-019, FR-020 | `crates/core/src/core/fs_ops.rs` patch parser + apply loop |
| FR-021, FR-022 | `crates/core/src/core/fs_ops.rs:30,39` (list_dir / tree + TREE_CAP) |
| FR-023, FR-024, FR-025 | `crates/core/src/core/fs_ops.rs:25-26,35,313,347-348` (walk, glob, grep) |
| FR-026 | `crates/core/src/core/fs_ops.rs` symbol search, calling `docs::symbols` |
| FR-027, FR-028 | `crates/core/src/core/fs_ops.rs` probes (stat/exists/hash/etc.) |
| FR-029 | `crates/core/src/core/fs_ops.rs:32` (hash allow-list) |
| FR-030, FR-031 | `crates/core/src/core/fs_ops.rs:1082` (delete, hard-delete gate) |
| FR-032, FR-033 | `crates/core/src/core/fs_ops.rs:1076,1180` (directory-deletion gate, move/copy) |
| FR-034 | `crates/core/src/core/fs_ops.rs` session introspection |

### FINDINGS FOR BACKLOG

- **Non-transactional `fs.apply_patch`.** A failure partway through a multi-file patch leaves earlier operations committed with no rollback, unlike `fs.multi_edit`'s all-or-nothing behaviour. Whether to make it transactional is an open product decision, not implemented today.
- **Duplicate MIME guessers.** `fs.read_bytes` uses the engine's own MIME table (`crates/core/src/core/fs_ops.rs`), while the REST download route uses a separate guesser in the document domain. Both currently hold identical extension tables, so no behavior differs, but nothing keeps them in sync; a future edit to one without the other is a silent regression risk. Worth collapsing into one shared table.
- **Unrecognized `fs.grep` `output_mode` falls through to `content` silently** rather than returning `ERR_INVALID_ARGUMENT`. Current behavior is as-built and documented; whether this should instead be a validation error is unresolved.

## 9. Legacy mapping

Source: specs/SPEC-0003_2026-09-18_18-05-00-filesystem-engine/spec.md (pre-move)

| Old id | New id | Note |
|---|---|---|
| SC-201..SC-212 | SC-001..SC-012 | Renumbered in appearance order |
| FR-201..FR-234 | FR-001..FR-034 | Renumbered in appearance order, `FR-NEW-` prefix n/a (none used) |
| E2E-201..E2E-290 | E2E-001..E2E-090 | Renumbered in appearance order |
| DEC-201 | DEC-001 | Scope split: 32 `fs.*` tools vs. document tools |
| DEC-202 | DEC-002 | Symbol tools specified here, matcher out of scope |
| DEC-203 | DEC-003 | Duplicate audit entry frozen as contract |
| DEC-204 | DEC-004 | `fs.apply_patch` non-transactional behaviour specified as-built |
| DEC-205 | DEC-005 | Engine ceilings specified as contract |
| DEC-206 | n/a | ID-block reservation note; not carried forward (single-spec id space now) |
| TBD-201 (apply_patch non-transactional) | FINDINGS FOR BACKLOG | Open product decision, carried to backlog findings |
| TBD-202 (duplicate MIME guessers) | FINDINGS FOR BACKLOG | Confirmed still duplicated at this pass; carried to backlog findings |
| TBD-203 (grep output_mode fallthrough) | FINDINGS FOR BACKLOG | Unresolved; carried to backlog findings |
| §18 Implementability Audit, §19 Drift Register (both empty/resolved) | Legacy mapping row (this section) | No open drift; nothing to carry as a separate register |
