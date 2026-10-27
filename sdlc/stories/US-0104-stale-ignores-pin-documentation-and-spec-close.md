# US-0104: stale ignores, pin documentation and spec close

> Spec: SPEC-0055 (sdlc/specs/SPEC-0055-datafusion-and-zstd-upgrade/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0103
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
Remove the two stale audit ignores (declared test exception), correct the reason text of any ignore whose version changed, write `.agent_docs/dependencies.md` and index it in `AGENTS.md`, confirm BL-0089 exists, and close the spec: full acceptance set and `make check` green.

## Requirements covered
- SPEC-0055/DR-006: The advisory acceptance file SHALL NOT ignore `RUSTSEC-2024-0384` or `RUSTSEC-2024-0436`, and every remaining ignore SHALL match an advisory present in the audited graph.
- SPEC-0055/DR-007: WHEN the workspace depends on a git revision, THE repository SHALL carry `.agent_docs/dependencies.md` stating the revision, the reason, the single allowed source and the exit condition (BL-0089), and `AGENTS.md` SHALL index it.
- SPEC-0055/DR-008: The change SHALL leave a backlog entry `sdlc/backlog/BL-0089-datafusion-from-crates-io.md` for the move to a released datafusion.
- SPEC-0055/DR-012: WHEN `make check` runs the security step, THE gate SHALL pass: `cargo audit` reports no vulnerability and no warning, and `cargo deny check` reports no error.

## Independent Test
`rg -n 'RUSTSEC-2024-0384|RUSTSEC-2024-0436' .cargo/audit.toml` finds nothing; `test -f .agent_docs/dependencies.md`; `make check` exit 0.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0055): Remove the two stale ignores and modify the one existing test that lists them (declared exception, Section 8.4). **Alternatives considered:** leaving the ignores (rejected: `cargo audit` may flag unused ignores and the recorded reason would be false). **Implemented by:** DR-006. **Evidence:** `.cargo/audit.toml:7-8`; `security_gate.rs:149-182`.
- DEC-009 (SPEC-0055): Record the pin in `.agent_docs/dependencies.md` and create BL-0089 for the exit. **Alternatives considered:** a comment in `Cargo.toml` only (rejected: no index, no exit tracking). **Implemented by:** DR-007, DR-008. **Evidence:** `AGENTS.md` documentation index.

### Data model
No schema change. The only persisted data concerned is the tantivy search index directory, which must stay readable (DR-010).

### Contracts
All internal; the `db.*` and `search.*` tool contracts are invariant (DR-009, DR-010). Versions in force after this story chain: datafusion git revision `6f1436fe1407495019d8afec8a9f55663f418d9a`, arrow `60`, parquet `60`, tantivy `0.27`, zstd 0.14.3, zstd-safe 8.2.0, zstd-sys 2.1.2.

### Paths
| Path | Action |
|---|---|
| `.cargo/audit.toml` | modify (remove 2 ignores, fix reason texts) |
| `crates/mcp-fs/tests/security_gate.rs` | modify (one declared test edit, new tests) |
| `.agent_docs/dependencies.md` | create |
| `AGENTS.md` | modify (index line) |

### Existing patterns
`.cargo/audit.toml:5-14`; `crates/mcp-fs/tests/security_gate.rs:149-182`; the documentation index of `AGENTS.md` (line 242); `.agent_docs/search.md` as a model for a short `.agent_docs` file.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Success criterion of the DEBT: the existing suite (`cargo test --workspace --no-fail-fast` and `make check`) passes UNMODIFIED, tolerated exceptions: one: `crates/mcp-fs/tests/security_gate.rs::accepted_advisories_are_recorded_once_with_a_reason` (line 149): remove `RUSTSEC-2024-0384` and `RUSTSEC-2024-0436` from the expected id list. Justification (design Section 8.4): DR-006 removes exactly those two ignores because `instant` and `paste` leave the dependency graph with datafusion 55.

### SPEC-0055/DT-023: no stale ignore
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** DR-006 : the raw text of `.cargo/audit.toml` contains neither `RUSTSEC-2024-0384` nor `RUSTSEC-2024-0436`, every id still listed appears in the output of `cargo audit --json` as an ignored or present advisory, and the stderr of `cargo deny check` does not contain `advisory-not-detected`. **Method:** text parse plus two commands.

### SPEC-0055/DT-025: pin documented
- Level / Driver / Location: structural / file and command assertion / `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** DR-007, DR-008 : `.agent_docs/dependencies.md` exists and names the revision and `BL-0089`; `AGENTS.md` links it; `sdlc/backlog/BL-0089-datafusion-from-crates-io.md` exists. **Method:** file assertions.

## Constraints
- Files not to touch: `Cargo.toml`, `Cargo.lock`, production source.
- Dependencies not to add: none.
- Patterns to avoid: weakening any security gate test beyond the declared edit.
- Scope boundary: nothing else.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (structural tests fail while the old form exists; a characterization test written on unchanged versions passes at once: say so).
- [ ] Every acceptance test above green; `make check` green except the `security` step before US-0103 (red on main today); main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: one: `crates/mcp-fs/tests/security_gate.rs::accepted_advisories_are_recorded_once_with_a_reason` (line 149): remove `RUSTSEC-2024-0384` and `RUSTSEC-2024-0436` from the expected id list. Justification (design Section 8.4): DR-006 removes exactly those two ignores because `instant` and `paste` leave the dependency graph with datafusion 55..
- [ ] Non regression: db tool results and search results byte identical (DR-009), tantivy 0.26 index still opens (DR-010).
- [ ] Docs updated: .agent_docs/dependencies.md and AGENTS.md.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] Closes spec: the spec's full acceptance set green (DT-001 to DT-042), the non regression command green, and every requirement DR-001 to DR-012 cited by at least one story with status done or landed.
- [ ] One commit `US-0104: stale ignores, pin documentation and spec close`, body `Spec: SPEC-0055` and the requirement ids.
