# US-0133: hydrate the upload-pack path through the marker

> Spec: SPEC-0066 (sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0132
> min_tier: 2
> Files touched: 1
> Closes spec: no

## Objective
Every clone and fetch probes every object of the repository in libgit2 before building the pack. Use the SPEC-0065 marker so the probe happens only when the object index changed.

## Requirements covered
- SPEC-0066/DR-001: `handle_upload_pack` SHALL call `entry.objects.hydrate_if_stale(&repo)` where it called `entry.objects.export_to_repo(&repo)`.
- SPEC-0066/DR-003: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every object `export_to_repo` copies into libgit2 is the same object as before.

## Independent Test
`cargo test -p mcp-fs-core --lib git::http` passes DT-001 and DT-002 and the three existing upload-pack tests; with `hydrate_if_stale` mutated to return `Ok(0)` whenever a marker exists, DT-001 fails.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0066): reuse `hydrate_if_stale` with no new mechanism; one marker per entry because both paths hydrate the same repository.
### Contracts
`http/mod.rs:434`: `entry.objects.export_to_repo(&repo).await?;` becomes `entry.objects.hydrate_if_stale(&repo).await?;` The comment at `:431` ("libgit2 packs from its own ODB, so the blob store has to be materialized first") stays true. The lock on `entry.repo` is held as before.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/http/mod.rs` | one line at `:434`; two tests in `mod tests` (line 673) |
### Existing patterns
The round trip test `a_push_then_a_fetch_round_trips_the_same_objects` (`http/mod.rs:1504` region) builds a want request with `pktline::encode(&format!("want {commit}\n"))` plus `pktline::flush()`, calls `handle_upload_pack(&e, &req)`, extracts sideband 1 data into a pack, and indexes it into a fresh bare repo; copy that shape. A second handle over the same store: `BlobObjectDb::new(e.objects.blobs().clone(), e.objects.index().clone())` if those accessors exist, else the constructor the `odb.rs` tests use for two handles.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0066/DT-001: a commit written by another handle is served
- Level / Driver / Location: integration / `handle_upload_pack` / `http/mod.rs` `mod tests`
- Given the entry of the other http tests, a first commit seeded with `seed_commit(&e.objects, ..)` and a first upload-pack for it; then a second `BlobObjectDb` over the same store seeds a second commit and `refs/heads/main` is set to it
- When an upload-pack for the second commit is handled
- Then the pack, indexed into a fresh bare repository, contains the second commit and its blob
- Red before the fix because (mutation): `hydrate_if_stale` returning `Ok(0)` whenever a marker exists leaves the second commit missing from libgit2, so the pack lacks it.

### SPEC-0066/DT-002: two requests in a row both serve the commit
- Level / Driver / Location: integration / `handle_upload_pack` / `http/mod.rs` `mod tests`
- Given one seeded commit
- When two identical upload-pack requests are handled one after the other
- Then each pack indexes into a fresh bare repository and contains the commit and its blob

## Constraints
- Files not to touch: `git/odb.rs` (the mutation of DT-001 is temporary and reverted), `tools/git.rs`.
- Dependencies not to add: none.
- Scope boundary: documentation belongs to US-0134; the pack build (`have` lines) is BL-0114.

## Definition of done
- [ ] RED evidence: DT-001 fails under the stated mutation, recorded before the commit; the mutation is reverted.
- [ ] DT-001, DT-002 green; `make check` green; main stays green.
- [ ] Non regression: the existing suite passes unmodified (0 tolerated exceptions): `cargo test -p mcp-fs-core --lib git`, `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --all -- --check`.
- [ ] Docs updated: none in this story (US-0134).
- [ ] Self review: one production line changed (`git diff -U0 -- crates/core/src/git/http/mod.rs` before `mod tests`); the lock is still held across the call.
- [ ] One commit `US-0133: hydrate the upload-pack path through the marker`, body `Spec: SPEC-0066` and DR-001, DR-003.
