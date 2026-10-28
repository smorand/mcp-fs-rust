# SPEC-0056: Ref name validation, design

> Spec: sdlc/specs/SPEC-0056-ref-name-validation/sdlc-spec.md
> Nature: BUG
> Depth: M
> Security: n/a
> Designed on: 2026-10-28
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024 workspace (`crates/core`) | per `Cargo.toml` | DEC-001 |
| Git engine | git2 (libgit2), used as the oracle of the guard test | 0.21 (`Cargo.toml:63`) | DEC-001, DEC-002 |

Skills loaded: `rust` (via `load_dynamic_context`, already in force): logic in `crates/core`, `ToolError` constructors, no `unwrap` on fallible paths, `make check` as the gate. No new dependency, so no context7 lookup was needed.

## 2. Approaches
### Chosen: fix at the root cause site
Add the four missing rules to the hand written validator `validate_ref_name` (`crates/core/src/tools/git.rs:2357-2392`), keep the reasons in the codebase's own style, and split it into a strict form (creation) and the current form (names that already exist). Pros: the user visible texts of BR-001 to BR-005 are ours, the rule order is explicit, no new dependency, one place. Cons: the rules are hand written, so a guard test must keep them equal to git's. Affected scope: `tools/git.rs` only for production code.
### Rejected: ask libgit2 at creation
Call `git2::Reference::is_valid_name` and refuse on false (DEC-002). It gives no reason text, so BR-001 to BR-005 (exact reasons and order) would need the hand rules anyway.
### Rejected: fix at the boundary (push)
Only improve the push error, leave creation permissive. It keeps the cause (a ref stored that can never be pushed) and contradicts SPEC-0012/FR-NEW-104 (DEC-003).

## 3. Architecture
```
git.branch_create / git.tag_create / git.remote_push (remote_branch, branch)
   -> validate_ref_name(name, noun)           strict: 7 existing rules, then 4 new (git.rs)
git.branch_switch / git.branch_reset / git.branch_delete
   -> validate_existing_ref_name(name, noun)  the 7 existing rules only (names stored before this fix stay manageable)
git.tag_delete: no name check (unchanged, git_tags.rs:65-82)
git.remote_push_tag / remote_push_tags: unchanged (is_valid_name before the push, git_tags.rs:700-705, 1167-1172)
```
Bounded context: Versioning (validator and its callers), Remote exchange (the push entry point `tool_remote_push`, `git.rs:1661`).

## 4. Data model
No entity, no migration. Refs already stored with a refused name are left as they are (BR-007, BR-008).

## 5. Contracts and interfaces
No tool, parameter, result shape or description changes (the frozen descriptions in `TOOL_CONTRACT.txt` do not mention name rules). Refusals, code `ERR_INVALID_ARGUMENT`, message `'<name>' is not a valid <branch|tag> name: <reason>`. Reasons, in the order they are tried after the seven existing ones:

| # | Condition | Reason |
|---|---|---|
| 8 | the name contains `//` | `it contains '//'` |
| 9 | a `/` separated component ends with `.` | `one of its '/' separated components ends with '.'` |
| 10 | the name contains `@{` | `it contains '@{'` |
| 11 | a `/` separated component ends with `.lock` (the whole name ending in `.lock` is rule 7, `it ends with '.lock'`) | `one of its '/' separated components ends with '.lock'` |

Internal signatures (Rust, `crates/core/src/tools/git.rs`): `pub(crate) fn validate_ref_name(name: &str, noun: &str) -> Result<()>` (strict, gains rules 8 to 11); new `pub(crate) fn validate_existing_ref_name(name: &str, noun: &str) -> Result<()>` (rules 1 to 7); the private `validate_branch_name` stays the strict branch wrapper; a new private `validate_existing_branch_name` is the existing rules wrapper.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | modify (validator split, 4 rules, 3 call sites switched to the existing form, `tool_remote_push` local branch check, tests) | BR-001..010 | 1, 2 |
| `.agent_docs/git.md` | modify (naming rules paragraph, line 869 note) | BR-005, BR-006 | 2 |

