# SPEC-0052: Git tags, design

> Spec: sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md
> Nature: FEAT
> Depth: L
> Security: n/a
> Designed on: 2026-10-24
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust, edition 2024, cargo workspace `crates/core` (lib) + `crates/mcp-fs` (bin) + `crates/agent` | per `Cargo.toml` | DEC-001 |
| Git engine | `git2` (libgit2) | 0.21 (`Cargo.toml:63`), API names checked against context7 `/websites/rs_git2`: `Repository::tag`, `tag_lightweight`, `tag_delete`, `Tag::message`, `target_id` | DEC-001, DEC-005 |
| MCP | `rmcp` | 3.5.0 (`Cargo.toml:57`) | DEC-001, DEC-002 |
| Store | existing relational `git_refs` table and blob store | unchanged | DEC-004 |

Skills loaded: `rust` (via `load_dynamic_context`). Effect on this design: logic stays in `crates/core`, `thiserror`-style `ToolError` constructors only, no `unwrap` on fallible paths, `tracing` spans on the remote boundary, `make check` is the gate. The project's own rule overrides the skill where they differ: the `cargo test`/`cargo clippy --all-features` commands of `AGENTS.md` are the quality gate, and `rustfmt.toml` stays authoritative. No new dependency, so no context7 version lookup was needed beyond API verification.

## 2. Approaches
### Chosen: three explicit tools, tags as plain refs in the existing store
Summary: `git.tag_create`, `git.tag_delete`, `git.remote_push_tag`; a tag is a `git_refs` row like a branch; receiving tags is an explicit, bounded step after the existing fetch. Pros: no schema change, one verb per tool like the 111 existing tools, the branch push code stays byte identical, each requirement maps to one place. Cons: three contract additions (golden regeneration, count bumps), two extra connections on publish (list, then push). Affected scope: `tools/git_tags.rs` (new), `tools/git.rs`, `git/remote.rs`, `mcp/server.rs`, contract files. Best suited when: the contract is frozen and reviewed, as here.
### Rejected: extend existing tools only
Add `tag` parameters to `git.remote_push` and a `message` to `git.branch_create`. Rejected: `git.remote_push` requires `branch` (an either/or contract that breaks schema clarity), and tag creation has no natural home among branch tools (DEC-002).
### Rejected: libgit2 automatic tag following
Turn on `AutotagOption::Auto` and import whatever libgit2 writes. Rejected: it cannot see the project's local tags (refs live in the database, only objects are hydrated), follows tags to objects from other branches, and behaves differently on the `file://` remotes every test uses (DEC-011, DEC-012).

## 3. Architecture
```
MCP tools (mcp/server.rs)          thin #[tool] methods, frozen descriptions
   git.tag_create  git.tag_delete  git.remote_push_tag      git.tags (changed shape)
        |               |                  |                       |
        v               v                  v                       v
tools/git_tags.rs  <- engines: create, delete, publish, list, resolve_target,
        |                reconcile_incoming_tags, tags_identical, short7
        |  uses pub(crate) helpers of tools/git.rs: open, hydrate, on_git_thread,
        |  reject_if_operation_in_progress, validate_branch_name, parse_oid
        v
git/remote.rs   push core split (refs/heads | refs/tags), list_advertised_refs,
                trace_remote_operation, FetchOutcome.incoming_tags
        v
entry.db (git_refs)  +  entry.objects (blob store)  +  on-disk libgit2 cache (rebuildable)
```
Fetch, pull and clone call `reconcile_incoming_tags` after `import_from_repo`. Bounded contexts of the spec: Versioning = `git_tags.rs` local engines; Remote exchange = `git/remote.rs` plus the wiring in `tools/git.rs`; Audit = the existing `safety.record_audit`.
Current structure evidence: tags are listed only by `tool_tags` (`tools/git.rs:1312`) through `refs_under` (`:2009`); no code creates a tag ref outside tests (`git/db.rs:768`, `tools/git.rs:10594`).

## 4. Data model
No new entity, no migration, no rollback needed. `git_refs(volume_id, name, target, symbolic)` gains rows named `refs/tags/<name>` (already a legal shape: `list_refs` filters on that prefix, `git/db.rs:397`). Plain tag: target = commit sha. Annotated tag: target = tag object sha; the tag object is a normal git object stored as `git:{sha}` (type `tag` is already supported, `git/odb.rs:52,63`). Stash refs live elsewhere and never match the prefix (existing test `tools/git.rs:17310`).

