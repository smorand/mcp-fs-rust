# Clear or formally accept the warning level RustSec advisories: technical debt design

> Id: SPEC-0035
> Nature: DEBT
> Depth: L
> Depth evidence: 3 bounded areas (dependency manifests and security gate config, the git layer: HTTP server, remote pipeline, tools), 8 requirements, 12 audit findings, no public contract
> Status: specified
> Generated on: 2026-10-16
> Behavior change: none
> From backlog: BL-0053
> Depends on: SPEC-0029 (security gate green)
> Security: RUSTSEC-2026-0183, RUSTSEC-2026-0184 (fixed); RUSTSEC-2024-0384, RUSTSEC-2024-0436, RUSTSEC-2026-0192, RUSTSEC-2026-0253, RUSTSEC-2026-0002, RUSTSEC-2024-0449 (accepted, recorded)
> CVSS: not scored
> Affected: every commit whose Cargo.lock pins git2 0.20.4, including main at 203360f
> Fixed in: unreleased

## 1. Debt

### DBT-001: Twelve audit warnings are neither fixed nor recorded
- **Current structure:** `make security` (`Makefile:46-48`) runs `cargo audit --ignore RUSTSEC-2023-0071` then `cargo deny check`. `cargo audit` reports 12 allowed warnings (run on 2026-10-16): unmaintained `instant` 0.1.13 (RUSTSEC-2024-0384) and `paste` 1.0.15 (RUSTSEC-2024-0436), both via `datafusion` 43; unmaintained `ttf-parser` 0.25.1 (RUSTSEC-2026-0192) via `pdf-extract` 0.12.1; unmaintained `unsafe-libyaml` 0.2.11 (RUSTSEC-2024-0449) via `serde_yaml` 0.9; unsound `git2` 0.20.4 (RUSTSEC-2026-0183, RUSTSEC-2026-0184), a direct dependency (`Cargo.toml:74`, `crates/core/Cargo.toml:60`); unsound `lru` 0.12.5 (RUSTSEC-2026-0253, RUSTSEC-2026-0002) via `aws-sdk-s3` 1.119 and `lru` 0.16.4 (RUSTSEC-2026-0253) via `tantivy` 0.26.2; yanked `yoke-derive` 0.8.3, `zip` 2.4.2 (direct, `Cargo.toml:84`) and `zip` 3.0.0 (build dependency of `utoipa-swagger-ui` 9, `Cargo.toml:113`). `deny.toml:17-21` ignores RUSTSEC-2023-0071, RUSTSEC-2026-0183, RUSTSEC-2026-0184 with "no fix released yet" for the two git2 ids.
- **Cost paid today:** a known unsound function is on a live path: `Remote::list()` is called by the fetch pipeline (`crates/core/src/git/remote.rs:722`). Every audit run prints 12 warnings, so a new one does not stand out. Two `deny.toml` reasons are false (git2 0.21.0 is released).
- **Target structure:** git2 0.21 and yoke-derive 0.8.4 in the lockfile; every remaining advisory recorded once in `.cargo/audit.toml` with its reason; `cargo audit` left with exactly the 2 yanked `zip` warnings, each tracked by a backlog entry.
- **Observable behavior:** unchanged.

