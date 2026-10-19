# US-0047: replace serde_yaml with serde_norway

> Spec: SPEC-0040 (sdlc/specs/SPEC-0040-replace-serde-yaml/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 12
> Closes spec: yes

## Objective
Swap the deprecated `serde_yaml` 0.9 for its drop in fork `serde_norway` 0.9, so the unmaintained `unsafe-libyaml` (RUSTSEC-2024-0449) leaves the build and the advisory leaves the accepted list. Config parsing behaves exactly as before.

## Requirements covered
- SPEC-0040/DR-001: The workspace SHALL parse and emit YAML through `serde_norway` 0.9, and SHALL NOT name `serde_yaml` in `Cargo.toml`, `crates/` or `.agent_docs/`.
- SPEC-0040/DR-002: `Cargo.lock` SHALL NOT contain `serde_yaml` nor `unsafe-libyaml`, `.cargo/audit.toml` SHALL NOT list RUSTSEC-2024-0449, and `cargo audit` SHALL report no vulnerability and no warning.
- SPEC-0040/DR-003: byte identical observable output for every input the existing suite covers; every tracked config file (`config/agent_test.yaml`, `config/local.yaml.template`, `config/minio.yaml.template`) SHALL deserialize to an equal value, the two templates read after the same `${VAR}` expansion the server applies; `tool-contract-golden.json` unchanged.

## Independent Test
`cargo test -p mcp-fs --test security_gate` passes DT-001 and DT-002; the DT-003 parity tests pass in `mcp-fs-core` and `agent`; `cargo tree -i unsafe-libyaml --all-features` reports no match.

## Design excerpt
### Decisions
- DEC-001: `serde_norway` 0.9.42 (same API: `from_str`, `to_value`, `from_value`, `Value`); not `serde_yaml_ng` (still pulls `unsafe-libyaml`), not `serde_yml` (RUSTSEC-2025-0068), not `serde-saphyr` (different semantics).
- DEC-002: rename the crate everywhere, no `package =` alias.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | `:25` `serde_yaml = "0.9"` to `serde_norway = "0.9"` |
| `crates/core/Cargo.toml` `:39`, `crates/agent/Cargo.toml` `:18` | dependency renamed |
| `Cargo.lock` | `cargo update` |
| `crates/core/src/config.rs` | `:906` comment, `:917` code, 15 test lines (`:1217` to `:1587`); DT-003 server test |
| `crates/agent/src/config.rs` | `:91`; DT-003 agent test |
| `crates/core/src/cli.rs` `:779`, `crates/core/src/git/remote.rs` `:47` | comments |
| `.cargo/audit.toml` | remove the `:12` RUSTSEC-2024-0449 line |
| `crates/mcp-fs/tests/security_gate.rs` | `:160` drop the id; DT-001, DT-002 |
| `.agent_docs/config.md` `:375`, `.agent_docs/git.md` `:169` | name updated |
### Existing patterns
- Lockfile and `cargo tree` assertions: `lock_packages` and the SPEC-0036 DT-030 test in `crates/mcp-fs/tests/security_gate.rs`.
- Config load path with `${VAR}` expansion: `crates/core/src/config.rs` around `:906-917` (`expand_env`), and the test at `:1587`.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0040/DT-001: no serde_yaml, no unsafe-libyaml
- Location: `security_gate.rs`. Cargo.lock has no `name = "serde_yaml"` and no `name = "unsafe-libyaml"`; `cargo tree -i unsafe-libyaml --all-features` exits non zero with `did not match any packages`; `Cargo.toml`, every `crates/**/Cargo.toml` and `crates/**/*.rs`, and `.agent_docs/*.md` contain no `serde_yaml`.
- RED before: Cargo.lock lists `serde_yaml` and `unsafe-libyaml`.

### SPEC-0040/DT-002: advisory leaves the accepted list
- Location: `security_gate.rs`. `.cargo/audit.toml` does not contain `RUSTSEC-2024-0449`; `cargo_audit_reports_no_vulnerability_and_no_warning` and `make_security_passes` stay green.
- RED before: `.cargo/audit.toml:12` lists it.

### SPEC-0040/DT-003: config parity
- Server (`config.rs` tests): `local.yaml.template` and `minio.yaml.template`, `${VAR}` expanded with fixed values set by the test, parsed into `ServerConfig`; agent (`agent` tests): `config/agent_test.yaml` into its config type. Each `format!("{:?}", parsed)` equals a snapshot literal recorded on serde_yaml BEFORE the swap. Guardrail: written and green first, must stay green after.

### SPEC-0040/DT-004: guardrail
- Non regression command green; `git diff main -- tool-contract-golden.json` empty.

## Constraints
- No behavior change; no `package =` alias; no other dependency moved beyond what `cargo update` needs for the swap.
- Never add an audit ignore.

## Definition of done
- [ ] RED evidence for DT-001 and DT-002 in `.implement/US-0047-red.log`; DT-003 snapshots recorded on serde_yaml first.
- [ ] Every acceptance test green; `make check` green.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 2 (the 15 `config.rs` test lines, crate path only; the `security_gate.rs:160` expected list, id removed).
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: `.agent_docs/config.md`, `.agent_docs/git.md`.
- [ ] Self review: full.
- [ ] Closes spec: DT-001..DT-004 green, DR-001..DR-003 cited.
- [ ] One commit `US-0047: replace serde_yaml with serde_norway`, body `Spec: SPEC-0040` and the DR ids.
