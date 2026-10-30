# Tag test gaps: technical debt design

> Id: SPEC-0060
> Nature: DEBT
> Depth: M
> Depth evidence: 1 module (the tests module of `crates/core/src/tools/git.rs`), 1 source file changed, no public contract, no persisted change; about 12 requirements.
> Status: designed
> Generated on: 2026-10-30
> Behavior change: none
> From backlog: BL-0081, BL-0082, BL-0083, BL-0086, BL-0087, BL-0094 (partly each; the rest is carried by BL-0098 to BL-0107)
> Depends on: SPEC-0052, SPEC-0053, SPEC-0054 (read only)
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: unreleased

## 1. Debt

### DBT-001: tag tests that would pass on a regression
- **Current structure:** seven spots in the tag test suites assert less than the specs promise.
  - No test deletes an annotated tag; every `untag` target is plain (`crates/core/src/tools/git.rs:25282,25308,25334,25365,25397,25405`). `tag_delete` returns the peeled commit as `sha` and audits it (`crates/core/src/tools/git_tags.rs:192-225`).
  - The two concurrency tests race with `tokio::join!` (`git.rs:25355`, `git.rs:25404`) while `tag_delete` takes `entry.write_lock` before its existence check (`git_tags.rs:200-202`), so a check moved outside the lock could still pass by luck.
  - `e2e_tag_048_two_thousand_tags` (`git.rs:25582`) makes 2000 tags of which 500 are annotated, and `TAG_REFSPEC_BATCH` is 500 (`crates/core/src/git/remote.rs:1336`, used at `:1463`): one batch, the loop never iterates twice.
  - No test names a blank remote (`e2e_tag_049_remote_delete_empty_or_blank_name_is_absent`, `git.rs:27283`, loops over blank TAG names only). The code matches the remote name exactly (`git_tags.rs:1274`).
  - `e2e_tag_029_remote_delete_during_a_paused_operation` (`git.rs:27758`) compares only `op_row()`; the guard it replaces compares only the operation row too (`git.rs:4431-4443`).
  - `pause_with_due_tag` (`git.rs:29320`) has merge, rebase and a default cherry pick arm; `assert_publish_during` is called for those three only (`git.rs:29367-29379`). SPEC-0054 FR-NEW-014 names revert as well.
  - Connection failure legs assert the prefix `fetch from host '<h>' failed: ` and not the reason: `e2e_tag_032`, `033`, `034` remote delete (`git.rs:27474`, `27542`, `27564`), and `assert_connection_refused` (`git.rs:29080`) used by publish all `e2e_tag_020` (`29087`), `029` (`29240`), `050` (`29423`).
- **Cost paid today:** a wrong error following the right prefix, a lock check outside the lock, a lost second batch, or a paused revert regression would keep the suite green.
- **Target structure:** each spot has a test (new, or an existing one with stronger assertions) that fails when the guarded behavior is removed.
- **Observable behavior:** unchanged. No production line changes.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/tools/git.rs` (tests module only) | 5 new tests (DR-001, 002, 003, 004, 006); 8 existing tests get extra assertions; `pause_with_due_tag` gets a `revert` arm; `assert_connection_refused` gains a parameter | 13 test functions, 2 helpers |
| `sdlc/backlog/` | 6 entries deleted, 10 residual entries created (Section 6, DEC-004) | 16 files |

Production code touched: none. `crates/core/src/tools/git_tags.rs` and `crates/core/src/git/remote.rs` are read only. The repository write lock is reachable from a test through `e.entry().await.write_lock` (precedent: `e2e_tag_035_publish_all_two_members_confirm_at_once`, `git.rs:29254-29262`).

**Non source occurrences:** none (no `.env.example`, CI, Makefile, container or manifest line names these tests; `.agent_docs/testing.md` lists suites, not individual tests).
**Contracts touched:** none. Internal tests only.
**Guardrail coverage:** `rg -c "fn e2e_tag_" crates/core/src/tools/git.rs` = 207 tests. **Test command:** `cargo test -p mcp-fs-core --lib e2e_tag`
**Measured on 2026-10-30** (3 runs for the first, 1 for the second, same machine):
- a dropped connection from `closing_remote` yields `fetch from host '127.0.0.1' failed: SSL error: error:0A000126:SSL routines::unexpected eof while reading; class=Ssl (16)`;
- a missing `file://` path yields `fetch from host 'file://<path>' failed: failed to resolve path '<path>': No such file or directory; class=Os (2)`.
**Total:** 1 source file, 15 code spots (13 tests, 2 helpers), all recounted. `assert_connection_refused` has 4 callers: `git.rs:29096,29177,29248,29449`.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)

