# Ref name validation accepts names git refuses: Bug Specification

> Generated on: 2026-10-28
> Id: SPEC-0056
> Nature: BUG
> Depth: M
> Depth evidence: 2 modules (tools/git.rs, tools/git_tags.rs), 10 requirements, the refusal texts of 3 tools change, no data model change
> Status: designed
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: Medium
> From backlog: BL-0077
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Defect

### BUG-001: branch and tag names that git refuses are accepted, then fail late
- **Observed behavior:** `git.branch_create` and `git.tag_create` accept names that `git check-ref-format` and libgit2 refuse. The creation succeeds, the database holds the ref, and the failure appears later, as an internal error, when the branch is pushed. Reproduced through the registered tools on a `file://` remote:

  | Name | Create | Push of the branch |
  |---|---|---|
  | `a//b` | accepted (branch and tag) | `ERR_INTERNAL_ERROR` `push of branch 'a//b' failed: invalid refspec refs/heads/a//b:refs/heads/a//b; class=Invalid (3)` |
  | `v1.`, `a.` | accepted | `ERR_INTERNAL_ERROR` `sync local branch ref: the given reference name 'refs/heads/v1.' is not valid; class=Reference (4); code=InvalidSpec (-12)` |
  | `x@{y}` | accepted | same internal error |
  | `a.lock/b` | accepted | same internal error |
  | `ok/name`, `@` | accepted | pushed |

  Delete, switch and reset of such a branch succeed (they call the same validator, which accepts the name).
- **Expected per authority:** SPEC-0012 FR-NEW-104 / FR-NEW-116 (`sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:103`): "branch-name validation mirrors `git check-ref-format`". For tags, SPEC-0052 section 3.3 (`sdlc/specs/SPEC-0052-git-tags/sdlc-spec.md:59`): "Tag names follow the same rules as branch names. Imposed by the user."
- **Reproduction:** project with one commit; create the branch `a//b`; push it. Expected: the creation is refused with a clear message. Actual: the creation succeeds and the push fails with an internal error.
- **Root cause:** `validate_ref_name` (`crates/core/src/tools/git.rs:2357-2392`) checks seven rules (empty, 255 bytes, forbidden characters, `..`, leading or trailing `/`, a component beginning with `.`, a whole name ending in `.lock`) and has none for four classes `git check-ref-format` refuses: a `//` anywhere, a name ending with `.`, the sequence `@{`, and a component ending with `.lock` that is not the whole name. A differential run of the validator against libgit2's `Reference::is_valid_name` on 66 436 names (alphabet `a . / @ { } - l :`, up to 5 characters, plus 7 hand picked) found 4 536 names the validator accepts and libgit2 refuses, and none the other way: 576 with `//`, 2 526 ending with `.`, 1 433 with `@{`, 1 with a non final `.lock` component, and no other class. Amended 2026-10-28: the first version of this analysis counted any `/` separated component ending with `.` (`a./b`); git accepts `a./b` and refuses only a name that ends with `.`, which the implementation of US-0105 found (`git check-ref-format refs/heads/a./b` is accepted).
- **Contributing factors:** `branch_create` (`git.rs:2465`) and `tag_create` (`git_tags.rs:157`) write the on disk ref with `let _ = repo.reference(...)`, so libgit2's refusal is dropped silently; the failure surfaces at the first push (`git.rs:3647-3653`, mapped to `sync local branch ref`) or as an `invalid refspec` from libgit2's refspec parser.
- **Impact:** a member creates a branch or tag that can never be pushed and gets an internal error with libgit2 wording, at a distance from the cause. No data loss. Workaround: delete the branch or tag and create it under another name.
- **Rollback posture:** names accepted today and valid for git stay accepted. Branches and tags that already exist with a refused name must stay switchable, resettable and deletable, so nobody is locked out of cleaning them.

## 2. Expected Behavior

