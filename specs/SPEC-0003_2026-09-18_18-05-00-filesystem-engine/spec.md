# mcp-fs Filesystem Engine and `fs.*` Tools — Specification Document

> Generated on: 2026-09-18
> Project: mcp-fs (Rust)
> Version: 1.0
> Status: Draft
> Type: Evolution Specification (retro-specification of shipped behaviour)

**ID convention.** This is S2 of eight sequenced specifications. To keep identifiers unique across the corpus, S2 owns the `2xx` block: `SC-2xx`, `FR-2xx`, `E2E-2xx`, `DEC-2xx`, `EXC-2xx`. S1 owns `0xx`.

## 1. Executive Summary

This document specifies the **filesystem engine** (`core/fs_ops.rs`) and the 32 `fs.*` tools that expose it. This is the product's reason to exist: an LLM-facing simulated filesystem with read paging, precise editing, search, and safe mutation semantics.

The engine is the single implementation of every filesystem operation. The MCP tool layer and the REST data plane (S3) are both thin adapters over it. That is a structural rule, not a convention: a fix applied in one adapter and not the other would silently leave the second surface on the old behaviour, so neither adapter is permitted to implement an operation.

This is a **retro-specification** of shipped behaviour; every claim about the code carries a `file:LINE` citation. It builds on S1 (`2026-09-18_17-37-46-platform-foundation.md`), which fixes authorization, path normalization, the safety contract, the error vocabulary and the MCP protocol framing. S2 does not restate those; it consumes them.

## 2. Current State Analysis

### 2.1 Project Overview

`core/fs_ops.rs` is 3355 lines holding every filesystem operation, plus `core/diff.rs` (212 lines) for unified diffs and `util/text.rs` (188 lines) for line splitting and glob matching. The tool layer is eight modules under `tools/`, each registering one family and doing exactly three things per handler: authorize, normalize, call the engine.

### 2.2 Existing Specifications

- **S1, `2026-09-18_17-37-46-platform-foundation.md`**: config and boot, the 14 error codes, identity, the MCP protocol layer, the storage seam and content addressing, the project ACL, the safety contract, and the 10 `admin.*` tools. S2 depends on S1 FR-020 (error vocabulary), FR-021 (membership gate), FR-032 (path normalization), FR-034 (read guard), FR-035 (write quota), FR-036 (audit), FR-037 (trash path), FR-038..FR-040 (content addressing).

### 2.3 Relevant Architecture

- **Engine**: `crates/mcp-fs/src/core/fs_ops.rs`, called by every tool handler and by `api/dataplane.rs`.
- **Tool modules**: `tools/read.rs`, `write.rs`, `edit.rs`, `listing.rs`, `metadata.rs`, `lifecycle.rs`, `search.rs`.
- **Shared write path**: `fs_ops::commit` (`core/fs_ops.rs:556-570`) charges the quota, writes atomically and records the audit entry. Every text mutation goes through it.
- **Frozen schemas**: `TOOL_CONTRACT.txt:41-200` and `tool-contract-golden.json`.

## 3. Scope

### 3.1 In Scope

The 32 `fs.*` tools and the engine behind them:

| Family | Tools |
|---|---|
| Read | `fs.read`, `fs.read_bytes`, `fs.read_lines`, `fs.read_section`, `fs.read_many`, `fs.head`, `fs.tail`, `fs.count_lines` |
| Write | `fs.write`, `fs.append`, `fs.create_empty`, `fs.write_bytes` |
| Edit | `fs.edit`, `fs.multi_edit`, `fs.search_replace`, `fs.insert_at_line`, `fs.apply_patch` |
| Listing | `fs.list_dir`, `fs.tree` |
| Metadata | `fs.stat`, `fs.exists`, `fs.hash` |
| Lifecycle | `fs.mkdir`, `fs.delete`, `fs.move`, `fs.copy`, `fs.list_allowed_roots`, `fs.audit_log` |
| Search | `fs.glob`, `fs.grep`, `fs.find_definition`, `fs.find_references` |

Plus the engine mechanics: line windowing and truncation, the unified diff, unique-match editing, fuzzy block replacement, the V4A patch format, the volume walk and its caps, glob and grep semantics.

### 3.2 Out of Scope (Non-Goals)

- `fs.extract_text`, `fs.write_docx` and `fs.documentize`, the three document tools of the `fs.*` family (S5). They are registered in `tools/document.rs` and bring the family to 35.
- The **symbol extraction engine** behind `fs.find_definition` and `fs.find_references`: tree-sitter grammars and the lexical fallback in `docs/symbols.rs` belong to S5. S2 specifies the two tools' gates, inputs and output shapes, and treats the matcher as a dependency.
- The `trigger_documentation_service` parameter of `fs.write_bytes` (S5). S2 specifies `fs.write_bytes` with that flag false; `write_bytes_documented` (`core/fs_ops.rs:619`) is S5's.
- The REST data plane, which is the second adapter over this same engine (S3).
- Everything S1 owns: authorization, normalization, quota, audit, trash derivation, error codes, protocol framing.
- Search indexing side effects of writes (S7).

## 4. User Personas & Actors

| Actor | Description |
|---|---|
| **LLM agent** | The primary consumer. Reads with paging, edits by unique string match, searches by glob and regex. Optimizes for token economy, so truncation and caps are part of the contract, not an implementation detail. |
| **Project member** | The authenticated human whose identity the agent acts under. Owns the session whose read set, write quota and audit log govern the mutations. |
| **Tool author** | A developer adding or changing a tool. Bound by the rule that operations live in the engine only. |

## 4.5 Bounded Contexts

| Context | Scope | Key entities |
|---|---|---|
| **Volume** (from S1) | The metadata tree and content-addressed bytes being read and mutated. | NodeRow, blob, path |
| **Session** (from S1) | Read set, write quota, audit log governing this caller's mutations. | read set, bytes_written, AuditEntry |
| **Engine** | The operations themselves and their result shapes. | window, diff, hunk, match |
| **Patch** | The V4A multi-file patch dialect: its markers, hunks and file operations. | FileOp, Hunk, OpKind |

A "match" in the Engine context is a grep hit or an edit site; in the Patch context the corresponding notion is a hunk anchor. A "path" everywhere is the normalized absolute form produced by S1 FR-032.

## 5. Usage Scenarios

### SC-201: Agent reads a large file in pages

**Actor:** LLM agent
**Preconditions:** the caller is a member of the project; the file exists and is text.
**Flow:**
1. Agent calls `fs.read` with `mount_id` and `path`, default `offset_lines` 0 and `limit_lines` 2000.
2. The engine reads the text, records the read against the session, and splits it into lines (`fs_ops.rs:61-64`).
3. The window is capped by the lesser of `limit_lines` and `safety.max_read_lines` (`fs_ops.rs:65`).
4. The content is returned line-numbered by default, with `total_lines`, `truncated` and `next_offset` (`fs_ops.rs:70-75`).
5. Agent repeats with `offset_lines` set to `next_offset` until `truncated` is false.

**Postconditions:** the session has recorded a read for that path, which unlocks later edits under the S1 read guard; the agent holds the full content across pages.
**Exceptions:**
- EXC-201a: path missing → `ERR_NOT_FOUND`
- EXC-201b: path is a directory → the storage layer's error surfaces
- EXC-201c: `limit_lines` above `safety.max_read_lines` → silently capped, not an error (`fs_ops.rs:65`)
- EXC-201d: `offset_lines` past the end → empty content, `truncated` false

**Cross-scenario notes:** step 2's read record is the precondition for SC-203, SC-204 and SC-205.

### SC-202: Agent creates a file and appends to it

**Actor:** LLM agent
**Preconditions:** the caller is a member; the target path does not exist.
**Flow:**
1. Agent calls `fs.write` with `content`, default `overwrite` false and `create_parents` true.
2. The engine refuses if the path exists and `overwrite` is false (`fs_ops.rs:596-598`).
3. Parent directories are created when requested (`fs_ops.rs:601-603`).
4. The bytes are charged against the quota, written atomically, and audited (`fs_ops.rs:565-568`).
5. Agent calls `fs.append` to add more content.

**Postconditions:** the file exists with exactly the written content; the session's `bytes_written` has advanced; `fs.audit_log` shows the operation.
**Exceptions:**
- EXC-202a: path exists and `overwrite` false → `ERR_NO_CLOBBER`, `"'{path}' exists (pass overwrite=true)"` (`fs_ops.rs:597`)
- EXC-202b: overwriting a file never read in this session → `ERR_EDIT_WITHOUT_PRIOR_READ` (`fs_ops.rs:600`)
- EXC-202c: the write would exceed the session quota → `ERR_WRITE_QUOTA_EXCEEDED`
- EXC-202d: `create_parents` false and the parent is missing → the storage layer refuses
- EXC-202e: `fs.append` on a missing file with `create` false → `ERR_NOT_FOUND`

### SC-203: Agent edits a file by unique string match

**Actor:** LLM agent
**Preconditions:** the file has been read in this session.
**Flow:**
1. Agent calls `fs.edit` with `old_string` and `new_string`, optionally `dry_run` true.
2. The engine enforces the read guard, then reads the current text (`fs_ops.rs:856-857`).
3. Occurrences are counted; exactly one is required unless `replace_all` is set (`fs_ops.rs:1181-1195`).
4. A unified diff of old against new is produced (`fs_ops.rs:858`).
5. Unless `dry_run`, the result is committed through the shared write path (`fs_ops.rs:859-861`).
6. The tool returns `path`, `applied` and `diff`.

**Postconditions:** with `dry_run` false the file holds the new text and `applied` is true; with `dry_run` true the file is untouched, `applied` is false, and the diff still shows what would change.
**Exceptions:**
- EXC-203a: `old_string` absent → `ERR_NO_MATCH`, `"old_string not found in '{path}'"` (`fs_ops.rs:1188`)
- EXC-203b: several occurrences without `replace_all` → `ERR_AMBIGUOUS_MATCH`, `"old_string matches {count} sites in '{path}' (use replace_all)"` (`fs_ops.rs:1191-1193`)
- EXC-203c: the file was not read in this session → `ERR_EDIT_WITHOUT_PRIOR_READ`
- EXC-203d: the new content exceeds the quota → `ERR_WRITE_QUOTA_EXCEEDED`, and nothing is written

### SC-204: Agent applies several edits atomically

**Actor:** LLM agent
**Preconditions:** the file has been read in this session.
**Flow:**
1. Agent calls `fs.multi_edit` with an `edits` array of `{old_string, new_string, replace_all?}`.
2. Each edit is applied in order against the text produced by the previous one (`fs_ops.rs:882-892`).
3. A single diff of the original against the final text is produced.
4. Unless `dry_run`, the final text is committed once.

**Postconditions:** either every edit applied and the file holds the final text, or nothing was written at all.
**Exceptions:**
- EXC-204a: any edit fails to match → the whole call fails and the file is unchanged (`fs_ops.rs:882-892`)
- EXC-204b: an edit that would have matched the original but not the intermediate text fails, because edits compose in order
- EXC-204c: an empty `edits` array produces an empty diff and `edits` 0

### SC-205: Agent replaces a multi-line block, optionally fuzzily

**Actor:** LLM agent
**Preconditions:** the file has been read in this session.
**Flow:**
1. Agent calls `fs.search_replace` with `search_block` and `replace_block`.
2. With `fuzzy` false the block must match exactly and uniquely.
3. With `fuzzy` true a window of the search block's line count slides over the file and the most similar candidate is kept (`fs_ops.rs:1205-1236`).
4. The candidate is accepted only at a similarity of at least 0.6 (`fs_ops.rs:41,1234`).
5. The replacement is committed and the diff returned.

**Postconditions:** the block is replaced; the replacement is newline-terminated whether or not the caller terminated it (`fs_ops.rs:1229-1233`).
**Exceptions:**
- EXC-205a: no candidate reaches the threshold → `ERR_NO_MATCH`, `"no fuzzy match for search_block in '{path}'"` (`fs_ops.rs:1235`)
- EXC-205b: exact mode with no match → `ERR_NO_MATCH`
- EXC-205c: exact mode with several matches → `ERR_AMBIGUOUS_MATCH`

### SC-206: Agent applies a multi-file V4A patch

