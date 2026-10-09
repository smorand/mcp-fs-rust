> Id: SPEC-0003
> Nature: FEAT
> Status: as-built
> Area: filesystem-engine
> Generated on: 2026-10-09 by /sdlc-retro-spec at base commit 27bc282acdfd2a5c972802e60ed0e7b566e37ab5
> Confidence: high

# Filesystem Engine and `fs.*` Tools

## 1. Summary

This document specifies the filesystem engine and the 32 `fs.*` tools that expose it: an LLM-facing simulated filesystem with read paging, precise editing, search, and safe mutation semantics. It is the product's reason to exist. The engine is the single implementation of every filesystem operation; every surface that reaches it (the tool layer and the REST data plane) is a thin adapter, never a second implementation.

## 2. Current State

### 2.1 Existing Specifications
None.

### 2.2 Current Architecture
None.

### 2.3 Relevant Context
The engine depends on the platform foundation's membership gate, path normalization, read guard, write quota, audit log, trash derivation, error vocabulary and content addressing. Those are out of scope here and are consumed as given.

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

Plus the engine mechanics: line windowing and truncation, the unified diff, unique-match editing, fuzzy block replacement, the multi-file patch format, the bounded volume walk, and glob/grep semantics.

### 3.2 Out of Scope

- The three document tools of the `fs.*` family (`fs.extract_text`, `fs.write_docx`, `fs.documentize`), which depend on the document extraction/conversion engine.
- The symbol extraction engine (tree-sitter grammars and lexical fallback) behind `fs.find_definition`/`fs.find_references`. This spec covers the two tools' gates, inputs and output shapes, and treats the matcher as a dependency.
- The documentation-trigger parameter of the byte-write tool.
- The REST data plane, a second adapter over this same engine.
- Everything owned by the platform foundation: authorization, normalization, quota, audit, trash derivation, error codes, protocol framing.
- Search indexing side effects of writes.

## 4. Actors

| Actor | Description |
|---|---|
| **LLM agent** | The primary consumer. Reads with paging, edits by unique string match, searches by glob and regex. Optimizes for token economy, so truncation and caps are part of the contract, not an implementation detail. |
| **Project member** | The authenticated human whose identity the agent acts under. Owns the session whose read set, write quota and audit log govern the mutations. |
| **Tool author** | A developer adding or changing a tool. Bound by the rule that operations live in the engine only. |

## 5. Usage Scenarios

### SC-001: Agent reads a large file in pages

**Actor:** LLM agent
**Preconditions:** the caller is a member of the project; the file exists and is text.
**Flow:**
1. Agent calls `fs.read` with `mount_id` and `path`, default `offset_lines` 0 and `limit_lines` 2000.
2. The engine reads the text, records the read against the session, and splits it into lines.
3. The window is capped by the lesser of `limit_lines` and the server's configured maximum read lines.
4. The content is returned line-numbered by default, with `total_lines`, `truncated` and `next_offset`.
5. Agent repeats with `offset_lines` set to `next_offset` until `truncated` is false.

**Postconditions:** the session has recorded a read for that path, which unlocks later edits under the read guard; the agent holds the full content across pages.
**Exceptions:**
- EXC-001a: path missing → `ERR_NOT_FOUND`
- EXC-001b: path is a directory → the storage layer's error surfaces
- EXC-001c: `limit_lines` above the server's configured maximum → silently capped, not an error
- EXC-001d: `offset_lines` past the end → empty content, `truncated` false

**Cross-scenario notes:** step 2's read record is the precondition for SC-003, SC-004 and SC-005.

### SC-002: Agent creates a file and appends to it

**Actor:** LLM agent
**Preconditions:** the caller is a member; the target path does not exist.
**Flow:**
1. Agent calls `fs.write` with `content`, default `overwrite` false and `create_parents` true.
2. The engine refuses if the path exists and `overwrite` is false.
3. Parent directories are created when requested.
4. The bytes are charged against the quota, written atomically, and audited.
5. Agent calls `fs.append` to add more content.

**Postconditions:** the file exists with exactly the written content; the session's write accounting has advanced; the audit log shows the operation.
**Exceptions:**
- EXC-002a: path exists and `overwrite` false → `ERR_NO_CLOBBER`, `"'{path}' exists (pass overwrite=true)"`
- EXC-002b: overwriting a file never read in this session → `ERR_EDIT_WITHOUT_PRIOR_READ`
- EXC-002c: the write would exceed the session quota → `ERR_WRITE_QUOTA_EXCEEDED`
- EXC-002d: `create_parents` false and the parent is missing → the storage layer refuses
- EXC-002e: `fs.append` on a missing file with `create` false → `ERR_NOT_FOUND`

### SC-003: Agent edits a file by unique string match

**Actor:** LLM agent
**Preconditions:** the file has been read in this session.
**Flow:**
1. Agent calls `fs.edit` with `old_string` and `new_string`, optionally `dry_run` true.
2. The engine enforces the read guard, then reads the current text.
3. Occurrences are counted; exactly one is required unless `replace_all` is set.
4. A unified diff of old against new is produced.
5. Unless `dry_run`, the result is committed through the shared write path.
6. The tool returns `path`, `applied` and `diff`.

**Postconditions:** with `dry_run` false the file holds the new text and `applied` is true; with `dry_run` true the file is untouched, `applied` is false, and the diff still shows what would change.
**Exceptions:**
- EXC-003a: `old_string` absent → `ERR_NO_MATCH`, `"old_string not found in '{path}'"`
- EXC-003b: several occurrences without `replace_all` → `ERR_AMBIGUOUS_MATCH`, `"old_string matches {count} sites in '{path}' (use replace_all)"`
- EXC-003c: the file was not read in this session → `ERR_EDIT_WITHOUT_PRIOR_READ`
- EXC-003d: the new content exceeds the quota → `ERR_WRITE_QUOTA_EXCEEDED`, and nothing is written

