# US-0244: bump utoipa-swagger-ui to 10 and drop the zip acceptances

> Spec: SPEC-0095 (sdlc/specs/SPEC-0095-utoipa-swagger-ui-10/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
utoipa-swagger-ui 10.0.2 builds with zip 9, so the second zip (3.0.0) and the four RUSTSEC acceptances kept for it can go. Move `utoipa` to 6 and `utoipa-swagger-ui` to 10, remove the acceptances, and update the three gate tests that freeze the exception. This is one unit: after the bump alone, two of those tests fail.

## Requirements covered
- SPEC-0095/DR-001: The workspace SHALL depend on utoipa-swagger-ui 10.x and utoipa 6.x.
- SPEC-0095/DR-002: `Cargo.lock` SHALL contain exactly one `zip` package, with a 9.x version.
- SPEC-0095/DR-003: `deny.toml` and `.cargo/audit.toml` SHALL NOT contain RUSTSEC-2026-0367, RUSTSEC-2026-0368, RUSTSEC-2026-0369, RUSTSEC-2026-0429 or the string `BL-0059`.
- SPEC-0095/DR-004: The three tests of the design Section 8.4 SHALL assert the remaining accepted advisories (`RUSTSEC-2023-0071`, `RUSTSEC-2026-0192`, `RUSTSEC-2026-0441`), the single zip 9.x, and the absence of the four ids.
- SPEC-0095/DR-005: The server SHALL produce, for every input covered by the existing test suite other than those three tests, byte identical observable output before and after this change.

## Independent Test
`env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs --test security_gate` passes (DT-001, DT-002 red before, the three updated tests, `make_security_passes`, `deny_toml_drops_the_fixed_git2_ignores`), then `make security` exits 0 with no ignore for a zip advisory, then the non regression command below.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0095): bump `utoipa` to 6 and keep `axum_extras`; the crate requires `^6`. Dropping the unused direct dependency is out of scope.
- DEC-002 (SPEC-0095): the embedded Swagger UI static assets may change with the crate; no test pins their bytes.
- DEC-003 (SPEC-0095): one story; the bump alone leaves two gate tests red.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | lines 113-114: `utoipa = { version = "6", features = ["axum_extras"] }`, `utoipa-swagger-ui = { version = "10", features = ["axum"] }` |
| `Cargo.lock` | `cargo update -p utoipa-swagger-ui -p utoipa`; expected: utoipa 6.0.0, utoipa-gen 6.0.1, utoipa-swagger-ui 10.0.2; zip 3.0.0, rust-embed (3 crates), arbitrary, derive_arbitrary removed |
| `deny.toml` | remove lines 19-24 (the 2 comment lines and the 4 ids) |
| `.cargo/audit.toml` | remove lines 9-12 (the 4 ids) |
| `crates/mcp-fs/tests/security_gate.rs` | tests at `:149`, `:182`, `:223`; add DT-001, DT-002 |
### Existing patterns
- `lock_packages(name) -> Vec<String>` (`security_gate.rs:41`) returns every `version` of a lock package; reuse it for DT-001 and the zip count.
- Id list pattern (`security_gate.rs:149-180`): `expected` array sorted, `assert_eq!(ids, expected)`, then a reason check per id. Keep it with the 3 remaining ids.
- The graph check inside `lockfile_and_graph_carry_zip_9_only_for_normal_dependencies` (`cargo tree --workspace --all-features -e normal`, every `zip v` line starts `zip v9.`) stays as is.

## Acceptance tests
> 100% must pass, run through the project targets. Level integration, driver direct file read, location `crates/mcp-fs/tests/security_gate.rs`.

### SPEC-0095/DT-001: utoipa and swagger-ui majors
- Given the repository `Cargo.lock`
- When `lock_packages("utoipa-swagger-ui")` and `lock_packages("utoipa")` are read
- Then the first is exactly one entry starting `10.`, the second exactly one entry starting `6.`
- Red before because: `lock_packages("utoipa-swagger-ui")` returns `["9.0.2"]`.

### SPEC-0095/DT-002: no zip acceptance left
- Given `.cargo/audit.toml` and `deny.toml`
- When each file is read
- Then it contains none of `RUSTSEC-2026-0367`, `RUSTSEC-2026-0368`, `RUSTSEC-2026-0369`, `RUSTSEC-2026-0429`, `BL-0059`
- Replaces `zip_acceptances_name_the_build_time_copy` (`:223`), which is deleted.
- Red before because: `deny.toml:21` contains `RUSTSEC-2026-0367`.

### Updated tests (not new)
- `accepted_advisories_are_recorded_once_with_a_reason`: expected ids `RUSTSEC-2023-0071`, `RUSTSEC-2026-0192`, `RUSTSEC-2026-0441`.
- `lockfile_and_graph_carry_zip_9_only_for_normal_dependencies`: `zips.len() == 1` and `zips[0].starts_with("9.")`; the remaining checks unchanged.

### SPEC-0095/DT-003: no stale acceptance (existing, unmodified)
- `deny_toml_drops_the_fixed_git2_ignores` (`:126`) and `make_security_passes` (`:21`) stay green; a leftover ignore would print `advisory-not-detected`.

## Constraints
- Files not to touch: `crates/core/src/api/openapi.rs`, `crates/core/Cargo.toml`, any other test.
- Dependencies not to add or remove: none. Do not remove the direct `utoipa` dependency.
- Patterns to avoid: editing `Cargo.lock` by hand; use `cargo update -p`.
- Scope boundary: BL-0089 (datafusion) and BL-0092 (tantivy) are separate.

## Definition of done
- [ ] RED evidence recorded for DT-001 and DT-002 before the production edits.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit.
- [ ] Non regression: `env -u MCPFS_TEST_PG_DSN cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `make security`.
- [ ] DEBT: the existing suite passes unmodified; exceptions: exactly the 3 tests of the design Section 8.4 (`security_gate.rs:149`, `:182`, `:223`), count 3.
- [ ] Smoke: `docs_serves_html_without_auth` and `docs_assets_are_served_from_the_embedded_distribution` in `openapi.rs` green.
- [ ] Docs updated: none required (design Section 2).
- [ ] Self review: no zip 3.0.0 in `Cargo.lock`; `rg 'BL-0059' deny.toml .cargo crates` empty; no comment dash added.
- [ ] Closes spec: the spec's full acceptance set green (DT-001 to DT-003, the non regression command), and every requirement `DR-001` to `DR-005` of SPEC-0095 cited by at least one story with status done or landed.
- [ ] One commit `US-0244: bump utoipa-swagger-ui to 10 and drop the zip acceptances` on the work branch, body with `Summary:`, `Spec: SPEC-0095` and the requirement ids.