**Actor:** LLM agent
**Preconditions:** the caller is a member; files targeted by update or delete operations have been read in this session.
**Flow:**
1. Agent calls `fs.apply_patch` with `patch_text` bracketed by `*** Begin Patch` and `*** End Patch`.
2. The patch is parsed into file operations: `*** Add File: `, `*** Update File: `, `*** Delete File: `, with an optional `*** Move to: ` on an update (`fs_ops.rs:1552-1558`).
3. Each operation normalizes its own path through the safety layer (`fs_ops.rs:1514`).
4. Adds charge the quota and write; updates enforce the read guard, apply hunks, charge and write; deletes enforce the read guard and remove (`fs_ops.rs:1516-1545`).
5. The tool returns `files`, one entry per touched path.

**Postconditions:** every operation in the patch has been applied in order; an update carrying a move leaves the content at the new path.
**Exceptions:**
- EXC-206a: malformed patch text → the parse fails before anything is written
- EXC-206b: an update targeting a file not read in this session → `ERR_EDIT_WITHOUT_PRIOR_READ`, after earlier operations in the same patch have already been applied
- EXC-206c: a hunk that does not locate its context → the update fails
- EXC-206d: the quota is exhausted partway → earlier operations stand

### SC-207: Agent explores the volume structure

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.list_dir`, defaulting to `/`, optionally with sizes and a sort key.
2. Agent calls `fs.tree` for a recursive view to `max_depth` 3 by default.
3. Agent calls `fs.stat`, `fs.exists`, `fs.count_lines` and `fs.hash` on individual paths.

**Postconditions:** the agent holds the directory structure; none of these operations mutates anything or records a read.
**Exceptions:**
- EXC-207a: `fs.tree` exceeding 2000 nodes → the walk stops and `truncated` is reported (`fs_ops.rs:39`)
- EXC-207b: `fs.hash` with an unsupported algorithm → `ERR_INVALID_ARGUMENT`; only `md5`, `sha1`, `sha256` and `sha512` are allowed (`fs_ops.rs:32`)
- EXC-207c: `fs.exists` on a missing path → a successful result reporting absence, not an error

### SC-208: Agent searches the volume by name and by content

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.glob` with a pattern, optionally excluding more globs.
2. The walk skips the default excluded directories and matches against both the full path and the bare file name (`fs_ops.rs:325-334`).
3. Results are sorted newest first and capped at 100, with `truncated` reported (`fs_ops.rs:338-344`).
4. Agent calls `fs.grep` with `output_mode` `content`, `files` or `count`.

**Postconditions:** the agent holds the matching paths or matching lines; no read is recorded, so a subsequent edit still needs an explicit read.
**Exceptions:**
- EXC-208a: more than 100 glob matches → the list is capped and `truncated` is true
- EXC-208b: a walk exceeding 5000 files → the walk stops at the ceiling (`fs_ops.rs:35`)
- EXC-208c: an invalid regex with `regex` true → `ERR_INVALID_ARGUMENT`

### SC-209: Agent locates a symbol

**Actor:** LLM agent
**Preconditions:** the caller is a member; the volume holds source files.
**Flow:**
1. Agent calls `fs.find_definition` with a symbol `name`, optionally a `root` and a `kind` filter.
2. The engine walks the volume and runs the language-aware matcher on every file with a known language (`fs_ops.rs:1374-1396`).
3. Agent calls `fs.find_references` for call sites.

**Postconditions:** the agent holds `definitions` with `path`, `name`, `kind` and `line`, or `references` with `path`, `line` and `kind`.
**Exceptions:**
- EXC-209a: no match → an empty array, not an error
- EXC-209b: a file in an unsupported language → skipped, not an error

### SC-210: Agent moves, copies and deletes

**Actor:** LLM agent
**Preconditions:** the caller is a member; the source exists.
**Flow:**
1. Agent calls `fs.copy`, no-clobber by default, `recursive` for trees.
2. Agent calls `fs.move` to rename or relocate, no-clobber by default.
3. Agent calls `fs.delete`, which moves to trash by default.
4. The trash destination is derived by the S1 safety layer and its parent is created before the rename (`fs_ops.rs:1007-1011`).

**Postconditions:** a trashed file is still present under the trash directory and is reported in `trash_path`; a copy shares its content blob with the source by refcount rather than duplicating bytes.
**Exceptions:**
- EXC-210a: deleting a directory without `recursive` → `ERR_INVALID_ARGUMENT`, `"'{path}' is a directory (pass recursive=true)"` (`fs_ops.rs:990-992`)
- EXC-210b: `trash` false while `allow_hard_delete` is false → `ERR_NOT_SUPPORTED`, `"hard delete disabled (server started without allow_hard_delete)"` (`fs_ops.rs:996-999`)
- EXC-210c: deleting a missing path → `ERR_NOT_FOUND`, `"'{path}' does not exist"` (`fs_ops.rs:985`)
- EXC-210d: destination exists without `overwrite` → `ERR_NO_CLOBBER`

### SC-211: Agent reads binary content and inspects the session

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.read_bytes` for a byte range, default 65536 bytes from offset 0.
2. The engine returns base64 plus a guessed MIME type and the actual length (`fs_ops.rs:90-94`).
3. Agent calls `fs.list_allowed_roots` and `fs.audit_log` to inspect its own session.

**Postconditions:** the binary payload is available base64-encoded; a read is recorded for the path, exactly as for a text read (`fs_ops.rs:89`).
**Exceptions:**
- EXC-211a: a negative offset or length → clamped to zero rather than rejected (`fs_ops.rs:88`)
- EXC-211b: an unknown extension → `application/octet-stream` (`fs_ops.rs:92`)
- EXC-211c: `fs.audit_log` in a fresh session → an empty entry list

### SC-212: Agent batch-reads several files with per-file error isolation

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.read_many` with a `paths` array and `per_file_cap_lines`, default 500.
2. Each path is normalized and read independently; a failure is recorded against that entry and the loop continues (`fs_ops.rs:157-173`).
3. Successful entries record a read and carry line-numbered content and a `truncated` flag.

**Postconditions:** one entry per requested path, in request order; the successes are usable even when some entries failed.
**Exceptions:**
- EXC-212a: a path failing normalization → the entry keeps the **raw** spelling and carries the rendered `ERR_*: message` (`fs_ops.rs:160-164`)
- EXC-212b: a missing file → the entry carries the low-level text `not found: {path}`, not an `ERR_*` string (`fs_ops.rs:188-190`)
- EXC-212c: an empty `paths` array → an empty `files` array

## 6. Functional Requirements

**Format:** EARS notation is mandatory. Forbidden modals are not used.

### Shared engine rules

#### FR-201 [EARS-U]: The engine is the only implementation
> Every filesystem operation SHALL be implemented once, in `core::fs_ops`, and both the MCP tool layer and the REST data plane SHALL call it rather than reimplement it.

- **Inputs:** any filesystem operation request from either surface.
- **Outputs:** identical behaviour on both surfaces.
- **Business Rules:** a tool handler performs exactly three steps: authorize, normalize every path, call the engine. `api/dataplane.rs` is bound by the same rule. A write that bypasses the engine charges no quota and leaves no audit entry, which is the defect that moved the REST upload path onto `fs_ops::write_bytes` (`core/fs_ops.rs:573-578`).
- **Priority:** Must-have

#### FR-202 [EARS-E]: Every text mutation goes through the shared commit path
> WHEN a text mutation is committed THE engine SHALL charge the byte count against the session quota, write the bytes atomically, and record an audit entry, in that order.

- **Inputs:** the new text, the operation name.
- **Outputs:** a committed file plus session accounting (`fs_ops.rs:556-570`).
- **Business Rules:** the quota is charged **before** the write, so a rejected charge writes nothing. The audit entry follows a successful write, so the log records what happened rather than what was attempted.
- **Priority:** Must-have

#### FR-203 [EARS-U]: Result shapes are frozen
> The engine SHALL return the result keys fixed in `TOOL_CONTRACT.txt` for every tool.

- **Inputs:** a successful tool call.
- **Outputs:** the documented key set, for example `{path, bytes_appended}` for `fs.append` and `{path, applied, diff}` for `fs.edit` (`TOOL_CONTRACT.txt:287-296`).
- **Business Rules:** the tool schemas and descriptions are machine-checked against `tool-contract-golden.json` on every test run; this requirement extends that discipline to the return shapes, which the golden file does not cover.
- **Priority:** Must-have

### Reading

#### FR-204 [EARS-E]: Paged line window
> WHEN `fs.read` is called THE engine SHALL return a line window starting at `offset_lines`, capped at the lesser of `limit_lines` and `safety.max_read_lines`, together with `total_lines`, `truncated` and `next_offset`.

- **Inputs:** `path`, `offset_lines` default 0, `limit_lines` default 2000, `line_numbered` default true.
- **Outputs:** `{content, total_lines, truncated, next_offset}` (`fs_ops.rs:70-75`).
- **Business Rules:** the server cap wins silently over a larger caller request (`fs_ops.rs:65`). `next_offset` is null exactly when `truncated` is false. Line numbering starts at `offset_lines + 1` (`fs_ops.rs:69`).
- **Priority:** Must-have

#### FR-205 [EARS-E]: A read unlocks later writes
> WHEN any read operation succeeds THE engine SHALL record the read against the session for that path.

- **Inputs:** the path read.
- **Outputs:** an updated session read set (`fs_ops.rs:62,89,167`).
- **Business Rules:** `fs.read`, `fs.read_bytes` and `fs.read_many` all record. Listing, globbing and grepping do **not**, so finding a file is not the same as having read it.
- **Priority:** Must-have

#### FR-206 [EARS-E]: Batch read isolates per-file failure
> WHEN `fs.read_many` encounters a path it cannot read THE engine SHALL record the failure against that entry and SHALL continue with the remaining paths.

- **Inputs:** `paths`, `per_file_cap_lines` default 500.
- **Outputs:** `{files: [{path, content, truncated} | {path, error}]}` (`fs_ops.rs:169-178`).
- **Business Rules:** a normalization failure keeps the caller's raw path spelling and the rendered `ERR_*: message` (`fs_ops.rs:160-164`); a storage failure carries the low-level text such as `not found: {path}` (`fs_ops.rs:188-190`). Entry order matches request order.
- **Priority:** Must-have

#### FR-207 [EARS-E]: Byte range reads
> WHEN `fs.read_bytes` is called THE engine SHALL return the requested byte range base64-encoded, with the MIME type guessed from the path extension and the actual length returned.

- **Inputs:** `offset_bytes` default 0, `length_bytes` default 65536.
- **Outputs:** `{base64, mime_type, length}` (`fs_ops.rs:90-94`).
- **Business Rules:** negative offsets and lengths are clamped to zero rather than rejected (`fs_ops.rs:88`). An unrecognized extension yields `application/octet-stream` (`fs_ops.rs:92`). The table consulted is the engine's own `mime_guess` (`core/fs_ops.rs:1286-1310`), **not** `docs/mime.rs`, despite the latter's comment claiming otherwise; see §13 item 3.
- **Priority:** Must-have

#### FR-208 [EARS-E]: Line-oriented read helpers
> WHEN `fs.read_lines`, `fs.head`, `fs.tail`, `fs.read_section` or `fs.count_lines` is called THE engine SHALL return the requested slice without returning the whole file.

- **Inputs:** per-tool: an inclusive `[start_line, end_line]`; `lines` default 20; an `anchor_line` with `max_lines` default 200.
- **Outputs:** the slice, or `{total_lines}` for `fs.count_lines` (`fs_ops.rs:99-146,201-236`).
- **Business Rules:** `fs.read_lines` bounds are 1-based and inclusive. `fs.read_section` returns the indentation block surrounding the anchor, computed from leading whitespace (`fs_ops.rs:1132-1162`).
- **Priority:** Must-have

### Writing

#### FR-209 [EARS-O]: No-clobber is the default for creation
> IF the target path already exists and `overwrite` is false THEN the engine SHALL refuse the write with `ERR_NO_CLOBBER` and the message `'{path}' exists (pass overwrite=true)`.

- **Inputs:** `path`, `content` or `base64`, `overwrite` default false.
- **Outputs:** the refusal, or `{path, bytes_written, overwritten}` (`fs_ops.rs:596-598,613`).
- **Business Rules:** the same rule governs `fs.write`, `fs.write_bytes`, `fs.copy` and `fs.move`.
- **Priority:** Must-have