### 4.1 Structural

#### DR-001 [EARS-U]: annotated tag deletion is tested
> The tag test suite SHALL contain a test that creates an annotated tag with a message, deletes it through `git.tag_delete`, and asserts the result `sha` is the commit the tag points at (not the tag object id) and that the one `git.tag_delete` audit line reads `deleted tag '<name>' (was at <commit sha>)`.
- **Applies to:** new test next to `e2e_tag_026_delete_a_local_tag` (`git.rs:25218`).

#### DR-002 [EARS-U]: local deletion lock is tested
> The tag test suite SHALL contain a test that holds the repository write lock, starts a `git.tag_delete` of an existing tag, asserts after a wait of 150 ms (as at `git.rs:27790`) that the tag still exists and the call has not returned, releases the lock, and asserts the call then succeeds and the tag is gone. The project has no paused operation, otherwise the pause guard (`git_tags.rs:72`) refuses before the lock.
- **Applies to:** new test next to `e2e_tag_032_concurrent_deletion` (`git.rs:25355`). Lock handle: `e.git.get_or_open_repo(MOUNT).await.unwrap().write_lock.lock().await`, as at `git.rs:27786-27787` and `29262`. Use `tokio::select!` over the call future and the 150 ms sleep, as `e2e_tag_030` does.

#### DR-003 [EARS-U]: tag reception spans several batches
> The tag test suite SHALL contain a test where a remote holds 501 annotated tags on at least 3 distinct commits, a fetch is run, and every one of the 501 tags is received with its message and commit.
- **Applies to:** new test next to `e2e_tag_048_two_thousand_tags` (`git.rs:25582`). 501 is `TAG_REFSPEC_BATCH` plus one, so the batching loop runs twice (`remote.rs:1336,1463`).

#### DR-004 [EARS-U]: blank remote name is tested
> The tag test suite SHALL contain a test that calls `git.remote_delete_tag` with the `remote` argument `""`, and again with `"   "`, on a project that tracks `origin`, and asserts each call is refused with the remote name shown as given, no remote tag removed, and no audit line written.
- **Old form:** no such test. **Governed by:** SPEC-0053 FR-NEW-014 and DEC-011 (a blank text is a given name). The exact refusal text is whatever `git_tags.rs:1274` area already produces for an untracked remote; the test reads it from the code via the existing `untracked_text` helper used by `e2e_tag_044_remote_delete_named_remote_not_tracked` (`git.rs:27630`).

#### DR-005 [EARS-U]: paused remote deletion proves nothing moved
> `e2e_tag_029_remote_delete_during_a_paused_operation` SHALL, for each of its four operation kinds, also assert equality before and after the deletion of: (a) the HEAD commit read with `e.ref_sha("refs/heads/main")`; (b) the full list `e.entry().await.db.list_refs()`; (c) the content, read with `e.read(path)`, of every path in the paused operation's conflict list, which is the JSON array of path strings held in the `conflicts` field of `e.op_row()` (for example `["/a.txt"]`, `git/db.rs:565`, written by `encode_op_json` at `tools/git.rs:4951`); all in addition to the existing `op_row()` comparison. It SHALL NOT assert anything about index entries.
- **Applies to:** `git.rs:27758`. Governed by SPEC-0053 FR-NEW-010 and DEC-009: the deletion does not call the pause guard, so it must also not touch the paused state.

#### DR-006 [EARS-U]: paused revert is tested for publish all
> `pause_with_due_tag` SHALL accept the kind `revert` (a paused revert with a conflict and one due tag), and the test suite SHALL contain `assert_publish_during("revert")` asserting what the three existing kinds assert: line `v1.0 published`, the remote holds the tag, the operation row unchanged, one audit line.
- **Applies to:** `git.rs:29320` and a new test after `e2e_tag_046` (publish all, cherry pick). The harness already produces a paused revert (`pause_operation` with `revert`, `git.rs:27733`).

#### DR-007 [EARS-U]: remote delete failure legs assert the reason
> `e2e_tag_032`, `e2e_tag_033` and `e2e_tag_034` (remote delete) SHALL assert, after the prefix, the transport reason: `No such file or directory` for the missing `file://` path leg, and `unexpected eof while reading` for each `closing_remote` leg.
- **Applies to:** `git.rs:27474,27542,27564`. The reason strings are measured (Section 2). Assertion is `contains`, not equality, because the OpenSSL error code and the temp path vary by machine.
- **Honest limit:** the `closing_remote` leg of 032 is labeled "rate limited" and still cannot tell a rate limit from a drop (BL-0098).

