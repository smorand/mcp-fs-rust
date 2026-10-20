# SPEC-0045: compare two text files, design

> Spec: sdlc/specs/SPEC-0045-fs-diff-two-files/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-20
> Target tier: 2 (default, no `--tier`, no `Tier:` field in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust, edition 2024 | per `rust-toolchain.toml` (SPEC-0042) | DEC-015 |
| MCP | rmcp `#[tool_router]` (existing `McpServer`) | 3.5.0 (`Cargo.toml`) | DEC-001 |
| HTTP | axum (existing data plane router) | as `Cargo.lock` | DEC-010 |
| Diff | `similar`, Myers (existing) | `similar = "2"` (`Cargo.toml:86`) | DEC-002 |
| Config | `serde_norway` YAML (existing) | as `Cargo.lock` | DEC-007 |

No new dependency, so no context7 lookup is needed (nothing is added to a manifest).
Skills loaded: `rust` (via `load_dynamic_context`). Effect: functional e2e at the real entry points, no
`unwrap()` on fallible paths, CPU bound work in `spawn_blocking`, `#[tracing::instrument]` at boundaries,
thiserror style errors (here the project's own `ToolError`). Where the project's `AGENTS.md` differs from the
skill (own `Makefile` targets are wrappers of `./test.sh`, `cargo clippy ... -D warnings`, `cargo fmt`), the
project wins; its gate is `make check`.

## 2. Approaches
### Chosen: one shared orchestrator in a new tools module, thin MCP and REST adapters
`tools/diff_files.rs` holds `diff_files(state, person, a, b)`: normalize both paths, authorize and open both
projects, read both files with the strict reader, build the labelled diff off the async worker, touch last access
on success. `McpServer::fs_diff_files` and the REST handler only parse their inputs and call it. Pros: one
implementation (AGENTS.md convention, SPEC-0004 FR-009), one place for the check order (FR-NEW-015). Cons: a
tool that is not scoped to one project does not fit the `self.volume(mount)` helper, so the orchestrator
authorizes explicitly. Affected scope: `core/diff.rs`, `storage/volume.rs`, `tools/`, `mcp/server.rs`,
`api/`, `config.rs`.

### Rejected: put the whole operation in `core/fs_ops.rs`
Every function there takes one `VolumeClient`; two projects would need a second client parameter on a file that
is already about 4000 lines. Rejected for readability (DEC-001). `tools/export.rs` already shows the other
pattern: an operation module with a `pub(crate)` entry point that both transports call.

### Rejected: implement the REST route as `/api/fs/{mount_id}/diff` with a `second_mount` parameter
Contradicts the pinned name and the symmetry of the two sides (spec 14.4). Rejected (DEC-010).

## 3. Architecture
```
MCP  fs.diff_files --\
                      >-- tools::diff_files::diff_files(state, person, side_a, side_b)
REST GET /api/fs/diff -/        1 normalize both paths          (SafetyManager::normalize_path)
                                2 authorize + client, A then B  (AppState::authorize, stores.client)
                                3 read A fully then B fully     (VolumeClient::read_text_checked)
                                4 compare + build diff          (spawn_blocking, core::diff::unified_labeled)
                                5 touch last access of A and B  (VolumeClient::touch_atime), success only
```
Bounded contexts of the spec: File storage (`storage/volume.rs`, `core/diff.rs`), Access control (existing
`AppState::authorize`), Programmatic access (`api/dataplane.rs`, `api/openapi.rs`), Safety (the read before
edit record is deliberately NOT called).

## 4. Data model
No persisted change. The one new value is a configuration field (section 5). No migration, no rollback.

## 5. Contracts and interfaces
**Tool** `fs.diff_files`. Required string arguments `mount_id_a`, `path_a`, `mount_id_b`, `path_b`. Description:
`Compare two text files (same or different project) and return a unified diff with 3 lines of context.`
Argument descriptions (frozen once written): `mount_id_a`: `Project/volume id of the old side.`;
`path_a`: `Absolute POSIX path of the old side within its volume.`; `mount_id_b`: `Project/volume id of the new side.`;
`path_b`: `Absolute POSIX path of the new side within its volume.` Annotations (test registry glue):
`readOnlyHint=true, idempotentHint=true, openWorldHint=false`.
Answer: `{"identical": bool, "diff": string}`, nothing else; `diff` is `""` when identical.

**Rust signatures** (all `pub(crate)` except the diff function):
```rust
// core/diff.rs
pub fn unified_labeled(old: &str, new: &str, old_label: &str, new_label: &str, context: usize) -> String;
// unified_with_context(old, new, path, context) == unified_labeled(old, new, path, path, context), byte identical.

// storage/volume.rs
pub async fn read_text_checked(&self, label: &str, path: &str, max_bytes: u64) -> Result<String>;

// tools/diff_files.rs
pub(crate) struct Side<'a> { pub mount_id: &'a str, pub path: &'a str }
pub(crate) async fn diff_files(state: &AppState, person: &str, a: Side<'_>, b: Side<'_>) -> Result<Value>;
```
**Errors** (exact, `Display` of `ToolError`, label = `<mount_id>:<normalized path>`):
- `ERR_NOT_FOUND: '<label>' not found`
- `ERR_INVALID_ARGUMENT: '<label>' is a directory`
- `ERR_INVALID_ARGUMENT: '<label>' is not a text file`
- `ERR_INVALID_ARGUMENT: '<label>' is <size> bytes, over the diff limit of <limit> bytes`
- forbidden, unknown project, NUL byte, over long path: the existing messages, unchanged
  (`storage/admin.rs:392-395`, `errors.rs:62`, `safety.rs:133,146`).
- REST missing value: `ERR_INVALID_ARGUMENT: '<name>' is required`, name in the order `mount_id_a`, `path_a`,
  `mount_id_b`, `path_b`; HTTP 400 (`errors.rs:128`).
**Header labels** `--- <mount_id>:<path>` and `+++ <mount_id>:<path>`.

**Configuration**: `safety.max_diff_file_bytes`, default `5242880`, whole number of bytes above zero.
Precedence for this server is `defaults < YAML` (`ServerConfig::from_yaml` expands `${VAR}` first,
`config.rs:894-923`); there is no CLI layer. Invalid value (zero, negative, non integer, not a number) makes
`ServerConfig::validate` fail with `ERR_INVALID_ARGUMENT: invalid safety.max_diff_file_bytes: must be a whole
number of bytes above zero`; the text after the code is the pinned startup message.

**REST** `GET /api/fs/diff?mount_id_a=&path_a=&mount_id_b=&path_b=`: bearer identity (`person_of`,
`api/dataplane.rs:213-217`), no project in the URL, answer and errors as above; manifest pair `("GET", "diff")`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/core/diff.rs` | modify: add `unified_labeled`, delegate `unified_with_context` | FR-NEW-002 | US-0055 |
| `crates/core/src/tools/diff_files.rs` | create: `Side`, `diff_files`, test registry `register()` | FR-NEW-001, 003, 004, 005 | US-0055 |
| `crates/core/src/tools/mod.rs` | modify: `mod diff_files`, `mod diff_files_e2e`, `register` call, name list | FR-NEW-001 | US-0055 |
| `crates/core/src/mcp/server.rs` | modify: `DiffFilesArgs`, `fs_diff_files`, count asserts 108 to 109 | FR-NEW-001 | US-0055 |
| `crates/core/src/tools/diff_files_e2e.rs` | create: acceptance tests | all | US-0055 to US-0060 |
| `crates/core/src/tools/contract_golden.rs` | modify: count asserts 108 to 109, 44 to 45 fs | FR-NEW-001 | US-0055 |
| `crates/agent/src/mcp.rs` | modify: catalogue count 108 to 109 | FR-NEW-001 | US-0055 |
| `tool-contract-golden.json` | regenerate (never hand edit) | FR-NEW-001 | US-0055 |
| `TOOL_CONTRACT.txt` | modify: tool entry and return shape | FR-NEW-001 | US-0055 |
| `crates/core/src/storage/volume.rs` | modify: `read_text_checked` | FR-NEW-006 to 009 | US-0056, US-0057 |
| `crates/core/src/config.rs` | modify: `max_diff_file_bytes`, `DiffLimit`, `validate`, snapshots | FR-NEW-009 | US-0057 |
| `.agent_docs/config.md` | modify: the new key | FR-NEW-009 | US-0057 |
| `crates/core/src/api/dataplane.rs` | modify: route, handler, `REST_ROUTES` | FR-NEW-013, 018 | US-0059 |
| `crates/core/src/api/openapi.rs` | modify: `Op` entry for `diff` | FR-NEW-013 | US-0059 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/api.md` | modify: 109 tools, 45 `fs.*`, new tool and route | FR-NEW-001, 013 | US-0060 |

Affected existing tests: keep all. Modify only numeric assertions (`reg.len()` 108 to 109 at
`contract_golden.rs:145,252,308,322`; `server.rs:4198,4219,4220`; `crates/agent/src/mcp.rs:319`), the
`SafetyConfig` Debug snapshots (`config.rs:1633,1634`) and the tool name list in `tools/mod.rs`. No behavior
change in any existing test. Existing `unified` callers (`core/fs_ops.rs:952,1037,1068,1102`) keep byte identical
output (DEC-002).

## 7. Compatibility and risks
- No consumer changes: purely additive tool, route and setting. No flag, no rollback beyond reverting.
- The tool count rises to 109; the agent CLI reads the catalogue (`crates/agent/src/mcp.rs`), updated in US-0055.
- Risk: CPU time of Myers on two 5 MiB files, bounded by the 10 s requirement (DEC-011); measured by E2E-230.
- Risk: `touch_atime` is a best effort background task (`storage/volume.rs:57-63`); FR-NEW-012 allows 2 s.
- Ordering constraint: US-0057 edits `read_text_checked` created by US-0056.

## 8. Security design
Not applicable: `Security: n/a` and the spec has no class sweep. Access control is nevertheless covered by E2E-107, 108, 226 (both projects checked before any file is looked up).

## 9. Test strategy
Driver for every test: the real `McpServer::fs_diff_files` handler (same dispatch as `app.rs`) or the REST router
through `tower::ServiceExt::oneshot`, on `tools::admin::test_support::Fixture` (pattern of
`tools/quota_e2e.rs`). Level is functional e2e unless noted.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001, 002, 003, 107, 108, 201, 202, 203, 206, 207, 208, 209, 210, 211, 225 | functional e2e | `McpServer::fs_diff_files` | `tools/diff_files_e2e.rs` | US-0055 |
| E2E-101, 102, 103, 109, 110, 111, 212, 217, 218, 219, 222, 226, 228, 229 | functional e2e | `McpServer::fs_diff_files` | `tools/diff_files_e2e.rs` | US-0056 |
| E2E-104, 105, 106, 204, 205 | functional e2e | `McpServer::fs_diff_files` | `tools/diff_files_e2e.rs` | US-0057 |
| E2E-221 | integration | `ServerConfig::from_yaml` (what boot calls) | `config.rs` tests | US-0057 |
| E2E-005, 006, 007, 213, 214, 215, 216 | functional e2e | `McpServer` (+ `fs_read`, `fs_edit`, `fs_stat`) | `tools/diff_files_e2e.rs` | US-0058 |
| E2E-004, 112, 220, 223, 224, 227 | functional e2e | REST router `oneshot` | `tools/diff_files_e2e.rs` | US-0059 |
| E2E-230 | functional e2e | `McpServer::fs_diff_files`, wall clock | `tools/diff_files_e2e.rs` | US-0060 |

Test data: generated in the tests (strings, byte vectors). Last access (E2E-006, 215, 216) is read with
`VolumeClient::stat(...).atime`, polled for up to 2 s. Tests that exercise behavior inherited from existing code
(access refusal, deleted project) are expected green on arrival, the story says so.

## 10. Decisions log
- **DEC-001:** The operation lives in a new `tools/diff_files.rs` (`diff_files` plus `Side`), called by the MCP
  adapter and the REST handler. **Obliged by:** SPEC-0045/FR-NEW-013. **Rationale:** one implementation for both
  surfaces; two projects do not fit `core::fs_ops` single client functions. **Alternatives considered:** `fs_ops`
  (section 2). **Source:** AGENTS.md Conventions, `tools/export.rs:58`. **Evidence:** `tools/export.rs:58`.
- **DEC-002:** Add `diff::unified_labeled(old, new, old_label, new_label, context)`; `unified_with_context`
  delegates with the same label twice. **Obliged by:** SPEC-0045/FR-NEW-002. **Rationale:** header needs two
  labels; the four existing callers must stay byte identical. **Alternatives considered:** a second diff routine
  (duplication); a post hoc string replace of the header (fragile for paths containing `+++`). **Source:** design.
  **Evidence:** `core/diff.rs:56,69`.
- **DEC-003:** `identical` is `old_text == new_text`, computed before diffing; `diff` is then `""`.
  **Obliged by:** FR-NEW-003. **Rationale:** both texts are valid UTF-8 at this point, so string equality is byte
  equality; line splitting is lossless (`diff.rs:17-45`). **Alternatives considered:** `diff.is_empty()` (same
  result, computed after the fact). **Source:** design. **Evidence:** `core/diff.rs:58-67`.
- **DEC-004:** Order: normalize path A, normalize path B, authorize and open A, authorize and open B, read A
  completely, read B completely. **Obliged by:** FR-NEW-015. **Rationale:** matches the spec's twelve checks;
  `AppState::authorize` already reports a deleted project before membership. **Alternatives considered:** reading
  both files concurrently (would reorder errors). **Source:** spec. **Evidence:** `mcp/server.rs:178-181`,
  `storage/admin.rs:392-395`.
- **DEC-005:** New `VolumeClient::read_text_checked(label, path, max_bytes)`: node lookup (not found), directory,
  size from `NodeRow.size` before any blob read, then raw bytes checked for a NUL byte and `String::from_utf8`.
  **Obliged by:** FR-NEW-006, 007, 008, 009. **Rationale:** the existing `read_text` is lossy and unlabelled
  (`volume.rs:119-122`, `:80`); changing it would alter every read. **Alternatives considered:** reuse
  `read_text` (cannot detect non text); change `read_text` (behavior change for every tool). **Source:** design.
  **Evidence:** `storage/volume.rs:76-105`, `storage/traits.rs:19`.
- **DEC-006:** On success only, call `touch_atime` on both files; never call `safety.record_read`.
  **Obliged by:** FR-NEW-011, FR-NEW-012. **Rationale:** `read_window` does both (`fs_ops.rs:65-66`); the spec
  wants only the second. **Alternatives considered:** touch before the diff (would touch on a refusal).
  **Source:** spec. **Evidence:** `core/fs_ops.rs:65-66`.
- **DEC-007:** `SafetyConfig.max_diff_file_bytes: DiffLimit`, a newtype that deserializes any YAML scalar and
  keeps `Some(n)` only for an integer above zero; `ServerConfig::validate` fails with the pinned text when it is
  `None`. **Obliged by:** FR-NEW-009. **Rationale:** a plain `u64` field fails inside serde with the reader's own
  message (`config.rs:917-918`), never the pinned one, for `-5` and `five megabytes`. **Alternatives
  considered:** `u64` field (wrong message); custom `deserialize_with` raising the message (gets wrapped as
  `invalid config: ...`). **Source:** spec 14.4. **Evidence:** `config.rs:377-398,917-926`.
- **DEC-008:** The answer is a `serde_json` object with exactly `identical` and `diff`. **Obliged by:**
  FR-NEW-003, 013. **Rationale:** pinned (spec 14.4). **Alternatives considered:** none. **Source:** spec
  constraint. **Evidence:** spec 14.4.
- **DEC-009:** Tool description and argument descriptions as in section 5; the golden file is regenerated with
  `MCPFS_REWRITE_TOOL_CONTRACT=1` and reviewed (one new entry, no other change); `TOOL_CONTRACT.txt` is edited
  by hand. **Obliged by:** FR-NEW-001. **Rationale:** AGENTS.md: golden never hand edited. **Alternatives
  considered:** none. **Source:** AGENTS.md. **Evidence:** `tools/contract_golden.rs`.
- **DEC-010:** REST route `GET /api/fs/diff` outside `{mount_id}`, own handler (no `guarded`, which authorizes
  one mount), authenticating with `person_of`; `("GET", "diff")` in `REST_ROUTES`; one `Op` in the OpenAPI
  table like `roots`. **Obliged by:** FR-NEW-013. **Rationale:** `roots` and `quota` already are routes not
  scoped to `{mount_id}`. **Alternatives considered:** POST with a JSON body (reads are GET in this plane).
  **Source:** spec 14.4. **Evidence:** `api/dataplane.rs:117,174-178,213-217`, `api/openapi.rs:536-538`.
- **DEC-011:** The comparison and the diff run inside `tokio::task::spawn_blocking`. **Obliged by:** spec 7.1
  (10 seconds for two 5 MiB files) and the rust skill rule on CPU bound work. **Rationale:** Myers on tens of
  thousands of lines must not hold an async worker. **Alternatives considered:** inline (blocks the runtime).
  **Source:** rust skill. **Evidence:** n/a.
- **DEC-012:** Acceptance tests live in `tools/diff_files_e2e.rs`, modelled on `tools/quota_e2e.rs`; the config
  test (E2E-221) is inline in `config.rs`. **Obliged by:** spec section 9. **Rationale:** project convention is
  inline `#[cfg(test)]` next to the code, families with cross file scenarios use an `_e2e` module. **Alternatives
  considered:** `crates/core/tests/` (the project keeps it empty, `.agent_docs/testing.md`). **Source:** existing
  code. **Evidence:** `tools/quota_e2e.rs:1-30`.
- **DEC-013:** Count updates (108 to 109, 44 to 45 `fs.*`) land with US-0055 in code and tests, and with
  US-0060 in README, AGENTS.md and `.agent_docs`. **Obliged by:** FR-NEW-001. **Rationale:** `make check` must be
  green on main after each story; docs may trail by the later stories. **Alternatives considered:** docs in
  US-0055 (adds three files to the heaviest story). **Source:** design. **Evidence:** `rg -n "\b108\b"` output.
- **DEC-014:** The REST missing value check is local to the new handler and spelled `'<name>' is required`;
  the existing `Q::req_str` message (`missing required argument '<key>'`, `api/dataplane.rs:277-281`) is left
  untouched. A value of only spaces counts as empty. **Obliged by:** FR-NEW-018. **Rationale:** the spec pins
  a different text; changing the shared helper would alter every route. **Alternatives considered:** change
  `Q::req_str` (breaks existing REST tests). **Source:** spec. **Evidence:** `api/dataplane.rs:265-285`.
- **DEC-015:** No new dependency, no schema or data change, no feature flag. **Obliged by:** n/a (no code
  impact): records that ASK ONLY FOR categories 1, 2 and 4 do not apply. **Rationale:** `similar`, rmcp, axum,
  serde_norway already carry everything. **Alternatives considered:** a dedicated text detection crate (a new
  dependency for a two line check). **Source:** design. **Evidence:** `Cargo.toml:86`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0055 | compare two text files with fs.diff_files | P1 | FR-NEW-001, 002, 003, 004, 005, 016, 017 | main | 1 | no |
| US-0056 | refuse what cannot be compared | P1 | FR-NEW-006, 007, 014, 015, 017 | US-0055 | 2 | no |
| US-0057 | text only and a size limit | P1 | FR-NEW-008, 009 | US-0056 | 2 | no |
| US-0058 | a comparison is read only and counts as access | P2 | FR-NEW-010, 011, 012 | US-0055 | 2 | no |
| US-0059 | the same comparison on the REST data plane | P2 | FR-NEW-013, 018 | US-0056 | 2 | no |
| US-0060 | large comparison bound, docs, spec close | P2 | spec 7.1 | US-0057, US-0058, US-0059 | 2 | yes |
