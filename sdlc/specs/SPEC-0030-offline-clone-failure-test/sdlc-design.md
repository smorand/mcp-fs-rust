# Clone failure test runs offline against a loopback peer: technical debt design

> Id: SPEC-0030
> Nature: DEBT
> Depth: S
> Depth evidence: one debt item, one file (`crates/core/src/tools/git.rs`, test module only), no public contract, no persisted data
> Status: specified
> Generated on: 2026-10-14
> Behavior change: none
> From backlog: BL-0045
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt

### DBT-001: One unit test needs the internet
- **Current structure:** `tools::git::tests::remote_clone_surfaces_a_failure_without_leaking_the_token` (`crates/core/src/tools/git.rs:8462`) declares `github.com` as `github` (`:8467`), stores a token for it (`:8468-8480`, host at `:8471`) and clones `https://github.com/does-not/exist-mcpfs.git` (`:8483`), a real outbound HTTPS connection.
- **Cost paid today:** on a slow network the clone stalls and the default remote deadline answers `remote timeout: host 'github.com' did not respond within 120s` (`crates/core/src/git/remote.rs:808-810`) instead of `clone failed:`, so `make check` goes red with no code change (BL-0045, observed at the SPEC-0020 landing) and cannot run offline.
- **Target structure:** the test clones from a loopback listener that accepts then immediately drops each connection; libgit2 fails the TLS handshake at once, which `clone_to_temp` maps to `clone failed: ...` (`crates/core/src/git/remote.rs:428-435`).
- **Observable behavior:** unchanged (test code only).

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/tools/git.rs` (test module) | add helper `closing_remote()` beside `silent_remote()` (`:9336-9346`); in the test, replace the `github.com` host, token host and URL with the loopback host and URL | 1 test, 4 code lines naming `github.com` (`:8467`, `:8471`, `:8483`, `:8492`) plus the comment at `:8463` |

**Non source occurrences:** none (`rg -n "github.com/does-not/exist-mcpfs"` finds 1 hit, `git.rs:8483`).
**Contracts touched:** none public; the test helpers are private to the `tests` module.
**Sweep of other network reaching tests** (`rg -n '"url": *"https?://(?!127\.0\.0\.1|localhost|\{)' -P crates/`): 13 other hits in `git.rs` (`:7805`, `:7809`, `:8504`, `:8551`, `:8951`, `:8972`, `:9015`, `:9056`, `:9077`, `:9097`, `:9140`, `:9166`, `:20199`), every one asserting a refusal before any network step (`FORBIDDEN`, `UNAUTHENTICATED`, `INVALID_ARGUMENT`, or a routing error). `bt_014_web_fetch_is_callable_through_mcp` already uses a loopback stub; no test calls web search or context7 tools. This test is the only one in scope.
**Guardrail coverage:** the test itself plus the full suite. **Test command:** see 8.1.
**Total:** 1 file, 1 test, recounted.

## 3. Class sweep
Not a security lot.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-UB]: No internet in the test
> The test `remote_clone_surfaces_a_failure_without_leaking_the_token` SHALL NOT open a connection to any host other than `127.0.0.1`.
- **Old form:** host `"github.com"` (declared `github`, token host), URL `"https://github.com/does-not/exist-mcpfs.git"`
- **New form:** host `"127.0.0.1"` (declared `github`, token host), URL `format!("https://127.0.0.1:{port}/does-not/exist-mcpfs.git")` from `closing_remote()`
- **Applies to:** the test body and the new helper
- **Casing convention:** snake_case Rust identifiers, as in `silent_remote`

#### DR-003 [EARS-U]: Same assertions
> The test SHALL keep its four assertions unchanged: `err.code == code::INTERNAL_ERROR`, `err.message.starts_with("clone failed:")`, the message does not contain `gho_supersecret`, and the stored token still reads `gho_supersecret`.

### 4.2 Invariance
#### DR-002 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

## 6. Decisions
- **DEC-001:** A loopback listener that accepts and drops the connection (`closing_remote()`, a sibling of `silent_remote()`, the stream dropped right after `accept`). **Alternatives considered:** an unroutable address such as `10.255.255.1`, rejected because it hangs until the deadline and yields `remote timeout`, not `clone failed:`; reusing `silent_remote()`, rejected for the same reason; a full stub git HTTPS server, rejected as speculative for a failure path. **Rationale:** fails instantly, offline, through the real libgit2 transport. **Implemented by:** DR-001, DR-003. **Evidence:** `crates/core/src/tools/git.rs:9336-9346`, `crates/core/src/git/remote.rs:428-435`.
- **DEC-002:** Keep provider `github` for host `127.0.0.1`. **Alternatives considered:** `generic` as in `e2e_new_defect_token_set_then_remote_clone_share_the_real_host` (`git.rs:8799`), rejected because the test is about a stored provider token not leaking, and the provider is free text validated only by `Provider::parse` (`crates/core/src/git/remote.rs:159`). **Implemented by:** DR-001. **Evidence:** `crates/core/src/git/remote.rs:155-175`.
- **Assumption:** if libgit2 reports the dropped connection through a path that does not reach `clone_to_temp`'s `map_err`, the `clone failed:` assertion goes red during implementation; that is a story block, not a scope change.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the row in 8.4:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None.

### 8.3 Structural tests
#### DT-010: The external URL is gone
- **Asserts:** `rg -n "github.com/does-not/exist-mcpfs" crates` prints nothing (today: 1 hit, `git.rs:8483`).
- **Method:** grep assertion.

#### DT-011: The test is fast and offline
- **Asserts:** `cargo test -p mcp-fs-core --lib tools::git::tests::remote_clone_surfaces_a_failure_without_leaking_the_token` prints `test result: ok. 1 passed` and `finished in` under 5 s; the test body contains no host other than `127.0.0.1`.
- **Method:** command output plus grep on the function body.

#### DT-012: Assertions unchanged
- **Asserts:** the four assertion lines of DR-003 appear byte identical in `git diff` context (no `-`/`+` on them).
- **Method:** diff inspection.

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | `remote_clone_surfaces_a_failure_without_leaking_the_token` | setup only: target host and URL move to the loopback peer; assertions untouched | the test is the debt (BL-0045): it needs the internet and flakes to `remote timeout` |

## 9. Implementation order
1. Add `closing_remote()` next to `silent_remote()` (unused helper, suite green).
2. Switch the test to it; run DT-011, DT-010, DT-012, then 8.1.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 1 | IMPLEMENTABLE |

Inline checklist (depth S): 1 PASS, 2 PASS (both forms spelled), 3 n/a, 4 PASS, 5 n/a, 6 PASS, 7 PASS, 8 PASS (test only, no behavior change), 9 PASS (lines recounted), 10 PASS (`map_err` path cited, assumption stated), 11 PASS, 12 PASS (not security), 13 n/a, 14 PASS (one declared test change).

**Amendments applied:** A (citations): fn line 8462, host lines 8467, 8471, 8492, generic Env line 8799, comment line 8463 added to the perimeter.
