# US-0158: serve the test transport over https

> Spec: SPEC-0078 (sdlc/specs/SPEC-0078-tag-test-transport-double/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0157
> min_tier: 2
> Files touched: 7
> Closes spec: yes

## Objective
The registered tools refuse anything but https, so two legs need the double over https. Add a test only certificate check seam, a fixture certificate and a `Tls` mode. If libgit2's TLS backend cannot be made to accept it on the development machine, stop and record what was tried.

## Requirements covered
- SPEC-0078/DR-004: IF a test starts the double with `Tls` THEN THE double SHALL serve https with the fixture certificate, and the registered tool path SHALL reach it with the `https://127.0.0.1:<port>/repo.git` URL.
- SPEC-0078/DR-005: The production code SHALL NOT contain the certificate seam outside `#[cfg(test)]`.
- SPEC-0078/DR-006: The git tools SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib dt_0078` is green including DT-008, DT-009 and DT-010; with the seam flag unset, DT-008 fails on a certificate error; with the seam outside `#[cfg(test)]`, DT-010 fails.

## Design excerpt
### Decisions
- DEC-003: https is a second story. Alternatives considered: one story (rejected: the TLS part may not work on every libgit2 TLS backend, and the plain HTTP legs must not wait on it). Implemented by: DR-004.
- DEC-004: a process wide test flag `TEST_ACCEPT_ANY_CERTIFICATE` read inside `credential_callbacks` under `#[cfg(test)]`, which installs `certificate_check(|_, _| Ok(CertificateCheckStatus::CertificateOk))`. Alternatives considered: `git2::opts::set_ssl_cert_file` with the fixture (rejected: libgit2's macOS HTTPS backend validates through the system trust store and ignores that option; ASSUMED, the RED run of DT-008 shows it); adding the fixture to the system keychain (rejected: a machine wide side effect). Rationale: test only, compiled out, and the engine functions otherwise unchanged. Implemented by: DR-004, DR-005. Evidence: `remote.rs:382`.
- DEC-005: `tokio-rustls` becomes a dev-dependency (no new crate in `Cargo.lock`); the certificate and key are committed fixtures generated once with `openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1`. Alternatives considered: `rcgen` (rejected: a new crate for a constant); generating at test time with the `openssl` CLI (rejected: a second tool on PATH). Implemented by: DR-004.
- DEC-007: two stories. US-0157 (plain HTTP: the server, the behaviors, the real hooks, seven tests, the doc) touches 4 files at tier 2. US-0158 (https: the seam, the fixtures, the dev-dependency, the `Tls` mode, three tests) touches 7 files and rests on an unverified TLS backend behavior (DEC-004), so it is `min_tier` 1: the density of decisions, not the file count, sets it. Obliged by: n/a (no code impact): partitioning only. Source: design.
### Data model
None.
### Contracts
`remote.rs`: under `#[cfg(test)]`, a process wide `static TEST_ACCEPT_ANY_CERTIFICATE: AtomicBool`; in `credential_callbacks` (`:382`), when it is set, `callbacks.certificate_check(|_, _| Ok(git2::CertificateCheckStatus::CertificateOk))`. `test_remote.rs`: `Behavior` gains nothing; `Remote::start_tls(repo_dir, behavior)` wraps the listener with `tokio_rustls::TlsAcceptor` loaded from `crates/core/tests/fixtures/test-remote.pem` and `test-remote.key`, `url()` returns `https://127.0.0.1:<port>/repo.git` and sets the flag. Fixture: `openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (the cfg(test) seam; DT-010) |
| `crates/core/src/git/test_remote.rs` | modify (Tls mode) |
| `crates/core/Cargo.toml` | modify (dev-dependency tokio-rustls) |
| `crates/core/tests/fixtures/test-remote.pem` | create |
| `crates/core/tests/fixtures/test-remote.key` | create |
| `crates/core/src/tools/git.rs` | modify (tests DT-008, DT-009) |
| `Cargo.lock` | modify only if the dev-dependency changes it |
### Existing patterns
`credential_callbacks` at `crates/core/src/git/remote.rs:382`; the registered tool tests that already build a host map (search `Env::with_hosts` in `crates/core/src/tools/git.rs`); `e2e_new_145` at `remote.rs:2011` (the tool layer refuses non https URLs).

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.
> The tests build their host map with `127.0.0.1` (DT-009: `github.ibm.com`) pointing at the double's port, as the existing registered tool tests do for their hosts. The certificate seam flag is set by `Remote::start_tls`.

### SPEC-0078/DT-008: a paused merge remote delete through the registered tool, over https
- Level / Driver / Location: functional e2e / the registered tools over https / `crates/core/src/tools/git.rs` tests; DT-010 structural by `include_str!` in `remote.rs` tests
- **Validates:** DR-004. The test builds its host map with `127.0.0.1` pointing at the double's port, as the existing registered tool tests do for their hosts. Given the double with `Tls`, a project whose pull paused on a merge conflict, When `git.remote_delete_tag` runs through its registered tool, Then it is refused as for any paused operation, HEAD and the conflict files equal their values before.
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-009: the `github.ibm.com` host on the down leg
- Level / Driver / Location: functional e2e / the registered tools over https / `crates/core/src/tools/git.rs` tests; DT-010 structural by `include_str!` in `remote.rs` tests
- **Validates:** DR-004. Given the host map entry `github.ibm.com` and the double stopped, When a remote tool runs, Then the refusal equals the `connection_error` text of `remote.rs:1603` for that host. The host map entry is built as in DT-008.
- Cleanup: temp directories are dropped with the test.

### SPEC-0078/DT-010: the seam is test only
- Level / Driver / Location: functional e2e / the registered tools over https / `crates/core/src/tools/git.rs` tests; DT-010 structural by `include_str!` in `remote.rs` tests
- **Asserts:** every line of `remote.rs` that names `TEST_ACCEPT_ANY_CERTIFICATE` is inside a `#[cfg(test)]` item, and `test_remote` is declared under `#[cfg(test)]`. **Method:** `include_str!` of `remote.rs` and `git/mod.rs`, checking the preceding attribute. Covers DR-005.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md` or design document, production code outside the `#[cfg(test)]` seam of `remote.rs`, `tool-contract-golden.json`.
- Dependencies not to add: none beyond `tokio-rustls` as a dev-dependency (US-0158).
- Patterns to avoid: dashes as punctuation in code, comments or docs; a fixed port; a test that sleeps to wait for the server.
- Scope boundary: the other story of SPEC-0078.

## Definition of done
- [ ] RED evidence recorded as the named mutation of each test failing (seam flag unset for DT-008, seam outside `#[cfg(test)]` for DT-010). If DT-008 cannot pass because the TLS backend refuses the certificate even with the seam, stop and write a `## Blocked` section: never weaken the test.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace` passes unmodified: 0 existing tests modified.
- [ ] The existing suite passes unmodified: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace`; exceptions: none (0 existing tests modified).
- [ ] Docs updated: `.agent_docs/testing.md` (the `Tls` mode and its seam).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0078 cited by at least one story with status done or landed.
- [ ] One commit `US-0158: serve the test transport over https`, body `Spec: SPEC-0078` and the requirement ids.
