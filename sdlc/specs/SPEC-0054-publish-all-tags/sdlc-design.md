# SPEC-0054: Publish every local tag in one guarded action, design

> Spec: sdlc/specs/SPEC-0054-publish-all-tags/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-26
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024 workspace (`crates/core`, `crates/mcp-fs`, `crates/agent`) | per `Cargo.toml` | DEC-001 |
| Git engine | git2 (libgit2) | 0.21 (`Cargo.toml:63`) | DEC-001, DEC-004 |
| MCP | rmcp | 3.5.0 (`Cargo.toml:57`) | DEC-001, DEC-002 |

Skills loaded: `rust` (via `load_dynamic_context`): logic in `crates/core`, `ToolError` constructors, no `unwrap` on fallible paths, `make check` as the gate. No new dependency, so no context7 lookup was needed.

## 2. Approaches
### Chosen: one tool, one listing, one multi ref push
A new tool `git.remote_push_tags` lists the remote once, classifies every local tag in process with the rules the single publish already holds, and sends all due tags in one push whose refspecs each carry a "must be absent" lease. Pros: connections do not grow with the tag count (only annotated lookalikes cost a probe), a remote change between listing and push is detected before anything is sent, the single publish tool is untouched. Cons: the shared push core becomes multi ref (a careful refactor), per ref status must be read from the transport. Affected scope: `tools/git_tags.rs`, `git/remote.rs`, `mcp/server.rs`, the contract files.
### Rejected: a loop over the single publish engine
Call `publish_tag` once per tag (DEC-003). Simple, but each tag costs a listing plus a push connection, so 50 tags open 100 connections inside one deadline, and a partial result after a dropped connection is harder to report.
### Rejected: a mode of `git.remote_push_tag`
Making `tag` optional on the existing tool (DEC-002). It turns a one tag contract into an either/or one and mixes the overwrite rule with a bulk rule that forbids overwriting.

## 3. Architecture
```
git.remote_push_tags (mcp/server.rs)
   -> tool_remote_push_tags (git_tags.rs): open (member), remote lookup, credential
   -> publish_all_tags: trace_remote_operation span, write lock, on_git_thread
        -> list_advertised_refs (git/remote.rs): one listing
        -> classify_tags: local tags (sorted by byte), outcome per tag
        -> parse_confirm + count check (preview, wrong number, nothing to publish)
        -> push_tags_to_remote (git/remote.rs): push_refs with N creation refspecs, each leased "absent"
             lease lost on some refs: re list, reclassify, push the rest (at most one tag fewer per round)
        -> per tag outcome, one audit line per tag sent
```
Bounded contexts: Remote exchange = the two files above; Versioning = the local tag refs read through `entry.db.list_refs`; Audit = `safety.record_audit`. Current structure: single publish at `git_tags.rs:535-726`, push core at `git/remote.rs:609`, tag push at `:696`, listing at `:881`.

## 4. Data model
No entity, no migration, no database write: a publication writes nothing to the database beyond what the single publish already does (it writes the tag ref on disk before pushing, `git_tags.rs:690`).

## 5. Contracts and interfaces
**`git.remote_push_tags`**: parameters `mount_id`, `remote` (default `origin`, an empty text means `origin`), `confirm_count` (optional; absent or null means preview). `confirm_count` accepts a JSON string of 1 to 9 ASCII digits or a non negative JSON integer; anything else, an empty string included, is the number refusal.

Result (preview, send and nothing to publish share one shape):
`{mount_id, remote, to_publish: <N>, published: <K>, sent: <bool>, tags: [{name, outcome, commit, remote_commit, reason}], lines: [<one text per tag>], message: <last line>, auth}` where `outcome` is one of `to publish`, `published`, `up to date`, `differs`, `commit not on remote`, `refused by remote`; `commit` is the tag's full commit id; `remote_commit` (full id) is present for `differs`; `reason` for `refused by remote`. `lines` holds exactly the texts of the spec (`<name> <outcome>`, with ` (remote at <7>)` after `differs` and `: <reason>` after `refused by remote`), in byte order of the name; `message` is `Tags to publish: <N>. Confirm by stating <N> to send them.` for a preview with N at least 1, `No tag to publish.` when N is 0, and empty after a send.