### SC-004: Agent applies several edits atomically

**Actor:** LLM agent
**Preconditions:** the file has been read in this session.
**Flow:**
1. Agent calls `fs.multi_edit` with an `edits` array of `{old_string, new_string, replace_all?}`.
2. Each edit is applied in order against the text produced by the previous one.
3. A single diff of the original against the final text is produced.
4. Unless `dry_run`, the final text is committed once.

**Postconditions:** either every edit applied and the file holds the final text, or nothing was written at all.
**Exceptions:**
- EXC-004a: any edit fails to match → the whole call fails and the file is unchanged
- EXC-004b: an edit that would have matched the original but not the intermediate text fails, because edits compose in order
- EXC-004c: an empty `edits` array produces an empty diff and `edits` 0

### SC-005: Agent replaces a multi-line block, optionally fuzzily

**Actor:** LLM agent
**Preconditions:** the file has been read in this session.
**Flow:**
1. Agent calls `fs.search_replace` with `search_block` and `replace_block`.
2. With `fuzzy` false the block must match exactly and uniquely.
3. With `fuzzy` true a window of the search block's line count slides over the file and the most similar candidate is kept.
4. The candidate is accepted only at a similarity of at least 0.6.
5. The replacement is committed and the diff returned.

**Postconditions:** the block is replaced; the replacement is newline-terminated whether or not the caller terminated it.
**Exceptions:**
- EXC-005a: no candidate reaches the threshold → `ERR_NO_MATCH`, `"no fuzzy match for search_block in '{path}'"`
- EXC-005b: exact mode with no match → `ERR_NO_MATCH`
- EXC-005c: exact mode with several matches → `ERR_AMBIGUOUS_MATCH`

### SC-006: Agent applies a multi-file patch

**Actor:** LLM agent
**Preconditions:** the caller is a member; files targeted by update or delete operations have been read in this session.
**Flow:**
1. Agent calls `fs.apply_patch` with patch text bracketed by begin/end markers.
2. The patch is parsed into file operations: add, update, delete, with an optional move modifier on an update.
3. Each operation normalizes its own path through the safety layer.
4. Adds charge the quota and write; updates enforce the read guard, apply hunks, charge and write; deletes enforce the read guard and remove.
5. The tool returns `files`, one entry per touched path.

**Postconditions:** every operation in the patch has been applied in order; an update carrying a move leaves the content at the new path.
**Exceptions:**
- EXC-006a: malformed patch text → the parse fails before anything is written
- EXC-006b: an update targeting a file not read in this session → `ERR_EDIT_WITHOUT_PRIOR_READ`, after earlier operations in the same patch have already been applied
- EXC-006c: a hunk that does not locate its context → the update fails
- EXC-006d: the quota is exhausted partway → earlier operations stand

### SC-007: Agent explores the volume structure

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.list_dir`, defaulting to `/`, optionally with sizes and a sort key.
2. Agent calls `fs.tree` for a recursive view to `max_depth` 3 by default.
3. Agent calls `fs.stat`, `fs.exists`, `fs.count_lines` and `fs.hash` on individual paths.

**Postconditions:** the agent holds the directory structure; none of these operations mutates anything or records a read.
**Exceptions:**
- EXC-007a: `fs.tree` exceeding 2000 nodes → the walk stops and `truncated` is reported
- EXC-007b: `fs.hash` with an unsupported algorithm → `ERR_INVALID_ARGUMENT`; only `md5`, `sha1`, `sha256` and `sha512` are allowed
- EXC-007c: `fs.exists` on a missing path → a successful result reporting absence, not an error

### SC-008: Agent searches the volume by name and by content

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.glob` with a pattern, optionally excluding more globs.
2. The walk skips the default excluded directories and matches against both the full path and the bare file name.
3. Results are sorted newest first and capped at 100, with `truncated` reported.
4. Agent calls `fs.grep` with `output_mode` `content`, `files` or `count`.

**Postconditions:** the agent holds the matching paths or matching lines; no read is recorded, so a subsequent edit still needs an explicit read.
**Exceptions:**
- EXC-008a: more than 100 glob matches → the list is capped and `truncated` is true
- EXC-008b: a walk exceeding 5000 files → the walk stops at the ceiling
- EXC-008c: an invalid regex with `regex` true → `ERR_INVALID_ARGUMENT`

### SC-009: Agent locates a symbol

**Actor:** LLM agent
**Preconditions:** the caller is a member; the volume holds source files.
**Flow:**
1. Agent calls `fs.find_definition` with a symbol `name`, optionally a `root` and a `kind` filter.
2. The engine walks the volume and runs the language-aware matcher on every file with a known language.
3. Agent calls `fs.find_references` for call sites.

**Postconditions:** the agent holds `definitions` with `path`, `name`, `kind` and `line`, or `references` with `path`, `line` and `kind`.
**Exceptions:**
- EXC-009a: no match → an empty array, not an error
- EXC-009b: a file in an unsupported language → skipped, not an error

### SC-010: Agent moves, copies and deletes

**Actor:** LLM agent
**Preconditions:** the caller is a member; the source exists.
**Flow:**
1. Agent calls `fs.copy`, no-clobber by default, `recursive` for trees.
2. Agent calls `fs.move` to rename or relocate, no-clobber by default.
3. Agent calls `fs.delete`, which moves to trash by default.
4. The trash destination is derived by the safety layer and its parent is created before the rename.

**Postconditions:** a trashed file is still present under the trash directory and is reported in `trash_path`; a copy shares its content blob with the source by refcount rather than duplicating bytes.
**Exceptions:**
- EXC-010a: deleting a directory without `recursive` → `ERR_INVALID_ARGUMENT`, `"'{path}' is a directory (pass recursive=true)"`
- EXC-010b: `trash` false while hard delete is disabled → `ERR_NOT_SUPPORTED`, `"hard delete disabled (server started without allow_hard_delete)"`
- EXC-010c: deleting a missing path → `ERR_NOT_FOUND`, `"'{path}' does not exist"`
- EXC-010d: destination exists without `overwrite` → `ERR_NO_CLOBBER`

