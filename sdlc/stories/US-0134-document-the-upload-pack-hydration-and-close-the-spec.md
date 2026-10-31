# US-0134: document the upload-pack hydration and close the spec

> Spec: SPEC-0066 (sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0133
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
Update the one documentation line that names the upload-pack path as an exception, run the structural and measurement checks, and mark the spec implemented.

## Requirements covered
- SPEC-0066/DR-001: `handle_upload_pack` SHALL call `entry.objects.hydrate_if_stale(&repo)` where it called `entry.objects.export_to_repo(&repo)`.
- SPEC-0066/DR-003: byte identical observable output (see US-0133); this story proves the whole set.

## Independent Test
`rg -n 'upload-pack path' .agent_docs/git.md` no longer names a limit; the DT-004 commands print what the spec says; DT-005 numbers are in the commit body.

## Design excerpt
### Decisions
- DEC-002 (SPEC-0066): the gain is about 8 to 10% of a request at 20 000 reachable objects; the larger cost, the pack build ignoring `have` lines, is BL-0114.
### Paths
| Path | Action |
|---|---|
| `.agent_docs/git.md` (lines 109 to 110, Hydration marker section) | replace the bullet saying the upload-pack path still calls `export_to_repo` by: the upload-pack path hydrates through the same marker; the pack build ignoring `have` lines is BL-0114 |
| `sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/sdlc-design.md` | `Status: implemented` |
### Existing patterns
Follow the bullet style of the surrounding section; no dashes as punctuation, timestamp the entry (2026-10-30).

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0066/DT-004: structural
- Level / Driver / Location: structural / shell / n/a
- When `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg -c 'hydrate_if_stale\('` runs
- Then it prints `1`; the same with `export_to_repo\(` finds nothing; `rg -n 'export_to_repo\(' crates/core/src --glob '*.rs'` lists only `git/odb.rs` and `tools/git.rs` test lines; `.agent_docs/git.md` no longer carries the exception.

### SPEC-0066/DT-005: measurement, not gating
- When the temporary test of Section 2 of the design runs after the change (a commit with 20 000 files, 5 requests; timing `hydrate_if_stale` on the synchronised repository)
- Then the hydration part is under 10 ms in each sample (baseline 124 to 172 ms); the numbers go in the commit body; the temporary test is not committed.

## Constraints
- Files not to touch: every `.rs` file.
- Scope boundary: BL-0114 stays open.

## Definition of done
- [ ] DT-004 outputs and DT-005 numbers recorded.
- [ ] `make check` green; main stays green.
- [ ] Non regression: the existing suite passes unmodified (0 tolerated exceptions): `cargo test -p mcp-fs-core --lib git`, `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --all -- --check`.
- [ ] Docs updated: `.agent_docs/git.md`.
- [ ] Closes spec: the full acceptance set (DT-001 to DT-004 and the unmodified suite) green, and DR-001, DR-002, DR-003 each cited by a story done or landed (US-0132, US-0133, US-0134).
- [ ] Self review: no source file in the diff.
- [ ] One commit `US-0134: document the upload-pack hydration and close the spec`, body `Spec: SPEC-0066` and DR-001, DR-003.
