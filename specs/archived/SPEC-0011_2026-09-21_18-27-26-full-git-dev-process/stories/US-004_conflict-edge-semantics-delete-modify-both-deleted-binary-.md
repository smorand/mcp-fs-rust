# US-004: Conflict edge semantics: delete/modify, both-deleted, binary, type change

> Parent Spec: `specs/archived/SPEC-0011_2026-09-21_18-27-26-full-git-dev-process/spec.md`
> Epic: n/a (tier 2 collapses the epic into the story)
> Status: ready
> Priority: 4
> Depends On: US-003
> Complexity: M
> min_tier: 2
> Files touched: 1

## Objective

Handle the conflict shapes that are not two edited text files: one side deleted while the other modified, both sides deleting the same path, binary content, and a file that became a directory. Each is surfaced rather than refused, and each has a defined resolution.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 (libgit2), rusqlite / sqlx / tiberius behind the `RelationalDb` abstraction, serde_json. Tests are `#[tokio::test]` in-crate.

### Relevant File Structure
```
crates/mcp-fs/src/tools/git.rs
```

### Existing Patterns
The existing three-way merge is `crates/mcp-fs/src/tools/git.rs:1835-1839` (libgit2 `merge_commits` + `MergeOptions::file_favor`); its refusal path is `crates/mcp-fs/src/tools/git.rs:1844-1849`.
Directory collision handling on write is `check_path_writable` (`crates/mcp-fs/src/tools/git.rs:2121-2140`).
Tests use the existing `Env` harness (`crates/mcp-fs/src/tools/git.rs:2371-2477`): `MOUNT`, `OWNER`, `Env::call`, `Env::as_person`, `Env::write`, `Env::read`, `Env::commit`. Remote tests seed real local bare repos (`seed_bare_remote`, `advance_bare_remote`) and drive internal functions over `file://` because the tool layer rejects non-HTTPS.

### Decisions That Govern This Story

Quoted from the specification's Interview Decisions Log. These are settled; do not re-litigate them.

- **DEC-902:** Conflicts are surfaced to the caller with both sides' content, and resolved per file, instead of being pre-resolved by one global strategy. **Rationale:** the caller cannot make a sensible global choice before seeing what it is choosing about; a global `theirs` silently discards local work in every conflicting file. The MCP client is an LLM that can genuinely read both sides and decide. **Alternatives considered:** keep the global strategy; refuse conflicted operations entirely. **Implemented by:** FR-NEW-170 through FR-NEW-186. **Round:** 1 (Q2). **Code evidence:** `crates/mcp-fs/src/tools/git.rs:1835-1839` (global `file_favor` today), `:1475-1481` (`parse_on_conflict`).

- **DEC-909:** A delete-versus-modify conflict, and a file-versus-directory type change, are surfaced as conflicts rather than refused up front with `ERR_INVALID_ARGUMENT`. **Rationale:** the shared model's premise is that combine operations surface rather than refuse; refusing would send the caller back to manual work for a case it can decide. **Alternatives considered:** up-front refusal, which the independent test designer flagged as the equally defensible branch. **Implemented by:** FR-NEW-180, FR-NEW-183. **Round:** 4 (raised by the test designer, resolved at test-plan review). **Code evidence:** n/a, new behaviour.

### Applicable NFRs

- 7.2 Security: authorization is membership-only; platform admin confers no implicit access.
- 7.5 Observability: destructive operations are audited; tokens are never logged or traced.

### Bounded Context
**Git operation** — as defined in the specification's Bounded Contexts section.

## Functional Requirements

4 requirements, quoted verbatim from the specification. The EARS statement is normative.

### FR-NEW-180 [EARS-E]: A delete/modify conflict is surfaced, not refused
> WHEN one side deletes a path and the other modifies it THE mcp-fs server SHALL surface it as a conflict entry whose deleted side is explicitly null, and SHALL accept `ours`, `theirs` or supplied content as its resolution.

- **Business Rules:** Choosing the deleting side removes the file; choosing the other side keeps it. Deliberately surfaced rather than refused, consistent with DEC-902 (DEC-909 records the alternative that was rejected).
- **Priority:** Must-have

### FR-NEW-181 [EARS-E]: A both-deleted path resolves to deletion
> WHEN both sides delete the same path THE mcp-fs server SHALL treat it as auto-resolved by deletion and SHALL NOT include it in the conflict set.

- **Priority:** Should-have

