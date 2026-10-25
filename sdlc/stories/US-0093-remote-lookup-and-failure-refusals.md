# US-0093: remote lookup, connection failures and declined deletions

> Spec: SPEC-0053 (sdlc/specs/SPEC-0053-delete-a-tag-on-the-remote/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0092
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Complete the refusal order around the remote: no remote at all, a named remote that is not tracked (default `origin`), credential and connection failures, a remote that declines, and the answer lost after the remote applied the deletion, always without audit.

## Requirements covered
- SPEC-0053/FR-NEW-005: IF the project tracks no remote at all, THEN THE Remote exchange context SHALL refuse with `Cannot delete tag 'v1.0': this project tracks no remote.`
- SPEC-0053/FR-NEW-006: IF the remote declines the deletion, THEN THE Remote exchange context SHALL refuse with `Remote refused to delete tag 'v1.0': <the remote's reason>` and leave the remote tag in place.
- SPEC-0053/FR-NEW-007 (partial: member, no remote, named remote, connection, then the tag checks): WHEN more than one refusal applies to one request, THE system SHALL give only the first of this list: not a member (FR-NEW-009); the project tracks no remote at all (FR-NEW-005); the named remote is not tracked (FR-NEW-014); the remote or the connection fails (FR-NEW-012, FR-NEW-013); the remote tag is absent (FR-NEW-004); no commit is stated (FR-NEW-002); the stated commit differs (FR-NEW-003); the remote declines (FR-NEW-006).
- SPEC-0053/FR-NEW-009: IF the requester is not a member of the project, THEN THE system SHALL refuse with `'carol@example.com' is not a member of 'proj-a'` (the requester's identifier and the project name), and reveal no tag name, commit or sign that the tag exists.
- SPEC-0053/FR-NEW-012: IF the remote is down, rate limits the request, does not answer in time, or the member's credential expired or is missing, THEN THE Remote exchange context SHALL refuse, leave the remote tag in place and record nothing.
- SPEC-0053/FR-NEW-013: IF the connection is lost after the deletion request was sent and before the remote's answer arrived, THEN THE Remote exchange context SHALL refuse with the connection refusal of FR-NEW-012 and record nothing, even when the remote did delete the tag.
- SPEC-0053/FR-NEW-014: IF the project tracks at least one remote but not the one the member named, THEN THE Remote exchange context SHALL refuse with `Cannot delete tag 'v1.0': this project does not track a remote named 'upstream'.`
- SPEC-0053/FR-NEW-015 (partial: named remote reported): WHEN a remote tag is really deleted, THE Remote exchange context SHALL tell the member the tag name as given, the remote name used and the full 40 character id of the commit the tag had. Behavioral and technology agnostic. Users: Alice and Bob are members of `proj-a`; Carol is not. The remote is a GitHub Enterprise repository tracked by `proj-a` under the name `origin`. Unless stated, the remote tag `v1.0` points at the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5` (shown as `f6a7b8c`) and the local tag `v1.0` points at `c3d4e5f`; the checked out commit is `a1b2c3d`.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag_` green; with `proj-a` tracking only `upstream`, a delete naming no remote is refused with `Cannot delete tag 'v1.0': this project does not track a remote named 'origin'.` and the audit trail is unchanged.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0053): Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then the tag checks (absent, no commit, differs), then the delete push (declined). Codes: not stated `invalid_argument`; differs `no_clobber` (existing `moved`); absent, no remote, named remote not tracked `not_found`; declined and connection failures `internal`.
- DEC-011 (SPEC-0053): `remote` defaults to `origin` through the serde default, as in `git.remote_push_tag`; a blank text is a given name. Remote lookup: `list_remotes()` empty gives the "tracks no remote" text, non empty without the name gives "does not track a remote named '<r>'" with the name as given, case sensitive.
- DEC-012 (SPEC-0053): A per ref rejection from the remote (`PushFailure::Rejected`) becomes `Remote refused to delete tag '<n>': <reason>` through a new helper in `git/remote.rs`; `remote_refused_tag` (`git/remote.rs:784`) words a name refusal and is not reused.
- DEC-013 (SPEC-0053): Connection, rate limit, timeout and credential refusals are produced by the code paths publication already uses (`connection_error`, `timeout_error`, `resolve_clone_credential`), unchanged, so one vocabulary covers every remote action.
- DEC-015 (SPEC-0053): Three `#[cfg(test)]` seams carry the cases a local file remote cannot produce: `after_listing(url)` (exists, `git_tags.rs`, reused: moves or deletes the remote tag between listing and push), `forced_rejection(url)` (the delete push reports a per ref rejection, since the local transport runs no hooks) and `fault_after_push(url)` (a successful delete push is reported as a connection error). `closing_remote` simulates a dropped connection. Production builds contain none of them.

### Data model
No schema change; a remote deletion writes nothing to the database.

### Contracts
Texts (verbatim) and codes: `Cannot delete tag '<n>': this project tracks no remote.` (`not_found`, project tracks no remote at all); `Cannot delete tag '<n>': this project does not track a remote named '<r>'.` (`not_found`, other remotes exist; names are case sensitive; omitted name means `origin`; a blank name counts as given); connection, rate limit and timeout: the existing `fetch from host '<h>' failed: <reason>` and `remote timeout: host '<h>' did not respond within <n>s`; credential: the existing `token expired for host <h>; ...` and `no token for host <h>; ...`; declined: `Remote refused to delete tag '<n>': <the remote's reason>` (`internal`, new helper, do not reuse `remote_refused_tag`).
Full order: member, no remote at all, named remote not tracked, credential and connection, remote tag absent, no commit, commit differs, remote declines.
No audit entry for any refusal or connection failure, including the answer lost after the remote applied the deletion (the push returns an error, so the audit line is never reached).
Test seams (`#[cfg(test)]` only): `forced_rejection(url)` makes the delete push report a per ref rejection with a given reason (the local transport runs no hooks); `fault_after_push(url)` turns a successful delete push into a connection error so the remote tag is gone while the member sees a refusal. Reuse `closing_remote` for connections that drop.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (remote lookup texts, declined mapping, the two test seams) |
| `crates/core/src/git/remote.rs` | modify (map `PushFailure::Rejected` for a delete) |
| `crates/core/src/tools/git.rs` | modify (tests and helpers) |

### Existing patterns
Existing pattern, the remote layer: `push_refs` (`crates/core/src/git/remote.rs:609`, `PushRefs`, `PushFailure`, lease in `push_negotiation`), `push_tag_to_remote` (`:696`), `tag_push_error` (`:767`), `list_advertised_refs` (`:793`), `connection_error` (`:1226`), `timeout_error` (`:1269`); stated commit rules `stated_commit_matches` (`git_tags.rs:799`), `short7` (`:812`), `moved` (`:774`).

Test helpers (private to the tests module of `crates/core/src/tools/git.rs`): `call_publish_tag`, `publish_as`, `bare_refs`, `publish_audit`, `cloned_remote`, `advance_bare_remote`, `plain_tag_bare_remote`, `bare_ref_sha`, `closing_remote`, `with_git_hosts_lock`; the seam `after_listing` in `git_tags.rs`; the SPEC-0052 tests `e2e_tag_033` to `e2e_tag_085` show the shapes (audit assertions read the audit log like `publish_audit`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `e2e_tag_<number>_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_delete_remote_tag` with a `file://` bare remote (the registered tool for non member and expired credential cases).

### SPEC-0053/E2E-017: The project tracks no remote
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_017_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-005
- **Given** `proj-a` tracks no remote at all. **When** Alice deletes `v1.0` and states `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project tracks no remote.`. **And** no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-018: No remote outranks absent tag and missing commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_018_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005, FR-NEW-007
- **Given** `proj-a` tracks no remote. **When** Alice deletes `v9.9` and states no commit. **Then** the refusal is `Cannot delete tag 'v9.9': this project tracks no remote.`. **And** no audit entry was added.
- **Priority:** Medium

### SPEC-0053/E2E-019: The remote declines a protected tag
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_019_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** the remote protects `v1.0` and answers `protected tag` to a deletion; `v1.0` points at `f6a7b8c`. **When** Alice deletes `v1.0` and states `f6a7b8c`. **Then** the refusal is `Remote refused to delete tag 'v1.0': protected tag`. **And** the remote tag `v1.0` still points at `f6a7b8c` and no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-020: The remote's reason is relayed as given
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_020_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-006
- **Given** the remote declines the deletion of `release-2026` with the reason `deletion of release tags is forbidden by policy`. **When** Bob deletes `release-2026` stating its correct commit. **Then** the refusal is `Remote refused to delete tag 'release-2026': deletion of release tags is forbidden by policy`. **And** the remote tag is unchanged.
- **Priority:** Medium

### SPEC-0053/E2E-021: A wrong commit outranks the remote's protection
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_021_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-006, FR-NEW-007
- **Given** `v1.0` is protected on the remote and points at `f6a7b8c`. **When** Alice states `c3d4e5f`. **Then** the refusal is `Remote tag 'v1.0' has moved to f6a7b8c since you last saw it. Fetch, then confirm again.`, not the protection refusal. **And** the remote tag is unchanged.
- **Priority:** Medium

### SPEC-0053/E2E-024: Non member outranks no remote
- Level / Driver / Location: functional e2e / registered tool / `crates/core/src/tools/git.rs`, `e2e_tag_024_*`
- **Category:** security. **Scenario:** SC-001. **Requirements:** FR-NEW-007, FR-NEW-009
- **Given** Carol is not a member and `proj-a` tracks no remote. **When** Carol deletes `v1.0`. **Then** the refusal is `'carol@example.com' is not a member of 'proj-a'`, not the no remote one. **And** no audit entry was added.
- **Priority:** Medium

### SPEC-0053/E2E-032: Remote down, rate limited, credential expired, no answer
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_032_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012
- **Given** the remote tag `v1.0` points at `f6a7b8c` on the host `github.ibm.com`. **When** Alice deletes `v1.0` stating `f6a7b8c` while the remote is unreachable, then while it rate limits, then with an expired credential, then with no stored credential, then while it does not answer in time. **Then** the refusals are, in turn, `fetch from host 'github.ibm.com' failed: <the transport's reason>`, `fetch from host 'github.ibm.com' failed: <the rate limit reason the remote gave>`, `token expired for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, `no token for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, and `remote timeout: host 'github.ibm.com' did not respond within <n>s`. **And** the remote tag `v1.0` stays and no audit entry was added.
- **Priority:** Critical

### SPEC-0053/E2E-033: Connection lost before the deletion was applied
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_033_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-012, FR-NEW-013
- **Given** the remote tag `v1.0` points at `f6a7b8c`. **When** the connection drops while Alice deletes `v1.0`, before the remote applied the request. **Then** Alice receives `fetch from host 'github.ibm.com' failed: <the transport's reason>` and the remote tag `v1.0` is still there. **And** no audit entry was added; a retry by Alice stating `f6a7b8c` succeeds and leaves exactly one entry.
- **Priority:** High

### SPEC-0053/E2E-034: Connection failure outranks absent tag and missing commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_034_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-007, FR-NEW-012
- **Given** the remote is unreachable. **When** Alice deletes `v9.9` and states no commit. **Then** she receives the connection refusal, not the absent tag or missing commit refusal. **And** no audit entry was added.
- **Priority:** Medium

### SPEC-0053/E2E-041: No remote while a local tag exists
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_041_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-005
- **Given** `proj-a` tracks no remote and has the local tag `v1.0`. **When** Alice deletes `v1.0` stating `a1b2c3d`. **Then** the refusal is `Cannot delete tag 'v1.0': this project tracks no remote.`. **And** the local tag `v1.0` still exists.
- **Priority:** Medium

### SPEC-0053/E2E-042: Connection lost after the remote applied the deletion
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_042_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** the remote tag `v1.0` points at `f6a7b8c`. **When** Alice deletes `v1.0` stating `f6a7b8c`; the remote applies the deletion, then the connection drops before the answer reaches her. **Then** Alice receives `fetch from host 'github.ibm.com' failed: <the transport's reason>`. **And** no audit entry was added although the remote tag is gone; a retry by Alice is refused with `Remote tag 'v1.0' does not exist.`.
- **Priority:** High

### SPEC-0053/E2E-043: Another member finds the deletion Alice was unable to confirm
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_043_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-013
- **Given** the same state as E2E-042. **When** Bob deletes `v1.0` stating `f6a7b8c`. **Then** Bob is refused with `Remote tag 'v1.0' does not exist.`. **And** the trail holds no entry for `v1.0`.
- **Priority:** Medium

### SPEC-0053/E2E-044: The project does not track the named remote
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_044_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** `proj-a` tracks only the remote `origin`. **When** Alice deletes `v1.0` on the remote `upstream` stating `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project does not track a remote named 'upstream'.`. **And** the remote `origin` is untouched and no audit entry was added.
- **Priority:** High

### SPEC-0053/E2E-045: Remote names are case sensitive
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_045_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** `proj-a` tracks only the remote `origin`. **When** Alice deletes `v1.0` on the remote `Origin` stating `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project does not track a remote named 'Origin'.`. **And** the remote `origin` is untouched.
- **Priority:** Medium

### SPEC-0053/E2E-046: A named remote that is not tracked outranks an absent tag and a missing commit
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_046_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-014, FR-NEW-007
- **Given** `proj-a` tracks only the remote `origin`. **When** Alice deletes `v9.9` on the remote `upstream` stating no commit. **Then** the refusal is `Cannot delete tag 'v9.9': this project does not track a remote named 'upstream'.`. **And** no audit entry was added.
- **Priority:** Medium

### SPEC-0053/E2E-048: The reported remote is the one named
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_048_*`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-015
- **Given** `proj-a` tracks the remotes `origin` and `upstream`; `upstream` has `v1.0` at `f6a7b8c`. **When** Alice deletes `v1.0` on `upstream` stating `f6a7b8c`. **Then** Alice is told the tag `v1.0`, the remote `upstream` and the commit `f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5`; the audit line reads `deleted tag 'v1.0' on remote 'upstream' (was at f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5)`. **And** the tag `v1.0` of `origin` is untouched.
- **Priority:** Medium

### SPEC-0053/E2E-052: No remote named, and origin is not tracked
- Level / Driver / Location: functional e2e / direct helper, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_052_*`
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-014
- **Given** `proj-a` tracks only the remote `upstream`. **When** Alice deletes `v1.0` naming no remote and states `f6a7b8c`. **Then** the refusal is `Cannot delete tag 'v1.0': this project does not track a remote named 'origin'.`. **And** no audit entry was added.
- **Priority:** High

### 9.4 Modified tests
None.

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Happy | Failure | Edge | State, side effect, cross |
|---|---|---|---|---|
| SC-001 | E2E-001 | E2E-007, E2E-010, E2E-014, E2E-017, E2E-019, E2E-022, E2E-023, E2E-024, E2E-026, E2E-032, E2E-033, E2E-042, E2E-044, E2E-052 | E2E-002, E2E-003, E2E-004, E2E-006, E2E-008, E2E-011, E2E-012, E2E-015, E2E-016, E2E-018, E2E-020, E2E-021, E2E-027, E2E-028, E2E-034, E2E-036, E2E-037, E2E-040, E2E-041, E2E-043, E2E-045, E2E-046, E2E-047, E2E-048, E2E-049, E2E-050, E2E-051 | E2E-005, E2E-009, E2E-013, E2E-025, E2E-029, E2E-030, E2E-031, E2E-035, E2E-038, E2E-039 |

Requirement to tests (recounted; every requirement has at least 3):

| Requirement | Tests |
|---|---|
| FR-NEW-001 | E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-027, E2E-028, E2E-029, E2E-035, E2E-036, E2E-037 (11) |
| FR-NEW-002 | E2E-010, E2E-011, E2E-012, E2E-013, E2E-027 (5) |
| FR-NEW-003 | E2E-006, E2E-007, E2E-008, E2E-009, E2E-031, E2E-039, E2E-051 (7) |
| FR-NEW-004 | E2E-014, E2E-015, E2E-016, E2E-028, E2E-030, E2E-035, E2E-037, E2E-038, E2E-049, E2E-050 (10) |
| FR-NEW-005 | E2E-017, E2E-018, E2E-041 (3) |
| FR-NEW-006 | E2E-019, E2E-020, E2E-021 (3) |
| FR-NEW-007 | E2E-015, E2E-018, E2E-021, E2E-022, E2E-024, E2E-034, E2E-046 (7) |
| FR-NEW-008 | E2E-025, E2E-026, E2E-027 (3) |
| FR-NEW-009 | E2E-022, E2E-023, E2E-024 (3) |
| FR-NEW-010 | E2E-029, E2E-038, E2E-039 (3) |
| FR-NEW-011 | E2E-030, E2E-031, E2E-040 (3) |
| FR-NEW-012 | E2E-032, E2E-033, E2E-034 (3) |
| FR-NEW-013 | E2E-033, E2E-042, E2E-043 (3) |
| FR-NEW-014 | E2E-044, E2E-045, E2E-046, E2E-052 (4) |
| FR-NEW-015 | E2E-001, E2E-047, E2E-048 (3) |

## 11. Open questions
None.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Remote tag | a tag held by the remote, not by the project | Remote exchange |
| Confirmation commit | the commit the member states to confirm a deletion: the full id or at least 7 hexadecimal characters, case insensitive, spaces ignored | Remote exchange |
| Moved | a remote tag whose commit is no longer the one the member saw or stated | Remote exchange |
| Declined | a deletion the remote refuses (for example a protected tag) | Remote exchange |
| Tag | a named mark on a commit | Remote exchange |
| Local tag | a tag held by the project | Remote exchange |
| Commit | a recorded state of the project's files | Remote exchange |
| Credential | the stored token a member's account holds for the remote | Remote exchange |
| Audit trail | the sequence of audit lines | Audit |
| Remote | the hosted repository the project tracks, known by a name (default `origin`) | Remote exchange |
| Branch | a named line of work in the project; deleting a tag never moves one | Remote exchange |
| Project | the set of files, tags and remotes the members work on (`proj-a` in the tests) | Actors |
| Host | the server name of a remote, shown in connection refusals | Remote exchange |
| Refusal | the message a member receives when an action is not carried out | Actors |
| Audit entry | one audit line with the member who caused it | Audit |
| Member | a person who belongs to the project | Actors |
| Non member | a person who does not belong to the project | Actors |
| Operator | the person who reads the audit trail | Audit |
| Audit line | the operator visible record of a deletion | Audit |
| Paused operation | a merge, rebase, cherry pick or revert waiting to be finished or abandoned | Remote exchange |
| Annotated tag | a tag with a message of its own, confirmed here by the commit it points at | Remote exchange |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 5 | 3 | NOT-IMPLEMENTABLE |
| 2 | 3 | 0 | NOT-IMPLEMENTABLE |
| 3 | 4 | 3 | NOT-IMPLEMENTABLE |
| 4 | 0 | 3 | IMPLEMENTABLE |
| consistency review | 3 precision points | 0 | fixed |

**Amendments applied:** round 1 (non member and connection texts spelled, FR-NEW-013 lost answer, FR-NEW-014 named remote, traceability column, E2E-042 to 046), round 2 (FR-NEW-015 result content, invalid tag names count as absent, non hexadecimal stated text, E2E-047 to 051), round 3 (default remote `origin`, E2E-052, annotated tag test E2E-027, race wording), round 4 (outcome statistics, gate record, timeout setting design input), final review (outcome statistics recounted, missing credential named as a trigger of FR-NEW-012, glossary completed, edge case table row).
**No implementation detail outside Section 14:** PASS (the tool names inside two verbatim messages are existing user visible text)

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)

### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Publishing sends one tag, replaces a differing remote tag only with the stated commit | `crates/core/src/tools/git_tags.rs:534-726` (`publish_tag`, `publish_inner`, `publish_on_repo`), `:799` (`stated_commit_matches`, 7 character minimum at `:29`) |
| The "moved" refusal wording | `crates/core/src/tools/git_tags.rs:774` (`moved`) |
| No remote refusal for publishing | `crates/core/src/tools/git_tags.rs:476-483` |
| Advertised refs listing and tag push | `crates/core/src/git/remote.rs:793` (`list_advertised_refs`), `:696` (`push_tag_to_remote`), `:767` (`tag_push_error`), `:784` (`remote_refused_tag`) |
| Publishing is not blocked by a paused operation | `crates/core/src/tools/git_tags.rs:466-499` (no `reject_if_operation_in_progress` call, unlike `tag_create` at `:44`) |
| Local deletion never touches the remote | `crates/core/src/tools/git_tags.rs:64-78` (`tool_tag_delete`), `:191` (`tag_delete`), SPEC-0052/FR-NEW-005 |
| Audit lines | `crates/core/src/tools/git_tags.rs:592-600` (publication) |
| Non member refusal text | `crates/core/src/storage/admin.rs:393` (`'{person}' is not a member of '{project_id}'`) |
| Connection, credential and timeout refusal texts | `crates/core/src/git/remote.rs:1226-1229` (`connection_error`), `:1269-1273` (`timeout_error`); `crates/core/src/git/oauth/store.rs:254-258` |
| Remote deletion is out of scope in SPEC-0052 | `sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:52` |
Test command: `cargo test --workspace`

### 14.3 Design inputs
- `<n>` in the no answer refusal is the existing remote timeout setting used by `timeout_error` (`crates/core/src/git/remote.rs:1269`); no new setting; signal: E2E-032.
- A new action is a contract change: new tool, golden regeneration (`MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current`), count sites (114 to 115 total and the `git.*` family count, recount with `rg -n '114' crates` and the family constants), `TOOL_CONTRACT.txt`, `AGENTS.md` and `.agent_docs/tools.md` parity (`e2e_new_957`, `e2e_new_958`); signal: the golden and parity tests fail until updated.
- Deleting a ref on the remote needs a delete form of the push refspec through the existing push core (`git/remote.rs:609` `push_refs`, `:696` `push_tag_to_remote`); the lease on the advertised tag object (as the forced publication does) is what makes FR-NEW-003's "moved meanwhile" safe; signal: E2E-009, E2E-031.
- Reuse `list_advertised_refs` (`git/remote.rs:793`) for the remote tag state and the peeled commit (annotated tags: the `^{}` entry); reuse `stated_commit_matches` and `short7` (`git_tags.rs:799,812`); signal: E2E-002 to E2E-004, E2E-006, E2E-027.
- The action label and location under which the audit line is recorded are the designer's choice, following the labels of SPEC-0052 (one label per action); signal: E2E-025.
- `crates/agent/src/mcp.rs:317-319` asserts the full 114 tool catalogue and must become 115; signal: that test fails until updated.
- The audit must not use `run_remote_operation` (it audits failures); use `trace_remote_operation` and write the line after a real deletion, as publication does (SPEC-0052 DEC-016); signal: E2E-026, E2E-033.
- Local file transport runs no hooks, so a protecting remote (E2E-019, E2E-020) must be simulated through the push status callback, as E2E-085 of SPEC-0052 was; connection loss is simulated with a transport that drops connections (E2E-033); signal: these tests need a seam.
- Refusal order (FR-NEW-007) follows the order of the calls: authorize, remote lookup, credential and listing, then the tag state checks; signal: E2E-015, 018, 021, 024, 034.
- A distinct refusal function is needed for the remote's declining of a deletion (`remote_refused_tag`, `git/remote.rs:784`, words a name refusal); signal: E2E-019, E2E-020.
- Result shape: tag, remote, the deleted commit (full id) and the credential kind, like publishing; design decides the exact keys.

## Constraints
- No new tool and no contract change.
- Seams exist only under `#[cfg(test)]`; production behaviour must be identical without them.
- Do not change the publish or branch push paths.
- Files not to touch: anything under `sdlc/specs/`, other stories.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing git test stays green, in particular the tag publish and branch push tests; `tool_contract_golden_is_current` green after review of the regenerated diff.
- [ ] Docs updated: only the counts the parity tests force; the closing story documents the action.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0093: remote lookup, connection failures and declined deletions`, body `Spec: SPEC-0053` and the requirement ids.
