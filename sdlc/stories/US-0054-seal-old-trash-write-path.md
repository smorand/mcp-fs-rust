# US-0054: seal the old write path and document it

> Spec: SPEC-0044 (sdlc/specs/SPEC-0044-atomic-trash-entry-write/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0053
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
Make the two step write impossible to reintroduce: `record_trash_entry` becomes test only, a structural test guards it, docs describe the joint write. Then verify the whole spec.

## Requirements covered
- SPEC-0044/DR-004: `record_trash_entry` SHALL be compiled only under `#[cfg(test)]` (fixture use at `conformance.rs:1105`, `tools/trash.rs:419`, `fs_ops.rs:3447`), so no production code can reintroduce the two step write.
- SPEC-0044/DR-005: The system SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.
- SPEC-0044/DR-008: The system SHALL NOT change any MCP tool schema, REST payload, `trash_entries` column, or `TOOL_CONTRACT.txt`/`tool-contract-golden.json` content.

## Independent Test
`make check` green with `record_trash_entry` gated: a release build (`cargo build --release`) compiles with no caller, and DT-001 fails if a production `record_trash_entry` appears. `git diff main -- TOOL_CONTRACT.txt tool-contract-golden.json` is empty.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0044): keep `record_trash_entry` as `#[cfg(test)]` instead of deleting it; deleting means rewriting 4 fixture callers for no behavior gain.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | `#[cfg(test)]` on `record_trash_entry` (`:616`); rewrite its doc comment (`:607-615`): fixture helper, production uses `rename_into_trash` |
| `crates/core/src/core/fs_ops.rs` | DT-001 structural test |
| `AGENTS.md` | line 78: add that `trash_entries` rows are written in the rename's own transaction (`MetaBackend::rename_into_trash`) |
| `.agent_docs/architecture.md` | one appended line on the joint write, under the safety/trash discussion (append only) |
### Existing patterns
Source scan: `include_str!("fs_ops.rs")`, split at the first `#[cfg(test)]\nmod tests`, assert the head does not contain `record_trash_entry`; same for `include_str!("../purge.rs")`.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0044/DT-001: no sequential write in production code
- Structural, `fs_ops.rs` tests. Given the production region of `fs_ops.rs` and `purge.rs` (text before the tests module). Then neither contains `record_trash_entry`.
### Spec close: the non regression command
- `cargo test --workspace && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check` (via `make check`), plus, with `docker compose -f docker-compose.test.yml up -d`, the PostgreSQL and SQL Server suites of `.agent_docs/testing.md` (DT-013 on every engine). Every DT-001 to DT-013 green.

## Constraints
- Files not to touch: production logic of any caller, tool contract files.
- Dependencies not to add: none.
- Patterns to avoid: deleting `record_trash_entry`.
- Scope boundary: nothing beyond sealing and docs.

## Definition of done
- [ ] RED evidence recorded for DT-001 (temporarily reintroduce a call, see it fail).
- [ ] `make check` green; main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified; exceptions for the spec as a whole: 4 edited tests/helpers listed in US-0053 plus the three doubles of US-0052.
- [ ] Docs updated: `AGENTS.md`, `.agent_docs/architecture.md`.
- [ ] Self review: no dash punctuation in the doc lines.
- [ ] Closes spec: DT-001 to DT-013 green; DR-001 to DR-008 each cited by US-0052, US-0053 or US-0054.
- [ ] One commit `US-0054: seal the old write path and document it`, body `Spec: SPEC-0044`, DR-004, DR-005, DR-008.
