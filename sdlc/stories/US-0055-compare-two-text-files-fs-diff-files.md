# US-0055: compare two text files with fs.diff_files

> Spec: SPEC-0045 (sdlc/specs/SPEC-0045-fs-diff-two-files/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 10
> Closes spec: no

## Objective
A member can compare two text files, in one project or in two projects, with one call to the new tool `fs.diff_files`, and gets a unified diff (3 lines of context, headers `--- <project>:<path>` and `+++ <project>:<path>`) or `identical: true`. This story adds the tool end to end for text files that exist; the wording of refusals other than access comes in US-0056. Over the usual budget on file count (10) because five of them are mechanical contract and count edits that the registry forces on any new tool.

## Requirements covered
- SPEC-0045/FR-NEW-001: WHEN a member asks to compare two text files THE product SHALL return the differences from the first file to the second as a unified diff with 3 unchanged lines of context around each change.
- SPEC-0045/FR-NEW-002: The product SHALL label the old side of a diff `--- <project>:<path>` and the new side `+++ <project>:<path>`, with the normalized path of each file.
- SPEC-0045/FR-NEW-003: IF the two files have exactly the same content THEN THE product SHALL answer that they are identical, with an empty diff.
- SPEC-0045/FR-NEW-004: The product SHALL accept two files from two different projects when the caller is a member of both.
- SPEC-0045/FR-NEW-005: IF the caller is not a member of the project of either file THEN THE product SHALL refuse the comparison and SHALL NOT reveal the existence or content of either file.
- SPEC-0045/FR-NEW-016: The product SHALL compare lines including their endings (line feed, carriage return plus line feed, lone carriage return), and SHALL NOT limit the diff to the first 2000 lines.
- SPEC-0045/FR-NEW-017 (partial: this story only reuses `normalize_path` unchanged; its refusal tests are in US-0056): The product SHALL normalize the path of each file exactly as every other file operation does, and SHALL show the normalized path in refusals and in the diff headers.

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e` : the 15 tests above pass, and E2E-001 returns exactly the diff text asserted (headers, block header `@@ -1,3 +1,3 @@`, lines ` one`, `-two`, `+2`, ` three`). Then `cargo test -p mcp-fs-core --lib tool_contract_golden_is_current tool_router_lists_exactly_the_100_contract_names ninety_five_tool_schemas_match_the_golden_contract_structurally` passes with 109 tools. The assertions read real files through the real storage layer; nothing is mocked.

## Design excerpt
### Decisions
- (SPEC-0045) DEC-001: The operation lives in a new `tools/diff_files.rs` (`diff_files` plus `Side`), called by the MCP adapter and the REST handler. **Obliged by:** SPEC-0045/FR-NEW-013. **Rationale:** one implementation for both surfaces; two projects do not fit `core::fs_ops` single client functions. **Alternatives considered:** `fs_ops` (section 2). **Source:** AGENTS.md Conventions, `tools/export.rs:58`. **Evidence:** `tools/export.rs:58`.
- (SPEC-0045) DEC-002: Add `diff::unified_labeled(old, new, old_label, new_label, context)`; `unified_with_context` delegates with the same label twice. **Obliged by:** SPEC-0045/FR-NEW-002. **Rationale:** header needs two labels; the four existing callers must stay byte identical. **Alternatives considered:** a second diff routine (duplication); a post hoc string replace of the header (fragile for paths containing `+++`). **Source:** design. **Evidence:** `core/diff.rs:56,69`.
- (SPEC-0045) DEC-003: `identical` is `old_text == new_text`, computed before diffing; `diff` is then `""`. **Obliged by:** FR-NEW-003. **Rationale:** both texts are valid UTF-8 at this point, so string equality is byte equality; line splitting is lossless (`diff.rs:17-45`). **Alternatives considered:** `diff.is_empty()` (same result, computed after the fact). **Source:** design. **Evidence:** `core/diff.rs:58-67`.
- (SPEC-0045) DEC-004: Order: normalize path A, normalize path B, authorize and open A, authorize and open B, read A completely, read B completely. **Obliged by:** FR-NEW-015. **Rationale:** matches the spec's twelve checks; `AppState::authorize` already reports a deleted project before membership. **Alternatives considered:** reading both files concurrently (would reorder errors). **Source:** spec. **Evidence:** `mcp/server.rs:178-181`, `storage/admin.rs:392-395`.
- (SPEC-0045) DEC-008: The answer is a `serde_json` object with exactly `identical` and `diff`. **Obliged by:** FR-NEW-003, 013. **Rationale:** pinned (spec 14.4). **Alternatives considered:** none. **Source:** spec constraint. **Evidence:** spec 14.4.
- (SPEC-0045) DEC-009: Tool description and argument descriptions as in section 5; the golden file is regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1` and reviewed (one new entry, no other change); `TOOL_CONTRACT.txt` is edited by hand. **Obliged by:** FR-NEW-001. **Rationale:** AGENTS.md: golden never hand edited. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `tools/contract_golden.rs`.
- (SPEC-0045) DEC-013: Count updates (108 to 109, 44 to 45 `fs.*`) land with US-0055 in code and tests, and with US-0060 in README, AGENTS.md and `.agent_docs`. **Obliged by:** FR-NEW-001. **Rationale:** `make check` must be green on main after each story; docs may trail by the later stories. **Alternatives considered:** docs in US-0055 (adds three files to the heaviest story). **Source:** design. **Evidence:** `rg -n "\b108\b"` output.
- (SPEC-0045) DEC-015: No new dependency, no schema or data change, no feature flag. **Obliged by:** n/a (no code impact): records that ASK ONLY FOR categories 1, 2 and 4 do not apply. **Rationale:** `similar`, rmcp, axum, serde_norway already carry everything. **Alternatives considered:** a dedicated text detection crate (a new dependency for a two line check). **Source:** design. **Evidence:** `Cargo.toml:86`.
### Data model
None (no persisted change).
### Contracts
Tool `fs.diff_files`, required string arguments `mount_id_a`, `path_a` (old side), `mount_id_b`, `path_b` (new side).
Descriptions (frozen once written, copy verbatim):
- tool: `Compare two text files (same or different project) and return a unified diff with 3 lines of context.`
- `mount_id_a`: `Project/volume id of the old side.`
- `path_a`: `Absolute POSIX path of the old side within its volume.`
- `mount_id_b`: `Project/volume id of the new side.`
- `path_b`: `Absolute POSIX path of the new side within its volume.`
Test registry annotations: `readOnlyHint=true`, `idempotentHint=true`, `openWorldHint=false`.
Answer: `{"identical": <bool>, "diff": "<unified diff or empty string>"}`.
Signatures:
```rust
// core/diff.rs
pub fn unified_labeled(old: &str, new: &str, old_label: &str, new_label: &str, context: usize) -> String;
// unified_with_context keeps its signature and delegates: unified_labeled(old, new, path, path, context)

// tools/diff_files.rs
pub(crate) struct Side<'a> { pub mount_id: &'a str, pub path: &'a str }
pub(crate) async fn diff_files(state: &AppState, person: &str, a: Side<'_>, b: Side<'_>) -> Result<Value>;
```
Order inside `diff_files`: `state.safety.normalize_path(a.path)`, same for b; `state.authorize(a.mount_id, person)` then `state.stores.client(a.mount_id)`; same for b; read a fully then b fully (here with the existing `client.read_text`); if the two texts are equal return `{"identical": true, "diff": ""}`; else build the diff inside `tokio::task::spawn_blocking` with labels `format!("{mount_id}:{path}")` (normalized paths) and return `{"identical": false, "diff": ...}`. Do NOT call `safety.record_read` (US-0058 relies on it) and do not touch last access here (US-0058).
Contract text to add to `TOOL_CONTRACT.txt` (after the `fs.hash` entry, same layout as its neighbours):
```
fs.diff_files
  desc: Compare two text files (same or different project) and return a unified diff with 3 lines of context.
  params: mount_id_a:string, path_a:string, mount_id_b:string, path_b:string
  required: ['mount_id_a', 'path_a', 'mount_id_b', 'path_b']
  annotations: readOnlyHint=true, idempotentHint=true, openWorldHint=false
```
and in the return shapes block: `fs.diff_files: {"identical": false, "diff": "--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -1 +1 @@\n-a\n+b\n"}`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/core/diff.rs` | modify |
| `crates/core/src/tools/diff_files.rs` | create |
| `crates/core/src/tools/mod.rs` | modify |
| `crates/core/src/mcp/server.rs` | modify |
| `crates/core/src/tools/diff_files_e2e.rs` | create |
| `crates/core/src/tools/contract_golden.rs` | modify (counts 108 to 109 at lines 145, 252, 308, 322; fs count 44 to 45 in the message) |
| `crates/agent/src/mcp.rs` | modify (count 108 to 109 at lines 297 to 319) |
| `tool-contract-golden.json` | regenerate |
| `TOOL_CONTRACT.txt` | modify |
### Existing patterns
- MCP method and args struct: `crates/core/src/mcp/server.rs:610-623` (`HashArgs`) and `:2018-2029` (`fs_hash`): `#[tool(name = "...", description = "...")]`, `Parameters(a)`, `let out = async { ... }.await; to_call_result("fs.hash", out)`. Here the body calls `crate::tools::diff_files::diff_files(&self.state, &self.person, Side { .. }, Side { .. })` instead of `self.volume(..)`.
- Test registry glue and annotations: `crates/core/src/tools/read.rs:186-198` (`fs.count_lines`: `ToolSchema::new(...).req_str(..).read_only(true).idempotent(true).open_world(false)` and a `handler(|ctx, a| async move { .. })`). Register in `tools/mod.rs` next to `read::register(reg)` (line 96) and add `"fs.diff_files"` to the name list in `tools/mod.rs` (around line 395 to 430, after `"fs.hash"`) and to the matching count test.
- Acceptance test harness: `crates/core/src/tools/quota_e2e.rs:1-70` (`Fixture`, `McpServer::new(state, person)`, `text_of`, `Parameters(Args)`). `Fixture::new().await`, `f.seed_project("alpha", "owner@test.com").await`, `f.state.admin.add_member("alpha", "alice@example.com", "owner@test.com").await`, files written with `f.state.stores.client("alpha").await?.write_text_atomic(path, text)`.
- Golden regeneration: `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, then review the diff: exactly one new tool entry, nothing else.

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions of all tests (copied from the spec):

Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

### SPEC-0045/E2E-001: Two files of one project differ on one line
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 002, 016. **Priority:** Critical
- **Given** Alice and the common data
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes/b.txt`
- **Then** she receives `--- alpha:/notes/a.txt`, `+++ alpha:/notes/b.txt`, the block header `@@ -1,3 +1,3 @@`,
  the lines ` one`, `-two`, `+2`, ` three`, and the identical indicator is false
- **And** nothing else is returned
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-002: Two projects
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** journey. **Scenario:** SC-002. **Requirements:** FR-NEW-001, 002, 004. **Priority:** Critical
- **Given** `alpha:/cfg/app.txt` = `port=80\n` and `beta:/cfg/app.txt` = `port=8080\n`, Alice a member of both
- **When** Alice compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`
- **Then** the headers are `--- alpha:/cfg/app.txt` and `+++ beta:/cfg/app.txt`, with `-port=80` and
  `+port=8080`, identical indicator false
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-003: Identical files
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** journey. **Scenario:** SC-003. **Requirements:** FR-NEW-003. **Priority:** Critical
- **Given** `alpha:/notes/a.txt` and `alpha:/notes/copy.txt` both `one\ntwo\nthree\n`
- **When** Alice compares them
- **Then** the identical indicator is true and the diff is empty (zero characters)
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-107: No access to the second project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival once both projects are authorized (inherited authorization)
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-005. **Priority:** Critical
- **Given** `beta:/cfg/app.txt` exists with the line `secret=1`; Bob is a member of `alpha` only
- **When** Bob compares `alpha:/notes/a.txt` with `beta:/cfg/app.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'`
- **And** the answer contains neither `secret` nor any line of either file, and is word for word the same
  when Bob names `beta:/cfg/nothing.txt` which does not exist
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-108: No access to the first project
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival once both projects are authorized (inherited authorization)
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-005. **Priority:** Critical
- **Given** as E2E-107
- **When** Bob compares `beta:/cfg/app.txt` with `alpha:/notes/a.txt`
- **Then** `ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'`, no content
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-201: Empty against non empty
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 003. **Priority:** Medium
- **Given** `alpha:/e.txt` empty (0 bytes) and `alpha:/notes/b.txt`
- **When** Alice compares `/e.txt` with `/notes/b.txt`
- **Then** the diff has the block header `@@ -0,0 +1,3 @@` and the three lines `+one`, `+2`, `+three`;
  indicator false
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-202: Two empty files
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-003, 008. **Priority:** Medium
- **Given** `alpha:/e1.txt` and `alpha:/e2.txt`, both 0 bytes
- **When** Alice compares them
- **Then** identical true, empty diff (an empty file is a text file)
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-203: The same file on both sides
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-003, 010. **Priority:** Medium
- **Given** the common data and the content and used storage of `alpha` recorded
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/notes/a.txt`
- **Then** identical true, empty diff
- **And** the file and the used storage of `alpha` are unchanged
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-206: Windows against Unix line endings
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-016. **Priority:** High
- **Given** `alpha:/w.txt` = `one\r\ntwo\r\n` and `alpha:/u.txt` = `one\ntwo\n`
- **When** Alice compares `/w.txt` with `/u.txt`
- **Then** identical false; both lines appear as removed and re added (2 `-` lines, 2 `+` lines)
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-207: No final line ending
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-016, 001. **Priority:** Medium
- **Given** `alpha:/n1.txt` = `a` (no ending) and `alpha:/n2.txt` = `b` (no ending)
- **When** Alice compares `/n1.txt` with `/n2.txt`
- **Then** the diff is `--- alpha:/n1.txt`, `+++ alpha:/n2.txt`, `@@ -1 +1 @@`, `-a`, `+b`, each on its own
  line, with no "no newline" marker
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-208: Two distant changes
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001. **Priority:** Medium
- **Given** `alpha:/l1.txt` with lines `line1` to `line20`, and `alpha:/l2.txt` the same except `LINE1` and
  `LINE20`
- **When** Alice compares them
- **Then** the diff has exactly 2 blocks, the first changing `line1`, the second changing `line20`, each with
  at most 3 unchanged lines around the change
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-209: Accents and emoji
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, 002. **Priority:** Medium
- **Given** `alpha:/é/a.txt` = `café ☕\n` and `alpha:/é/b.txt` = `café 🍵\n`
- **When** Alice compares them
- **Then** the headers are `--- alpha:/é/a.txt` and `+++ alpha:/é/b.txt` and the lines `-café ☕` and
  `+café 🍵` appear intact
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-210: More than 2000 lines
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-016. **Priority:** High
- **Given** `alpha:/big1.txt` with 3000 lines `row1` to `row3000`, and `alpha:/big2.txt` identical except
  line 2999 is `CHANGED`
- **When** Alice compares them
- **Then** the diff contains `-row2999` and `+CHANGED`, which sit beyond line 2000, and the diff carries
  no truncation notice
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-211: A plain member
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: green on arrival (inherited authorization)
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-004, 005. **Priority:** Medium
- **Given** Carol (carol@example.com), a plain member (not owner) of `alpha` and `beta`
- **When** Carol compares `alpha:/cfg/app.txt` with `beta:/cfg/app.txt`
- **Then** she receives the same diff as in E2E-002
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-225: Path with doubled separators
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-017. **Priority:** Medium
- **Given** the common data
- **When** Alice compares `alpha:/notes//a.txt` with `alpha:/notes/missing.txt`
- **Then** `ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found`, and a successful comparison of
  `alpha:/notes//a.txt` with `alpha:/notes/b.txt` shows the header `--- alpha:/notes/a.txt`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

## Constraints
- Files not to touch: `core/fs_ops.rs`, `storage/volume.rs`, `config.rs`, `api/*`, README.md, AGENTS.md, `.agent_docs/*`.
- Dependencies not to add: none.
- Patterns to avoid: calling `safety.record_read` or `touch_atime`; changing `read_text`; hand editing `tool-contract-golden.json`.
- Scope boundary: refusal wording for missing file, directory, non text, size (US-0056, US-0057), last access (US-0058), REST (US-0059), docs counts (US-0060).
- Existing diff callers (`core/fs_ops.rs:952,1037,1068,1102`) must keep byte identical output: their tests stay unmodified.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, except the tests marked "Expectation on arrival" above, whose green status on arrival is recorded as such.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: All 12 unit tests of `core/diff.rs` and every existing `fs.write`/`fs.edit`/`fs.multi_edit` diff assertion pass unmodified; no existing test changes except the numeric tool counts and the tool name list named above.
- [ ] Docs updated: none in this story (counts in README, AGENTS.md and `.agent_docs` are US-0060).
- [ ] Self review (full): every requirement above is demonstrably met by a named test; no `unwrap()` on a fallible path; no dash as punctuation in code, comments or docs; errors carry the exact texts of this story.
- [ ] One commit `US-0055: compare two text files with fs.diff_files`, body `Spec: SPEC-0045` and the requirement ids.