## 5. Contracts and interfaces
All parameter descriptions below are the frozen LLM-facing text and must be copied verbatim into `TOOL_CONTRACT.txt` (a deliberate contract change, DEC-021).

**`git.tag_create`** (`mount_id`, `name`, `target`="", `message`="", `overwrite`=false) returns `{mount_id, name, kind, sha, object, overwritten, previous_sha?}`.
**`git.tag_delete`** (`mount_id`, `name`) returns `{mount_id, name, sha}`.
**`git.remote_push_tag`** (`mount_id`, `tag`, `remote`="origin", `overwrite`=false, `expected_remote_commit`="") returns `{mount_id, tag, remote, created, up_to_date, remote_sha, auth}`.
**`git.tags`** entry: `{name, full_ref, sha, object, kind, message?, tagger?, tagger_email?, tagged_at?}`, ascending by `name`.
**`git.remote_fetch`, `git.remote_pull`, `git.remote_clone`** results gain `tags_added` and `tags_kept_local` (sorted arrays, always present).
Exact refusal texts and codes (all verbatim from the spec):
| Situation | Code | Text |
|---|---|---|
| name taken, no confirmation | `ERR_NO_CLOBBER` | `Tag '<n>' already exists. Choose another name or confirm overwriting it.` |
| invalid name | `ERR_INVALID_ARGUMENT` | `'<n>' is not a valid tag name: <reason>` |
| unknown target | `ERR_NOT_FOUND` | `Cannot tag '<t>': no such branch, tag or commit.` |
| project has no commit | `ERR_NOT_FOUND` | `Cannot tag: this project has no commit yet.` |
| unknown tag (delete, publish) | `ERR_NOT_FOUND` | `Tag '<n>' does not exist.` |
| paused operation | `ERR_INVALID_ARGUMENT` | the existing text of `reject_if_operation_in_progress` (`tools/git.rs:4195-4204`) with the tool name |
| no remote | `ERR_NOT_FOUND` | `Cannot publish tag '<n>': this project tracks no remote.` |
| commit not on the remote | `ERR_INVALID_ARGUMENT` | `Cannot publish tag '<n>': commit <7> is not on the remote. Publish its branch first.` |
| remote tag differs, no or incomplete confirmation | `ERR_NO_CLOBBER` | `Remote tag '<n>' already exists at a different commit (<7>). Confirm overwriting to replace it.` |
| stated commit differs | `ERR_NO_CLOBBER` | `Remote tag '<n>' has moved to <7> since you last saw it. Fetch, then confirm again.` |
| remote rejects the name | `ERR_INTERNAL_ERROR` | `Remote refused tag '<n>': <reason>` |
Audit lines (action, then detail; path `/`): `git.tag_create` `created tag '<n>' at <40>`; `git.tag_overwrite` `overwrote tag '<n>' at <new 40> (was <old 40>)`; `git.tag_delete` `deleted tag '<n>' (was at <40>)`; `git.tag_publish` `published tag '<n>' at <40>`.
Config keys: none added.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git_tags.rs` | create | FR-NEW-001..021 | US-0083 and later |
| `crates/core/src/tools/mod.rs` | modify (`pub mod git_tags;`) | structure | US-0083 |
| `crates/core/src/tools/git.rs` | modify (visibility, wiring, tests, 2 flipped tests) | FR-MOD-001, FR-MOD-002, FR-NEW-009..011 | US-0083, 0086, 0087, 0088, 0091 |
| `crates/core/src/git/remote.rs` | modify (push core split, advertised list, incoming tags, span helper) | FR-NEW-007..010, FR-NEW-018, FR-NEW-019 | US-0086, 0089, 0090 |
| `crates/core/src/mcp/server.rs` | modify (3 tools, args, counts) | FR-NEW-001, 005, 007 | US-0083, 0084, 0085, 0089, 0090 |
| `tool-contract-golden.json` | regenerate | contract | US-0083, 0084, 0085, 0089, 0090 |
| `TOOL_CONTRACT.txt` | modify | contract | same |
| `crates/core/src/tools/contract_golden.rs` | modify (counts) | contract | US-0083, 0085, 0089 |
| `crates/agent/src/mcp.rs` | modify (count at `:319`) | contract | US-0083, 0085, 0089 |
| `AGENTS.md, .agent_docs/{architecture,testing,tools,git,git-divergences}.md` | modify | docs | US-0091 |

Affected existing tests: modify `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags` (`tools/git.rs:10808-10830`) and `e2e_new_217_pull_will_inherit_the_fetch_refspec` (`:10843-10870`), both in US-0086 (DEC-022). Keep: the other 527 tests of `tools/git.rs` (`rg -c '#\[(tokio::)?test' crates/core/src/tools/git.rs` = 529 before this work), including every branch push test, which guard DEC-015.
Affected documentation: listed in US-0091.

## 7. Compatibility and risks
- Consumers: `git.tags` keeps `name`, `full_ref`, `sha` (now always the commit; before, annotated tags could not exist), gains keys. Fetch, pull, clone results gain keys. No key removed. No route, no config key, no environment variable.
- Behaviour change that is declared by the spec: fetch now brings tags (FR-MOD-002), reversing SPEC-0011/FR-NEW-062.
- Ordering: US-0086 must flip the two old tests in the same story that changes fetch, or main goes red. Tool counts are bumped by the story that adds the tool; each story reads the live number.
- Risks: (1) `message` byte exactness depends on writing the tag object through the odb (DEC-005), verified by E2E-005, E2E-078. (2) Names accepted by the branch rules but refused by libgit2 (`//`, trailing `.`, `@{`) cannot be synced to the on-disk repo or pushed: DEC-019, recorded as a concern. (3) A shallow probe fetch compares remote annotated messages (DEC-017); on `file://` remotes shallow fetch must be verified in US-0090 first, fallback: full single tag fetch. (4) 2000 explicit refspecs in one fetch (E2E-048): measure; fallback: batches of 500. (5) `short` is 8 characters elsewhere, 7 here (DEC-018), a visible but intended difference.
- Rollback: revert the commits; no data migration; tags written stay as inert rows.
- Dependencies added or removed: none.