#### FR-210 [EARS-O]: Overwriting requires a prior read
> IF a write targets an existing path THEN the engine SHALL require that the session has read that path, refusing otherwise with `ERR_EDIT_WITHOUT_PRIOR_READ`.

- **Inputs:** the target path, the session read set.
- **Outputs:** the refusal, or a committed write (`fs_ops.rs:599-600`).
- **Business Rules:** the guard applies only when the path exists; creating a new file needs no prior read. After a successful write the path is recorded as read, so an immediate second write succeeds (`fs_ops.rs:606`).
- **Priority:** Must-have

#### FR-211 [EARS-E]: Parent creation
> WHEN `create_parents` is true THE engine SHALL create every missing parent directory before writing.

- **Inputs:** `create_parents`, default true.
- **Outputs:** the directory chain plus the file (`fs_ops.rs:601-603,1094-1104`).
- **Business Rules:** with `create_parents` false and a missing parent, the storage layer refuses and nothing is written.
- **Priority:** Must-have

#### FR-212 [EARS-E]: Append
> WHEN `fs.append` is called THE engine SHALL append the content to the existing file and SHALL report the number of bytes appended.

- **Inputs:** `content`, `create` default false.
- **Outputs:** `{path, bytes_appended}` (`TOOL_CONTRACT.txt:287`).
- **Business Rules:** with `create` true a missing file is created; with `create` false a missing file is `ERR_NOT_FOUND`. The append is a read-modify-write through the shared commit path, so it charges the full resulting size against the quota.
- **Priority:** Must-have

#### FR-213 [EARS-E]: Empty file creation
> WHEN `fs.create_empty` is called THE engine SHALL create a zero-length file, and WHEN `exist_ok` is false and the path exists THE engine SHALL refuse.

- **Inputs:** `path`, `exist_ok` default false.
- **Outputs:** the created node (`fs_ops.rs:824-843`).
- **Business Rules:** an empty file stores no blob, per S1 FR-040.
- **Priority:** Must-have

### Editing

#### FR-214 [EARS-O]: Unique match editing
> IF `old_string` occurs exactly once THEN the engine SHALL replace it; IF it occurs more than once and `replace_all` is false THEN the engine SHALL refuse with `ERR_AMBIGUOUS_MATCH`; IF it does not occur THEN the engine SHALL refuse with `ERR_NO_MATCH`.

- **Inputs:** `old_string`, `new_string`, `replace_all` default false.
- **Outputs:** `{path, applied, diff}` or the refusal (`fs_ops.rs:1180-1201`).
- **Business Rules:** the messages are `"old_string not found in '{path}'"` and `"old_string matches {count} sites in '{path}' (use replace_all)"`, the second carrying the actual count. Matching is byte-exact substring matching with no normalization.
- **Priority:** Must-have

#### FR-215 [EARS-E]: Dry run produces the diff without writing
> WHEN `dry_run` is true THE engine SHALL compute and return the unified diff and SHALL NOT write the file.

- **Inputs:** `dry_run`, default false.
- **Outputs:** `applied` false with a populated `diff` (`fs_ops.rs:858-864`).
- **Business Rules:** `applied` equals the negation of `dry_run`. A dry run still enforces the read guard and still fails on a non-matching `old_string`, so it is a true preview of the same operation.
- **Priority:** Must-have

#### FR-216 [EARS-E]: Multi-edit is all-or-nothing and order-dependent
> WHEN `fs.multi_edit` is called THE engine SHALL apply each edit against the text produced by the previous edit and SHALL write nothing unless every edit resolves.

- **Inputs:** `edits`, an array of `{old_string, new_string, replace_all?}`.
- **Outputs:** `{path, applied, edits, diff}` where `edits` is the count applied (`fs_ops.rs:869-902`).
- **Business Rules:** the returned diff compares the **original** text with the final text, not the intermediate steps. Composition order matters: an edit whose `old_string` an earlier edit destroyed fails the whole call.
- **Priority:** Must-have

#### FR-217 [EARS-O]: Fuzzy block replacement threshold
> IF `fuzzy` is true THEN the engine SHALL slide a window of the search block's line count over the file, keep the most similar candidate, and accept it only at a similarity of at least 0.6.

- **Inputs:** `search_block`, `replace_block`, `fuzzy` default false.
- **Outputs:** `{path, applied, diff}` or `ERR_NO_MATCH` with `"no fuzzy match for search_block in '{path}'"` (`fs_ops.rs:1205-1236`).
- **Business Rules:** the fuzzy replace threshold `FUZZY_THRESHOLD` is 0.6 (`fs_ops.rs:41`). The similarity ratio is LCS-based, in `0.0..=1.0` (`fs_ops.rs:1240-1256`). The replacement block is newline-terminated whether or not the caller terminated it (`fs_ops.rs:1229-1233`).
- **Priority:** Must-have

#### FR-218 [EARS-E]: Line insertion
> WHEN `fs.insert_at_line` is called THE engine SHALL insert the content immediately before the given 1-based line number.

- **Inputs:** `line` (1-based), `content`.
- **Outputs:** `{path, applied, line}` (`TOOL_CONTRACT.txt:288`).
- **Business Rules:** a line number past the end appends; the read guard applies as for every edit.
- **Priority:** Must-have

#### FR-219 [EARS-E]: V4A patch application
> WHEN `fs.apply_patch` is called THE engine SHALL parse the patch and apply each file operation in order, returning one entry per touched path.

- **Inputs:** `patch_text` delimited by `*** Begin Patch` and `*** End Patch`, with operations `*** Add File: `, `*** Update File: `, `*** Delete File: ` and the modifier `*** Move to: `, hunks introduced by `@@` (`fs_ops.rs:1552-1558`).
- **Outputs:** `{files: [{path, op} | {path, op, moved_to}]}` (`fs_ops.rs:1519-1546`).
- **Business Rules:** every operation normalizes its own path (`fs_ops.rs:1514`). Adds charge the quota and write without a read-guard check; updates and deletes enforce the guard (`fs_ops.rs:1521,1526`). An update carrying `*** Move to: ` writes in place and then renames (`fs_ops.rs:1539-1543`). **The patch is not transactional**: operations are applied in sequence and a failure partway leaves earlier operations committed.
- **Priority:** Must-have

#### FR-220 [EARS-X]: Duplicate audit entry on patched updates
> `fs.apply_patch` records two audit entries per updated file: one inside the update branch and one in the loop tail (`fs_ops.rs:1537,1546`). The duplicate is observable through `fs.audit_log` and is retained deliberately.

- **Inputs:** an update operation in a patch.
- **Outputs:** two `apply_patch` audit entries for the same path.
- **Business Rules:** this is the one behaviour in this spec with no clean EARS pattern, because it specifies the preservation of a quirk rather than an intended rule. It is frozen because `fs.audit_log` output is part of the observable contract.
- **Priority:** Should-have

### Listing, metadata and search

#### FR-221 [EARS-E]: Directory listing
> WHEN `fs.list_dir` is called THE engine SHALL return a flat listing of the directory with each entry's name and kind, defaulting the path to `/`.

- **Inputs:** `path` default `/`, `include_hidden` default false, `sort_by` default `name`, `with_sizes` default false.
- **Outputs:** `{path, entries: [{name, kind}], total}` (`TOOL_CONTRACT.txt:291`).
- **Business Rules:** hidden entries are those whose name begins with `.`; they are omitted unless requested. Sizes appear only when `with_sizes` is true.
- **Priority:** Must-have

#### FR-222 [EARS-E]: Recursive tree with caps
> WHEN `fs.tree` is called THE engine SHALL return a nested tree to `max_depth` and SHALL stop after 2000 nodes, reporting truncation.

- **Inputs:** `path` default `/`, `max_depth` default 3, `exclude_patterns`, `with_sizes`.
- **Outputs:** `{path, tree: [...]}` with nested `children` (`TOOL_CONTRACT.txt:292`).
- **Business Rules:** the tree cap `TREE_CAP` is 2000 (`fs_ops.rs:39`). The tree prunes `.git`, `node_modules`, `target`, `dist`, `.build` and `coverage`, and **deliberately does not prune the trash directory**, unlike the walk used by glob and grep (`fs_ops.rs:27-30`).
- **Priority:** Must-have

#### FR-223 [EARS-U]: Volume walk ceiling
> The engine SHALL visit at most 5000 files in any single walk.

- **Inputs:** any walk-based operation: glob, grep, symbol search.
- **Outputs:** results limited to the files visited (`fs_ops.rs:35,286-316`).
- **Business Rules:** `MAX_FILES` is 5000. The walk skips `DEFAULT_EXCLUDES`: `.git`, `node_modules`, `target`, `dist`, `.build`, `coverage` and `.mcp_trash` (`fs_ops.rs:25-26`).
- **Priority:** Must-have

#### FR-224 [EARS-E]: Glob matching and ordering
> WHEN `fs.glob` is called THE engine SHALL match the pattern against both the full path and the bare file name, return matches newest first, and cap the result at 100 with a truncation flag.

- **Inputs:** `pattern`, `root` default `/`, `exclude_patterns`.
- **Outputs:** `{matches, truncated}` (`fs_ops.rs:338-344`).
- **Business Rules:** the glob cap `GLOB_CAP` is 100 (`fs_ops.rs:37`). Sorting is by mtime descending and is stable, so equal mtimes keep walk order (`fs_ops.rs:337-338`). Matching uses `util::text::Fnmatch`, which is Python `fnmatch` semantics, so a single `*` **does** cross a `/` boundary.
- **Priority:** Must-have

#### FR-225 [EARS-E]: Grep output modes
> WHEN `fs.grep` is called THE engine SHALL shape its result by `output_mode`: `files` yields `{files}`, `count` yields `{count, files}`, and any other value yields `{matches, truncated}`.

- **Inputs:** `pattern`, `root`, `include_glob`, `exclude_glob`, `regex` default true, `case_sensitive` default true, `output_mode` default `content`, `context_lines` default 0, `max_matches` default 100.
- **Outputs:** one of the three shapes (`fs_ops.rs:346-349`, `TOOL_CONTRACT.txt:294-295`).
- **Business Rules:** `content` is the default and is reached by the fall-through branch, so an unrecognized `output_mode` behaves as `content` rather than failing.
- **Priority:** Must-have

#### FR-226 [EARS-E]: Symbol search
> WHEN `fs.find_definition` or `fs.find_references` is called THE engine SHALL walk the volume from `root` and run the language-aware matcher on every file whose language is recognized.

- **Inputs:** `name`, `root` default `/`, `kind` filter for definitions.
- **Outputs:** `{definitions: [{path, name, kind, line}]}` or `{references: [{path, line, kind}]}` (`TOOL_CONTRACT.txt:283-284`).
- **Business Rules:** files in unrecognized languages are skipped silently. No match yields an empty array, not an error. The matcher itself is S5's.
- **Priority:** Must-have

#### FR-227 [EARS-E]: Metadata probes do not mutate or record
> WHEN `fs.stat`, `fs.exists`, `fs.hash`, `fs.count_lines`, `fs.list_dir`, `fs.tree`, `fs.glob` or `fs.grep` is called THE engine SHALL neither mutate the volume nor record a read against the session.

- **Inputs:** any probe.
- **Outputs:** the probe result only.
- **Business Rules:** these are pure probes (`tools/metadata.rs:1-4`). The consequence is deliberate: locating a file through search does not satisfy the read guard for a later edit.
- **Priority:** Must-have

#### FR-228 [EARS-U]: Stat reports synthetic ownership
> `fs.stat` SHALL report uid 1000 and gid 1000 for every entry.

- **Inputs:** a path.
- **Outputs:** POSIX-shaped metadata including mode, size and times (`fs_ops.rs:237-252`).
- **Business Rules:** `STAT_UID` and `STAT_GID` are both 1000 (`fs_ops.rs:44-45`); the volume has no real POSIX owner, so the values are synthetic and constant.
- **Priority:** Must-have

#### FR-229 [EARS-O]: Hash algorithm allow-list
> IF `algo` is not one of `md5`, `sha1`, `sha256` or `sha512` THEN the engine SHALL refuse with `ERR_INVALID_ARGUMENT`.