### SC-011: Agent reads binary content and inspects the session

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.read_bytes` for a byte range, default 65536 bytes from offset 0.
2. The engine returns base64 plus a guessed MIME type and the actual length.
3. Agent calls `fs.list_allowed_roots` and `fs.audit_log` to inspect its own session.

**Postconditions:** the binary payload is available base64-encoded; a read is recorded for the path, exactly as for a text read.
**Exceptions:**
- EXC-011a: a negative offset or length → clamped to zero rather than rejected
- EXC-011b: an unknown extension → `application/octet-stream`
- EXC-011c: `fs.audit_log` in a fresh session → an empty entry list

### SC-012: Agent batch-reads several files with per-file error isolation

**Actor:** LLM agent
**Preconditions:** the caller is a member.
**Flow:**
1. Agent calls `fs.read_many` with a `paths` array and `per_file_cap_lines`, default 500.
2. Each path is normalized and read independently; a failure is recorded against that entry and the loop continues.
3. Successful entries record a read and carry line-numbered content and a `truncated` flag.

**Postconditions:** one entry per requested path, in request order; the successes are usable even when some entries failed.
**Exceptions:**
- EXC-012a: a path failing normalization → the entry keeps the raw spelling and carries the rendered `ERR_*: message`
- EXC-012b: a missing file → the entry carries the low-level text `not found: {path}`, not an `ERR_*` string
- EXC-012c: an empty `paths` array → an empty `files` array

## 6. Functional Requirements

**Format:** EARS notation.

### Shared engine rules

#### FR-001 [EARS-U]: The engine is the only implementation
> Every filesystem operation SHALL be implemented once, in the filesystem engine, and both the MCP tool layer and the REST data plane SHALL call it rather than reimplement it.

- **Inputs:** any filesystem operation request from either surface.
- **Outputs:** identical behaviour on both surfaces.
- **Business Rules:** a tool handler performs exactly three steps: authorize, normalize every path, call the engine. The REST data plane is bound by the same rule.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:601` (write_bytes charging/writing path)

#### FR-002 [EARS-E]: Every text mutation goes through the shared commit path
> WHEN a text mutation is committed THE engine SHALL charge the byte count against the session quota, write the bytes atomically, and record an audit entry, in that order.

- **Inputs:** the new text, the operation name.
- **Outputs:** a committed file plus session accounting.
- **Business Rules:** the quota is charged before the write, so a rejected charge writes nothing. The audit entry follows a successful write, so the log records what happened rather than what was attempted.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:608-611`

#### FR-003 [EARS-U]: Result shapes are frozen
> The engine SHALL return the result keys fixed in the tool contract for every tool.

- **Inputs:** a successful tool call.
- **Outputs:** the documented key set, for example `{path, bytes_appended}` for `fs.append` and `{path, applied, diff}` for `fs.edit`.
- **Business Rules:** the tool schemas and descriptions are machine-checked against the golden contract on every test run; this requirement extends that discipline to the return shapes, which the golden file does not cover.
- **Priority:** Must-have
- **Evidence:** `TOOL_CONTRACT.txt`

### Reading

#### FR-004 [EARS-E]: Paged line window
> WHEN `fs.read` is called THE engine SHALL return a line window starting at `offset_lines`, capped at the lesser of `limit_lines` and the configured maximum read lines, together with `total_lines`, `truncated` and `next_offset`.

- **Inputs:** `path`, `offset_lines` default 0, `limit_lines` default 2000, `line_numbered` default true.
- **Outputs:** `{content, total_lines, truncated, next_offset}`.
- **Business Rules:** the server cap wins silently over a larger caller request. `next_offset` is null exactly when `truncated` is false. Line numbering starts at `offset_lines + 1`.
- **Priority:** Must-have

#### FR-005 [EARS-E]: A read unlocks later writes
> WHEN any read operation succeeds THE engine SHALL record the read against the session for that path.

- **Inputs:** the path read.
- **Outputs:** an updated session read set.
- **Business Rules:** `fs.read`, `fs.read_bytes` and `fs.read_many` all record. Listing, globbing and grepping do not, so finding a file is not the same as having read it.
- **Priority:** Must-have

#### FR-006 [EARS-E]: Batch read isolates per-file failure
> WHEN `fs.read_many` encounters a path it cannot read THE engine SHALL record the failure against that entry and SHALL continue with the remaining paths.

- **Inputs:** `paths`, `per_file_cap_lines` default 500.
- **Outputs:** `{files: [{path, content, truncated} | {path, error}]}`.
- **Business Rules:** a normalization failure keeps the caller's raw path spelling and the rendered `ERR_*: message`; a storage failure carries the low-level text such as `not found: {path}`. Entry order matches request order.
- **Priority:** Must-have

#### FR-007 [EARS-E]: Byte range reads
> WHEN `fs.read_bytes` is called THE engine SHALL return the requested byte range base64-encoded, with the MIME type guessed from the path extension and the actual length returned.

- **Inputs:** `offset_bytes` default 0, `length_bytes` default 65536.
- **Outputs:** `{base64, mime_type, length}`.
- **Business Rules:** negative offsets and lengths are clamped to zero rather than rejected. An unrecognized extension yields `application/octet-stream`. The table consulted is the engine's own MIME guesser, not the document-domain one used by the REST download route; both hold the same extensions today (see §10 note).
- **Priority:** Must-have

#### FR-008 [EARS-E]: Line-oriented read helpers
> WHEN `fs.read_lines`, `fs.head`, `fs.tail`, `fs.read_section` or `fs.count_lines` is called THE engine SHALL return the requested slice without returning the whole file.

- **Inputs:** per-tool: an inclusive `[start_line, end_line]`; `lines` default 20; an `anchor_line` with `max_lines` default 200.
- **Outputs:** the slice, or `{total_lines}` for `fs.count_lines`.
- **Business Rules:** `fs.read_lines` bounds are 1-based and inclusive. `fs.read_section` returns the indentation block surrounding the anchor, computed from leading whitespace.
- **Priority:** Must-have

### Writing

#### FR-009 [EARS-O]: No-clobber is the default for creation
> IF the target path already exists and `overwrite` is false THEN the engine SHALL refuse the write with `ERR_NO_CLOBBER` and the message `'{path}' exists (pass overwrite=true)`.

- **Inputs:** `path`, `content` or `base64`, `overwrite` default false.
- **Outputs:** the refusal, or `{path, bytes_written, overwritten}`.
- **Business Rules:** the same rule governs `fs.write`, `fs.write_bytes`, `fs.copy` and `fs.move`.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:601`

