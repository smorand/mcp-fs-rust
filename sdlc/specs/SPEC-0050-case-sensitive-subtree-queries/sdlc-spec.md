# Folder operations mix folders that differ only by letter case: Bug Specification

> Generated on: 2026-10-23
> Id: SPEC-0050
> Nature: BUG
> Depth: M
> Depth evidence: 4 user operations defective (delete, move, copy, recursive reads) plus 2 secondary (trash listing prefix, automatic purge), one storage module with callers in 3 modules, 8 requirements, no public contract change, no data model change.
> Status: designed
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: High
> From backlog: BL-0072
> Security: n/a
> CVSS: n/a
> Affected: main at commit 5a84e33 on the SQLite metadata engine
> Fixed in: unreleased

## 1. Defect

### BUG-001: an operation on a folder also acts on the folders whose name differs only by letter case
- **Observed behavior:** every operation that selects "a folder and everything below it" builds a `LIKE '<folder>/%'` pattern (`crates/core/src/storage/meta.rs:574`). SQLite's `LIKE` ignores ASCII letter case by default and no pragma or collation changes it (`rg -n 'case_sensitive_like|COLLATE' crates/core/src/storage` prints nothing; the only `PRAGMA` lines are `journal_mode`, `busy_timeout` and `foreign_keys`, `crates/core/src/storage/sqlite.rs:57-67`). So `/video` selects the rows of `/Video` as well. Reproduced on 2026-10-23 through the real tool methods over real storage, with `/Video/x.txt` ("upper") and `/video/y.txt` ("lower"):
  - `fs.delete` `/video` recursive, permanent: both files gone, `files left: []`.
  - `fs.delete` `/video` recursive, to the trash: both files moved, `/.mcp_trash/<stamp>__video/x.txt` and `/.mcp_trash/<stamp>__video/y.txt`.
  - `fs.move` `/video` to `/clip`: `/clip/x.txt` and `/clip/y.txt`; `/Video/x.txt` is gone.
  - `fs.copy` `/video` to `/vcopy` recursive: `/vcopy/x.txt` and `/vcopy/y.txt` (the copy holds a file that is not under `/video`).
  By code reading, not run: recursive reads (`fs.glob`, `fs.grep`, the zip export and the web download of a folder) walk the same selection (`crates/core/src/storage/volume.rs:179`, `crates/core/src/core/fs_ops.rs:301`, `crates/core/src/tools/export.rs:158`, `crates/core/src/api/dataplane.rs:745`); `fs.trash_list` with a `path_prefix` filters with the same kind of pattern (`crates/core/src/storage/meta.rs:1742`, `1783`); the automatic purge excludes the trash folder with `NOT LIKE` (`crates/core/src/storage/meta.rs:1996`, called at `crates/core/src/purge.rs:74`).
