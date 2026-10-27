# US-0103: upgrade datafusion, arrow and parquet and remove the old zstd

> Spec: SPEC-0055 (sdlc/specs/SPEC-0055-datafusion-and-zstd-upgrade/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0102
> min_tier: 1
> Files touched: 5
> Closes spec: no

## Objective
Move datafusion to the pinned git revision with arrow 60 and parquet 60, rename the option type, allow the single git source in `deny.toml`, and prove the old zstd is gone. DT-001 to DT-011 recorded in the first story are the oracle: where a datafusion difference changes an output, restore the old output in code and never edit an expectation (DEC-006). The security step of `make check` becomes green here. Decision density is high (lockfile regeneration, output restoration), hence tier 1.

## Requirements covered
- SPEC-0055/DR-001: The workspace SHALL depend on datafusion from the git repository `https://github.com/apache/datafusion` at revision `6f1436fe1407495019d8afec8a9f55663f418d9a`, on arrow `60` and on parquet `60`.
- SPEC-0055/DR-003: The `db` tool family SHALL register NDJSON files with `JsonReadOptions`.
- SPEC-0055/DR-004: The lockfile SHALL NOT contain a `zstd` package below 0.14.3, a `zstd-safe` package below 8.2.0 or a `zstd-sys` package below 2.1.2.
- SPEC-0055/DR-005: The dependency policy file SHALL allow the git source `https://github.com/apache/datafusion` and no other git source.
- SPEC-0055/DR-009: The system SHALL produce, for every input covered by the existing test suite and by the characterization tests DT-001 to DT-011, byte identical observable output before and after this change, including the column type names `db.schema` reports.
- SPEC-0055/DR-012: WHEN `make check` runs the security step, THE gate SHALL pass: `cargo audit` reports no vulnerability and no warning, and `cargo deny check` reports no error.

## Independent Test
`rg -c 'name = "zstd' Cargo.lock` lists only zstd 0.14.3, zstd-safe 8.2.0, zstd-sys 2.1.2; `cargo audit` and `cargo deny check` exit 0; DT-001 to DT-011 green unmodified; `make check` exit 0.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0055): Depend on datafusion by git revision `6f1436fe1407495019d8afec8a9f55663f418d9a` (apache/datafusion main, committed 2026-10-27, workspace version 55.1.0) with arrow 60 and parquet 60. **Alternatives considered:** (a) datafusion 55.2.0 from crates.io plus advisory ignores (rejected: it keeps zstd 0.13 because arrow 59 requires `zstd ^0.13`; nothing is removed); (b) vendored `[patch]` copies of `arrow-ipc` and `parquet` with zstd 0.14 (rejected: large copies in the repository); (c) waiting for a release (rejected: the audit gate is red on main now). **Rationale:** the only way to remove the vulnerable code today, chosen by the user. **Implemented by:** DR-001. **Evidence:** `Cargo.toml:123`; crates.io `datafusion` max version 55.2.0; apache/datafusion `Cargo.toml` (`arrow = 60.0.0`, `zstd = 0.14`).
- DEC-002 (SPEC-0055): arrow and parquet are direct dependencies at 60 so they unify with the versions datafusion main uses. **Alternatives considered:** letting datafusion bring them in (rejected: `crates/core/Cargo.toml:89-90` depends on both directly). **Implemented by:** DR-001. **Evidence:** `Cargo.toml:124-125`.
- DEC-004 (SPEC-0055): Allow exactly one git source in `deny.toml`, the repository URL without a wildcard; the revision stays pinned by `Cargo.toml` and `Cargo.lock`. **Alternatives considered:** `unknown-git = "warn"` (rejected: weakens the gate for every future dependency). **Implemented by:** DR-005. **Evidence:** `deny.toml:40-42`.
- DEC-006 (SPEC-0055): The characterization tests are written first, recorded on datafusion 43, and become the oracle: where datafusion 55 differs, the code restores the old output; a test expectation is never edited. A difference that cannot be restored is an F finding for the user (a declared break). **Alternatives considered:** accepting new outputs (rejected: smuggles behaviour change into a DEBT). **Implemented by:** DR-009. **Evidence:** `crates/core/src/tools/db.rs:585-703` (8 tests, thin).
- DEC-007 (SPEC-0055): `cargo update` is limited to the four packages (`cargo update -p datafusion -p arrow -p parquet -p tantivy`) so unrelated crates keep their locked versions. **Alternatives considered:** a full `cargo update` (rejected: the probe changed about 1900 lockfile lines and mixes unrelated upgrades into a security move). **Implemented by:** DR-001, DR-002. **Evidence:** probe diff on a scratch branch.
- DEC-008 (SPEC-0055): Security tests are structural plus a rejection guard because the effect of the class (uninitialized or freed memory exposure) is not observable by an assertion. The observable instrumentation is DR-004 and DR-012. **Alternatives considered:** a fuzzing harness (rejected: out of proportion for a dependency bump). **Implemented by:** DR-004, DR-012. **Evidence:** advisories text (`cargo audit` output).