### FR-NEW-182 [EARS-E]: A binary conflict is surfaced with a binary marker
> WHEN a conflicting path holds content that is not valid UTF-8 THE mcp-fs server SHALL mark that conflict entry `binary: true`, SHALL omit the inline content, and SHALL accept only `ours` or `theirs` as its resolution.

- **Business Rules:** Supplying literal `content` for a binary path through a JSON string field cannot round-trip arbitrary bytes, so it is rejected rather than corrupting data.
- **Priority:** Must-have

### FR-NEW-183 [EARS-O]: A type change is surfaced as a conflict
> IF a path is a file on one side and a directory on the other THEN THE mcp-fs server SHALL surface it as a conflict entry with a `type_change: true` marker and SHALL accept only `ours` or `theirs` as its resolution.

- **Priority:** Should-have

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly.

13 tests: EdgeCase 7, Failure 3, Happy 1, SideEffect 2.
Scenarios covered: SC-904, SC-905.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| Seed repositories and commits | Each test body below names its own preconditions with exact file contents and commit messages | fixture, built in-test via the existing `Env` harness | ready |
| Bare remotes | `seed_bare_remote` / `advance_bare_remote` helpers already in the git test module | fixture | ready |

#### E2E-NEW-563 — both sides deleted the same file

**Scenario:** SC-904. **Requirements:** FR-NEW-181.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-DELDEL.
**When** `git.merge {source_ref:"feature"}`.
**Then** `conflicts` contains exactly one entry, `/keep.txt`; `/tmp/scratch.txt` is **not** listed — an identical deletion on both sides is not a conflict.
**When** resolved with `{path:"/keep.txt", content:"k-merged\n"}`.
**Then** `status == "merged"`; `fs.exists("/tmp/scratch.txt")` is false; `/keep.txt` reads exactly `k-merged\n`; the merge commit's tree has no entry `tmp/scratch.txt`.
**Verification:** conflicts path list equality; `fs.exists`; file read; `tree().get_path()` returns `Err`.

#### E2E-NEW-564 — one side deleted, the other modified

**Scenario:** SC-904. **Requirements:** FR-NEW-180.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-DELMOD (feature deletes `/lib/util.rs`, main modifies it).
**When** `git.merge {source_ref:"feature"}`.
**Then** `status == "conflict"`; `conflicts[0].path == "/lib/util.rs"`; `conflicts[0].theirs.exists == false` with `content == null`; `conflicts[0].ours.exists == true` with content exactly `pub fn a() { println!("x"); }\n`; `conflicts[0].base.exists == true` with `pub fn a() {}\n`.
**When** resolved with `{path:"/lib/util.rs", strategy:"theirs"}` (take the deletion).
**Then** `status == "merged"`; `fs.exists("/lib/util.rs")` is false; the merge commit tree has no `lib/util.rs`; the parent directory `/lib` is handled consistently (assert the exact observed state: `fs.exists("/lib")` matches what the engine leaves, asserted as `true`, documenting that empty directories are not pruned).
**Verification:** JSON fields including explicit `null`; `fs.exists` both paths; tree lookup.

#### E2E-NEW-565 — binary file conflict

**Scenario:** SC-904. **Requirements:** FR-NEW-182.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-BINARY.
**When** `git.merge {source_ref:"feature"}`.
**Then** `status == "conflict"`; `conflicts[0].path == "/assets/logo.png"`; `conflicts[0].binary == true`; `ours.content` base64-decodes to exactly `[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x03]` and `theirs.content` to `...,0x00,0x02]`; `base.content` to `...,0x00,0x01]`. No line-level hunk fields present.
**When** resolved with `{path:"/assets/logo.png", strategy:"theirs"}`.
**Then** `read_bytes("/assets/logo.png")` equals exactly the 10-byte vector ending `0x00,0x02`, and its length is exactly 10 (no marker insertion, no text mangling).
**Verification:** base64 decode + byte-vector equality; `read_bytes` length and content.

#### E2E-NEW-566 — file/directory type change