### Owner decisions (2026-10-16)
Fix now: `yoke-derive` (lockfile) and `git2` 0.21. Accept with a written reason: `lru` via tantivy, `instant` and `paste` via datafusion, `ttf-parser` via pdf-extract. Separate backlog entries: `zip` 9, `utoipa-swagger-ui` 10, a Rust toolchain at least 1.94.1 (unlocks a fixed `aws-sdk-s3` and its `lru`), replacing `serde_yaml` by a maintained fork. Until those land, their advisories (`lru` via aws-sdk-s3, `unsafe-libyaml`) are recorded as accepted with the backlog id as reason.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `Cargo.toml` | `git2 = "0.20"` (`:74`) to `git2 = "0.21"` | 1 |
| `Cargo.lock` | `git2` 0.20.4 to 0.21.x (dependency list of the `git2` entry included); `yoke-derive` 0.8.3 to 0.8.4; `libgit2-sys` unchanged at 0.18.8+1.9.7 (`Cargo.lock:3399-3400`); nothing else moves | 2 package entries |
| `crates/core/src/git/http/mod.rs` | `:336` `let Some(name) = r.name() else { continue };` to `let Ok(name) = r.name() else { continue };` (`Reference::name` returns `Result` in 0.21) | 1 |
| `crates/core/src/git/remote.rs` | `:557` `u.dst_refname() == Some(...)` to `u.dst_refname().ok() == Some(...)` (`PushUpdate::dst_refname` returns `Result`) | 1 |
| `crates/core/src/tools/git.rs` | blame `:2864-2867`: the author, email and date of a hunk computed by a new helper `blame_identity(sig: Option<&git2::Signature<'_>>) -> (String, String, String)` (`BlameHunk::final_signature` returns `Option` in 0.21); `:3086` `.and_then(\|h\| h.shorthand())` to `.and_then(\|h\| h.shorthand().ok())` | 2 sites, 1 helper |
| `crates/core/src/tools/git.rs` (tests module from `:7381`) | 4 existing test functions adapted, see 8.4; new tests DT-016, DT-018 | 5 lines in 4 functions |
| `crates/core/src/git/http/mod.rs` (tests module from `:662`) | new test DT-017 | 1 test |
| `.cargo/audit.toml` | new: `[advisories] ignore` with 7 ids, one reason comment each | new file |
| `Makefile` | `:47` `cargo audit --ignore RUSTSEC-2023-0071` to `cargo audit` | 1 |
| `deny.toml` | remove `:19` RUSTSEC-2026-0183 and `:20` RUSTSEC-2026-0184 | 2 |
| `crates/mcp-fs/tests/security_gate.rs` | new tests DT-010, DT-012, DT-013, DT-014, DT-015 beside `make_security_passes` | 5 tests |

**Non source occurrences:** `deny.toml:18-20`, `Makefile:47` are the only places naming a RUSTSEC id outside `sdlc/` (`rg -n "RUSTSEC-20" --glob '!sdlc/**'`); no doc names the git2 version; `.cargo/` does not exist yet.
**Contracts touched:** none public. The git HTTP protocol output, the tool outputs and the REST plane are unchanged (DR-008).
**Probe evidence (scratch worktree, 2026-10-16):** with exactly the edits above, `cargo check --workspace --all-targets --all-features` exits 0, `cargo update` locks 1 package (`Updating git2 v0.20.4 -> v0.21.0`), and `cargo test -p mcp-fs-core --all-features --lib -- git:: tools::git` gives 879 passed, 0 failed. `cargo audit` honors `.cargo/audit.toml` at the workspace root; `cargo deny check` warns `advisory-not-detected` on the two git2 ids once git2 is fixed.
**Guardrail coverage:** the 879 git tests, the security gate tests (`crates/mcp-fs/tests/security_gate.rs:12-30`), the full suite. **Test command:** see 8.1.
**Total:** 11 files, recounted.

## 3. Class sweep

**Class:** a third party component version flagged by a published security advisory, or withdrawn by its publisher, reaches the built binaries (CWE-1395, dependency on a vulnerable third party component).
**Searches run:**
1. `cargo audit --ignore RUSTSEC-2023-0071 --json`: the RustSec database against every package of `Cargo.lock`, warnings of every kind (unmaintained, unsound, yanked).
2. `cargo deny check advisories` (`deny.toml:1-2`, `all-features = true`): the same database through the feature graph, with this repository's ignore list.
3. `cargo tree -i <package> --all-features -e normal,build` per finding: who pulls each in.
Idioms not searched: vendored or copied third party source (`git ls-files` holds no vendor directory; both tools read only the dependency graph).

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | git2 0.20.4, RUSTSEC-2026-0183 (`Remote::list`, called at `crates/core/src/git/remote.rs:722`) | unsound, on a live path, fix released (>= 0.21.0) | DR-001, DR-003, this lot (the triggering occurrence) |
| 2 | git2 0.20.4, RUSTSEC-2026-0184 (`BlameHunk` signature) | unsound only for buffer created blames; we call `blame_file` only (`crates/core/src/tools/git.rs:2854`), fix released | DR-001, DR-004, this lot |
| 3 | yoke-derive 0.8.3, yanked | lockfile bump to 0.8.4 available | DR-002, this lot |
| 4 | lru 0.16.4, RUSTSEC-2026-0253, via tantivy 0.26.2 (latest) | no upstream fix | accepted, DR-005 (owner decision 2026-10-16) |
| 5 | lru 0.12.5, RUSTSEC-2026-0253 and RUSTSEC-2026-0002, via aws-sdk-s3 | a fixed aws-sdk-s3 needs Rust 1.94.1, the project pins 1.88 (`Cargo.toml:8`, `rust-toolchain.toml:2`) | accepted until BL-0060, DR-005 (owner decision) |
| 6 | instant 0.1.13 RUSTSEC-2024-0384, paste 1.0.15 RUSTSEC-2024-0436, via datafusion 43 (latest 55) | unmaintained, no advisory of a flaw | accepted, DR-005 (owner decision) |
| 7 | ttf-parser 0.25.1 RUSTSEC-2026-0192 via pdf-extract 0.12.1 (latest) | unmaintained, no upstream fix | accepted, DR-005 (owner decision) |
| 8 | unsafe-libyaml 0.2.11 RUSTSEC-2024-0449 via serde_yaml 0.9 (deprecated) | unmaintained; a maintained fork exists | accepted until BL-0061, DR-005 (owner decision) |
| 9 | zip 2.4.2, yanked, direct | no 2.x release selectable; 9.0.4 is a large API move | stays visible, BL-0058 (owner decision) |
| 10 | zip 3.0.0, yanked, build dependency of utoipa-swagger-ui 9 | utoipa-swagger-ui 10.0.1 available | stays visible, BL-0059 (owner decision) |

