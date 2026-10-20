# US-0060: large comparison bound, docs, and spec close

> Spec: SPEC-0045 (sdlc/specs/SPEC-0045-fs-diff-two-files/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0057, US-0058, US-0059
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
Prove the 10 second bound of the spec (section 7.1) for two 5 MiB files, bring README, AGENTS.md and `.agent_docs` to the new reality (109 tools, 45 `fs.*`, the new tool, the REST route), and close SPEC-0045 by running its full acceptance set and the requirement coverage audit.

## Requirements covered
- SPEC-0045 section 7.1 (non functional): WHEN two files of 5 MiB each, differing on 1000 lines, are compared THE product SHALL answer within 10 seconds.
- Closing audit of all 18 requirements FR-NEW-001 to FR-NEW-018 of SPEC-0045 (each is cited by an earlier story; this story verifies their tests all pass together).

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e` : all 49 acceptance tests of SPEC-0045 pass, E2E-230 finishing under 10 seconds wall clock. `rg -n "108 tools|108 .fs|the 108" README.md AGENTS.md .agent_docs` returns nothing, and `rg -n "fs.diff_files" README.md AGENTS.md .agent_docs/tools.md .agent_docs/api.md` returns at least one hit per file.

## Design excerpt
### Decisions
- (SPEC-0045) DEC-011: The comparison and the diff run inside `tokio::task::spawn_blocking`. **Obliged by:** spec 7.1 (10 seconds for two 5 MiB files) and the rust skill rule on CPU bound work. **Rationale:** Myers on tens of thousands of lines must not hold an async worker. **Alternatives considered:** inline (blocks the runtime). **Source:** rust skill. **Evidence:** n/a.
- (SPEC-0045) DEC-013: Count updates (108 to 109, 44 to 45 `fs.*`) land with US-0055 in code and tests, and with US-0060 in README, AGENTS.md and `.agent_docs`. **Obliged by:** FR-NEW-001. **Rationale:** `make check` must be green on main after each story; docs may trail by the later stories. **Alternatives considered:** docs in US-0055 (adds three files to the heaviest story). **Source:** design. **Evidence:** `rg -n "\b108\b"` output.
### Data model
None.
### Contracts
E2E-230 data: two files of exactly 5242880 bytes made of 80 byte lines (79 characters plus a line feed, 66 405 lines plus a shorter tail line, pad the last line to reach the byte count), differing on 1000 evenly spread lines; assert the answer has `identical` false, the diff contains 1000 removed and 1000 added body lines (`-` and `+` lines excluding the two header lines), and the call finished in under 10 seconds measured with `std::time::Instant`.
If the bound is missed in a debug test build, first confirm the diff runs in `spawn_blocking` (DEC-011); if Myers itself is too slow in debug, add `[profile.dev.package.similar] opt-level = 3` and `[profile.test.package.similar] opt-level = 3` to the workspace `Cargo.toml` and record that as a new DEC in `sdlc-design.md` (rust skill: optimize dependencies, own crates stay at opt-level 0).
Docs edits (counts change 108 to 109, `fs.*` 44 to 45): `README.md` lines 4 and 381; `AGENTS.md` lines 5, 85, 118, 156, 229; add `fs.diff_files` to the `fs.*` family list in `.agent_docs/tools.md` (arguments, answer, errors, authorization: both projects must be readable) and the `GET /api/fs/diff` route to `.agent_docs/api.md`. Add one dated line to `.agent_docs/architecture.md` only if it lists tool families.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/diff_files_e2e.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/api.md` | modify |
### Existing patterns
- Doc style: follow the existing table rows and sentences of each file; no dashes as punctuation.
- Timing assertion: wall clock with `std::time::Instant` around `server.fs_diff_files(..).await`.

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions of all tests (copied from the spec):

Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

### SPEC-0045/E2E-230: Large comparison within the time bound
- Level / Driver / Location: functional e2e / McpServer::fs_diff_files (the real handler) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: expected green on arrival if the comparison already runs inside `spawn_blocking` (US-0055); if it is red, fix the cause in `tools/diff_files.rs` (see Constraints)
- **Category:** edge. **Scenario:** SC-001. **Requirements:** Section 7.1, FR-NEW-001. **Priority:** Medium
- **Given** `alpha:/p1.txt` and `alpha:/p2.txt`, each 5242880 bytes of 80 byte lines, differing on 1000 spread
  lines
- **When** Alice compares them
- **Then** a diff with 1000 changed lines is returned within 10 seconds, identical false
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

## Constraints
- Files not to touch: source files other than the e2e test file (unless the performance fallback above is needed, then `Cargo.toml` and `tools/diff_files.rs` only).
- Dependencies not to add: none.
- Patterns to avoid: weakening the 10 second bound; ignoring the test.
- Scope boundary: nothing new in behavior.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, except the tests marked "Expectation on arrival" above, whose green status on arrival is recorded as such.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: Every existing test passes unmodified; `make check` green.
- [ ] Docs updated: README.md, AGENTS.md, `.agent_docs/tools.md`, `.agent_docs/api.md` as listed under Contracts.
- [ ] Self review (full): every requirement above is demonstrably met by a named test; no `unwrap()` on a fallible path; no dash as punctuation in code, comments or docs; errors carry the exact texts of this story.
- [ ] Closes spec: the spec's full acceptance set is green (all 49 tests E2E-001 to E2E-230 listed in `sdlc/specs/SPEC-0045-fs-diff-two-files/sdlc-spec.md` section 9.1), and each of FR-NEW-001 to FR-NEW-018 is cited by at least one story with status landed (US-0055 to US-0059).
- [ ] One commit `US-0060: large comparison bound, docs, and spec close`, body `Spec: SPEC-0045` and the requirement ids.
