# US-0057: text only and a size limit

> Spec: SPEC-0045 (sdlc/specs/SPEC-0045-fs-diff-two-files/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0056
> min_tier: 2
> Files touched: 5
> Closes spec: no

## Objective
The comparison refuses files that are not text (a NUL byte or invalid UTF-8) and files over the operator limit (default 5 MiB, setting `safety.max_diff_file_bytes`), with the exact messages, and the server refuses to start when the setting is not a whole number of bytes above zero.

## Requirements covered
- SPEC-0045/FR-NEW-008: IF a file contains a NUL byte or bytes that are not valid UTF-8 THEN THE product SHALL refuse the comparison and name that file.
- SPEC-0045/FR-NEW-009: IF a file is larger than the diff size limit THEN THE product SHALL refuse the comparison and name that file, its size and the limit.

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e config::` : the 6 tests above pass. E2E-205 returns `ERR_INVALID_ARGUMENT: 'alpha:/logs/over.log' is 5242881 bytes, over the diff limit of 5242880 bytes`; E2E-221 shows `ServerConfig::from_yaml` failing with the pinned text for `0`, `-5` and `five megabytes`.

## Design excerpt
### Decisions
- (SPEC-0045) DEC-005: New `VolumeClient::read_text_checked(label, path, max_bytes)`: node lookup (not found), directory, size from `NodeRow.size` before any blob read, then raw bytes checked for a NUL byte and `String::from_utf8`. **Obliged by:** FR-NEW-006, 007, 008, 009. **Rationale:** the existing `read_text` is lossy and unlabelled (`volume.rs:119-122`, `:80`); changing it would alter every read. **Alternatives considered:** reuse `read_text` (cannot detect non text); change `read_text` (behavior change for every tool). **Source:** design. **Evidence:** `storage/volume.rs:76-105`, `storage/traits.rs:19`.
- (SPEC-0045) DEC-007: `SafetyConfig.max_diff_file_bytes: DiffLimit`, a newtype that deserializes any YAML scalar and keeps `Some(n)` only for an integer above zero; `ServerConfig::validate` fails with the pinned text when it is `None`. **Obliged by:** FR-NEW-009. **Rationale:** a plain `u64` field fails inside serde with the reader's own message (`config.rs:917-918`), never the pinned one, for `-5` and `five megabytes`. **Alternatives considered:** `u64` field (wrong message); custom `deserialize_with` raising the message (gets wrapped as `invalid config: ...`). **Source:** spec 14.4. **Evidence:** `config.rs:377-398,917-926`.
### Data model
Configuration field only.
### Contracts
```rust
// config.rs, in SafetyConfig (derives stay: Debug, Clone, Serialize, Deserialize; #[serde(default)] already on the struct)
pub max_diff_file_bytes: DiffLimit,           // default DiffLimit::new(5_242_880)
/// Operator value kept lenient so validate() can emit the pinned message (DEC-007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLimit(Option<u64>);
impl DiffLimit { pub fn new(bytes: u64) -> Self; pub fn bytes(&self) -> Option<u64>; }
// Deserialize: accept any YAML scalar via serde_norway::Value; Some(n) only for an integer n > 0 (i64 or u64), else None.
// Serialize: the number when Some, unit/null when None.
```
`ServerConfig::validate` (config.rs:926) gains, before the other checks: if `self.safety.max_diff_file_bytes.bytes().is_none()` return `Err(ToolError::invalid_argument("invalid safety.max_diff_file_bytes: must be a whole number of bytes above zero"))`. The rendered error is `ERR_INVALID_ARGUMENT: invalid safety.max_diff_file_bytes: must be a whole number of bytes above zero`.
`read_text_checked(label, path, max_bytes)` now: after the directory check, if `node.size as u64 > max_bytes` return `ERR_INVALID_ARGUMENT: '<label>' is <size> bytes, over the diff limit of <max_bytes> bytes` BEFORE any blob read; then read the bytes; if they contain `0u8` or `String::from_utf8` fails return `ERR_INVALID_ARGUMENT: '<label>' is not a text file`; an empty file is text. The orchestrator passes `state.safety.config().max_diff_file_bytes.bytes()` (it is always `Some` after boot validation; do not default with `unwrap_or`, map `None` to `ToolError::internal`).
A changed limit takes effect at the next server start (the value is read from the loaded config).
Documentation row for `.agent_docs/config.md`, in the `safety` table next to `max_read_lines` (line 131): `| max_diff_file_bytes | int | 5242880 | largest file fs.diff_files accepts, per file; a value that is not a whole number above zero stops the boot |`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/volume.rs` | modify |
| `crates/core/src/config.rs` | modify (field, DiffLimit, validate, tests, both Debug snapshot constants at lines 1633 and 1634) |
| `crates/core/src/tools/diff_files.rs` | modify |
| `crates/core/src/tools/diff_files_e2e.rs` | modify |
| `.agent_docs/config.md` | modify |
### Existing patterns
- Field and default style: `crates/core/src/config.rs:360-400` (`d_max_read_lines`, `SafetyConfig`, `Default`). Validation entry: `config.rs:926-935`. Config tests with YAML: `config.rs:1217-1240` (`serde_norway::from_str(...)`, `ServerConfig::from_yaml`).
- The two Debug snapshot constants (`LOCAL_TEMPLATE_SNAPSHOT`, `MINIO_TEMPLATE_SNAPSHOT`) contain `SafetyConfig { ... project_purge_grace_days: 30 }`; add `, max_diff_file_bytes: DiffLimit(Some(5242880))` before the closing brace in both, and nothing else.
- Limit tests: build the fixture with `Fixture::with_config(|c| c.safety.max_diff_file_bytes = DiffLimit::new(6_291_456))` (`tools/admin.rs:712`), restart is modelled by building a new fixture.

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions of all tests (copied from the spec):

Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

### SPEC-0045/E2E-104: A file with a NUL byte
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** `alpha:/img/logo.png` holding the 8 bytes `89 50 4E 47 0D 0A 1A 00` (it contains a NUL)
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/img/logo.png`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/img/logo.png' is not a text file`, and no partial diff
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-105: A file with invalid UTF-8
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-008. **Priority:** High
- **Given** `alpha:/data/latin1.txt` holding the 4 bytes `63 61 66 E9` (`caf` then a lone byte E9, no NUL)
- **When** Alice compares `alpha:/data/latin1.txt` with `alpha:/notes/a.txt`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/data/latin1.txt' is not a text file`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-106: A file over the limit
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** default limit; `alpha:/logs/big.log` of 5242881 bytes of the letter `a`
- **When** Alice compares `alpha:/notes/a.txt` with `alpha:/logs/big.log`
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/logs/big.log' is 5242881 bytes, over the diff limit of 5242880 bytes`
- **And** after the operator sets the limit to 6291456 bytes and restarts the server, the same request succeeds
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-204: Exactly at the limit
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** default limit; `alpha:/logs/edge.log` of exactly 5242880 bytes of the letter `a` (no line ending)
- **When** Alice compares it with `alpha:/notes/a.txt`
- **Then** a diff is returned, no refusal
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-205: One byte over the limit
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** High
- **Given** default limit; `alpha:/logs/over.log` of 5242881 bytes
- **When** Alice compares `alpha:/notes/a.txt` with it
- **Then** `ERR_INVALID_ARGUMENT: 'alpha:/logs/over.log' is 5242881 bytes, over the diff limit of 5242880 bytes`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-221: Invalid limit at start up
- Level / Driver / Location: integration / `ServerConfig::from_yaml` (what boot calls) / crates/core/src/config.rs (tests module)
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-009. **Priority:** Medium
- **Given** an operator who sets the diff size limit to `0`, then to `-5`, then to `five megabytes`
- **When** the operator starts the server with each value
- **Then** the server refuses to start each time, with the startup message pinned in Section 14.4 (the same
  text for the three values)
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

## Constraints
- Files not to touch: `core/diff.rs`, `api/*`, golden files, `config/*.yaml` templates.
- Dependencies not to add: none.
- Patterns to avoid: a plain `u64` field; applying the limit after the blob read; defaulting silently when the value is invalid.
- Scope boundary: last access and edit guard (US-0058), REST (US-0059).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, except the tests marked "Expectation on arrival" above, whose green status on arrival is recorded as such.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: Every existing config test passes except the two snapshot constants, which change only by the new field; the boot of `config/local.yaml` still validates.
- [ ] Docs updated: `.agent_docs/config.md` row for `max_diff_file_bytes`.
- [ ] Self review (full): every requirement above is demonstrably met by a named test; no `unwrap()` on a fallible path; no dash as punctuation in code, comments or docs; errors carry the exact texts of this story.
- [ ] One commit `US-0057: text only and a size limit`, body `Spec: SPEC-0045` and the requirement ids.