#### DR-008 [EARS-U]: publish all failure legs assert the reason
> `assert_connection_refused` SHALL take the expected reason and assert it after the prefix, and `e2e_tag_020`, `e2e_tag_029` and `e2e_tag_050` (publish all) SHALL pass `No such file or directory`. `e2e_tag_021` already asserts `connection reset` (`git.rs:29104`) and `e2e_tag_022` keeps its `closing_remote` leg with `unexpected eof while reading`.
- **Applies to:** `git.rs:29080,29087,29137,29240,29423`.

### 4.2 Invariance

#### DR-009 [EARS-U]: behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; no line outside `#[cfg(test)]` code is modified.

### 4.3 Compatibility
No public contract is touched: no compatibility requirement.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

## 6. Decisions
- **DEC-001:** only tests, no production change. **Alternatives:** (a) fix the two wording gaps now (BL-0083 connection wording, BL-0093), rejected: they change what an operator sees, so they are FEAT (SPEC C). (b) add a test seam for 429 and hook rejection, rejected: a new seam is a bigger move than this lot. **Rationale:** the lot stays pure debt. **Implemented by:** DR-009. **Evidence:** `git.rs:25536,26062,27467`, `remote.rs:997-999`.
- **DEC-002:** `contains` on the transport reason, not equality. **Alternatives:** equality (rejected: the temp path and the OpenSSL code differ per machine, measured Section 2); prefix only (rejected: it is the debt). The reasons were measured with the bundled OpenSSL libgit2 backend; if the TLS backend changes, re-measure. **Implemented by:** DR-007, DR-008.
- **DEC-003:** lock proofs hold the lock from the test and bound the wait, not a looped race. **Alternatives:** repeat the race N times (rejected: still probabilistic, and the repository already uses the deterministic form at `git.rs:29254-29262`). **Implemented by:** DR-002.
- **DEC-004:** the six source entries are deleted and every bullet this lot does not carry is rewritten as its own entry, one idea each (invariant "never batch ideas"). **Alternatives:** keep the six entries open with a note (rejected: the contract deletes the consumed entry in the same commit). **Implemented by:** n/a, no code impact. Carried bullets:
  - BL-0098 (DEBT): a test transport double that answers 429, enforces hooks, serves https to the registered tool, and allows a shallow fetch. Carries: rate limit legs (BL-0086, BL-0094), real remote rejection test and simulations E2E-039/077 (BL-0083), registered tool paused test (BL-0087), clone with depth (BL-0082), spec example host `github.ibm.com` on the down leg (BL-0086).
  - BL-0099 (FEAT): which branch's tags a fetch receives. From BL-0082.
  - BL-0100 (FEAT): a pull whose merge fails after the tag reconcile loses `tags_added`; an empty incoming list still runs `list_refs`. From BL-0082.
  - BL-0101 (DEBT): clone hydrates the whole repository before the tag reconcile. From BL-0082.
  - BL-0102 (DEBT): `reconcile_incoming_tags` writes one `set_ref` await per tag. From BL-0082.
  - BL-0103 (FEAT): `commit_on_remote` counts every advertised ref, DEC-020 says branches. From BL-0083.
  - BL-0104 (FEAT): connection error wording on tag publish reads `fetch from host`. From BL-0083.
  - BL-0105 (FEAT): `tag_create` and `reconcile_incoming_tags` normalise `a//b` silently. From BL-0083.
  - BL-0106 (FEAT): a remote tag deleted between listing and push reads `has moved to 0000000`. From BL-0083.
  - BL-0107 (DEBT): the annotated message probe is a full single tag fetch, not the shallow fetch of DEC-017. From BL-0083.
- **DEC-005:** BL-0087 asked for a remote delete lock proof, dropped. `e2e_tag_030_remote_delete_two_members_at_once` already holds the write lock and asserts both calls wait (`git.rs:27786-27800`), so the entry's claim that it would not catch a missing lock is wrong. **Alternatives:** add a duplicate (rejected: no new signal). **Implemented by:** n/a, no code impact. **Evidence:** `git.rs:27780-27800`.
- **Assumption:** the 501 tag test runs in the same order of time as `e2e_tag_048` (2000 tags); not measured. If it exceeds 10 seconds the implementer reports it.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified** in every assertion it makes today (8 tests only gain assertions, Section 8.4):

    cargo test -p mcp-fs-core --lib e2e_tag
    cargo test --workspace
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt --all -- --check

