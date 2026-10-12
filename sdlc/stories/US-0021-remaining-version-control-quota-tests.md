# US-0021: Remaining version control operations proven under the limit

> Spec: SPEC-0020 (sdlc/specs/SPEC-0020-project-quota-enforcement/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0020
> min_tier: 1
> Files touched: 2
> Closes spec: yes

## Objective
The SPEC-0020 converge audit found that 11 version control operations listed by FR-NEW-003 and 3
aborts listed by FR-NEW-012 have no acceptance evidence, although they go through the admitted
`charge_and_apply` path. This story proves each one against the limit, and fixes any that is not
admitted. Inserted by `/sdlc-implement` (converge gap); it now closes the spec.

## Requirements covered
- SPEC-0020/FR-NEW-003 (partial: rebase, rebase continue, revert, revert continue, cherry pick continue, merge conflict resolution, reset, branch creation, branch reset, stash save, stash apply): The Storage context SHALL check the limit on writes, appends, uploads, every kind of edit, copies, Word document creation, a conversion's Markdown file, archive extraction, and every version control operation that changes project files: clone, pull, merge, merge conflict resolution, cherry pick and its continuation, rebase and its continuation, revert and its continuation, reset, branch creation, branch switch, branch reset, checkout of a version or of a file, stash save, stash apply, stash pop, and a pull request merge into the project.
- SPEC-0020/FR-NEW-012 (partial: rebase abort, cherry pick abort, revert abort): The Storage context SHALL accept moves, deletes to the trash, trash restores and version control pushes whatever the limit and the usage.

## Evidence
Converge audit of US-0020 (2026-10-11): callers of `merge::charge_and_apply` at
`crates/core/src/tools/git.rs:2300, 2600, 2752, 4635, 5361, 6907, 7112` have no over limit test;
`replay_abort` at `crates/core/src/tools/git.rs:5746` has no over limit test.

## Independent Test
`make check`; then with Atlas lowered to 1 MB at 1 500 000 bytes used, a rebase whose replayed commit
adds 10 bytes is refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`
and the branch files are byte identical to before, read through the volume.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0020): per volume `volume_usage` counter, checked under a per volume admission lock held across the operation's writes.
- DEC-003 (SPEC-0020): net change computed by the caller (new minus previous size per written path, minus removed paths).
- DEC-009 (SPEC-0020): aborts, moves, trash, restores, pushes never call `admit`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/quota_e2e.rs` | extend: one test per operation below |
| `crates/core/src/tools/git.rs` or `crates/core/src/git/merge.rs` | modify only if an operation is found unadmitted |
### Existing patterns
- `spec_0020_e2e_065_a_cherry_pick_over_the_limit_writes_nothing` and `spec_0020_e2e_068_a_merge_abort_is_accepted_over_the_limit` in `crates/core/src/tools/quota_e2e.rs` (the `Vc` harness).

## Acceptance tests
> 100% must pass, run through `make check`.

Shared setup: Atlas lowered to a 1 MB limit while using 1 500 000 bytes (as SPEC-0020/E2E-044), Mia a
member. Each growing operation adds a 10 byte file `/k.txt` (or grows a tracked file by 10 bytes) and
must be refused with `project storage quota of 1 MB exceeded: 1500000 bytes used, 10 bytes requested`,
leaving the files and usage unchanged. Tests named `spec_0020_us_0021_<operation>_*`:
- rebase; rebase continue (after a conflict resolved by Mia); revert; revert continue; cherry pick
  continue; merge conflict resolution; reset to a commit holding `/k.txt`; branch creation with
  checkout from a commit holding `/k.txt`; branch reset; stash save of a working tree adding `/k.txt`
  is accepted (it removes project bytes or keeps them) and its later stash apply is refused.
- rebase abort, cherry pick abort and revert abort over the limit are accepted, and the files return to
  their state before the operation, usage unchanged.

## Constraints
- Files not to touch: `crates/core/src/storage/rel/*`; existing acceptance tests.
- Dependencies not to add: any crate.
- Scope boundary: no new behavior; only proof and, if needed, admission of an unadmitted path.

## Definition of done
- [ ] RED evidence for any operation found unadmitted; otherwise a lock release mutation shows the tests can fail.
- [ ] Every test above green; `make check` green.
- [ ] Closes spec: every E2E-001 to E2E-069 green, every requirement of SPEC-0020 cited by a done story; converge audit 15 of 15.
- [ ] One commit `US-0021: Remaining version control operations proven under the limit`, body `Spec: SPEC-0020`.