- **Expected per authority:** paths are case sensitive. (1) SPEC-0049/FR-NEW-009 business rules: "names are case sensitive (`/Video` is `folder '/Video' not found`)". (2) The tool contract describes every `path` as an "Absolute POSIX path within the volume" (`crates/core/src/mcp/server.rs:207`), and POSIX paths are case sensitive. (3) The store itself keeps `/Video` and `/video` as two different rows: the key `(volume_id, path)` is the primary key of `nodes` (`crates/core/src/storage/meta.rs:234`) and is read with `=` elsewhere, which is case sensitive on SQLite, and the e2e test of SPEC-0049 stores both side by side.
- **Reproduction:** preconditions: project `P`, owner `o@x.com`, files `/Video/x.txt` ("upper", 5 bytes) and `/video/y.txt` ("lower", 5 bytes). Input: any of the four operations above on `/video`. Expected: only the files of `/video` are affected. Actual: the files of `/Video` are affected too.
- **Root cause:** `RelationalMetaStore::descendant_pattern` (`crates/core/src/storage/meta.rs:574`) feeds a `LIKE` whose case rule is the engine's. The rows are selected or changed by that `LIKE` alone at `meta.rs:1614` (rename, shared by the move and the soft delete), `1936` (subtree, behind copy, walk, glob, grep, export and download), `2139` and `2164` (permanent removal), `1996` (stale files) and `1742`, `1783` (trash listing). Nothing re checks the exact prefix afterwards. SPEC-0049's folder read had to add that check by hand (`crates/core/src/storage/meta.rs:743`).
- **Contributing factors:** the wildcard escaping tests (`a_b` against `axb`, `50%`) cover the characters, never letter case (`crates/core/src/storage/meta.rs:2826`, `2843`, `crates/core/src/storage/conformance.rs:199`); the engines differ (PostgreSQL's `LIKE` is case sensitive), so the shared conformance suite could not have shown it without a case collision fixture; a project holding two folders that differ only by case is rare, which hid it.
- **Impact:** data loss without a trace for the permanent delete (the other folder's files are removed and cannot be restored); wrong files in the trash, in a move and in a copy; extra files in a search, an export and a download. Frequency: only projects holding two paths that differ only by case. Workaround: none inside the product; avoid such names.
- **Rollback posture:** the corrected operations only touch fewer rows than before. If the fix regresses, deletes, moves and copies of a folder must still affect the folder and everything below it, and a restore from the trash must still work; nothing is left in a format the previous version cannot read.

## 2. Expected Behavior

All requirements restore the authority above; none is a new capability. "Below a folder" means any depth under it. Letter case is compared exactly, for every character, accented ones included.

#### BR-001 [EARS-E]: Permanent delete
> WHEN a folder is deleted permanently THE product SHALL remove that folder and the files and folders below it, and no other path, comparing letter case exactly.

- **Source:** restored from SPEC-0049/FR-NEW-009 (names are case sensitive) and the POSIX path contract (`crates/core/src/mcp/server.rs:207`).
- **Inputs:** `fs.delete` with `path`, `recursive` true, `trash` false.
- **Outputs:** the folder and its content are gone; a folder differing only by case keeps all its files and keeps being readable; the bytes used drop by the removed files only.
- **Business Rules:** the same holds when the deleted path is a single file; a path below the folder is matched as before.
- **Exact names:** `fs.delete`, `recursive`, `trash`.

#### BR-002 [EARS-E]: Delete to the trash
> WHEN a folder is deleted to the trash THE product SHALL move that folder and the files and folders below it, and no other path, and SHALL keep the others where they are.

- **Source:** restored from the same authority.
- **Business Rules:** the trash holds exactly the entries of the deleted folder; the original path recorded for each is its own, with its own letter case.
- **Exact names:** `fs.delete` with `trash` true, the trash folder (`.mcp_trash` by default).

#### BR-003 [EARS-E]: Move and rename
> WHEN a folder is moved or renamed THE product SHALL move that folder and the files and folders below it, and no other path.

- **Source:** restored from the same authority.
- **Exact names:** `fs.move` with `source`, `destination`, `overwrite`.

#### BR-004 [EARS-E]: Copy
> WHEN a folder is copied recursively THE product SHALL copy that folder and the files and folders below it, and no other path.

- **Source:** restored from the same authority.
- **Exact names:** `fs.copy` with `source`, `destination`, `recursive`.

#### BR-005 [EARS-U]: Recursive reads stay inside the folder
> The product SHALL limit a search by name, a search in file content, a zip export and a download of a folder to that folder and the files and folders below it, comparing letter case exactly.

- **Source:** restored from the same authority.
- **Exact names:** `fs.glob` (`root`), `fs.grep` (`root`), `fs.export_zip` (`paths`), the download route of the data plane.

#### BR-006 [EARS-E]: Trash listing by original path
> WHEN the trash is listed with an original path prefix THE product SHALL return only the entries whose original path equals the prefix or lies below it, comparing letter case exactly.

- **Source:** restored from the same authority.
- **Exact names:** `fs.trash_list` with `path_prefix`.

#### BR-007 [EARS-O]: Automatic purge and the trash folder name
> IF a stale file lies under a folder whose name differs from the trash folder name only by letter case THEN THE product SHALL purge that file like any other stale file.

- **Source:** restored from the same authority (the trash folder is the one named in the configuration, letter case included).
- **Business Rules:** the real trash folder stays exempt from the file purge, as today.

#### BR-008 [EARS-UB]: Wildcard characters in names
> The product SHALL NOT widen the set of paths of a folder operation through the characters `%` and `_` in a name.

- **Source:** restored from the existing behavior pinned by `subtree_does_not_match_a_sibling_through_an_underscore` and `remove_subtree_does_not_delete_a_sibling_through_a_percent`; kept, not changed.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| names `/Video` and `/video` both exist | each operation on one leaves the other untouched | BR-001 to BR-005 |
| nested difference `/a/Docs` and `/a/docs` | same, at any depth | BR-001 to BR-004 |
| accented capitals: `/École` and `/école` | letter case compared exactly for non ASCII letters | BR-001 |
| the folder deleted holds a subfolder of the same name in another case | only the exact path is removed | BR-001 |
| `/video` and `/videos` (prefix, not case) | `/videos` untouched, as today | BR-008 |
| a file (not a folder) deleted whose name differs by case from another file | only that file | BR-001 |
| a folder named `.MCP_TRASH` holding a stale file | purged like any other stale file | BR-007 |
| trash entries from `/Video/...` and `/video/...` | a prefix lists its own only | BR-006 |

## 4. Tests

Shared setup unless stated: project `P` owned by `o@x.com`, which can delete permanently; files `/Video/x.txt` containing `upper` and `/video/y.txt` containing `lower`, 10 bytes in all. All tests drive the real tool methods (the assistant interface) over real storage.

### 4.1 End to end reproduction (mandatory)
#### BT-001: a permanent delete leaves the other case alone
- **Reproduces:** BUG-001
- **Driver:** the assistant tool methods `fs.delete`, `fs.read`, `fs.list_dir` and the usage read, through the project's functional e2e harness.
- **Fails today because:** the removal selects `/Video/x.txt` through the case insensitive `LIKE` (`crates/core/src/storage/meta.rs:2139`).
- **Given** the shared setup
- **When** the owner deletes `/video` recursively and permanently
- **Then** listing `/` shows the single folder `/Video`; reading `/Video/x.txt` returns `upper`; reading `/video/y.txt` fails with `ERR_NOT_FOUND`
- **And** the project's bytes used is 5

### 4.2 New behavior
#### BT-002: a delete to the trash moves only the exact folder
- **Validates:** BR-002
- Given the shared setup / When the owner deletes `/video` recursively to the trash / Then `/Video/x.txt` is still at its place and reads `upper`; the trash list has exactly 1 entry, with original path `/video` (kind folder); the trash folder holds `y.txt` and no `x.txt`

#### BT-003: a move takes only the exact folder
- **Validates:** BR-003
- Given the shared setup / When the owner moves `/video` to `/clip` / Then `/clip/y.txt` reads `lower`; `/clip/x.txt` does not exist; `/Video/x.txt` is still at its place and reads `upper`; `/video` no longer exists

#### BT-004: a copy takes only the exact folder
- **Validates:** BR-004
- Given the shared setup / When the owner copies `/video` to `/vcopy` recursively / Then `/vcopy/y.txt` reads `lower`; `/vcopy/x.txt` does not exist; `/Video/x.txt` and `/video/y.txt` are unchanged

#### BT-005: recursive reads stay inside the folder
- **Validates:** BR-005
- Given the shared setup / When the owner runs `fs.glob` with `root` `/video` and pattern `*.txt`, then `fs.grep` with `root` `/video` and pattern `upper`, then `fs.export_zip` with `paths` `["/video"]` and reads the zip names / Then the glob returns `/video/y.txt` only; the grep returns no match; the zip holds `y.txt` only

#### BT-006: the trash listing prefix is exact
- **Validates:** BR-006
- Given the shared setup, both `/Video/x.txt` and `/video/y.txt` deleted to the trash one by one / When the owner lists the trash with `path_prefix` `/Video` and then with `/video` / Then the first list has 1 entry with original path `/Video/x.txt`, the second 1 entry with original path `/video/y.txt`

#### BT-007: a folder named like the trash folder in another case is purged normally
- **Validates:** BR-007
- Given automatic purge on with a retention of 1 day, the trash folder `.mcp_trash`, and the stale file `/.MCP_TRASH/old.txt` (last access 10 days ago) / When the file purge runs / Then `/.MCP_TRASH/old.txt` has been moved to the trash like any stale file, and a stale file already inside `/.mcp_trash` is left where it is

#### BT-008: nested and accented differences
- **Validates:** BR-001, BR-003
- Given `/a/Docs/p.txt` (`P`), `/a/docs/q.txt` (`Q`), `/École/e.txt` and `/école/f.txt` / When the owner deletes `/a/docs` permanently and moves `/école` to `/ecole` / Then `/a/Docs/p.txt` reads `P`; `/École/e.txt` is untouched; `/ecole/f.txt` exists and `/ecole/e.txt` does not

### 4.3 Non regression
#### BT-020: the folder and everything below it still go
- **Protects:** the delete, move and copy of a folder (BR-001, BR-003, BR-004)
- Given `/video/sub/z.txt`, `/video/y.txt` and an unrelated `/other/o.txt` / When the owner deletes `/video` recursively and permanently / Then `/video` and both files below it are gone; `/other/o.txt` is untouched; the same files moved to `/clip` appear under `/clip` with the same sub folder

#### BT-021: a name that only starts like the folder is not touched
- **Protects:** BR-008, the prefix rule
- Given `/video/y.txt` and `/videos/q.txt` / When the owner deletes `/video` recursively / Then `/videos/q.txt` is untouched

#### BT-022: wildcard characters
- **Protects:** BR-008
- Given `/a_b/x.txt`, `/axb/y.txt`, `/50%/p.txt` and `/50x/q.txt` / When the owner deletes `/a_b` and then `/50%` / Then `/axb/y.txt` and `/50x/q.txt` are untouched

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/storage/meta.rs` | `subtree_does_not_match_a_sibling_through_an_underscore` | Kept as is | wildcard rule BR-008 |
| `crates/core/src/storage/meta.rs` | `remove_subtree_does_not_delete_a_sibling_through_a_percent` | Kept as is | BR-008 |
| `crates/core/src/storage/meta.rs` | `subtree_includes_root_and_descendants`, `remove_subtree_gcs_all_orphaned_blobs`, `remove_subtree_keeps_blob_shared_outside`, `remove_subtree_is_scoped_to_its_volume` | Kept as is | none assert letter case folding; `rg -n '"/[A-Z]' crates/core/src/storage/meta.rs crates/core/src/storage/conformance.rs` prints nothing |
| `crates/core/src/storage/conformance.rs` | the `/a_b` subtree check (line 199) | Kept as is | runs on every engine; a case collision case is added by the design |
| `crates/core/src/tools/folder_usage_e2e.rs` | `spec_0049_us_0073_folders_differing_only_by_case_do_not_mix` | Kept as is | passes today because of the manual prefix check; must pass after the fix |

## 5. Existing Specs Affected
- SPEC-0049 (implemented): FR-NEW-009 "names are case sensitive" is the authority restored here; its manual exact prefix check in the folder read (`crates/core/src/storage/meta.rs:743`) becomes redundant but stays valid. SPEC-0049 is not modified.
- SPEC-0047, SPEC-0020: unchanged; their reads use `kind='file'` queries without `LIKE` for the file list, and the usage counter follows the removal (BT-001 asserts it).
- SPEC-0011 (trash): `fs.trash_list` `path_prefix` semantics restored (BR-006).
- SPEC-0014 (auto purge): the trash folder exemption restored to exact name (BR-007).

## 6. Assumptions
- **Assumption:** PostgreSQL compares `LIKE` case sensitively and is therefore not defective (PostgreSQL documentation, not tested here; ASSUMED:). It shapes BR-001 to BR-007 only in that the tests run on the default SQLite engine.
- **Assumption:** SQL Server, with a default case insensitive collation, may not hold two paths differing only by case at all (the key comparison itself would be case insensitive); not verified here (ASSUMED:). Recorded as a backlog entry; the requirements above are stated for engines that hold both paths.
- **Assumption:** the stale purge case (BR-007) was found by reading the query, not run; BT-007 is its first run.
- **Backlog entries created by this run:** BL-0073 (unverified: path comparison on SQL Server).
- **Backlog entry consumed:** BL-0072.

## 7. Implementability Checklist

| # | Check | Result |
|---|---|---|
| 1 | No technical decision | PASS (the root cause `file:LINE` is a fact; no fix design) |
| 2 | No buried requirement | PASS (one BR per operation) |
| 3 | Every observable name spelled | PASS (tool names, arguments, `.mcp_trash`) |
| 4 | No forced choice | PASS (case compared exactly for all characters; the trash exemption stays for the exact name) |
| 5 | Every test specified; existing tests listed | PASS (4.4) |
| 6 | Reproduction is end to end | PASS (BT-001 reads files and the usage back through the tools) |
| 7ante | Security misclassification | PASS: no authentication or authorization bypass, injection or traversal; other files of the same project are affected, reachable by the same members; `Security: n/a` holds. Integrity defect (unintended deletion) rated High |
| 8 | No hidden prerequisite | PASS (the owner can delete permanently: stated in the shared setup) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (behavior changes, no new capability; depth M for 8 requirements and 4 operations) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS (reproduction run on 2026-10-23, call sites cited; read only claims labeled) |
| 13 | Round trip read | PASS: writer `put_file`/`rename` store the path as given (`crates/core/src/storage/meta.rs:2008`, `1624-1626`), reader is the same `LIKE`; the engines are two (SQLite run, PostgreSQL ASSUMED) and BT-001..006 run on the default engine; the conformance suite runs the same checks on every engine configured for tests |
| 14 | Verdict consistent | PASS |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: BL-0073 |
