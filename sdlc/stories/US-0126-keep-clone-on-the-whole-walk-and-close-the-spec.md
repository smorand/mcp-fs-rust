# US-0126: keep clone on the whole walk and close the spec

> Spec: SPEC-0063 (sdlc/specs/SPEC-0063-import-new-objects-of-packs/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0125
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
Pin that clone still uses the whole walk and the three pack sites use `import_tips`, then close the spec: run the structural check and the before and after measurement of `git.remote_fetch`.

## Requirements covered
- SPEC-0063/DR-003 [EARS-UB]: The server SHALL NOT change the clone import (`tools/git.rs:3326`), which keeps calling `import_from_repo(&cloned)`.
  - Reason: clone reads a fresh temporary repository, so the whole walk already equals the closure of its refs, and a shallow clone (`depth > 0`) may leave a boundary commit's parents out of the odb, which the strict import would refuse.
- SPEC-0063/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object a site stores has the same id, type, payload, blob key and index row as before.

## Independent Test
The DT-010 commands print what the test says. `cargo test -p mcp-fs-core --lib git` and `cargo test --workspace` are green (the whole existing suite is the non mock assertion for DR-004). DT-011 is run and its two tables go in the commit body.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0063): clone is not changed (no gain on a fresh repository; the shallow case).
- DEC-007: the measurement is evidence in the commit body, not a gating test.
- DEC-010: this story carries DR-003 because a negative requirement is proven on the final tree.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No production change in this story.
### Paths
| Path | Action |
|---|---|
| (none) | no file change expected; if a comment above `mod tests` would contain `import_tips(` or `import_from_repo(` fix it so the DT-010 counts stay exact |
### Existing patterns
DT-010 uses `mod tests {` at column 0 as the cut between production and tests in each file (`tools/git.rs:7657`, `git/http/mod.rs:663`, `git/odb.rs:455`). The scratch measurement test: for N in [500, 5000, 20000]: `Env::new()`, `cloned_remote(&e, "label")`, `advance_bare_remote(&dir, "refs/heads/main", "first\n")`, `call_fetch_branch(&e, &url, "anonymous")`, lock `entry.repo`, `hydrate(&entry, &repo)`, write N blobs with `repo.blob(format!("blob {i}").as_bytes())`, `entry.objects.import_from_repo(&repo)`, then 5 times: `advance_bare_remote(&dir, "refs/heads/main", &format!("c{i}\n"))` and time `call_fetch_branch` in milliseconds, `eprintln!("MEASURE objects={n} fetch_ms={v:?}")`; run with `cargo test -p mcp-fs-core --lib zz_measure_fetch -- --nocapture`; revert the scratch test with `git checkout -- crates/core/src/tools/git.rs` before committing.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0063/DT-010: structural, the three sites changed and clone kept
- Level / Driver / Location: structural / shell / repository
- When `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_from_repo\('` runs
- Then it prints `1` (clone); `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg 'import_from_repo'` prints nothing (rg exit status 1); `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'import_tips\('` prints `2`; `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg -c 'import_tips\('` prints `1`; `awk '/^mod tests/{exit} {print}' crates/core/src/git/odb.rs | rg -c 'import_from_repo\('` prints `2` (the definition at `:192` and the fallback call)

### SPEC-0063/DT-011: measurement, not gating
- Level / Driver / Location: measurement / temporary test, not committed / scratch in `crates/core/src/tools/git.rs`
- Given the temporary test above, for N in 500, 5 000, 20 000
- When run on the final tip of the chain
- Then the fetch at N = 20 000 was under 250 ms in each of the 5 samples (baseline 1132 to 1184 ms; recorded evidence, enforced by no test); both tables (before from SPEC-0063 Section 2, after measured now) are in the commit body; the scratch test is reverted before committing

## Constraints
- Files not to touch: every production file (a production change here means an earlier story is incomplete: report it instead).
- Dependencies not to add: none.
- Patterns to avoid: a timing assertion in a committed test.
- Scope boundary: BL-0111 (the hydrate floor) is separate.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-004): every object a site stores has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (measurement and shell checks only), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Closing story: every requirement of SPEC-0063 (DR-001 to DR-005) cited by a story with status done or landed; the measurement tables are in the commit body.
- [ ] One commit `US-0126: keep clone on the whole walk and close the spec`, body `Spec: SPEC-0063` and the requirement ids.