Affected existing tests: `e2e_tag_085_remote_refuses_the_tag_name` (`git.rs:26040`) and SPEC-0054 `e2e_tag_057_publish_all_an_invalid_ref_name` (`git.rs:28680`) modified (setup only, DEC-007); `e2e_new_404`, `e2e_new_891`, `e2e_tag_010`, `e2e_new_405` to `407` and the unit test `a_remote_name_refusal_names_the_tag_and_the_reason` (`git_tags.rs:1521`) kept. Affected documentation: `.agent_docs/git.md`.

## 7. Compatibility and risks
- No consumer sees a new field or tool. A client that created `a//b` branches or tags now gets `ERR_INVALID_ARGUMENT` at creation instead of an internal error at push; stored names keep working (BR-007, BR-008).
- Risk: the hand rules drift from git's. The guard test of DEC-002 compares the validator with libgit2 over a generated alphabet, so a future disagreement fails the suite.
- Risk: an existing test pushes a nonexistent branch with a name the new rules refuse and expects `branch not found`. Mitigation: story 2 runs the full suite and reports any such test as a mismatch instead of editing it.
- Ordering: story 1 lands before story 2 (story 2 needs the strict form to exist). Rollback: revert the commits; no data to undo.

## 9. Test strategy
All tests are functional e2e tests through the registered tools of the test harness (`Env::call`) in the tests module of `crates/core/src/tools/git.rs`, plus one guard unit test (DEC-002). Legacy state (a branch or tag stored with a refused name, as a project created before the fix would hold) is written straight into the project's ref table by the test (DEC-006). The BUG reproduction tests BT-001 and BT-002 are RED on today's code because `git.branch_create` succeeds, and GREEN after.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | registered tool `git.branch_create` | `crates/core/src/tools/git.rs` tests, `bt_0056_001_*` | US-0105 |
| BT-002 | functional e2e | registered tool `git.branch_create` | same, `bt_0056_002_*` | US-0105 |
| BT-010 | functional e2e | registered tool `git.branch_create` | same, `bt_0056_010_*` | US-0105 |
| BT-011 | functional e2e | registered tool `git.tag_create` | same, `bt_0056_011_*` | US-0105 |
| BT-012 | functional e2e | registered tools | same, `bt_0056_012_*` | US-0105 |
| BT-013 | functional e2e | registered tool `git.remote_push` | same, `bt_0056_013_*` | US-0106 |
| BT-014 | functional e2e | registered tool `git.remote_push` | same, `bt_0056_014_*` | US-0106 |
| BT-015 | functional e2e | registered tools `git.branch_switch`, `git.branch_reset`, `git.branch_delete` | same, `bt_0056_015_*` | US-0105 |
| BT-016 | functional e2e | registered tool `git.tag_delete`, publish helper | same, `bt_0056_016_*` | US-0105 |
| BT-020 | functional e2e | registered tools | same, `bt_0056_020_*` | US-0105 |
| BT-021 | functional e2e | registered tool `git.branch_create` | same, `bt_0056_021_*` | US-0105 |
| BT-022 | functional e2e | registered tool `git.branch_create` | same, `bt_0056_022_*` | US-0105 |

Test data: generated by the harness; none user provided.