The four backlog entries are `Nature: DEBT`, not `BUG`: a dependency upgrade with no functional delta routes to DEBT (contract, Phase 1 security table). Each carries a `Decision:` field and non empty `Evidence`.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-U]: git2 at a fixed version
> The workspace SHALL depend on `git2 = "0.21"` and `Cargo.lock` SHALL pin exactly one `git2` package at version 0.21.x, with `libgit2-sys` unchanged at 0.18.8+1.9.7.
- **Old form:** `git2 = "0.20"`, lock `version = "0.20.4"`
- **New form:** `git2 = "0.21"`, lock `version = "0.21.0"` (or a later 0.21 patch)

#### DR-002 [EARS-U]: yoke-derive not yanked
> `Cargo.lock` SHALL pin `yoke-derive` 0.8.4, by a lockfile update only.

#### DR-003 [EARS-U]: The git API adaptations keep behavior
> The 3 production sites SHALL treat the new `Result` returns the way they treated the old `Option` returns: a reference whose name is not valid UTF-8 is skipped (`crates/core/src/git/http/mod.rs:336`), a push update whose destination is not valid UTF-8 does not match (`crates/core/src/git/remote.rs:557`), and a branch shorthand that is not valid UTF-8 is treated as no branch (`crates/core/src/tools/git.rs:3086`).

#### DR-004 [EARS-O]: Blame without a signature
> IF a blame hunk carries no final signature THEN the blame tool SHALL report empty strings for its author, email and date; a hunk with a signature SHALL report the same three strings as before.
- Implemented by `fn blame_identity(sig: Option<&git2::Signature<'_>>) -> (String, String, String)` in `crates/core/src/tools/git.rs`, `snake_case`.

#### DR-005 [EARS-U]: Accepted advisories recorded once for cargo audit
> `.cargo/audit.toml` SHALL list, under `[advisories] ignore`, exactly RUSTSEC-2023-0071, RUSTSEC-2024-0384, RUSTSEC-2024-0436, RUSTSEC-2026-0192, RUSTSEC-2026-0253, RUSTSEC-2026-0002 and RUSTSEC-2024-0449, each with a one line reason comment naming the crate and why it stays (no upstream fix, or the backlog id that removes it), and the `security` target SHALL run `cargo audit` with no `--ignore` flag.
- **Old form:** `cargo audit --ignore RUSTSEC-2023-0071` (`Makefile:47`)
- **New form:** `cargo audit`

#### DR-006 [EARS-UB]: No stale cargo deny ignore
> `deny.toml` SHALL NOT list RUSTSEC-2026-0183 nor RUSTSEC-2026-0184, and SHALL keep RUSTSEC-2023-0071.

#### DR-007 [EARS-U]: Audit end state
> `cargo audit` SHALL exit 0 and report exactly 2 warnings, both of kind `yanked`, for `zip` 2.4.2 and `zip` 3.0.0; `cargo deny check` SHALL exit 0 with no `advisory-not-detected` warning.

### 4.2 Invariance
#### DR-008 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

**Rollback:** revert the commit; `git2` 0.20 and the Makefile flag come back together.