**Scenario:** SC-904. **Requirements:** FR-NEW-183.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-TYPECHANGE (feature makes `/mod` a directory holding `/mod/inner.rs`; main keeps `/mod` a file).
**When** `git.merge {source_ref:"feature"}`.
**Then** the call returns `status:"conflict"` with `conflicts[0].type_change == true`. FR-NEW-183 decides this: a file-versus-directory type change is surfaced as a conflict, never refused, so `ERR_INVALID_ARGUMENT` is NOT an acceptable outcome here. The side objects carry exactly `exists` and `content` per FR-NEW-186, so the type change is readable only from `type_change`: assert `conflicts[0].type_change == true`, `conflicts[0].ours.exists == true` with the file's content, `conflicts[0].theirs.exists == false` and `conflicts[0].theirs.content == null`. The keys `ours.kind` and `theirs.kind` are emitted by no tool.
**And** in either case `/mod` still reads exactly `placeholder v2\n` and `fs.exists("/mod/inner.rs")` is false — nothing applied.
**And** `bytes_written` delta == 0.
**Verification:** JSON fields or error code; file read; `fs.exists`; quota delta.
**Cleanup:** `git.merge_abort` if in progress.

#### E2E-NEW-570 — resolution path collides with an existing directory

**Scenario:** SC-905. **Requirements:** FR-NEW-183, FR-NEW-175.
**Category:** EdgeCase. **Priority:** P0. **Preconditions:** SEED-COLLIDE — main has directory `/report/` containing `/report/q.csv` = `a,b\n`; feature has file `/report` = `r\n`.
**When** `git.merge {source_ref:"feature"}` and, if a conflict is surfaced, `git.merge_resolve {resolutions:[{path:"/report", strategy:"theirs"}]}`.
**Then** the resolve fails with `ERR_INVALID_ARGUMENT` whose message contains `/report` and `already exists as a directory`.
**And** `/report/q.csv` still reads exactly `a,b\n`; `fs.is_dir("/report")` is true; no commit created; op still in progress.
**Verification:** error code + substrings; file read; `fs.is_dir`; log length; relational count.
**Cleanup:** `git.merge_abort`.

#### E2E-NEW-836 — Happy — choosing the modifying side keeps the file with its exact bytes**

**Category:** Happy. **Scenario:** SC-904. **Requirements:** FR-NEW-180, FR-NEW-174.
- **Preconditions:** SEED-DELMOD (`feature` deletes `/lib/util.rs`; `main` rewrites it to `pub fn a() { println!("x"); }\n`). HEAD on `main`.
- **Given** `git.merge {source_ref:"feature"}` returns `status == "conflict"` with one entry: `path == "/lib/util.rs"`, `ours.exists == true`, `theirs.exists == false`, `base.exists == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/lib/util.rs", strategy:"ours"}]}`.
- **Then** `status == "merged"` and `files_changed == 1`.
- **And** `/lib/util.rs` reads exactly `b"pub fn a() { println!(\"x\"); }\n"` — the modifying side, byte for byte.
- **And** the merge commit has exactly 2 parents, parent 0 = `main`'s pre-merge tip, parent 1 = `feature`'s tip.
- **And** no `git_operations` row remains.
- **Verification:** conflict shape fields; byte-exact read; `parent_count()`/`parent_id()`; relational count.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-837 — Failure — an invented strategy for the deleted side is refused**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-180, FR-NEW-179.
- **Preconditions:** SEED-DELMOD, merge paused as above.
- **When** `git.merge_resolve {resolutions:[{path:"/lib/util.rs", strategy:"delete"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"delete"`, `"ours"` and `"theirs"` — the deleting side is chosen by naming the side, never by a special verb.
- **And** the `git_operations` row still lists `/lib/util.rs` in `conflicts`, with no recorded resolution for it.
- **And** `/lib/util.rs` still reads exactly `b"pub fn a() { println!(\"x\"); }\n"` (the pre-merge volume state).
- **And** `git.merge_resolve {resolutions:[{path:"/lib/util.rs", strategy:"theirs"}]}` then succeeds and the file is gone: `VolumeClient::read_bytes` errors `ERR_NOT_FOUND`.
- **Verification:** error code + three substrings; relational row read; byte compare; recovery call plus read failure.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-838 — SideEffect — the both-deleted path is absent from the conflict set and from the volume**