### Data model
No schema change. The only persisted data concerned is the tantivy search index directory, which must stay readable (DR-010).

### Contracts
All internal; the `db.*` and `search.*` tool contracts are invariant (DR-009, DR-010). Versions in force after this story chain: datafusion git revision `6f1436fe1407495019d8afec8a9f55663f418d9a`, arrow `60`, parquet `60`, tantivy `0.27`, zstd 0.14.3, zstd-safe 8.2.0, zstd-sys 2.1.2.

### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify (lines 123-125) |
| `Cargo.lock` | regenerate for datafusion, arrow, parquet only |
| `crates/core/src/tools/db.rs` | modify (line 93 rename; restore old outputs if a characterized one changed) |
| `deny.toml` | modify ([sources] allow-git) |
| `crates/mcp-fs/tests/security_gate.rs` | modify (new tests only) |

### Existing patterns
`Cargo.toml:123-125`; `crates/core/src/tools/db.rs:85-97` (the registration of the three formats); `deny.toml:40-42`; the lock reading helper `lock_packages` at `crates/mcp-fs/tests/security_gate.rs:41` and the audit test at `:103`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Success criterion of the DEBT: the existing suite (`cargo test --workspace --no-fail-fast` and `make check`) passes UNMODIFIED, tolerated exceptions: none

### SPEC-0055/DT-020: no old zstd in the lockfile
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** DR-004 : no `zstd` below 0.14.3, `zstd-safe` below 8.2.0, `zstd-sys` below 2.1.2. **Method:** parse `Cargo.lock` as `lock_packages` does in `security_gate.rs:41`.

### SPEC-0055/DT-021: audit reports nothing
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** DR-012 : `cargo audit --json` has zero vulnerabilities and an empty warnings object (the existing `cargo_audit_reports_no_vulnerability_and_no_warning`, `security_gate.rs:103`, goes green; no edit).

### SPEC-0055/DT-022: one git source allowed
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** DR-005 : `deny.toml` contains exactly one `allow-git` entry equal to `https://github.com/apache/datafusion`. **Method:** text parse in `security_gate.rs`.

### SPEC-0055/DT-024: renamed option type
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** DR-003 : `rg -n 'NdJsonReadOptions' crates` finds no occurrence. **Method:** grep assertion in `security_gate.rs` style test.

### SPEC-0055/DT-042: the security step passes
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Covers:** DR-012, all occurrences. **Driver:** the existing `make_security_passes` (`security_gate.rs:21`) goes green on the new set; no edit.

## Constraints
- Files not to touch: `.cargo/audit.toml` (the ignore cleanup is US-0104), the characterization tests of US-0101 (never edit an expectation).
- Dependencies not to add: none besides the git source for datafusion (DEC-001); `cargo update` limited to `-p datafusion -p arrow -p parquet` (DEC-007).
- Patterns to avoid: a wildcard git allowance; `unknown-git = "warn"`.
- Scope boundary: documentation and ignore cleanup belong to US-0104. If an output cannot be restored, stop and ask (declared break).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (structural tests fail while the old form exists; a characterization test written on unchanged versions passes at once: say so).
- [ ] Every acceptance test above green; `make check` green except the `security` step before US-0103 (red on main today); main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none.
- [ ] Non regression: db tool results and search results byte identical (DR-009), tantivy 0.26 index still opens (DR-010).
- [ ] Docs updated: none beyond code comments.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0103: upgrade datafusion, arrow and parquet and remove the old zstd`, body `Spec: SPEC-0055` and the requirement ids.
