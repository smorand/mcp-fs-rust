# Search does not follow the trash: deleted files come back, restored files vanish: Bug Specification

> Generated on: 2026-10-13
> Id: SPEC-0025
> Nature: BUG
> Depth: L
> Depth evidence: 3 modules (search indexer, auto purge, trash restore), 6 requirements, no public contract change, no data model change
> Status: specified
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: Medium
> From backlog: BL-0050
> Security: n/a
> CVSS: n/a
> Affected: every deployment with search enabled, since SPEC-0008 (mode change backfill) and SPEC-0013 (trash restore)
> Fixed in: unreleased

`Security: n/a` is deliberate. The content that resurfaces is only ever shown to members of the same
project, who can already list and read the trash (`fs.trash_list`, `fs.read_bytes` on the trash path).
No data reaches a principal who could not read it before; this is an index correctness defect.

## 1. Defect

### BUG-001: the search index disagrees with the trash
- **Observed behavior:**
  - A soft delete moves the file to `/{trash_dir}/{epoch_ms}__{flattened path}`, `trash_dir` defaulting
    to `.mcp_trash` (`crates/core/src/safety.rs:221-225`, `crates/core/src/config.rs:362-364`).
  - `fs.delete` then clears the path's chunks (`indexer::after_delete_many`, called at
    `crates/core/src/mcp/server.rs:1999`, `crates/core/src/tools/lifecycle.rs:70`,
    `crates/core/src/api/dataplane.rs:459`).
  - Four other paths do not follow the trash:
    1. **Mode change backfill.** `admin.set_index_mode` starts `full_index`
       (`crates/core/src/search/indexer.rs:84`), which walks the whole volume with no exclusion,
       `fs_ops::iter_files(client, "/", &[])` (`indexer.rs:136`), and indexes every trash file under its
       trash path.
    2. **Recursive `search.index`.** It walks with the same empty exclusion list
       (`crates/core/src/mcp/server.rs:2845-2850`), so `search.index {path: "/", recursive: true}`
       indexes the trash too.
    3. **Move into the trash folder.** `fs.move` into the trash folder reindexes the destination
       (`indexer.rs:327-356`, `index_tree` at `:355`).
    4. **Auto purge sweep.** It soft deletes stale files with a rename into the trash
       (`crates/core/src/purge.rs:51-95`, rename at `:83`) and calls no search hook (no `indexer` call
       anywhere in `purge.rs`), so the old path's chunks stay.
  - `fs.trash_restore` renames the file back (`crates/core/src/tools/trash.rs:177`) and calls no search
    hook (no `indexer` reference in `tools/trash.rs`), so a restored file stays out of search until it
    is written again. The same function serves `/app/trash` (`crates/core/src/trash_screen.rs`) and
    `mcp/server.rs:2140-2146`.
- **Expected per authority:** SPEC-0008/SC-004: "A delete removes the path's chunks the same way" and
  "**Postconditions:** the index converges on the volume's content without any caller waiting for it"
  (`sdlc/specs/SPEC-0008-search-and-rag/spec.md:115-124`). SPEC-0013 defines the trash as soft deleted
  content that `fs.trash_restore` puts back at its original path
  (`sdlc/specs/SPEC-0013-trash-listing-and-recovery/spec.md:66`, FR-NEW-009 `:93`).
- **Reproduction:** project `atlas`, index mode `bm25`, `/memo.txt` = `quarterly forecast draft`.
  - **Resurrection:** delete `/memo.txt`; `search.query {query: "forecast"}` returns nothing (correct).
    Call `admin.set_index_mode {mode: "rag"}` and wait for the backfill. Expected: still nothing.
    Actual: a result whose `path` starts with `/.mcp_trash/` and whose `chunk` contains
    `quarterly forecast draft`; `search.status` counts 2 documents for 1 live file
    (BL-0050 evidence, 5 runs of 5 on 4 backends).
  - **Restore:** restore it with `fs.trash_restore`. Expected: `search.query` finds `/memo.txt`.
    Actual: nothing.
- **Root cause:**
  - `crates/core/src/search/indexer.rs:136` and `crates/core/src/mcp/server.rs:2846` pass an empty
    exclusion list for the trash folder.
  - `crates/core/src/search/indexer.rs:355` indexes any move destination, the trash folder included.
  - `crates/core/src/purge.rs:83` and `crates/core/src/tools/trash.rs:177` move files without the
    delete and write hooks that `fs.delete` and `fs.move` use.
- **Contributing factors:** the trash is an ordinary directory of the volume
  (`safety.rs:221-225`), so every walker sees it unless told otherwise. The SPEC-0008 tests seed the
  mode before writing.
- **Impact:** members of a project whose index mode changes see deleted content come back under trash
  paths, and the stats overcount. After the auto purge, search returns paths that no longer exist.
  After a restore, the file is missing from search. Workaround: `search.delete` on trash paths, or
  rewrite restored files.