## 10. Decisions log
- **DEC-001:** No new dependency and no version change: Rust 2024, git2 0.21 (`Cargo.toml:63`). Skill loaded: rust. **Obliged by:** n/a (no code impact), records the stack. **Rationale:** the fix is rules in existing code. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `Cargo.toml:63`.
- **DEC-002:** Fix at the root cause site: append four rules to the validator after the seven existing ones, in the order and with the reasons of SPEC-0056 Section 5 here; add a guard unit test that compares `validate_ref_name(n, "branch").is_ok()` with `git2::Reference::is_valid_name("refs/heads/" + n)` over every name up to 5 characters of the alphabet `a . / @ { } - l :`, plus the hand picked names `a.lock`, `a.lock/b`, `a/b.lock`, `@`, `a@b`, `a@{b`, `x.lock/`, requiring equality both ways. **Obliged by:** SPEC-0056/BR-001, BR-002, BR-003, BR-004, BR-005, BR-009. **Rationale:** the reasons are user visible and ordered, which libgit2 cannot give; the guard keeps the hand rules equal to git's (the 66 439 name sweep of the spec is the model). **Alternatives considered:** calling `Reference::is_valid_name` and refusing without a reason (BR texts impossible); a regex (the rules are positional per component, clearer as code). **Source:** design. **Evidence:** `crates/core/src/tools/git.rs:2357-2392`.
- **DEC-003:** Fix at the creation boundary and the push boundary, never only at push. Creation entry points call the strict validator: `branch_create` (`git.rs:2408`), `tag_create` (`git_tags.rs:46`), and the push entry point validates both the local `branch` and `remote_branch` names with the strict form at the same place the remote side is checked today (`git.rs:1685-1687`), before any remote is resolved. **Obliged by:** SPEC-0056/BR-001, BR-006. **Rationale:** the cause is a ref stored that can never be pushed. **Alternatives considered:** validating inside `push_branch_inner` (after the network is resolved); improving only the push error. **Source:** existing code. **Evidence:** `git.rs:1685-1687`, `2408`, `git_tags.rs:46`.
- **DEC-004:** Names that already exist stay manageable: `branch_switch` (`git.rs:2542`), `branch_delete` (`2644`) and `branch_reset` (`2710`) call `validate_existing_ref_name`, the seven original rules, so their behavior is byte identical to today. **Obliged by:** SPEC-0056/BR-007, BR-010. **Rationale:** otherwise a stored `a//b` branch could not be deleted. **Alternatives considered:** no validation on these three tools (changes the refusal for names that were already refused, e.g. `has space`, breaking BR-010). **Source:** existing code. **Evidence:** `git.rs:2542`, `2644`, `2710`.
- **DEC-005:** Tags need no further change: `tag_delete` has no name check (`git_tags.rs:65-82`) and the publish paths keep their `Reference::is_valid_name` guard and `Remote refused tag '<n>': invalid ref name` text. **Obliged by:** SPEC-0056/BR-008. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `git_tags.rs:65-82`, `700-705`, `1167-1172`.
- **DEC-006:** Legacy state in tests is written directly: a helper in the tests module calls `entry.db.set_ref("refs/heads/a//b" or "refs/tags/a//b", sha, false)` on the repository entry. No production helper, no test only code path in production. **Obliged by:** SPEC-0056/BR-007, BR-008. **Alternatives considered:** creating the ref with the old validator (gone after the fix). **Source:** existing code. **Evidence:** `git.rs:2461` (`entry.db.set_ref(&branch_ref, &sha, false)`).
- **DEC-007:** The two existing tests that create the tag `a//b` through the tool, `e2e_tag_085_remote_refuses_the_tag_name` and SPEC-0054 `e2e_tag_057_publish_all_an_invalid_ref_name`, get the legacy tag written by the DEC-006 helper instead; their assertions are unchanged. **Obliged by:** SPEC-0056/BR-005, BR-008. **Alternatives considered:** deleting them (the publish refusal for stored names stays specified). **Source:** spec 4.4. **Evidence:** `git.rs:26040`, `28680`.
- **DEC-008:** The silent `let _ = repo.reference(...)` in `branch_create` and `tag_create` stays: with refused names refused up front it is a defence for failures other than the name. It becomes backlog BL-0095. **Obliged by:** n/a (no code impact): out of scope of every requirement. **Alternatives considered:** surfacing the error now (a behavior change outside the spec). **Source:** existing code. **Evidence:** `git.rs:2465`, `git_tags.rs:157`.
- **DEC-009:** Documentation: `.agent_docs/git.md` gets the list of the eleven naming rules and the legacy rule, and the line 869 note on `a//b` is corrected. **Obliged by:** SPEC-0056/BR-005, BR-007. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `.agent_docs/git.md:869`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0105 | refuse the ref names git refuses at creation | P1 | BR-001, BR-002, BR-003, BR-004, BR-005, BR-007, BR-008, BR-009, BR-010 | main | 2 | no |
| US-0106 | refuse a refused branch name at push, documentation and spec close | P2 | BR-006 | US-0105 | 2 | yes |