| Situation | Code | Text |
|---|---|---|
| not a member | `ERR_FORBIDDEN` | existing `'<person>' is not a member of '<project>'` |
| no remote at all | `ERR_NOT_FOUND` | `Cannot publish tags: this project tracks no remote.` |
| named remote not tracked | `ERR_NOT_FOUND` | `Cannot publish tags: this project does not track a remote named '<r>'.` |
| credential, connection, timeout | as publishing | existing texts |
| not a count | `ERR_INVALID_ARGUMENT` | `The number to confirm must be a whole number, zero or more.` |
| wrong number | `ERR_NO_CLOBBER` | `Tags to publish: <M>, not <N>. Review the list, then confirm again.` then one line per tag, newline separated, in the message |

Audit: action `git.tag_publish`, path `/`, `published tag '<n>' at <full 40>` (the single publish text), one per tag sent. Config keys: none.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (engine, classification, parsing, report) | FR-NEW-001..020 | 1 to 4 |
| `crates/core/src/git/remote.rs` | modify (multi ref push core, per ref status, seams) | FR-NEW-002, 008, 012, 020 | 1, 3, 4 |
| `crates/core/src/mcp/server.rs` | modify (tool, args, counts) | FR-NEW-001 | 1 |
| `crates/core/src/tools/git.rs` | modify (registry entry, helpers, 59 tests) | all | 1 to 5 |
| `tool-contract-golden.json, TOOL_CONTRACT.txt` | regenerate, modify | FR-NEW-001 | 1 |
| `crates/core/src/tools/{contract_golden,all,tool_surface_e2e}.rs, crates/agent/src/mcp.rs` | modify (counts) | FR-NEW-001 | 1 |
| `AGENTS.md, .agent_docs/{tools,git,git-divergences}.md` | modify | docs | 1, 5 |

Affected existing tests: none modified (DEC-015); counts, registry lengths and parity docs change as for every new tool. Affected documentation: listed in the stories.

## 7. Compatibility and risks
- No consumer sees a change: a new tool, no key removed, the single publish tool untouched (FR-NEW-016).
- Risks: (1) the shared push core `push_refs` becomes multi ref; branch pushes, tag pushes and remote tag deletion must stay byte identical in behaviour, so the refactor lands first with all their existing tests unchanged (story 1). (2) The local file transport reports no per ref status and runs no hooks; the per ref refusal and the partial application of a dropped connection are produced by `#[cfg(test)]` seams (DEC-013). (3) A "must be absent" lease: libgit2's pre push advertisement carries the zero id for an absent ref (`git/remote.rs:635` reads `u.src()`); verified first by E2E-054 in story 3, fallback: classify the race from the listing taken after a failed push. (4) Annotated tags sharing a commit with the remote tag cost one probe connection each (`probe_identical`, `git_tags.rs:673-737`); the deadline is shared by the whole action (DEC-016).
- Ordering: story 1 owns the contract change, the push core refactor and the count edits; stories 2 to 5 add no tool.
- Rollback: revert the commits; no data to undo. Dependencies added or removed: none.

