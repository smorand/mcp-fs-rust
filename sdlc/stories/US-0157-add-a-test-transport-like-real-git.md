# US-0157: add a test transport that answers like a real git server

> Spec: SPEC-0078 (sdlc/specs/SPEC-0078-tag-test-transport-double/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
Five tag legs cannot be tested because the only test remotes are `file://` and a listener that drops every connection. Add a test only HTTP smart server backed by the real `git`, with injectable behaviors and real hooks, and use it for seven legs.

## Requirements covered
- SPEC-0078/DR-001: The git test support SHALL provide one HTTP smart server, started per test on a loopback port, that serves a real bare repository through the `git` command line (`upload-pack` and `receive-pack` over the stateless protocol).
- SPEC-0078/DR-002: WHEN a test selects a `Behavior` THE double SHALL answer a request with that behavior and no other: `Rate(status)` answers every request with that HTTP status and the header `Retry-After: 1`; `DropAfterPush` forwards a push to the real `git receive-pack`, applies it, then closes the connection without sending the answer; `DropBeforePush` closes the connection when the push arrives, before `receive-pack` runs; `Normal` forwards everything.
- SPEC-0078/DR-003: WHEN the served repository holds a `pre-receive` hook THE double SHALL run it through the real `git receive-pack`, so a rejection reaches the client as the server side rejection of the push.
- SPEC-0078/DR-006 (partial: no existing test changes in this story): The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib dt_0078` is green; with the double answering 500, DT-001 fails; with `Rate` closing the connection, DT-002 fails; with the hook removed, DT-004 fails.

## Design excerpt
### Decisions
- DEC-001: the double is an axum or hyper style server written with `tokio::net::TcpListener` and `tokio::process::Command`, one `git <service> --stateless-rpc` child per request; no new production dependency. Alternatives considered: `git http-backend` as a CGI (rejected: environment plumbing per request, harder to inject a status or a drop); a Rust git protocol implementation (rejected: the point is a real git on the server side); the project's own `/git/{mount_id}` server (rejected: it implements neither shallow fetch nor 429). Rationale: a real `git` gives real hooks and real shallow negotiation, as the SPEC-0067 harness already relies on `git upload-pack`. Implemented by: DR-001, DR-002, DR-003. Evidence: `crates/core/tests/git_negotiation_e2e.rs` (reference program).
- DEC-002: the simulated tests stay and the real ones are added next to them. Alternatives considered: replacing E2E-039 and E2E-077 (rejected by the owner, 2026-11-03: they stay as cheap guards). Implemented by: DR-006.
- DEC-006: the reason of a hook rejection (DT-004) is read in the RED run, not assumed. The `Rate` behavior answers 429 with `Retry-After: 1`; the error the client reports for it is read, not assumed, in the RED run, and the spec of the wording stays the one of SPEC-0052/E2E-038 and SPEC-0053/E2E-032. Implemented by: DR-002.
- DEC-008: the double picks a free loopback port (`127.0.0.1:0`) per test and serves until its handle is dropped; no fixed port, so tests run in parallel. Obliged by: DR-001. Source: existing practice `git.rs:9809-9818` (`closing_remote` binds port 0).
- DEC-009: a request to `git upload-pack` or `git receive-pack` runs with the bare repository directory as its working directory and `GIT_HTTP_EXPORT_ALL` unset; the service names come from the `service` query of `info/refs` and the path of the POST. Obliged by: DR-001. Source: design.
- DEC-007: two stories. US-0157 (plain HTTP: the server, the behaviors, the real hooks, seven tests, the doc) touches 4 files at tier 2. US-0158 (https: the seam, the fixtures, the dev-dependency, the `Tls` mode, three tests) touches 7 files and rests on an unverified TLS backend behavior (DEC-004), so it is `min_tier` 1: the density of decisions, not the file count, sets it. Obliged by: n/a (no code impact): partitioning only. Source: design.
### Data model
None.
### Contracts
`crates/core/src/git/test_remote.rs` (`#[cfg(test)]`): `pub(crate) enum Behavior { Normal, Rate(u16), DropAfterPush, DropBeforePush }`, `pub(crate) struct Remote` with `Remote::start(repo_dir: &Path, behavior: Behavior) -> Remote`, `Remote::url(&self) -> String` (`http://127.0.0.1:<port>/repo.git`), stopped on drop. Endpoints: `GET /repo.git/info/refs?service=git-upload-pack|git-receive-pack`, `POST /repo.git/git-upload-pack`, `POST /repo.git/git-receive-pack`, each served by `git <service> --stateless-rpc` (with `--advertise-refs` for info/refs) run in the repository directory. `Rate(s)` answers every request with status `s` and `Retry-After: 1`. The DT-004 hook is a `hooks/pre-receive` script written into the bare repository by the test.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/test_remote.rs` | create |
| `crates/core/src/git/mod.rs` | modify (declare the module under cfg(test)) |
| `crates/core/src/tools/git.rs` | modify (tests DT-001 to DT-007) |
| `.agent_docs/testing.md` | modify (a section on the double) |
### Existing patterns
`closing_remote` at `crates/core/src/tools/git.rs:9809-9818` (a loopback listener bound to port 0); the reference program of `crates/core/tests/git_negotiation_e2e.rs` (a real `git upload-pack --stateless-rpc`); the simulated legs `e2e_tag_039_connection_lost_while_publishing` (`git.rs:26752`) and `e2e_tag_077_connection_lost_after_the_remote_accepted_the_tag` (`:27862`).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> These tests describe behavior that exists, so they pass on arrival; each is proven by the named mutation of its block. The exact error text of DT-002 (429), DT-003 and DT-004 (the hook reason) is read in the first run, then pinned.

### SPEC-0078/DT-001: the double serves a fetch and a clone
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-001. Given a bare repository with `main` at `c1`, When a `git2` clone runs against `Remote::start(.., Normal)`, Then the clone holds `c1`. Mutation: the double answers 500, DT-001 fails.
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-002: 429 is not a dropped connection
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-002. Given `Rate(429)`, When a fetch runs, Then it fails with a message that names the status and differs from the dropped connection message of `closing_remote`. Mutation: `Rate` closes the connection instead, DT-002 fails.
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-003: rate limited remote delete and publish all
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-002. Given `Rate(429)`, When `git.remote_delete_tag` and publish all run through the engine functions, Then each fails with the wording of `connection_error` plus the status cause and the remote refs and audit are unchanged (the `e2e_tag_032` and `e2e_tag_022` legs).
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-004: a real rejection of a tag push
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-003. Given a `pre-receive` hook that exits 1 with the line `tags are frozen`, When the tag `v1.0` is published, Then the refusal starts with `Remote refused tag 'v1.0': ` and its reason is the one read in the RED run (expected `pre-receive hook declined`, not assumed, DEC-006), and the remote has no `v1.0`. Mutation: the hook removed, DT-004 fails.
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-005: clone with depth filters the tags by ancestry
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-001. Given a remote with five commits and tags `v1` on the first and `v5` on the last, When a clone runs with `depth` 2, Then only `v5` is brought in. Mutation: the filter removed, DT-005 fails.
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-006: the answer is lost after the remote accepted the tag
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-002. Given `DropAfterPush`, When `v2.0` is published, Then the call fails with a connection error, the remote holds `v2.0`, and a retry against `Normal` reports an identical remote tag (E2E-077 made real).
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-007: the connection drops before the tag arrives
- Level / Driver / Location: integration / the engine functions against `test_remote::Remote` over http / `crates/core/src/tools/git.rs` tests
- **Validates:** DR-002. Given `DropBeforePush`, When `v1.0` is published, Then the call fails and the remote holds no `v1.0` (E2E-039 made real).
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md` or design document, production code outside the `#[cfg(test)]` seam of `remote.rs`, `tool-contract-golden.json`.
- Dependencies not to add: none beyond `tokio-rustls` as a dev-dependency (US-0158).
- Patterns to avoid: dashes as punctuation in code, comments or docs; a fixed port; a test that sleeps to wait for the server.
- Scope boundary: the other story of SPEC-0078.

## Definition of done
- [ ] RED evidence recorded as the named mutation of each test failing (a double answering 500 for DT-001, `Rate` closing the connection for DT-002, the hook removed for DT-004, the clone filter removed for DT-005); the first run of DT-002, DT-003 and DT-004 reads and pins the real error text.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace` passes unmodified: 0 existing tests modified.
- [ ] The existing suite passes unmodified: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace`; exceptions: none (0 existing tests modified).
- [ ] Docs updated: `.agent_docs/testing.md` (the double, its behaviors, how to add a leg).
- [ ] Self review: full.
- [ ] One commit `US-0157: add a test transport that answers like a real git server`, body `Spec: SPEC-0078` and the requirement ids.