- **Inputs:** `algo`, default `sha256`.
- **Outputs:** the digest, or the refusal (`fs_ops.rs:32,263-285`).
- **Business Rules:** the allow-list is closed; the default is `sha256`, which is also the content-addressing hash.
- **Priority:** Must-have

### Lifecycle

#### FR-230 [EARS-E]: Delete moves to trash by default
> WHEN `fs.delete` is called with `trash` true THE engine SHALL create the trash parent directory and rename the path into it, returning the destination.

- **Inputs:** `path`, `recursive` default false, `trash` default true.
- **Outputs:** `{path, trashed, trash_path}` (`fs_ops.rs:1019-1023`).
- **Business Rules:** the destination comes from the S1 trash derivation; its parent is created before the rename (`fs_ops.rs:1007-1010`). The audit detail is `-> {destination}` for a soft delete and `hard` for a hard one (`fs_ops.rs:1015-1018`).
- **Priority:** Must-have

#### FR-231 [EARS-O]: Hard delete requires explicit configuration
> IF `trash` is false and `safety.allow_hard_delete` is false THEN the engine SHALL refuse with `ERR_NOT_SUPPORTED` and the message `hard delete disabled (server started without allow_hard_delete)`.

- **Inputs:** `trash` false.
- **Outputs:** the refusal (`fs_ops.rs:995-999`).
- **Business Rules:** `allow_hard_delete` defaults to false, so destroying data requires a deliberate deployment decision.
- **Priority:** Must-have

#### FR-232 [EARS-O]: Directory deletion requires recursive
> IF the target is a directory and `recursive` is false THEN the engine SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'{path}' is a directory (pass recursive=true)`.

- **Inputs:** `path`, `recursive`.
- **Outputs:** the refusal (`fs_ops.rs:989-993`).
- **Business Rules:** the existence check precedes the directory check, so a missing path yields `ERR_NOT_FOUND` rather than the directory message (`fs_ops.rs:984-987`).
- **Priority:** Must-have

#### FR-233 [EARS-E]: Move and copy
> WHEN `fs.move` or `fs.copy` is called THE engine SHALL refuse an existing destination unless `overwrite` is true, and SHALL require `recursive` for a tree copy.

- **Inputs:** `source`, `destination`, `overwrite` default false, `recursive` default false for copy.
- **Outputs:** `{source, destination}` (`TOOL_CONTRACT.txt:288-289`).
- **Business Rules:** a copy is metadata-only: the destination node references the same content blob and the refcount rises, per S1 FR-038. No bytes are duplicated.
- **Priority:** Must-have

#### FR-234 [EARS-E]: Session introspection tools
> WHEN `fs.list_allowed_roots` or `fs.audit_log` is called THE engine SHALL apply the membership gate and SHALL return session-scoped information without opening the volume.

- **Inputs:** `mount_id`; for the log, `since` and `limit` default 20.
- **Outputs:** `{person, roots: [{mount_id, root, owner}]}` and `{entries: [{timestamp, op, path, detail}]}` (`TOOL_CONTRACT.txt:290-291`).
- **Business Rules:** both take `mount_id` and pass the membership gate even though neither opens a volume (`tools/lifecycle.rs:4-5`). The audit log is the capped per-session ring from S1 FR-036, so it is empty after a restart.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Every walk-based operation is bounded: 5000 files visited, 100 glob results, 2000 tree nodes (`fs_ops.rs:35-39`). These ceilings are contract, not tuning: they bound both server work and the tokens returned to an agent.
- `fs.read` caps its window at `safety.max_read_lines` regardless of the caller's request (FR-204).
- `fs.count_lines` and `fs.hash` avoid returning content, so an agent can size a file before reading it.

### 7.2 Security
- Inherited unchanged from S1: membership gate before any storage access, normalization of every path argument, quota on every write, audit on every mutation.
- The V4A patch normalizes each operation's path individually (`fs_ops.rs:1514`), so a patch cannot reach outside the volume through an unnormalized entry.
- Hard delete is off by default (FR-231).

### 7.3 Usability
- Error messages name the remedy: `pass overwrite=true`, `use replace_all`, `pass recursive=true` (`fs_ops.rs:597,1192,991`).
- `fs.read_many` isolates failures so one bad path does not lose the whole batch (FR-206).
- `dry_run` gives an agent a true preview with the same validation as the real edit (FR-215).

### 7.4 Reliability
- Writes are atomic at the volume client level; a failed write leaves the previous content intact.
- `fs.multi_edit` is all-or-nothing (FR-216).
- `fs.apply_patch` is explicitly **not** transactional (FR-219); this is a known limitation recorded as TBD-201 rather than a defect.

### 7.5 Observability
Unchanged from S1 §7.5: `tracing` to stderr, `RUST_LOG` filtering, expected client errors at INFO without a backtrace. Every mutation additionally leaves a session audit entry readable through `fs.audit_log`, which is the closest thing this layer has to an operation log. OpenTelemetry remains absent (S1 BL-001).

### 7.6 Deployment
No deployment surface of its own: the engine ships inside the single server binary specified by S1 §7.6.

### 7.7 Scalability
- The walk ceilings bound worst-case work per request independently of volume size.
- A copy is metadata-only, so duplicating a large tree costs rows, not bytes.
- Because the session read set is per process (S1 TBD-001), the read guard behaves differently under multiple replicas; an agent's read on one replica does not unlock an edit on another.

## 8. Data Model

No new persisted entity. S2 operates on S1's `NodeRow`, `blob_refs` and `SessionState`. Transient engine-level structures:

| Structure | Shape | Home |
|---|---|---|
| **Window** | `{content, total_lines, truncated, next_offset}` | `fs_ops.rs:70-75` |
| **Diff** | unified text, `--- path` / `+++ path` / `@@` hunks | `core/diff.rs` |
| **FileOp** | `{kind: Add\|Update\|Delete, path, add_content, move_to, hunks}` | `fs_ops.rs:1560+` |
| **Hunk** | context, removed and added line groups | `fs_ops.rs:1666-1680` |
| **Match** | `{path, line, text, context}` | `TOOL_CONTRACT.txt:294` |

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact Type | Description |
|---|---|---|
| `crates/mcp-fs/src/core/fs_ops.rs` | Specified, unchanged | FR-201..FR-234 |
| `crates/mcp-fs/src/core/diff.rs` | Specified, unchanged | FR-215, FR-216 |
| `crates/mcp-fs/src/util/text.rs` | Specified, unchanged | FR-224 |
| `crates/mcp-fs/src/tools/{read,write,edit,listing,metadata,lifecycle,search}.rs` | Specified, unchanged | Thin adapters, FR-201 |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | Referenced | FR-203 |
| `tests/functional/scenarios/` | Extended | New scenarios per §12 |

### 9.2 Affected Requirements

S1 requirements consumed unchanged: FR-020, FR-021, FR-032, FR-034, FR-035, FR-036, FR-037, FR-038, FR-040. None is modified. S1 §3.2 deferred the safety layer's end-to-end exercise to this spec; §12 here discharges that by testing the guard, the quota and the trash path through real tool calls.

### 9.3 Affected Tests

| Test location | Coverage today | Action |
|---|---|---|
| `crates/mcp-fs/src/core/fs_ops.rs` `#[cfg(test)]` | engine behaviours | Keep; annotate with ids |
| `crates/mcp-fs/src/core/diff.rs` | unified diff shape | Keep |
| `crates/mcp-fs/src/util/text.rs` | line splitting, Fnmatch | Keep |
| `tests/functional/scenarios/02_fs_crud.sh` | create, read, write, delete over HTTP | Extend |
| `tests/functional/scenarios/03_fs_edit.sh` | edit paths over HTTP | Extend |
| `tests/functional/scenarios/04_fs_tree.sh` | listing and tree | Extend |
| `tests/functional/scenarios/06_read_advanced.sh` | paging and read helpers | Extend |
| `tests/functional/scenarios/13_workflow.sh` | end-to-end agent workflow | Extend |

### 9.4 Affected Documentation

| Document | Section | Action |
|---|---|---|
| `AGENTS.md` | Documentation index | Reference this spec beside S1 |
| `.agent_docs/tools.md` | `fs.*` reference | Cross-reference rather than restate |

### 9.5 Dependencies & Risks

No new dependency and no code change. Two risks worth naming:

1. **`fs.apply_patch` is not transactional** (FR-219). An agent applying a large patch that fails midway is left with a partially patched volume and no automatic rollback. Recorded as TBD-201.
2. **Two MIME guessers exist.** `fs.read_bytes` uses `fs_ops::mime_guess` (`core/fs_ops.rs:1286-1310`), while the REST download route uses `docs::guess_mime` (`docs/mime.rs:10`). The cross-spec audit compared them: both hold exactly 32 extensions with identical values, so they agree today. The duplication is a standing regression risk rather than a live defect. Recorded as TBD-202.

## 10. Documentation Requirements

### 10.1 README.md
No change required.

### 10.2 AGENTS.md & .agent_docs/
- `AGENTS.md`: list this spec in the documentation index.
- `.agent_docs/tools.md`: point the `fs.*` section at §6 rather than duplicating the rules.

### 10.3 docs/*
None required.

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Tests (Happy) | E2E Tests (Failure) | E2E Tests (Edge) |
|---|---|---|---|---|
| SC-201 | FR-204, FR-205, FR-208 | E2E-201, E2E-202 | E2E-203, E2E-204, E2E-205 | E2E-206, E2E-207, E2E-208 |
| SC-202 | FR-202, FR-209, FR-210, FR-211, FR-212, FR-213 | E2E-209, E2E-210, E2E-211 | E2E-212, E2E-213, E2E-214, E2E-215, E2E-216, E2E-217 | E2E-218, E2E-219, E2E-220 |
| SC-203 | FR-214, FR-215 | E2E-221, E2E-222 | E2E-223, E2E-224, E2E-225, E2E-226 | E2E-227, E2E-228 |
| SC-204 | FR-216 | E2E-229 | E2E-230, E2E-231, E2E-232 | E2E-233, E2E-234 |
| SC-205 | FR-217 | E2E-235 | E2E-236, E2E-237, E2E-238 | E2E-239, E2E-240 |
| SC-206 | FR-219, FR-220 | E2E-241 | E2E-242, E2E-243, E2E-244 | E2E-245, E2E-246, E2E-247 |
| SC-207 | FR-221, FR-222, FR-227, FR-228, FR-229 | E2E-248, E2E-249, E2E-250 | E2E-251, E2E-252, E2E-253 | E2E-254, E2E-255, E2E-256 |
| SC-208 | FR-223, FR-224, FR-225 | E2E-257, E2E-258 | E2E-259, E2E-260, E2E-261 | E2E-262, E2E-263, E2E-264, E2E-265 |
| SC-209 | FR-226 | E2E-266 | E2E-267, E2E-268 | E2E-269 |
| SC-210 | FR-230, FR-231, FR-232, FR-233 | E2E-270, E2E-271 | E2E-272, E2E-273, E2E-274, E2E-275 | E2E-276, E2E-277 |
| SC-211 | FR-207, FR-234 | E2E-278, E2E-279 | E2E-280, E2E-281 | E2E-282, E2E-283 |
| SC-212 | FR-201, FR-203, FR-206, FR-218 | E2E-284, E2E-285 | E2E-286, E2E-287, E2E-288 | E2E-289, E2E-290 |

Per-FR coverage (each at least three tests):

