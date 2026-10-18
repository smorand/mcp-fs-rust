# US-0045: local put rewrites a blob deleted mid call

> Spec: SPEC-0038 (sdlc/specs/SPEC-0038-local-put-self-heal/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
`LocalBlobStore::put` fails with not found when its blob is deleted between the existence decision and the modified time refresh. Open directly and fall through to the normal write on `NotFound`, so the write succeeds and the blob is rewritten.

## Requirements covered
- SPEC-0038/DR-001: IF opening an existing blob for the modified time refresh fails with `NotFound` THEN `LocalBlobStore::put` SHALL write the bytes through the temp file then rename path and return `Ok`.
- SPEC-0038/DR-002: `LocalBlobStore::put` SHALL NOT rewrite the blob when opening it fails with an error other than `NotFound`; it SHALL return that error.
- SPEC-0038/DR-003: `local.rs` SHALL call a `#[cfg(test)]` only hook after deciding the blob exists and before refreshing it, absent from non test builds.
- SPEC-0038/DR-004: byte identical observable output for every input the existing suite covers; `tool-contract-golden.json` unchanged.

## Independent Test
`cargo test -p mcp-fs-core --lib storage::blob::local` passes DT-001..DT-003; then the non regression command.

## Design excerpt
### Decisions
- DEC-001: open directly and fall through on `NotFound`; no retry, no locking.
- DEC-002: a `#[cfg(test)]` hook as the seam.
- DEC-003: hook is a `thread_local!` `Cell<Option<fn(&Path)>>`, invoked in async context before `spawn_blocking` (current thread test runtime), never a global static.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/blob/local.rs` | `put` `:40-70`; hook; tests in module `:188` |
### Existing patterns
- Backdating and modified time assertions: `put_of_existing_bytes_refreshes_the_modified_time` (`local.rs:325`); store fixture `store()` and path layout `{tmp}/mcpfs-test/{sha[..2]}/{sha}` (same test).

## Acceptance tests
### SPEC-0038/DT-001: vanished blob is rewritten
- Given `put("racy", b"data")` done and the hook set to delete `{root}/ra/racy` / When `put("racy", b"data")` / Then `Ok`; `get("racy", 0, None)` == `data`; `std::fs::metadata` of the file succeeds. Hook cleared after.
- RED before because: `put` returns the `NotFound` from `File::open`.

### SPEC-0038/DT-002: refresh path leaves no temp file
- Given blob `kept` backdated 48 h / When `put("kept", b"data")` / Then modified time under 2 s old, bytes unchanged, shard folder `ke` holds no entry starting `.tmp-`. Guardrail.

### SPEC-0038/DT-003: permission denied is not swallowed
- Unix only, skipped when running as root. Given blob `locked` stored, file mode `0o000` / When `put("locked", b"data")` / Then `Err`; file mode still `0o000`. Cleanup restores `0o644`.
- Mutation RED: treating every open error as `NotFound` makes `put` return `Ok`.

### SPEC-0038/DT-004: guardrail
- Non regression command green; `git diff main -- tool-contract-golden.json` empty.

## Constraints
- Files not to touch: everything except `local.rs`.
- Dependencies not to add: any.
- No hook code in non test builds.

## Definition of done
- [ ] RED evidence for DT-001 and mutation RED for DT-003 in `.implement/US-0045-red.log`.
- [ ] Every acceptance test green; `make check` green.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none (0).
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: DT-001..DT-004 green, DR-001..DR-004 cited.
- [ ] One commit `US-0045: local put rewrites a blob deleted mid call`, body `Spec: SPEC-0038` and the DR ids.
