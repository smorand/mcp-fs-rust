# US-0063: other diffs unchanged, scale, docs, spec close

> Spec: SPEC-0046 (sdlc/specs/SPEC-0046-diff-context-lines/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0061, US-0062
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
Prove that write, edit and multi edit still return 3 unchanged lines, that the largest accepted size works on files at the size limit, document the option, and close SPEC-0046 by running its full acceptance set.

## Requirements covered
- SPEC-0046/FR-NEW-008: The product SHALL NOT change the number of unchanged lines (3) shown in the diff returned by writing, editing and multi editing a file.
- Closing audit of all requirements FR-NEW-001 to FR-NEW-010 and FR-MOD-001 of SPEC-0046 (each is cited by US-0061 or US-0062; this story verifies their tests all pass together).

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e` and the dataplane diff tests: every E2E-001 to E2E-049 and the 48 existing tests pass. `rg -n "context_lines" README.md .agent_docs/tools.md .agent_docs/api.md` returns at least one hit per file.

## Design excerpt
### Decisions
- DEC-007 (SPEC-0046): `DEFAULT_CONTEXT` and `core::diff::unified()` stay untouched for write, edit and multi edit (`core/diff.rs:11, 48-51`); this story only proves it.
- DEC-009 (SPEC-0046): README and `.agent_docs` carry the frozen description text.
- Scale: E2E-031 builds two 5242880 byte files of 2621440 lines with `put()`; the diff already runs in `spawn_blocking` (SPEC-0045 DEC-011). If the test is red, do not change production code without first recording a new DEC in `sdlc-design.md`.
### Data model
None.
### Contracts
None.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/diff_files_e2e.rs` | modify (E2E-020, 024, 025, 031) |
| `README.md` | modify (line 276, the sentence about 3 lines of context) |
| `.agent_docs/tools.md` | modify (the `fs.diff_files` row near line 100 and the paragraph near line 102: add `context_lines`, default 3, 0 to 10000000, refusal text) |
| `.agent_docs/api.md` | modify (the `diff` row near line 82 and the paragraph near line 90: add `context_lines`, first occurrence rule, empty refused) |
### Existing patterns
- Doc style: follow the existing rows and sentences of each file; no dashes as punctuation.
- Test style: as US-0061; read a file before editing it (read before edit rule), as the existing edit tests in `tools/diff_files_e2e.rs` do.

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions (copied from the spec). `|` below stands for a line feed in expected diffs; the real answer separates lines with line feeds. Expected values in the form `--- alpha:/a.txt|+++ ...` are the exact `diff` text. The identical indicator is the answer's `identical` field. Each project and file named below is created by the test itself with `put()`; Alice is a member of alpha and beta, Mallory is not a member of alpha (add members with `f.state.admin.add_member`, as `world()` does).

Fixtures (every line is `l<number>` followed by a line feed unless stated):
- **P1:** project `alpha`: `/a.txt` = `l1`..`l10`; `/b.txt` = the same with line 6 replaced by `L6`; `/same.txt` = identical to `/a.txt`.
- **P2:** project `alpha`: `/c.txt` = `l1`..`l14`; `/d.txt` = the same with line 3 replaced by `L3` and line 12 by `L12`.
- **P3:** project `alpha`: `/e.txt` = `l1`..`l10`; `/f.txt` = same with line 1 replaced by `L1`.
- **P5:** alpha: `/g.txt` = `l1`..`l15`; `/h.txt` = same with line 3 replaced by `L3` and line 10 by `L10`.
- **P6:** alpha: `/i.txt` = `l1`..`l16`; `/j.txt` = same with line 3 replaced by `L3` and line 11 by `L11`.
- **P4:** project `beta`: `/b.txt` = same content as alpha `/b.txt`.
- Actor: `Alice`, member of `alpha` and `beta`; `Mallory` is not a member of `alpha`.
A diff is shown below with `|` as line separator; the real answer separates lines with a line feed.

Refusal message everywhere: `ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000` (no value echoed).

### SPEC-0046/E2E-020: write after comparison
- Level / Driver / Location: `McpServer` write, edit and multi edit handlers (exact names in `mcp/server.rs`) plus `fs_diff_files`, in `crates/core/src/tools/diff_files_e2e.rs`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** P1, Alice has compared `/a.txt` with `/b.txt` with size 0
- **When** Alice reads then overwrites `/a.txt` changing line 6 to `X6`
- **Then** the diff returned by the write shows 3 unchanged lines before and after (`@@ -3,7 +3,7 @@`)

### SPEC-0046/E2E-024: edit after comparison
- Level / Driver / Location: `McpServer` write, edit and multi edit handlers (exact names in `mcp/server.rs`) plus `fs_diff_files`, in `crates/core/src/tools/diff_files_e2e.rs`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** P1, Alice has compared `/a.txt` with `/b.txt` with size 0 and read `/a.txt`
- **When** Alice edits `/a.txt` replacing `l6` by `X6`
- **Then** the diff returned shows 3 unchanged lines before and after

### SPEC-0046/E2E-025: multi edit after comparison
- Level / Driver / Location: `McpServer` write, edit and multi edit handlers (exact names in `mcp/server.rs`) plus `fs_diff_files`, in `crates/core/src/tools/diff_files_e2e.rs`
- **Category:** state transition. **Scenario:** SC-001. **Requirements:** FR-NEW-008
- **Given** as E2E-024
- **When** Alice applies a multi edit replacing `l6` by `X6`
- **Then** the diff returned shows 3 unchanged lines before and after

### SPEC-0046/E2E-031: Alice compares two files of 5242880 byte
- Level / Driver / Location: `McpServer` write, edit and multi edit handlers (exact names in `mcp/server.rs`) plus `fs_diff_files`, in `crates/core/src/tools/diff_files_e2e.rs`
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-003, FR-NEW-005
- **Given** P1, Alice (or her program client)
- **When** Alice compares two files of exactly 5242880 bytes each, made of 2621440 lines, each line being `x` followed by a line feed, the second file differing only by line 1 being `y`, with size 10000000
- **Then** the diff is returned without failure, in one block `@@ -1,2621440 +1,2621440 @@`

## Constraints
- Files not to touch: production code under `crates/core/src` (unless E2E-031 shows a defect: then record a DEC first), contract files.
- Dependencies not to add: none.
- Scope boundary: nothing remains after this story.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the 48 existing tests of `crates/core/src/tools/diff_files_e2e.rs` and every other existing test pass unmodified; write, edit and multi edit diffs still show 3 unchanged lines.
- [ ] Spec closing: every E2E- of SPEC-0046 green, and every requirement of SPEC-0046 cited by at least one story with status done or landed.
- [ ] Docs updated: README.md, `.agent_docs/tools.md`, `.agent_docs/api.md`; `AGENTS.md` counts unchanged (109 tools).
- [ ] Self review: every EARS statement above traced to a test; no `unwrap()` on a fallible path; no dash as punctuation in comments or docs; comments explain why.
- [ ] One commit `US-0063: other diffs unchanged, scale, docs, spec close`, body `Spec: SPEC-0046` and the requirement ids.
