# Refused branch checkout leaves the new branch behind: Bug Specification

> Generated on: 2026-10-12
> Id: SPEC-0021
> Nature: BUG
> Depth: S
> Depth evidence: 1 module (git tools), 1 defective behavior, 4 requirements, no public contract change, no data model change
> Status: implemented
> Verdict: no authority
> Defect class: absent
> Severity: Medium
> From backlog: BL-0044
> Security: n/a
> CVSS: n/a
> Affected: main since SPEC-0012 (session budget refusal) and SPEC-0020 (project limit refusal)
> Fixed in: unreleased

## 1. Defect

### BUG-001: `git.branch_create` with `checkout: true` keeps the branch when the checkout is refused
- **Observed behavior:** the branch row and the on disk ref are written
  (`crates/core/src/tools/git.rs:2292`, `:2295`) before the checkout is admitted and applied
  (`crates/core/src/tools/git.rs:2300`). When `merge::charge_and_apply` returns an error
  (`crates/core/src/git/merge.rs:684-692`), the `?` at `git.rs:2301` returns it and nothing
  removes the ref. HEAD, files and usage are unchanged (asserted by
  `spec_0020_us_0021_branch_create_checkout_over_the_limit_is_refused`,
  `crates/core/src/tools/quota_e2e.rs:2445-2459`), but the branch exists.
- **Expected per authority:** none: no authority. SPEC-0020/FR-NEW-013 forbids changing "any file
  or the usage" on a limit refusal (`sdlc/specs/SPEC-0020-project-quota-enforcement/sdlc-spec.md:231-233`);
  a branch is neither. SPEC-0012/FR-NEW-101 (`sdlc/specs/SPEC-0012-full-git-dev-process/spec.md:100`)
  states creation but not its outcome on a refused checkout; FR-NEW-105 (`:104`) makes
  `git.branch_switch` atomic but says nothing of `git.branch_create`.
- **Reproduction:** project `Atlas` over its 1 MB limit (1 500 000 bytes used), commit `with_k`
  adding 10 bytes. Mia calls `git.branch_create {name: "old", start_point: with_k, checkout: true}`:
  refused with `ERR_PROJECT_QUOTA_EXCEEDED`. Ada raises the limit. Mia repeats the same call.
  Expected: the second call succeeds. Actual: `ERR_NO_CLOBBER`, `branch 'old' already exists`
  (`git.rs:2255-2257`), and `git.branches` lists `refs/heads/old` after the first, refused call.
- **Root cause:** `crates/core/src/tools/git.rs:2292` writes `refs/heads/{name}` before the fallible
  `charge_and_apply` at `:2300`, and the error path out of `:2301` leaves it. The sibling
  `stash_save` removes its ref when the same call fails (`git.rs:6907-6909`); `branch_create` does not.
- **Contributing factors:** the comment at `git.rs:2284-2285` covers only the dirty volume refusal,
  which happens before the write. The SPEC-0020 test checks files, usage and HEAD, not the ref list.
- **Impact:** every member creating and checking out a branch in a project at its limit, or past
  their session write budget (`ERR_WRITE_QUOTA_EXCEEDED`, same path through
  `crate::quota::admit_paths`, `merge.rs:685`). Workaround: `git.branch_delete` then retry.
- **Rollback posture:** reverting the fix restores today's behavior; no stored data changes shape.

## 2. Expected Behavior

#### BR-001 [EARS-O]: A refused checkout creates no branch
> IF `git.branch_create` with `checkout: true` is refused with `ERR_PROJECT_QUOTA_EXCEEDED` or
> `ERR_WRITE_QUOTA_EXCEEDED` THEN THE Git context SHALL leave the set of branches listed by
> `git.branches` identical to before the call.
- **Source:** decided here: ASSUMED, by analogy with SPEC-0012/FR-NEW-105 (switch is atomic) and
  FR-NEW-185 (quota exhaustion changes nothing), and with `stash_save` (`git.rs:6907-6909`).