| FR | Tests | FR | Tests |
|---|---|---|---|
| FR-201 | E2E-284, E2E-286, E2E-289 | FR-218 | E2E-285, E2E-288, E2E-290 |
| FR-202 | E2E-209, E2E-216, E2E-219 | FR-219 | E2E-241, E2E-242, E2E-243, E2E-245, E2E-246 |
| FR-203 | E2E-284, E2E-287, E2E-290 | FR-220 | E2E-244, E2E-247, E2E-241 |
| FR-204 | E2E-201, E2E-203, E2E-206, E2E-207 | FR-221 | E2E-248, E2E-251, E2E-254 |
| FR-205 | E2E-202, E2E-204, E2E-208 | FR-222 | E2E-249, E2E-252, E2E-255 |
| FR-206 | E2E-284, E2E-286, E2E-288, E2E-289 | FR-223 | E2E-257, E2E-259, E2E-262 |
| FR-207 | E2E-278, E2E-280, E2E-282 | FR-224 | E2E-258, E2E-260, E2E-263, E2E-264 |
| FR-208 | E2E-202, E2E-205, E2E-208 | FR-225 | E2E-257, E2E-261, E2E-265 |
| FR-209 | E2E-210, E2E-212, E2E-218 | FR-226 | E2E-266, E2E-267, E2E-268, E2E-269 |
| FR-210 | E2E-213, E2E-217, E2E-220 | FR-227 | E2E-250, E2E-253, E2E-256 |
| FR-211 | E2E-211, E2E-214, E2E-219 | FR-228 | E2E-250, E2E-253, E2E-256 |
| FR-212 | E2E-209, E2E-215, E2E-218 | FR-229 | E2E-248, E2E-251, E2E-254 |
| FR-213 | E2E-211, E2E-216, E2E-220 | FR-230 | E2E-270, E2E-272, E2E-276 |
| FR-214 | E2E-221, E2E-223, E2E-224, E2E-227 | FR-231 | E2E-273, E2E-276, E2E-277 |
| FR-215 | E2E-222, E2E-225, E2E-226, E2E-228 | FR-232 | E2E-274, E2E-275, E2E-277 |
| FR-216 | E2E-229, E2E-230, E2E-231, E2E-232, E2E-233 | FR-233 | E2E-271, E2E-274, E2E-275 |
| FR-217 | E2E-235, E2E-236, E2E-237, E2E-239 | FR-234 | E2E-279, E2E-281, E2E-283 |

## 12. End-to-End Test Suite

**Placement.** Tool-level tests are shell scenarios under `tests/functional/scenarios/`, extending `02_fs_crud.sh`, `03_fs_edit.sh`, `04_fs_tree.sh`, `06_read_advanced.sh` and `13_workflow.sh`. Engine-level tests are Rust tests in the `#[cfg(test)]` modules of `core/fs_ops.rs`, `core/diff.rs` and `util/text.rs`. Tests marked **Existing** are already implemented and are required only to carry their `E2E-2xx` id; tests marked **New** are specified in full in §12.2.

**Fixtures, shared by every tool-level test:** project `spec-fs` with `ALICE` = `alice@example.com` as owner and member; a seeded volume containing `/a.txt` with the three lines `one`, `two`, `three`; `/src/app.py`; `/bin.dat` holding 4 non-UTF8 bytes; `/big.txt` with 5000 numbered lines.

### 12.1 Test Summary

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-201 | Existing | Core Journey | SC-201 | FR-204 | Critical |
| E2E-202 | New | Core Journey | SC-201 | FR-205, FR-208 | Critical |
| E2E-203 | New | Error | SC-201 | FR-204 | High |
| E2E-204 | New | Security | SC-201 | FR-205 | Critical |
| E2E-205 | Existing | Error | SC-201 | FR-208 | High |
| E2E-206 | New | Edge | SC-201 | FR-204 | High |
| E2E-207 | New | Edge | SC-201 | FR-204 | High |
| E2E-208 | Existing | Edge | SC-201 | FR-205, FR-208 | Medium |
| E2E-209 | Existing | Core Journey | SC-202 | FR-202, FR-212 | Critical |
| E2E-210 | Existing | Feature | SC-202 | FR-209 | Critical |
| E2E-211 | Existing | Feature | SC-202 | FR-211, FR-213 | High |
| E2E-212 | Existing | Error | SC-202 | FR-209 | Critical |
| E2E-213 | New | Security | SC-202 | FR-210 | Critical |
| E2E-214 | Existing | Error | SC-202 | FR-211 | High |
| E2E-215 | Existing | Error | SC-202 | FR-212 | High |
| E2E-216 | New | Error | SC-202 | FR-202, FR-213 | Critical |
| E2E-217 | New | Error | SC-202 | FR-210 | High |
| E2E-218 | New | Edge | SC-202 | FR-209, FR-212 | Medium |
| E2E-219 | New | Side Effect | SC-202 | FR-202, FR-211 | High |
| E2E-220 | Existing | Edge | SC-202 | FR-210, FR-213 | Medium |
| E2E-221 | Existing | Core Journey | SC-203 | FR-214 | Critical |
| E2E-222 | Existing | Feature | SC-203 | FR-215 | Critical |
| E2E-223 | Existing | Error | SC-203 | FR-214 | Critical |
| E2E-224 | Existing | Error | SC-203 | FR-214 | Critical |
| E2E-225 | New | Error | SC-203 | FR-215 | High |
| E2E-226 | New | Error | SC-203 | FR-215 | High |
| E2E-227 | New | Edge | SC-203 | FR-214 | High |
| E2E-228 | New | Edge | SC-203 | FR-215 | Medium |
| E2E-229 | Existing | Core Journey | SC-204 | FR-216 | Critical |
| E2E-230 | Existing | Error | SC-204 | FR-216 | Critical |
| E2E-231 | New | Data Integrity | SC-204 | FR-216 | Critical |
| E2E-232 | New | Error | SC-204 | FR-216 | High |
| E2E-233 | New | Edge | SC-204 | FR-216 | Medium |
| E2E-234 | New | Edge | SC-204 | FR-216 | Medium |
| E2E-235 | Existing | Feature | SC-205 | FR-217 | Critical |
| E2E-236 | Existing | Error | SC-205 | FR-217 | Critical |
| E2E-237 | New | Error | SC-205 | FR-217 | High |
| E2E-238 | Existing | Error | SC-205 | FR-217 | High |
| E2E-239 | New | Edge | SC-205 | FR-217 | High |
| E2E-240 | New | Edge | SC-205 | FR-217 | Medium |
| E2E-241 | Existing | Core Journey | SC-206 | FR-219, FR-220 | Critical |
| E2E-242 | Existing | Error | SC-206 | FR-219 | Critical |
| E2E-243 | New | Error | SC-206 | FR-219 | Critical |
| E2E-244 | New | Data Integrity | SC-206 | FR-220 | High |
| E2E-245 | New | Edge | SC-206 | FR-219 | High |
| E2E-246 | New | Edge | SC-206 | FR-219 | High |
| E2E-247 | New | Edge | SC-206 | FR-220 | Medium |
| E2E-248 | Existing | Feature | SC-207 | FR-221, FR-229 | Critical |
| E2E-249 | Existing | Feature | SC-207 | FR-222 | Critical |
| E2E-250 | Existing | Feature | SC-207 | FR-227, FR-228 | High |
| E2E-251 | Existing | Error | SC-207 | FR-221, FR-229 | High |
| E2E-252 | New | Error | SC-207 | FR-222 | High |
| E2E-253 | New | Security | SC-207 | FR-227, FR-228 | Critical |
| E2E-254 | New | Edge | SC-207 | FR-221, FR-229 | Medium |
| E2E-255 | New | Edge | SC-207 | FR-222 | High |
| E2E-256 | New | Edge | SC-207 | FR-227, FR-228 | Medium |
| E2E-257 | Existing | Feature | SC-208 | FR-223, FR-225 | Critical |
| E2E-258 | Existing | Feature | SC-208 | FR-224 | Critical |
| E2E-259 | New | Error | SC-208 | FR-223 | High |
| E2E-260 | Existing | Error | SC-208 | FR-224 | High |
| E2E-261 | Existing | Error | SC-208 | FR-225 | High |
| E2E-262 | New | Edge | SC-208 | FR-223 | High |
| E2E-263 | New | Edge | SC-208 | FR-224 | Critical |
| E2E-264 | New | Edge | SC-208 | FR-224 | High |
| E2E-265 | New | Edge | SC-208 | FR-225 | Medium |
| E2E-266 | Existing | Feature | SC-209 | FR-226 | High |
| E2E-267 | Existing | Error | SC-209 | FR-226 | Medium |
| E2E-268 | New | Error | SC-209 | FR-226 | Medium |
| E2E-269 | New | Edge | SC-209 | FR-226 | Medium |
| E2E-270 | Existing | Core Journey | SC-210 | FR-230 | Critical |
| E2E-271 | Existing | Feature | SC-210 | FR-233 | Critical |
| E2E-272 | New | Side Effect | SC-210 | FR-230 | Critical |
| E2E-273 | Existing | Security | SC-210 | FR-231 | Critical |
| E2E-274 | Existing | Error | SC-210 | FR-232, FR-233 | High |
| E2E-275 | Existing | Error | SC-210 | FR-232, FR-233 | High |
| E2E-276 | New | Edge | SC-210 | FR-230, FR-231 | High |
| E2E-277 | New | Edge | SC-210 | FR-231, FR-232 | Medium |
| E2E-278 | Existing | Feature | SC-211 | FR-207 | Critical |
| E2E-279 | Existing | Feature | SC-211 | FR-234 | High |
| E2E-280 | New | Error | SC-211 | FR-207 | Medium |
| E2E-281 | New | Security | SC-211 | FR-234 | High |
| E2E-282 | New | Edge | SC-211 | FR-207 | High |
| E2E-283 | New | Edge | SC-211 | FR-234 | Medium |
| E2E-284 | New | Core Journey | SC-212 | FR-201, FR-203, FR-206 | Critical |
| E2E-285 | Existing | Feature | SC-212 | FR-218 | High |
| E2E-286 | New | Error | SC-212 | FR-201, FR-206 | Critical |
| E2E-287 | New | Error | SC-212 | FR-203 | High |
| E2E-288 | New | Error | SC-212 | FR-206, FR-218 | High |
| E2E-289 | New | Edge | SC-212 | FR-201, FR-206 | High |
| E2E-290 | New | Edge | SC-212 | FR-203, FR-218 | Medium |

**Coverage Statistics** (90 tests):
- Happy path (Core Journey + Feature): 26
- Failure/error (Error + Security): 38
- Side effects: 2
- Edge cases: 21
- Data integrity: 3
- Happy:Failure ratio: 1:1.46

### 12.2 New Test Specifications

#### E2E-202: Reading a file records the read and unlocks an edit
- **Category:** Core Journey | **Scenario:** SC-201 | **Requirements:** FR-205, FR-208
- **Preconditions:** `/a.txt` holds `one\ntwo\nthree\n`; a fresh session for `ALICE`.
- **Steps:**
  - Given a fresh session in which `/a.txt` has never been read
  - When `fs.edit` is attempted with `old_string` `two` and `new_string` `2`
  - Then the call fails with `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And when `fs.read` is called for `/a.txt` and then the same `fs.edit` is retried
  - Then the edit succeeds with `applied` true
  - And `fs.read` of `/a.txt` now returns content containing `2` and not `two`
- **Cleanup:** restore `/a.txt`
- **Priority:** Critical

#### E2E-203: limit_lines above the server cap is silently capped
- **Category:** Error | **Scenario:** SC-201 | **Requirements:** FR-204
- **Preconditions:** `/big.txt` with 5000 lines; `safety.max_read_lines` set to 100.
- **Steps:**
  - Given that configuration
  - When `fs.read` is called with `limit_lines` 4000
  - Then the call succeeds, not errors
  - And `content` holds exactly 100 numbered lines
  - And `total_lines` equals 5000, `truncated` is true and `next_offset` equals 100
- **Cleanup:** none
- **Priority:** High

#### E2E-204: Listing and grepping do not satisfy the read guard
- **Category:** Security | **Scenario:** SC-201 | **Requirements:** FR-205
- **Preconditions:** fresh session; `/a.txt` exists.
- **Steps:**
  - Given a fresh session
  - When `fs.list_dir` on `/`, `fs.glob` with `*.txt`, `fs.grep` for `two` and `fs.stat` on `/a.txt` are all called successfully
  - Then a subsequent `fs.edit` on `/a.txt` still fails with `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And after `fs.read` of `/a.txt` the same edit succeeds
- **Cleanup:** restore `/a.txt`
- **Priority:** Critical

#### E2E-206: An offset past the end returns an empty window, not an error
- **Category:** Edge | **Scenario:** SC-201 | **Requirements:** FR-204
- **Preconditions:** `/a.txt` with 3 lines.
- **Steps:**
  - Given `/a.txt`
  - When `fs.read` is called with `offset_lines` 999
  - Then the call succeeds with `content` equal to the empty string
  - And `total_lines` equals 3, `truncated` is false and `next_offset` is null