#### BR-001 [EARS-O]: refuse a branch name containing `//`
> IF a member creates a branch whose name contains `//`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: it contains '//'` and create nothing.

- **Source:** restored from SPEC-0012/FR-NEW-104.
- **Inputs:** `git.branch_create` with `name` `a//b`.
- **Outputs:** `ERR_INVALID_ARGUMENT`, the message above with `<name>` as given, no ref row, no audit entry, no quota charge (`git.rs:2346-2348` states a refused name never leaves a partial row).
- **Exact names:** code `ERR_INVALID_ARGUMENT`.

#### BR-002 [EARS-O]: refuse a name ending in `.`
> IF a member creates a branch whose name ends with `.`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: it ends with '.'` and create nothing.

- **Source:** restored from SPEC-0012/FR-NEW-104.
- **Business Rules:** `v1.`, `a.` and `a/b.` are refused; only the whole name matters, so `a./b` stays accepted (git accepts it, BR-009).

#### BR-003 [EARS-O]: refuse a name containing `@{`
> IF a member creates a branch whose name contains `@{`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: it contains '@{'` and create nothing.

- **Source:** restored from SPEC-0012/FR-NEW-104.

#### BR-004 [EARS-O]: refuse a non final component ending in `.lock`
> IF a member creates a branch whose name has a `/` separated component ending with `.lock` and the whole name does not end with `.lock`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: one of its '/' separated components ends with '.lock'` and create nothing.

- **Source:** restored from SPEC-0012/FR-NEW-104. The whole name ending in `.lock` keeps today's text `it ends with '.lock'` (`git.rs:2388-2390`).

#### BR-005 [EARS-U]: tags follow the same rules, in the same order
> The Versioning context SHALL apply BR-001 to BR-004 to tag names with the word `tag` in place of `branch` (`'<name>' is not a valid tag name: <reason>`), and SHALL check the reasons in this order, the first that applies being the one given: the seven existing reasons in their existing order, then `it contains '//'`, then `it ends with '.'`, then `it contains '@{'`, then `one of its '/' separated components ends with '.lock'`.

- **Source:** restored from SPEC-0052 section 3.3 (tag names follow the branch rules) and decided here for the order and wording (user answer 2026-10-28: "go" on the defaults).
- **Business Rules:** `a@{b.` gives the `'.'` reason; `a.lock/b//c` gives the `'//'` reason.

#### BR-006 [EARS-O]: refuse pushing a branch whose name git refuses, before the network
> IF a member pushes a branch whose local name, or whose `remote_branch` name, git refuses, THEN THE Remote exchange context SHALL refuse with `'<name>' is not a valid branch name: <reason>` before any remote is resolved or contacted.

- **Source:** restored from SPEC-0012/FR-NEW-104 and the precedent of E2E-NEW-891 (`git.rs:23740`): the remote side name is already checked before the remote. Decided here for the local name (it fails today as `ERR_INTERNAL_ERROR`).
- **Outputs:** `ERR_INVALID_ARGUMENT`; the remote is untouched; no audit entry as a remote operation.
- **Exact names:** tool `git.remote_push`, parameters `branch`, `remote_branch`.

#### BR-007 [EARS-UB]: existing branches stay manageable
> The Versioning context SHALL NOT refuse to switch to, reset or delete an existing branch because its name is one that BR-001 to BR-004 refuse for creation.

- **Source:** decided here (user answer 2026-10-28: "go" on the default). The other refusals of these tools (unknown branch, dirty volume, checked out branch) stay as they are.

#### BR-008 [EARS-UB]: existing tags stay manageable and keep their publish refusal
> The Versioning context SHALL NOT refuse to delete an existing tag because of its name, and publishing such a tag SHALL keep giving `Remote refused tag '<name>': invalid ref name`.

- **Source:** decided here, same answer. `git.tag_delete` does no name check today (`git_tags.rs:65-82`); the publish refusal is SPEC-0052 DEC-019 (`git_tags.rs:700-705`).

#### BR-009 [EARS-UB]: names accepted today and valid for git stay accepted
> The Versioning context SHALL NOT refuse a branch or tag name that `git check-ref-format` accepts.

- **Source:** restored (the differential run found no name refused by the validator that libgit2 accepts; this keeps it so).
- **Business Rules:** `@`, `ok/name`, `release/2026-q1`, `v1.0`, an accented name, a 255 byte name stay accepted; 256 bytes stays refused (E2E-NEW-406/407).

#### BR-010 [EARS-UB]: the seven existing reasons are unchanged
> The Versioning context SHALL NOT change the refusal text, the code or the order of the seven existing reasons.

- **Source:** restored (E2E-NEW-404, `git.rs:16432`; E2E-TAG-010, `git.rs:24194`).

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| `a//b`, `a///b` | refused, `'//'` reason | BR-001 |
| `v1.`, `a.`, `a/b.` | refused, `'.'` reason | BR-002 |
| `a./b` | accepted (git accepts it) | BR-009 |
| `x@{y}`, `@{` | refused, `'@{'` reason | BR-003 |
| `a.lock/b` | refused, component `.lock` reason | BR-004 |
| `a/b.lock`, `tip.lock` | refused, `it ends with '.lock'` (unchanged) | BR-004, BR-010 |
| `a//b.` | `'//'` reason (first that applies) | BR-005 |
| `a@{b.` | `'.'` reason | BR-005 |
| `@`, `ok/name`, `é` | accepted | BR-009 |
| legacy branch `a//b` already stored | switch, reset, delete accepted; push refused before the network | BR-007, BR-006 |
| legacy tag `a//b` already stored | delete accepted; publish refused with the remote style text | BR-008 |
| push of a valid branch with `remote_branch` `a//b` | refused before the remote | BR-006 |
| creation refused | no ref, no audit entry, no quota charge | BR-001 |

## 4. Tests

### 4.1 End to end reproduction (mandatory)
#### BT-001: a branch named `a//b` is refused at creation
- **Reproduces:** BUG-001
- **Driver:** the registered MCP tools through the e2e harness (`e.call("git.branch_create", ...)`, `e.call("git.branches", ...)`), against a project with one commit.
- **Fails today because:** `validate_ref_name` has no `//` rule (`git.rs:2357-2392`), so the call succeeds.
- Given a member Alice of project `gitproj` with one commit on `main`. When Alice calls `git.branch_create` with name `a//b`. Then the call fails with `ERR_INVALID_ARGUMENT` and the message `'a//b' is not a valid branch name: it contains '//'`, And `git.branches` lists only `main`, And no audit entry and no quota charge were added.

#### BT-002: a branch named `v1.` is refused at creation
- **Reproduces:** BUG-001
- **Driver:** as BT-001.
- **Fails today because:** no rule for a name ending in `.`.
- Given the project of BT-001. When Alice creates the branch `v1.`. Then `ERR_INVALID_ARGUMENT` `'v1.' is not a valid branch name: it ends with '.'`, And `git.branches` lists only `main`.

### 4.2 New behavior
#### BT-010: the four classes are refused for branches
- **Validates:** BR-001, BR-002, BR-003, BR-004
- **Driver:** as BT-001.
- Given the project. When Alice creates, one at a time, `a//b`, `a/b.`, `x@{y}`, `a.lock/b`. Then each is refused with `ERR_INVALID_ARGUMENT` and respectively `it contains '//'`, `it ends with '.'`, `it contains '@{'`, `one of its '/' separated components ends with '.lock'` (full message `'<name>' is not a valid branch name: <reason>`), And `git.branches` lists only `main` after the four.

#### BT-011: the four classes are refused for tags
- **Validates:** BR-005
- **Driver:** `e.call("git.tag_create", ...)`, `e.call("git.tags", ...)`.
- Given the project. When Alice creates the tags `a//b`, `v1.`, `x@{y}`, `a.lock/b` one at a time. Then each is refused with `ERR_INVALID_ARGUMENT` and `'<name>' is not a valid tag name: <reason>` with the same four reasons, And `git.tags` lists no tag.

#### BT-012: the first reason that applies is given
- **Validates:** BR-005
- **Driver:** as BT-011, and `git.branch_create`.
- Given the project. When Alice creates the tag `a@{b.` and the branch `a.lock/b//c`. Then the tag is refused with `'a@{b.' is not a valid tag name: it ends with '.'`, And the branch is refused with `'a.lock/b//c' is not a valid branch name: it contains '//'`.

#### BT-013: a legacy branch is refused at push before the network
- **Validates:** BR-006
- **Driver:** the registered tool `e.call("git.remote_push", ...)` as E2E-NEW-891 (`git.rs:23740`): the refusal happens before the remote is resolved or contacted; the legacy branch is written straight into the project's ref table by the test (the state of a project created before this fix).
- Given the project with a legacy branch row `a//b` pointing at the head commit and a bare remote seeded. When Alice pushes `a//b`. Then `ERR_INVALID_ARGUMENT` `'a//b' is not a valid branch name: it contains '//'`, And the remote refs are unchanged, And no audit entry has the operation `git.remote_push`.

#### BT-014: an invalid remote branch name is refused for the new classes
- **Validates:** BR-006
- **Driver:** as E2E-NEW-891.
- Given the project with a valid branch `main` and a bare remote. When Alice pushes `main` with `remote_branch` `a//b`, then `v1.`, then `x@{y}`. Then each is refused with `ERR_INVALID_ARGUMENT` and `'<name>' is not a valid branch name: <reason>`, And the remote refs are unchanged.

#### BT-015: legacy branches stay manageable
- **Validates:** BR-007
- **Driver:** `e.call("git.branch_switch" | "git.branch_reset" | "git.branch_delete", ...)`.
- Given the project with the legacy branch row `a//b` at the head commit. When Alice switches to `a//b`, back to `main`, resets `a//b` to the head commit, and deletes it. Then each call succeeds (switch returns `branch` `a//b`; delete returns `branch` `a//b`), And `git.branches` then lists only `main`.

#### BT-016: legacy tags stay deletable and keep their publish refusal
- **Validates:** BR-008
- **Driver:** `git.tag_delete` through the registered tool; the publish helper of the SPEC-0052 tests for the `file://` remote.
- Given the project with the legacy tag row `a//b` at the head commit and a bare remote. When Alice publishes `a//b`. Then `ERR_INTERNAL_ERROR` `Remote refused tag 'a//b': invalid ref name`, And the remote refs are unchanged, And when Alice then calls `git.tag_delete` on `a//b` it succeeds and `git.tags` lists no tag.

### 4.3 Non regression
#### BT-020: valid names stay accepted
- **Protects:** BR-009
- Given the project. When Alice creates the branches `@`, `ok/name`, `release/2026-q1`, `é`, a 255 byte name, and the tags `v1.0`, `@`, `release/2026-q1`, `é`. Then all are accepted, And a 256 byte branch name is refused with `it is 256 bytes of UTF-8, over the 255 byte limit` (unchanged).

#### BT-021: the seven existing reasons are unchanged
- **Protects:** BR-010
- Given the project. When Alice creates the branches `bad..name`, `has space`, `/leading`, `trailing/`, `tip.lock`, `a/.hidden`, and the empty name. Then each is refused with the same code and text as before this change (`it contains '..'`, `it contains the forbidden character ' '`, `it begins or ends with '/'`, `it begins or ends with '/'`, `it ends with '.lock'`, `one of its '/' separated components begins with '.'`, `it is empty`), And `a/b.lock` gives `it ends with '.lock'`.

#### BT-022: a refused creation leaves nothing behind
- **Protects:** BR-001
- Given the project with one audit entry for the first commit. When Alice's creation of `a//b` is refused. Then the audit trail, the quota usage and the list of refs are identical to before the call.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/tools/git.rs` | `e2e_new_404_an_invalid_branch_name_is_refused_before_any_write` (16432) | Kept | uses only classic invalid names (BR-010) |
| `crates/core/src/tools/git.rs` | `e2e_new_891_an_invalid_remote_branch_name_is_refused_before_the_remote` (23740) | Kept | classic names, same behavior |
| `crates/core/src/tools/git.rs` | `e2e_tag_010_other_invalid_names` (24194) | Kept | classic names |
| `crates/core/src/tools/git.rs` | `e2e_new_405`, `406`, `407` (16461-16501) | Kept | accepted and 255 or 256 byte names (BR-009) |
| `crates/core/src/tools/git.rs` | `e2e_tag_085_remote_refuses_the_tag_name` (26040) | Modified | the setup creates the tag `a//b` through `git.tag_create`, which is now refused; the setup writes the legacy tag row directly; the assertions (publish refusal text, remote unchanged, audit empty) are unchanged (BR-008) |
| `crates/core/src/tools/git.rs` | `e2e_tag_057_publish_all_an_invalid_ref_name` (28680, SPEC-0054/E2E-057) | Modified | same setup change; the preview and report lines are unchanged |
| `crates/core/src/tools/git_tags.rs` | `a_remote_name_refusal_names_the_tag_and_the_reason` (1521) | Kept | calls the helper directly |

## 5. Existing Specs Affected
- SPEC-0052/FR-NEW-004 lists seven rules and E2E-085 (`sdlc-spec.md:791`) creates `a//b` as valid. This specification deviates: `a//b` is refused at creation (BR-005); the publish side refusal for an already stored tag stays (BR-008). SPEC-0052 is not modified.
- SPEC-0054/E2E-057 (`sdlc-spec.md:587`) uses `a//b` the same way and deviates the same way.
- SPEC-0012/FR-NEW-104 is restored, not changed.

## 6. Assumptions
- **Assumption:** the refusal wording for the four new reasons and their order are decided by the user's "go" on the proposed defaults (2026-10-28), shaping BR-001 to BR-005.
- **Assumption:** a project created before this fix may hold branches and tags with refused names; they are left alone (BR-007, BR-008). No migration, no scan.
- **Assumption:** `git.remote_push_tag` and `git.remote_push_tags` need no change: they already check `Reference::is_valid_name` before the push (`git_tags.rs:700-705`, `1167-1172`).
- **Backlog entries created by this run:** none.
- **Overshoot:** none.

## 7. Implementability Checklist
| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS (the root cause cites code locations only; no fix design, no file list, no DEC) |
| 2 | No buried requirement | PASS (BR-001..BR-004 each carry one class; BR-005..BR-010 their own) |
| 3 | Every observable name is spelled | PASS (tool names, parameters, code `ERR_INVALID_ARGUMENT`, `ERR_INTERNAL_ERROR`, the four message texts, casing as in the codebase) |
| 4 | No forced choice | PASS (order of reasons in BR-005; legacy behavior in BR-007, BR-008; leftmost rule order stated) |
| 5 | Every test is specified | PASS (BT-001..BT-022 have Given/When/Then; 4.4 lists the 7 touched tests) |
| 6 | Reproduction test is end to end | PASS (BT-001 and BT-002 drive the registered tools and assert what the member sees) |
| 7ante | Security misclassification | PASS (no authn, injection, exposure or traversal flaw) |
| 7bis | Class sweep | n/a (not a security entry) |
| 8 | No hidden prerequisite | PASS (legacy state is set up by the test and named in each Given) |
| 9 | EARS clean | PASS (no forbidden modal, no nesting) |
| 10 | Right tool | PASS (behavior changes, no new capability; depth M, 10 requirements) |
| 11 | Length | PASS |
| 12 | Every claim cited | PASS (`git.rs:2357-2392`, `2465`, `3647-3653`, `16432`, `23740`, `24194`, `26040`, `28680`, `git_tags.rs:157`, `700-705`, `1167-1172`, `65-82`; counts from the differential run) |
| 13 | Round trip read | PASS (writer: `validate_ref_name` and the two `repo.reference` calls; reader: libgit2 `is_valid_name`, the push sync and `Reference` refspec parse, both observed) |
| 14 | Verdict consistent | PASS (authority exists: every BR Source cites it; decided items are the legacy and order questions the authority did not cover) |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