**Category:** SideEffect. **Scenario:** SC-904. **Requirements:** FR-NEW-181, FR-NEW-171.
- **Preconditions:** SEED-DELDEL (both branches delete `/tmp/scratch.txt`; `/keep.txt` is `k-main\n` vs `k-feature\n`). HEAD on `main`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** `status == "conflict"` and `conflicts` holds **exactly one** entry, `path == "/keep.txt"`; no entry has `path == "/tmp/scratch.txt"`.
- **And** while paused, `/tmp/scratch.txt` is still present in the volume with its base bytes `b"x\n"` (a conflict applies nothing, including the auto-resolved deletion).
- **When** `git.merge_resolve {resolutions:[{path:"/keep.txt", strategy:"ours"}]}`.
- **Then** `VolumeClient::read_bytes("/tmp/scratch.txt")` errors `ERR_NOT_FOUND` and the merge commit's tree holds no `tmp/` entry.
- **And** `/keep.txt` reads exactly `b"k-main\n"`.
- **Verification:** conflict array length and paths; volume read while paused; volume read and `git.show` tree listing after; byte compare.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-839 — EdgeCase — both sides delete an entire directory**

**Category:** EdgeCase. **Scenario:** SC-904. **Requirements:** FR-NEW-181.
- **Preconditions:** `C0` commits `/old/a.txt` = `"a\n"`, `/old/b.txt` = `"b\n"`, `/keep.txt` = `"k\n"`; `feature` deletes both files under `/old/` and sets `/keep.txt` = `"k-feature\n"`; `main` deletes both files under `/old/` and sets `/keep.txt` = `"k-main\n"`. HEAD on `main`.
- **When** `git.merge {source_ref:"feature"}`.
- **Then** `conflicts` holds exactly one entry, `/keep.txt`; neither `/old/a.txt` nor `/old/b.txt` appears.
- **When** `git.merge_resolve {resolutions:[{path:"/keep.txt", strategy:"theirs"}]}`.
- **Then** `fs.list {path:"/"}` returns entries whose names do not include `old`, and `fs.list {path:"/old"}` errors `ERR_NOT_FOUND`.
- **And** `/keep.txt` reads exactly `b"k-feature\n"`.
- **Verification:** conflict array; `fs.list` results; error assertion; byte compare.
- **Cleanup:** fixture drop. **Priority:** P2.

---

#### E2E-NEW-840 — Failure — literal content for a binary path is refused**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-182, FR-NEW-175.
- **Preconditions:** SEED-BINARY (`/assets/logo.png`, feature last byte `0x02`, main `0x03`), merge paused with `conflicts[0].binary == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", content:"\u0089PNG"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"binary"`, `"ours"` and `"theirs"`.
- **And** `/assets/logo.png` still reads the exact 10 bytes `89 50 4E 47 0D 0A 1A 08 00 03`-form of the pre-merge main side (`[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x03]`), compared as a byte slice.
- **And** the `git_operations` row retains `/assets/logo.png` in `conflicts` with no recorded resolution.
- **And** `git.merge_resolve {resolutions:[{path:"/assets/logo.png", strategy:"theirs"}]}` then succeeds and the file's last byte is `0x02`.
- **Verification:** error code + substrings; byte-slice equality; relational row read; recovery call plus byte read.
- **Cleanup:** fixture drop. **Priority:** P0.

#### E2E-NEW-841 — SideEffect — the binary entry omits the bytes but reports the sizes**

**Category:** SideEffect. **Scenario:** SC-904. **Requirements:** FR-NEW-182, FR-NEW-170.
- **Preconditions:** SEED-BINARY, merge paused.
- **When** the conflict response is inspected.
- **Then** `conflicts[0]` is exactly `{"path":"/assets/logo.png","ours":{"exists":true,"content":null},"theirs":{"exists":true,"content":null},"base":{"exists":true,"content":null},"binary":true,"type_change":false}` — per FR-NEW-186 a binary side reports `exists` with `content` null, and **no** `content` key.
- **And** the serialized response, converted to a string, contains none of the byte sequences `"\u0000"`, `"PNG"` or `"\u0089"` (no smuggled inline bytes, no lossy UTF-8 replacement characters either: the string contains no `U+FFFD`).
- **And** the whole response is valid JSON that round-trips through `serde_json::from_str` unchanged.
- **Verification:** exact JSON object equality; substring absence on the serialized form; round-trip equality.
- **Cleanup:** `git.merge_abort`. **Priority:** P0.

---

#### E2E-NEW-907 — EdgeCase — one mixed resolution call spanning a text, a binary and a delete/modify path**

