# US-0040: git2 0.21 and yoke-derive 0.8.4 with unchanged behavior

> Spec: SPEC-0035 (sdlc/specs/SPEC-0035-clear-warning-advisories/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 6
> Closes spec: no

## Objective
git2 0.20.4 carries two unsound advisories, one of them (`Remote::list`) on our fetch path, and yoke-derive 0.8.3 is yanked. Move to git2 0.21 and yoke-derive 0.8.4, adapt the 4 production call sites and 4 existing test lines to the new `Result`/`Option` returns with byte identical behavior, and extract `blame_identity` so the blame branch the new API introduces is tested. The audit configuration is US-0041's.

## Requirements covered
#### SPEC-0035/DR-001 [EARS-U]: git2 at a fixed version
> The workspace SHALL depend on `git2 = "0.21"` and `Cargo.lock` SHALL pin exactly one `git2` package at version 0.21.x, with `libgit2-sys` unchanged at 0.18.8+1.9.7.
- **Old form:** `git2 = "0.20"`, lock `version = "0.20.4"`
- **New form:** `git2 = "0.21"`, lock `version = "0.21.0"` (or a later 0.21 patch)

#### SPEC-0035/DR-002 [EARS-U]: yoke-derive not yanked
> `Cargo.lock` SHALL pin `yoke-derive` 0.8.4, by a lockfile update only.

#### SPEC-0035/DR-003 [EARS-U]: The git API adaptations keep behavior
> The 3 production sites SHALL treat the new `Result` returns the way they treated the old `Option` returns: a reference whose name is not valid UTF-8 is skipped (`crates/core/src/git/http/mod.rs:336`), a push update whose destination is not valid UTF-8 does not match (`crates/core/src/git/remote.rs:557`), and a branch shorthand that is not valid UTF-8 is treated as no branch (`crates/core/src/tools/git.rs:3086`).

#### SPEC-0035/DR-004 [EARS-O]: Blame without a signature
> IF a blame hunk carries no final signature THEN the blame tool SHALL report empty strings for its author, email and date; a hunk with a signature SHALL report the same three strings as before.
- Implemented by `fn blame_identity(sig: Option<&git2::Signature<'_>>) -> (String, String, String)` in `crates/core/src/tools/git.rs`, `snake_case`.

#### SPEC-0035/DR-008 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.


## Independent Test
`cargo test -p mcp-fs --test security_gate` passes DT-010 and DT-013 (both red on `main`: lock has git2 `0.20.4`, `cargo tree -i git2@0.20.4` resolves); `cargo test -p mcp-fs-core --lib tools::git` passes DT-016 (red against a stub returning `("?", "?", "?")`), DT-018 and the 4 adapted tests; DT-017 passes in `git::http`.

## Design excerpt
### Decisions
- **(SPEC-0035) DEC-001:** Bump git2 to 0.21 and adapt the 4 call sites with `.ok()` or `let Ok`. **Alternatives considered:** keep 0.20 and accept (rejected: the unsound `Remote::list` is called on the fetch path and a fix is released); wrap git2 behind an adapter (rejected: a new layer for 4 type changes). **Rationale:** the probe shows 1 lockfile package, 9 mechanical edits and 879 green git tests. **Implemented by:** DR-001, DR-003, DR-004. **Evidence:** Section 2 probe.
- **(SPEC-0035) DEC-005:** Extract `blame_identity` so the no signature branch is unit tested. **Alternatives considered:** inline `Option` handling with no test (rejected: an untested branch); a `blame_buffer` integration test (rejected: production never calls it). **Implemented by:** DR-004. **Evidence:** `crates/core/src/tools/git.rs:2854` (only `blame_file`).
- **(SPEC-0035) DEC-006:** Lockfile movement is checked by a diff in the story's definition of done, not a committed baseline fixture. **Alternatives considered:** a frozen list of package versions (rejected: stale on the next legitimate bump). **Implemented by:** DR-001, DR-002. **Evidence:** n/a.
- **(SPEC-0035) DEC-008:** Two stories along the seam "code that must compile together" then "gate configuration". **Obliged by:** SPEC-0035/DR-001..DR-008. **Rationale:** the 9 git2 edits cannot be split (the workspace does not compile in between), but the audit configuration can follow: after US-0040, `cargo deny check` only warns `advisory-not-detected` on the two stale ignores and still reports `advisories ok` (Section 2 probe), and `cargo audit` with the Makefile flag still exits 0, so main stays green; each story stays within the tier 2 file budget (6 and 4 paths). Section 9's single step is kept as one story for its code part; the split does not change its order. **Alternatives considered:** one story of 9 paths (above every tier budget, and it would mix a code bump with gate policy); splitting the git2 edits (the build breaks in between). **Source:** design. **Evidence:** Section 2 probe.
- **(SPEC-0035) DEC-009:** Both stories at `min_tier: 2`. **Obliged by:** SPEC-0035/DR-001..DR-008. **Rationale:** decision density is low: the probe fixed every compile edit, and the story inlines them; the only judgment is crafting a raw `packed-refs` line for DT-017 and DT-018. **Alternatives considered:** `min_tier: 1` for US-0040 (6 paths sits at the top of the tier 2 budget; rejected because the edits are dictated, not designed). **Source:** design. **Evidence:** Section 2 probe.
### Assumptions (SPEC-0035)
- **Assumption:** the advisory database is read live; an advisory published after this lot makes DT-012 red for an unrelated crate. That is the intended signal (a new backlog entry), not a scope change.
- **Assumption:** DT-012 also goes red when `zip` is fixed upstream (the count drops below 2). Intended: it prompts updating the expectation in the lot that closes BL-0058 or BL-0059.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | `:74` `git2 = "0.20"` to `git2 = "0.21"` |
| `Cargo.lock` | `cargo update -p yoke-derive`, then the git2 bump; only `git2` and `yoke-derive` entries move |
| `crates/core/src/git/http/mod.rs` | `:336` adaptation; DT-017 in the tests module (`:662`) |
| `crates/core/src/git/remote.rs` | `:557` adaptation |
| `crates/core/src/tools/git.rs` | `blame_identity` used at `:2864-2867`; `:3086` adaptation; the 4 declared test adaptations; DT-016, DT-018 in the tests module (`:7381`) |
| `crates/mcp-fs/tests/security_gate.rs` | DT-010, DT-013 |
### Existing patterns
The scratch worktree probe (2026-10-16) compiled the whole workspace with all features and passed 879 git tests with exactly these edits (git2 0.21 returns `Result` where 0.20 returned `Option`, and `BlameHunk::final_signature` now returns `Option<Signature>`):
- `crates/core/src/git/http/mod.rs:336`: `let Some(name) = r.name() else { continue };` becomes `let Ok(name) = r.name() else { continue };`
- `crates/core/src/git/remote.rs:557`: `if u.dst_refname() == Some(dst_for_negotiation.as_str()) {` becomes `if u.dst_refname().ok() == Some(dst_for_negotiation.as_str()) {`
- `crates/core/src/tools/git.rs:2864-2867`: `let sig = hunk.final_signature();` then author, email, date from `sig`; becomes `let (author, email, date) = blame_identity(hunk.final_signature().as_ref());` with `blame_identity` returning `(String::new(), String::new(), String::new())` for `None`, and for `Some(s)`: `s.name().ok().unwrap_or_default().to_string()`, `s.email().ok().unwrap_or_default().to_string()`, `format_git_time(s.when(), "%Y-%m-%d")` (`format_git_time` at `:7193`), the same strings as today.
- `crates/core/src/tools/git.rs:3086`: `.and_then(|h| h.shorthand())` becomes `.and_then(|h| h.shorthand().ok())`
- Existing tests: `message().ok()` (`:13737`), `e.name().ok()` (`:18422`), `.email().ok()` twice (`:20079-20080`), `r.name().ok()` (`:23062`).
- Security gate test style: `crates/mcp-fs/tests/security_gate.rs:12-30` (`repo_root()`, reading files, running commands).
- Non UTF-8 ref for DT-017 and DT-018: macOS refuses a loose ref file with a non UTF-8 name, so write a raw line `<40 hex sha> refs/heads/` followed by byte `0xff` into the repository's `packed-refs` file with `std::fs::write` on bytes.

## Acceptance tests
> 100% must pass, run through `make check`.

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| SPEC-0035/DT-010 | structural | DR-001 | `crates/mcp-fs/tests/security_gate.rs`, parses `Cargo.lock` text | the `[[package]]` entries named `git2` are exactly one, version starting `0.21.`; `libgit2-sys` version is `0.18.8+1.9.7` | the git2 version is `0.20.4` |
| SPEC-0035/DT-011 | structural | `git diff main -- Cargo.lock` | story definition of done | US-0040 |
| SPEC-0035/DT-013 | security (second channel) | DR-001, DR-002, DR-007 | `security_gate.rs`, runs `cargo tree` | `cargo tree -i git2@0.20.4 --all-features` and `cargo tree -i yoke-derive@0.8.3 --all-features` both fail with `did not match`; `cargo tree -i git2@0.21.0 --all-features` succeeds | both old versions resolve, exit 0 |
| SPEC-0035/DT-016 | edge | DR-004 | `crates/core/src/tools/git.rs` tests | `blame_identity(None)` is `("", "", "")`; `blame_identity(Some(&Signature::new("Ada", "ada@x.test", &Time::new(1_700_000_000, 0)).unwrap()))` is `("Ada", "ada@x.test", "2023-11-14")` | written against a stub returning `("?", "?", "?")`, both assertions fail |
| SPEC-0035/DT-017 | behavior (pin) | DR-003, DR-008 | `crates/core/src/git/http/mod.rs` tests | a repository whose `packed-refs` holds `refs/heads/ok` and a ref whose raw name is `refs/heads/` followed by byte `0xff`: the ref advertisement lists `refs/heads/ok`, omits the other, and does not error | none: green before and after (invariance pin) |
| SPEC-0035/DT-018 | behavior (pin) | DR-003, DR-008 | `crates/core/src/tools/git.rs` tests, the `:3086` caller | HEAD symbolic to that non UTF-8 branch: the operation reports the same no branch outcome as a detached HEAD | none: invariance pin |

## Constraints
- Do not touch `.cargo/audit.toml`, `Makefile`, `deny.toml` (US-0041); `make security` stays green on its own (the two stale git2 ignores only warn in `cargo deny`).
- `git diff main -- Cargo.lock` moves only the `git2` and `yoke-derive` entries (DT-011); `libgit2-sys` stays 0.18.8+1.9.7. Any other package moving: stop and report BLOCKED.
- The 4 existing test edits are `.ok()` adaptations only; no asserted value changes.
- No new dependency.

## Definition of done
- [ ] RED evidence recorded for every new test before the change, in `.implement/US-0040-red.log`.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 4, declared in SPEC-0035 Section 8.4:

| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | `e2e_new_556_no_commit_object_on_a_conflicted_merge` (`:13737`) | `c.message().map(...)` to `c.message().ok().map(...)` | git2 0.21 returns `Result`; same value, compile only |
| `crates/core/src/tools/git.rs` | helper `tree_names` (`:18422`) | `e.name().map(String::from)` to `e.name().ok().map(String::from)` | same |
| `crates/core/src/tools/git.rs` | `e2e_new_876_a_second_member_continues_the_rebase` (`:20079-20080`) | `commit.committer().email()` to `commit.committer().email().ok()`, same for `author()`; asserted values `Some(SECOND)`, `Some(OWNER)` unchanged | same |
| `crates/core/src/tools/git.rs` | helper `bare_ref_names` (`:23062`) | `r.name().map(...)` to `r.name().ok().map(...)` | same |
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] One commit `US-0040: git2 0.21 and yoke-derive 0.8.4 with unchanged behavior`, body `Spec: SPEC-0035` and the DR ids.
