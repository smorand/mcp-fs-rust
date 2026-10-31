# SPEC-0067: Incremental fetch, design

> Spec: sdlc/specs/SPEC-0067-incremental-fetch-negotiation/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-10-31
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)
> Status: designed

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust 2024 | workspace manifest | existing |
| Git library | git2 (libgit2): revwalk `hide`, `graph_descendant_of`, packbuilder | 0.21.0 (`Cargo.lock`) | DEC-001 |
| HTTP | axum | 0.8 (`Cargo.toml:15`) | existing |
| Test oracle | the `git` program (`git upload-pack --stateless-rpc`), tests only | 2.50.1 on the owner's machine | DEC-007 |
| Test runner | `cargo test --workspace` through `make check` | n/a | existing |

No library is added. Skills loaded: rust (`load_dynamic_context`); effect: `make check` (fmt, clippy `-D warnings`, audit, tests) is the gate, comments explain why, no dashes as punctuation.

## 2. Approaches
### Chosen: port git's stateless negotiation over libgit2, in a new module
Summary: a new module (`git/http/negotiate.rs`) reads the whole request, then answers the way git's `upload-pack` does in stateless mode with `multi_ack_detailed`: one `ACK <id> common` line per held commit the project has, `ACK <id> ready` when git would send it, `NAK`, and, once the client sends `done`, the closing line and the objects the wanted commits need minus the contents of the held commits. `handle_upload_pack` becomes the dispatcher. Pros: no new dependency, keeps the blob store as the source of truth, each rule is small and testable against the reference program. Cons and risks: the object set depends on how libgit2 picks objects (E2E-006, E2E-007 compare it with git's). Affected scope: `git/http` (3 files). Best suited when the rules are fixed by a reference, as here.
### Rejected: run the `git` program inside the service
Summary: spawn `git upload-pack --stateless-rpc` on a repository materialised from the blob store. Rejected (DEC-002): it makes the git binary a runtime dependency of every deployment (new infrastructure) and loses the blob store as source of truth.
### Rejected: hand written object selection
Summary: walk commits and trees ourselves. Rejected (DEC-003): libgit2's packbuilder already does the selection once the right commits are hidden; hiding exactly git's edge commits reproduces git's set.