### 8.2 Compatibility tests
None (no contract).

### 8.3 Structural tests

#### DT-001: mutation proof for the new tests
- **Validates:** DR-001 to DR-008
- **Method:** for each requirement, apply the mutation below to production code, run the named test, see it fail with the quoted reason, revert (`git checkout`). The implementer's report lists the command and the failing assertion per row.

| Req | Mutation | Expected failing assertion |
|---|---|---|
| DR-001 | in `tag_delete`, report the row target instead of the peeled commit (`git_tags.rs:210,225`) | `sha` equals the tag object id, not the commit |
| DR-002 | move the `get_ref` check and delete before `write_lock.lock()` (`git_tags.rs:200`) | the call returns while the lock is held |
| DR-003 | set `TAG_REFSPEC_BATCH` loop to process only the first chunk (`remote.rs:1463`) | fewer than 501 tags received |
| DR-004 | treat a blank `a.remote` as `origin` (`git_tags.rs:1272-1274`) | the call is not refused with `does not track a remote named ''` |
| DR-005 | make `delete_remote_tag` call `reject_if_operation_in_progress` or touch HEAD | HEAD/refs/conflict comparison differs |
| DR-006 | make publish all call `reject_if_operation_in_progress` | publish refused during the paused revert |
| DR-007, DR-008 | replace the transport message with a fixed string after the prefix in `connection_error` | reason substring missing |

#### DT-002: no production line changed
- **Asserts:** the diff outside the tests module is empty.
- **Method:** `git diff main -- crates | rg '^[+-]' ` restricted to lines before the tests module start (the line of `#[cfg(test)]` in `git.rs`) returns nothing; `git diff main --stat -- crates` lists only `crates/core/src/tools/git.rs`.

#### DT-003: residual backlog entries exist
- **Asserts:** `sdlc/backlog/BL-0098` to `BL-0107` exist, each with `Nature:` and an `Evidence:` section; `BL-0081`, `BL-0082`, `BL-0083`, `BL-0086`, `BL-0087`, `BL-0094` no longer exist.
- **Method:** `ls sdlc/backlog | rg 'BL-(009[89]|010[0-7])'` returns 10 lines; `ls sdlc/backlog | rg 'BL-00(81|82|83|86|87|94)'` returns nothing.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | `e2e_tag_029_remote_delete_during_a_paused_operation` | adds HEAD, ref list, conflict comparison | DR-005; adds, removes nothing |
| same | `e2e_tag_032_remote_delete_down_rate_limited_credential_or_no_answer` | adds reason `contains` on 2 legs | DR-007 |
| same | `e2e_tag_033_remote_delete_connection_lost_before_the_deletion` | adds reason `contains` | DR-007 |
| same | `e2e_tag_034_remote_delete_connection_failure_outranks_tag_checks` | adds reason `contains` | DR-007 |
| same | `e2e_tag_020_publish_all_the_remote_is_unreachable` | passes the reason to `assert_connection_refused` | DR-008 |
| same | `e2e_tag_029_publish_all_a_connection_failure_beats_a_wrong_number` | same | DR-008 |
| same | `e2e_tag_050_publish_all_a_word_instead_of_a_number` | same | DR-008 |
| same | `e2e_tag_022_publish_all_credential_or_answer_problems` | passes `unexpected eof while reading` to `assert_connection_refused` (`git.rs:29177`) | DR-008; the helper's signature change forces it |

Plus the helpers `assert_connection_refused` (signature gains the reason; its 4 callers are 020, 022, 029, 050) and `pause_with_due_tag` (gains an arm). `e2e_tag_021` does not call the helper and is not touched.

### 8.5 Security tests
Not applicable.

## 9. Implementation order
1. Add DR-006's `revert` arm and test, DR-004, DR-003, DR-001 (additions only; suite stays green).
2. Add DR-002 (lock proof; green).
3. Strengthen DR-005, then DR-007 and DR-008 together (the helper signature change and its 4 callers in one step so the build stays green).
4. Run DT-001 mutation proofs one at a time; revert each.
5. Run the Section 8.1 commands and DT-002.
6. In the same commit as this document: create BL-0098 to BL-0107 and delete BL-0081, 0082, 0083, 0086, 0087, 0094 (DT-003). Not done until then.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 2 | 4 | NOT-IMPLEMENTABLE |
| 2 | 0 | 4 (MINOR) | IMPLEMENTABLE |

