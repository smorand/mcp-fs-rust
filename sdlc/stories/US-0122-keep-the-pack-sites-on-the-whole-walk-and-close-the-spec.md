# US-0122: keep the pack sites on the whole walk and close the spec

> Spec: SPEC-0062 (sdlc/specs/SPEC-0062-import-reachable-objects/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0121
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
Pin that the four pack sites still use the whole walk, then close the spec: run the structural check and the before and after measurement of `git.commit`.

## Requirements covered
- SPEC-0062/DR-003 [EARS-UB]: The server SHALL NOT change the four sites that import a pack: push receive (`git/http/mod.rs:572`), clone (`tools/git.rs:3324`), fetch (`:3918`) and the fetch step of pull (`:6803`). Reason: a pack brings an unbounded set, and `reconcile_incoming_tags` relies on the tag objects it brings (BL-0112).
- SPEC-0062/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

## Independent Test
The DT-018 commands print what the test says. `cargo test -p mcp-fs-core --lib git` and `cargo test --workspace` are green (the whole existing suite is the non mock assertion for DR-004). DT-019 is run and its two tables go in the commit body.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0062): the measurement is evidence in the commit body, not a gating test.
- DEC-005: two repairs of the whole walk are dropped on purpose for the seven bounded sites; the four pack sites keep them.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No production change in this story.
### Paths
| Path | Action |
|---|---|
| (none) | no file change expected; if a comment above `mod tests` in `tools/git.rs` would contain the text `import_reachable(` or `import_from_repo(` fix it so DT-018 counts stay exact |
### Existing patterns
DT-018 uses `mod tests {` at column 0 (`tools/git.rs:7655`) as the cut between production and tests. The scratch measurement test is the one of SPEC-0062 Section 2: for N in [500, 5000, 20000]: `Env::new()`, `seed_tags(&e)`, lock `entry.repo`, `hydrate`, write N blobs with `repo.blob(format!("blob {i}").as_bytes())`, `entry.objects.import_from_repo(&repo)`, then 5 times `e.write("/c.txt", &format!("c{i}\n"))` + time `e.commit(&format!("c{i}"))` in milliseconds, `eprintln!("MEASURE objects={n} commit_ms={v:?}")`; run with `cargo test -p mcp-fs-core --lib zz_measure_commit -- --nocapture`; revert the scratch test with `git checkout -- crates/core/src/tools/git.rs` before committing.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0062/DT-018: structural, only the four pack sites keep the whole walk
- Level / Driver / Location: structural / shell / repository
- When `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_from_repo\('` runs
- Then it prints `3`; `rg -c 'import_from_repo\(' crates/core/src/git/http/mod.rs` prints `1`; `rg 'import_from_repo\(' crates/core/src/tools/git_tags.rs` prints nothing (rg exit status 1); `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_reachable\('` prints `7`

### SPEC-0062/DT-019: measurement, not gating
- Level / Driver / Location: measurement / temporary test, not committed / scratch in `crates/core/src/tools/git.rs`
- Given the temporary test above, for N in 500, 5 000, 20 000
- When run on the final tip of the chain
- Then `git.commit` at N = 20 000 is under 250 ms in each of the 5 samples (baseline 1099 to 1173 ms); both tables (before from SPEC-0062 Section 2, after measured now) are in the commit body; the scratch test is reverted before committing

## Constraints
- Files not to touch: every production file (a production change here means an earlier story is incomplete: report it instead).
- Dependencies not to add: none.
- Patterns to avoid: a timing assertion in a committed test.
- Scope boundary: BL-0112 (the pack sites) and BL-0111 (the hydrate floor) are separate.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (measurement and shell checks only), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Closing story: every requirement of SPEC-0062 (DR-001 to DR-004) cited by a story with status done or landed; the measurement tables are in the commit body.
- [ ] One commit `US-0122: keep the pack sites on the whole walk and close the spec`, body `Spec: SPEC-0062` and the requirement ids.