## 3. Architecture
```
POST /git/{mount}/git-upload-pack
   -> upload_pack (route, access rules unchanged)           mod.rs:179
   -> handle_upload_pack(entry, body)                       mod.rs:421
        negotiate::parse(body)        tolerant, reads to the end of the input
        hydrate_if_stale(repo)        SPEC-0066, unchanged   mod.rs:434
        negotiate::answer(repo, ref_values, req) -> bytes
             servable wants  (reachability, tags peeled)
             acknowledgements (ACK common / ready / NAK)
             object set  (revwalk wants, hide edge commits; packbuilder)
```
Bounded contexts: Git service (this change), Project access (untouched; the route's gate stays at `mod.rs:249`). The service keeps no state between requests: every answer derives from one request (14.3 of the spec).

## 4. Data model
None. No table, column or setting changes (spec 3.3: no migration, no setting).

## 5. Contracts and interfaces
`pktline.rs` (additions, existing returns unchanged):
```rust
impl<'a> PktReader<'a> {
    /// True once every byte of the input was consumed.
    pub fn at_end(&self) -> bool;
    /// True when the last packet read announced more bytes than the input held (its data was cut).
    pub fn last_was_cut(&self) -> bool;
}
```
`negotiate.rs` (new, `pub(super)` items):
```rust
pub(super) struct Request {
    pub wants: Vec<String>,   // raw ids in order, capabilities of the first want removed
    pub haves: Vec<String>,   // raw ids in order, up to the first flush or `done` after the wants
    pub have_lines: usize,    // count of `have` lines, taken before any id is discarded
    pub done: bool,           // the end marker was present
    pub option_line: bool,    // a shallow, deepen (any form) or filter line was present
}
pub(super) fn parse(body: &[u8]) -> Request;
/// The whole response body of `git-upload-pack`.
pub(super) fn answer(repo: &git2::Repository, ref_values: &[String], req: &Request) -> crate::errors::Result<Vec<u8>>;
```
`mod.rs`: `UploadPackRequest` and `parse_upload_pack_request` keep their fields and results (two unit tests read them); `handle_upload_pack(entry: &GitRepoEntry, body: &[u8]) -> Result<Vec<u8>>` keeps its signature. Wire contract: Section 3.3 of the spec (the lines `ACK <id> common`, `ACK <id> ready`, `ACK <id>`, `NAK`, then the objects framed on side-band channel 1 as today, then the closing channel byte and a flush as today). A round without `done` ends after `NAK`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/tests/git_negotiation_e2e.rs` | create (harness, oracle, 48 e2e tests) | all E2E | US-0135 to US-0140 |
| `crates/core/src/git/http/pktline.rs` | modify (`at_end`, `last_was_cut`) | FR-NEW-010, 014 | US-0136, US-0139 |
| `crates/core/src/git/http/negotiate.rs` | create | FR-NEW-001 to 014 | US-0136 to US-0139 |
| `crates/core/src/git/http/mod.rs` | modify (`mod negotiate;`, dispatcher; 5 perf tests) | FR-NEW-001, NFR-001, NFR-003 | US-0136, US-0140 |
| `.agent_docs/git.md` | modify (the no negotiation paragraph) | FR-NEW-001, 002 | US-0141 |
| `.agent_docs/git-divergences.md`, `AGENTS.md` | modify (index line, divergence note) | FR-NEW-001 | US-0141 |
Affected existing tests: keep all (`parses_wants_and_haves`, `parses_an_empty_upload_pack_request`, `upload_pack_without_wants_answers_a_bare_flush`, the three `handle_upload_pack` round trips at `mod.rs:870,1504,1535`); 0 modified. Documentation: the three files above.

## 7. Compatibility and risks
- Consumers: git clients 2.x; every request a real client sends is covered by the reference rule. Requests the reference rejects are covered by the declared differences (spec FR-NEW-004, 007 to 010, 012 to 014).
- Ordering: US-0136 changes behavior for requests with `have` lines; before it, stories only add tests that pass today.
- Risk 1: libgit2's object set differs from git's in an edge case (E2E-006, 007). Mitigation: the comparison tests fail loudly; the fallback is to hide different commits (DEC-005), not to change the spec.
- Risk 2: `graph_descendant_of` per want and per common is O(history). Mitigation: measured by NFR-001 tests (US-0140); a smaller algorithm (stop at the first common) replaces it if needed.
- Rollback: revert the story commits; no data changes.
- Dependencies: none added or removed.

## 8. Security design
Not applicable (`Security: n/a`). Access rules are untouched and covered by E2E-019, 020, 021.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-012 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-013 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-015 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-016 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-017 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-019 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-020 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-022 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0135 |
| E2E-002 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-003 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-004 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-005 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-006 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-007 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-011 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-014 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-021 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-029 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-032 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-033 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-034 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-038 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0136 |
| E2E-010 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0137 |
| E2E-041 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0137 |
| E2E-042 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0137 |
| E2E-043 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0137 |
| E2E-008 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0138 |
| E2E-009 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0138 |
| E2E-030 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0138 |
| E2E-047 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0138 |
| E2E-048 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0138 |
| E2E-049 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0138 |
| E2E-018 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-031 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-035 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-036 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-037 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-044 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-045 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-046 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-050 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-051 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-052 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-053 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0139 |
| E2E-023 | integration | direct call | `crates/core/src/git/http/mod.rs` | US-0140 |
| E2E-024 | integration | direct call | `crates/core/src/git/http/mod.rs` | US-0140 |
| E2E-025 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0140 |
| E2E-026 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0140 |
| E2E-027 | functional e2e | git client / hand written request + reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0140 |
| E2E-028 | integration | direct call | `crates/core/src/git/http/mod.rs` | US-0140 |
| E2E-039 | integration | direct call | `crates/core/src/git/http/mod.rs` | US-0140 |
| E2E-040 | integration | direct call | `crates/core/src/git/http/mod.rs` | US-0140 |
Test data: generated (commits made with the `git` program in temp directories; 20 000 object projects seeded through the object store); no fixture files. The tests need the `git` program and loopback networking; a missing `git` fails the test (like the full stack suite).

## 10. Decisions log
- **DEC-001:** implement the negotiation in a new module over libgit2 (revwalk `hide`, packbuilder). **Obliged by:** SPEC-0067/FR-NEW-001, FR-NEW-002. **Rationale:** reuses what the service already uses to build packs. **Alternatives considered:** DEC-002, DEC-003. **Source:** design. **Evidence:** `crates/core/src/git/http/mod.rs:460-494`.
- **DEC-002:** never run the `git` program in production. **Obliged by:** SPEC-0067/FR-NEW-001 (answer as the reference without depending on it). **Rationale:** a runtime dependency on a binary is new infrastructure and breaks the blob store model. **Alternatives considered:** spawn `git upload-pack`. **Source:** design. **Evidence:** `.agent_docs/git.md:147` (objects live in the blob store).
- **DEC-003:** keep libgit2's packbuilder for object selection, hide exactly git's edge commits (the uninteresting parents of included commits), not every held commit. **Obliged by:** SPEC-0067/FR-NEW-002. **Rationale:** git marks the trees of edge commits only (`mark_edges_uninteresting`); libgit2's `insert_walk` marks the trees of the commits hidden (`pack-objects.c:1684-1697` in libgit2-sys 0.18.8), so hiding only edges reproduces git. **Alternatives considered:** hide every held commit (differs when two held commits are on one line, E2E-007). **Source:** design, auditor finding. **Evidence:** spec 14.3.
- **DEC-004:** acknowledgements follow git's `get_common_commits` (stateless, `multi_ack_detailed`): one `ACK <id> common` per have the project holds as a commit, repeats included; at the end of the haves, without `done`: `ACK <last> ready` when at least one held commit, no unknown well formed have, and every wanted commit reaches a held commit, then `NAK`; with `done`: `ACK <last>` or `NAK`, then the objects. **Obliged by:** SPEC-0067/FR-NEW-001, FR-NEW-011. **Rationale:** verified on 2026-10-31 against git 2.50.1: duplicates are acknowledged twice; an unknown have suppresses `ready`; a want that does not reach the held commit suppresses it. **Alternatives considered:** never send `ready` (rejected: the spec follows the reference). **Source:** reference runs. **Evidence:** spec 14.3; comparison tests E2E-010, 041.
- **DEC-005:** declared differences are applied while reading the haves: a have that is not 40 hexadecimal characters is skipped; a well formed id of an object that is not a commit is skipped; neither acknowledges nor suppresses `ready`; a well formed id the project does not hold suppresses `ready` and is not acknowledged (as the reference). **Obliged by:** SPEC-0067/FR-NEW-004, FR-NEW-007. **Rationale:** matches "treat as not stated" for the two cases the spec declares, and the reference for the rest. **Alternatives considered:** treat unknown well formed ids as skipped too (rejected: differs from the reference answer on `ready`). **Source:** design. **Evidence:** reference run A (spec 14.3).
- **DEC-006:** a wanted id is servable when it exists and either equals the value of a branch or tag, or is a commit reachable from one (`graph_descendant_of` over the ref values), or is a tree or file version contained in a commit reachable from one. Unservable wanted ids are dropped; if none is left the answer is `NAK` and an empty pack. A wanted annotated tag adds the tag object and continues with its target. **Obliged by:** SPEC-0067/FR-NEW-008, FR-NEW-013. **Rationale:** the reference refuses unreachable wants; the spec declares silence instead. **Alternatives considered:** serve any held object as today (rejected: contradicts FR-NEW-008). **Source:** design. **Evidence:** `crates/core/src/git/http/mod.rs:471-480`.
- **DEC-007:** the tests compare with the reference program and with real clients over an in-process server; the `git` program is a test dependency only. **Obliged by:** SPEC-0067/FR-NEW-001, FR-NEW-002 and all E2E (spec 3.3: "equals the reference answer"). **Rationale:** the contract is the reference; enumerating cases by hand is what five audits showed does not converge. **Alternatives considered:** golden bytes recorded once (rejected: hides drift with new git versions). **Source:** spec constraint. **Evidence:** `crates/core/tests/full_stack_e2e.rs:94` (`require_git_cli` pattern), `app::build` at `crates/core/src/app.rs:63`.
- **DEC-008:** the wire e2e tests live in one new integration test file with a server booted by `app::build` on SQLite and local blobs (default config), anonymous reading switched per test; projects are created and filled through the real `git push` with a minted admin token. **Obliged by:** SPEC-0067/FR-NEW-005, FR-NEW-006 and all E2E. **Rationale:** crosses the same wire as users; no crate private helper. **Alternatives considered:** extend `full_stack_e2e` (rejected: needs PostgreSQL and MinIO, `#[ignore]`). **Source:** design. **Evidence:** `crates/core/tests/full_stack_e2e.rs:369-371`.
- **DEC-009:** the five 20 000 object tests run as integration tests inside `mod.rs` tests, seeded through the object store as SPEC-0066 did. **Obliged by:** SPEC-0067/NFR-001, NFR-003. **Rationale:** pushing 20 000 objects through the wire in a debug build costs minutes; the answer function is the thing measured. **Alternatives considered:** wire push (rejected for cost). **Source:** SPEC-0066 design. **Evidence:** `sdlc/specs/SPEC-0066-hydrate-the-upload-pack-path/sdlc-design.md:38-44`.
- **DEC-010:** the fetch keeps taking only the repository lock; if E2E-025 to 027 are red, the answer is built under the project write lock instead. **Obliged by:** SPEC-0067/NFR-002. **Rationale:** measure first. **Alternatives considered:** take the write lock now (rejected: serialises fetches behind pushes without evidence). **Source:** design. **Evidence:** `crates/core/src/git/http/mod.rs:433,580-582,623`.
- **DEC-011:** no limit on the number of have lines is added. **Obliged by:** n/a (no code impact): the spec leaves it to design and requires no behavior; recorded as backlog BL-0117. **Rationale:** a cap changes what a client observes. **Alternatives considered:** a cap of 10 000 lines. **Source:** design. **Evidence:** `crates/core/src/git/http/mod.rs:398-418`.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0135 | build the oracle harness and the baseline guards | P1 | FR-NEW-003, FR-NEW-005, FR-NEW-006, FR-NEW-008, FR-NEW-010 | main | 2 | no |
| US-0136 | answer the end of a negotiation like git does | P1 | FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-007, NFR-001 | US-0135 | 1 | no |
| US-0137 | answer the rounds that do not end the negotiation | P1 | FR-NEW-001, FR-NEW-004, FR-NEW-007, FR-NEW-011 | US-0136 | 2 | no |
| US-0138 | serve only wanted ids a branch or tag reaches | P1 | FR-NEW-002, FR-NEW-008, FR-NEW-013 | US-0137 | 2 | no |
| US-0139 | tolerate odd requests and options the service does not offer | P1 | FR-NEW-003, FR-NEW-009, FR-NEW-010, FR-NEW-012, FR-NEW-014 | US-0138 | 2 | no |
| US-0140 | prove speed and consistency of fetches | P1 | FR-NEW-002, NFR-001, NFR-002, NFR-003 | US-0139 | 2 | no |
| US-0141 | document incremental fetch and close the spec | P1 | FR-NEW-001, FR-NEW-002 | US-0140 | 2 | yes |
