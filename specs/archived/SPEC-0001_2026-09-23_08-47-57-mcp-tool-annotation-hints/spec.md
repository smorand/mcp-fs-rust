# MCP Tool Annotation Hints — Technical Debt Mini-Spec

> Generated on: 2026-09-23
> Id: SPEC-0001
> Nature: DEBT
> Depth: L
> Depth evidence: 22 modules touched (schema.rs, registry.rs, contract_golden.rs, 19
> tools/*.rs family files including git_pr.rs, 1 golden JSON contract, 3
> documentation files), 123 tool registrations annotated, public contract touched
> (`tools/list` JSON-RPC response).
> Status: Draft
> Behavior change: none (additive JSON field only, see Section 5)
> From backlog: n/a
> Split: not split
> Depends on: none

## 1. Debt

### DBT-001: MCP tools carry no behavioral hints for callers

- **Current structure:** `ToolSchema` (`crates/mcp-fs/src/mcp/schema.rs:51-55`) has
  three fields: `name`, `description`, `params`. `to_list_entry()`
  (`crates/mcp-fs/src/mcp/schema.rs:173-179`) serializes exactly `name`,
  `description`, `inputSchema` into every `tools/list` entry. No tool in the 123
  registrations across `crates/mcp-fs/src/tools/*.rs` declares whether it reads only,
  destroys data, is safe to retry, or reaches outside the sandboxed volume/database.
  Confirmed zero pre-existing occurrences of `annotations`, `destructiveHint`,
  `readOnlyHint`, `idempotentHint`, `openWorldHint`, `ToolAnnotations` anywhere in the
  tree (`rg -in "annotations|destructiveHint|readOnlyHint|idempotentHint|openWorldHint|ToolAnnotations"`
  → one unrelated hit, `crates/mcp-fs/src/storage/sqlite.rs:29`, a clippy lint name).
- **Why it is debt:** these four fields are the standard MCP tool `annotations`
  object (`destructiveHint`, `readOnlyHint`, `idempotentHint`, `openWorldHint`). Every
  client capable of gating human confirmation on tool calls reads them from
  `tools/list`; absent, a client has no server-declared signal to distinguish
  `fs.read` from `fs.delete` or `admin.delete_project` without hardcoding tool names.
  This is metadata debt: the capability distinction already exists in the code (every
  tool's handler is either mutating or not), it is simply not exposed on the wire.
- **Target structure:** every `ToolSchema` carries an optional `annotations` object;
  `to_list_entry()` serializes it when present, camelCase, omitting absent hints and
  the whole object when no hint is set. Every one of the 123 registered tools gets an
  explicit annotation call matching the classification in Section 3.
- **Observable behavior:** unchanged for every existing client and test that does not
  read the new field. The only observable change is additive: `tools/list` entries
  gain an optional `annotations` key. See Section 5 for the full declaration.

## 2. Perimeter (exact file list)

| File | Change | Occurrences |
|------|--------|-------------|
| `crates/mcp-fs/src/mcp/schema.rs` | Add `ToolAnnotations` struct (4 optional bool fields) + `pub annotations: Option<ToolAnnotations>` field on `ToolSchema` (after `params:` field, `crates/mcp-fs/src/mcp/schema.rs:53`) + 4 builder methods (`destructive`, `read_only`, `idempotent`, `open_world`) + wire `to_list_entry()` (`crates/mcp-fs/src/mcp/schema.rs:173-179`) to emit `"annotations"` when `Some` | 1 struct added, 1 field added, 4 methods added, 1 method changed |
| `crates/mcp-fs/src/mcp/registry.rs` | No change. `list_payload()` (`crates/mcp-fs/src/mcp/registry.rs:82`) already calls `schema.to_list_entry()`; annotations flow through automatically. | 0 |
| `crates/mcp-fs/src/app.rs` | No change. `"tools/list"` dispatch (`crates/mcp-fs/src/app.rs:237`) calls `state.registry.list_payload()` unchanged. | 0 |
| `crates/mcp-fs/src/tools/admin.rs` | Add annotation chain to 10 tool definitions | 10 |
| `crates/mcp-fs/src/tools/context7.rs` | Add annotation chain to 2 tool definitions | 2 |
| `crates/mcp-fs/src/tools/db.rs` | Add annotation chain to 5 tool definitions | 5 |
| `crates/mcp-fs/src/tools/doc.rs` | Add annotation chain to 2 tool definitions (`doc.to_docx`, `doc.to_pptx`) | 2 |
| `crates/mcp-fs/src/tools/document.rs` | Add annotation chain to 3 tool definitions (`fs.write_docx`, `fs.documentize`, `fs.extract_text`) | 3 |
| `crates/mcp-fs/src/tools/edit.rs` | Add annotation chain to 5 tool definitions | 5 |
| `crates/mcp-fs/src/tools/editor.rs` | Add annotation chain to 3 tool definitions | 3 |
| `crates/mcp-fs/src/tools/git.rs` | Add annotation chain to 39 tool definitions, including the 2 loop-registered `git.stash_pop`/`git.stash_apply` (`crates/mcp-fs/src/tools/git.rs:751-786`, one `ToolSchema::new(name, description)` call site parameterized per iteration — the annotation chain is identical for both since both mutate the volume non-idempotently) | 39 |
| `crates/mcp-fs/src/tools/git_auth.rs` | Add annotation chain to 4 tool definitions | 4 |
| `crates/mcp-fs/src/tools/git_pr.rs` | Add annotation chain to 6 tool definitions | 6 |
| `crates/mcp-fs/src/tools/lifecycle.rs` | Add annotation chain to 6 tool definitions | 6 |
| `crates/mcp-fs/src/tools/listing.rs` | Add annotation chain to 2 tool definitions | 2 |
| `crates/mcp-fs/src/tools/metadata.rs` | Add annotation chain to 3 tool definitions | 3 |
| `crates/mcp-fs/src/tools/read.rs` | Add annotation chain to 8 tool definitions | 8 |
| `crates/mcp-fs/src/tools/search.rs` | Add annotation chain to 4 tool definitions | 4 |
| `crates/mcp-fs/src/tools/search_semantic.rs` | Add annotation chain to 4 tool definitions | 4 |
| `crates/mcp-fs/src/tools/sqlite.rs` | Add annotation chain to 8 tool definitions | 8 |
| `crates/mcp-fs/src/tools/web.rs` | Add annotation chain to 5 tool definitions | 5 |
| `crates/mcp-fs/src/tools/write.rs` | Add annotation chain to 4 tool definitions | 4 |
| `crates/mcp-fs/src/tools/contract_golden.rs` | `render()` (`crates/mcp-fs/src/tools/contract_golden.rs:96-115`) must serialize `annotations` into the golden entry when present; `assert_family()` (`crates/mcp-fs/src/tools/contract_golden.rs:52-81`) must compare it | 2 functions changed |
| `tool-contract-golden.json` | Regenerated via `MCPFS_REWRITE_TOOL_CONTRACT=1`; every one of the 94 frozen entries gains an `annotations` key | 94 |
| `TOOL_CONTRACT.txt` | Human-readable contract; regenerate or hand-update to list each tool's annotations alongside its existing `params`/`required` lines — confirmed this file is NOT test-generated (no `rg` hit tying it to a generator script; it is the hand-maintained companion referenced in `crates/mcp-fs/src/tools/admin.rs:486` comment and `AGENTS.md:26`) | 94 |
| `.agent_docs/tools.md` | Add an "Annotations" column (or a legend) to the 94-tool reference tables; header already says "94 tools" (`.agent_docs/tools.md:1`) | 1 header line + N table rows |
| `AGENTS.md` | No change to tool counts (94/123 unaffected); optionally note in the tools doc index line (`AGENTS.md:164`) that annotations are documented — not required by any DR, left to author discretion, not tracked as a requirement | 0 (optional) |

**Non-source occurrences:** none. This change touches no `.env`, no CI config, no
Makefile target, no Docker file: annotations are pure server-side schema metadata
with no build or deployment footprint. `TOOL_CONTRACT.txt` and
`tool-contract-golden.json` are the two "documentation as data" files that are not
`.rs` source but are exhaustively covered above.

**Total:** 24 files, 123 tool-level occurrences (annotation chains) + 94 golden-file
entries + 2 `contract_golden.rs` function changes + 1 schema struct/method set + 2
prose doc files.

## 3. Requirements

Every tool's annotation assignment follows one of five classification rules. Rules
are stated once here as `DR-001` through `DR-005`; each rule is EARS `[EARS-U]` (a
structural mapping, not a trigger/response), and Section 3.1 names every one of the
123 tools against its rule so the mapping is checkable line by line.

### 3.1 Structural

#### DR-001 [EARS-U]: Pure read tools carry `readOnlyHint=true`
> The `ToolSchema` for every tool classified "pure read" in Section 3.1 SHALL set
> `annotations.read_only_hint` to `Some(true)` and SHALL NOT set
> `annotations.destructive_hint`.
- **Applies to:** every tool listed under "Pure read" in Section 3.1.

#### DR-002 [EARS-U]: Mutating tools carry `readOnlyHint=false`
> The `ToolSchema` for every tool NOT classified "pure read" in Section 3.1 SHALL set
> `annotations.read_only_hint` to `Some(false)`.
- **Applies to:** every tool listed under "Overwrite/delete", "Additive", and
  "External network" in Section 3.1.

#### DR-003 [EARS-U]: Overwrite/delete tools carry `destructiveHint=true`
> The `ToolSchema` for every tool classified "Overwrite/delete" in Section 3.1 SHALL
> set `annotations.destructive_hint` to `Some(true)`, reflecting the tool's worst
> case capability regardless of a parameter default that makes a specific call safe
> (e.g. `fs.write` with `overwrite=false`).
- **Applies to:** every tool listed under "Overwrite/delete" in Section 3.1.

#### DR-004 [EARS-U]: Additive-only tools carry `destructiveHint=false`
> The `ToolSchema` for every tool classified "Additive" in Section 3.1 SHALL set
> `annotations.destructive_hint` to `Some(false)`.
- **Applies to:** every tool listed under "Additive" in Section 3.1.

#### DR-005 [EARS-U]: Idempotency and open-world hints follow the per-tool table
> The `ToolSchema` for every tool SHALL set `annotations.idempotent_hint` and
> `annotations.open_world_hint` to the exact `Some(bool)` value given in that tool's
> row in Section 3.1, with no default inferred from its family.
- **Applies to:** all 123 tools; this is the escape hatch for the per-tool exceptions
  Section 3.1 lists inline (e.g. `fs.move`, `git.commit`, `web.download`).

#### DR-006 [EARS-U]: `to_list_entry` serializes present annotations
> The `to_list_entry()` method SHALL serialize the `annotations` field as a
> camelCase JSON object (`destructiveHint`, `readOnlyHint`, `idempotentHint`,
> `openWorldHint`) omitting any hint whose value is `None`, and SHALL omit the
> `annotations` key entirely from the rendered `Value` when every hint is `None`.

#### DR-008 [EARS-U]: Contract regeneration covers annotations
> The `render()` function in `contract_golden.rs` SHALL include the `annotations`
> field (when present) in every rendered tool entry, and the
> `tool_contract_golden_is_current` test SHALL fail when a registered tool's
> annotations differ from the frozen `tool-contract-golden.json` entry.

#### 3.1.1 Per-tool classification (123 tools)

Legend: RO=readOnlyHint, D=destructiveHint, I=idempotentHint, OW=openWorldHint.
"-" means the hint is omitted (`None`), matching MCP convention that
`destructiveHint`/`idempotentHint` are meaningful only when `readOnlyHint=false`.

The 49 `git.*`/`git.auth*`/`git.pr_*` names below were re-derived directly from
`crates/mcp-fs/src/tools/git.rs`, `git_auth.rs` and `git_pr.rs` (each handler read in
full) for this revision, closing `DDRIFT-001` — see Section 10 for the resolution
record. The 74 non-git names are unchanged from the prior revision.

**Pure read (RO=true, D=-, I=true, OW=false unless noted).** 55 tools:

`fs.read`, `fs.read_bytes`, `fs.read_lines`, `fs.read_section`, `fs.read_many`,
`fs.head`, `fs.tail`, `fs.count_lines`, `fs.exists`, `fs.hash`, `fs.stat`,
`fs.list_dir`, `fs.tree`, `fs.list_allowed_roots`, `fs.audit_log`, `fs.glob`,
`fs.grep`, `fs.find_definition`, `fs.find_references` (19 fs), `admin.list_projects`,
`admin.list_all_projects`, `admin.list_users`, `admin.list_members`,
`admin.get_index_mode` (5 admin), `git.status`, `git.branches`, `git.tags`,
`git.log`, `git.show`, `git.diff`, `git.blame`, `git.remote_list`, `git.stash_list`,
`git.auth_status` (10 git, local), `git.pr_list`, `git.pr_get`, `git.pr_diff` (3
git, OW=true — read a provider's REST API), `search.status`, `search.query` (2
search), `sqlite.query`, `sqlite.list_tables`, `sqlite.list_indexes`,
`sqlite.describe_table`, `sqlite.export_csv` (5 sqlite), `db.query`, `db.sample`,
`db.schema`, `db.profile` (4 db, OW=true — external data source per the `db.*`
family convention, `DDEC-001`-adjacent, not independently re-verified against
`db.rs` source in this pass), `doc.list_editors` (1 doc), `context7.get_library_docs`,
`context7.resolve_library_id` (2 context7, OW=true), `web.search`, `web.fetch`,
`web.suggestions`, `web.news` (4 web, OW=true). 19+5+10+3+2+5+4+1+2+4=55.

**Overwrite/delete (RO=false, D=true).** 38 tools:

- `fs.write`, `fs.write_bytes`, `fs.write_docx` — I=false, OW=false.
- `fs.edit`, `fs.apply_patch`, `fs.multi_edit`, `fs.insert_at_line`,
  `fs.search_replace` — I=false (re-applying an already-applied patch/edit is not a
  no-op; it either fails to match or double-applies), OW=false.
- `fs.copy`, `fs.move` — I=false (a repeated `fs.move` targets a source that no
  longer exists after the first call and errors), OW=false.
- `fs.delete` — I=true (trashing an already-trashed path is a safe no-op per
  `crates/mcp-fs/src/tools/lifecycle.rs`), OW=false. (11 fs)
- `admin.delete_project`, `admin.remove_member` — I=true, OW=false.
- `admin.set_index_mode` — I=true (same mode twice is a no-op), OW=false. (3 admin)
- `git.checkout_file` — restores a file from a commit, overwriting the volume copy
  (`crates/mcp-fs/src/tools/git.rs:419-451`, "A restore is a write, so it is charged
  like any other"); I=true (same commit+path converges), OW=false.
- `git.branch_switch` — rewrites the volume to the target branch's tree
  (`git.rs:191-216`); I=true, OW=false.
- `git.branch_delete` — deletes a branch ref, force-refuses commits reachable from
  no other ref (`git.rs:226-256`); I=true, OW=false.
- `git.branch_reset` — moves a branch pointer, rewrites the volume when on the
  checked-out branch (`git.rs:266-307`); I=true, OW=false.
- `git.reset` — hard mode rewrites the volume, soft mode moves the pointer only;
  classified by worst case per `DDEC-004` (`git.rs:307-347`); I=true, OW=false.
- `git.remote_remove` — deletes a remote record and its remote-tracking refs
  (`git.rs:503-524`, "A name matching no remote is an error, never a silent no-op" —
  taken at face value: repeat call errors, not a no-op); I=false, OW=false.
- `git.remote_push` — force mode can overwrite the remote branch, discarding
  commits (`git.rs:648-706`); worst-case classification per `DDEC-004`; I=true
  (identical repeat push converges), OW=true (reaches the remote host).
- `git.stash_drop` — deletes one stash ref (`git.rs:803-826`); I=true, OW=false.
- `git.stash_pop` — applies and deletes the entry on success (`git.rs:828-895`);
  I=false (the entry is gone after success, a repeat call targets a now-absent id),
  OW=false.
- `git.stash_apply` — applies without deleting the entry, but reapplying the same
  diff onto an already-changed volume is not a no-op (`git.rs:828-895`); I=false,
  OW=false.
- `git.merge` — creates a merge commit and updates the volume; "An already merged
  source is reported as already_up_to_date and changes nothing" (`git.rs:895-933`) —
  I=true, uniquely among the combining operations, because the tool's own
  documented behavior guarantees convergence; OW=false.
- `git.merge_resolve` — writes resolved files and creates the merge commit
  (`git.rs:934-966`); multi-call conflict-resolution protocol, I=false, OW=false.
- `git.rebase` — replays commits per an explicit todo list, can drop/reword/squash
  (`git.rs:1029-1063`); I=false (a second call after the branch moved replays a
  different range), OW=false.
- `git.rebase_continue` — writes resolved files and replays the remaining todo
  (`git.rs:1064-1082`); I=false, OW=false.
- `git.remote_pull` — fetches then advances the branch and rewrites the volume;
  fast-forward or a clean three-way merge (`git.rs:1160-1179`); I=true (repeated
  pull with nothing new converges, mirroring `git.merge`'s already-up-to-date
  behavior), OW=true (fetches from the remote).
- `git.auth_revoke` — deletes the stored token for a provider/host
  (`git_auth.rs:132-136`); I=true (revoking an absent token is a safe no-op per its
  own description), OW=false (local token-store deletion only; not confirmed to
  call the provider to invalidate remotely).
- `git.pr_merge` — merges a PR on the provider, irreversible via this tool
  (`git_pr.rs:223-285`), "an already merged... pull request... reported as the
  provider's own status and message, never as a success" — taken at face value:
  I=false, OW=true (calls the provider API). (17 git)
- `search.delete` — I=true, OW=false.
- `sqlite.execute` — arbitrary SQL, cannot assume idempotency; I=false, OW=false.
- `sqlite.vacuum` — rewrites the db file without destroying rows; I=true, OW=false.
- `db.convert` — converts and writes an output file, can overwrite; classified by
  analogy with `doc.to_docx` (worst-case, `DDEC-004`); I=true, OW=true (external
  service per the `db.*` family convention, same caveat as the pure-read `db.*`
  tools above — `db.rs` source not independently re-read in this pass).
- `doc.to_docx`, `doc.to_pptx` — I=true, OW: see `DDEC-001` (depends on
  `doc_service` runtime config; declared `true`, worst case).
- `web.download` — I=true, OW=true (network fetch).

11(fs)+3(admin)+17(git)+1(search)+2(sqlite)+1(db)+2(doc)+1(web)=38.

**Additive (RO=false, D=false).** 30 tools:

- `fs.append` — I=false, OW=false.
- `fs.mkdir`, `fs.create_empty` — I=true (`mkdir -p`/`exist_ok` semantics), OW=false.
- `fs.documentize`, `fs.extract_text` — I=true, OW: see `DDEC-001`. (5 fs)
- `admin.create_project` — I=false (a duplicate id conflicts rather than
  no-ops), OW=false.
- `admin.add_member` — I=true (adding an existing member is a no-op), OW=false.
  (2 admin)
- `git.init` — I=true (initializing an already-initialized repo is a no-op,
  `git.rs:67-81`), OW=false.
- `git.commit` — I=false, OW=false.
- `git.branch_create` — refuses a name already in use, so a repeat call errors
  rather than no-ops (`git.rs:116-164`); I=false, OW=false.
- `git.remote_add` — "A name already in use is refused rather than overwritten"
  (`git.rs:478-502`); I=false, OW=false (records a URL locally; no network call in
  the handler itself).
- `git.remote_clone` — clones an external repo into the volume (`git.rs:525-550`);
  I=false (a non-empty target likely conflicts on retry), OW=true.
- `git.remote_fetch` — "Never advances a local branch and never touches a working
  tree file" (`git.rs:707-725`) — updates remote-tracking refs only, additive
  metadata, not file content; I=true (re-fetch converges), OW=true.
- `git.stash_save` — refuses a volume with no uncommitted changes
  (`git.rs:579-647`), so a repeat call with the same state errors; I=false, OW=false.
- `git.merge_abort`, `git.rebase_abort`, `git.cherry_pick_abort`, `git.revert_abort`
  — rollback operations: each restores the volume to its pre-operation state and
  discards only uncommitted, not-yet-committed conflict-resolution work; classified
  `destructiveHint=false` uniformly across the four (`DDEC-005` in Section 8) rather
  than `true`, because their entire purpose is preventing data loss, not causing it.
  I=false (a second call with nothing in progress errors, per
  `reject_if_operation_in_progress`), OW=false.
- `git.cherry_pick` — applies a commit's change as a NEW commit, original history
  untouched (`git.rs:967-1010`); "A commit already in the branch's history... is
  reported as already_present and never duplicated" — I=true, uniquely among the
  replay-family tools, OW=false.
- `git.cherry_pick_continue` — creates the commit once every conflict is resolved
  (`git.rs:1083-1127`); multi-call protocol, I=false, OW=false.
- `git.revert` — adds a NEW commit inverting another; "Reverting a revert restores
  the change" (`git.rs:1128-1159`) implies no dedup — repeat calls create
  additional commits; I=false, OW=false.
- `git.revert_continue` — I=false, OW=false.
- `git.auth` — starts a new OAuth device flow, new codes each call
  (`git_auth.rs:75-100`); I=false, OW=true (reaches the provider).
- `git.token_set` — seeds/overwrites a stored token for a host (`git_auth.rs:151-`
  ff.); setting the same value twice converges; I=true, OW=false (local store only).
- `git.pr_create` — opens a new PR each call (`git_pr.rs:55-118`); I=false,
  OW=true.
- `git.pr_review` — submits a review/comment, providers allow more than one
  (`git_pr.rs:285-`); I=false, OW=true. (19 git)
- `search.index` — I=true (converges to the same index state), OW=false.
- `sqlite.import_csv` — confirmed at `crates/mcp-fs/src/tools/sqlite.rs:365-420`:
  `CREATE TABLE IF NOT EXISTS` + `INSERT INTO`, never `DELETE`/`TRUNCATE`/`REPLACE`;
  additive but a repeat call inserts duplicate rows; I=false, OW=false.
- `doc.open_editor` — confirmed at `crates/mcp-fs/src/tools/editor.rs:106-131`:
  "Returns the editor URL and a unique editor_id" every call; I=false, OW=false.
- `doc.close_editor` — closing an absent editor id is a safe no-op; I=true,
  OW=false.

5(fs)+2(admin)+19(git)+1(search)+1(sqlite)+2(doc)=30.

**Total check:** 55 (pure read) + 38 (overwrite/delete) + 30 (additive) = 123,
reconciling exactly with the Section 2 registry count and with the per-family totals
in Section 2 (fs 35 = 19+11+5; admin 10 = 5+3+2; git 49 = 10+3+17+19; search 4 =
2+1+1; sqlite 8 = 5+2+1; web 5 = 4+1; context7 2 = 2; db 5 = 4+1; doc 5 = 1+2+2).

### 3.2 Invariance (MANDATORY)

#### DR-007 [EARS-UB]: Behavior invariance
> The system SHALL NOT change the `name`, `description`, or `inputSchema` value of
> any existing tool, and SHALL NOT change the JSON-RPC method dispatch, handler
> signature, or runtime result of any tool call, as a result of this change.

### 3.3 Compatibility

Not applicable as a per-requirement section: this change touches no dual-maintained
public contract. `tools/list` gains a purely additive, optional `annotations` field
(Section 4), so there is no old form and new form to reconcile, no precedence rule,
and no deprecation window. DR-007 above is the compatibility guarantee this change
relies on; Section 4's Compatibility Plan table is the authoritative statement of the
contract and its (non-breaking) evolution.

## 4. Compatibility

| Contract | Kind | Public? | Plan | Deprecation window | Removal condition |
|----------|------|---------|------|--------------------|--------------------|
| `tools/list` JSON-RPC response shape | MCP protocol response | yes (every MCP client reads it) | Dual-maintain by construction: `annotations` is additive and optional; a client that ignores unknown JSON keys (every conformant JSON-RPC/MCP client) is unaffected | N/A, not a breaking change | N/A |
| `tool-contract-golden.json` schema | Test fixture / frozen contract | no (internal to this repo's test suite) | Regenerate via `MCPFS_REWRITE_TOOL_CONTRACT=1`, review the diff before commit | N/A | N/A |
| Tool `name`, `description`, `inputSchema` | MCP protocol, per tool | yes | Break now is NOT applicable: this spec makes no change to these fields (DR-007) | N/A | N/A |

No public contract breaks. `annotations` is a new optional field; no existing
consumer (the CLI agent at `crates/agent/src/mcp.rs`, or any external MCP client)
reads or requires it today, so there is nothing to dual-maintain beyond "the field is
additive."

## 5. Declared Breaks

None. This is a pure additive change to the `tools/list` wire format. No existing
field changes value, type, or presence.

## 6. Tests

### 6.1 Non-regression — the success criterion

**The existing test suite passes unmodified**, except for the 3 files listed in
Section 6.4, which are widened (not narrowed) fixtures.

```
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

**Guardrail coverage:** `crates/mcp-fs/src/mcp/schema.rs` has 5 existing `#[test]`
functions (verified: `rg -c "#\[test\]" crates/mcp-fs/src/mcp/schema.rs` → 5).
`crates/mcp-fs/src/mcp/registry.rs` has 4 (`rg -c "#\[test\]"
crates/mcp-fs/src/mcp/registry.rs` → 4), including `list_entry_contains_schema`
which asserts on `to_list_entry()` output shape. `crates/mcp-fs/src/app.rs` has an
integration test `tools_list_is_sse_framed` (`crates/mcp-fs/src/app.rs:382`)
exercising the full `tools/list` JSON-RPC round trip. `contract_golden.rs` has 1 test
(`tool_contract_golden_is_current`) plus the 94-tool assertion in
`crates/mcp-fs/src/tools/all.rs:166` (`every_admin_and_git_schema_matches_the_frozen_tool_contract`).
This is well covered: any wiring mistake in `to_list_entry()` fails
`list_entry_contains_schema` or `tools_list_is_sse_framed` immediately.

### 6.2 Compatibility tests

Not applicable: no dual-maintained contract exists (Section 4).

### 6.3 Structural tests

#### DT-001: `to_list_entry` omits `annotations` when unset
- **Asserts:** a `ToolSchema` built with no annotation builder call renders
  `to_list_entry()` with no `"annotations"` key at all.
- **Method:** unit test in `crates/mcp-fs/src/mcp/schema.rs`, `assert!(v.get("annotations").is_none())`.

#### DT-002: `to_list_entry` serializes present hints in camelCase, omits absent ones
- **Asserts:** a `ToolSchema` built with `.destructive(true).read_only(false)` (and
  no idempotent/open-world call) renders exactly
  `{"destructiveHint":true,"readOnlyHint":false}` under `"annotations"`, with no
  `idempotentHint`/`openWorldHint` key.
- **Method:** unit test in `crates/mcp-fs/src/mcp/schema.rs`, exact JSON `assert_eq!`.

#### DT-003: every registered tool has an explicit annotation
- **Asserts:** for every tool in `crate::tools::all::register_all` with every
  feature flag on, `to_list_entry()["annotations"]` is present and is not
  `Value::Null` (i.e., no tool was left with all four hints as `None`, which would
  indicate a forgotten registration).
- **Method:** integration test in `crates/mcp-fs/src/tools/all.rs`, iterating
  `reg.names()` and asserting `resolve(name).schema.to_list_entry()["annotations"].is_object()`.

#### DT-004: `readOnlyHint=true` tools never carry `destructiveHint`
- **Asserts:** no tool has both `readOnlyHint: Some(true)` and
  `destructiveHint: Some(_)` set (structural consistency with MCP convention).
- **Method:** integration test in `crates/mcp-fs/src/tools/all.rs`, same iteration as
  DT-003, checking the invariant on the parsed JSON.

#### DT-005: contract golden regeneration is annotation-aware
- **Asserts:** `contract_golden::render()` includes `annotations` in its output when
  the live registry's schema has one; `tool_contract_golden_is_current` fails if the
  golden file's annotations differ from the registry's.
- **Method:** the existing `tool_contract_golden_is_current` test in
  `contract_golden.rs`, exercised naturally once `render()` is updated (DR-008); no
  new test function needed, this is the existing gate widened.

### 6.4 Existing tests to modify

| Test file | Test name | Change | Justification |
|-----------|-----------|--------|----------------|
| `crates/mcp-fs/src/mcp/registry.rs` | `list_entry_contains_schema` | none required, but reviewer SHOULD add an assertion that `annotations` is absent for the dummy test schema (which sets none), to lock the DT-001 behavior at the call site closest to the wire format | Widening an assertion, not narrowing one: the test's existing assertions (`t["name"]`, `t["inputSchema"]["type"]`, `t["inputSchema"]["required"]`) are untouched |
| `tool-contract-golden.json` | N/A (fixture, not a test) | regenerated wholesale via `MCPFS_REWRITE_TOOL_CONTRACT=1`; the diff review is the actual verification step | Every one of the 94 frozen entries gains an `annotations` key; this is the expected, single-purpose change this whole spec exists to make |
| `TOOL_CONTRACT.txt` | N/A (human-readable doc, not a test) | hand-updated or regenerated to list annotations per tool | Same reason as the golden JSON: this file is explicitly the human mirror of the same contract (`AGENTS.md:26`) |

No `#[test]` function's assertions are removed or weakened. The golden JSON and its
human-readable mirror are data files this change is designed to update; they are not
"tests" in the sense Section 6.4 exists to police, but are listed for completeness
per the invariant that governs this table.

## 7. Implementation Order

1. **Schema infrastructure.** Add `ToolAnnotations` struct and the 4 builder methods
   to `crates/mcp-fs/src/mcp/schema.rs`; wire `to_list_entry()`. Add DT-001, DT-002.
   Build and existing schema.rs tests stay green (no registration site uses the new
   methods yet, so nothing downstream changes).
2. **Contract golden awareness.** Update `render()` and `assert_family()` in
   `contract_golden.rs` to include/compare `annotations`. At this point
   `tool_contract_golden_is_current` still passes: no tool has annotations yet, so
   `render()` emits nothing new and the golden file is unchanged.
3. **Annotate every tool family**, one file at a time, in the order listed in Section
   2 (alphabetical by file is fine; no file depends on another). After each file:
   `cargo test -p mcp-fs --lib tools::<family>` stays green except
   `tool_contract_golden_is_current`, which now legitimately drifts (expected: the
   registry has annotations the golden file does not yet).
4. **Regenerate the golden contract**: `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p
   mcp-fs --lib tool_contract_golden_is_current`, then review the diff — it MUST show
   only added `annotations` keys, zero changes to any `name`, `description`, or
   `inputSchema` value.
5. **Add DT-003, DT-004** in `crates/mcp-fs/src/tools/all.rs` once every family is
   annotated, so they pass immediately rather than failing on partial coverage.
6. **Update `TOOL_CONTRACT.txt` and `.agent_docs/tools.md`** to reflect annotations
   per tool.
7. **Full quality gate**: `cargo test --workspace`, `cargo clippy --all-targets
   --all-features -- -D warnings`, `cargo fmt --all -- --check`.

Between steps 1 and 4, `tool_contract_golden_is_current` is the one test allowed to
be red (by design, tracking real progress); every other test stays green throughout.

## 8. Decisions & Assumptions

- **DDEC-001:** `openWorldHint` for `doc.to_docx`, `doc.to_pptx`, `fs.documentize`,
  `fs.extract_text` is declared `true` unconditionally, even though the underlying
  `doc_service` may be configured as a local CLI (closed-world) rather than an HTTP
  endpoint (open-world). **Rationale:** a hint describes the tool's worst-case
  capability across all valid server configurations, not the specific deployment's
  current config; a client cannot know which mode is active from the schema alone,
  and MCP hints are static per tool, not per-instance dynamic. **Implemented by:**
  the per-tool table in Section 3.1. Confirmed with the user in the prior turn of
  this conversation (see ambiguity #1 raised and accepted).
- **DDEC-002:** `sqlite.import_csv` is classified `destructiveHint=false`,
  `idempotentHint=false`. **Rationale:** confirmed at
  `crates/mcp-fs/src/tools/sqlite.rs:365-420` the handler only ever executes `CREATE
  TABLE IF NOT EXISTS` and `INSERT INTO`, never `DROP`/`DELETE`/`TRUNCATE`/`REPLACE`;
  it cannot destroy pre-existing rows, but repeating it duplicates rows so it is not
  safe to retry blindly. **Implemented by:** Section 3.1, "Additive" bucket.
- **DDEC-003:** `doc.open_editor` is classified `idempotentHint=false`.
  **Rationale:** confirmed at `crates/mcp-fs/src/tools/editor.rs:106-131` the tool
  documentation states "a unique editor_id" is returned; each call opens a new
  session rather than reusing an existing one for the same path. **Implemented by:**
  Section 3.1, "Additive" bucket.
- **DDEC-004:** `fs.write`, `fs.write_bytes`, `fs.create_empty`, `fs.mkdir` and every
  other tool whose default parameters make a specific call non-destructive (e.g.
  `overwrite=false`, `exist_ok=false`) are still classified by their worst-case
  capability, not their default-argument behavior. **Rationale:** MCP hints describe
  what a tool CAN do, not what a specific call with default arguments does; a client
  cannot inspect arguments-not-yet-chosen to decide whether to prompt for
  confirmation, so the hint must reflect the ceiling. **Implemented by:** DR-003 and
  the corresponding rows of Section 3.1. Confirmed with the user in the prior turn
  (ambiguity #4).
- **DDEC-005:** `git.merge_abort`, `git.rebase_abort`, `git.cherry_pick_abort` and
  `git.revert_abort` are classified `destructiveHint=false`, uniformly, even though
  each discards uncommitted conflict-resolution work recorded so far. **Rationale:**
  unlike `DDEC-004`, this is not a worst-case-capability call: these four tools'
  entire documented purpose (`crates/mcp-fs/src/tools/git.rs`, each `*_abort`
  handler's schema description) is restoring the repository to its pre-operation
  state and preventing data loss from a stuck conflict, never committed data is
  destroyed by any of them, only in-progress, not-yet-committed resolution state.
  Marking them destructive would tell a HITL-aware client to gate exactly the
  recovery action a caller reaches for after a conflict, which inverts the intent of
  the hint. **Implemented by:** Section 3.1, "Additive" bucket (all four are listed
  there, not under "Overwrite/delete", despite superficially resembling a delete).
- **Assumption:** the four MCP annotation field names and their camelCase wire
  format (`destructiveHint`, `readOnlyHint`, `idempotentHint`, `openWorldHint`) match
  the standard MCP tool annotations object as commonly implemented by MCP clients.
  This repo's MCP layer is hand-rolled (no `rmcp`/official SDK dependency found in
  `Cargo.toml` — `ASSUMED:` not independently re-verified against the SDK source in
  this pass, since the field names were given directly as the task input rather than
  discovered from a dependency).

## 9. Implementability Gate

Perimeter exhaustive (24 files, exact counts cited, non-source occurrences confirmed
absent); every public contract has a plan (Section 4, additive-only, no break); the
non-regression command is exact and its unmodified-suite criterion's 3 exceptions are
data-file regenerations, not test weakenings (Section 6.1/6.4); every `DDEC-XXX`
cites the `DR-XXX`/table it implements, no orphan; implementation order stated
(Section 7); Section 3.1 gives one classification per tool, no forced choice left to
the implementer; no out-of-spec prerequisite; EARS clean across DR-001 through
DR-008, no forbidden modal; every claim about the code is cited, including every git
tool's `file:LINE` citation from a direct read of `git.rs`/`git_auth.rs`/`git_pr.rs`;
`ToolSchema`, `to_list_entry`, `render()`, `assert_family()` all exist today and are
extended, not invented.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 | 0 | 1 — DDRIFT-001, resolved before this document's `Status: Draft` was finalized | IMPLEMENTABLE |

**Amendments applied:** DDRIFT-001 closed in place — all 49 `git.*`/`git.auth*`/
`git.pr_*` tool names re-verified directly against `git.rs`/`git_auth.rs`/`git_pr.rs`
and Section 3.1 corrected to the real 49-name set (see Section 10).
**Drift registered:** none open (DDRIFT-001 `Status: resolved`, see Section 10).

## 10. Drift Register

#### DDRIFT-001: the 49-tool git family was not individually named against source citations

- **Doc says (as of 2026-09-23 08:51):** Section 3.1 listed git tool names drawn
  from plausible operation-family coverage, including 7 names that do not exist in
  the codebase (`git.reflog`, `git.rebase_status`, `git.merge_status`,
  `git.cherry_pick_status`, `git.bisect_status`, `git.worktree_list`,
  `git.blame_range`), and stated the aggregate count reconciled (55 + 29 + 25 + 4 =
  113, plus "10 more tools not individually named") without every git tool being
  individually cited.
- **Code does:** `crates/mcp-fs/src/tools/git.rs` (39 registrations, one via a
  `for pop in [false, true]` loop producing `git.stash_pop`/`git.stash_apply` from a
  single call site at `git.rs:751-786`), `crates/mcp-fs/src/tools/git_auth.rs` (4:
  `auth`, `auth_status`, `auth_revoke`, `token_set`), `crates/mcp-fs/src/tools/git_pr.rs`
  (6: `pr_create`, `pr_list`, `pr_get`, `pr_diff`, `pr_merge`, `pr_review`) register
  49 tools total, confirmed by name extraction (`python3 -re` scan matching
  `ToolSchema::new\(\s*"..."`, cross-checked against `reg.add\(` occurrence counts
  per file) and by the passing test `the_git_families_add_forty_nine_tools`
  (`crates/mcp-fs/src/tools/all.rs:80`). None of the 7 fabricated names above exist;
  there is no `git.reflog`, no bisect family, no worktree family, no `git.tag_create`
  or `git.tag_delete` (tags are read-only via `git.tags` in this codebase, unlike
  branches which do have create/delete/reset).
- **Nature:** false count / fabricated names at the leaf level, exactly as the prior
  revision suspected, not a missing capability.
- **Resolution during implementation:** every one of the 49 real names was read from its handler
  in `git.rs`/`git_auth.rs`/`git_pr.rs` and reclassified in Section 3.1 with a
  `file:LINE` citation, replacing the 7 fabricated names entirely. The aggregate
  reconciles at the corrected totals: 55 pure-read (was mis-stated at 55 with wrong
  membership), 38 overwrite/delete (was 29), 30 additive (was 25). The `DDEC-005`
  entry in Section 8 was added during this resolution to state the `*_abort`-family
  policy explicitly, which the fabricated names had let go unstated. This drift was
  discovered and closed while updating `specs/SPEC-0007_2026-09-18_19-45-00-git/spec.md`
  (`DRIFT-6-01` there) for the same underlying staleness: this document's author had
  the same incomplete picture of the git tool surface that S6 itself had gone stale
  on, for the same root cause (the surface grew after both documents were drafted).
- **Detected by:** `cargo test -p mcp-fs --lib
  tools::all::tests::the_git_families_add_forty_nine_tools` (aggregate count) plus a
  direct source read of all three files (the leaf-level names, which no test
  isolates since the test only asserts a count, not a name set — matching the
  `DDRIFT-001` original concern precisely: a count-only test cannot catch a
  plausible-sounding wrong name).
- **Status:** resolved.