## 6. Decisions
- **DEC-001:** Bump git2 to 0.21 and adapt the 4 call sites with `.ok()` or `let Ok`. **Alternatives considered:** keep 0.20 and accept (rejected: the unsound `Remote::list` is called on the fetch path and a fix is released); wrap git2 behind an adapter (rejected: a new layer for 4 type changes). **Rationale:** the probe shows 1 lockfile package, 9 mechanical edits and 879 green git tests. **Implemented by:** DR-001, DR-003, DR-004. **Evidence:** Section 2 probe.
- **DEC-002:** Record acceptances in `.cargo/audit.toml`, the cargo audit configuration, and move the rsa ignore there so cargo audit has one source. **Alternatives considered:** more `--ignore` flags in the Makefile (rejected: no room for reasons, and split across two places); `deny.toml` only (rejected: cargo deny does not report these advisories under `unmaintained = "workspace"`, and listing them there triggers `advisory-not-detected`). **Implemented by:** DR-005. **Evidence:** probe in Section 2; `deny.toml:12-21`.
- **DEC-003:** Advisories with a fix path (lru via aws-sdk-s3, unsafe-libyaml) are recorded as accepted with their backlog id as reason, so the audit output shows only new or untracked findings. **Alternatives considered:** leave them visible as reminders (rejected: twelve standing warnings are why a new one would not stand out). **Implemented by:** DR-005. **Evidence:** n/a.
- **DEC-004:** The yanked `zip` warnings stay visible: cargo audit cannot ignore a yanked crate by advisory id, and they are tracked by BL-0058 and BL-0059. **Alternatives considered:** disable yanked checks (rejected: hides every future yank). **Implemented by:** DR-007. **Evidence:** n/a.
- **DEC-005:** Extract `blame_identity` so the no signature branch is unit tested. **Alternatives considered:** inline `Option` handling with no test (rejected: an untested branch); a `blame_buffer` integration test (rejected: production never calls it). **Implemented by:** DR-004. **Evidence:** `crates/core/src/tools/git.rs:2854` (only `blame_file`).
- **DEC-006:** Lockfile movement is checked by a diff in the story's definition of done, not a committed baseline fixture. **Alternatives considered:** a frozen list of package versions (rejected: stale on the next legitimate bump). **Implemented by:** DR-001, DR-002. **Evidence:** n/a.
- **DEC-007:** The new audit tests skip when `cargo-audit` or `cargo-deny` is not installed, like `make_security_passes` (`crates/mcp-fs/tests/security_gate.rs:22-26`). **Alternatives considered:** fail under `CI` (rejected: changes an existing test, out of scope). **Implemented by:** DR-005, DR-006, DR-007. **Evidence:** cited.
- **Assumption:** the advisory database is read live; an advisory published after this lot makes DT-012 red for an unrelated crate. That is the intended signal (a new backlog entry), not a scope change.
- **Assumption:** DT-012 also goes red when `zip` is fixed upstream (the count drops below 2). Intended: it prompts updating the expectation in the lot that closes BL-0058 or BL-0059.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the 4 rows of 8.4:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

Designed by an independent test designer subagent (2026-10-16), trimmed and approved by the owner. All tests close DBT-001 through the requirement each names.

### 8.2 Compatibility tests
None.

