# US-0105: refuse the ref names git refuses at creation

> Spec: SPEC-0056 (sdlc/specs/SPEC-0056-ref-name-validation/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Fix the root cause: add the four missing rules to the ref name validator so a branch or tag named `a//b`, `v1.`, `x@{y}` or `a.lock/b` is refused at creation with an exact reason, while names already stored stay switchable, resettable and deletable. Carries the two reproduction tests, a guard test that keeps the validator equal to libgit2, and the setup change of the two existing tests that created `a//b` through the tool.

## Requirements covered
- SPEC-0056/BR-001: IF a member creates a branch whose name contains `//`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: it contains '//'` and create nothing.
- SPEC-0056/BR-002: IF a member creates a branch whose name has a `/` separated component ending with `.`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: one of its '/' separated components ends with '.'` and create nothing.
- SPEC-0056/BR-003: IF a member creates a branch whose name contains `@{`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: it contains '@{'` and create nothing.
- SPEC-0056/BR-004: IF a member creates a branch whose name has a `/` separated component ending with `.lock` and the whole name does not end with `.lock`, THEN THE Versioning context SHALL refuse with `'<name>' is not a valid branch name: one of its '/' separated components ends with '.lock'` and create nothing.
- SPEC-0056/BR-005: The Versioning context SHALL apply BR-001 to BR-004 to tag names with the word `tag` in place of `branch` (`'<name>' is not a valid tag name: <reason>`), and SHALL check the reasons in this order, the first that applies being the one given: the seven existing reasons in their existing order, then `it contains '//'`, then `one of its '/' separated components ends with '.'`, then `it contains '@{'`, then `one of its '/' separated components ends with '.lock'`.
- SPEC-0056/BR-007: The Versioning context SHALL NOT refuse to switch to, reset or delete an existing branch because its name is one that BR-001 to BR-004 refuse for creation.
- SPEC-0056/BR-008: The Versioning context SHALL NOT refuse to delete an existing tag because of its name, and publishing such a tag SHALL keep giving `Remote refused tag '<name>': invalid ref name`.
- SPEC-0056/BR-009: The Versioning context SHALL NOT refuse a branch or tag name that `git check-ref-format` accepts.
- SPEC-0056/BR-010: The Versioning context SHALL NOT change the refusal text, the code or the order of the seven existing reasons.

## Independent Test
`cargo test -p mcp-fs-core --lib bt_0056` is green; before the fix `bt_0056_001_*` and `bt_0056_002_*` fail because `git.branch_create` succeeds; the guard test `ref_name_validator_matches_libgit2` is green; `cargo test -p mcp-fs-core --lib e2e_tag_` and `e2e_new_4` stay green.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0056): No new dependency and no version change: Rust 2024, git2 0.21 (`Cargo.toml:63`). Skill loaded: rust. **Obliged by:** n/a (no code impact), records the stack. **Rationale:** the fix is rules in existing code. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `Cargo.toml:63`.
- DEC-002 (SPEC-0056): Fix at the root cause site: append four rules to the validator after the seven existing ones, in the order and with the reasons of SPEC-0056 Section 5 here; add a guard unit test that compares `validate_ref_name(n, "branch").is_ok()` with `git2::Reference::is_valid_name("refs/heads/" + n)` over every name up to 5 characters of the alphabet `a . / @ { } - l :`, plus the hand picked names `a.lock`, `a.lock/b`, `a/b.lock`, `@`, `a@b`, `a@{b`, `x.lock/`, requiring equality both ways. **Obliged by:** SPEC-0056/BR-001, BR-002, BR-003, BR-004, BR-005, BR-009. **Rationale:** the reasons are user visible and ordered, which libgit2 cannot give; the guard keeps the hand rules equal to git's (the 66 439 name sweep of the spec is the model). **Alternatives considered:** calling `Reference::is_valid_name` and refusing without a reason (BR texts impossible); a regex (the rules are positional per component, clearer as code). **Source:** design. **Evidence:** `crates/core/src/tools/git.rs:2357-2392`.
- DEC-003 (SPEC-0056): Fix at the creation boundary and the push boundary, never only at push. Creation entry points call the strict validator: `branch_create` (`git.rs:2408`), `tag_create` (`git_tags.rs:46`), and the push entry point validates both the local `branch` and `remote_branch` names with the strict form at the same place the remote side is checked today (`git.rs:1685-1687`), before any remote is resolved. **Obliged by:** SPEC-0056/BR-001, BR-006. **Rationale:** the cause is a ref stored that can never be pushed. **Alternatives considered:** validating inside `push_branch_inner` (after the network is resolved); improving only the push error. **Source:** existing code. **Evidence:** `git.rs:1685-1687`, `2408`, `git_tags.rs:46`.
- DEC-004 (SPEC-0056): Names that already exist stay manageable: `branch_switch` (`git.rs:2542`), `branch_delete` (`2644`) and `branch_reset` (`2710`) call `validate_existing_ref_name`, the seven original rules, so their behavior is byte identical to today. **Obliged by:** SPEC-0056/BR-007, BR-010. **Rationale:** otherwise a stored `a//b` branch could not be deleted. **Alternatives considered:** no validation on these three tools (changes the refusal for names that were already refused, e.g. `has space`, breaking BR-010). **Source:** existing code. **Evidence:** `git.rs:2542`, `2644`, `2710`.
- DEC-005 (SPEC-0056): Tags need no further change: `tag_delete` has no name check (`git_tags.rs:65-82`) and the publish paths keep their `Reference::is_valid_name` guard and `Remote refused tag '<n>': invalid ref name` text. **Obliged by:** SPEC-0056/BR-008. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `git_tags.rs:65-82`, `700-705`, `1167-1172`.
- DEC-006 (SPEC-0056): Legacy state in tests is written directly: a helper in the tests module calls `entry.db.set_ref("refs/heads/a//b" or "refs/tags/a//b", sha, false)` on the repository entry. No production helper, no test only code path in production. **Obliged by:** SPEC-0056/BR-007, BR-008. **Alternatives considered:** creating the ref with the old validator (gone after the fix). **Source:** existing code. **Evidence:** `git.rs:2461` (`entry.db.set_ref(&branch_ref, &sha, false)`).
- DEC-007 (SPEC-0056): The two existing tests that create the tag `a//b` through the tool, `e2e_tag_085_remote_refuses_the_tag_name` and SPEC-0054 `e2e_tag_057_publish_all_an_invalid_ref_name`, get the legacy tag written by the DEC-006 helper instead; their assertions are unchanged. **Obliged by:** SPEC-0056/BR-005, BR-008. **Alternatives considered:** deleting them (the publish refusal for stored names stays specified). **Source:** spec 4.4. **Evidence:** `git.rs:26040`, `28680`.
- DEC-008 (SPEC-0056): The silent `let _ = repo.reference(...)` in `branch_create` and `tag_create` stays: with refused names refused up front it is a defence for failures other than the name. It becomes backlog BL-0095. **Obliged by:** n/a (no code impact): out of scope of every requirement. **Alternatives considered:** surfacing the error now (a behavior change outside the spec). **Source:** existing code. **Evidence:** `git.rs:2465`, `git_tags.rs:157`.

### Data model
No schema change. Refs already stored with a refused name are left as they are.

### Contracts
No tool, parameter or result change. Refusal: `ERR_INVALID_ARGUMENT`, `'<name>' is not a valid <branch|tag> name: <reason>`. Reasons tried in order after the seven existing ones: 8 `it contains '//'`; 9 `one of its '/' separated components ends with '.'`; 10 `it contains '@{'`; 11 `one of its '/' separated components ends with '.lock'` (the whole name ending in `.lock` stays `it ends with '.lock'`). Signatures in `crates/core/src/tools/git.rs`: `pub(crate) fn validate_ref_name(name: &str, noun: &str) -> Result<()>` (strict); `pub(crate) fn validate_existing_ref_name(name: &str, noun: &str) -> Result<()>` (the seven original rules).

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git.rs` | modify (validator split and four rules, switch/reset/delete call the existing form, tests, helper `seed_legacy_ref`, setup of `e2e_tag_085_*` and `e2e_tag_057_publish_all_*`) |

### Existing patterns
`validate_ref_name` and `validate_branch_name` at `crates/core/src/tools/git.rs:2349-2392`; the tests `e2e_new_404_an_invalid_branch_name_is_refused_before_any_write` (`git.rs:16432`) and `e2e_tag_010_other_invalid_names` (`git.rs:24194`) for the refusal assertions; `entry.db.set_ref` at `git.rs:2461` for the legacy rows; the helper `tag`/`branch_create` of the tests module.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. Test functions are named `bt_0056_<number>_<slug>` in the tests module of `crates/core/src/tools/git.rs`. BUG: the reproduction tests are RED on today's code for the root cause (the validator accepts the name) and GREEN after the fix. Legacy state is written straight into the project ref table (DEC-006). Fixtures: project `gitproj` with member Alice (`OWNER`) and one commit on `main`.

### SPEC-0056/BT-001: a branch named `a//b` is refused at creation
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_001_*`
- **Reproduces:** BUG-001
- **Driver:** the registered MCP tools through the e2e harness (`e.call("git.branch_create", ...)`, `e.call("git.branches", ...)`), against a project with one commit.
- **Fails today because:** `validate_ref_name` has no `//` rule (`git.rs:2357-2392`), so the call succeeds.
- Given a member Alice of project `gitproj` with one commit on `main`. When Alice calls `git.branch_create` with name `a//b`. Then the call fails with `ERR_INVALID_ARGUMENT` and the message `'a//b' is not a valid branch name: it contains '//'`, And `git.branches` lists only `main`, And no audit entry and no quota charge were added.

### SPEC-0056/BT-002: a branch named `v1.` is refused at creation
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_002_*`
- **Reproduces:** BUG-001
- **Driver:** as BT-001.
- **Fails today because:** no rule for a component ending in `.`.
- Given the project of BT-001. When Alice creates the branch `v1.`. Then `ERR_INVALID_ARGUMENT` `'v1.' is not a valid branch name: one of its '/' separated components ends with '.'`, And `git.branches` lists only `main`.

### SPEC-0056/BT-010: the four classes are refused for branches
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_010_*`
- **Validates:** BR-001, BR-002, BR-003, BR-004
- **Driver:** as BT-001.
- Given the project. When Alice creates, one at a time, `a//b`, `a./b`, `x@{y}`, `a.lock/b`. Then each is refused with `ERR_INVALID_ARGUMENT` and respectively `it contains '//'`, `one of its '/' separated components ends with '.'`, `it contains '@{'`, `one of its '/' separated components ends with '.lock'` (full message `'<name>' is not a valid branch name: <reason>`), And `git.branches` lists only `main` after the four.

### SPEC-0056/BT-011: the four classes are refused for tags
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_011_*`
- **Validates:** BR-005
- **Driver:** `e.call("git.tag_create", ...)`, `e.call("git.tags", ...)`.
- Given the project. When Alice creates the tags `a//b`, `v1.`, `x@{y}`, `a.lock/b` one at a time. Then each is refused with `ERR_INVALID_ARGUMENT` and `'<name>' is not a valid tag name: <reason>` with the same four reasons, And `git.tags` lists no tag.

### SPEC-0056/BT-012: the first reason that applies is given
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_012_*`
- **Validates:** BR-005
- **Driver:** as BT-011, and `git.branch_create`.
- Given the project. When Alice creates the tag `a@{b.` and the branch `a.lock/b//c`. Then the tag is refused with `'a@{b.' is not a valid tag name: one of its '/' separated components ends with '.'`, And the branch is refused with `'a.lock/b//c' is not a valid branch name: it contains '//'`.

### SPEC-0056/BT-015: legacy branches stay manageable
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_015_*`
- **Validates:** BR-007
- **Driver:** `e.call("git.branch_switch" | "git.branch_reset" | "git.branch_delete", ...)`.
- Given the project with the legacy branch row `a//b` at the head commit. When Alice switches to `a//b`, back to `main`, resets `a//b` to the head commit, and deletes it. Then each call succeeds (switch returns `branch` `a//b`; delete returns `branch` `a//b`), And `git.branches` then lists only `main`.

### SPEC-0056/BT-016: legacy tags stay deletable and keep their publish refusal
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_016_*`
- **Validates:** BR-008
- **Driver:** `git.tag_delete` through the registered tool; the publish helper of the SPEC-0052 tests for the `file://` remote.
- Given the project with the legacy tag row `a//b` at the head commit and a bare remote. When Alice publishes `a//b`. Then `ERR_INTERNAL_ERROR` `Remote refused tag 'a//b': invalid ref name`, And the remote refs are unchanged, And when Alice then calls `git.tag_delete` on `a//b` it succeeds and `git.tags` lists no tag.

### SPEC-0056/BT-020: valid names stay accepted
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_020_*`
- **Protects:** BR-009
- Given the project. When Alice creates the branches `@`, `ok/name`, `release/2026-q1`, `é`, a 255 byte name, and the tags `v1.0`, `@`, `release/2026-q1`, `é`. Then all are accepted, And a 256 byte branch name is refused with `it is 256 bytes of UTF-8, over the 255 byte limit` (unchanged).

### SPEC-0056/BT-021: the seven existing reasons are unchanged
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_021_*`
- **Protects:** BR-010
- Given the project. When Alice creates the branches `bad..name`, `has space`, `/leading`, `trailing/`, `tip.lock`, `a/.hidden`, and the empty name. Then each is refused with the same code and text as before this change (`it contains '..'`, `it contains the forbidden character ' '`, `it begins or ends with '/'`, `it begins or ends with '/'`, `it ends with '.lock'`, `one of its '/' separated components begins with '.'`, `it is empty`), And `a/b.lock` gives `it ends with '.lock'`.

### SPEC-0056/BT-022: a refused creation leaves nothing behind
- Level / Driver / Location: functional e2e / registered tools through `Env::call` / `crates/core/src/tools/git.rs`, `bt_0056_022_*`
- **Protects:** BR-001
- Given the project with one audit entry for the first commit. When Alice's creation of `a//b` is refused. Then the audit trail, the quota usage and the list of refs are identical to before the call.

### Guard: `ref_name_validator_matches_libgit2` (DEC-002, not a spec test)
- Level / Driver / Location: unit / direct call / `crates/core/src/tools/git.rs` tests module
- For every name of 1 to 5 characters over the alphabet `a . / @ { } - l :` plus `a.lock`, `a.lock/b`, `a/b.lock`, `@`, `a@b`, `a@{b`, `x.lock/`: `validate_ref_name(n, "branch").is_ok()` equals `git2::Reference::is_valid_name(&format!("refs/heads/{n}"))`. Expected counts on the spec's run: 0 names accepted by one and refused by the other.

## Constraints
- Files not to touch: `crates/core/src/tools/git_tags.rs`, `mcp/server.rs`, `TOOL_CONTRACT.txt` and the golden (no contract change), anything under `sdlc/specs/`.
- Dependencies not to add: none.
- Patterns to avoid: changing the text, code or order of the seven existing reasons (BR-010); calling libgit2 to decide a refusal.
- Scope boundary: the push entry point (`tool_remote_push`) and the docs belong to US-0106.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (the reproduction tests fail because the call succeeds; tests of already delivered behavior may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `e2e_new_404`, `405`, `406`, `407`, `e2e_new_891`, `e2e_tag_010`, `e2e_tag_011` and the whole git suite pass unmodified, except the setup of `e2e_tag_085_remote_refuses_the_tag_name` and SPEC-0054 `e2e_tag_057_publish_all_an_invalid_ref_name` (assertions unchanged).
- [ ] Docs updated: none beyond code comments (US-0106 owns `.agent_docs/git.md`).
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0105: refuse the ref names git refuses at creation`, body `Spec: SPEC-0056` and the requirement ids.