- **Outputs:** the refusal error unchanged: code and message as today.
- **Exact names:** `git.branch_create`, `git.branches`, `ERR_PROJECT_QUOTA_EXCEEDED`,
  `ERR_WRITE_QUOTA_EXCEEDED`.

#### BR-002 [EARS-O]: A failed checkout apply creates no branch
> IF `git.branch_create` with `checkout: true` fails after admission while applying the checkout
> THEN THE Git context SHALL leave the set of branches listed by `git.branches` identical to before
> the call.
- **Source:** decided here: ASSUMED, same reasoning as BR-001; the error returned is unchanged.

#### BR-003 [EARS-E]: The same call succeeds once the cause is gone
> WHEN `git.branch_create` with `checkout: true` is repeated with the same `name` after a refusal and
> the limit no longer blocks it THEN THE Git context SHALL create the branch, check it out and return
> `{branch, sha, checked_out: true}`.
- **Source:** decided here: follows from BR-001; the reported symptom of BL-0044.

#### BR-004 [EARS-UB]: The refusal keeps its other guarantees
> The Git context SHALL NOT change HEAD, any file or the usage when it refuses or fails a
> `git.branch_create` checkout.
- **Source:** restored from SPEC-0020/FR-NEW-013 and SPEC-0012/FR-NEW-101 (HEAD moves only on a
  completed checkout).

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| `checkout: false` over the limit | branch created, no admission (no file changes), as today | BT-020 |
| refusal for the session budget, project limit fine | no branch left, `ERR_WRITE_QUOTA_EXCEEDED` | BR-001 |
| dirty volume refusal | already writes nothing (`git.rs:2284-2290`), unchanged | BT-021 |
| a branch of that name existed before | refused by `ERR_NO_CLOBBER` before any write, existing ref untouched | BT-022 |

## 4. Tests

### 4.1 End to end reproduction (mandatory)
#### BT-001: Retry after a refused branch checkout succeeds
- **Reproduces:** BUG-001
- **Driver:** the `git.branch_create` and `git.branches` tool calls as an MCP client issues them
  (`vc::tool_branch_create`, the harness of `crates/core/src/tools/quota_e2e.rs`).
- **Fails today because:** the ref written at `git.rs:2292` survives the refusal at `:2301`.
- Given Atlas over its 1 MB limit at 1 500 000 bytes used, and a commit `with_k` adding `/k.txt` (10 bytes)
- When Mia calls `git.branch_create {mount_id: "Atlas", name: "old", start_point: with_k, checkout: true}`
- Then the call fails with `ERR_PROJECT_QUOTA_EXCEEDED`, message
  `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`
- And `git.branches` lists exactly `refs/heads/main`
- When Ada removes Atlas's limit and Mia repeats the same call
- Then it returns `{"branch": "old", "sha": with_k, "checked_out": true}` and HEAD is `refs/heads/old`

### 4.2 New behavior
#### BT-010: Session budget refusal leaves no branch
- **Validates:** BR-001, BR-004
- Given Atlas with no limit and Mia's session budget of 262 144 bytes already spent, `with_k` as in BT-001
- When Mia calls the BT-001 `git.branch_create`
- Then it fails with `ERR_WRITE_QUOTA_EXCEEDED`, message `session write quota of 262144 bytes exceeded`
- And `git.branches` lists exactly `refs/heads/main`, HEAD is `refs/heads/main`, `/k.txt` absent, usage unchanged

#### BT-011: Apply failure leaves no branch
- **Validates:** BR-002, BR-004
- Given Atlas with no limit, `with_k` as in BT-001, and the volume write of `/k.txt` made to fail
- When Mia calls the BT-001 `git.branch_create`
- Then it returns the apply error, and `git.branches` lists exactly `refs/heads/main`, HEAD is `refs/heads/main`

