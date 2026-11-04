# A test transport double for the tag legs the file remote cannot carry: technical debt design

> Id: SPEC-0078
> Nature: DEBT
> Depth: M
> Depth evidence: 4 modules (git test support, remote seam, tools tests, test fixtures), 10 tests, no public contract change
> Status: implemented
> Generated on: 2026-11-03
> Behavior change: none
> From backlog: BL-0098
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: five tag legs only pass on a wrong error with the right prefix
- **Current structure:** the tag tests reach a remote through `file://` (libgit2's local transport) or through `closing_remote`, a TCP listener that drops every connection (`crates/core/src/tools/git.rs:9809`). The comments of three tests say no file remote can answer 429 (`:26131-26132`, `:26692-26695`, `:29254`). The local transport runs no server hooks and refuses a shallow fetch (`crates/core/src/git/remote.rs:1091-1095`). The registered tool layer refuses anything but https (`remote.rs:246-258`, test `e2e_new_145` at `:2011`). So E2E-085 only exercises `Reference::is_valid_name` (`git.rs:27912`), E2E-039 and E2E-077 are simulated (`:26752`, `:27862`), and no test clones with `depth`.
- **Cost paid today:** a wrong error that keeps the right prefix passes the rate limited and rejected legs; a guard added only in the tool path is unseen by the engine level tests; the clone ancestry filter (`clone_history_tags`, `remote.rs:1542`) has no test with a shallow history.
- **Target structure:** a test only HTTP smart server (`git/test_remote.rs`) backed by the real `git` CLI that answers a chosen status, runs real server hooks, drops an answer after accepting a push, and allows shallow fetch; an https mode with a test certificate for the legs that go through the registered tool; ten new tests (Section 8).
- **Observable behavior:** unchanged. Production code gains one `#[cfg(test)]` certificate seam (DEC-004), compiled out of every non test build.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/git/test_remote.rs` | new, `#[cfg(test)]`: the double (HTTP and https) | 1 file |
| `crates/core/src/git/mod.rs` | `#[cfg(test)] mod test_remote;` next to `pub mod remote;` (`:37`) | 1 line |
| `crates/core/src/git/remote.rs` | `#[cfg(test)]` certificate seam in `credential_callbacks` (`:382`) | 1 site |
| `crates/core/src/tools/git.rs` | new tests DT-001 to DT-009 in its test module (9 tests); the existing tests are untouched | 9 tests |
| `crates/core/src/git/test_remote.rs` (tests) | DT-010, the structural test of the seam | 1 test |
| `crates/core/tests/fixtures/test-remote.pem`, `test-remote.key` | new: a self signed certificate for `127.0.0.1` and its key, test only | 2 files |
| `crates/core/Cargo.toml` | `tokio-rustls` as a `[dev-dependencies]` entry (already in `Cargo.lock` at 0.26.5, a transitive of sqlx and reqwest) | 1 line |
| `.agent_docs/testing.md` | one section: the double, what it can answer, how to add a leg | 1 |
| `sdlc/backlog/BL-0098-*.md` | deleted in the landing commit | 1 |

**Non source occurrences:** none outside the files above (`rg -n 'closing_remote|no file remote' --glob '!crates/**' .` finds no CI or Makefile use).
**Contracts touched:** none public. The seam is crate internal and `cfg(test)`.
**Guardrail coverage:** the whole git suite, among it `e2e_tag_032_remote_delete_down_rate_limited_credential_or_no_answer` (`git.rs:29263`), `e2e_tag_038_remote_unavailable_rate_limited_or_credential_expired` (`:26697`), `e2e_tag_022_publish_all_credential_or_answer_problems` (`:30971`), `e2e_tag_039_connection_lost_while_publishing` (`:26752`), `e2e_tag_039_remote_delete_paused_merge_and_a_wrong_commit` (`:29689`), `e2e_tag_077_connection_lost_after_the_remote_accepted_the_tag` (`:27862`), `e2e_tag_085_remote_refuses_the_tag_name` (`:27912`), which stay as they are (DEC-002). **Test command:** `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`
**Total:** 3 source files changed, 1 new source file, 2 fixtures, 1 manifest line, 1 doc.

## 3. Class sweep
Not applicable (`Security: n/a`). The certificate and key are test fixtures for `127.0.0.1`, not secrets.

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: one test transport
> The git test support SHALL provide one HTTP smart server, started per test on a loopback port, that serves a real bare repository through the `git` command line (`upload-pack` and `receive-pack` over the stateless protocol).
- **Old form:** tests reach a remote through `file://` or `closing_remote`.
- **New form:** `test_remote::Remote::start(repo_dir, Behavior)` returning the base URL (`http://127.0.0.1:<port>/repo.git`) and a handle that stops the server on drop.
- **Applies to:** `crates/core/src/git/test_remote.rs`, `git/mod.rs`.
- **Casing convention:** snake_case functions, `PascalCase` types.

#### DR-002 [EARS-E]: injected behaviors
> WHEN a test selects a `Behavior` THE double SHALL answer a request with that behavior and no other: `Rate(status)` answers every request with that HTTP status and the header `Retry-After: 1`; `DropAfterPush` forwards a push to the real `git receive-pack`, applies it, then closes the connection without sending the answer; `DropBeforePush` closes the connection when the push arrives, before `receive-pack` runs; `Normal` forwards everything.
#### DR-003 [EARS-E]: real hooks
> WHEN the served repository holds a `pre-receive` hook THE double SHALL run it through the real `git receive-pack`, so a rejection reaches the client as the server side rejection of the push.
#### DR-004 [EARS-O]: https mode
> IF a test starts the double with `Tls` THEN THE double SHALL serve https with the fixture certificate, and the registered tool path SHALL reach it with the `https://127.0.0.1:<port>/repo.git` URL.
#### DR-005 [EARS-UB]: no production behavior in a release build
> The production code SHALL NOT contain the certificate seam outside `#[cfg(test)]`.

### 4.2 Invariance
#### DR-006 [EARS-U]: behavior invariance
> The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract moves.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `test_remote` | test module | no | introduced | n/a | n/a |
| `credential_callbacks` test seam | cfg(test) code | no | introduced | n/a | n/a |

**Dual maintenance semantics:** none.

## 6. Decisions
- **DEC-001:** the double is an axum or hyper style server written with `tokio::net::TcpListener` and `tokio::process::Command`, one `git <service> --stateless-rpc` child per request; no new production dependency. **Alternatives considered:** `git http-backend` as a CGI (rejected: environment plumbing per request, harder to inject a status or a drop); a Rust git protocol implementation (rejected: the point is a real git on the server side); the project's own `/git/{mount_id}` server (rejected: it implements neither shallow fetch nor 429). **Rationale:** a real `git` gives real hooks and real shallow negotiation, as the SPEC-0067 harness already relies on `git upload-pack`. **Implemented by:** DR-001, DR-002, DR-003. **Evidence:** `crates/core/tests/git_negotiation_e2e.rs` (reference program).
- **DEC-002:** the simulated tests stay and the real ones are added next to them. **Alternatives considered:** replacing E2E-039 and E2E-077 (rejected by the owner, 2026-11-03: they stay as cheap guards). **Implemented by:** DR-006.
- **DEC-003:** https is a second story. **Alternatives considered:** one story (rejected: the TLS part may not work on every libgit2 TLS backend, and the plain HTTP legs must not wait on it). **Implemented by:** DR-004.
- **DEC-004:** a process wide test flag `TEST_ACCEPT_ANY_CERTIFICATE` read inside `credential_callbacks` under `#[cfg(test)]`, which installs `certificate_check(|_, _| Ok(CertificateCheckStatus::CertificateOk))`. **Alternatives considered:** `git2::opts::set_ssl_cert_file` with the fixture (rejected: libgit2's macOS HTTPS backend validates through the system trust store and ignores that option; ASSUMED, the RED run of DT-008 shows it); adding the fixture to the system keychain (rejected: a machine wide side effect). **Rationale:** test only, compiled out, and the engine functions otherwise unchanged. **Implemented by:** DR-004, DR-005. **Evidence:** `remote.rs:382`.
- **DEC-005:** `tokio-rustls` becomes a dev-dependency (no new crate in `Cargo.lock`); the certificate and key are committed fixtures generated once with `openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1`. **Alternatives considered:** `rcgen` (rejected: a new crate for a constant); generating at test time with the `openssl` CLI (rejected: a second tool on PATH). **Implemented by:** DR-004.
- **DEC-006:** the reason of a hook rejection (DT-004) is read in the RED run, not assumed. The `Rate` behavior answers 429 with `Retry-After: 1`; the error the client reports for it is read, not assumed, in the RED run, and the spec of the wording stays the one of SPEC-0052/E2E-038 and SPEC-0053/E2E-032. **Implemented by:** DR-002.
- **Assumption:** the SPEC-0052 and SPEC-0053 wording for a rate limited remote is the same text as a down remote with a different cause (`connection_error`, `remote.rs:1603`); DT-003 pins the actual text.

## 7. Declared breaks
None.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:
    export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace

### 8.2 Behavior tests
Red evidence: these are tests of behavior that exists, so they pass on arrival; each carries the named mutation that must turn it red.
#### DT-001: the double serves a fetch and a clone
- **Validates:** DR-001. Given a bare repository with `main` at `c1`, When a `git2` clone runs against `Remote::start(.., Normal)`, Then the clone holds `c1`. Mutation: the double answers 500, DT-001 fails.
#### DT-002: 429 is not a dropped connection
- **Validates:** DR-002. Given `Rate(429)`, When a fetch runs, Then it fails with a message that names the status and differs from the dropped connection message of `closing_remote`. Mutation: `Rate` closes the connection instead, DT-002 fails.
#### DT-003: rate limited remote delete and publish all
- **Validates:** DR-002. Given `Rate(429)`, When `git.remote_delete_tag` and publish all run through the engine functions, Then each fails with the wording of `connection_error` plus the status cause and the remote refs and audit are unchanged (the `e2e_tag_032` and `e2e_tag_022` legs).
#### DT-004: a real rejection of a tag push
- **Validates:** DR-003. Given a `pre-receive` hook that exits 1 with the line `tags are frozen`, When the tag `v1.0` is published, Then the refusal starts with `Remote refused tag 'v1.0': ` and its reason is the one read in the RED run (expected `pre-receive hook declined`, not assumed, DEC-006), and the remote has no `v1.0`. Mutation: the hook removed, DT-004 fails.
#### DT-005: clone with depth filters the tags by ancestry
- **Validates:** DR-001. Given a remote with five commits and tags `v1` on the first and `v5` on the last, When a clone runs with `depth` 2, Then only `v5` is brought in. Mutation: the filter removed, DT-005 fails.
#### DT-006: the answer is lost after the remote accepted the tag
- **Validates:** DR-002. Given `DropAfterPush`, When `v2.0` is published, Then the call fails with a connection error, the remote holds `v2.0`, and a retry against `Normal` reports an identical remote tag (E2E-077 made real).
#### DT-007: the connection drops before the tag arrives
- **Validates:** DR-002. Given `DropBeforePush`, When `v1.0` is published, Then the call fails and the remote holds no `v1.0` (E2E-039 made real).
#### DT-008: a paused merge remote delete through the registered tool, over https
- **Validates:** DR-004. The test builds its host map with `127.0.0.1` pointing at the double's port, as the existing registered tool tests do for their hosts. Given the double with `Tls`, a project whose pull paused on a merge conflict, When `git.remote_delete_tag` runs through its registered tool, Then it is refused as for any paused operation, HEAD and the conflict files equal their values before.
#### DT-009: the `github.ibm.com` host on the down leg
- **Validates:** DR-004. Given the host map entry `github.ibm.com` and the double stopped, When a remote tool runs, Then the refusal equals the `connection_error` text of `remote.rs:1603` for that host. The host map entry is built as in DT-008.

### 8.3 Structural tests
#### DT-010: the seam is test only
- **Asserts:** every line of `remote.rs` that names `TEST_ACCEPT_ANY_CERTIFICATE` is inside a `#[cfg(test)]` item, and `test_remote` is declared under `#[cfg(test)]`. **Method:** `include_str!` of `remote.rs` and `git/mod.rs`, checking the preceding attribute. Covers DR-005.

### 8.4 Existing tests to modify
None.

## 9. Implementation order
1. Story A (plain HTTP): `test_remote.rs` with `Normal`, `Rate`, `DropAfterPush`, `DropBeforePush`, real hooks; DT-001 to DT-007; `.agent_docs/testing.md`. Green after this step.
2. Story B (https): the seam, the fixtures, the dev-dependency, `Tls` mode; DT-008, DT-009, DT-010. If the https path cannot be made to work on the development machine, the story stops and records what was tried (no weakened test).

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 4 (all MINOR, all amended) | IMPLEMENTABLE |

**Amendments applied:** the hook reason read in the RED run (DT-004, DEC-006), the perimeter row of DT-001 to DT-010, the host map of DT-008 and DT-009 with the expected wording of DT-009, the citations `remote.rs:1603` and `:2011` and the named guardrail tests. Auditor: a general purpose subagent of the same model family, no F finding.

## Design additions (2026-11-03)
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`); story B is `min_tier` 1 (DEC-007). Skills loaded: rust.

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/test_remote.rs` | new (`Remote`, `Behavior`, the plain HTTP server, the real git services) | DR-001, DR-002, DR-003 | US-0157 |
| `crates/core/src/git/mod.rs` | modify (`#[cfg(test)] mod test_remote;`) | DR-001 | US-0157 |
| `crates/core/src/tools/git.rs` | modify (tests DT-001 to DT-007, then DT-008 and DT-009) | all | US-0157, US-0158 |
| `.agent_docs/testing.md` | modify (the double, its behaviors, how to add a leg) | DR-001 | US-0157 |
| `crates/core/src/git/remote.rs` | modify (the `#[cfg(test)]` certificate seam; DT-010 in its test module) | DR-004, DR-005 | US-0158 |
| `crates/core/src/git/test_remote.rs` | modify (the `Tls` mode) | DR-004 | US-0158 |
| `crates/core/Cargo.toml` | modify (`tokio-rustls` dev-dependency) | DR-004 | US-0158 |
| `crates/core/tests/fixtures/test-remote.pem`, `test-remote.key` | new | DR-004 | US-0158 |
Affected existing tests: keep all (DEC-002). Documentation: `.agent_docs/testing.md`.

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001 to DT-007 | integration | the engine functions (`fetch_from_remote`, `delete_tag_on_remote`, `push_tag_to_remote`, `push_tags_to_remote`, clone) against `test_remote::Remote` over `http://127.0.0.1:<port>/repo.git` | `crates/core/src/tools/git.rs` tests | US-0157 |
| DT-008, DT-009 | functional e2e | the registered tools over `https://127.0.0.1:<port>/repo.git` | `crates/core/src/tools/git.rs` tests | US-0158 |
| DT-010 | structural | `include_str!` of `remote.rs` and `git/mod.rs` | `crates/core/src/git/remote.rs` tests | US-0158 |
Test data: generated bare repositories in the test's temporary directory; one committed certificate and key.

### Decisions (continuing)
- **DEC-007:** two stories. US-0157 (plain HTTP: the server, the behaviors, the real hooks, seven tests, the doc) touches 4 files at tier 2. US-0158 (https: the seam, the fixtures, the dev-dependency, the `Tls` mode, three tests) touches 7 files and rests on an unverified TLS backend behavior (DEC-004), so it is `min_tier` 1: the density of decisions, not the file count, sets it. **Obliged by:** n/a (no code impact): partitioning only. **Source:** design.
- **DEC-008:** the double picks a free loopback port (`127.0.0.1:0`) per test and serves until its handle is dropped; no fixed port, so tests run in parallel. **Obliged by:** DR-001. **Source:** existing practice `git.rs:9809-9818` (`closing_remote` binds port 0).
- **DEC-009:** a request to `git upload-pack` or `git receive-pack` runs with the bare repository directory as its working directory and `GIT_HTTP_EXPORT_ALL` unset; the service names come from the `service` query of `info/refs` and the path of the POST. **Obliged by:** DR-001. **Source:** design.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0157 | add a test transport that answers like a real git server | P1 | DR-001, DR-002, DR-003, DR-006 | main | 2 | no |
| US-0158 | serve the test transport over https | P1 | DR-004, DR-005, DR-006 | US-0157 | 1 | yes |