- **Cleanup:** none
- **Priority:** High

#### E2E-207: Paging with next_offset walks the whole file exactly once
- **Category:** Edge | **Scenario:** SC-201 | **Requirements:** FR-204
- **Preconditions:** `/big.txt` with 5000 lines numbered `line-1` to `line-5000`.
- **Steps:**
  - Given `limit_lines` 1000
  - When `fs.read` is called repeatedly, each time with `offset_lines` set to the previous `next_offset`, until `truncated` is false
  - Then exactly 5 calls are made
  - And the concatenated content contains `line-1` and `line-5000` exactly once each
  - And the final response has `next_offset` null
- **Cleanup:** none
- **Priority:** High

#### E2E-213: Overwriting a file never read in this session is refused
- **Category:** Security | **Scenario:** SC-202 | **Requirements:** FR-210
- **Preconditions:** `/a.txt` exists; fresh session.
- **Steps:**
  - Given a session that has not read `/a.txt`
  - When `fs.write` is called with `path` `/a.txt`, `content` `clobbered` and `overwrite` true
  - Then the call fails with `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And `fs.read` of `/a.txt` still returns `one\ntwo\nthree`
  - And after an explicit `fs.read`, the same write succeeds and `overwritten` is true
- **Cleanup:** restore `/a.txt`
- **Priority:** Critical

#### E2E-216: A write refused by the quota changes nothing
- **Category:** Error | **Scenario:** SC-202 | **Requirements:** FR-202, FR-213
- **Preconditions:** `safety.write_quota_bytes` set to 50; a fresh session.
- **Steps:**
  - Given a session that has already written 40 bytes
  - When `fs.write` is called with 20 bytes of new content at `/quota.txt`
  - Then the call fails with `ERR_WRITE_QUOTA_EXCEEDED`
  - And `fs.exists` on `/quota.txt` reports that it does not exist
  - And `fs.audit_log` contains no entry for `/quota.txt`
- **Cleanup:** none
- **Priority:** Critical

#### E2E-217: A fresh write needs no prior read, an overwrite does
- **Category:** Error | **Scenario:** SC-202 | **Requirements:** FR-210
- **Preconditions:** fresh session; `/new.txt` does not exist.
- **Steps:**
  - Given a session that has read nothing
  - When `fs.write` is called at `/new.txt` with `content` `hello`
  - Then the call succeeds with `overwritten` false
  - And an immediate second `fs.write` at the same path with `overwrite` true also succeeds, because the first write recorded the read
  - And `bytes_written` equals 5 on the first call
- **Cleanup:** delete `/new.txt`
- **Priority:** High

#### E2E-218: Append creates only when asked
- **Category:** Edge | **Scenario:** SC-202 | **Requirements:** FR-209, FR-212
- **Preconditions:** `/missing.txt` does not exist.
- **Steps:**
  - Given that path
  - When `fs.append` is called with `content` `x` and `create` false
  - Then the call fails with `ERR_NOT_FOUND`
  - And when the same call is made with `create` true, it succeeds with `bytes_appended` 1
  - And `fs.read` returns `x`
- **Cleanup:** delete `/missing.txt`
- **Priority:** Medium

#### E2E-219: A write creates parents and leaves one audit entry
- **Category:** Side Effect | **Scenario:** SC-202 | **Requirements:** FR-202, FR-211
- **Preconditions:** `/deep` does not exist; fresh session.
- **Steps:**
  - Given `create_parents` true
  - When `fs.write` is called at `/deep/nested/file.txt` with `content` `hi`
  - Then the call succeeds
  - And `fs.list_dir` on `/deep` reports one entry `nested` of kind `dir`
  - And `fs.audit_log` contains exactly one entry with `op` `write`, `path` `/deep/nested/file.txt` and `detail` `2 bytes`
- **Cleanup:** delete `/deep` recursively
- **Priority:** High

#### E2E-225: A dry run enforces the read guard
- **Category:** Error | **Scenario:** SC-203 | **Requirements:** FR-215
- **Preconditions:** `/a.txt` exists; fresh session.
- **Steps:**
  - Given a session that has not read `/a.txt`
  - When `fs.edit` is called with `dry_run` true, `old_string` `two`, `new_string` `2`
  - Then the call fails with `ERR_EDIT_WITHOUT_PRIOR_READ`, proving a preview is not a way around the guard
- **Cleanup:** none
- **Priority:** High

#### E2E-226: A dry run on a non-matching string fails exactly like a real edit
- **Category:** Error | **Scenario:** SC-203 | **Requirements:** FR-215
- **Preconditions:** `/a.txt` read in this session.
- **Steps:**
  - Given `/a.txt` containing no occurrence of `zzz`
  - When `fs.edit` is called with `dry_run` true and `old_string` `zzz`
  - Then the call fails with `ERR_NO_MATCH` and the message `old_string not found in '/a.txt'`
  - And `fs.read` shows `/a.txt` unchanged
- **Cleanup:** none
- **Priority:** High

#### E2E-227: replace_all rewrites every occurrence and reports the count in the ambiguity error
- **Category:** Edge | **Scenario:** SC-203 | **Requirements:** FR-214
- **Preconditions:** `/dup.txt` containing `x\nx\nx\n`, read in this session.
- **Steps:**
  - Given three occurrences of `x`
  - When `fs.edit` is called with `old_string` `x`, `new_string` `y` and `replace_all` false
  - Then the call fails with `ERR_AMBIGUOUS_MATCH` and the message `old_string matches 3 sites in '/dup.txt' (use replace_all)`
  - And when the same call is made with `replace_all` true, it succeeds
  - And `fs.read` returns `y\ny\ny`
- **Cleanup:** restore `/dup.txt`
- **Priority:** High

#### E2E-228: A dry run returns a non-empty diff and leaves the file byte-identical
- **Category:** Edge | **Scenario:** SC-203 | **Requirements:** FR-215
- **Preconditions:** `/a.txt` read in this session.
- **Steps:**
  - Given `fs.hash` of `/a.txt` recorded as `H` before the call
  - When `fs.edit` is called with `dry_run` true, `old_string` `two`, `new_string` `2`
  - Then `applied` is false
  - And `diff` contains the lines `-two` and `+2`
  - And `fs.hash` of `/a.txt` still equals `H`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-231: A multi-edit whose last edit fails writes nothing
- **Category:** Data Integrity | **Scenario:** SC-204 | **Requirements:** FR-216
- **Preconditions:** `/a.txt` containing `one\ntwo\nthree\n`, read in this session.
- **Steps:**
  - Given the edits `[{old_string:"one",new_string:"1"},{old_string:"NOPE",new_string:"x"}]`
  - When `fs.multi_edit` is called
  - Then the call fails with `ERR_NO_MATCH`
  - And `fs.read` of `/a.txt` still returns `one\ntwo\nthree`, proving the first edit was not committed
  - And `fs.audit_log` contains no `multi_edit` entry
- **Cleanup:** none
- **Priority:** Critical

#### E2E-232: Edits compose in order against the intermediate text
- **Category:** Error | **Scenario:** SC-204 | **Requirements:** FR-216
- **Preconditions:** `/a.txt` containing `one\ntwo\nthree\n`, read in this session.
- **Steps:**
  - Given the edits `[{old_string:"one",new_string:"two"},{old_string:"two",new_string:"2",replace_all:false}]`
  - When `fs.multi_edit` is called
  - Then the call fails with `ERR_AMBIGUOUS_MATCH`, because the first edit produced a second occurrence of `two`
  - And `/a.txt` is unchanged
- **Cleanup:** none
- **Priority:** High

#### E2E-233: An empty edits array is a no-op that still reports success
- **Category:** Edge | **Scenario:** SC-204 | **Requirements:** FR-216
- **Preconditions:** `/a.txt` read in this session.
- **Steps:**
  - Given an empty `edits` array
  - When `fs.multi_edit` is called with `dry_run` false
  - Then the call succeeds with `edits` equal to 0
  - And `diff` is the empty string
  - And `fs.hash` of `/a.txt` is unchanged
- **Cleanup:** none
- **Priority:** Medium

#### E2E-234: The multi-edit diff compares original to final, not step by step
- **Category:** Edge | **Scenario:** SC-204 | **Requirements:** FR-216
- **Preconditions:** `/a.txt` containing `one\ntwo\nthree\n`, read in this session.
- **Steps:**
  - Given the edits `[{old_string:"one",new_string:"X"},{old_string:"X",new_string:"1"}]`
  - When `fs.multi_edit` is called
  - Then the call succeeds with `edits` equal to 2
  - And `diff` contains `-one` and `+1`
  - And `diff` contains no line mentioning the intermediate value `X`
- **Cleanup:** restore `/a.txt`
- **Priority:** Medium

#### E2E-237: A block below the similarity threshold is refused
- **Category:** Error | **Scenario:** SC-205 | **Requirements:** FR-217
- **Preconditions:** `/a.txt` containing `one\ntwo\nthree\n`, read in this session.
- **Steps:**
  - Given a `search_block` of `zzzzzzzzzz\nyyyyyyyyyy\n`, sharing almost no characters with any window of the file
  - When `fs.search_replace` is called with `fuzzy` true
  - Then the call fails with `ERR_NO_MATCH` and the message `no fuzzy match for search_block in '/a.txt'`
  - And `/a.txt` is unchanged
- **Cleanup:** none
- **Priority:** High

#### E2E-239: A near-miss block above the threshold is accepted and normalized
- **Category:** Edge | **Scenario:** SC-205 | **Requirements:** FR-217
- **Preconditions:** `/a.txt` containing `one\ntwo\nthree\n`, read in this session.
- **Steps:**
  - Given a `search_block` of `two\n` misspelled as `tWo\n`, whose similarity exceeds 0.6
  - When `fs.search_replace` is called with `fuzzy` true and `replace_block` `2` with **no** trailing newline
  - Then the call succeeds
  - And `fs.read` returns `one\n2\nthree`, showing the replacement was newline-terminated automatically
- **Cleanup:** restore `/a.txt`
- **Priority:** High

#### E2E-240: Exact mode ignores the threshold entirely
- **Category:** Edge | **Scenario:** SC-205 | **Requirements:** FR-217
- **Preconditions:** `/a.txt` read in this session.
- **Steps:**
  - Given the `search_block` `tWo\n`, which differs from the file only in case
  - When `fs.search_replace` is called with `fuzzy` false
  - Then the call fails with `ERR_NO_MATCH`, proving exact mode does no similarity matching
- **Cleanup:** none
- **Priority:** Medium

#### E2E-243: A patch whose update targets an unread file fails after earlier operations committed
- **Category:** Error | **Scenario:** SC-206 | **Requirements:** FR-219
- **Preconditions:** fresh session; `/a.txt` exists and has **not** been read.
- **Steps:**
  - Given a patch that first adds `/added.txt` and then updates `/a.txt`
  - When `fs.apply_patch` is called
  - Then the call fails with `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And `fs.exists` on `/added.txt` reports that it **does** exist, documenting that the patch is not transactional
  - And `/a.txt` is unchanged
- **Cleanup:** delete `/added.txt`
- **Priority:** Critical

#### E2E-244: An updated file receives two audit entries
- **Category:** Data Integrity | **Scenario:** SC-206 | **Requirements:** FR-220
- **Preconditions:** `/a.txt` read in this session; a fresh audit log.
- **Steps:**
  - Given a patch containing a single `*** Update File: /a.txt` operation
  - When `fs.apply_patch` is called and `fs.audit_log` is then read
  - Then exactly two entries have `op` equal to `apply_patch` and `path` equal to `/a.txt`
  - And a patch containing a single `*** Add File: /added.txt` operation produces exactly one entry for that path
- **Cleanup:** restore `/a.txt`, delete `/added.txt`
- **Priority:** High

#### E2E-245: An update carrying Move to relocates the patched content
- **Category:** Edge | **Scenario:** SC-206 | **Requirements:** FR-219
- **Preconditions:** `/a.txt` read in this session.
- **Steps:**
  - Given a patch with `*** Update File: /a.txt` followed by `*** Move to: /moved.txt` and a hunk changing `two` to `2`
  - When `fs.apply_patch` is called
  - Then the result entry for that file carries `op` `update` and `moved_to` `/moved.txt`
  - And `fs.exists` on `/a.txt` reports absence
  - And `fs.read` of `/moved.txt` returns content containing `2`
