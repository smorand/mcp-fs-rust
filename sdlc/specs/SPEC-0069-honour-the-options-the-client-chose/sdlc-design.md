# SPEC-0069: Honour the options the client chose, design

> Spec: sdlc/specs/SPEC-0069-honour-the-options-the-client-chose/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-01
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
Existing Rust workspace, no library added. Skills loaded: rust (`make check` gate). Test harness: the SPEC-0067 one (`crates/core/tests/git_negotiation_e2e.rs`).

## 2. Approaches
### Chosen: record the side band choice in the request, announce only what is honoured
`parse` notes whether the first wanted line asks for `side-band-64k`; `finish` writes the objects raw when it did not; the announced text loses `multi_ack`. Pros: three small edits; plain `multi_ack` needs no code (the options are already dropped, the detailed answer is the only form). Cons: a client that asks for the plain form gets the detailed lines it did not choose, accepted by the spec (FR-MOD-001). Scope: `git/http/negotiate.rs`, `git/http/mod.rs` (constant and comments).
### Rejected: also answer the plain `multi_ack` form
Rejected in the spec (Section 3.2): no client can use it over HTTP (DEC-001).

## 3. Architecture
Unchanged flow (`handle_upload_pack` then `negotiate::answer`). `Request` gains one flag; `finish` (`negotiate.rs:105-125`) branches on it; `UPLOAD_PACK_CAPABILITIES` (`mod.rs:67-68`) is the single text used by the advertisement (`:383`).

## 4. Data model
None.

## 5. Contracts and interfaces
- `Request` (`negotiate.rs:18`): new field `pub side_band: bool`, true when the first wanted line carries the option `side-band-64k` (options are the words after the id; only the first wanted line is read for them).
- `finish`: with `side_band` the output is as today (chunks of 65 515 bytes on channel 1, closing byte, flush); without it the pack bytes follow the acknowledgement lines directly, with nothing after them.
- `UPLOAD_PACK_CAPABILITIES`: `"multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0"`.
- Harness helpers (test only): an answer parser for the unframed form, a request builder taking the options as a string, and a reader of the announced options from the refs listing.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/http/mod.rs` | modify (the constant, its two comments) | FR-NEW-001 | US-0144 |
| `crates/core/tests/git_negotiation_e2e.rs` | modify (helpers, 5 tests, then 3 tests) | all | US-0144, US-0145 |
| `crates/core/src/git/http/negotiate.rs` | modify (`Request.side_band`, `parse`, `finish`) | FR-NEW-002 | US-0145 |
| `.agent_docs/git.md`, `.agent_docs/git-divergences.md` | modify (options list, D18, known gaps) | FR-NEW-001, FR-NEW-002, FR-MOD-001 | US-0145 |
Affected existing tests: keep all (the unit tests that assert the constant follow it: `mod.rs:639,684`). Documentation: the two agent docs.

## 7. Compatibility and risks
Real clients see no change. A client that relied on the announced plain `multi_ack` now sees it absent: git's own HTTP client already refuses to negotiate without `multi_ack_detailed`, so none works with the old text. Ordering: US-0144 changes the announcement only; US-0145 changes the framing. Rollback: revert the story commits.

## 8. Security design
Not applicable (`Security: n/a`).

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | real client or hand written request | `crates/core/tests/git_negotiation_e2e.rs` | US-0144 |
| E2E-002 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0144 |
| E2E-003 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0145 |
| E2E-004 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0144 |
| E2E-005 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0145 |
| E2E-006 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0144 |
| E2E-007 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0144 |
| E2E-008 | functional e2e | hand written request or refs listing, reference program | `crates/core/tests/git_negotiation_e2e.rs` | US-0145 |
Test data: generated (commits with the `git` program in temp directories; the 400 KB file of E2E-008 from the harness helper `heavy_text`).

## 10. Decisions log
- **DEC-001:** announce `multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0` and honour exactly those; plain `multi_ack` is dropped, not implemented. **Obliged by:** SPEC-0069/FR-NEW-001, SPEC-0069/FR-MOD-001. **Rationale:** git requires the detailed form over HTTP, so the plain form is dead weight. **Alternatives considered:** implement `ACK <id> continue` (rejected in the spec, owner decision). **Source:** spec. **Evidence:** `crates/core/src/git/http/mod.rs:61-68` (the comment already says detailed is required).
- **DEC-002:** the side band choice is read from the first wanted line's words after the id. **Obliged by:** SPEC-0069/FR-NEW-002. **Rationale:** git sends its options only on the first wanted line; `parse` already takes the id as the first word of that line. **Alternatives considered:** scanning every wanted line (rejected: not the protocol). **Source:** design. **Evidence:** `crates/core/src/git/http/negotiate.rs:45`.
- **DEC-003:** without the side band the objects are appended as they are and nothing follows: no closing byte, no flush. **Obliged by:** SPEC-0069/FR-NEW-002. **Rationale:** that is git's own server's output (checked with git 2.50.1 on 2026-11-01) and the client reads the pack to its checksum. **Alternatives considered:** keep the closing flush (rejected: bytes after a raw pack break clients that read to the end). **Source:** reference program. **Evidence:** spec 14.1 row 5.
- **DEC-004:** the stories are split by what they change on the wire: the announcement first (FR-NEW-001, FR-MOD-001), the framing second (FR-NEW-002, closes the spec). **Obliged by:** n/a (no code impact): partitioning only. **Rationale:** each leaves `main` green; the first is two lines. **Alternatives considered:** one story (rejected: the announcement tests are independent of the framing). **Source:** design.
- **DEC-005:** the tests that guard behavior that already exists (E2E-001, 004, 007) get their red evidence by mutation: ask for the plain form in the answer, or announce `multi_ack` again. **Obliged by:** SPEC-0069/FR-MOD-001. **Rationale:** a test cannot be red before a change that is not needed. **Alternatives considered:** none. **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0144 | announce only the options the service honours | P1 | FR-NEW-001, FR-MOD-001 | US-0143 | 2 | no |
| US-0145 | send the objects unframed without the side band | P1 | FR-NEW-002 | US-0144 | 2 | yes |
