# SPEC-0072: Tag publish error wording, design

> Spec: sdlc/specs/SPEC-0072-tag-publish-error-wording/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-02
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
Existing Rust workspace, no library added. Skills loaded: rust. Harness: the `Env` tool harness of `crates/core/src/tools/git.rs` tests.

## 2. Approaches
### Chosen: a wording argument carried to the connection error
`connection_error` (`remote.rs:1568`) is shared by fetch, pull, clone, delete and publish. A small enum picks its text, passed by the caller that knows the operation. Pros: typed, no string surgery, fetch stays the default. Cons: one extra parameter on `list_advertised_refs`, `advertise` and `probe_remote_tag` (3 functions, about 12 call sites). Best when the wording depends on the caller, which FR-MOD-001 requires.
### Rejected: rewrite the message prefix in the tools layer
Replacing `fetch from host` by string search in `git_tags.rs`. Rejected (DEC-002): it ties wording to a message prefix and misses the `tag_push_error` path.

## 3. Architecture
`remote.rs` gets `pub enum ConnectionWording<'a> { Fetch, PublishTag(&'a str), PublishTags }` and `fn connection_error_for(w: ConnectionWording<'_>, origin_url: &str, e: &git2::Error, token: Option<&str>) -> ToolError`; `connection_error(...)` becomes `connection_error_for(Fetch, ...)`. Publish callers in `git_tags.rs` pass `PublishTag(tag)` or `PublishTags`; delete and fetch callers pass `Fetch`.

## 4. Data model
None.

## 5. Contracts and interfaces
- Texts, code `ERR_INTERNAL_ERROR` as today: `Fetch` `fetch from host '<host>' failed: <cause>`; `PublishTag` `push of tag '<tag>' to host '<host>' failed: <cause>`; `PublishTags` `push of tags to host '<host>' failed: <cause>`. The redaction of a token in `<cause>` is unchanged.
- Signatures: `list_advertised_refs(repo, origin_url, token, deadline, wording: ConnectionWording<'_>)`; `probe_remote_tag(repo, origin_url, tag, token, deadline)` uses `PublishTag(tag)` itself (only publish calls it); `advertise(.., wording)` is private, `fetch_from_remote` passes `Fetch`; `tag_push_error` uses `PublishTag(tag)`; `push_tags_to_remote` uses `PublishTags` at its two `connection_error` sites (`remote.rs:882`, `:896`).
- Vanished tag: in `publish_on_repo` (`git_tags.rs:715-726`) the lease-lost branch replaces `unwrap_or(Oid::ZERO_SHA1)` by: remote tag absent in the new listing gives `remote_tag_absent(tag)` (`Remote tag 'v1.0' does not exist.`, `ERR_NOT_FOUND`); present keeps `moved(tag, ...)`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/remote.rs` | modify (enum, `connection_error_for`, wording parameter, `PublishTag` then `PublishTags`) | FR-NEW-001, FR-NEW-002, FR-MOD-001 | US-0149, US-0150 |
| `crates/core/src/tools/git_tags.rs` | modify (callers pass a wording; vanished tag branch) | FR-NEW-001 to FR-NEW-003 | US-0149, US-0150 |
| `crates/core/src/tools/git.rs` | modify (tests; helpers `publish_tag_prefix`, `publish_tags_prefix`; five adapted publish all assertions) | all | US-0149, US-0150 |
| `.agent_docs/git.md` | modify (error wording table of the tag section) | FR-MOD-001 | US-0150 |
Affected existing tests: modify the publish all assertions of `e2e_tag_020` (`git.rs:29508`), `e2e_tag_022` (`:29589`), `e2e_tag_029` (`:29660`), `e2e_tag_050` (`:30089`) and the drop assertion at `:29528`, all through `assert_connection_refused` (`:29491`), in US-0150 only; the delete tests (`:27856-27978`) keep `connection_prefix` unchanged.

## 7. Compatibility and risks
Only the text of two refusals and one refusal code (`not found` for a vanished tag instead of `no clobber`) change. A client matching on the old text or on `ERR_NO_CLOBBER` for a vanished tag sees a change; the spec declares it. Risk: a publish step missed keeps `fetch from host` (signal: E2E-002, E2E-007, E2E-009, E2E-011 each name a step).

## 8. Security design
Not applicable (`Security: n/a`).

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-002, 004, 005, 006, 007, 008, 009, 010 | functional e2e | direct tool call on `Env` (`call_publish_tag`, `down_remote`, `closing_remote`, `after_listing` seam) | `crates/core/src/tools/git.rs` tests | US-0149 |
| E2E-001, 003, 011, E2E-MOD-001 | functional e2e | direct tool call (`call_publish_all_tags`, `bulk_seams::drop_after`) | `crates/core/src/tools/git.rs` tests | US-0150 |
Test data: generated bare remotes (`cloned_remote`, `advance_bare_remote`, `plain_tag_bare_remote`).

## 10. Decisions log
- **DEC-001:** wording is chosen by an enum argument `ConnectionWording`, default `Fetch`. **Obliged by:** SPEC-0072/FR-NEW-001, SPEC-0072/FR-NEW-002, SPEC-0072/FR-MOD-001. **Rationale:** FR-MOD-001 and E2E-006 need the same callee to word differently per caller. **Alternatives considered:** per function copies of `connection_error` (rejected: 11 call sites). **Source:** design. **Evidence:** `remote.rs:1568-1571`.
- **DEC-002:** no string rewriting in the tools layer (see Approaches). **Obliged by:** SPEC-0072/FR-MOD-001. **Source:** design.
- **DEC-003:** `probe_remote_tag` always words as `PublishTag`: its only caller is the annotated message check of a publish. **Obliged by:** SPEC-0072/FR-NEW-001 (E2E-009). **Source:** existing code. **Evidence:** `git_tags.rs:739`.
- **DEC-004:** a vanished remote tag reuses `remote_tag_absent` verbatim. **Obliged by:** SPEC-0072/FR-NEW-003. **Rationale:** one condition, one message (owner decision, spec 3.3). **Evidence:** `git_tags.rs:1452-1454`.
- **DEC-005:** two stories: US-0149 (one tag: enum with `Fetch` and `PublishTag`, the vanished tag, eight tests), US-0150 (publish all: `PublishTags`, the adapted assertions, docs, four tests). **Obliged by:** n/a (no code impact): partitioning only. **Rationale:** the adapted assertions must change together with the production wording they check, and only publish all uses them. **Source:** design.
- **DEC-006:** the preview of publish all (`git_tags.rs:1028`, `:1234` listings) is a publish step and takes `PublishTags`. **Obliged by:** SPEC-0072/FR-NEW-002. **Source:** spec business rule.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0149 | word a one tag publish failure as a push and a vanished tag as gone | P1 | FR-NEW-001, FR-NEW-003, FR-MOD-001 (part) | US-0148 | 2 | no |
| US-0150 | word a publish all failure as a push of tags | P1 | FR-NEW-002, FR-MOD-001 (part) | US-0149 | 2 | yes |