- **Cleanup:** move `/moved.txt` back to `/a.txt`
- **Priority:** High

#### E2E-246: Each patch operation normalizes its own path
- **Category:** Edge | **Scenario:** SC-206 | **Requirements:** FR-219
- **Preconditions:** fresh session.
- **Steps:**
  - Given a patch adding a file at the path `/../escape.txt`
  - When `fs.apply_patch` is called
  - Then the call fails with `ERR_PATH_OUT_OF_BOUNDS`
  - And when the patch instead adds at `/dir/./sub/../file.txt`, the call succeeds and `fs.exists` reports the file at the normalized path `/dir/file.txt`
- **Cleanup:** delete `/dir` recursively
- **Priority:** High

#### E2E-247: An add operation does not require a prior read
- **Category:** Edge | **Scenario:** SC-206 | **Requirements:** FR-220
- **Preconditions:** fresh session in which nothing has been read.
- **Steps:**
  - Given a patch containing only `*** Add File: /fresh.txt`
  - When `fs.apply_patch` is called
  - Then the call succeeds
  - And `fs.read` of `/fresh.txt` returns the added content
- **Cleanup:** delete `/fresh.txt`
- **Priority:** Medium

#### E2E-252: A tree deeper than max_depth stops at the limit
- **Category:** Error | **Scenario:** SC-207 | **Requirements:** FR-222
- **Preconditions:** a chain `/d1/d2/d3/d4/leaf.txt`.
- **Steps:**
  - Given `max_depth` 2
  - When `fs.tree` is called on `/`
  - Then the returned tree contains `d1` and its child `d2`
  - And no node named `d4` or `leaf.txt` appears anywhere in the result
- **Cleanup:** delete `/d1` recursively
- **Priority:** High

#### E2E-253: Probes leave the session untouched
- **Category:** Security | **Scenario:** SC-207 | **Requirements:** FR-227, FR-228
- **Preconditions:** fresh session; `/a.txt` exists.
- **Steps:**
  - Given a fresh session
  - When `fs.stat`, `fs.exists`, `fs.hash` and `fs.count_lines` are each called on `/a.txt`
  - Then every call succeeds
  - And `fs.audit_log` returns an empty entry list, proving no probe is audited
  - And a subsequent `fs.edit` on `/a.txt` still fails with `ERR_EDIT_WITHOUT_PRIOR_READ`
  - And the `fs.stat` result reports `uid` 1000 and `gid` 1000
- **Cleanup:** none
- **Priority:** Critical

#### E2E-254: The hash allow-list is closed and defaults to sha256
- **Category:** Edge | **Scenario:** SC-207 | **Requirements:** FR-221, FR-229
- **Preconditions:** `/a.txt` exists.
- **Steps:**
  - Given `/a.txt`
  - When `fs.hash` is called with no `algo`
  - Then the digest returned equals the one returned for `algo` `sha256`
  - And calls with `md5`, `sha1` and `sha512` all succeed with digests of 32, 40 and 128 hexadecimal characters respectively
  - And a call with `algo` `crc32` fails with `ERR_INVALID_ARGUMENT`
- **Cleanup:** none
- **Priority:** Medium

#### E2E-255: The tree shows the trash directory while the walk hides it
- **Category:** Edge | **Scenario:** SC-207 | **Requirements:** FR-222
- **Preconditions:** a file has been deleted so that `/.mcp_trash` exists and holds one entry.
- **Steps:**
  - Given `/.mcp_trash` populated by a soft delete
  - When `fs.tree` is called on `/` with sufficient depth
  - Then the tree contains a node named `.mcp_trash`
  - And when `fs.glob` is called with pattern `*` , no returned path begins with `/.mcp_trash`
- **Cleanup:** none
- **Priority:** High

#### E2E-256: Hidden entries appear only when requested
- **Category:** Edge | **Scenario:** SC-207 | **Requirements:** FR-227, FR-228
- **Preconditions:** `/.hidden` and `/a.txt` both exist.
- **Steps:**
  - Given both files at the volume root
  - When `fs.list_dir` is called on `/` with `include_hidden` false
  - Then `entries` contains `a.txt` and does not contain `.hidden`
  - And when called with `include_hidden` true, `entries` contains both
  - And `total` differs by exactly one between the two calls
- **Cleanup:** none
- **Priority:** Medium

#### E2E-259: A walk stops at the file ceiling
- **Category:** Error | **Scenario:** SC-208 | **Requirements:** FR-223
- **Preconditions:** a volume seeded with 5100 files matching `*.tmp`.
- **Steps:**
  - Given more files than the 5000-file ceiling
  - When `fs.grep` is called with a pattern matching every file
  - Then the call completes without error
  - And the number of distinct paths reported does not exceed 5000
- **Cleanup:** delete the seeded files
- **Priority:** High

#### E2E-262: Excluded directories are skipped by the walk
- **Category:** Edge | **Scenario:** SC-208 | **Requirements:** FR-223
- **Preconditions:** files at `/node_modules/x.js`, `/target/y.rs`, `/.git/config` and `/src/app.py`.
- **Steps:**
  - Given those files
  - When `fs.glob` is called with pattern `*`
  - Then `/src/app.py` appears in `matches`
  - And no match begins with `/node_modules`, `/target` or `/.git`
- **Cleanup:** none
- **Priority:** High

#### E2E-263: A single star crosses a slash boundary
- **Category:** Edge | **Scenario:** SC-208 | **Requirements:** FR-224
- **Preconditions:** `/src/nested/deep.rs` exists.
- **Steps:**
  - Given that path
  - When `fs.glob` is called with pattern `*.rs`
  - Then `matches` contains `/src/nested/deep.rs`, confirming Python `fnmatch` semantics rather than path-segment-aware matching
- **Cleanup:** none
- **Priority:** Critical

#### E2E-264: Glob results are newest first and capped at 100
- **Category:** Edge | **Scenario:** SC-208 | **Requirements:** FR-224
- **Preconditions:** 120 files matching `page*.md`, written in ascending order so mtimes increase.
- **Steps:**
  - Given 120 matching files
  - When `fs.glob` is called with pattern `page*.md`
  - Then `matches` holds exactly 100 entries and `truncated` is true
  - And the first entry is the most recently written file
  - And the last-written file appears before the first-written one
- **Cleanup:** delete the seeded files
- **Priority:** High

#### E2E-265: An unrecognized output_mode behaves as content
- **Category:** Edge | **Scenario:** SC-208 | **Requirements:** FR-225
- **Preconditions:** `/src/app.py` containing the line `    total = 1`.
- **Steps:**
  - Given that file
  - When `fs.grep` is called with pattern `total` and `output_mode` `banana`
  - Then the result has the keys `matches` and `truncated`, the `content` shape
  - And it has neither a `files` key nor a `count` key
- **Cleanup:** none
- **Priority:** Medium

#### E2E-268: A symbol search in an unsupported language returns empty, not an error
- **Category:** Error | **Scenario:** SC-209 | **Requirements:** FR-226
- **Preconditions:** a file `/notes.xyz` containing the text `def hello():`.
- **Steps:**
  - Given a file whose extension maps to no known language
  - When `fs.find_definition` is called with `name` `hello`
  - Then the call succeeds
  - And `definitions` is an empty array
- **Cleanup:** delete `/notes.xyz`
- **Priority:** Medium

#### E2E-269: The root parameter scopes the symbol search
- **Category:** Edge | **Scenario:** SC-209 | **Requirements:** FR-226
- **Preconditions:** `/src/app.py` and `/other/app.py` both defining `hello`.
- **Steps:**
  - Given both files
  - When `fs.find_definition` is called with `name` `hello` and `root` `/src`
  - Then every entry in `definitions` has a `path` beginning with `/src`
  - And no entry references `/other/app.py`
- **Cleanup:** delete `/other` recursively
- **Priority:** Medium

#### E2E-272: A soft delete leaves the content retrievable under the trash path
- **Category:** Side Effect | **Scenario:** SC-210 | **Requirements:** FR-230
- **Preconditions:** `/gone.txt` containing `keepme`.
- **Steps:**
  - Given that file
  - When `fs.delete` is called with `trash` true
  - Then the result has `trashed` true and a `trash_path` matching `^/\.mcp_trash/\d+__gone\.txt$`
  - And `fs.exists` on `/gone.txt` reports absence
  - And `fs.read` of the returned `trash_path` returns `keepme`
  - And `fs.audit_log` contains an entry with `op` `delete` and `detail` equal to `-> ` followed by the trash path
- **Cleanup:** hard-delete the trash entry if hard delete is enabled, otherwise leave it
- **Priority:** Critical

#### E2E-276: Hard delete works when explicitly enabled
- **Category:** Edge | **Scenario:** SC-210 | **Requirements:** FR-230, FR-231
- **Preconditions:** a server started with `safety.allow_hard_delete: true`; `/gone.txt` exists.
- **Steps:**
  - Given hard delete enabled
  - When `fs.delete` is called with `trash` false
  - Then the result has `trashed` false and `trash_path` null
  - And `fs.exists` on `/gone.txt` reports absence
  - And `fs.glob` with pattern `*gone*` returns no match anywhere, including under the trash directory
  - And the audit entry for the operation has `detail` equal to `hard`
- **Cleanup:** stop the server
- **Priority:** High

#### E2E-277: Deleting a missing directory reports not-found, not the recursive hint
- **Category:** Edge | **Scenario:** SC-210 | **Requirements:** FR-231, FR-232
- **Preconditions:** `/nodir` does not exist; `/realdir` exists and holds a file.
- **Steps:**
  - Given those paths
  - When `fs.delete` is called on `/nodir` with `recursive` false
  - Then the call fails with `ERR_NOT_FOUND` and the message `'/nodir' does not exist`
  - And when `fs.delete` is called on `/realdir` with `recursive` false
  - Then the call fails with `ERR_INVALID_ARGUMENT` and the message `'/realdir' is a directory (pass recursive=true)`
- **Cleanup:** delete `/realdir` recursively
- **Priority:** Medium

#### E2E-280: A negative byte offset is clamped rather than rejected
- **Category:** Error | **Scenario:** SC-211 | **Requirements:** FR-207
- **Preconditions:** `/bin.dat` holding exactly 4 bytes.
- **Steps:**
  - Given that file
  - When `fs.read_bytes` is called with `offset_bytes` -10 and `length_bytes` -5
  - Then the call succeeds rather than failing
  - And `length` equals 0 and `base64` is the empty string
- **Cleanup:** none
- **Priority:** Medium

#### E2E-281: Session tools still enforce membership
- **Category:** Security | **Scenario:** SC-211 | **Requirements:** FR-234
- **Preconditions:** `BOB` is not a member of `spec-fs`.
- **Steps:**
  - Given `BOB`'s bearer
  - When `fs.list_allowed_roots` is called with `mount_id` `spec-fs`
  - Then the call fails with `ERR_FORBIDDEN`
  - And `fs.audit_log` for the same mount also fails with `ERR_FORBIDDEN`, proving both pass the gate despite opening no volume
- **Cleanup:** none
- **Priority:** High

#### E2E-282: Reading bytes records a read and reports the MIME type
- **Category:** Edge | **Scenario:** SC-211 | **Requirements:** FR-207
- **Preconditions:** fresh session; `/bin.dat` with 4 bytes; `/a.txt` exists.
- **Steps:**
  - Given a fresh session
  - When `fs.read_bytes` is called on `/a.txt`
  - Then `mime_type` equals `text/plain`
  - And a subsequent `fs.edit` on `/a.txt` succeeds, proving a byte read satisfies the guard
  - And `fs.read_bytes` on `/bin.dat` returns `mime_type` `application/octet-stream` and `length` 4
- **Cleanup:** restore `/a.txt`
- **Priority:** High

#### E2E-283: The audit log is capped and ordered oldest first
- **Category:** Edge | **Scenario:** SC-211 | **Requirements:** FR-234
- **Preconditions:** a fresh session.
- **Steps:**
  - Given 3 writes performed in order at `/log1.txt`, `/log2.txt`, `/log3.txt`
  - When `fs.audit_log` is called with `limit` 20
  - Then the entries appear in that write order
  - And every entry carries the keys `timestamp`, `op`, `path` and `detail`
  - And calling with `limit` 2 returns at most 2 entries
