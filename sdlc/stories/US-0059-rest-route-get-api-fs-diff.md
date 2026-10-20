# US-0059: the same comparison on the REST data plane

> Spec: SPEC-0045 (sdlc/specs/SPEC-0045-fs-diff-two-files/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0056
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A program can call `GET /api/fs/diff` with the four values as query parameters and receive exactly the answer or refusal the tool gives, plus the missing value refusal of FR-NEW-018.

## Requirements covered
- SPEC-0045/FR-NEW-013: The programmatic access context SHALL offer the comparison with the same inputs, the same diff, the same identical indicator and the same refusal messages as the tool surface, under the names pinned in Section 14.4, a refusal keeping the status the product already attaches to its code.
- SPEC-0045/FR-NEW-018: IF a program client omits one of the four values (the project and path of each file) or gives it empty THEN THE product SHALL refuse the comparison, naming the first missing value in the order first project, first path, second project, second path.

## Independent Test
Run `cargo test -p mcp-fs-core --lib tools::diff_files_e2e api::` : the 6 tests above pass and the existing OpenAPI/route manifest consistency test still passes. E2E-004 compares the REST JSON body with the tool answer for the same inputs, field by field.

## Design excerpt
### Decisions
- (SPEC-0045) DEC-010: REST route `GET /api/fs/diff` outside `{mount_id}`, own handler (no `guarded`, which authorizes one mount), authenticating with `person_of`; `("GET", "diff")` in `REST_ROUTES`; one `Op` in the OpenAPI table like `roots`. **Obliged by:** FR-NEW-013. **Rationale:** `roots` and `quota` already are routes not scoped to `{mount_id}`. **Alternatives considered:** POST with a JSON body (reads are GET in this plane). **Source:** spec 14.4. **Evidence:** `api/dataplane.rs:117,174-178,213-217`, `api/openapi.rs:536-538`.
- (SPEC-0045) DEC-014: The REST missing value check is local to the new handler and spelled `'<name>' is required`; the existing `Q::req_str` message (`missing required argument '<key>'`, `api/dataplane.rs:277-281`) is left untouched. A value of only spaces counts as empty. **Obliged by:** FR-NEW-018. **Rationale:** the spec pins a different text; changing the shared helper would alter every route. **Alternatives considered:** change `Q::req_str` (breaks existing REST tests). **Source:** spec. **Evidence:** `api/dataplane.rs:265-285`.
### Data model
None.
### Contracts
Route: `.route("/api/fs/diff", get(diff))` in `api/dataplane.rs::router`, next to `/api/fs/roots` (line 117). Add `("GET", "diff")` to `REST_ROUTES` (line 43 onward). Handler shape:
```rust
async fn diff(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<Vec<(String, String)>>) -> Response
```
Steps: `person_of(&state, &headers)` (401 body via `unauthorized` on error); read the four values in the order `mount_id_a`, `path_a`, `mount_id_b`, `path_b`; a value that is missing, empty, or only spaces gives `ToolError::invalid_argument(format!("'{name}' is required"))` for the FIRST such name (HTTP 400 via `error_response`); otherwise call `crate::tools::diff_files::diff_files(&state, &person, Side{..}, Side{..})` and answer `Json(value)` or `error_response(&e)`. The helper `Q::req_str` and its message `missing required argument '<key>'` stay untouched (DEC-014).
OpenAPI: add one `Op` in `api/openapi.rs` like the `roots` entry (line 536): `method: "GET"`, `sub: "diff"`, `path: "/api/fs/diff"`, `tool: "fs.diff_files"`, four required string query params, no body. The test at `openapi.rs` that compares `REST_ROUTES` with the `Op` table must pass.
Answer: HTTP 200 `{"identical": <bool>, "diff": <string>}`. Errors keep the status of their code (`errors.rs:120-128`: 403 forbidden, 404 not found and unknown project, 400 invalid argument).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/api/dataplane.rs` | modify |
| `crates/core/src/api/openapi.rs` | modify |
| `crates/core/src/tools/diff_files_e2e.rs` | modify |
### Existing patterns
- Non project route and its handler: `crates/core/src/api/dataplane.rs` `roots` (route line 117) and `person_of` (lines 213 to 217); error rendering `error_response` and `unauthorized` (lines 241 to 252).
- Query reading: `Q` at `dataplane.rs:265-285` (`opt` already treats an empty value as absent; the whitespace rule is extra, do it locally with `trim()`).
- REST test driving: `tower::ServiceExt::oneshot` on the real router as in `tools/quota_e2e.rs` (the REST quota route test) and `api/dataplane.rs` tests (`h.get(&u("count-lines?path=/a.txt"))` near line 1981).

## Acceptance tests
> 100% must pass, run through `make check` (or `./test.sh`). Never run test files ad hoc for the final verdict.

Common data and conventions of all tests (copied from the spec):

Common data. Projects `alpha` and `beta`. Alice (alice@example.com) is a member of both. Bob
(bob@example.com) is a member of `alpha` only. `alpha:/notes/a.txt` = `one\ntwo\nthree\n`;
`alpha:/notes/b.txt` = `one\n2\nthree\n`. "Compare X with Y" means the member asks the product to compare X
(old side) with Y (new side). All tests are driven through the tool surface unless the test says otherwise.

### SPEC-0045/E2E-004: Program client gets the same answer
- Level / Driver / Location: functional e2e / REST router via `tower::ServiceExt::oneshot` (the tool answer is read with `McpServer::fs_diff_files` where the test compares both) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** journey. **Scenario:** SC-005. **Requirements:** FR-NEW-013, 001. **Priority:** High
- **Given** the common data
- **When** Alice's program asks the web interface to compare `alpha:/notes/a.txt` with `alpha:/notes/b.txt`
  and, separately, Alice asks the same through the tool surface
- **Then** the diff text and the identical indicator are exactly equal in both answers
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-112: Program client gets the same refusal
- Level / Driver / Location: functional e2e / REST router via `tower::ServiceExt::oneshot` (the tool answer is read with `McpServer::fs_diff_files` where the test compares both) / crates/core/src/tools/diff_files_e2e.rs
- Expectation on arrival: RED only because the route does not exist yet
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-013. **Priority:** High
- **Given** as E2E-107
- **When** Bob's program asks the web interface for the same comparison as E2E-107
- **Then** the refusal message equals the one from E2E-107 word for word
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-220: Identical files through the web interface
- Level / Driver / Location: functional e2e / REST router via `tower::ServiceExt::oneshot` (the tool answer is read with `McpServer::fs_diff_files` where the test compares both) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-013, 003. **Priority:** Medium
- **Given** E2E-003 data
- **When** Alice's program asks the web interface to compare the two identical files
- **Then** identical true and an empty diff, exactly as E2E-003
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-223: Web interface, second project missing
- Level / Driver / Location: functional e2e / REST router via `tower::ServiceExt::oneshot` (the tool answer is read with `McpServer::fs_diff_files` where the test compares both) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-018. **Priority:** Medium
- **Given** the common data
- **When** Alice's program asks the web interface with the first project and path and the second path, but no
  second project
- **Then** `ERR_INVALID_ARGUMENT: 'mount_id_b' is required`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-224: Web interface, several values missing
- Level / Driver / Location: functional e2e / REST router via `tower::ServiceExt::oneshot` (the tool answer is read with `McpServer::fs_diff_files` where the test compares both) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-018, 013. **Priority:** Medium
- **Given** the common data
- **When** Alice's program asks the web interface with only the first project given and the first path empty
- **Then** `ERR_INVALID_ARGUMENT: 'path_a' is required` (the first missing value in the fixed order)
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

### SPEC-0045/E2E-227: Value of spaces on the web interface
- Level / Driver / Location: functional e2e / REST router via `tower::ServiceExt::oneshot` (the tool answer is read with `McpServer::fs_diff_files` where the test compares both) / crates/core/src/tools/diff_files_e2e.rs
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-018. **Priority:** Medium
- **Given** the common data
- **When** Alice's program asks the web interface with all values given but the second path equal to three
  spaces
- **Then** `ERR_INVALID_ARGUMENT: 'path_b' is required`
- Cleanup: none (each test builds its own temporary fixture, dropped at the end of the test).

## Constraints
- Files not to touch: `core/diff.rs`, `storage/*`, `config.rs`, golden files.
- Dependencies not to add: none.
- Patterns to avoid: a second implementation of the comparison in the handler; editing `Q::req_str`.
- Scope boundary: docs (US-0060).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code, except the tests marked "Expectation on arrival" above, whose green status on arrival is recorded as such.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: The route manifest and OpenAPI consistency tests and every existing REST test pass unmodified.
- [ ] Docs updated: none in this story.
- [ ] Self review (full): every requirement above is demonstrably met by a named test; no `unwrap()` on a fallible path; no dash as punctuation in code, comments or docs; errors carry the exact texts of this story.
- [ ] One commit `US-0059: the same comparison on the REST data plane`, body `Spec: SPEC-0045` and the requirement ids.
