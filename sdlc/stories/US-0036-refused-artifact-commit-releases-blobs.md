# US-0036: A refused artifact commit releases its blobs

> Spec: SPEC-0033 (sdlc/specs/SPEC-0033-refused-artifact-commit-leaks-blobs/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 3
> Closes spec: yes

## Objective
When a document is edited while it converts, the artifact commit is refused, but the text and picture blobs it already stored stay in the project's blob store forever with no reference. Release them on refusal, without ever deleting a blob something references and without changing the error the caller sees.

## Requirements covered
- SPEC-0033/DR-001: IF `commit_artifact_set` returns an error THEN `PendingSet::commit` SHALL delete, for each hash it stored in this call (the text hash and every picture hash), the blob of that hash when the volume has no `blob_refs` row for it.
- SPEC-0033/DR-002: `PendingSet::commit` SHALL NOT delete a blob whose hash has a `blob_refs` row in the volume, whatever referenced it (a file node or another artifact set).
- SPEC-0033/DR-003: On a refused commit, `PendingSet::commit` SHALL return the error `commit_artifact_set` returned, code and message byte identical.
- SPEC-0033/DR-004: IF deleting a blob fails during the release THEN `PendingSet::commit` SHALL log the failure at warn level with the hash and continue with the remaining hashes, and SHALL still return the original commit error.
- SPEC-0033/DR-005: The release SHALL consider each distinct hash once, even when the text and a picture, or two pictures, share a hash.
- SPEC-0033/DR-006: On a successful commit, `PendingSet::commit` SHALL NOT delete any blob.
- SPEC-0033/DR-007: The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib core::fs_ops::tests` passes including DT-010..DT-014; DT-010 fails on `main` with `blob.exists(<text sha>) == true` after the refusal.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0033): compensating release right after a refused commit; a general orphan sweep is BL-0056, out of scope.
- DEC-002 (SPEC-0033): "referenced" means a `blob_refs` row for `(volume_id, sha256)`, the count files and artifacts share (`tx_incref` `meta.rs:1179`, `tx_decref` `:1214`).
- DEC-003 (SPEC-0033): best effort: delete errors logged (`tracing::warn!`), never returned.
### Contracts
- New: `RelationalMetaStore::blob_is_referenced(&self, sha: &str) -> Result<bool>`, `volume_id` in the `WHERE` (AGENTS.md rule), through `self.db` and a `Query`, never raw driver SQL.
- Unchanged: the refusal `ERR_INVALID_ARGUMENT` / `Document changed during conversion: <path>` (`meta.rs:70`, `:1155-1156`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | add `blob_is_referenced` near `blob_refcount_for_test` (`:2069`) but not test gated |
| `crates/core/src/docs/artifacts/capture.rs` | `PendingSet::commit` (`:150-207`) |
| `crates/core/src/core/fs_ops.rs` | tests near `late_refusal` (`:3984`) |
### Existing patterns
- `PausingBlob` (`fs_ops.rs:3886`): a `BlobBackend` wrapper pausing a put; extend or mirror it for a failing `delete` and a delete counter.
- `late_refusal` (`fs_ops.rs:3984`) and `a_late_refusal_restores_the_former_sibling_and_refunds` (`:4026`): how a commit is held, the document rewritten, and the refusal observed.
- `blob_refcount_for_test` (`meta.rs:2069`): the `blob_refs` query shape.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0033/DT-010: A refused commit leaves no unreferenced blob
- Given `/deck.pptx` converting to text `# deck\n` and one picture / When the document is rewritten while the commit is held, then released / Then the error message is `Document changed during conversion: /deck.pptx` / And `blob.exists(<text sha>)` and `blob.exists(<picture sha>)` are both `false`.
- Red before the change because: both `exists` return `true`.

### SPEC-0033/DT-011: A shared hash is kept
- Given `/copy.md` with bytes exactly `# deck\n` / When the commit is refused as in DT-010 / Then `blob.exists(<text sha>)` is `true` and `/copy.md` reads `# deck\n`.

### SPEC-0033/DT-012: The refusal is unchanged
- Then code `ERR_INVALID_ARGUMENT`, message exactly `Document changed during conversion: /deck.pptx`.

### SPEC-0033/DT-013: A failing delete does not mask the refusal
- Given a blob backend whose `delete` errors / When the commit is refused / Then the caller receives the DT-012 error, and the backend records 2 delete calls (text and picture).

### SPEC-0033/DT-014: Shared hashes released once; a committed set keeps its blobs
- Given a conversion whose text and picture bytes are identical / When refused / Then exactly 1 delete for that hash. Given an unrefused commit / Then 0 deletes and every stored hash `exists`.

## Constraints
- Only the three paths above change; no schema change, no new dependency.
- Never delete inside the database transaction; the release runs after `commit_artifact_set` returned.
- Do not touch `volume.rs` write paths or the purge sweep (BL-0056 owns the general case).

## Definition of done
- [ ] RED evidence: DT-010 red on the unchanged `capture.rs`, recorded in `.implement/US-0036-red.log`.
- [ ] DT-010..DT-014 green; `make check` green with `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true'`, no `skipping` DB suite.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none (0).
- [ ] Docs updated: none.
- [ ] Self review: full (time and space of the release: O(k) reference reads and deletes for k distinct hashes).
- [ ] Closes spec: DT-010..DT-014 and 8.1 green; DR-001..DR-007 cited here.
- [ ] One commit `US-0036: A refused artifact commit releases its blobs`, body `Spec: SPEC-0033` and the DR ids.
