# US-0131: document the hydration marker and close the spec

> Spec: SPEC-0065 (sdlc/specs/SPEC-0065-hydrate-only-when-the-index-changed/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0130
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
Document the marker and its declared limits, pin with a structural check that only the upload-pack path still uses the full export, and close the spec with the before and after measurement.

## Requirements covered
- SPEC-0065/DR-005 [EARS-UB]: The server SHALL NOT change the upload-pack export (`git/http/mod.rs:434`), which keeps calling `export_to_repo`.
  - **Reason:** it is a protocol path, not a tool call (BL-0113).
- SPEC-0065/DR-006 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object `export_to_repo` copies into libgit2 is the same object as before.


## Independent Test
The DT-009 commands print what the test says. `cargo test -p mcp-fs-core --lib git` and `cargo test --workspace` are green (the whole existing suite is the non mock assertion for DR-006). DT-010 is run and its two tables go in the commit body.

## Design excerpt
### Decisions
- DEC-006 (SPEC-0065): declared limits, written into `.agent_docs/git.md`: (a) the object index only grows (`delete` removes a blob, never an index row; no code deletes `git_objects` rows except `purge_repo`, which drops the entry first); (b) the on disk libgit2 repository is a cache rebuilt at restart, wiping it while the server runs is repaired only at restart; (c) several replicas are supported through the count; re-writing an existing sha is not detected.
- DEC-007: the upload-pack path stays on `export_to_repo`.
- DEC-008: the measurement is evidence in the commit body, not a gating test.
### Data model
None. The object index schema is not touched; the marker is process memory only.
### Contracts
No code change in this story.
### Paths
| Path | Action |
|---|---|
| `.agent_docs/git.md` | modify (a section next to the hydration lines 81 and 138, timestamped 2026-10-30, naming `hydrate_if_stale`, `bind_repo` and the limits (a) to (c)) |
### Existing patterns
DT-009's last check needs the doc to contain the text `hydrate_if_stale`. The scratch measurement test is the one of SPEC-0065 Section 2: for N in [500, 5000, 20000]: `Env::new()`, `seed_tags(&e)`, `e.entry().await`, lock `entry.repo`, `hydrate`, write N blobs with `repo.blob(format!("blob {i}").as_bytes())`, `entry.objects.import_from_repo(&repo)`, then 5 times: time `hydrate(&entry, &repo)` in microseconds, release the repo lock, time a lightweight `tag(&e, json!({"name": format!("l{i}")}))` in milliseconds; `eprintln!("MEASURE objects={n} hydrate_us={h:?} lightweight_tag_ms={t:?}")`; run with `cargo test -p mcp-fs-core --lib zz_measure_hydrate -- --nocapture`; revert the scratch test with `git checkout -- crates/core/src/tools/git.rs` before committing.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0065/DT-009: structural, callers and documentation
- **Method:** `awk '/^mod tests/{exit} {print}' crates/core/src/tools/git.rs | rg -c 'export_to_repo\('` prints nothing (rg exit status 1) and the same with `hydrate_if_stale\(` prints `1`; `awk '/^mod tests/{exit} {print}' crates/core/src/git/http/mod.rs | rg -c 'export_to_repo\('` prints `1`; `rg -c 'bind_repo\(' crates/core/src/git/repo.rs` prints `1`; `rg -n 'hydrate_if_stale' .agent_docs/git.md` prints at least one line (the limits of DEC-006 documented).

### SPEC-0065/DT-010: measurement, not gating
- **Method:** the temporary test of Section 2 (write N blobs, import, time `hydrate` and a lightweight `git.tag_create` 5 times), run before and after for N in 500, 5 000, 20 000; not committed. After the change `hydrate` at 20 000 objects was under 10 ms and the lightweight tag under 30 ms in each of 5 samples (recorded evidence, enforced by no test); both tables go in the commit body.

## Constraints
- Files not to touch: every production file (a production change here means an earlier story is incomplete: report it instead).
- Dependencies not to add: none.
- Patterns to avoid: a timing assertion in a committed test.
- Scope boundary: BL-0113 (the upload-pack path) is separate.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a stub, or the mutation named for it where the behavior pre exists); mutations reverted with `git checkout -- <file>`.
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib git` stays green.
- [ ] DEBT: observable behavior unchanged (DR-006): every object `export_to_repo` copies into libgit2 is the same object as before.
- [ ] Docs: `.agent_docs/git.md` updated in this story's commit (it is the story's deliverable).
- [ ] Self review: time and space of the critical path stated (measurement and shell checks only), no lock held across an await, no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Closing story: every requirement of SPEC-0065 (DR-001 to DR-006) cited by a story with status done or landed; the measurement tables are in the commit body.
- [ ] One commit `US-0131: document the hydration marker and close the spec`, body `Spec: SPEC-0065` and the requirement ids.
