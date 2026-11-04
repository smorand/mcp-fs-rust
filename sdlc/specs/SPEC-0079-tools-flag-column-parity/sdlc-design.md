# Test the flag column of tools.md against the contract annotations: technical debt design

> Id: SPEC-0079
> Nature: DEBT
> Depth: S
> Depth evidence: 1 file, 1 debt item, no contract change, no persisted change
> Status: implemented
> Generated on: 2026-11-03
> Behavior change: none
> From backlog: BL-0115
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Debt
### DBT-001: the human copy of the annotations is untested
- **Current structure:** `.agent_docs/tools.md` carries 116 table rows (`:23` to `:402`, recounted with `rg -c '^\| `(fs|admin|git)\.' .agent_docs/tools.md`), each with a flags cell (`D`, `RO`, `I`, `OW`, each `!` negated; legend `:15`). `tool-contract-golden.json` holds the 116 tools with `annotations`, compared with the registry at `crates/core/src/tools/contract_golden.rs:81`. Nothing compares the table with the golden. Existing doc parity tests (`:461`, `:497`, `:507`) check counts and `git.*` names only.
- **Cost paid today:** `git.remote_push_tag` drifted to `!D` after SPEC-0052 and nobody noticed for days (BL-0115).
- **Target structure:** a test in `contract_golden.rs` parses the table and compares each row with the golden annotations, with a verdict function so a negative test proves the guard fires (pattern of `:443` and `:507`).
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `crates/core/src/tools/contract_golden.rs` | add parser, verdict function, 4 tests | 1 file |

**Non source occurrences:** `.agent_docs/tools.md` read only; `.agent_docs/testing.md` untouched.
**Contracts touched:** none. The golden JSON and the table are read, never written.
**Guardrail coverage:** the golden tests in the same file (`:138` and others). **Test command:** `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace`
**Total:** 1 file, 0 renames.

## 3. Class sweep
Not applicable (`Security: n/a`).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: Flag column parity test
> The contract test module SHALL fail when the flags cell of any `fs.*`, `admin.*` or `git.*` row of `.agent_docs/tools.md` differs from the flags implied by that tool's golden `annotations`.
- **Rule:** `D` iff `destructiveHint` is true (absent counts false); `RO` iff `readOnlyHint`; `I` iff `idempotentHint`; `OW` iff `openWorldHint`. A row is written `D,RO,I,OW` with `!` before each false flag, except that a row whose `RO` is true omits the `D`/`!D` flag (`RO,I,!OW`).
- **Row selection:** a line whose trimmed text starts with ``| ` `` and whose first cell names a tool starting `fs.`, `admin.` or `git.`; the flags cell is the second to last cell.

#### DR-002 [EARS-U]: Name parity
> The same test SHALL fail when the set of tool names in the table differs from the golden set or a name appears twice.

#### DR-003 [EARS-UB]: No silent parse
> The parser SHALL NOT skip a row whose flags cell it cannot read; it SHALL fail naming the tool.

### 4.2 Invariance
#### DR-004 [EARS-U]: Behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract touched.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

## 6. Decisions
- **DEC-001:** test only, the column is not generated. **Alternatives considered:** generate the column from the golden (changes the docs workflow, needs a generator step). **Rationale:** minimum that closes the drift. **Implemented by:** DR-001. **Evidence:** `sdlc/backlog` note of BL-0115.
- **DEC-002:** expected flags come from the golden JSON, not the live registry. **Alternatives considered:** the live registry (the golden test at `contract_golden.rs:81` already ties both). **Rationale:** the golden is the reviewed contract. **Implemented by:** DR-001.
- **Assumption:** the 4 `search.*` tools are out of scope: absent from the golden (116 tools) and from the table, documented in `.agent_docs/search.md`.

## 7. Declared breaks
None.

## 8. Tests
### 8.1 Non regression
The existing test suite passes **unmodified**:
    export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test --workspace

### 8.2 Compatibility tests
None.

### 8.3 Structural tests
#### DT-001: every row matches the golden
- **Validates:** DR-001. Given the real table and golden, When each row is compared, Then no mismatch is reported (116 rows compared).
#### DT-002: names match
- **Validates:** DR-002. Given the real table and golden, Then table names equal golden names, no duplicates.
#### DT-003: the guard fires
- **Validates:** DR-001, DR-002. Given the real table text with `git.remote_push_tag` flipped from `!D` to `D`, Then the verdict names that tool and both flag strings; Given a row removed, Then the verdict lists it as missing; Given a row duplicated, Then it lists the duplicate.
#### DT-004: malformed cell
- **Validates:** DR-003. Given a row whose flags cell is `D,XX`, Then the parser error names the tool.
- **Red evidence:** before merge, flip one flag in `tools.md`, run DT-001, expect failure naming the tool, restore.

### 8.4 Existing tests to modify
None.

## 9. Implementation order
1. One story: parser + verdict + DT-001..004 in `contract_golden.rs`; mutation check on `tools.md`; `make check`.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline checklist, depth S) |

**Amendments applied:** none

## Design additions (2026-11-03)
- **File structure impact:** `crates/core/src/tools/contract_golden.rs` modified (one parser `documented_flags`, one pure function `flag_column_verdict(table_text, golden_tools) -> Result<(), String>`, four tests next to `e2e_new_957..959`, reusing the `TOOLS_MD` const and `frozen_tools()`). No other file. Affected existing tests: none. Docs: one line in `.agent_docs/testing.md` is not needed; `.agent_docs/tools.md` is read only.
- **Test strategy:** DT-001..DT-004 are unit level tests, direct calls, in `crates/core/src/tools/contract_golden.rs`, all in US-0159. DT-003 and DT-004 feed `flag_column_verdict` literal mutated text, so they never touch the real files.
- **DEC-003:** one story. **Obliged by:** DR-001..DR-003. **Alternatives considered:** parser then verdict as two stories (rejected: neither is verifiable alone). **Rationale:** 1 file, 4 tests, one coherent guard; tier 2 enough.