**Category:** EdgeCase. **Scenario:** SC-905. **Requirements:** FR-NEW-176, FR-NEW-180, FR-NEW-182.
Existing angles: 505 (three text files), 521 (Failure, strategy+content together). This one mixes conflict *kinds* in one call.
- **Preconditions:** a composite seed on `proj`: `/src/config.toml` (text, both sides edit line 2), `/assets/logo.png` (binary, last byte `0x02` vs `0x03`), `/lib/util.rs` (feature deletes, main modifies). `git.merge {source_ref:"feature"}` returns three conflict entries.
- **When** `git.merge_resolve {resolutions:[{path:"/src/config.toml", content:"[server]\nport = 8500\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n"}, {path:"/assets/logo.png", strategy:"theirs"}, {path:"/lib/util.rs", strategy:"ours"}]}`.
- **Then** `status == "merged"` and `files_changed == 3`.
- **And** `/src/config.toml` reads exactly the supplied string's bytes (line 2 `port = 8500`, a value on neither side).
- **And** `/assets/logo.png` reads exactly `[0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x02]`.
- **And** `/lib/util.rs` reads exactly `b"pub fn a() { println!(\"x\"); }\n"`.
- **And** no `git_operations` row remains and the merge commit has 2 parents.
- **Verification:** three byte-exact reads; response fields; relational count; `parent_count()`.
- **Cleanup:** fixture drop. **Priority:** P0.

---

#### E2E-NEW-909 — Failure — literal content is refused for a type-change conflict**

**Category:** Failure. **Scenario:** SC-904. **Requirements:** FR-NEW-183, FR-NEW-175.
Existing angles: 566 (the conflict shape), 570 (path colliding with a directory). This one covers the resolution restriction the requirement states.
- **Preconditions:** SEED-TYPECHANGE (`/mod` a file on main = `"placeholder v2\n"`, a directory containing `/mod/inner.rs` on feature); merge paused with `conflicts[0].type_change == true`.
- **When** `git.merge_resolve {resolutions:[{path:"/mod", content:"whatever\n"}]}`.
- **Then** assert error `ERR_INVALID_ARGUMENT` containing `"type_change"` (or `"type change"`), `"ours"` and `"theirs"`.
- **And** `/mod` still reads exactly `b"placeholder v2\n"` and `/mod/inner.rs` does not exist.
- **And** the `git_operations` row still lists `/mod` unresolved.
- **And** `git.merge_resolve {resolutions:[{path:"/mod", strategy:"theirs"}]}` then succeeds: `/mod/inner.rs` reads exactly `b"pub const N: u8 = 1;\n"` and `read_bytes("/mod")` errors (it is a directory now).
- **Verification:** error code + substrings; volume reads; relational row; recovery call and post-state reads.
- **Cleanup:** fixture drop. **Priority:** P1.

---

## Constraints

### Files Not to Touch
`crates/mcp-fs/src/api/dataplane.rs` and `api/openapi.rs`: git is MCP-only and this specification adds no REST route. `crates/mcp-fs/src/storage/` blob layout: git objects keep the `git:{sha}` key form.

### Dependencies Not to Add
No new crates. `git2`, `serde_json` and `reqwest` are already workspace dependencies; the specification adds none (Impact Analysis, Dependencies & Risks).

### Patterns to Avoid
Do not reimplement an operation in the tool layer: the engine lives in one place and the tool is a thin adapter. Do not build a `LIKE` pattern by hand; use `descendant_pattern` / `Dialect::escape_like_literal`. Do not block a request thread on the database or on libgit2.

### Scope Boundary
Only the requirements listed above. Anything else in the parent specification belongs to another story.

## Non Regression

### Existing Tests That Must Pass
The whole existing suite, unmodified, except where the parent specification lists a test as `E2E-MOD-*` or `E2E-DEL-*` (Section 12.3 and 12.4). `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` must all be clean.

### Behaviors That Must Not Change
Clone, commit, log, show, diff, blame, tags and the git HTTP smart-protocol routes keep their current behaviour. Membership-only authorization is unchanged. Conflict markers never enter the volume.

### API Contracts to Preserve
The frozen tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`) changes only where this story adds or modifies a tool. Regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff; any change beyond this story's tools is an accident.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5: spec compliance, unnecessary complexity, native idiom, dead code. In addition, confirm explicitly:

- Every response key emitted matches `FR-NEW-199`'s table exactly, with no extra and none missing.
- Every new ref-mutating tool takes the per-project write lock (`crates/mcp-fs/src/git/repo.rs:44`).
- Every new tool calls `state.authorize(mount_id, person)` before any other work.
- No token value reaches a response, an error, a log line or a tracing span.