## 9. Test strategy
All 59 acceptance tests are functional e2e tests through the real functions of the crate against a real bare remote (`file://`) reached through a direct helper `call_publish_all_tags` (the registered tool refuses `file://`); the non member case (E2E-019) and the credential cases (E2E-022) go through the registered tool. Seams per DEC-013. Test data: generated by the existing helpers (`cloned_remote`, `advance_bare_remote`, `plain_tag_bare_remote`, `bare_ref_sha`, `publish_audit`, `with_git_hosts_lock`); none user provided. Test functions are named `e2e_tag_<nnn>_publish_all_<slug>` in the tests module of `crates/core/src/tools/git.rs`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_001_publish_all_*` | US-0096 |
| E2E-002 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_002_publish_all_*` | US-0096 |
| E2E-003 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_003_publish_all_*` | US-0096 |
| E2E-004 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_004_publish_all_*` | US-0096 |
| E2E-005 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_005_publish_all_*` | US-0096 |
| E2E-006 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_006_publish_all_*` | US-0096 |
| E2E-007 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_007_publish_all_*` | US-0096 |
| E2E-008 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_008_publish_all_*` | US-0100 |
| E2E-009 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_009_publish_all_*` | US-0097 |
| E2E-010 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_010_publish_all_*` | US-0097 |
| E2E-011 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_011_publish_all_*` | US-0097 |
| E2E-012 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_012_publish_all_*` | US-0097 |
| E2E-013 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_013_publish_all_*` | US-0098 |
| E2E-014 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_014_publish_all_*` | US-0098 |
| E2E-015 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_015_publish_all_*` | US-0098 |
| E2E-016 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_016_publish_all_*` | US-0098 |
| E2E-017 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_017_publish_all_*` | US-0096 |
| E2E-018 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_018_publish_all_*` | US-0096 |
| E2E-019 | functional e2e | registered tool, `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_019_publish_all_*` | US-0096 |
| E2E-020 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_020_publish_all_*` | US-0099 |
| E2E-021 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_021_publish_all_*` | US-0099 |
| E2E-022 | functional e2e | registered tool, `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_022_publish_all_*` | US-0099 |
| E2E-023 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_023_publish_all_*` | US-0098 |
| E2E-024 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_024_publish_all_*` | US-0097 |
| E2E-025 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_025_publish_all_*` | US-0097 |
| E2E-026 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_026_publish_all_*` | US-0099 |
| E2E-027 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_027_publish_all_*` | US-0096 |
| E2E-028 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_028_publish_all_*` | US-0097 |
| E2E-029 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_029_publish_all_*` | US-0099 |
| E2E-030 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_030_publish_all_*` | US-0096 |
| E2E-031 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_031_publish_all_*` | US-0096 |
| E2E-032 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_032_publish_all_*` | US-0096 |
| E2E-033 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_033_publish_all_*` | US-0098 |
| E2E-034 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_034_publish_all_*` | US-0096 |
| E2E-035 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_035_publish_all_*` | US-0099 |
| E2E-036 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_036_publish_all_*` | US-0099 |
| E2E-037 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_037_publish_all_*` | US-0098 |
| E2E-038 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_038_publish_all_*` | US-0097 |
| E2E-039 | functional e2e | registered tool, `e.call` | `crates/core/src/tools/git.rs` tests, `e2e_tag_039_publish_all_*` | US-0097 |
| E2E-040 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_040_publish_all_*` | US-0098 |
| E2E-041 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_041_publish_all_*` | US-0098 |
| E2E-042 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_042_publish_all_*` | US-0097 |
| E2E-043 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_043_publish_all_*` | US-0099 |
| E2E-044 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_044_publish_all_*` | US-0100 |
| E2E-045 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_045_publish_all_*` | US-0099 |
| E2E-046 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_046_publish_all_*` | US-0099 |
| E2E-047 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_047_publish_all_*` | US-0099 |
| E2E-048 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_048_publish_all_*` | US-0100 |
| E2E-049 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_049_publish_all_*` | US-0097 |
| E2E-050 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_050_publish_all_*` | US-0099 |
| E2E-051 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_051_publish_all_*` | US-0098 |
| E2E-052 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_052_publish_all_*` | US-0097 |
| E2E-053 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_053_publish_all_*` | US-0097 |
| E2E-054 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_054_publish_all_*` | US-0098 |
| E2E-055 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_055_publish_all_*` | US-0098 |
| E2E-056 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_056_publish_all_*` | US-0098 |
| E2E-057 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_057_publish_all_*` | US-0098 |
| E2E-058 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_058_publish_all_*` | US-0096 |
| E2E-059 | functional e2e | direct helper `call_publish_all_tags`, `file://` bare remote | `crates/core/src/tools/git.rs` tests, `e2e_tag_059_publish_all_*` | US-0097 |