- **Rollback posture:** reverting returns to today's behavior; the index stays rebuildable from the
  volume (`admin.set_index_mode` to `none` then back).

## 1bis. Occurrence sweep
Not a security sweep (`Security: n/a`), but the same mistake was looked for at every place a file
enters or leaves the trash, or a walk covers the volume:
- `rg -n 'iter_files\(' crates/core/src`: index walkers at `indexer.rs:136` and `server.rs:2846`. The
  other callers (grep, glob, export) are outside search and out of scope.
- `rg -n 'trash_destination|rename_with_collision_retry' crates/core/src`: moves into the trash at
  `fs_ops.rs:1239` (`fs.delete`, hooked) and `purge.rs:83` (not hooked).
- `rg -n 'indexer::' crates/core/src`: hooked call sites. `tools/trash.rs` and `purge.rs` have none.

| Occurrence | Verdict | Requirement |
|---|---|---|
| `indexer.rs:136` mode change backfill | defective | BR-001 |
| `server.rs:2846` recursive `search.index` | defective | BR-002 |
| `indexer.rs:355` move destination inside the trash | defective | BR-003 |
| `purge.rs:83` auto purge soft delete | defective | BR-004 |
| `tools/trash.rs:177` restore | defective | BR-005 |
| `fs_ops.rs:1239` `fs.delete` | correct: hooked at `server.rs:1999`, `lifecycle.rs:70`, `dataplane.rs:459` | BT-020 |

## 2. Expected Behavior

"The trash" means every path under `/{trash_dir}` (default `/.mcp_trash`). All requirements hold for
every search mode (`bm25`, `rag`, `both`) and backend.

