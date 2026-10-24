# US-0086: receive remote tags on fetch

> Spec: SPEC-0052 (sdlc/specs/SPEC-0052-git-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0084
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Make a successful fetch bring in the remote tags that point into the fetched branch's history (decision DEC-011, DEC-012), keep a conflicting local tag, report `tags_added` and `tags_kept_local`, and flip the two existing tests that asserted the opposite.

## Requirements covered
- SPEC-0052/FR-MOD-002: WHEN a fetch succeeds, THE Remote exchange context SHALL apply FR-NEW-009 and FR-NEW-010.
- SPEC-0052/FR-NEW-009: WHEN a fetch or a pull succeeds, THE Remote exchange context SHALL bring in the remote tags that point into the fetched branch's history, and report which were added.
- SPEC-0052/FR-NEW-010: IF a remote tag has the name of a local tag at a different commit, THEN THE Remote exchange context SHALL keep the local tag and report the tag as `kept local`.
- SPEC-0052/FR-NEW-014 (partial: fetch and pull refusal): IF the requester is not a member of the project, THEN THE system SHALL refuse to create, delete, publish, list or receive tags with the refusal it gives for any other project action, and reveal no tag name.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_ e2e_new_215 e2e_new_217` green; fetching a bare remote holding `v1.0` in `main`'s history makes `git.tags` list `v1.0` and the fetch result list it in `tags_added`.

## Design excerpt
### Decisions
- DEC-011 (SPEC-0052): Receiving tags does NOT turn on libgit2 auto tag following. `fetch_from_remote` (`git/remote.rs:703`) already lists the advertised refs; after the branch fetch it computes candidates from that list: a tag whose peeled commit (the `^{}` entry, or the ref's own oid for a lightweight tag) is the fetched branch tip or one of its ancestors (`graph_descendant_of`), and is a commit. Lightweight candidates need no download; annotated ones are fetched with explicit refspecs `+refs/tags/<n>:refs/mcpfs/incoming/<n>` in one extra fetch. The result carries the candidate list. Obliged by: FR-NEW-009, FR-MOD-002. Alternatives: `AutotagOption::Auto` or `All` (follows tags to objects present from other branches, downloads tag history we then discard, not transport independent on the `file://` test remotes), one wildcard refspec (downloads every tag).
- DEC-012 (SPEC-0052): A reconcile step in `git_tags.rs`, `reconcile_incoming_tags(entry, repo, incoming) -> (added, kept_local)`, runs right after `import_from_repo` in fetch, pull and clone: for each incoming tag, absent locally means `set_ref` and added; identical (DEC-017) means skip; different means keep the local tag and list it as kept local. Results gain `tags_added` and `tags_kept_local` (arrays of names, sorted). The incoming temporary refs are deleted from the on-disk repo. Obliged by: FR-NEW-009, FR-NEW-010, FR-NEW-011. Alternatives: letting libgit2 write refs (it cannot see the db's local tags, since only objects are hydrated, `tools/git.rs:1898`).
- DEC-017 (SPEC-0052): One function `tags_identical(repo, a, b)`: both plain with the same commit; both annotated with the same peeled commit and the same message (tagger and time ignored); mixed kinds differ. For a remote annotated tag whose object id differs from the local one, the message is read by a one tag shallow fetch (`depth 1`, refspec `+refs/tags/<n>:refs/mcpfs/probe/<n>`), then the probe ref is deleted. Obliged by: FR-NEW-007, FR-NEW-009, FR-NEW-010. Alternatives: comparing tag object ids (would compare tagger and time, which the spec excludes), asking the user (the remote exposes no message without a transfer).
- DEC-022 (SPEC-0052): Two existing tests assert the opposite of FR-MOD-002 and are modified, not deleted: `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags` (`tools/git.rs:10808-10830`) and `e2e_new_217_pull_will_inherit_the_fetch_refspec` (`:10843-10870`). Comments citing FR-NEW-062 at `git/remote.rs:745` and `tools/git.rs:10593` are updated. Obliged by: FR-MOD-002. Alternatives: leave them failing (rejected).

### Data model
No schema change. Tags are `git_refs` rows named `refs/tags/<name>` scoped by `volume_id` (DEC-004); objects live in the blob store as `git:{sha}`.

### Contracts
`git.remote_fetch` result gains `tags_added` (sorted array of tag names) and `tags_kept_local` (sorted array). Both are always present, empty when none.
`FetchOutcome` (`git/remote.rs:678`) gains `incoming_tags: Vec<IncomingTag { name, oid, peeled_commit, annotated }>`; `fetch_from_remote` fills it per DEC-011 and fetches annotated candidates into `refs/mcpfs/incoming/<name>`.
`git_tags::reconcile_incoming_tags(entry, repo, incoming) -> Result<(Vec<String>, Vec<String>)>` and `git_tags::tags_identical(repo, a: Oid, b: Oid) -> Result<bool>` (DEC-012, DEC-017).

### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (`FetchOutcome`, candidate selection, incoming fetch; update the FR-NEW-062 comment at `:745`) |
| `crates/core/src/tools/git_tags.rs` | modify (`reconcile_incoming_tags`, `tags_identical`) |
| `crates/core/src/tools/git.rs` | modify (`fetch_branch_inner` calls reconcile after `import_from_repo`; tests; flip `e2e_new_215_216` and `e2e_new_217`; update the comment at `:10593`) |

### Existing patterns
Existing pattern: `fetch_from_remote` at `crates/core/src/git/remote.rs:703` (lists advertised refs, then `update_tips` callback, `download_tags(AutotagOption::None)` at `:747`), consumed by `fetch_branch_inner` in `crates/core/src/tools/git.rs` (~3545: `write_lock`, `hydrate`, `fetch_from_remote`, `objects.import_from_repo`, `set_ref` per updated ref). Test helpers (private to the `tests` module): `seed_bare_remote` (`:9373`), `advance_bare_remote` (`:9428`), `tag_bare_remote` (`:10594`), `call_fetch_branch`, `call_pull_branch` (`:10987`), `call_clone_and_import` (`:8399`), `call_push_branch` (`:9599`).

Test harness: `e.call("git.tags", json!({"mount_id": MOUNT}))` goes through the registry with `MOUNT` and `OWNER` constants; a remote is a real bare repository in a temp dir used as a `file://` URL and reached through the direct helper functions because the registered remote tools reject `file://`. Concurrency tests: `e2e_new_084_concurrent_pushes_serialize` at `tools/git.rs:9847` and `e2e_new_577_two_concurrent_merges_exactly_one_wins` at `:13949`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` inside the `tests` module of `crates/core/src/tools/git.rs` (level: functional e2e; driver: the tool registry via `e.call`, or the direct remote helpers with a `file://` bare remote for fetch, pull, clone and publish).

### SPEC-0052/E2E-041: Fetch brings in history tags
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_041_*`
- **Category:** journey (happy). **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote `main` has `v1.0` (`a1b2c3d`) and `v1.1` (`c3d4e5f`) in its history; the project has neither. **When** Alice fetches `main`. **Then** both tags exist locally. **And** the result lists `v1.0` and `v1.1` as added.
- **Priority:** Critical

### SPEC-0052/E2E-042: Tags outside the history are ignored
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_042_*`
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote has `v9.0` at `e5f6a7b` reachable only from branch `experimental`. **When** Alice fetches `main`. **Then** `v9.0` does not exist locally. **And** the result does not mention it.
- **Priority:** High

### SPEC-0052/E2E-043: Pull keeps a conflicting local tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_043_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-010, FR-NEW-009
- **Given** local `v1.0` at `a1b2c3d`; remote `v1.0` at `c3d4e5f`. **When** Alice pulls `main`. **Then** the pull succeeds and local `v1.0` stays at `a1b2c3d`. **And** the result lists `v1.0` as kept local.
- **Priority:** Critical

### SPEC-0052/E2E-044: Same name at the same commit
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_044_*`
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-010, FR-NEW-009
- **Given** local and remote `v1.0` are both at `a1b2c3d`. **When** Alice fetches. **Then** `v1.0` is neither added nor kept local in the result. **And** nothing changes.
- **Priority:** Medium

### SPEC-0052/E2E-045: Failed fetch brings no tag
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_045_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote has a new tag `v3.0` in history. **When** Alice fetches while the remote is down, then rate limited, then with an expired credential. **Then** each gives the usual fetch refusal. **And** `v3.0` does not exist locally.
- **Priority:** High

### SPEC-0052/E2E-048: 2000 tags
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_048_*`
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote `main` history holds 2000 tags, some annotated with messages. **When** Alice fetches. **Then** all 2000 arrive, annotated ones keeping kind, message and tagger. **And** all 2000 are listed as added.
- **Priority:** Medium

### SPEC-0052/E2E-049: Non member cannot receive
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_049_*`
- **Category:** security. **Scenario:** SC-005. **Requirements:** FR-NEW-014, FR-NEW-009
- **Given** Carol is not a member. **When** Carol fetches, then pulls. **Then** each gets the usual refusal. **And** no tag arrives in `proj-a`.
- **Priority:** High

### SPEC-0052/E2E-050: Remote tag moved between two fetches
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_050_*`
- **Category:** state transition. **Scenario:** SC-005. **Requirements:** FR-NEW-009, FR-NEW-010
- **Given** Alice fetched and has `v1.0` at `a1b2c3d`; the remote then moves `v1.0` to `c3d4e5f`. **When** Alice fetches again. **Then** local `v1.0` stays at `a1b2c3d`. **And** the result lists it as kept local and no other tag changed.
- **Priority:** High

### SPEC-0052/E2E-062: Tags that do not point at a commit
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_062_*`
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-009
- **Given** the remote has `t-tree` pointing at a folder, and `v8.0` annotated pointing at another annotated tag whose commit is in `main`'s history. **When** Alice fetches `main`. **Then** `v8.0` arrives. **And** `t-tree` does not, and the result does not mention it.
- **Priority:** Medium

### SPEC-0052/E2E-076: Same commit, different message
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_076_*`
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-010, FR-NEW-009
- **Given** local annotated `v1.0` (message `A`) at `a1b2c3d`; the remote has annotated `v1.0` (message `B`) at `a1b2c3d`. **When** Alice fetches. **Then** local `v1.0` keeps message `A`. **And** the result lists `v1.0` as kept local.
- **Priority:** Medium

### SPEC-0052/E2E-MOD-002: Fetch takes in history tags (was: the existing test "fetch brings branches and takes no tags", see 14.1)
- Level / Driver / Location: functional e2e / direct remote helper with `file://` bare remote / `crates/core/src/tools/git.rs` tests module, `e2e_tag_mod_002_*`
- **Scenario:** SC-005. **Requirements:** FR-MOD-002, FR-NEW-009
- **Validated before:** after fetching a remote with `v1.0`, no tag existed.
- **Validates now:** branches arrive as before and `v1.0` also arrives.
- **Given** the remote `main` has `v1.0` at `a1b2c3d` in its history. **When** Alice fetches `main`. **Then** the branch's remote position is updated as before. **And** `v1.0` exists locally and is listed as added.
- **Category:** state transition. **Priority:** Critical

### 9.5 Removed tests
None. The "no tag follows" assertion of the existing test is replaced by E2E-MOD-002 and E2E-042.

## 10. Traceability
| Scenario | Happy | Failure (incl. refusals) | Edge | State, side effect, cross |
|---|---|---|---|---|
| SC-001 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-052 | E2E-006, E2E-009, E2E-010, E2E-012, E2E-014, E2E-015, E2E-016, E2E-067, E2E-070, E2E-081, E2E-082 | E2E-005, E2E-011, E2E-013, E2E-061, E2E-078, E2E-080 | E2E-007, E2E-008, E2E-017, E2E-018, E2E-019, E2E-051, E2E-068, E2E-079 |
| SC-002 | E2E-020 | E2E-023 | E2E-021, E2E-022, E2E-025 | E2E-024, E2E-MOD-001 |
| SC-003 | E2E-026 | E2E-027, E2E-030, E2E-032, E2E-053 | E2E-028, E2E-065 | E2E-029, E2E-031, E2E-066 |
| SC-004 | E2E-033, E2E-072 | E2E-034, E2E-036, E2E-038, E2E-039, E2E-040, E2E-055, E2E-057, E2E-058, E2E-069, E2E-071, E2E-074, E2E-075, E2E-083, E2E-085 | E2E-035, E2E-056, E2E-073, E2E-077, E2E-084 | E2E-037 |
| SC-005 | E2E-041, E2E-046 | E2E-043, E2E-045, E2E-047, E2E-049, E2E-060, E2E-063, E2E-064, E2E-076 | E2E-042, E2E-044, E2E-048, E2E-054, E2E-059, E2E-062 | E2E-050, E2E-MOD-002 |

Requirement to tests (recounted from the 9.1 table; every requirement has at least 3):

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, E2E-002, E2E-003, E2E-007, E2E-013, E2E-015, E2E-052, E2E-078, E2E-080 (9) |
| FR-NEW-002 | E2E-004, E2E-005, E2E-008, E2E-051, E2E-078 (5) |
| FR-NEW-003 | E2E-006, E2E-007, E2E-008, E2E-016, E2E-068, E2E-079 (6) |
| FR-NEW-004 | E2E-009, E2E-010, E2E-011, E2E-012, E2E-013, E2E-081, E2E-082 (7) |
| FR-NEW-005 | E2E-026, E2E-027, E2E-028, E2E-030, E2E-031, E2E-032, E2E-053 (7) |
| FR-NEW-006 | E2E-017, E2E-018, E2E-024, E2E-029 (4) |
| FR-NEW-007 | E2E-033, E2E-034, E2E-035, E2E-038, E2E-039, E2E-040, E2E-051, E2E-056, E2E-069, E2E-075, E2E-077, E2E-085 (12) |
| FR-NEW-008 | E2E-036, E2E-037, E2E-057, E2E-058, E2E-069, E2E-083 (6) |
| FR-NEW-009 | E2E-041, E2E-042, E2E-043, E2E-044, E2E-045, E2E-048, E2E-049, E2E-050, E2E-051, E2E-054, E2E-062, E2E-063, E2E-076, E2E-MOD-002 (14) |
| FR-NEW-010 | E2E-043, E2E-044, E2E-050, E2E-060, E2E-064, E2E-076 (6) |
| FR-NEW-011 | E2E-046, E2E-047, E2E-059, E2E-060 (4) |
| FR-NEW-012 | E2E-006, E2E-019, E2E-031, E2E-035, E2E-040, E2E-051, E2E-066, E2E-077, E2E-079 (9) |
| FR-NEW-013 | E2E-014, E2E-052, E2E-067 (3) |
| FR-NEW-014 | E2E-015, E2E-023, E2E-030, E2E-040, E2E-049, E2E-058, E2E-074 (7) |
| FR-NEW-015 | E2E-016, E2E-061, E2E-068 (3) |
| FR-NEW-016 | E2E-032, E2E-065, E2E-066 (3) |
| FR-NEW-017 | E2E-054, E2E-063, E2E-064 (3) |
| FR-NEW-018 | E2E-055, E2E-056, E2E-057, E2E-083 (4) |
| FR-NEW-019 | E2E-071, E2E-072, E2E-073, E2E-084 (4) |
| FR-NEW-020 | E2E-070, E2E-074, E2E-075 (3) |
| FR-NEW-021 | E2E-080, E2E-081, E2E-082 (3) |
| FR-MOD-001 | E2E-020, E2E-021, E2E-022, E2E-024, E2E-025, E2E-MOD-001 (6) |
| FR-MOD-002 | E2E-041, E2E-042, E2E-MOD-002 (3) |

## 11. Open questions
None.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Tag | a named mark on a commit | Versioning |
| Plain tag | a tag with no message; shown as `lightweight` | Versioning |
| Annotated tag | a tag with a message, a tagger and a creation time | Versioning |
| Tagger | the member who created an annotated tag | Versioning |
| Kind | `lightweight` or `annotated` | Versioning |
| Overwrite confirmation | the member's explicit statement that an existing tag can be replaced | Versioning, Remote exchange |
| Publish | send one named tag to the remote | Remote exchange |
| Receive | bring remote tags into the project on fetch, pull or clone | Remote exchange |
| In history | reachable from the fetched branch | Remote exchange |
| Kept local | a remote tag skipped because a local tag of that name points elsewhere | Remote exchange |
| Up to date | the remote already has the identical tag | Remote exchange |
| Paused operation | a merge, rebase, cherry pick or revert waiting to be finished or abandoned | Versioning |
| Audit entry | the operator visible record of a change | Audit |
| Member | a person who belongs to the project | Actors |
| Non member | a person who does not belong to the project | Actors |
| Remote | the hosted repository the project tracks | Remote exchange |
| Operator | the person who reads the audit trail | Audit |
| Usual refusal | the refusal given today to a non member for any other project action | Actors |
| Target | the commit, branch or tag a new tag is placed on; default the checked out commit | Versioning |
| Fetch, pull, clone | bring remote content into the project; pull also merges it; clone creates the project from the remote | Remote exchange |
| Commit | a recorded state of the project's files | Versioning |
| `lightweight` | the label shown for a plain tag | Versioning |
| Branch | a named line of work in the project | Versioning |
| Checked out commit | the commit the project's files currently match | Versioning |
| Credential | the stored token a member's account holds for the remote | Remote exchange |
| Added | a tag newly brought in by a fetch, pull or clone and listed as such in its result | Remote exchange |
| Identical tags | same name, kind and commit and, for annotated tags, same message | Remote exchange |
| On the remote | a commit belonging to the history of any branch the remote holds | Remote exchange |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 6 | 1 | NOT-IMPLEMENTABLE |
| 2 | 5 | 3 | NOT-IMPLEMENTABLE |
| 3 | 2 | 2 | NOT-IMPLEMENTABLE |
| 4 | 4 | 2 | NOT-IMPLEMENTABLE |
| 5 | 4 (all minor precision) | 2 | NOT-IMPLEMENTABLE |
| waiver | 0 open | 0 open | WAIVED by the owner on 2026-10-24: round 5 amendments applied, not re-audited |

**Amendments applied:** rounds 1 and 2 amendments verified by the next round; round 3 amendments verified by round 4; round 4 amendments verified by round 5; round 5 amendments (forbidden character wording, refusal order in FR-NEW-018, remote name refusal and E2E-085, tagger and time format, two design inputs) applied, not audited.
**No implementation detail outside Section 14:** FAIL at round 2 (minor), amended at round 3; later rounds raised none. **Accepted concern:** the amendments of rounds 5 were not re-audited.

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Listing shows name and commit only | `crates/core/src/tools/git.rs:2016-2022` |
| Only a listing tag tool exists | `crates/core/src/mcp/server.rs:3290`, `TOOL_CONTRACT.txt:599-603` |
| Fetch takes no tag, by design | `crates/core/src/git/remote.rs:745-747` |
| A test asserts it (test name `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags`, the one E2E-MOD-002 replaces) | `crates/core/src/tools/git.rs:10808-10830` |
| Clone records no tag | `crates/core/src/tools/git.rs:3105-3200` (no `refs/tags` write; recounted by search) |
| Publish sends one branch refspec | `crates/core/src/git/remote.rs:588-592` |
| Branch changes are refused while an operation is paused | `crates/core/src/tools/git.rs:1169,1226,1245,1264,1283,1302`, message at `:4201-4202` |
| Overwrite of a remote branch needs the remote's current commit | `crates/core/src/git/remote.rs:545-592`, `tools/git.rs:1513` |
| Branch naming rules | `crates/core/src/tools/git.rs:2200-2240` |
| FR-NEW-062 | `sdlc/specs/SPEC-0011-github-enterprise-and-token-store/spec.md:243` |
| Tag push out of scope earlier | `sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:54` |
| Membership is checked before any tag action | `crates/core/src/tools/git.rs:1781-1788` (`open` calls `authorize` first), listing uses it at `:1318` |
| Branch name limit is 255 bytes | `crates/core/src/tools/git.rs:2194` |
| FR-NEW-155 to 157 lease rules and FR-NEW-279/280 paused operation rule | `crates/core/src/git/remote.rs:545-592`; `sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:215` |
| Short commit display today is 8 characters | `crates/core/src/tools/git.rs:1866-1868` (this spec shows 7; see 14.3) |
| First ranked gap of the register | `.agent_docs/git-divergences.md` row D15 |
| Pull ending in a conflict is an `Ok` result with `status: "conflict"` | `AGENTS.md` section "Behaviour worth knowing"; `crates/core/src/tools/git.rs:1747` (`tool_remote_pull`) |
Test command: `cargo test --workspace`

### 14.2 Existing tests to modify
| Test | Site | Replaced by |
|---|---|---|
| `e2e_new_215_216_fetch_brings_branches_and_takes_no_tags` | `crates/core/src/tools/git.rs:10808-10830` | E2E-MOD-002 (branch assertions kept, "no tag" assertions removed) |
| `e2e_new_217_pull_will_inherit_the_fetch_refspec` | `crates/core/src/tools/git.rs:10843-10870` (asserts `refs/tags/v1` absent through the shared fetch primitive) | E2E-MOD-002 on the shared fetch primitive; its "no tag" assertions are removed and `v1` asserted present; signal: `cargo test --workspace` fails on it |

### 14.3 Design inputs
- Proposed action names: tag create, tag delete, publish via the existing remote push action with a tag parameter; the frozen tool contract (`tool-contract-golden.json`, regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1`) and `TOOL_CONTRACT.txt` must be updated; detection signal: the three golden contract tests fail until regenerated.
- Overwrite on publish: the existing force path ties force to a lease (`git/remote.rs:545-592`, `tools/git.rs:1513`). FR-NEW-008 maps to it; signal: a force flag without a lease is refused today, so a test of E2E-037 fails if mapped naively.
- Fetch uses one explicit refspec with tag following off (`git/remote.rs:745-751`, `tools/git.rs:3504` cited by SPEC-0011 is stale). "In history" needs either an automatic follow or a computed reachability check; signal: E2E-042.
- Clone path imports refs by hand (`tools/git.rs:3118-3125`); it must also write tag refs; signal: E2E-046.
- Annotated tag objects must survive the object import and export between the blob store and the libgit2 cache (`git/odb.rs`); signal: E2E-048 kind and tagger after receive.
- Tagger identity: the caller's person id, as for commits (`tools/git.rs:368-371`).
- Audit entries use the existing audit recorder (`tools/git.rs:1414`); an `up to date` publish records none.
- Concurrency: tag creation must run under the repository write lock already used by commit (`tools/git.rs:2121-2135`); signal: E2E-016.
- Messages `Tag '...' already exists...` map to the existing no clobber refusal code, `Tag '...' does not exist` to the not found code, invalid names to the invalid argument code (`errors.rs:15,16,20`); the codes are design's choice.
- Stash references live under the same reference store and never appear as tags (`tools/git.rs:17310` test); keep that.
- The 5000 tag listing time has no baseline; measure in design. No threshold is promised (7.1).
- Abbreviated commits: the existing helper shows 8 characters (`tools/git.rs:1866`), this spec's messages show 7. Design either aligns the helper call for these messages to 7 or raises the spec; signal: E2E-036 and E2E-057 assert the exact text.
- Listing order is ascending by name compared byte by byte (FR-MOD-001); today's order comes from `list_refs()` (`tools/git.rs:2009-2022`) and is not documented; signal: E2E-025.
- Pull that pauses on a conflict returns `Ok` with `status: "conflict"`; tag import must happen before that return; signal: E2E-054.
- Clone into an already populated project is allowed today (`tools/git.rs:3174-3176`, re-cloning replaces the origin row), which E2E-060 relies on; signal: E2E-060.
- "On the remote" (FR-NEW-019) needs the remote's advertised branch tips at publish time; the push transport already reads them before sending (`git/remote.rs:545-592`); signal: E2E-071, E2E-073.
- Audit lines (FR-NEW-012) go through the existing recorder with action, path and detail; design picks the action labels; signal: E2E-019.
- The listing, publish and receive comparisons must peel annotated tags (a tag of a tag included) to the commit; `refs_under` returns the unpeeled target today (`crates/core/src/tools/git.rs:2008-2022`); signal: E2E-004 (commit shown), E2E-062, E2E-069.
- The published list of actions is a machine checked contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`); adding tag actions is a deliberate change of it; signal: the three golden contract tests.
- `git.tags` entries keep `name`, `full_ref` and `sha`; kind, message and tagger are added keys; signal: the existing listing assertions and the golden contract tests.
- Update the comments citing FR-NEW-062 at `git/remote.rs:745-746` and `tools/git.rs:10593` (review only, NOTHING DETECTS THIS automatically).
- Tagger identifier: use the caller's commit identity (`tools/git.rs:368-371`); signal: E2E-004.
- Tags that point at a non commit (a tree or a blob) must be filtered on receive; signal: E2E-062.
- `tools/git.rs:3504` cited by SPEC-0011 for FR-NEW-062 is stale; the live site is `git/remote.rs:745-747`.

## Constraints
- Keep `AutotagOption::None` and prune off: tags come only from the explicit candidate step.
- Never overwrite a local tag.
- Pull and clone wiring belong to the next stories; the shared primitive must already return `incoming_tags` for them.
- Only commits are brought in; a tag pointing at a folder or file is skipped.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the whole existing git suite stays green (`cargo test -p mcp-fs-core --lib git`), except the two tests this spec modifies (DEC-022) in the story that owns them; `tool_contract_golden_is_current` green after review of the regenerated golden diff.
- [ ] Docs updated: none in this story unless Paths lists them; the closing story updates the documentation.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0086: receive remote tags on fetch`, body `Spec: SPEC-0052` and the requirement ids.