#### FR-010 [EARS-O]: Overwriting requires a prior read
> IF a write targets an existing path THEN the engine SHALL require that the session has read that path, refusing otherwise with `ERR_EDIT_WITHOUT_PRIOR_READ`.

- **Inputs:** the target path, the session read set.
- **Outputs:** the refusal, or a committed write.
- **Business Rules:** the guard applies only when the path exists; creating a new file needs no prior read. After a successful write the path is recorded as read, so an immediate second write succeeds.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:604-606`

#### FR-011 [EARS-E]: Parent creation
> WHEN `create_parents` is true THE engine SHALL create every missing parent directory before writing.

- **Inputs:** `create_parents`, default true.
- **Outputs:** the directory chain plus the file.
- **Business Rules:** with `create_parents` false and a missing parent, the storage layer refuses and nothing is written.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:607-609`

#### FR-012 [EARS-E]: Append
> WHEN `fs.append` is called THE engine SHALL append the content to the existing file and SHALL report the number of bytes appended.

- **Inputs:** `content`, `create` default false.
- **Outputs:** `{path, bytes_appended}`.
- **Business Rules:** with `create` true a missing file is created; with `create` false a missing file is `ERR_NOT_FOUND`. The append is a read-modify-write through the shared commit path, so it charges the full resulting size against the quota.
- **Priority:** Must-have

#### FR-013 [EARS-E]: Empty file creation
> WHEN `fs.create_empty` is called THE engine SHALL create a zero-length file, and WHEN `exist_ok` is false and the path exists THE engine SHALL refuse.

- **Inputs:** `path`, `exist_ok` default false.
- **Outputs:** the created node.
- **Business Rules:** an empty file stores no blob.
- **Priority:** Must-have

### Editing

#### FR-014 [EARS-O]: Unique match editing
> IF `old_string` occurs exactly once THEN the engine SHALL replace it; IF it occurs more than once and `replace_all` is false THEN the engine SHALL refuse with `ERR_AMBIGUOUS_MATCH`; IF it does not occur THEN the engine SHALL refuse with `ERR_NO_MATCH`.

- **Inputs:** `old_string`, `new_string`, `replace_all` default false.
- **Outputs:** `{path, applied, diff}` or the refusal.
- **Business Rules:** the messages are `"old_string not found in '{path}'"` and `"old_string matches {count} sites in '{path}' (use replace_all)"`, the second carrying the actual count. Matching is byte-exact substring matching with no normalization.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:1294,1298`

#### FR-015 [EARS-E]: Dry run produces the diff without writing
> WHEN `dry_run` is true THE engine SHALL compute and return the unified diff and SHALL NOT write the file.

- **Inputs:** `dry_run`, default false.
- **Outputs:** `applied` false with a populated `diff`.
- **Business Rules:** `applied` equals the negation of `dry_run`. A dry run still enforces the read guard and still fails on a non-matching `old_string`, so it is a true preview of the same operation.
- **Priority:** Must-have

#### FR-016 [EARS-E]: Multi-edit is all-or-nothing and order-dependent
> WHEN `fs.multi_edit` is called THE engine SHALL apply each edit against the text produced by the previous edit and SHALL write nothing unless every edit resolves.

- **Inputs:** `edits`, an array of `{old_string, new_string, replace_all?}`.
- **Outputs:** `{path, applied, edits, diff}` where `edits` is the count applied.
- **Business Rules:** the returned diff compares the original text with the final text, not the intermediate steps. Composition order matters: an edit whose `old_string` an earlier edit destroyed fails the whole call.
- **Priority:** Must-have

#### FR-017 [EARS-O]: Fuzzy block replacement threshold
> IF `fuzzy` is true THEN the engine SHALL slide a window of the search block's line count over the file, keep the most similar candidate, and accept it only at a similarity of at least 0.6.

- **Inputs:** `search_block`, `replace_block`, `fuzzy` default false.
- **Outputs:** `{path, applied, diff}` or `ERR_NO_MATCH` with `"no fuzzy match for search_block in '{path}'"`.
- **Business Rules:** the fuzzy replace threshold is 0.6. The similarity ratio is LCS-based, in `0.0..=1.0`. The replacement block is newline-terminated whether or not the caller terminated it.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:41,1330`

#### FR-018 [EARS-E]: Line insertion
> WHEN `fs.insert_at_line` is called THE engine SHALL insert the content immediately before the given 1-based line number.

- **Inputs:** `line` (1-based), `content`.
- **Outputs:** `{path, applied, line}`.
- **Business Rules:** a line number past the end appends; the read guard applies as for every edit.
- **Priority:** Must-have

#### FR-019 [EARS-E]: Multi-file patch application
> WHEN `fs.apply_patch` is called THE engine SHALL parse the patch and apply each file operation in order, returning one entry per touched path.

