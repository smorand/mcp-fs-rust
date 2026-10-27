# US-0102: upgrade tantivy to 0.27

> Spec: SPEC-0055 (sdlc/specs/SPEC-0055-datafusion-and-zstd-upgrade/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0101
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Move tantivy from 0.26 to 0.27 (`cargo update -p tantivy`), which removes the last zstd 0.13 consumer outside the datafusion path. DT-030 and the whole search suite must stay green with no edit. If DT-030 shows the 0.26 index no longer opens, stop: DR-011 makes it a declared break for the user.

## Requirements covered
- SPEC-0055/DR-002: The workspace SHALL depend on tantivy `0.27`.
- SPEC-0055/DR-010: The search family SHALL open a tantivy index directory written by tantivy 0.26 and answer the same query with the same top result.
- SPEC-0055/DR-011: IF DT-030 shows that a tantivy 0.26 index does not open under 0.27, THEN the change SHALL NOT land and the incompatibility SHALL be handled as a declared break decided by the user (Section 7).

## Independent Test
`cargo tree -i zstd@0.13.3 --all-features --depth 1` no longer lists `tantivy-sstable`; `cargo test -p mcp-fs-core --lib search` green including DT-030.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0055): Upgrade tantivy to 0.27. **Alternatives considered:** `default-features = false` without `zstd-compression` (rejected: it would change which stored field compressors can be read; the upgrade keeps all); staying on 0.26 with an ignore (rejected: the user asked to remove zstd 0.13). **Implemented by:** DR-002. **Evidence:** `Cargo.toml:88`; tantivy 0.27.0 CHANGELOG (breaking changes concern APIs this code does not use: it compiled unchanged).
- DEC-007 (SPEC-0055): `cargo update` is limited to the four packages (`cargo update -p datafusion -p arrow -p parquet -p tantivy`) so unrelated crates keep their locked versions. **Alternatives considered:** a full `cargo update` (rejected: the probe changed about 1900 lockfile lines and mixes unrelated upgrades into a security move). **Implemented by:** DR-001, DR-002. **Evidence:** probe diff on a scratch branch.

### Data model
No schema change. The only persisted data concerned is the tantivy search index directory, which must stay readable (DR-010).

### Contracts
All internal; the `db.*` and `search.*` tool contracts are invariant (DR-009, DR-010). Versions in force after this story chain: datafusion git revision `6f1436fe1407495019d8afec8a9f55663f418d9a`, arrow `60`, parquet `60`, tantivy `0.27`, zstd 0.14.3, zstd-safe 8.2.0, zstd-sys 2.1.2.

### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify (line 88) |
| `Cargo.lock` | regenerate for tantivy only |

### Existing patterns
`Cargo.toml:88` (the tantivy line), the DT-030 test from US-0101.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Success criterion of the DEBT: the existing suite (`cargo test --workspace --no-fail-fast` and `make check`) passes UNMODIFIED, tolerated exceptions: none

### SPEC-0055/DT-031: search e2e unmodified
- Level / Driver / Location: integration / direct call to the search backend / `crates/core/src/search/bm25_sqlite.rs` tests module and search e2e
- **Validates:** DR-009 on the search family
- Given the 161 tests of `crates/core/src/search/e2e.rs`, When they run on the new dependency set, Then all pass with no edit.

## Constraints
- Files not to touch: production source, `deny.toml`, `.cargo/audit.toml`.
- Dependencies not to add: none; `cargo update` limited to `-p tantivy` (DEC-007).
- Patterns to avoid: editing a search test or the DT-030 fixture to make it pass.
- Scope boundary: datafusion, arrow and parquet belong to US-0103.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (structural tests fail while the old form exists; a characterization test written on unchanged versions passes at once: say so).
- [ ] Every acceptance test above green; `make check` green except the `security` step before US-0103 (red on main today); main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none.
- [ ] Non regression: db tool results and search results byte identical (DR-009), tantivy 0.26 index still opens (DR-010).
- [ ] Docs updated: none beyond code comments.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0102: upgrade tantivy to 0.27`, body `Spec: SPEC-0055` and the requirement ids.

## Blocked
- Category: 1 (architecture not in `design.md`: a coupling between the stories of SPEC-0055).
- Question: US-0102 (tantivy 0.27 alone) cannot leave main green. May the tantivy bump move into US-0103, so that one story bumps tantivy, datafusion, arrow and parquet together?
- Evidence: after setting `Cargo.toml:88` to `tantivy = "0.27"`, `cargo update -p tantivy` fails with `failed to select a version for 'zstd-sys'`. The resolver output: `parquet v53.2.0` (the `parquet = "^53"` of `mcp-fs-core`) requires `zstd-sys >=2.0.0, <2.0.14`, and `zstd-safe 7.2.1` (via `zstd 0.13.3` of `datafusion 43`) requires `zstd-sys ^2.0.10`, while tantivy 0.27 needs `zstd 0.14`, whose `zstd-safe 8.x` needs `zstd-sys ^2.1`. Only one `zstd-sys` can be linked, so tantivy 0.27 and parquet 53 cannot coexist. The reverse order works: on a scratch branch, datafusion at the git revision with arrow 60 and parquet 60 plus `tantivy = "0.26"` resolved, and left zstd 0.13.3 only through `tantivy-sstable` (`cargo tree -i zstd@0.13.3`).
- Recommendation: re-slice so the order is datafusion, arrow and parquet first (US-0103 content, with tantivy 0.26 still resolving), then tantivy 0.27 (US-0102 content). The queue cannot reorder rows and `Depends on` cannot be edited here, so the practical fix is to run US-0103's content first by editing the queue (a `/sdlc-design` amendment of SPEC-0055 stories) or to merge US-0102 into US-0103.
- Default I would apply: merge: US-0103 also carries DR-002 and DT-031, US-0102 is set to `skipped` as obsolete, and US-0103 depends on US-0101.
- Round log: no code change survived; `Cargo.toml` and `Cargo.lock` were restored.

## Superseded
Merged into US-0103 on 2026-10-27 by the user's decision ("go" on the re-slice proposal above). Status `skipped`: no work is left in this story.
