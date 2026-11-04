# US-0159: test the flag column of tools.md against the contract

> Spec: SPEC-0079 (sdlc/specs/SPEC-0079-tools-flag-column-parity/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
`.agent_docs/tools.md` repeats the `D`, `RO`, `I`, `OW` annotations of 116 tools and nothing checks it; `git.remote_push_tag` drifted unnoticed. Add a test that compares every row with `tool-contract-golden.json`.

## Requirements covered
- SPEC-0079/DR-001: The contract test module SHALL fail when the flags cell of any `fs.*`, `admin.*` or `git.*` row of `.agent_docs/tools.md` differs from the flags implied by that tool's golden `annotations`. Rule: `D` iff `destructiveHint` true (absent is false); `RO`, `I`, `OW` iff `readOnlyHint`, `idempotentHint`, `openWorldHint`. Written `D,RO,I,OW` with `!` before each false flag, except a row with `RO` true omits the `D`/`!D` flag (`RO,I,!OW`). Rows: trimmed line starting ``| ` `` whose first cell starts `fs.`, `admin.` or `git.`; the flags cell is the second to last cell.
- SPEC-0079/DR-002: The same test SHALL fail when the set of tool names in the table differs from the golden set or a name appears twice.
- SPEC-0079/DR-003: The parser SHALL NOT skip a row whose flags cell it cannot read; it SHALL fail naming the tool.
- SPEC-0079/DR-004: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
`cargo test -p mcp-fs-core --lib dt_0079` is green (4 tests). Red evidence: change `git.remote_push_tag`'s `D` to `!D` (or the reverse) in `.agent_docs/tools.md`, DT-001 fails naming that tool; restore.

## Design excerpt
### Decisions
- DEC-003 (SPEC-0079): one story; parser and verdict are not verifiable alone.
- DEC-001/DEC-002 (SPEC-0079): test only, the column is not generated; expected flags come from the golden JSON.
### Data model
None.
### Contracts
Test only: `fn flag_column_verdict(table: &str, golden: &[serde_json::Value]) -> Result<(), String>` (pure, error text names each tool with the table flags and the expected flags). Not part of the tool contract.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/contract_golden.rs` | modify |
### Existing patterns
`documented_git_tools` (`:432`), `documentation_parity_verdict` (`:443`), tests `e2e_new_957..959` (`:461`, `:497`, `:507`); `TOOLS_MD` (`:406`) and `frozen_tools()` (`:37`). Golden entries: `{"name", "annotations": {"destructiveHint"?, "readOnlyHint", "idempotentHint", "openWorldHint"}}`; read only tools omit `destructiveHint`.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0079/DT-001: every row matches the golden
- Level / Driver / Location: unit / direct call / `contract_golden.rs` `dt_0079_001_*`
- Given the real `tools.md` and golden, When `flag_column_verdict` runs, Then `Ok` and 116 rows compared (assert the count).
### SPEC-0079/DT-002: names match
- Given the real files, Then table names equal golden names with no duplicate (`dt_0079_002_*`).
### SPEC-0079/DT-003: the guard fires
- Given the real table text with the `git.remote_push_tag` flags cell flipped on `D`, Then the error contains `git.remote_push_tag` and both flag strings; Given one row removed, Then it lists that tool as missing; Given one row duplicated, Then it lists the duplicate (`dt_0079_003_*`).
### SPEC-0079/DT-004: malformed cell
- Given a row for `fs.read` whose flags cell is `D,XX`, Then the error contains `fs.read` (`dt_0079_004_*`).

## Constraints
- Files not to touch: `tools.md`, the golden JSON, production code.
- Dependencies not to add: any.
- Scope boundary: no generation of the column; `search.*` out of scope.

## Definition of done
- [ ] RED evidence: DT-001 fails on a flipped flag in `tools.md` (record the output), then restore.
- [ ] Four tests green; `make check` green.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0).
- [ ] Docs: nothing to update.
- [ ] Self review: tests assert real strings, not the function's own output.
- [ ] Closes spec: DR-001..DR-004 covered by this story.
- [ ] One commit `US-0159: test the flag column of tools.md against the contract`, body `Spec: SPEC-0079`.