- **Inputs:** patch text delimited by begin/end markers, with add/update/delete operations and a move modifier, hunks introduced by a context marker.
- **Outputs:** `{files: [{path, op} | {path, op, moved_to}]}`.
- **Business Rules:** every operation normalizes its own path. Adds charge the quota and write without a read-guard check; updates and deletes enforce the guard. An update carrying the move modifier writes in place and then renames. The patch is not transactional: operations are applied in sequence and a failure partway leaves earlier operations committed.
- **Priority:** Must-have

#### FR-020 [EARS-X]: Duplicate audit entry on patched updates
> `fs.apply_patch` records two audit entries per updated file: one inside the update branch and one in the loop tail. The duplicate is observable through `fs.audit_log` and is retained deliberately.

- **Inputs:** an update operation in a patch.
- **Outputs:** two `apply_patch` audit entries for the same path.
- **Business Rules:** this is the one behaviour in this spec with no clean EARS pattern, because it specifies the preservation of a quirk rather than an intended rule. It is frozen because `fs.audit_log` output is part of the observable contract.
- **Priority:** Should-have

### Listing, metadata and search

#### FR-021 [EARS-E]: Directory listing
> WHEN `fs.list_dir` is called THE engine SHALL return a flat listing of the directory with each entry's name and kind, defaulting the path to `/`.

- **Inputs:** `path` default `/`, `include_hidden` default false, `sort_by` default `name`, `with_sizes` default false.
- **Outputs:** `{path, entries: [{name, kind}], total}`.
- **Business Rules:** hidden entries are those whose name begins with `.`; they are omitted unless requested. Sizes appear only when `with_sizes` is true.
- **Priority:** Must-have

#### FR-022 [EARS-E]: Recursive tree with caps
> WHEN `fs.tree` is called THE engine SHALL return a nested tree to `max_depth` and SHALL stop after 2000 nodes, reporting truncation.

- **Inputs:** `path` default `/`, `max_depth` default 3, `exclude_patterns`, `with_sizes`.
- **Outputs:** `{path, tree: [...]}` with nested `children`.
- **Business Rules:** the tree cap is 2000. The tree prunes `.git`, `node_modules`, `target`, `dist`, `.build` and `coverage`, and deliberately does not prune the trash directory, unlike the walk used by glob and grep.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:30,39`

#### FR-023 [EARS-U]: Volume walk ceiling
> The engine SHALL visit at most 5000 files in any single walk.

- **Inputs:** any walk-based operation: glob, grep, symbol search.
- **Outputs:** results limited to the files visited.
- **Business Rules:** the walk visits at most 5000 files. The walk skips the default excludes: `.git`, `node_modules`, `target`, `dist`, `.build`, `coverage` and the trash directory.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:25-26,35,313`

#### FR-024 [EARS-E]: Glob matching and ordering
> WHEN `fs.glob` is called THE engine SHALL match the pattern against both the full path and the bare file name, return matches newest first, and cap the result at 100 with a truncation flag.

- **Inputs:** `pattern`, `root` default `/`, `exclude_patterns`.
- **Outputs:** `{matches, truncated}`.
- **Business Rules:** the glob cap is 100. Sorting is by mtime descending and is stable, so equal mtimes keep walk order. Matching uses the shared glob matcher, which is Python `fnmatch` semantics, so a single `*` does cross a `/` boundary.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:347-348`

#### FR-025 [EARS-E]: Grep output modes
> WHEN `fs.grep` is called THE engine SHALL shape its result by `output_mode`: `files` yields `{files}`, `count` yields `{count, files}`, and any other value yields `{matches, truncated}`.

- **Inputs:** `pattern`, `root`, `include_glob`, `exclude_glob`, `regex` default true, `case_sensitive` default true, `output_mode` default `content`, `context_lines` default 0, `max_matches` default 100.
- **Outputs:** one of the three shapes.
- **Business Rules:** `content` is the default and is reached by the fall-through branch, so an unrecognized `output_mode` behaves as `content` rather than failing.
- **Priority:** Must-have

#### FR-026 [EARS-E]: Symbol search
> WHEN `fs.find_definition` or `fs.find_references` is called THE engine SHALL walk the volume from `root` and run the language-aware matcher on every file whose language is recognized.

- **Inputs:** `name`, `root` default `/`, `kind` filter for definitions.
- **Outputs:** `{definitions: [{path, name, kind, line}]}` or `{references: [{path, line, kind}]}`.
- **Business Rules:** files in unrecognized languages are skipped silently. No match yields an empty array, not an error. The matcher itself is outside this spec's scope.
- **Priority:** Must-have

#### FR-027 [EARS-E]: Metadata probes do not mutate or record
> WHEN `fs.stat`, `fs.exists`, `fs.hash`, `fs.count_lines`, `fs.list_dir`, `fs.tree`, `fs.glob` or `fs.grep` is called THE engine SHALL neither mutate the volume nor record a read against the session.

- **Inputs:** any probe.
- **Outputs:** the probe result only.
- **Business Rules:** these are pure probes. The consequence is deliberate: locating a file through search does not satisfy the read guard for a later edit.
- **Priority:** Must-have

#### FR-028 [EARS-U]: Stat reports synthetic ownership
> `fs.stat` SHALL report uid 1000 and gid 1000 for every entry.

- **Inputs:** a path.
- **Outputs:** POSIX-shaped metadata including mode, size and times.
- **Business Rules:** the synthetic uid/gid are both 1000; the volume has no real POSIX owner, so the values are synthetic and constant.
- **Priority:** Must-have

#### FR-029 [EARS-O]: Hash algorithm allow-list
> IF `algo` is not one of `md5`, `sha1`, `sha256` or `sha512` THEN the engine SHALL refuse with `ERR_INVALID_ARGUMENT`.

- **Inputs:** `algo`, default `sha256`.
- **Outputs:** the digest, or the refusal.
- **Business Rules:** the allow-list is closed; the default is `sha256`, which is also the content-addressing hash.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:32`

### Lifecycle