## 10. Decisions log
- **DEC-001:** No new dependency and no version change: the stack is the manifests' (git2 0.21, `Cargo.toml:63`; rmcp 3.5.0, `Cargo.toml:57`; Rust 2024). Skill loaded: rust. **Obliged by:** n/a (no code impact), records the stack. **Rationale:** the work reuses the single publish and push code. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `Cargo.toml:57,63`.
- **DEC-002:** One new tool, `git.remote_push_tags(mount_id, remote="origin", confirm_count)`, next to `git.remote_push_tag` in the `git.remote_*` family. Tool count 115 to 116, `git.*` family 53 to 54. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-016. **Rationale:** FR-NEW-016 forbids changing the single publish; a separate tool keeps its contract exact. **Alternatives considered:** making `tag` optional on `git.remote_push_tag` (an either/or contract and an overwrite rule that bulk forbids); two tools, one to preview and one to send (the preview and the count check must share one classification, and the count guard is one action in the spec). **Source:** design. **Evidence:** `crates/core/src/mcp/server.rs:3530-3538`.
- **DEC-003:** The engine lives in `crates/core/src/tools/git_tags.rs` beside single publish (`tool_remote_push_tags`, `publish_all_tags`, `classify_tags`, `parse_confirm`, `render`); the remote layer in `crates/core/src/git/remote.rs` (`push_tags_to_remote`). Tests go into the existing tests module of `tools/git.rs` where the bare remote helpers are private. **Obliged by:** SPEC-0054/FR-NEW-002. **Alternatives considered:** a new module (the helpers `commit_on_remote`, `advertised_oid`, `probe_identical`, `short7` are private to `git_tags.rs`); looping over `publish_tag` per tag (50 tags would open 100 connections, FR-NEW-017). **Source:** existing code. **Evidence:** `git_tags.rs:535`, `:986`.
- **DEC-004:** The push core `push_refs` (`git/remote.rs:609`) takes a slice of `PushRefs` and returns the per ref statuses; every existing caller (`push_to_remote`, `push_tag_to_remote`, `delete_tag_on_remote`) passes a one element slice and behaves as before. The status callback keeps a list of `(refname, message)` instead of one cell, and the lease check records every mismatched destination instead of the first. A creation refspec carries the lease "absent" (the zero object id) so a remote that gained the tag after the listing aborts the push before anything is sent. **Obliged by:** SPEC-0054/FR-NEW-002, FR-NEW-008, FR-NEW-020. **Rationale:** one connection for N tags and a stale listing detected without sending. **Alternatives considered:** a second push function duplicating the credential, deadline and callback setup (two copies to keep in step); no lease and trusting the remote to refuse an update (the local transport raises one hard error for the whole push, so no per tag report). **Source:** design. **Evidence:** `git/remote.rs:609-680`.
- **DEC-005:** Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then `parse_confirm` (not a count), then the count comparison (wrong number), then the push. Codes in Section 5. **Obliged by:** SPEC-0054/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-015, FR-NEW-019. **Alternatives considered:** parsing the number before the connection (rejected: FR-NEW-015 puts the connection first, E2E-050). **Source:** design. **Evidence:** `git_tags.rs:467-505`.
- **DEC-006:** Classification, per local tag read from `entry.db.list_refs()` (prefix `refs/tags/`), sorted in Rust by the bytes of the name (never by the database collation). Order of checks, applied to the one listing (FR-NEW-018): commit not on remote (memoized per distinct commit with a `HashMap<Oid, bool>`, so tags sharing a commit cost one history walk), then identical (same object id, or two annotated tags on the same commit whose message the existing `probe_identical` compares), then differs (remote commit is the peeled `^{}` entry when advertised, shown with `short7`), then to publish. Time O(t log t + u d) for t tags, u distinct commits, d history depth, plus one probe connection per annotated lookalike. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-018. **Alternatives considered:** one probe fetch of all lookalikes at once (a larger refactor of `probe_remote_tag` for a case that is rare). **Source:** existing code. **Evidence:** `git_tags.rs:636-695`, `:986`.
- **DEC-007:** The whole action holds the repository `write_lock`, preview included, so the count shown and the count checked come from one consistent view and two confirmations serialize; the second finds nothing to publish and gets the wrong number refusal with `Tags to publish: 0, not N` (FR-NEW-013). `reject_if_operation_in_progress` is NOT called (FR-NEW-014). The span is `trace_remote_operation("git.remote_push_tags", ...)`. **Obliged by:** SPEC-0054/FR-NEW-013, FR-NEW-014. **Alternatives considered:** no lock for the preview (a preview racing a confirm could show a stale count; harmless but untestable); taking the lock only for the send (the count check would race). **Source:** existing code. **Evidence:** `git_tags.rs:560`.
- **DEC-008:** The count guard: `confirm_count` is `Option<serde_json::Value>`. Absent or null is a preview. A string of 1 to 9 ASCII digits or a JSON integer between 0 and 999999999 is a number. Anything else (empty string, sign, space, decimal, exponent, other script, longer than 9 digits, boolean, array, object, negative or fractional number) is the number refusal. A number different from the count of tags to publish is the wrong number refusal, and the count of 0 with a stated 0 is the "No tag to publish." answer. **Obliged by:** SPEC-0054/FR-NEW-003, FR-NEW-004, FR-NEW-019. **Rationale:** a typed integer parameter would let the transport reject `two` with its own message; a raw value lets the refusal text be the spec's. **Alternatives considered:** `Option<u32>` (transport message, FR-NEW-019 broken); `Option<String>` only (an MCP client sending the integer 2 would fail deserialization). **Source:** design. **Evidence:** `mcp/server.rs:1121-1136` (the args pattern).
- **DEC-009:** Output rendering: one `render(tags)` function builds `lines` and the multi line error message from the same list, so the preview, the report and the wrong number refusal cannot drift (FR-NEW-001, 002, 003 share the wording). The refusal line is always first. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004. **Alternatives considered:** three formatters. **Source:** design.
- **DEC-010:** The push set is the `to publish` tags whose name `git2::Reference::is_valid_name` accepts; a name it refuses (for example `a//b`, accepted by the branch rules, the SPEC-0052 case of an invalid reference name) is reported `refused by remote: invalid ref name` without a refspec, as the single publish refuses it before its push. Each tag ref is first written to the on disk repository (`repo.reference`), as the single publish does, because a push names refs. **Obliged by:** SPEC-0054/FR-NEW-008. **Alternatives considered:** pushing it and letting libgit2 fail the whole push. **Source:** existing code. **Evidence:** `git_tags.rs:688-695`.
- **DEC-011:** Reading the push result. Ok with no rejection: every pushed tag is `published`. Per ref rejections (smart protocol): that tag is `refused by remote: <reason>`, the others are `published`. Lease lost on some refs (nothing was sent): re list once, move each lost tag to `differs (remote at <7>)` or, when the remote now holds the identical object, `up to date`, drop it from the push set and push the rest again; every round removes at least one tag, so at most t rounds, and in practice one. The lists of FR-NEW-020 are decided by the fresh listing, tags not due at the first listing keep their first outcome. Any other failure (open, transport) is the connection refusal and reports nothing as sent. **Obliged by:** SPEC-0054/FR-NEW-008, FR-NEW-012, FR-NEW-020. **Alternatives considered:** one global retry (leaves a second race); reporting the tags a dropped connection may have applied (unknowable, FR-NEW-012 reports none). **Source:** design.
- **DEC-012:** The audit line is `record_audit(person, project, "git.tag_publish", "/", "published tag '<n>' at <40>")`, once per tag reported `published`, after the push returned. Nothing else is audited (preview, refusals, skipped tags). **Obliged by:** SPEC-0054/FR-NEW-009. **Alternatives considered:** one summary line (the spec fixes one per tag). **Source:** existing code. **Evidence:** `git_tags.rs:595-600`.
- **DEC-013:** `#[cfg(test)]` seams carry what a local file remote cannot produce, all keyed by remote URL and one shot so parallel tests never meet: `after_listing(url)` (exists, `git_tags.rs`, reused: moves or creates a remote tag between listing and push, E2E-054 to 056); `bulk_seams::reject(url, [(tag, reason)])` in `git/remote.rs` (the named tags are held back from the real push and reported as per ref rejections, E2E-016, 033, 041); `bulk_seams::drop_after(url, n)` (the real push carries only the first n refspecs, then reports a connection error, E2E-021). **Obliged by:** SPEC-0054/FR-NEW-008, FR-NEW-012, FR-NEW-020. **Alternatives considered:** a git hook on the bare remote (the local transport runs no hook); a fake smart HTTP server (a large harness for three tests). **Source:** existing code. **Evidence:** `git_tags.rs:745-770`, `git/remote.rs:739-750`.
- **DEC-014:** Contract changes are deliberate: `#[tool]` method and args struct in `mcp/server.rs` with frozen descriptions, a `#[cfg(test)]` registry entry in `tools/git.rs`, `TOOL_CONTRACT.txt`, the golden regenerated with `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current` and reviewed, every count site raised by one (`contract_golden.rs`, `mcp/server.rs`, `crates/agent/src/mcp.rs:317-319`, `tools/all.rs`, `tools/tool_surface_e2e.rs`, `ALL_GIT_TOOLS`), and the parity docs (`AGENTS.md`, `.agent_docs/tools.md`). Annotations: destructive hint false (nothing is overwritten or deleted), idempotent true (a repeat reports up to date), open world true. **Obliged by:** SPEC-0054/FR-NEW-001. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `TOOL_CONTRACT.txt:535`.
- **DEC-015:** No existing test is modified; the 59 tests are all new. The assertions that change are counts, registry lengths and parity docs, as for every new tool. **Obliged by:** SPEC-0054/FR-NEW-016. **Alternatives considered:** none. **Source:** design.
- **DEC-016:** Cost: the whole action shares one deadline (`remote_timeout_secs`), one listing, one push per round, and one probe connection per annotated tag whose object differs from the remote's but shares its commit. Identical objects short circuit without a probe, so 50 annotated tags already on the remote cost nothing extra (E2E-026). **Obliged by:** SPEC-0054/FR-NEW-017. **Alternatives considered:** batching the probes (DEC-006). **Source:** existing code. **Evidence:** `git_tags.rs:673-737`.
- **DEC-017:** An empty `remote` text means `origin` (the serde default applies only to an absent field): the tool maps an empty text to `origin` before the lookup. **Obliged by:** SPEC-0054/FR-NEW-010. **Alternatives considered:** refusing an empty name with the "does not track a remote named ''" text (the spec chose `origin`). **Source:** spec constraint. **Evidence:** `mcp/server.rs:843`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0096 | preview and send the due tags | P1 | FR-NEW-001, FR-NEW-002, FR-NEW-007, FR-NEW-009, FR-NEW-010, FR-NEW-011 | main | 1 | no |
| US-0097 | the confirmation number guard | P1 | FR-NEW-003, FR-NEW-004, FR-NEW-015, FR-NEW-019 | US-0096 | 2 | no |
| US-0098 | tags that are skipped, refused or changed on the remote | P1 | FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-008, FR-NEW-018, FR-NEW-020 | US-0097 | 1 | no |
| US-0099 | connection failures, concurrency, paused operations and cost | P2 | FR-NEW-012, FR-NEW-013, FR-NEW-014, FR-NEW-015, FR-NEW-017 | US-0098 | 2 | no |
| US-0100 | single publish unchanged, documentation and spec close | P2 | FR-NEW-016 | US-0098, US-0099 | 2 | yes |
