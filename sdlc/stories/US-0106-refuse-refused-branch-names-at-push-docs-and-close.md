# US-0106: refuse a refused branch name at push, documentation and spec close

> Spec: SPEC-0056 (sdlc/specs/SPEC-0056-ref-name-validation/)
> Nature: BUG
> Priority: P2
> Depends on: US-0105
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
Make `git.remote_push` refuse, before any remote is resolved or contacted, a local branch name or a `remote_branch` name that git refuses, with the strict validator's message instead of the internal error. Then document the naming rules and close the spec: full acceptance set and the requirement coverage audit.

## Requirements covered
- SPEC-0056/BR-006: IF a member pushes a branch whose local name, or whose `remote_branch` name, git refuses, THEN THE Remote exchange context SHALL refuse with `'<name>' is not a valid branch name: <reason>` before any remote is resolved or contacted.

## Independent Test
`cargo test -p mcp-fs-core --lib bt_0056` is green (12 tests); pushing the stored legacy branch `a//b` returns `ERR_INVALID_ARGUMENT` `'a//b' is not a valid branch name: it contains '//'` and `bare_ref_names` of the remote is unchanged.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0056): Fix at the creation boundary and the push boundary, never only at push. Creation entry points call the strict validator: `branch_create` (`git.rs:2408`), `tag_create` (`git_tags.rs:46`), and the push entry point validates both the local `branch` and `remote_branch` names with the strict form at the same place the remote side is checked today (`git.rs:1685-1687`), before any remote is resolved. **Obliged by:** SPEC-0056/BR-001, BR-006. **Rationale:** the cause is a ref stored that can never be pushed. **Alternatives considered:** validating inside `push_branch_inner` (after the network is resolved); improving only the push error. **Source:** existing code. **Evidence:** `git.rs:1685-1687`, `2408`, `git_tags.rs:46`.
- DEC-009 (SPEC-0056): Documentation: `.agent_docs/git.md` gets the list of the eleven naming rules and the legacy rule, and the line 869 note on `a//b` is corrected. **Obliged by:** SPEC-0056/BR-005, BR-007. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `.agent_docs/git.md:869`.

### Data model
No schema change. Refs already stored with a refused name are left as they are.

### Contracts
No tool, parameter or result change. Refusal: `ERR_INVALID_ARGUMENT`, `'<name>' is not a valid <branch|tag> name: <reason>`. Reasons tried in order after the seven existing ones: 8 `it contains '//'`; 9 `it ends with '.'`; 10 `it contains '@{'`; 11 `one of its '/' separated components ends with '.lock'` (the whole name ending in `.lock` stays `it ends with '.lock'`). Signatures in `crates/core/src/tools/git.rs`: `pub(crate) fn validate_ref_name(name: &str, noun: &str) -> Result<()>` (strict); `pub(crate) fn validate_existing_ref_name(name: &str, noun: &str) -> Result<()>` (the seven original rules).

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (`tool_remote_push` validates the local `branch` with the strict form next to `remote_branch`, tests) |
| `.agent_docs/git.md` | modify (the eleven naming rules, legacy rule, correct the `a//b` note) |

### Existing patterns
`tool_remote_push` at `crates/core/src/tools/git.rs:1661-1688` (the existing `remote_branch` check at `1685-1687`); `e2e_new_891_an_invalid_remote_branch_name_is_refused_before_the_remote` (`git.rs:23740`) for the assertions on the remote refs and the audit; `seed_legacy_ref` from US-0105.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `bt_0056_<number>_<slug>` in the tests module of `crates/core/src/tools/git.rs`. BUG: the reproduction tests are RED on today's code for the root cause (the validator accepts the name) and GREEN after the fix. Legacy state is written straight into the project ref table (DEC-006). Fixtures: project `gitproj` with member Alice (`OWNER`) and one commit on `main`.

### SPEC-0056/BT-013: a legacy branch is refused at push before the network
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_013_*`
- **Validates:** BR-006
- **Driver:** the registered tool `e.call("git.remote_push", ...)` as E2E-NEW-891 (`git.rs:23740`): the refusal happens before the remote is resolved or contacted; the legacy branch is written straight into the project's ref table by the test (the state of a project created before this fix).
- Given the project with a legacy branch row `a//b` pointing at the head commit and a bare remote seeded. When Alice pushes `a//b`. Then `ERR_INVALID_ARGUMENT` `'a//b' is not a valid branch name: it contains '//'`, And the remote refs are unchanged, And no audit entry has the operation `git.remote_push`.

### SPEC-0056/BT-014: an invalid remote branch name is refused for the new classes
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_014_*`
- **Validates:** BR-006
- **Driver:** as E2E-NEW-891.
- Given the project with a valid branch `main` and a bare remote. When Alice pushes `main` with `remote_branch` `a//b`, then `v1.`, then `x@{y}`. Then each is refused with `ERR_INVALID_ARGUMENT` and `'<name>' is not a valid branch name: <reason>`, And the remote refs are unchanged.

## Constraints
- Files not to touch: `git_tags.rs`, `mcp/server.rs`, contract files, anything under `sdlc/specs/`.
- Dependencies not to add: none.
- Patterns to avoid: validating inside `push_branch_inner` (after the network is resolved).
- Scope boundary: nothing else.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (the reproduction tests fail because the call succeeds; tests of already delivered behavior may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every existing push test passes unmodified; if one pushes a nonexistent branch with a name the strict form refuses and expects `branch not found`, report it as a mismatch and do not edit it.
- [ ] Docs updated: .agent_docs/git.md.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] Closes spec: the spec's full acceptance set green (BT-001 to BT-022), and every requirement BR-001 to BR-010 cited by at least one story with status done or landed.
- [ ] One commit `US-0106: refuse a refused branch name at push, documentation and spec close`, body `Spec: SPEC-0056` and the requirement ids.