- **Cleanup:** delete the three files
- **Priority:** Medium

#### E2E-284: Batch read isolates one bad path among good ones
- **Category:** Core Journey | **Scenario:** SC-212 | **Requirements:** FR-201, FR-203, FR-206
- **Preconditions:** `/a.txt` exists; `/nope.txt` does not.
- **Steps:**
  - Given the request `paths: ["/a.txt", "/nope.txt", "/src/app.py"]`
  - When `fs.read_many` is called
  - Then `files` holds exactly 3 entries in that order
  - And entry 1 has `path` `/a.txt` with `content` and `truncated` keys
  - And entry 2 has `path` `/nope.txt` and `error` equal to `not found: /nope.txt`
  - And entry 3 has `path` `/src/app.py` with content
- **Cleanup:** none
- **Priority:** Critical

#### E2E-286: A path failing normalization keeps its raw spelling
- **Category:** Error | **Scenario:** SC-212 | **Requirements:** FR-201, FR-206
- **Preconditions:** `/a.txt` exists.
- **Steps:**
  - Given the request `paths: ["/a.txt", "/../escape.txt"]`
  - When `fs.read_many` is called
  - Then entry 2 has `path` exactly `/../escape.txt`, the raw spelling rather than a normalized form
  - And its `error` begins with `ERR_PATH_OUT_OF_BOUNDS: `
  - And entry 1 still carries the content of `/a.txt`
- **Cleanup:** none
- **Priority:** Critical

#### E2E-287: Result key sets match the frozen contract
- **Category:** Error | **Scenario:** SC-212 | **Requirements:** FR-203
- **Preconditions:** `/a.txt` read in this session.
- **Steps:**
  - Given a successful call to each of `fs.append`, `fs.insert_at_line`, `fs.multi_edit` and `fs.delete`
  - When each result is parsed
  - Then `fs.append` yields exactly the keys `path` and `bytes_appended`
  - And `fs.insert_at_line` yields exactly `path`, `applied` and `line`
  - And `fs.multi_edit` yields exactly `path`, `applied`, `edits` and `diff`
  - And `fs.delete` yields exactly `path`, `trashed` and `trash_path`
- **Cleanup:** restore the fixtures
- **Priority:** High

#### E2E-288: Per-file caps truncate without failing the batch
- **Category:** Error | **Scenario:** SC-212 | **Requirements:** FR-206, FR-218
- **Preconditions:** `/big.txt` with 5000 lines; `/a.txt` with 3.
- **Steps:**
  - Given the request `paths: ["/big.txt", "/a.txt"]` with `per_file_cap_lines` 10
  - When `fs.read_many` is called
  - Then entry 1 has `truncated` true and content holding exactly 10 numbered lines
  - And entry 2 has `truncated` false and content holding 3 lines
- **Cleanup:** none
- **Priority:** High

#### E2E-289: An empty paths array yields an empty result, not an error
- **Category:** Edge | **Scenario:** SC-212 | **Requirements:** FR-201, FR-206
- **Steps:**
  - Given the request `paths: []`
  - When `fs.read_many` is called
  - Then the call succeeds and `files` is an empty array
- **Cleanup:** none
- **Priority:** High

#### E2E-290: Insertion before line 1 and past the end both behave predictably
- **Category:** Edge | **Scenario:** SC-212 | **Requirements:** FR-203, FR-218
- **Preconditions:** `/a.txt` containing `one\ntwo\nthree\n`, read in this session.
- **Steps:**
  - Given that file
  - When `fs.insert_at_line` is called with `line` 1 and `content` `zero`
  - Then the result equals `{"path":"/a.txt","applied":true,"line":1}`
  - And `fs.read` returns content beginning with `zero` followed by `one`
  - And when `fs.insert_at_line` is called with `line` 9999 and `content` `last`, `fs.read` returns content ending with `last`
- **Cleanup:** restore `/a.txt`
- **Priority:** Medium

### 12.3 Modified Test Specifications

None.

### 12.4 Removed Tests

None.

## 13. Consistency Notes

1. **Tool count.** S1 §5 SC-001 states the registry holds 45 always-on tools, 35 `fs.*` plus 10 `admin.*`. S2 specifies 32 of those 35; the remaining three are S5's. There is no disagreement, only a scope split, recorded here so the audit does not read it as a contradiction.
2. **The safety layer's end-to-end exercise.** S1 §3.2 said the safety layer would be verified end to end here. §12 discharges that: E2E-204 and E2E-213 exercise the read guard through real tool calls, E2E-216 the quota, E2E-272 the trash derivation.
3. **Two MIME implementations.** `AGENTS.md` and `docs/mime.rs:1-3` both describe `docs/mime.rs` as the guesser used by `fs.read_bytes`. That is not what the code does: the engine calls its own `mime_guess` (`core/fs_ops.rs:1286`) from `fs_ops.rs:92`, and `docs::guess_mime` serves the REST download route instead (S3 FR-313). The two tables were compared during the cross-spec audit and hold identical content, so no behaviour differs today, but the documentation comment at `docs/mime.rs:2` is inaccurate. Recorded as TBD-202.

## 14. Migration & Implementation Notes

No production code change. Ordering for the test work:

1. **Annotate existing tests** with their `E2E-2xx` ids, in `core/fs_ops.rs`, `core/diff.rs`, `util/text.rs` and the five functional scenario scripts named in §9.3.
2. **Add the engine-level new tests** that need no server: E2E-233, E2E-234, E2E-240, E2E-263, E2E-265, E2E-289.
3. **Extend the functional scenarios** with the remaining new tests, grouped by the script that already owns the subject: reads into `06_read_advanced.sh`, writes and deletes into `02_fs_crud.sh`, edits and patches into `03_fs_edit.sh`, listing and search into `04_fs_tree.sh`.
4. **E2E-203, E2E-216, E2E-259, E2E-264 and E2E-276 need non-default configuration** (a lowered `max_read_lines`, a lowered `write_quota_bytes`, a large seeded volume, `allow_hard_delete` true). Each starts its own server rather than mutating the shared fixture, so scenario order stays irrelevant.
5. **Seed fixtures once** per scenario script; E2E-259 and E2E-264 seed thousands of files and belong in their own script so they do not slow the others.

## 15. Open Questions & TBDs

- **TBD-201:** `fs.apply_patch` is not transactional (FR-219). A failure partway leaves earlier operations committed, which E2E-243 pins as current behaviour. Whether to make it all-or-nothing like `fs.multi_edit` is a product decision, not specified here.
- **TBD-202:** Two MIME guessers exist and both are live: `core/fs_ops.rs:1286` serves `fs.read_bytes`, `docs/mime.rs` serves the REST download route. The audit established that their tables are identical, 32 extensions with the same values, so they agree today; nothing keeps them in step, and the doc comment at `docs/mime.rs:2` names the wrong caller. Whether to collapse them into one table is an open decision. E2E-548 in S5 detects future divergence.
- **TBD-203:** `fs.grep` falls through to `content` for any unrecognized `output_mode` (FR-225, E2E-265). Whether an unknown mode ought to be `ERR_INVALID_ARGUMENT` instead is undecided; current behaviour is specified as-built.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| **Window** | The slice of lines returned by a paged read, with its truncation flag and next offset. | Engine |
| **next_offset** | The `offset_lines` value that continues a truncated read where it stopped. | Engine |
| **Read guard** | The S1 rule that a write to a path not read in this session is refused. | Session |
| **No-clobber** | The default refusal to overwrite an existing path unless `overwrite` is true. | Engine |
| **Unique match** | The editing rule requiring `old_string` to occur exactly once unless `replace_all` is set. | Engine |
| **Dry run** | An edit that validates and produces its diff without writing. | Engine |
| **Unified diff** | The `--- / +++ / @@` text returned by every editing tool. | Engine |
| **Fuzzy replace** | Block replacement accepting the most similar window at a similarity of at least 0.6. | Engine |
| **Similarity ratio** | The LCS-based score in `0.0..=1.0` comparing a candidate window to the search block. | Engine |
| **V4A patch** | The multi-file patch dialect delimited by `*** Begin Patch` and `*** End Patch`. | Patch |
| **Hunk** | One `@@`-introduced group of context, removed and added lines inside an update. | Patch |
| **Walk** | The bounded traversal of a volume used by glob, grep and symbol search. | Engine |
| **Glob cap** | The 100-path ceiling on `fs.glob` results. | Engine |
| **Tree cap** | The 2000-node ceiling on `fs.tree`. | Engine |
| **Trash** | The soft-delete destination derived by the S1 safety layer. | Session |
| **Probe** | A read-only operation that neither mutates nor records a read: stat, exists, hash, listing, search. | Engine |

## 17. Interview Decisions Log

Produced non-interactively from the code at the user's instruction. These are the scoping decisions taken.

- **DEC-201:** S2 covers 32 `fs.*` tools; the three document tools stay with S5. **Rationale:** they depend on the extraction engine, the docx writer and the external document service, none of which exists in this layer; splitting on the dependency rather than on the name prefix keeps each spec self-contained. **Alternatives considered:** covering all 35 here and deferring only the engines. **Implemented by:** §3.2 scope statement. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/tools/document.rs:1-7`.
- **DEC-202:** The symbol tools are specified here but their matcher is S5's. **Rationale:** `fs.find_definition` and `fs.find_references` are walk-based fs operations whose gates and result shapes belong with the other search tools; the tree-sitter grammars and lexical fallback are a document-domain engine. **Alternatives considered:** moving both tools wholesale to S5. **Implemented by:** FR-226. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/core/fs_ops.rs:1374-1396` calls into `crate::docs::symbols`.
- **DEC-203:** The duplicate audit entry on patched updates is specified and frozen rather than reported as a defect. **Rationale:** `fs.audit_log` output is observable contract; silently fixing it would change what an agent sees. **Alternatives considered:** specifying one entry and treating the second as a bug. **Implemented by:** FR-220, E2E-244. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/core/fs_ops.rs:1534-1537,1546`.
- **DEC-204:** `fs.apply_patch`'s non-transactional behaviour is specified as-built and pinned by a test rather than corrected. **Rationale:** this is a retro-specification; changing the semantics is a product decision recorded as TBD-201. **Alternatives considered:** specifying all-or-nothing and registering the gap as drift. **Implemented by:** FR-219, E2E-243. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/core/fs_ops.rs:1511-1547`, a sequential loop with no rollback.
- **DEC-205:** Engine ceilings (5000 files, 100 globs, 2000 tree nodes, 0.6 similarity) are specified as contract, not as tuning. **Rationale:** they are observable through `truncated` flags and shape what an agent receives, so changing one changes the product. **Alternatives considered:** treating them as implementation detail outside the spec. **Implemented by:** FR-217, FR-222, FR-223, FR-224. **Round:** n/a. **Code evidence:** `crates/mcp-fs/src/core/fs_ops.rs:35-41`.
- **DEC-206:** The `2xx` id block is reserved for S2. **Rationale:** the eight specs will be audited together, and colliding `FR-001` ids across documents would make cross-references ambiguous. **Alternatives considered:** per-document numbering restarting at 001. **Implemented by:** the ID convention note in the header. **Round:** n/a. **Code evidence:** n/a.

## 18. Implementability Audit

Audited as part of the cross-spec audit of S1 through S8 on 2026-09-18; see
`specs/AUDIT.md` for the method, the full findings and the limitations. Sub-agent
execution was unavailable (provider budget error), so the contract's fresh-context
auditor was replaced by mechanical verification plus a targeted reading pass. That
substitution is weaker in one specific respect, recorded in `AUDIT.md`.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|-------|--------------------------|-------------------|---------|
| 1 | 0 | 1 (A-6 MIME duplication resolved with evidence) | IMPLEMENTABLE-WITH-DRIFT |

**Amendments applied:** TBD-202 rewritten from an open question into a stated fact
**Drift registered:** none. Every A finding was evident and amended in place, which
the contract prefers to a register entry.

## 19. Implementation Drift Register

Empty, and deliberately so: all A findings from the cross-spec audit were evident
corrections applied in place rather than drift to resolve during implementation.
See `specs/AUDIT.md` for each finding and its evidence.
