# US-0197: destinations inside the project unchanged and documentation

> Spec: SPEC-0086 (sdlc/specs/SPEC-0086-refuse-destination-outside-the-project/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0196
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
A destination inside the project, typed relative, absolute, with `.` segments, repeated slashes or a climb that comes back inside, resolves to the same folder for every format, the default destination is unchanged, a refused call changes nothing and a corrected one succeeds; the documentation states the one destination rule. This closes SPEC-0086.

## Requirements covered
- SPEC-0086/FR-NEW-002: WHEN a member extracts an archive with a destination that is not outside the project THE archive extraction SHALL write the entries under the folder that destination resolves to from the project root, and SHALL answer and audit that resolved folder as the destination.
- SPEC-0086/FR-NEW-006: THE archive extraction SHALL extract or refuse every call whose destination is absent or inside the project exactly as before this increment, except a destination whose first folder name begins with two dots (FR-NEW-001), and SHALL leave every other path option of every tool unchanged.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0086_` and `cargo test -p mcp-fs-core --lib --features rar -- e2e_0086_`: every test of SPEC-0086 passes. Real assertion: the files read back under the resolved folders and the audit trail of E2E-033.

## Design excerpt
### Decisions
- **DEC-001 (SPEC-0086):** one function `archive_single::resolve_destination(raw: &str) -> Result<String>`: NUL refused first (`path contains a NUL byte`), then a segment walk on `/` with a depth counter (`""` and `"."` stay, `".."` at depth 0 refused with `path escapes the volume root: <raw>`, `".."` otherwise up, any other segment down, dots inside a name included), then `PosixPath::normpath(&format!("/{raw}"))`; it never calls `SafetyManager::normalize_path`. **Obliged by:** SPEC-0086/FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001. **Rationale:** the walk is the repository's existing climb rule (`export.rs:119-126`, `archive.rs:455-476`); judging while walking refuses `a/../../x` and `/a/../../b` even though a clamp would land inside (spec D-6); `normpath` of a non climbing path stays under `/`, and skipping `normalize_path` drops its `starts_with("/..")` prefix test that refuses `..out` (`safety.rs:105`). Splitting on `/` only keeps a backslash a plain character (spec 3.2). **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/tools/export.rs:119-126`, `crates/core/src/safety.rs:99-111`, `crates/core/src/util/posix.rs:48-55`.
- **DEC-006 (SPEC-0086):** documentation in the closing story: `README.md` (destination rule), `AGENTS.md` (`archive_single.rs` entry), `.agent_docs/tools.md` (one destination rule for every format, order, `..out`, the old clamp sentence at `:217` removed), `.agent_docs/testing.md` (filter `e2e_0086_`). **Obliged by:** n/a (no code impact): project convention. **Rationale:** `.agent_docs/tools.md:217` says "the other formats clamp it". **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** as cited.
- **DEC-007 (SPEC-0086):** tests whose rule an earlier story built are green on arrival, each proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0180 to US-0182. **Source:** design. **Evidence:** n/a.
### Data model
None.
### Contracts
No production contract change. Documentation: `README.md:306-309` (destination rule), `AGENTS.md:137-140` (`archive_single.rs` entry: `resolve_destination` used by every format), `.agent_docs/tools.md:183-217` (one destination rule for every format, `..out` accepted, refusal right after the name checks; remove "the other formats clamp it" at `:217`; dated `(2026-11-09, SPEC-0086.)` paragraph), `.agent_docs/testing.md` (dated line: filter `e2e_0086_`).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_destination_e2e.rs` | modify |
| `README.md` | modify |
| `AGENTS.md` | modify |
| `.agent_docs/tools.md` | modify |
| `.agent_docs/testing.md` | modify |
### Existing patterns
Helpers live in `crates/core/src/tools/archive_testkit.rs` (SPEC-0084 DEC-008): `MOUNT` (`atlas`), `OWNER` (`mia@example.com`), `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (calls `f.state.authorize(MOUNT, OWNER)` then `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`), `read`, `exists`, `limit_remaining(f, n)`, `remaining_storage`, `allowance`, `held_reset`, `held` (0 when the hook is unset), `held_some` (fails when unset), `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal(v, n)`, `corrupt_refusal(v, format)`. Their models: the tests module of `crates/core/src/tools/archive_single.rs:297-319` (`fixture`, `seed`, `extract`, `read`), `:786-866` (`fixture_allowance`, `limit_remaining`, `remaining_storage`, `allowance`, `project_refusal`, `session_refusal`, `assert_extraction_bounded`). Test module declaration precedent: `#[cfg(test)] mod quota_e2e;` in `crates/core/src/tools/mod.rs:76`. Fixture builders: `build_tar` (`crates/core/src/tools/archive.rs:1297-1311`), `build_tar_gz` (`:1138-1160`), `xz` (`crates/core/src/tools/archive_single.rs:1700`), the x86 and SHA-256 encoders of `e2e_0082_015` and `e2e_0082_056` and the check type patch of `xz_0082_unknown_check_type_is_corrupt` (`archive_single.rs`, SPEC-0082 DEC-007). The climb walk to mirror: `ensure_entry_path_safe` (`crates/core/src/tools/archive.rs:455-476`) and `tools::export::ensure_no_escape` (`crates/core/src/tools/export.rs:119-126`). `PosixPath::normpath` (`crates/core/src/util/posix.rs:48-55`). "Nothing is written" reads `exists` on `/x`, `/uploads/pack` and every folder the test names, `quota::used_bytes`, `allowance(f)`, and `extraction_audit_lines`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0086/E2E-001: Relative destination inside the project, zip
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** High
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `out`
- **Then** `/out/a.txt` reads `hello`; the answer gives destination `/out`, 1 file written, 5 bytes written
- **And** the audit trail has one extraction line for `/uploads/pack.zip` naming destination `/out`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_001_relative_destination_inside_the_proj`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-002: Absolute destination inside the project, 7z
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-002, FR-NEW-006; **Priority:** High
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with destination `/out`
- **Then** `/out/a.txt` reads `hello`; the answer gives destination `/out`, 1 file written, 5 bytes written
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_002_absolute_destination_inside_the_proj`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-003: A climb that stays inside, compressed tar
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** Critical
- **Given** `/uploads/pack.tar.gz`
- **When** the member extracts it with destination `a/../b`
- **Then** `/b/a.txt` reads `hello`; the answer gives destination `/b`; `/a` does not exist
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_003_a_climb_that_stays_inside_compressed`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-004: An absolute climb that stays inside, every format
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** on a server with RAR support, `/uploads/pack.zip`, `/uploads/pack.tar`, `/uploads/pack.rar`, and `/uploads/a.txt.gz` holding `hello` compressed
- **When** the member extracts each with destination `/a/../b`, removing `/b` between calls
- **Then** each call succeeds with destination `/b` and `/b/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs (nested `#[cfg(feature = "rar")] mod archive_rar`)
- Function: `e2e_0086_004_an_absolute_climb_that_stays_inside`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-005: No destination keeps the default
- **Category:** happy; **Scenario:** SC-001; **Requirements:** FR-NEW-006; **Priority:** Medium
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with no destination
- **Then** `/uploads/pack/a.txt` reads `hello`; the answer gives destination `/uploads/pack`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_005_no_destination_keeps_the_default`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-006: Back to the root exactly
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** `/uploads/pack.7z`
- **When** the member extracts it with destination `a/..`
- **Then** `/a.txt` reads `hello`; the answer gives destination `/`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_006_back_to_the_root_exactly`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-007: Dot segments and repeated slashes
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-002; **Priority:** Medium
- **Given** `/uploads/pack.tar`
- **When** the member extracts it with destination `./out/./x`, then, after removing `/out`, with `out//x`
- **Then** both calls answer destination `/out/x` and `/out/x/a.txt` reads `hello`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_007_dot_segments_and_repeated_slashes`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-008: To the root, then down
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-001, FR-NEW-002; **Priority:** High
- **Given** `/uploads/pack.zip`
- **When** the member extracts it with destination `a/b/../../c`
- **Then** `/c/a.txt` reads `hello`; the answer gives destination `/c`
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_008_to_the_root_then_down`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0086/E2E-033: Refused twice, then corrected
- **Category:** side effect; **Scenario:** SC-002; **Requirements:** FR-NEW-003, FR-NEW-006; **Priority:** High
- **Given** a storage limit of 1 MB and `/uploads/pack.tar`
- **When** the member extracts it with destination `../../x`, again with `../../x`, then with `x`
- **Then** the first two calls are refused with `path escapes the volume root: ../../x` and leave the stored bytes, the used session allowance and the audit trail unchanged
- **And** the third succeeds with destination `/x`, `/x/a.txt` reads `hello`, the stored bytes grow by 5 and the audit trail gains exactly one extraction line
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` as both doors do / crates/core/src/tools/archive_destination_e2e.rs
- Function: `e2e_0086_033_refused_twice_then_corrected`
- Green on arrival: the rule is built by an earlier story or already holds; baseline run recorded, then the mutation below
- Cleanup: none, the fixture is dropped with the in memory store

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs. Each mutation below was reasoned to change the test's observable outcome; if a run shows it does not, find one that does and record both.

| Test | Mutation |
|---|---|
| E2E-001 | answer the raw destination instead of the resolved one in `finish_extraction`: the answer says `out`, not `/out`, the test fails |
| E2E-002 | refuse a destination that starts with `/` in `resolve_destination`: `/out` is refused, the test fails |
| E2E-003 | refuse any `..` segment in `resolve_destination`: `a/../b` is refused, the test fails |
| E2E-004 | the E2E-003 mutation: `/a/../b` is refused for every format, the test fails |
| E2E-005 | use the archive's folder as the default destination of a zip (the bare file rule): the answer gives `/uploads`, the test fails |
| E2E-006 | refuse a destination that resolves to `/`: `a/..` is refused, the test fails |
| E2E-007 | return `format!("/{raw}")` without `normpath`: the answer gives `/./out/./x`, the test fails |
| E2E-008 | refuse `..` when the depth is 1 or less (`depth <= 1` for `depth == 0`): `a/b/../../c` is refused at its second `..`, the test fails |
| E2E-033 | record the audit line before `resolve_destination` runs: the two refused calls leave extraction lines, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none
- No dash as punctuation in code, comments or docs
- Documentation describes what US-0194 to US-0196 built, nothing more

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival (none in this story), before the production code.
- [ ] Green on arrival tests (E2E-001, E2E-002, E2E-003, E2E-004, E2E-005, E2E-006, E2E-007, E2E-008, E2E-033): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` and `cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_` pass; named neighbours unchanged: `e2e_0080_061_destination_outside_the_project` (`archive_rar.rs:1283-1301`), the `../../x` tests of SPEC-0081 E2E-035, SPEC-0082 E2E-022 and SPEC-0083 E2E-032 (`archive_single.rs:1096-1097`, `:1533-1534`, `:2271-2272`), the path pinning tests `safety.rs:263-265` and `util/posix.rs:115-118`, every `e2e_0084_` and `e2e_0085_` test except the one this spec replaces; no change to the tool contract
- [ ] DEBT only: not applicable (FEAT).
- [ ] Docs updated: `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` as listed in Contracts.
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Closes spec: the full acceptance set of SPEC-0086 is green (`cargo test -p mcp-fs-core --lib -- e2e_0086_ && cargo test -p mcp-fs-core --lib --features rar -- e2e_0086_`), and every requirement of SPEC-0086 is cited by at least one story with status done or landed.
- [ ] One commit `US-0197: destinations inside the project unchanged and documentation`, body `Spec: SPEC-0086` and the requirement ids (SPEC-0086/FR-NEW-002, FR-NEW-006).