## 9. Test strategy
Levels: every acceptance test is a functional e2e test through the real tool functions of the crate; the remote is a real bare repository on disk (`file://`), reached through the direct helpers because the registered remote tools reject `file://` by design (`tools/git.rs:3295-3296` doc comment). Unit tests of `tags_identical`, `reconcile_incoming_tags`, target resolution and `short7` are written by the implementer alongside (not part of the acceptance set). Coverage gate: the project's `make check`.
Test data: generated in the tests (bare repos built by the existing helpers); no user provided data.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_001_*` | US-0083 |
| E2E-002 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_002_*` | US-0083 |
| E2E-003 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_003_*` | US-0083 |
| E2E-004 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_004_*` | US-0084 |
| E2E-005 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_005_*` | US-0084 |
| E2E-006 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_006_*` | US-0083 |
| E2E-007 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_007_*` | US-0083 |
| E2E-008 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_008_*` | US-0084 |
| E2E-009 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_009_*` | US-0083 |
| E2E-010 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_010_*` | US-0083 |
| E2E-011 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_011_*` | US-0083 |
| E2E-012 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_012_*` | US-0083 |
| E2E-013 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_013_*` | US-0083 |
| E2E-014 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_014_*` | US-0083 |
| E2E-015 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_015_*` | US-0083 |
| E2E-016 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_016_*` | US-0083 |
| E2E-017 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_017_*` | US-0083 |
| E2E-018 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_018_*` | US-0083 |
| E2E-019 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_019_*` | US-0083 |
| E2E-020 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_020_*` | US-0084 |
| E2E-021 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_021_*` | US-0083 |
| E2E-022 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_022_*` | US-0084 |
| E2E-023 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_023_*` | US-0083 |
| E2E-024 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_024_*` | US-0083 |
| E2E-025 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_025_*` | US-0083 |
| E2E-026 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_026_*` | US-0085 |
| E2E-027 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_027_*` | US-0085 |
| E2E-028 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_028_*` | US-0085 |
| E2E-029 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_029_*` | US-0085 |
| E2E-030 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_030_*` | US-0085 |
| E2E-031 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_031_*` | US-0085 |
| E2E-032 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_032_*` | US-0085 |
| E2E-033 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_033_*` | US-0089 |
| E2E-034 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_034_*` | US-0089 |
| E2E-035 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_035_*` | US-0089 |
| E2E-036 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_036_*` | US-0090 |
| E2E-037 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_037_*` | US-0090 |
| E2E-038 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_038_*` | US-0089 |
| E2E-039 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_039_*` | US-0089 |
| E2E-040 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_040_*` | US-0089 |
| E2E-041 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_041_*` | US-0086 |
| E2E-042 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_042_*` | US-0086 |
| E2E-043 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_043_*` | US-0086 |
| E2E-044 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_044_*` | US-0086 |
| E2E-045 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_045_*` | US-0086 |
| E2E-046 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_046_*` | US-0088 |
| E2E-047 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_047_*` | US-0088 |
| E2E-048 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_048_*` | US-0086 |
| E2E-049 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_049_*` | US-0086 |
| E2E-050 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_050_*` | US-0086 |
| E2E-051 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_051_*` | US-0091 |
| E2E-052 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_052_*` | US-0083 |
| E2E-053 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_053_*` | US-0085 |
| E2E-054 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_054_*` | US-0087 |
| E2E-055 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_055_*` | US-0089 |
| E2E-056 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_056_*` | US-0090 |
| E2E-057 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_057_*` | US-0090 |
| E2E-058 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_058_*` | US-0090 |
| E2E-059 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_059_*` | US-0088 |
| E2E-060 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_060_*` | US-0088 |
| E2E-061 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_061_*` | US-0083 |
| E2E-062 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_062_*` | US-0086 |
| E2E-063 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_063_*` | US-0087 |
| E2E-064 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_064_*` | US-0087 |
| E2E-065 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_065_*` | US-0085 |
| E2E-066 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_066_*` | US-0085 |
| E2E-067 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_067_*` | US-0084 |
| E2E-068 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_068_*` | US-0083 |
| E2E-069 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_069_*` | US-0090 |
| E2E-070 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_070_*` | US-0083 |
| E2E-071 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_071_*` | US-0089 |
| E2E-072 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_072_*` | US-0089 |
| E2E-073 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_073_*` | US-0089 |
| E2E-074 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_074_*` | US-0089 |
| E2E-075 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_075_*` | US-0089 |
| E2E-076 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_076_*` | US-0086 |
| E2E-077 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_077_*` | US-0089 |
| E2E-078 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_078_*` | US-0084 |
| E2E-079 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_079_*` | US-0083 |
| E2E-080 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_080_*` | US-0083 |
| E2E-081 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_081_*` | US-0083 |
| E2E-082 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_082_*` | US-0083 |
| E2E-083 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_083_*` | US-0090 |
| E2E-084 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_084_*` | US-0089 |
| E2E-085 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_085_*` | US-0089 |
| E2E-MOD-001 | functional e2e | tool registry `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_mod_001_*` | US-0084 |
| E2E-MOD-002 | functional e2e | direct remote helper, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_mod_002_*` | US-0086 |