### 8.3 Structural and security tests

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| DT-010 | structural | DR-001 | `crates/mcp-fs/tests/security_gate.rs`, parses `Cargo.lock` text | the `[[package]]` entries named `git2` are exactly one, version starting `0.21.`; `libgit2-sys` version is `0.18.8+1.9.7` | the git2 version is `0.20.4` |
| DT-011 | structural | DR-001, DR-002 | story definition of done | `git diff main -- Cargo.lock` changes only the `git2` entry (version, checksum, its dependency lines) and the `yoke-derive` entry (version, checksum) | n/a (diff check) |
| DT-012 | security | DR-005, DR-007 | `security_gate.rs`, runs `cargo audit --json`, skips when not installed | exit 0; the JSON `warnings` hold exactly 2 entries, both kind `yanked`, packages `zip` 2.4.2 and `zip` 3.0.0 | 12 warnings, among them git2 RUSTSEC-2026-0183 and RUSTSEC-2026-0184 |
| DT-013 | security (second channel) | DR-001, DR-002, DR-007 | `security_gate.rs`, runs `cargo tree` | `cargo tree -i git2@0.20.4 --all-features` and `cargo tree -i yoke-derive@0.8.3 --all-features` both fail with `did not match`; `cargo tree -i git2@0.21.0 --all-features` succeeds | both old versions resolve, exit 0 |
| DT-014 | security | DR-006, DR-007 | `security_gate.rs`: `deny.toml` text, then `cargo deny check` (skips when not installed) | `deny.toml` contains `RUSTSEC-2023-0071` and neither `RUSTSEC-2026-0183` nor `RUSTSEC-2026-0184`; `cargo deny check` exits 0 and its output has no `advisory-not-detected` | `deny.toml:19-20` list both git2 ids |
| DT-015 | structural | DR-005 | `security_gate.rs`, text | `.cargo/audit.toml` exists, its `ignore` list holds exactly the 7 ids of DR-005, each line carrying a `#` comment; the `security:` recipe of `Makefile` contains `cargo audit` and no `--ignore` | file absent; `Makefile:47` has `--ignore RUSTSEC-2023-0071` |
| DT-016 | edge | DR-004 | `crates/core/src/tools/git.rs` tests | `blame_identity(None)` is `("", "", "")`; `blame_identity(Some(&Signature::new("Ada", "ada@x.test", &Time::new(1_700_000_000, 0)).unwrap()))` is `("Ada", "ada@x.test", "2023-11-14")` | written against a stub returning `("?", "?", "?")`, both assertions fail |
| DT-017 | behavior (pin) | DR-003, DR-008 | `crates/core/src/git/http/mod.rs` tests | a repository whose `packed-refs` holds `refs/heads/ok` and a ref whose raw name is `refs/heads/` followed by byte `0xff`: the ref advertisement lists `refs/heads/ok`, omits the other, and does not error | none: green before and after (invariance pin) |
| DT-018 | behavior (pin) | DR-003, DR-008 | `crates/core/src/tools/git.rs` tests, the `:3086` caller | HEAD symbolic to that non UTF-8 branch: the operation reports the same no branch outcome as a detached HEAD | none: invariance pin |
| DT-019 | guardrail | DR-008 | full suite | 8.1 green; `tool-contract-golden.json` unchanged | n/a |

No test exists for the push update site (`remote.rs:557`): its destination is built from our own UTF-8 string, so a non UTF-8 destination cannot reach it.

By category: structural 3, security 3, edge 1, behavior 2, guardrail 1.

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | `e2e_new_556_no_commit_object_on_a_conflicted_merge` (`:13737`) | `c.message().map(...)` to `c.message().ok().map(...)` | git2 0.21 returns `Result`; same value, compile only |
| `crates/core/src/tools/git.rs` | helper `tree_names` (`:18422`) | `e.name().map(String::from)` to `e.name().ok().map(String::from)` | same |
| `crates/core/src/tools/git.rs` | `e2e_new_876_a_second_member_continues_the_rebase` (`:20079-20080`) | `commit.committer().email()` to `commit.committer().email().ok()`, same for `author()`; asserted values `Some(SECOND)`, `Some(OWNER)` unchanged | same |
| `crates/core/src/tools/git.rs` | helper `bare_ref_names` (`:23062`) | `r.name().map(...)` to `r.name().ok().map(...)` | same |

### 8.5 Security tests
DT-012, DT-013 and DT-014 are the advisory tests: each is red before the change because the advisory is reported (first channel, the audit report) and the vulnerable version resolves in the dependency graph (second channel, `cargo tree`).

## 9. Implementation order
1. One story: the git2 bump and its 9 edits must land together (the workspace does not compile in between), and the audit configuration only reaches its end state once git2 and yoke-derive moved. Order inside the story: tests DT-010, DT-012..DT-016 written and red; `cargo update -p yoke-derive`; git2 bump with the 4 production edits and `blame_identity`; the 4 test adaptations; `.cargo/audit.toml`, `Makefile`, `deny.toml`; DT-017, DT-018 pins; 8.1.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 2 | IMPLEMENTABLE |

**Amendments applied:** A-1 DT-016 example unwraps `Signature::new` (it returns `Result`, `git2-0.21.0/src/signature.rs:47`); A-2 DEC-007 citation corrected to `security_gate.rs:22-26`.

**Gate process:** depth L. Self consistency reviewer (fresh subagent): PASS on every check, including the documented `Nature: DEBT` for the deferred dependency entries. Auditor (fresh subagent, path and contract only): every citation opened, the git2 0.20.4 and 0.21.0 sources diffed for each API named, `cargo audit` and `cargo deny` re run live; 0 F, 2 A amended above. Tests designed by an independent test designer subagent, trimmed and approved by the owner.