#### BT-012: Project limit refusal keeps files, usage and HEAD
- **Validates:** BR-001, BR-004
- Given the BT-001 refusal
- Then `/k.txt` is absent, `/c.txt` unchanged, used bytes 1 500 000, HEAD `refs/heads/main`

### 4.3 Non regression
#### BT-020: Creation without checkout over the limit still succeeds
- **Protects:** SPEC-0012/FR-NEW-101 and SPEC-0020 (no file change, no check)
- Given the BT-001 state, When Mia calls `git.branch_create {name: "old", start_point: with_k, checkout: false}`
- Then it returns `{"branch": "old", "sha": with_k, "checked_out": false}` and HEAD stays `refs/heads/main`

#### BT-021: Dirty volume refusal still writes nothing
- **Protects:** SPEC-0012/FR-NEW-106
- Given an uncommitted change in the volume, When `git.branch_create {checkout: true}` is called
- Then it is refused with today's message and `git.branches` lists only the previous branches

#### BT-022: Duplicate name still refused without moving the ref
- **Protects:** SPEC-0012/FR-NEW-102
- Given `main` at `c2`, When `git.branch_create {name: "main", start_point: c1}` is called
- Then `ERR_NO_CLOBBER`, `branch 'main' already exists`, `main` stays at `c2`

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/tools/quota_e2e.rs:2445` | `spec_0020_us_0021_branch_create_checkout_over_the_limit_is_refused` | Kept | asserts files, usage, HEAD; still true |
| `crates/core/src/tools/git.rs` (test module) | `e2e_new_400_create_a_branch_at_an_explicit_start_point` | Kept | no refusal involved |
| `crates/core/src/tools/git.rs` (test module) | `e2e_new_402_an_existing_branch_name_is_refused` | Kept | covered by BT-022 |
| `crates/core/src/tools/git.rs` (test module) | `e2e_new_403_an_unresolvable_start_point` | Kept | refused before any write |

No existing test asserts that the branch survives a refusal (`rg -n "branch_create" crates/core/src/tools/quota_e2e.rs`: 2 hits, `:1424` setup and `:2456`).

## 5. Existing Specs Affected
- SPEC-0012/FR-NEW-101: adds the outcome of a refused checkout; no deviation from its text.
- SPEC-0020/FR-NEW-013: extends "changes nothing" to the branch list for this tool; no deviation.

## 6. Assumptions
- **Assumption:** no question asked; BR-001 to BR-003 follow the atomicity the repo already applies
  to `git.branch_switch` and `git.stash_save`. A user preferring "keep the branch, report it" would
  overturn BR-001.
- **Assumption:** the on disk ref written at `git.rs:2295` is part of "the branch" and follows BR-001
  (read side: `git.branches` uses `entry.db.list_refs`, `git.rs:1961`).
- **Backlog entries created by this run:** none.

## 7. Implementability Checklist

| # | Item | Result |
|---|---|---|
| 1 | No technical decision | PASS (root cause `file:LINE` only) |
| 2 | No buried requirement | PASS |
| 3 | Observable names spelled | PASS (tool names, error codes, messages, response fields) |
| 4 | No forced choice | PASS (error returned unchanged, branch list identical) |
| 5 | Every test specified | PASS |
| 6 | Reproduction end to end | PASS (BT-001, user visible retry failure) |
| 7ante | Security misclassification | PASS (no bypass, injection or exposure) |
| 8 | No hidden prerequisite | PASS (limit change by Ada in Given) |
| 9 | EARS clean | PASS |
| 10 | Right tool | PASS (behavior changes, no new capability, 4 requirements at S) |
| 11 | Length | PASS |
| 12 | Claims cited | PASS |
| 13 | Writer and reader read | PASS (writer `git.rs:2292`, reader `git.rs:1961`) |
| 14 | Verdict consistent | PASS (no authority, class absent) |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
