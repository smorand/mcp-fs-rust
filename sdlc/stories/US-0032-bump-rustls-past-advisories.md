# US-0032: Bump rustls past the four advisories

> Spec: SPEC-0029 (sdlc/specs/SPEC-0029-rustls-advisory-bump/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 3
> Files touched: 1
> Closes spec: yes

## Objective
`make security` fails on rustls 0.23.45 (RUSTSEC-2026-0346, 0347, 0348, 0349), so `make check` never reaches the tests. Bump rustls to 0.23.46 in the lockfile only, nothing else.

## Requirements covered
- SPEC-0029/DR-001: The workspace lockfile SHALL pin exactly one `rustls` package, at version `0.23.46` or later within `0.23`.
- SPEC-0029/DR-002: The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.
- SPEC-0029/DR-003: The change SHALL NOT edit `Cargo.toml`, any `crates/*/Cargo.toml`, `deny.toml` or `Makefile`, and SHALL NOT add an advisory ignore.

## Independent Test
`cargo test -p mcp-fs --test security_gate` prints `test result: ok. 2 passed`; on main it prints `1 passed; 1 failed` with `error: 4 vulnerabilities found!`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0029): Lockfile only patch bump (`cargo update -p rustls`). Rejected: a direct `rustls` dependency floor, a whole tree `cargo update`, ignoring the advisories.
- DEC-002 (SPEC-0029): the ten warning level advisories are out of scope (BL-0053).
### Paths
| Path | Action |
|---|---|
| `Cargo.lock` | `cargo update -p rustls` |
### Existing patterns
Literal example, the expected lockfile entry after the change:
```
name = "rustls"
version = "0.23.46"
```
The dry run on main printed exactly `Updating rustls v0.23.45 -> v0.23.46` and no other package.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0029/DT-020: The security gate passes
- Level / Driver / Location: integration / `cargo test` / `crates/mcp-fs/tests/security_gate.rs::make_security_passes`
- Red before the fix because: `make security failed: ... error: 4 vulnerabilities found!`
- Given the bumped lockfile / When `cargo test -p mcp-fs --test security_gate` runs / Then `make_security_passes ... ok` and `test result: ok. 2 passed; 0 failed`.

### SPEC-0029/DT-021: The vulnerable version is gone from the graph
- Red before the fix because: `cargo tree -i rustls@0.23.45 --all-features` exits 0 on main.
- When `cargo tree -i rustls@0.23.45 --all-features` runs / Then it exits non zero with `did not match any packages` / And `rg -n -A1 '^name = "rustls"$' Cargo.lock` prints exactly one entry with `version = "0.23.46"`.

### SPEC-0029/DT-010: Manifests untouched
- When `git diff --name-only main...HEAD` runs on the story branch / Then it prints only `Cargo.lock` plus `sdlc/` paths / And `git diff main...HEAD -- deny.toml Makefile Cargo.toml crates/*/Cargo.toml` is empty.

## Constraints
- Only `Cargo.lock` changes outside `sdlc/`.
- Never `cargo update` without `-p rustls`; never add `--ignore` or a `deny.toml` entry.
- If `cargo update -p rustls` moves any other package, stop and report BLOCKED.
- A new advisory on another crate making the gate red is a new backlog entry, not a fix here.
- Run the full gate with the DB DSNs (below).

## Definition of done
- [ ] RED evidence: DT-020 and DT-021 output on the unchanged lockfile recorded before the bump.
- [ ] DT-010, DT-020, DT-021 green; `make check` green with `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none (0).
- [ ] Docs updated: none needed.
- [ ] Self review (5 yes/no): only Cargo.lock changed? only rustls moved? no ignore added? security gate green? full suite green with no `skipping` DB suite?
- [ ] Closes spec: DT-010, DT-020, DT-021 and 8.1 green; DR-001..DR-003 cited here.
- [ ] One commit `US-0032: Bump rustls past the four advisories`, body `Spec: SPEC-0029` and `DR-001, DR-002, DR-003`.
