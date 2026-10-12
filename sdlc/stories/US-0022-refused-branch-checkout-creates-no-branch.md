# US-0022: Refused branch checkout creates no branch

> Spec: SPEC-0021 (sdlc/specs/SPEC-0021-refused-branch-checkout-leaves-ref/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 3
> Closes spec: yes

## Objective
`git.branch_create` with `checkout: true` writes the branch before the checkout is admitted, so a
refused checkout leaves the branch and a retry fails with `branch 'old' already exists`. After this
story a refused or failed checkout leaves the branch list unchanged and the retry succeeds.

## Requirements covered
- SPEC-0021/BR-001: IF `git.branch_create` with `checkout: true` is refused with `ERR_PROJECT_QUOTA_EXCEEDED` or `ERR_WRITE_QUOTA_EXCEEDED` THEN THE Git context SHALL leave the set of branches listed by `git.branches` identical to before the call.
- SPEC-0021/BR-002: IF `git.branch_create` with `checkout: true` fails after admission while applying the checkout THEN THE Git context SHALL leave the set of branches listed by `git.branches` identical to before the call.
- SPEC-0021/BR-003: WHEN `git.branch_create` with `checkout: true` is repeated with the same `name` after a refusal and the limit no longer blocks it THEN THE Git context SHALL create the branch, check it out and return `{branch, sha, checked_out: true}`.
- SPEC-0021/BR-004: The Git context SHALL NOT change HEAD, any file or the usage when it refuses or fails a `git.branch_create` checkout.

## Independent Test
`make check`; the seven `spec_0021_bt_*` tests in `crates/core/src/tools/quota_e2e.rs` pass, and
BT-001 reads the branch list through `git.branches`, not through `branch_create`'s own output.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0021): with `checkout: true`, call `merge::charge_and_apply` before `entry.db.set_ref` and `repo.reference`; write the branch, then HEAD, only on success. `checkout: false` keeps today's order.
- DEC-002 (SPEC-0021): BT-011 induces the apply failure by removing the target blob of `/k.txt` from the git object store (blob key `git:{sha}`) before the call, or any equivalent fault reachable from the harness without a production hook.
- DEC-003 (SPEC-0021): all seven tests in `quota_e2e.rs`, named `spec_0021_bt_0NN_*`, reusing the `Vc` harness.
### Root cause
`crates/core/src/tools/git.rs:2292` (`entry.db.set_ref`) and `:2295` (`repo.reference`) run before
`merge::charge_and_apply` at `:2300`; its error returns through `?` at `:2301` and nothing removes the ref.
### Contracts
Unchanged. `ERR_PROJECT_QUOTA_EXCEEDED` message `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested` in the harness; `ERR_WRITE_QUOTA_EXCEEDED` message `session write quota of 262144 bytes exceeded`; `ERR_NO_CLOBBER` `branch 'main' already exists`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify `branch_create` (`:2239-2306`) |
| `crates/core/src/tools/quota_e2e.rs` | add tests |
| `.agent_docs/git.md` | one timestamped line |
### Existing patterns
`spec_0020_us_0021_branch_create_checkout_over_the_limit_is_refused` (`quota_e2e.rs:2445-2459`):
`Vc::new`, `k_then_removed(false)`, `lower()`, `assert_refused`, `unchanged`, `head`. Session budget
exhaustion: `spec_0020_e2e_007_*` (`quota_e2e.rs:573`). Branch listing: `git.branches` returns
`branches[].full_ref` (`git.rs:1960`).

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0021/BT-001: Retry after a refused branch checkout succeeds
- Red before the fix because: `git.branches` lists `refs/heads/old` after the refusal.
- Given Atlas over its 1 MB limit at 1 500 000 bytes used, commit `with_k` adding `/k.txt` (10 bytes)
- When Mia calls `git.branch_create {mount_id: "Atlas", name: "old", start_point: with_k, checkout: true}`
- Then `ERR_PROJECT_QUOTA_EXCEEDED`, `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`
- And `git.branches` full refs are exactly `["refs/heads/main"]`
- When Ada removes Atlas's limit and Mia repeats the call
- Then `{"branch": "old", "sha": with_k, "checked_out": true}` and HEAD is `refs/heads/old`

### SPEC-0021/BT-010: Session budget refusal leaves no branch
- Red before the fix because: `refs/heads/old` listed.
- Given Atlas with no limit, Mia's 262 144 byte session budget spent, `with_k` as in BT-001
- When the BT-001 call
- Then `ERR_WRITE_QUOTA_EXCEEDED`, `session write quota of 262144 bytes exceeded`; branches exactly `["refs/heads/main"]`; HEAD `refs/heads/main`; `/k.txt` absent; used bytes unchanged

### SPEC-0021/BT-011: Apply failure leaves no branch
- Red before the fix because: `refs/heads/old` listed.
- Given Atlas with no limit, `with_k`, the fault of DEC-002 in place
- When the BT-001 call
- Then an error is returned; branches exactly `["refs/heads/main"]`; HEAD `refs/heads/main`

### SPEC-0021/BT-012: Project limit refusal keeps files, usage and HEAD
- Given the BT-001 refusal / Then `/k.txt` absent, `/c.txt` unchanged, used 1 500 000, HEAD `refs/heads/main`

### SPEC-0021/BT-020: Creation without checkout over the limit still succeeds
- Given the BT-001 state / When `checkout: false` / Then `{"branch": "old", "sha": with_k, "checked_out": false}`, HEAD `refs/heads/main`

### SPEC-0021/BT-021: Dirty volume refusal still writes nothing
- Given an uncommitted file in the volume / When `git.branch_create {name: "old", checkout: true}` / Then refused with today's dirty volume message; branches unchanged

### SPEC-0021/BT-022: Duplicate name still refused without moving the ref
- Given `main` at `c2` / When `git.branch_create {name: "main", start_point: c1}` / Then `ERR_NO_CLOBBER`, `branch 'main' already exists`, `main` stays at `c2`

## Constraints
- Files not to touch: `crates/core/src/git/merge.rs`, `TOOL_CONTRACT.txt`, `tool-contract-golden.json`.
- Dependencies not to add: any.
- Patterns to avoid: a production fault injection hook; a compensating delete instead of reordering.
- Scope boundary: other git tools are out of scope.

## Definition of done
- [ ] RED evidence for BT-001, BT-010, BT-011 before the production change.
- [ ] Every acceptance test green; `make check` green.
- [ ] Non regression: `spec_0020_us_0021_branch_create_checkout_over_the_limit_is_refused`, `e2e_new_400`, `e2e_new_402`, `e2e_new_403` unchanged and green.
- [ ] Docs updated: `.agent_docs/git.md`.
- [ ] Self review: full.
- [ ] Closes spec: every BT- of SPEC-0021 green; BR-001..BR-004 cited.
- [ ] One commit `US-0022: Refused branch checkout creates no branch`, body `Spec: SPEC-0021`.
