# US-0043: narrow the zip acceptances and drop stale citations

> Spec: SPEC-0036 (sdlc/specs/SPEC-0036-move-zip-to-9/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0042
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
With the direct zip on 9.x, the three zip advisories remain only for the build time zip 3.0.0 of utoipa-swagger-ui; reword their acceptance to say so and point to BL-0059, and replace the six `zip-2.4.2` source citations in `tools/archive.rs`.

## Requirements covered
- SPEC-0036/DR-003: `crates/core/src/tools/archive.rs` SHALL NOT cite `zip-2.4.2` source paths; the comments at `:288-289`, `:380-381`, `:1120` and `:1626` cite the zip 9.0.5 locations of the same functions or drop the path.
- SPEC-0036/DR-004: `.cargo/audit.toml` and `deny.toml` SHALL keep RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369 with a reason naming zip 3.0.0 as a build dependency of utoipa-swagger-ui and BL-0059, SHALL NOT name BL-0058; `make security` SHALL exit 0.
- SPEC-0036/DR-005 (partial: full suite closing proof): byte identical output except the declared breaks; `tool-contract-golden.json` unchanged.

## Independent Test
`cargo test -p mcp-fs --test security_gate` passes DT-031 and DT-036 (both red before), `make security` exits 0, then the non regression command (DT-037).

## Design excerpt
### Decisions
- DEC-003: keep the three ids accepted for the build time zip 3.0.0 (utoipa-swagger-ui 10.0.1 still needs `zip ^8.6`), removal under BL-0059.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive.rs` | comments `:288`, `:289`, `:380`, `:381`, `:1120`, `:1626` |
| `.cargo/audit.toml` | `:13-15` reasons |
| `deny.toml` | `:19-23` comment and reasons |
| `crates/mcp-fs/tests/security_gate.rs` | DT-031, DT-036 |
### Existing patterns
- `accepted_advisories_are_recorded_once_with_a_reason` (`security_gate.rs:149`) pins the exact id set: keep the same 10 ids.
- zip 9.0.5 locations: `by_index_raw` `zip-9.0.5/src/read/zip_archive.rs:699`, `is_symlink` `zip-9.0.5/src/read/zipfile.rs:191`, `unix_mode` `zipfile.rs:208`, `encrypted` `zipfile.rs:167`.

## Acceptance tests
### SPEC-0036/DT-031: no zip 2.4.2 citation
- Location: `crates/mcp-fs/tests/security_gate.rs`; reads `crates/core/src/tools/archive.rs`, asserts it does not contain `zip-2.4.2`.
- Red before because: `:288`, `:289`, `:380`, `:381`, `:1120`, `:1626` contain it.

### SPEC-0036/DT-036: acceptance narrowed and gate green
- Location: `crates/mcp-fs/tests/security_gate.rs`
- Asserts: in `.cargo/audit.toml` and `deny.toml`, each line holding RUSTSEC-2026-0367, 0368 or 0369 contains `utoipa-swagger-ui` and `BL-0059`; neither file contains `BL-0058` anywhere; `make_security_passes` stays green.
- Red before because: `.cargo/audit.toml:13-15` read `until BL-0058` and `deny.toml:21-23` lack `utoipa-swagger-ui`.

### SPEC-0036/DT-037: guardrail
- The non regression command green; `git diff main -- tool-contract-golden.json` empty.

## Constraints
- No code change outside comments; no ignore id added or removed.
- Files not to touch: everything US-0042 owns except the six comments.

## Definition of done
- [ ] RED evidence for DT-031 and DT-036 in `.implement/US-0043-red.log`.
- [ ] Every acceptance test green; `make check` green.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 0 in this story (US-0042's 10 already landed).
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: the full acceptance set of SPEC-0036 green (DT-030..DT-039), every DR-001..DR-009 cited by a done or landed story.
- [ ] One commit `US-0043: narrow the zip acceptances and drop stale citations`, body `Spec: SPEC-0036` and the DR ids.
