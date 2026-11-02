# SPEC-0074: What counts as "on the remote" for a tag publish, design

> Spec: sdlc/specs/SPEC-0074-what-counts-as-on-the-remote/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-02
> Target tier: 2 (default)

## 1. Stack and skills
Existing Rust workspace, no library added. Skills loaded: rust.

## 2. Approaches
### Chosen: keep the code, specify and test it
`commit_on_remote` already treats every advertised position as a tip (`git_tags.rs:1474-1477`). The work is tests plus the wording of its doc comment and the docs. Pros: no behavior change risk. Cons: most tests pass on arrival, so red evidence is by named mutation (DEC-003).
### Rejected: restrict to `refs/heads/*`
The owner chose the wide rule (BL-0103 decision, 2026-11-02).

## 3. Architecture
`publish_on_repo` lists the remote once (`git_tags.rs:647`) then calls `commit_on_remote`, which builds a `HashSet<Oid>` of every advertised object id (peeled `^{}` entries and `HEAD` included) and tests equality or `graph_descendant_of` for tips the project holds (`:1481-1486`).

## 4. Data model
None.

## 5. Contracts and interfaces
No change. The refusal text is `Cannot publish tag '<tag>': commit <short7> is not on the remote. Publish its branch first.` (`git_tags.rs:651-656`). Test helpers needed: a bare remote ref of any name made with `create_branch_on_bare_remote(dir, "refs/pull/7/head", from_ref)` (accepts a full ref name, `git.rs:10865`); a new helper `detach_bare_remote_head(dir, commit)` (sets the bare repository HEAD to a commit id).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | modify (ten tests, one helper) | all | US-0151 |
| `crates/core/src/tools/git_tags.rs` | modify (doc comment of `commit_on_remote` states the wide rule; no code) | FR-MOD-001 | US-0151 |
| `.agent_docs/git.md` | modify (the "on the remote" sentence) | FR-MOD-001 | US-0151 |
Affected existing tests: keep all (`e2e_tag_071` to `073`, `084` stay as is).

## 7. Compatibility and risks
No behavior change. Risk: E2E-007 (default position only) depends on whether the listing returns `HEAD` for a detached bare HEAD (signal: the test fails on arrival; then the fix is in this story and the spec's FR-MOD-001 is the authority).

## 8. Security design
Not applicable.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 to E2E-010 | functional e2e | direct tool call on `Env` (`call_publish_tag`, `cloned_remote`, `plain_tag_bare_remote`, `bare_refs`, `audit_size`) | `crates/core/src/tools/git.rs` tests | US-0151 |
Test data: generated.

## 10. Decisions log
- **DEC-001:** no production change. **Obliged by:** SPEC-0074/FR-MOD-001. **Rationale:** the code already implements the wide rule. **Alternatives considered:** a filter on `refs/heads/*` (rejected by the owner). **Evidence:** `git_tags.rs:1474-1477`.
- **DEC-002:** one story: ten tests, three files, no production code. **Obliged by:** n/a (no code impact): partitioning only. **Source:** design.
- **DEC-003:** red evidence by mutation: restricting `commit_on_remote` to tips of `refs/heads/*` makes E2E-002, 004 and 007 fail; ignoring the project's held history (accept only equality) makes E2E-005 fail; fetching unheld tips makes E2E-006 fail. E2E-003, 008, 009, 010 guard the refusal and are proven by the mutation "accept every commit". **Obliged by:** SPEC-0074/FR-MOD-001, FR-NEW-001. **Source:** design.
- **DEC-004:** the doc comment of `commit_on_remote` is rewritten to name "any advertised position" instead of "an advertised commit the project already holds". **Obliged by:** SPEC-0074/FR-MOD-001. **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0151 | pin what counts as on the remote for a tag publish | P1 | FR-MOD-001, FR-NEW-001 | US-0150 | 2 | yes |
