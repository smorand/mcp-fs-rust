# US-0225: code search answers source of any nesting

> Spec: SPEC-0092 (sdlc/specs/SPEC-0092-bound-nesting-depth-in-pdf-extraction-and-code-search/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0224
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
Searching a folder for a definition or its references ends the server today when one source file nests ten thousand constructs: the scan of the syntax tree recurses once per level, and so does the search for a C definition's name. After this story both visits use an explicit stack in the same order, so any nesting is answered with exactly today's definitions and references, through both doors.

## Requirements covered
- SPEC-0092/FR-NEW-006 (partial: source files): THE product SHALL NOT end, restart or stop answering because a member asked for the text of a PDF or searched a folder holding a source file, whatever their nesting.
- SPEC-0092/FR-NEW-008: THE code search SHALL answer a definition or reference search over source files of any nesting with the same definitions and references, in the same order, with the same fields, as it answers today for the same files when it does not end the server.
- SPEC-0092/FR-NEW-009 (partial: the definition and reference search): THE product SHALL answer a PDF text extraction and a definition or reference search through the HTTP API with the same code, message, results and effects as through the assistant tool.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0092_sec_011 e2e_0092_sec_012 e2e_0092_sec_013 e2e_0092_003 e2e_0092_004 e2e_0092_010 e2e_0092_025 e2e_0092_026` and `cargo test -p mcp-fs-core --lib docs::symbols`: the 8 new tests and the 22 existing symbol tests pass. Real assertions: exact definitions (path, name, kind, line), references and their order, the HTTP answer equal to the MCP answer, the child exit status, the bystander read.

## Design excerpt
### Decisions
- **DEC-007 (SPEC-0092):** `symbols::walk` and `walk_descendants` become explicit stack loops: pop a node, visit it, push its named children in reverse so the leftmost is visited next; `walk_descendants` starts with the root's named children (the root itself is not visited). `name_of` stops the descendant visit at the first identifier found (today it keeps the first and ignores the rest: same result, less work). Pre order and results are identical to today's for every tree. **Obliged by:** SPEC-0092/FR-NEW-006, FR-NEW-008. **Rationale:** tree-sitter builds and walks 1 000 000 levels without recursion (spec 14.5 row 24); only our visit recursed.
- **DEC-008 (SPEC-0092):** `in_child(module, name, body)` (in `nesting_testkit.rs`): when the environment variable `MCPFS_CHILD_CASE` equals `name`, it runs `body` on a fresh `tokio` multi thread runtime of 2 workers with `thread_stack_size(2 * 1024 * 1024)` (the production default: `#[tokio::main]` sets no stack size, so std's 2 MiB applies to workers and blocking threads alike), then prints `MCPFS_CHILD_CASE_PASSED <name>` on stdout. Otherwise it spawns `std::env::current_exe()` with `--exact <module><name> --nocapture --test-threads 1`, the variable set and `RUST_MIN_STACK` removed, waits at most 120 s (killing the child past it), and asserts the exit status is success and stdout holds the marker; a failed assertion message carries the exit code or the signal (`std::os::unix::process::ExitStatusExt::signal`) and the child's last 4 096 bytes of stderr. The body's own assertions run in the child; a panic there fails the child, hence the parent. **Obliged by:** SPEC-0092/FR-NEW-006, spec D-14, 14.3 testability. **Rationale:** an abort on any thread ends the whole process, so only another process can observe it; re running the test binary for one case is the in tree pattern.
- **DEC-009 (SPEC-0092):** a test runs in a child exactly when its document or source aborts the process on today's code: E2E-015, 016, 017, 021, 022, 023, 024, 027, 028 and every E2E-SEC test; the others run in process (their documents are within the measured thresholds, so RED shows as a plain assertion). **Obliged by:** SPEC-0092/FR-NEW-006 (spec D-14). **Rationale:** RED evidence must be an assertion, not a crashed runner; child runs cost about one second each.
- **DEC-010 (SPEC-0092):** "the server is still up" is `still_up(&f, pid_before)`: within 1 s after the answer, `McpServer::new(state, "leo@example.com").fs_read` of `/notes/hello.txt` in `orbit` returns `hello`, and `std::process::id()` equals `pid_before` read before the call. E2E-SEC-001 and E2E-SEC-011 also run `bystander_loop` (a task reading every 50 ms until told to stop, each read asserted `hello` within 1 s) while the attacked call runs. `nesting_fixture()` is `archive_testkit::office_fixture()` plus `orbit` seeded with `f.seed_project("orbit", "leo@example.com")` and the note. **Obliged by:** SPEC-0092/FR-NEW-006, spec 9 header. **Rationale:** reads through the real handler, from another project and member.
- **DEC-015 (SPEC-0092):** timing bounds are asserted inside the child around the attacked call with `Instant`: E2E-SEC-004 under 2 s (refused at depth 65 after a load of 10 010 objects), E2E-SEC-011 under 10 s. **Obliged by:** SPEC-0092 7.1 (E2E-SEC-004, E2E-SEC-011). **Rationale:** the spec's figures, measured where the call runs.
### Contracts
- `fn walk<'t, F: FnMut(Node<'t>)>(root: Node<'t>, visit: &mut F)`: `let mut stack = vec![root]; while let Some(node) = stack.pop() { visit(node); let mut cursor = node.walk(); let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect(); stack.extend(children.into_iter().rev()); }`.
- `fn walk_descendants<'t>(node: Node<'t>, visit: &mut impl FnMut(Node<'t>) -> bool)`: same loop seeded with the node's named children (reversed), the node itself not visited; `visit` returns `false` to stop (used by `name_of` once it holds the first identifier).
- Answers unchanged: `core::fs_ops::find_definitions` (`{"definitions": [{path, name, kind, line}]}`), `find_references` (`{"references": [{path, line, kind}]}`, `name is required` for an empty name), `GET /api/fs/{mount}/find-definition`, `find-references` (`crates/core/src/api/dataplane.rs:152-153`).
- Test only: `nesting_testkit::deep_source(head: &str, open: char, middle: &str, close: char, n: usize, tail: &str) -> String`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/docs/symbols.rs` | modify |
| `crates/core/src/tools/nesting_testkit.rs` | modify (`deep_source`) |
| `crates/core/src/tools/nesting_e2e.rs` | modify |
| `crates/core/src/api/dataplane.rs` | modify (tests `e2e_0092_004`, `e2e_0092_026`, `e2e_0092_sec_013`) |
### Existing patterns
`walk`, `name_of`, `walk_descendants` (`crates/core/src/docs/symbols.rs:207-247`); the find routes' tests in `crates/core/src/api/dataplane.rs:2151`; `McpServer::fs_find_definition`, `fs_find_references` (`crates/core/src/mcp/server.rs:2034`, `:2051`); US-0222's `in_child` (for a test inside the dataplane tests module, `module` is `"api::dataplane::tests::"`).
### Fixtures
- `/src/mid.py` (E2E-003): `def f():\n    return ` + 1 000 `(` + `1` + 1 000 `)` + `\n`; E2E-004 adds a third line `f()`.
- `/src/order.py` (E2E-010): `total = 1\nprint(total)\ntotal += 2\n`; the recorded answer (spec 14.5 row 18): identifier references on lines 1, 2 and 3 in that order, kind `identifier`.
- `/src/deep.py` (E2E-SEC-011, E2E-025, E2E-026): `def f():\n    return ` + 100 000 `(` + `1` + 100 000 `)` + `\n`.
- `/src/deep.c` (E2E-SEC-012): `int f(void) { return ` + 100 000 `(` + `1` + 100 000 `)` + `; }`.
- `/src/deep.js` (E2E-SEC-013): `const g = ` + 100 000 `[` + `g` + 100 000 `]` + `;`.
Files are written with the volume client of `atlas` (`/src` created first). The HTTP tests use the dataplane `Harness` (project `proj`, member `owner@test.com`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0092/E2E-SEC-011: Definition search over 100 000 nested parentheses
- **Covers:** occurrence 5
- **Red before the fix because:** `Then the answer lists ...` fails: the process ends (14.5 row 19; 10 000 already ends it, row 18).
- **Given** `/src/deep.py`, about 200 000 bytes: `def f():` newline, `    return ` followed by 100 000 `(`, the digit `1`, 100 000 `)` and a newline
- **When** the member asks for the definition of `f` under `/src`, while `leo@example.com` reads `/notes/hello.txt` in a loop
- **Then** the answer lists exactly one definition: path `/src/deep.py`, name `f`, kind `function_definition`, line 1, within 10 seconds
- **And** every read of `leo@example.com` receives `hello`; the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::find_definitions(&c, "/src", "f", None)` inside `in_child`, with `bystander_loop` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_011_definition_search_over_100_000_ne`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 19)
- Note: the 10 seconds are measured around the call inside the child (DEC-015).
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-012: C definition whose name hides in a deep body
- **Covers:** occurrence 6
- **Red before the fix because:** `Then the answer lists ...` fails: the process ends (14.5 row 20). A C definition has no name field, so its whole subtree is scanned for the name (occurrence 6) once the outer scan (occurrence 5) reaches it; both go one level deeper per nesting level, so fixing occurrence 5 alone leaves this test red.
- **Given** `/src/deep.c` holding `int f(void) { return ` followed by 100 000 `(`, the digit `1`, 100 000 `)` and `; }`
- **When** the member asks for the definition of `f` under `/src`
- **Then** the answer lists exactly one definition: path `/src/deep.c`, name `f`, kind `function_definition`, line 1
- **And** the server is still up
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::find_definitions` inside `in_child` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_sec_012_c_definition_whose_name_hides_in`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 20); fixing `walk` alone leaves it red, `walk_descendants` must change too
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-SEC-013: Reference search through the HTTP API over 100 000 nested lists
- **Covers:** occurrence 5, the second door and another language
- **Red before the fix because:** `Then the answer lists ...` fails: the process ends (14.5 row 21).
- **Given** `/src/deep.js` holding `const g = ` followed by 100 000 `[`, `g`, 100 000 `]` and `;`
- **When** a client asks the HTTP API for the references to `g` under `/src`
- **Then** the answer lists exactly two references to `g`, both on line 1
- **And** the server is still up
- Level / Driver / Location: exploit (functional e2e) / HTTP door `GET /api/fs/{mount}/find-references` inside `in_child` / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0092_sec_013_reference_search_through_the_http`
- Runs in a child process: `nesting_testkit::in_child` (DEC-008); the assertions above run in the child, the parent asserts a normal exit and the marker line
- Red before the change because: the child ends by signal 6 (spec 14.5 row 21)
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-003: Definition in a moderately nested Python file
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/src/mid.py` holding `def f():` then `    return ` followed by 1 000 `(`, the digit `1` and 1 000 `)`
- **When** the member asks for the definition of `f` under `/src`
- **Then** the answer lists exactly one definition: path `/src/mid.py`, name `f`, kind `function_definition`, line 1
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::find_definitions` or `find_references` after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_003_definition_in_a_moderately_nested_pyt`
- Green on arrival: a nesting of 1 000 is answered today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-004: References in a moderately nested file through the HTTP API
- **Category:** happy; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-009; **Priority:** High
- **Given** `/src/mid.py` of E2E-003 with a third line `f()`
- **When** a client asks the HTTP API for the references to `f` under `/src`
- **Then** the answer lists exactly the references the assistant tool lists for the same call, the line 3 reference among them, with the same fields and order
- Level / Driver / Location: functional e2e / HTTP door `GET /api/fs/{mount}/find-references` and `McpServer::fs_find_references` for the same call / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0092_004_references_in_a_moderately_nested_fil`
- Green on arrival: answered today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-010: References keep their order
- **Category:** edge; **Scenario:** SC-005; **Requirements:** FR-NEW-008; **Priority:** High
- **Given** `/src/order.py` holding `total = 1`, `print(total)`, `total += 2` on lines 1 to 3
- **When** the member asks for the references to `total` under `/src`
- **Then** the answer lists the identifier references on lines 1, 2 and 3, in the same order and with the same kinds as the answer recorded before the change
- Level / Driver / Location: functional e2e / typed engine call `core::fs_ops::find_definitions` or `find_references` after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_010_references_keep_their_order`
- Green on arrival: the recorded order is today's; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-025: Not a member, deep source
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008; **Priority:** Medium
- **Given** `/src/deep.py` of E2E-SEC-011 and `eve@example.com`, not a member of `atlas`
- **When** `eve@example.com` asks for the definition of `f` under `/src`
- **Then** the refusal is `ERR_FORBIDDEN` with `'eve@example.com' is not a member of 'atlas'`
- Level / Driver / Location: functional e2e / `McpServer::new(state, "eve@example.com").fs_find_definition` (the real handler) / crates/core/src/tools/nesting_e2e.rs
- Function: `e2e_0092_025_not_a_member_deep_source`
- Green on arrival: membership is checked first today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### SPEC-0092/E2E-026: Empty name over a deep source through the HTTP API
- **Category:** failure; **Scenario:** SC-005; **Requirements:** FR-NEW-008, FR-NEW-009; **Priority:** Medium
- **Given** `/src/deep.py` of E2E-SEC-011
- **When** a client asks the HTTP API for the references to an empty name under `/src`
- **Then** the refusal is `ERR_INVALID_ARGUMENT` with `name is required`, as through the assistant tool
- Level / Driver / Location: functional e2e / HTTP door and `McpServer::fs_find_references` with an empty name / crates/core/src/api/dataplane.rs tests
- Function: `e2e_0092_026_empty_name_over_a_deep_source_through`
- Green on arrival: the empty name is refused before any file is read today; then the mutation below
- Cleanup: none (temporary directories dropped with the fixture)

### Mutation evidence for tests green on arrival
> Apply the mutation, see the test fail, revert. Record both runs.

| Test | Mutation |
|---|---|
| E2E-003 | in `walk`, never push the children of the root: no definition is found, the test fails |
| E2E-004 | in `walk`, never push the children of the root: both answers are empty and the line 3 reference is missing, the test fails |
| E2E-010 | push the named children in their own order instead of reversed (siblings visited right to left): lines come 3, 2, 1, the test fails |
| E2E-025 | in `McpServer::fs_find_definition`, call the engine before `authorize`: eve gets an answer, the test fails |
| E2E-026 | drop the empty name check of `core::fs_ops::find_references`: the answer is a list, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/docs/pdf_bound.rs`, the SPEC-0088 and SPEC-0089 kits
- Dependencies not to add: none
- Never add a depth limit or a refusal to the search (spec D-10)
- The visit order must stay the pre order of today: the 22 existing symbol tests are the oracle
- No dash as punctuation in code, comments or docs

## Definition of done
- [ ] RED evidence recorded for every new test before the production code: E2E-SEC-011, E2E-SEC-012, E2E-SEC-013 (step 1: fixtures and tests on US-0224 code, the three exploit tests failing through the parent (signal 6); step 2: the two explicit stacks).
- [ ] Green on arrival tests (E2E-003, E2E-004, E2E-010, E2E-025, E2E-026): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green after this commit (`cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`); the story lands on main alone, in queue order (repo practice), so main is green after it.
- [ ] Non regression: the 22 tests of `docs::symbols`, the 13 of `tools::search`, the find tests of `api::dataplane`, every earlier `e2e_0092_` test
- [ ] Docs updated: none in this story.
- [ ] Self review: full (T2): every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; no dash used as punctuation in code, comments or docs; failure modes listed in the commit body.
- [ ] One commit `US-0225: code search answers source of any nesting` on its story branch `story/US-0225-code-search-answers-source-of-any-nestin`, cut from the previous story's branch, or from main once that story has landed (repo practice for US-0183 to US-0221; the pipeline contract names the spec branch `impl/SPEC-0092-<slug>`), body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0092` and the requirement ids (SPEC-0092/FR-NEW-006, FR-NEW-008, FR-NEW-009).