#### BR-001 [EARS-E]: A mode change backfill skips the trash
> WHEN an index mode change backfills a project THE Search context SHALL index every live file and no
> file under the trash.
- **Source:** restored from SPEC-0008/SC-004 ("A delete removes the path's chunks"; "the index
  converges on the volume's content").
- **Exact names:** `admin.set_index_mode`, `search.status` fields `bm25_docs`, `vector_chunks`.

#### BR-002 [EARS-E]: A recursive index skips the trash
> WHEN `search.index` is called with `recursive: true` on a path that contains the trash THE Search
> context SHALL index no file under the trash, and SHALL report only the files it indexed.
- **Source:** restored from SPEC-0008/SC-004; decided here (ASSUMED): `search.index` without
  `recursive` on one explicit trash file still indexes it, because the caller named that file.
- **Exact names:** `search.index` params `path`, `recursive`; response fields as today.

#### BR-003 [EARS-E]: A move into the trash folder removes from search
> WHEN `fs.move` moves a file into the trash THE Search context SHALL remove the source's chunks and
> SHALL NOT index the destination.
- **Source:** restored from SPEC-0008/SC-004.

#### BR-004 [EARS-E]: The auto purge removes from search
> WHEN the auto purge soft deletes a stale file THE Search context SHALL remove that file's chunks.
- **Source:** restored from SPEC-0008/SC-004 ("A delete removes the path's chunks").

#### BR-005 [EARS-E]: A restored file is searchable again
> WHEN `fs.trash_restore` puts a file back THE Search context SHALL index it at the path it was
> restored to, including a `_restored` collision path.
- **Source:** restored from SPEC-0008/SC-004 ("converges on the volume's content") and SPEC-0013/FR-NEW-009.
- **Exact names:** `fs.trash_restore` response `{"restored_path": "...", "trash_path": "..."}`.

#### BR-006 [EARS-UB]: Search never answers with a trash path
> The Search context SHALL NOT return from `search.query` a result whose `path` is under the trash,
> unless that file was indexed explicitly as in BR-002's assumption.
- **Source:** restored from SPEC-0008/SC-004.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| custom `safety.trash_dir` (for example `.bin`) | that folder is skipped, `.mcp_trash` is an ordinary folder | BR-001 to BR-004 |
| a live folder named like the trash but nested (`/docs/.mcp_trash/a.txt`) | indexed: only the top level folder is the trash | BR-001, BR-002 |
| index mode `none` | no hook indexes or removes anything, as today | BR-004, BR-005 |
| restore onto a taken path | the `_restored` path is indexed, the existing file keeps its chunks | BR-005 |
| an index built before the fix already holds trash entries | removed by the next mode change, which wipes first (`indexer.rs:68`) | BR-001 |

## 4. Tests

Each test runs with BM25 on Tantivy in `make check`. The same test runs on PostgreSQL BM25 and on
sqlite-vec with stub embeddings when `MCPFS_TEST_PG_DSN` is set, and on the stub embeddings otherwise.

### 4.1 End to end reproduction (mandatory)
#### BT-001: A deleted file does not come back after a mode change
- **Reproduces:** BUG-001
- **Driver:** tool calls `fs.write_bytes`, `fs.delete`, `admin.set_index_mode`, `search.query`, `search.status`.
- **Fails today because:** `indexer.rs:136` walks the trash.
- Given `atlas` in mode `bm25` with `/memo.txt` = `quarterly forecast draft` and `/plan.txt` = `merger with Borealis`
- When `/memo.txt` is deleted, then `admin.set_index_mode {project_id: "atlas", mode: "both"}` runs and its backfill completes
- Then `search.query {mount_id: "atlas", query: "forecast"}` returns `"results": []`
- And `search.status` reports `bm25_docs: 1`

#### BT-002: A restored file is found again
- **Reproduces:** BUG-001 (restore side)
- **Fails today because:** `tools/trash.rs:177` calls no index hook.
- Given `atlas` in mode `bm25`, `/memo.txt` deleted
- When `fs.trash_restore {mount_id: "atlas", trash_path: <its trash_path from fs.trash_list>}` / Then `search.query {query: "forecast"}` returns one result with `path: "/memo.txt"`.

### 4.2 New behavior
#### BT-010: Recursive index of the root skips the trash
- **Validates:** BR-002
- Given `/memo.txt` deleted and `/plan.txt` live / When `search.index {path: "/", recursive: true}` / Then `search.query {query: "forecast"}` returns `"results": []`, `search.query {query: "Borealis"}` returns `/plan.txt`.

#### BT-011: Explicit index of one trash file still works
- **Validates:** BR-002 assumption, BR-006 exception
- When `search.index {path: <trash path of /memo.txt>}` / Then `search.query {query: "forecast"}` returns that trash path.

#### BT-012: Move into the trash folder
- **Validates:** BR-003
- When `fs.move {source: "/plan.txt", destination: "/.mcp_trash/manual.txt"}` / Then `search.query {query: "Borealis"}` returns `"results": []`.

#### BT-013: Auto purge removes the stale file from search
- **Validates:** BR-004
- Given `atlas` with auto purge on, `file_retention_days: 1`, `/old.txt` = `obsolete roadmap` last accessed 2 days ago
- When the purge cycle runs / Then `search.query {query: "roadmap"}` returns `"results": []`, and `fs.trash_list` lists `/old.txt`.

#### BT-014: Restore onto a taken path
- **Validates:** BR-005
- Given `/memo.txt` deleted, then a new `/memo.txt` = `fresh notes` written
- When the deleted one is restored / Then `search.query {query: "forecast"}` returns `/memo.txt_restored`, and `search.query {query: "fresh"}` still returns `/memo.txt`.

#### BT-015: Custom trash folder
- **Validates:** BR-001, edge case
- Given `safety.trash_dir: ".bin"` and `/memo.txt` deleted into `/.bin/` / When the mode changes / Then `"results": []` for `forecast`.

#### BT-016: Nested folder named like the trash is indexed
- **Validates:** edge case
- Given `/docs/.mcp_trash/a.txt` = `nested keep` (a live file) / When the mode changes / Then `search.query {query: "nested"}` returns `/docs/.mcp_trash/a.txt`.

### 4.3 Non regression
#### BT-020: Delete still clears, write still indexes
- **Protects:** SPEC-0008/SC-004 hooks
- `fs.delete` of an indexed file empties its results; `fs.write_bytes` makes a new file findable; existing indexer tests stay green.

#### BT-021: Trash listing and restore unchanged
- **Protects:** SPEC-0013
- `fs.trash_list` and `fs.trash_restore` return exactly what they return today for the BT-002 data.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/search/indexer.rs:532` | `on_mode_change_to_an_active_mode_wipes_then_backfills` | Kept | no trash in its data |
| `crates/core/src/search/indexer.rs:572` | `full_index` test | Kept | same |
| `crates/core/src/tools/trash.rs` (test module) | restore tests | Kept | response unchanged |
| `crates/core/src/purge.rs` (test module) | sweep tests | Kept | trash rows unchanged |
| `crates/core/src/tools/project_removal_e2e.rs:278` | seed sets the mode before writing | Kept | still valid |

No existing test asserts that trash files are indexed.

## 5. Existing Specs Affected
None deviated. Restores SPEC-0008/SC-004 for the trash introduced by SPEC-0013 and the auto purge
of SPEC-0017.

## 6. Assumptions
- **Assumption:** an explicit, non recursive `search.index` of a trash path keeps working (BR-002,
  BT-011): the caller named it.
- **Assumption:** only the top level `/{trash_dir}` folder is the trash, as `safety.rs:221-225` builds it.
- **Overshoot note:** classified L on module count; the requirement count is S sized.
- **Backlog entries created by this run:** none.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS |
| 4 | No forced choice | PASS (explicit index assumption stated) |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001, BT-002 tool calls) |
| 7ante | Security misclassification | PASS (no new reader: members already read the trash) |
| 8 | No hidden prerequisite | PASS (mode and purge config in Given) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS |
| 11 | Length | PASS |
| 12 | Claims cited | PASS |
| 13 | Writer and reader read | PASS (trash writer `safety.rs:221-225`; readers `indexer.rs:136`, `server.rs:2846`) |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