#### FR-030 [EARS-E]: Delete moves to trash by default
> WHEN `fs.delete` is called with `trash` true THE engine SHALL create the trash parent directory and rename the path into it, returning the destination.

- **Inputs:** `path`, `recursive` default false, `trash` default true.
- **Outputs:** `{path, trashed, trash_path}`.
- **Business Rules:** the destination comes from the shared trash derivation; its parent is created before the rename. The audit detail is `-> {destination}` for a soft delete and `hard` for a hard one.
- **Priority:** Must-have

#### FR-031 [EARS-O]: Hard delete requires explicit configuration
> IF `trash` is false and hard delete is not enabled THEN the engine SHALL refuse with `ERR_NOT_SUPPORTED` and the message `hard delete disabled (server started without allow_hard_delete)`.

- **Inputs:** `trash` false.
- **Outputs:** the refusal.
- **Business Rules:** hard delete defaults to disabled, so destroying data requires a deliberate deployment decision.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:1082`

#### FR-032 [EARS-O]: Directory deletion requires recursive
> IF the target is a directory and `recursive` is false THEN the engine SHALL refuse with `ERR_INVALID_ARGUMENT` and the message `'{path}' is a directory (pass recursive=true)`.

- **Inputs:** `path`, `recursive`.
- **Outputs:** the refusal.
- **Business Rules:** the existence check precedes the directory check, so a missing path yields `ERR_NOT_FOUND` rather than the directory message.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:1076`

#### FR-033 [EARS-E]: Move and copy
> WHEN `fs.move` or `fs.copy` is called THE engine SHALL refuse an existing destination unless `overwrite` is true, and SHALL require `recursive` for a tree copy.

- **Inputs:** `source`, `destination`, `overwrite` default false, `recursive` default false for copy.
- **Outputs:** `{source, destination}`.
- **Business Rules:** a copy is metadata-only: the destination node references the same content blob and the refcount rises. No bytes are duplicated.
- **Priority:** Must-have
- **Evidence:** `crates/core/src/core/fs_ops.rs:1180`

#### FR-034 [EARS-E]: Session introspection tools
> WHEN `fs.list_allowed_roots` or `fs.audit_log` is called THE engine SHALL apply the membership gate and SHALL return session-scoped information without opening the volume.

- **Inputs:** `mount_id`; for the log, `since` and `limit` default 20.
- **Outputs:** `{person, roots: [{mount_id, root, owner}]}` and `{entries: [{timestamp, op, path, detail}]}`.
- **Business Rules:** both take `mount_id` and pass the membership gate even though neither opens a volume. The audit log is the capped per-session ring, so it is empty after a restart.
- **Priority:** Must-have

## 7. Non-Functional Requirements

### 7.1 Performance
- Every walk-based operation is bounded: 5000 files visited, 100 glob results, 2000 tree nodes. These ceilings are contract, not tuning: they bound both server work and the tokens returned to an agent.
- `fs.read` caps its window at the configured maximum read lines regardless of the caller's request (FR-004).
- `fs.count_lines` and `fs.hash` avoid returning content, so an agent can size a file before reading it.

### 7.2 Security
- Inherited unchanged from the platform foundation: membership gate before any storage access, normalization of every path argument, quota on every write, audit on every mutation.
- The multi-file patch normalizes each operation's path individually, so a patch cannot reach outside the volume through an unnormalized entry.
- Hard delete is off by default (FR-031).

### 7.3 Usability
- Error messages name the remedy: `pass overwrite=true`, `use replace_all`, `pass recursive=true`.
- `fs.read_many` isolates failures so one bad path does not lose the whole batch (FR-006).
- `dry_run` gives an agent a true preview with the same validation as the real edit (FR-015).

### 7.4 Reliability
- Writes are atomic at the volume client level; a failed write leaves the previous content intact.
- `fs.multi_edit` is all-or-nothing (FR-016).
- `fs.apply_patch` is explicitly not transactional (FR-019); this is a known limitation (see §10).

### 7.5 Observability
Every mutation leaves a session audit entry readable through `fs.audit_log`, which is the closest thing this layer has to an operation log.

### 7.6 Deployment
No deployment surface of its own: the engine ships inside the single server binary.

### 7.7 Scalability
- The walk ceilings bound worst-case work per request independently of volume size.
- A copy is metadata-only, so duplicating a large tree costs rows, not bytes.
- Because the session read set is per process, the read guard behaves differently under multiple replicas; an agent's read on one replica does not unlock an edit on another.

## 8. End-to-End Tests

**Fixtures, shared by every test:** project `spec-fs` with `ALICE` as owner and member; a seeded volume containing `/a.txt` with the three lines `one`, `two`, `three`; `/src/app.py`; `/bin.dat` holding 4 non-UTF8 bytes; `/big.txt` with 5000 numbered lines.

