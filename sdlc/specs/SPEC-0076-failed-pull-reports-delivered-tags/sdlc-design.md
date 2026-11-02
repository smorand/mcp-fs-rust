# SPEC-0076: A failed pull reports the tags it delivered, design

> Spec: sdlc/specs/SPEC-0076-failed-pull-reports-delivered-tags/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-11-02
> Target tier: 2 (default)

## 1. Stack and skills
Existing Rust workspace, no library added. Skills loaded: rust.

## 2. Approaches
### Chosen: read the tag slot on the error path and append a sentence
The pull already stores `(tags_added, tags_kept_local)` in `tags_slot` right after the reconcile (`git.rs:6784`, `:6823`). On error the call returns through `.await?` before the slot is read (`:6995-6998`). The error path reads the slot, builds the sentence and appends it to `ToolError::message`. Pros: three lines of control flow, no new type. Cons: the tag list lives in the message text (the error type carries no data, `errors.rs:30-40`). Best when the wire shape stays `{code}: {message}`.
### Rejected: a new error field
Rejected in the spec (out of scope: structured error).

## 3. Architecture
`fn delivered_tags_sentence(added: &[String]) -> Option<String>` in `git.rs` (pure, unit tested): `None` for an empty slice; otherwise `Tags received before the failure: <names>.` with at most `MAX_NAMED_DELIVERED_TAGS = 20` names; beyond that `..., t20 and <N> more.`. The pull function matches the result of `with_remote_deadline(...).await`: `Err(mut e)` takes the slot (`mem::take`), appends `" "` and the sentence to `e.message`, returns the error. A test only fault injection hook (DEC-004) fires after the reconcile.

## 4. Data model
None.

## 5. Contracts and interfaces
- Message: `<usual refusal text> Tags received before the failure: v1.0, v2.0.`; code unchanged. Names are `tags_added` as the reconcile returns them (sorted by name, `git_tags.rs` end of `reconcile_incoming_tags`).
- Test hook: `#[cfg(test)] pub(crate) mod after_reconcile` in `git.rs` with `set(url, PullFault)` and `fire(url) -> Option<PullFault>`, `PullFault::{Fail(ToolError), Stall}`; the pull closure calls it after storing the slot: `Fail` returns `Err`, `Stall` awaits `std::future::pending()` so the call hits `with_remote_deadline`. Same pattern as `after_listing` (`git_tags.rs:757+`).

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/git.rs` | modify (sentence function, error path, hook, fifteen tests) | all | US-0154, US-0155 |
| `.agent_docs/git.md` | modify (one sentence in the pull section) | FR-NEW-001 | US-0155 |
Affected existing tests: keep all.

## 7. Compatibility and risks
Only the text of a failed pull changes, and only when a tag was added. Risk: on a deadline the worker thread may still run; the slot read is a snapshot (signal: E2E-010 is worded "when delivered"; it uses `Stall` after the slot is stored, so it is deterministic).

## 8. Security design
Not applicable. Tag names are repository data already shown in `tags_added`; no secret enters the message.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001, 002, 003, 004, 005, 008, 009, 010, 013, 014 | functional e2e | direct tool call (`call_pull_branch`, `Env::with_quota` as in `e2e_new_141` for a quota refusal, `after_reconcile` hook for a merge error and a deadline, `Env::with_hosts_and_timeout`) | `crates/core/src/tools/git.rs` tests | US-0154 |
| E2E-006, 007, 011, 012, 015 | functional e2e | same, with 20, 21, 24, 25 tags | `crates/core/src/tools/git.rs` tests | US-0155 |
Test data: generated bare remotes; tag lists built in a loop with `plain_tag_bare_remote`.

## 10. Decisions log
- **DEC-001:** the sentence is appended to the message, a space after the usual text. **Obliged by:** SPEC-0076/FR-NEW-001. **Rationale:** the error type has no data field. **Alternatives considered:** a new error field (rejected by the spec). **Evidence:** `errors.rs:30-40`.
- **DEC-002:** the slot is read with `mem::take` on both paths; on success the code of `git.rs:6997` is unchanged. **Obliged by:** SPEC-0076/FR-NEW-003. **Source:** design.
- **DEC-003:** the cap is the constant `MAX_NAMED_DELIVERED_TAGS = 20`, added in the second story. **Obliged by:** SPEC-0076/FR-NEW-002. **Source:** spec business rule.
- **DEC-004:** fault injection is a `#[cfg(test)]` hook keyed by URL, as `after_listing` is. **Obliged by:** SPEC-0076/FR-NEW-001 (E2E-003, E2E-010). **Rationale:** a merge error that is not a conflict and a deadline cannot be made from outside. **Alternatives considered:** a corrupted object (rejected: unreliable across libgit2 versions). **Source:** existing code `git_tags.rs:757-775`.
- **DEC-005:** two stories: US-0154 (the sentence without the cap, hook, ten tests), US-0155 (the cap, five tests, docs, closes the spec). **Obliged by:** n/a (no code impact): partitioning only. **Source:** design.
- **DEC-006:** red evidence: E2E-002, 003, 010 fail before the change (no sentence); E2E-001, 004, 005, 013, 014 guard unchanged text and are proven by the mutation "always append the sentence". **Obliged by:** SPEC-0076/FR-NEW-003. **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0154 | name the tags a failed pull delivered | P1 | FR-NEW-001, FR-NEW-003 | US-0153 | 2 | no |
| US-0155 | shorten a long list of delivered tags | P1 | FR-NEW-002 | US-0154 | 2 | yes |