**Amendments applied:** round 1: DR-006 rewritten with observable comparisons; `e2e_tag_022` added to Section 8.4 and counts corrected; DR-005 (remote delete lock proof) dropped as already covered by `e2e_tag_030` (DEC-005); DT-001 mutation for DR-004 corrected; DR-002 wait and lock handle specified. Round 2: DEC-002 note on the TLS backend; gate table filled. Unopened citations (`git_tags.rs:72`, `git.rs:4431-4443`) were opened by the author in round 1 evidence.

## Design additions (2026-10-30)

Target tier: 2 (default, no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`). Skill loaded: `rust` (project gate is `make check`; the repository's own quality gate in `AGENTS.md` is `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`).

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git.rs` (tests module) | modify: add tests, assertions, helper arm | DR-001 to DR-008 | US-0111 to US-0115 |
| `sdlc/backlog/BL-0098` to `BL-0107` | created in SPEC-0060 commit | DT-003 | US-0115 verifies |
| `.agent_docs/testing.md` | no change (lists suites, not tests) | n/a | none |

Affected existing tests: Section 8.4 (8 modify, 0 remove). Affected documentation: none.

### Test strategy
All tests live in the in-crate test module, level integration (in-process server against bare `file://` remotes and TCP stubs), driver: direct tool call through the `Env` harness.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DR-001 test | integration | `untag_as` / `tag` | `git.rs` near `e2e_tag_026_delete_a_local_tag` | US-0111 |
| DR-002 test | integration | `untag_as` + held `write_lock` | `git.rs` near `e2e_tag_032_concurrent_deletion` | US-0111 |
| DR-003 test | integration | `call_fetch_branch` | `git.rs` near `e2e_tag_048_two_thousand_tags` | US-0112 |
| DR-004 test | integration | `delete_by_tool` | `git.rs` near `e2e_tag_049_remote_delete_empty_or_blank_name_is_absent` | US-0112 |
| DR-006 test | integration | `confirm_all` | `git.rs` after `e2e_tag_046` publish all | US-0113 |
| DT-001 mutation rows | mutation proof | manual mutation, `cargo test` | production files, reverted | US-0115 |
| DT-002, DT-003 | structural | `git diff`, `ls` | repository | US-0115 |

DR-005, DR-007, DR-008 are assertions in existing tests (US-0114, US-0115). Test data: generated by the harness; none user provided.

### Decisions (continued)
- **DEC-006:** five stories in the order of Section 9, serialized (each depends on the previous) because all five edit the same test module. **Obliged by:** DR-009 (invariance, one writer at a time keeps the suite green). **Rationale:** parallel stories on one file would conflict on every merge. **Alternatives considered:** one story for the whole lot (rejected: 13 test functions and 2 helpers with 3 distinct themes, too wide for a tier 2 review); one story per requirement (rejected: tier 3 shape, more re read context). **Source:** design. **Evidence:** `git.rs` is the only file in Section 2.
- **DEC-007:** the mutation proofs (DT-001) are carried out by each story for its own requirement as RED evidence, and the closing story re runs the whole table and records the commands. **Obliged by:** DR-001 to DR-008. **Rationale:** a mutation proven late is a red test found late. **Alternatives considered:** all proofs in the closing story only (rejected). **Source:** design. **Evidence:** n/a.
- **DEC-008:** no `Tier` override; every story `min_tier` 2: each needs reading a harness of 29k lines to find helpers (`Env`, `closing_remote`, `pause_operation`), which tier 3 and 4 budgets cannot hold. **Obliged by:** n/a (no code impact): routing choice. **Source:** design.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0111 | test the deletion of an annotated tag and its lock | P1 | DR-001, DR-002 | main | 2 | no |
| US-0112 | test a fetch of 501 annotated tags and a blank remote name | P1 | DR-003, DR-004 | US-0111 | 2 | no |
| US-0113 | test publish all during a paused revert | P1 | DR-006 | US-0112 | 2 | no |
| US-0114 | prove a remote delete leaves a paused operation untouched | P2 | DR-005 | US-0113 | 2 | no |
| US-0115 | assert the transport reason on connection failure tests and close the spec | P2 | DR-007, DR-008, DR-009 | US-0114 | 2 | yes |