## 10. Decisions log
- **DEC-001:** No new dependency, no version change: the stack is the manifests' (`git2 0.21` at `Cargo.toml:63`, `rmcp 3.5.0` at `Cargo.toml:57`, Rust 2024). Obliged by: n/a (no code impact), it records the stack. Alternatives: none needed.
- **DEC-002:** Three new MCP tools, `git.tag_create`, `git.tag_delete` and `git.remote_push_tag`; `git.tags` and the fetch, pull and clone results gain additive keys. Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: extra parameters on `git.remote_push` (it requires `branch`, so a tag would make it an either/or contract), one `git.tag` tool with an `action` argument (breaks the one tool one verb convention of the 111 tools). Total 114 tools.
- **DEC-003:** Tag engines live in a new module `crates/core/src/tools/git_tags.rs` (precedent: `git_pr.rs`, `git_auth.rs`, registered in `tools/mod.rs:55-56`); the helpers it needs from `tools/git.rs` (`open`, `hydrate`, `on_git_thread`, `parse_oid`, `reject_if_operation_in_progress`, `validate_branch_name`, `short`) become `pub(crate)`. New tests go into the existing `#[cfg(test)] mod tests` of `tools/git.rs` because its bare remote helpers are private there. Obliged by: all requirements (structure). Alternatives: code in `git.rs` (20k lines already), tests in `git_tags.rs` (would need every helper exported).
- **DEC-004:** A tag is one `git_refs` row `refs/tags/<name>` whose target is the commit sha (plain tag) or the tag object sha (annotated tag); objects reach the blob store through the existing `entry.objects.import_from_repo`. No schema change, no migration. Obliged by: FR-NEW-001, FR-NEW-002. Evidence: `git/db.rs:371` `set_ref`, `git/odb.rs:188`. Alternatives: a dedicated tags table (second source of truth, rejected).
- **DEC-005:** An annotated tag object is written with `repo.odb()?.write(git2::ObjectType::Tag, bytes)` where bytes are `object <commit>\ntype commit\ntag <name>\ntagger <name> <<email>> <unix> +0000\n\n<message>` and the message is the caller's string unchanged (no `message_prettify`, which `commit` applies at `tools/git.rs:2160`). Obliged by: FR-NEW-002 (message kept byte for byte), FR-NEW-004 (names the branch rules accept). Alternatives: `Repository::tag` (its own stricter name validation and message handling, verified in the context7 docs of `/websites/rs_git2`).
- **DEC-006:** The tagger is built exactly as a commit author is (`tools/git.rs:2137-2142`): name = the person id before `@`, email = the person id. A listing entry for an annotated tag carries `kind: "annotated"`, `message`, `tagger` (name), `tagger_email`, `tagged_at` (UTC `%Y-%m-%dT%H:%M:%SZ`); a plain tag carries `kind: "lightweight"` and none of the three. `sha` is always the peeled commit, `object` the ref's own target. Obliged by: FR-MOD-001. Alternatives: renaming `sha` (breaks readers, rejected), `commit_json`'s date format (not the format FR-MOD-001 spells).
- **DEC-007:** Target resolution (FR-NEW-021) is a new function in `git_tags.rs`: `refs/heads/<t>` then `refs/tags/<t>` (peeled to its commit) then a 40 hex id that `find_commit` accepts, then a hex prefix of 7 or more characters through `repo.find_object_by_prefix`, where an ambiguous result and any shorter text mean unknown target. The existing `resolve_ref` (`tools/git.rs:1879`) is not reused: it returns an unverified hex string as is. Obliged by: FR-NEW-021, FR-NEW-004. Alternatives: reuse `resolve_ref` (no existence check, no ambiguity handling).
- **DEC-008:** Refusals are produced in the order of FR-NEW-020 by the order of the calls: `open` (membership), `reject_if_operation_in_progress`, name validation, no commit, target resolution, existing name (checked again under the write lock). Codes: invalid name `invalid_argument`; unknown target, no commit and unknown tag `not_found`; existing name and remote tag refusals `no_clobber`; commit not on the remote `invalid_argument`. Messages are the spec's, verbatim. Obliged by: FR-NEW-020, FR-NEW-003, FR-NEW-004, FR-NEW-005, FR-NEW-013. Alternatives: new error codes (the 16 codes are a frozen set, `errors.rs:9-24`).
- **DEC-009:** Create, overwrite and delete run under `entry.write_lock` (the lock `branch_create` and `commit` hold, `tools/git.rs:2121-2135`), so concurrent same-name calls serialise and exactly one wins. Audit goes through `safety.record_audit(person, project, action, "/", detail)` with actions `git.tag_create`, `git.tag_overwrite`, `git.tag_delete`, `git.tag_publish` and the spec's detail lines; full 40 character ids. Obliged by: FR-NEW-012, FR-NEW-015, FR-NEW-016. Alternatives: optimistic retry (rejected, the lock already exists).
- **DEC-010:** Listing reads the `git_refs` tag rows, then inside `on_git_thread` after `hydrate` peels each target with the repo (`find_object`, `peel_to_commit`, `find_tag` for annotated) and sorts by name bytes ascending (`Vec<String>` sort is byte order). Stash refs never match `refs/tags/`. Obliged by: FR-MOD-001. Alternatives: parse tag objects by hand from the blob store (brittle).
- **DEC-011:** Receiving tags does NOT turn on libgit2 auto tag following. `fetch_from_remote` (`git/remote.rs:703`) already lists the advertised refs; after the branch fetch it computes candidates from that list: a tag whose peeled commit (the `^{}` entry, or the ref's own oid for a lightweight tag) is the fetched branch tip or one of its ancestors (`graph_descendant_of`), and is a commit. Lightweight candidates need no download; annotated ones are fetched with explicit refspecs `+refs/tags/<n>:refs/mcpfs/incoming/<n>` in one extra fetch. The result carries the candidate list. Obliged by: FR-NEW-009, FR-MOD-002. Alternatives: `AutotagOption::Auto` or `All` (follows tags to objects present from other branches, downloads tag history we then discard, not transport independent on the `file://` test remotes), one wildcard refspec (downloads every tag).
- **DEC-012:** A reconcile step in `git_tags.rs`, `reconcile_incoming_tags(entry, repo, incoming) -> (added, kept_local)`, runs right after `import_from_repo` in fetch, pull and clone: for each incoming tag, absent locally means `set_ref` and added; identical (DEC-017) means skip; different means keep the local tag and list it as kept local. Results gain `tags_added` and `tags_kept_local` (arrays of names, sorted). The incoming temporary refs are deleted from the on-disk repo. Obliged by: FR-NEW-009, FR-NEW-010, FR-NEW-011. Alternatives: letting libgit2 write refs (it cannot see the db's local tags, since only objects are hydrated, `tools/git.rs:1898`).
- **DEC-013:** In pull, reconcile runs immediately after the fetch import and before the merge decision, so a pull that pauses with `status: "conflict"` has already delivered its tags; a failed exchange returns before reconcile. Obliged by: FR-NEW-017. Alternatives: reconcile after the merge (loses the tags on a pause).
- **DEC-014:** Clone takes its tags from the temporary clone itself: every `refs/tags/*` of the cloned repo whose peeled target is a commit equal to or an ancestor of the cloned branch tip, passed to the same reconcile step (a re-clone into a populated project is allowed today, `tools/git.rs:3174-3176`, so a conflicting local tag is kept). Obliged by: FR-NEW-011, FR-NEW-010. Alternatives: a second network fetch (clone already holds the data).
- **DEC-015:** Publishing is the tool `git.remote_push_tag(mount_id, tag, remote="origin", overwrite=false, expected_remote_commit="")`. Flow: `authorize`, local tag exists, `require_remote`, credential resolution (`resolve_clone_credential`), one connection to list the advertised refs, the checks of FR-NEW-019 and FR-NEW-008, then one push of `refs/tags/<n>:refs/tags/<n>` (a `+` prefix only when overwrite is confirmed and the stated commit matched). The push core of `push_to_remote` (`git/remote.rs:516`, hardcoded `refs/heads` refspec at `:601`) is split into a ref name taking private function that both the branch and the tag entry points call; branch behaviour stays byte identical. Obliged by: FR-NEW-007, FR-NEW-008, FR-NEW-018, FR-NEW-019. Alternatives: libgit2 `Remote::push` with both connections merged (the branch path keeps its own negotiation view).
- **DEC-016:** Tag publication cannot use `run_remote_operation` (`git/remote.rs:338`): it audits every outcome, failures included, which FR-NEW-012 forbids. A sibling `trace_remote_operation` keeps the same `git.remote` span and records nothing; `git_tags.rs` calls `record_audit` itself, only after a real publication. This deliberately departs from the one audit per remote call rule of SPEC-0011 FR-NEW-057/072 for this one tool. Obliged by: FR-NEW-012. Alternatives: auditing failures too (contradicts the spec).
- **DEC-017:** One function `tags_identical(repo, a, b)`: both plain with the same commit; both annotated with the same peeled commit and the same message (tagger and time ignored); mixed kinds differ. For a remote annotated tag whose object id differs from the local one, the message is read by a one tag shallow fetch (`depth 1`, refspec `+refs/tags/<n>:refs/mcpfs/probe/<n>`), then the probe ref is deleted. Obliged by: FR-NEW-007, FR-NEW-009, FR-NEW-010. Alternatives: comparing tag object ids (would compare tagger and time, which the spec excludes), asking the user (the remote exposes no message without a transfer).
- **DEC-018:** Overwrite semantics: without `overwrite`, a differing remote tag is refused with the FR-NEW-008 text; with `overwrite` and no `expected_remote_commit` the same text; with a stated commit (full id or unique prefix of at least 7 characters, compared with the remote tag's peeled commit) that differs, the `has moved` text; with a match, the push is forced with a lease on the advertised oid. Confirmed overwrite on an absent remote tag is a plain creation, on an identical one `up_to_date`. Commits are shown as 7 characters (`short7`), distinct from the 8 character `short` (`tools/git.rs:1866`) that stays elsewhere. Obliged by: FR-NEW-008, FR-NEW-018. Alternatives: reuse `expected_remote_sha` (it leases a branch sha; a tag has both an object and a commit).
- **DEC-019:** Name refusals on publish: libgit2 validates ref names more strictly than the branch rules (`//`, trailing `.`, `@{` pass `validate_branch_name`, `tools/git.rs:2200-2236`). A tag with such a name is stored (db row, odb written tag object) and listed, but syncing its ref into the on-disk repo or the remote's per ref rejection both produce `Remote refused tag '<n>': <reason>` (code `internal`), no audit entry, nothing changed. E2E-085 is realised through the push status callback with a rejection message (the local file transport runs no hooks). This is a recorded concern, not a spec change. Obliged by: FR-NEW-007. Alternatives: tightening the name rules (contradicts FR-NEW-004), refusing at create (contradicts SC-001).
- **DEC-020:** "On the remote" (FR-NEW-019): the tag's commit equals an advertised branch tip, or is an ancestor of one whose objects the project already holds (`graph_descendant_of`); tips absent from the local odb are skipped; the publish never fetches. Obliged by: FR-NEW-019. Alternatives: asking the remote with a want/have probe (a fetch in disguise).
- **DEC-021:** Contract changes are deliberate and reviewed: three new `#[tool]` methods in `mcp/server.rs`, three matching `#[cfg(test)]` registry entries, the `message`, `overwrite`, `expected_remote_commit` parameters with their frozen descriptions copied from `TOOL_CONTRACT.txt`, the `git.tags` description, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`, and the 111 constants raised stepwise to 112, 113, 114 (`tools/contract_golden.rs:146,:253,:309,:323`, `mcp/server.rs:4294,4315,4316`, `crates/agent/src/mcp.rs:319`). Obliged by: FR-NEW-001, FR-NEW-005, FR-NEW-007. Alternatives: none (the golden test fails otherwise).
- **DEC-022:** Two existing tests assert the opposite of FR-MOD-002 and are modified, not deleted: `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags` (`tools/git.rs:10808-10830`) and `e2e_new_217_pull_will_inherit_the_fetch_refspec` (`:10843-10870`). Comments citing FR-NEW-062 at `git/remote.rs:745` and `tools/git.rs:10593` are updated. Obliged by: FR-MOD-002. Alternatives: leave them failing (rejected).

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0083 | create a plain tag and list tags with kind and order | P1 | FR-MOD-001, FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-006, FR-NEW-012, FR-NEW-013, FR-NEW-014, FR-NEW-015, FR-NEW-020, FR-NEW-021 | main | 1 | no |
| US-0084 | annotated tags with message, tagger and creation time | P1 | FR-MOD-001, FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-013 | US-0083 | 2 | no |
| US-0085 | delete a local tag | P1 | FR-NEW-005, FR-NEW-006, FR-NEW-012, FR-NEW-014, FR-NEW-016, FR-NEW-020 | US-0083 | 1 | no |
| US-0086 | receive remote tags on fetch | P1 | FR-MOD-002, FR-NEW-009, FR-NEW-010, FR-NEW-014 | US-0084 | 1 | no |
| US-0087 | receive remote tags on pull, also when the merge pauses | P2 | FR-NEW-009, FR-NEW-010, FR-NEW-017 | US-0086 | 2 | no |
| US-0088 | receive remote tags on clone | P2 | FR-NEW-010, FR-NEW-011 | US-0086 | 2 | no |
| US-0089 | publish one tag to the remote | P1 | FR-NEW-007, FR-NEW-012, FR-NEW-014, FR-NEW-018, FR-NEW-019, FR-NEW-020 | US-0083 | 1 | no |
| US-0090 | overwrite a remote tag only on explicit confirmation | P2 | FR-NEW-007, FR-NEW-008, FR-NEW-014, FR-NEW-018 | US-0089, US-0084 | 1 | no |
| US-0091 | documentation, round trip test and spec close | P2 | FR-NEW-002, FR-NEW-007, FR-NEW-009, FR-NEW-012 | US-0090, US-0085, US-0087, US-0088 | 1 | yes |