| Test ID | Scenario | Requirements | Summary |
|---|---|---|---|
| E2E-001 | SC-001 | FR-004 | Paged read of a large file returns a window capped by `limit_lines`. |
| E2E-002 | SC-001 | FR-005, FR-008 | Reading a file records the read and unlocks a later edit. |
| E2E-003 | SC-001 | FR-004 | `limit_lines` above the server cap is silently capped, not an error. |
| E2E-004 | SC-001 | FR-005 | Listing and grepping do not satisfy the read guard; an explicit read does. |
| E2E-005 | SC-001 | FR-008 | Existing read-helper coverage. |
| E2E-006 | SC-001 | FR-004 | An offset past the end returns an empty window, not an error. |
| E2E-007 | SC-001 | FR-004 | Paging with `next_offset` walks the whole file exactly once. |
| E2E-008 | SC-001 | FR-005, FR-008 | Existing edge coverage of read + helpers interaction. |
| E2E-009 | SC-002 | FR-002, FR-012 | Create then append; quota and audit advance. |
| E2E-010 | SC-002 | FR-009 | No-clobber refuses an existing path without `overwrite`. |
| E2E-011 | SC-002 | FR-011, FR-013 | Parent creation and empty-file creation. |
| E2E-012 | SC-002 | FR-009 | No-clobber error path, existing coverage. |
| E2E-013 | SC-002 | FR-010 | Overwriting a file never read in this session is refused, then succeeds after an explicit read. |
| E2E-014 | SC-002 | FR-011 | Missing parent without `create_parents` refuses, existing coverage. |
| E2E-015 | SC-002 | FR-012 | Append error path, existing coverage. |
| E2E-016 | SC-002 | FR-002, FR-013 | A write refused by the quota changes nothing: no file, no audit entry. |
| E2E-017 | SC-002 | FR-010 | A fresh write needs no prior read; an overwrite does. |
| E2E-018 | SC-002 | FR-009, FR-012 | `fs.append` creates only when `create` is true. |
| E2E-019 | SC-002 | FR-002, FR-011 | A write creates parents and leaves exactly one audit entry. |
| E2E-020 | SC-002 | FR-010, FR-013 | Existing edge coverage of overwrite + empty-file interaction. |
| E2E-021 | SC-003 | FR-014 | Unique-match edit succeeds. |
| E2E-022 | SC-003 | FR-015 | Dry run returns the diff without writing. |
| E2E-023 | SC-003 | FR-014 | No match → `ERR_NO_MATCH`. |
| E2E-024 | SC-003 | FR-014 | Ambiguous match → `ERR_AMBIGUOUS_MATCH`. |
| E2E-025 | SC-003 | FR-015 | A dry run still enforces the read guard. |
| E2E-026 | SC-003 | FR-015 | A dry run on a non-matching string fails exactly like a real edit. |
| E2E-027 | SC-003 | FR-014 | `replace_all` rewrites every occurrence; the ambiguity message reports the actual count. |
| E2E-028 | SC-003 | FR-015 | A dry run returns a non-empty diff and leaves the file byte-identical (verified by hash). |
| E2E-029 | SC-004 | FR-016 | Happy-path multi-edit. |
| E2E-030 | SC-004 | FR-016 | Any failing edit fails the whole call, existing coverage. |
| E2E-031 | SC-004 | FR-016 | A multi-edit whose last edit fails writes nothing: the file is unchanged and no audit entry is written. |
| E2E-032 | SC-004 | FR-016 | Edits compose in order against the intermediate text, producing an ambiguity the original text did not have. |
| E2E-033 | SC-004 | FR-016 | An empty `edits` array is a no-op that still reports success with `edits` 0. |
| E2E-034 | SC-004 | FR-016 | The multi-edit diff compares original to final text, never mentioning an intermediate value. |
| E2E-035 | SC-005 | FR-017 | Fuzzy replace happy path. |
| E2E-036 | SC-005 | FR-017 | Below-threshold fuzzy match refused, existing coverage. |
| E2E-037 | SC-005 | FR-017 | A block below the similarity threshold is refused with the fuzzy-specific message. |
| E2E-038 | SC-005 | FR-017 | Existing error coverage. |
| E2E-039 | SC-005 | FR-017 | A near-miss block above the threshold is accepted and the replacement is newline-normalized. |
| E2E-040 | SC-005 | FR-017 | Exact mode ignores the similarity threshold entirely; a near-miss in exact mode still fails. |
| E2E-041 | SC-006 | FR-019, FR-020 | Happy-path patch application with one update, two audit entries. |
| E2E-042 | SC-006 | FR-019 | Malformed patch fails before anything is written, existing coverage. |
| E2E-043 | SC-006 | FR-019 | A patch whose update targets an unread file fails after earlier add operations already committed, proving non-transactionality. |
| E2E-044 | SC-006 | FR-020 | An updated file receives exactly two audit entries; an added file receives exactly one. |
| E2E-045 | SC-006 | FR-019 | An update carrying the move modifier relocates the patched content to the new path. |
| E2E-046 | SC-006 | FR-019 | Each patch operation normalizes its own path; an escaping path is refused, a `./..` path resolves normally. |
| E2E-047 | SC-006 | FR-020 | An add operation does not require a prior read. |
| E2E-048 | SC-007 | FR-021, FR-029 | Listing happy path with default hash algorithm. |
| E2E-049 | SC-007 | FR-022 | Tree happy path. |
| E2E-050 | SC-007 | FR-027, FR-028 | Probe happy path including synthetic uid/gid. |
| E2E-051 | SC-007 | FR-021, FR-029 | Listing error coverage, existing. |
| E2E-052 | SC-007 | FR-022 | A tree deeper than `max_depth` stops at the limit; deeper nodes never appear. |
| E2E-053 | SC-007 | FR-027, FR-028 | Probes leave the session untouched: no audit entry, and the read guard is unaffected. |
| E2E-054 | SC-007 | FR-021, FR-029 | The hash allow-list is closed and the default equals `sha256`'s digest. |
| E2E-055 | SC-007 | FR-022 | The tree shows the trash directory while the walk used by glob hides it. |
| E2E-056 | SC-007 | FR-027, FR-028 | Hidden entries appear only when `include_hidden` is requested. |
| E2E-057 | SC-008 | FR-023, FR-025 | Grep happy path across output modes. |
| E2E-058 | SC-008 | FR-024 | Glob happy path. |
| E2E-059 | SC-008 | FR-023 | A walk stops at the 5000-file ceiling without erroring. |
| E2E-060 | SC-008 | FR-024 | Glob error coverage, existing. |
| E2E-061 | SC-008 | FR-025 | Grep error coverage, existing. |
| E2E-062 | SC-008 | FR-023 | Excluded directories are skipped by the walk entirely. |
| E2E-063 | SC-008 | FR-024 | A single `*` crosses a `/` boundary, confirming `fnmatch` semantics. |
| E2E-064 | SC-008 | FR-024 | Glob results are newest-first and capped at 100 with `truncated` true. |
| E2E-065 | SC-008 | FR-025 | An unrecognized `output_mode` behaves as `content`. |
| E2E-066 | SC-009 | FR-026 | Symbol search happy path. |
| E2E-067 | SC-009 | FR-026 | Symbol search error coverage, existing. |
| E2E-068 | SC-009 | FR-026 | A symbol search in an unsupported-language file returns an empty array, not an error. |
| E2E-069 | SC-009 | FR-026 | The `root` parameter scopes the symbol search to that subtree. |
| E2E-070 | SC-010 | FR-030 | Soft delete happy path. |
| E2E-071 | SC-010 | FR-033 | Move/copy happy path. |
| E2E-072 | SC-010 | FR-030 | A soft delete leaves the content retrievable under the returned trash path, with a matching audit detail. |
| E2E-073 | SC-010 | FR-031 | Hard delete disabled by default, existing security coverage. |
| E2E-074 | SC-010 | FR-032, FR-033 | Directory deletion without `recursive` error, existing coverage. |
| E2E-075 | SC-010 | FR-032, FR-033 | Overwrite without `overwrite` error, existing coverage. |
| E2E-076 | SC-010 | FR-030, FR-031 | Hard delete works end to end when explicitly enabled by server configuration. |
| E2E-077 | SC-010 | FR-031, FR-032 | Deleting a missing directory reports not-found rather than the recursive-hint message. |
| E2E-078 | SC-011 | FR-007 | Byte-range read happy path. |
| E2E-079 | SC-011 | FR-034 | Session introspection happy path. |
| E2E-080 | SC-011 | FR-007 | A negative byte offset and length are clamped rather than rejected. |
| E2E-081 | SC-011 | FR-034 | Session tools still enforce membership even though they open no volume. |
| E2E-082 | SC-011 | FR-007 | Reading bytes records a read and reports the correct MIME type for text and binary files. |
| E2E-083 | SC-011 | FR-034 | The audit log is capped and ordered oldest first, with every entry carrying the full key set. |
| E2E-084 | SC-012 | FR-001, FR-003, FR-006 | Batch read isolates one bad path among good ones. |
| E2E-085 | SC-012 | FR-018 | Line insertion happy path, existing coverage. |
| E2E-086 | SC-012 | FR-001, FR-006 | A path failing normalization keeps its raw spelling and carries a rendered `ERR_*` message. |
| E2E-087 | SC-012 | FR-003 | Result key sets for several tools match the frozen contract exactly, no extra or missing keys. |
| E2E-088 | SC-012 | FR-006, FR-018 | Per-file caps truncate one entry without failing the batch. |
| E2E-089 | SC-012 | FR-001, FR-006 | An empty `paths` array yields an empty result, not an error. |
| E2E-090 | SC-012 | FR-003, FR-018 | Insertion before line 1 and past the end both behave predictably. |

## 9. Glossary

| Term | Definition | Context |
|---|---|---|
| **Window** | The slice of lines returned by a paged read, with its truncation flag and next offset. | Engine |
| **next_offset** | The `offset_lines` value that continues a truncated read where it stopped. | Engine |
| **Read guard** | The platform rule that a write to a path not read in this session is refused. | Session |
| **No-clobber** | The default refusal to overwrite an existing path unless `overwrite` is true. | Engine |
| **Unique match** | The editing rule requiring `old_string` to occur exactly once unless `replace_all` is set. | Engine |
| **Dry run** | An edit that validates and produces its diff without writing. | Engine |
| **Unified diff** | The `--- / +++ / @@` text returned by every editing tool. | Engine |
| **Fuzzy replace** | Block replacement accepting the most similar window at a similarity of at least 0.6. | Engine |
| **Similarity ratio** | The LCS-based score in `0.0..=1.0` comparing a candidate window to the search block. | Engine |
| **Multi-file patch** | The patch dialect delimited by begin/end markers, carrying add/update/delete file operations. | Patch |
| **Hunk** | One context-introduced group of context, removed and added lines inside an update. | Patch |
| **Walk** | The bounded traversal of a volume used by glob, grep and symbol search. | Engine |
| **Glob cap** | The 100-path ceiling on `fs.glob` results. | Engine |
| **Tree cap** | The 2000-node ceiling on `fs.tree`. | Engine |
| **Trash** | The soft-delete destination derived by the platform safety layer. | Session |
| **Probe** | A read-only operation that neither mutates nor records a read: stat, exists, hash, listing, search. | Engine |

## 10. Confidence notes

- **Confidence: high.** The engine's structural rules (single implementation, shared commit path, result-shape freeze), the paging/edit/patch/search mechanics, and every cited constant (walk ceiling, glob cap, tree cap, fuzzy threshold, hash allow-list, synthetic uid/gid) were spot-checked directly against `crates/core/src/core/fs_ops.rs` at the base commit and confirmed present and unchanged in substance.
- **Line-number drift.** The engine file has grown since the original draft (from 3355 to roughly 4070 lines) and its crate path moved (`crates/mcp-fs/src/core/fs_ops.rs` → `crates/core/src/core/fs_ops.rs`), so most original `fs_ops.rs:N` citations no longer point at the same content. Evidence lines in this document were re-verified against the current file for the constants and the handful of FRs carrying an Evidence line; FRs without an Evidence line were not individually re-verified line-by-line but their described behaviour was confirmed by targeted grep during this pass (no-clobber messages, read-guard ordering, fuzzy threshold, hash allow-list, hard-delete message, directory-deletion message, move/copy no-clobber).
- **Scope split preserved.** The three document tools and the symbol-matcher engine remain out of scope, as in the source document; this spec covers the 32 `fs.*` tools and the gates/shapes of `fs.find_definition`/`fs.find_references`.
- **Open item retained.** Whether `fs.grep`'s fall-through for an unrecognized `output_mode` should instead be `ERR_INVALID_ARGUMENT` is unresolved and carried into the design document's findings.
