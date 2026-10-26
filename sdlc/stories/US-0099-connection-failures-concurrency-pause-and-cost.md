# US-0099: connection failures, concurrency, paused operations and cost

> Spec: SPEC-0054 (sdlc/specs/SPEC-0054-publish-all-tags/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0098
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
Prove and complete the failure and scale behaviour: the existing connection texts for down, rate limited, no answer and credential problems with nothing reported sent, a dropped connection mid send leaving a safe retry (the `drop_after` seam), two members confirming at once, a paused merge, rebase or cherry pick not blocking, and 50 tags within the deadline. Most of it is delivered by US-0096 to US-0098; add only what a failing test proves missing.

## Requirements covered
- SPEC-0054/FR-NEW-012: IF the remote cannot be reached, answers too slowly, rate limits or the credential is missing or expired, THEN THE Remote exchange context SHALL refuse with the texts publishing one tag gives and report no tag as sent.
- SPEC-0054/FR-NEW-013: WHEN two members confirm at the same time, THE Remote exchange context SHALL send each tag once and give the second member the refusal of FR-NEW-003, because the count of tags to publish is then smaller than the number the second member stated.
- SPEC-0054/FR-NEW-014: The Remote exchange context SHALL NOT refuse to publish all tags because a merge, rebase, cherry pick or revert is paused, and SHALL leave that paused operation as it was.
- SPEC-0054/FR-NEW-015 (partial: the connection position in the order): The Remote exchange context SHALL give refusals in this order: non member, remote lookup, connection and credential, then a number that is not a count (FR-NEW-019) or a wrong number (FR-NEW-003).
- SPEC-0054/FR-NEW-017: WHEN 50 tags are due, THE Remote exchange context SHALL finish the preview and the confirmed send each without a timeout refusal under the same configured remote deadline that applies to publishing one tag.

## Independent Test
`cargo test -p mcp-fs-core --lib publish_all` green; after a dropped connection the retry reports the tag the remote had applied as `up to date` and writes one audit entry for the other.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0054): Flow and order of refusals follow the order of the calls: `open` (member), remote lookup (none at all, then named not tracked), `resolve_clone_credential`, one listing connection (`list_advertised_refs`), then `parse_confirm` (not a count), then the count comparison (wrong number), then the push. Codes in Section 5. **Obliged by:** SPEC-0054/FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-015, FR-NEW-019. **Alternatives considered:** parsing the number before the connection (rejected: FR-NEW-015 puts the connection first, E2E-050). **Source:** design. **Evidence:** `git_tags.rs:467-505`.
- DEC-007 (SPEC-0054): The whole action holds the repository `write_lock`, preview included, so the count shown and the count checked come from one consistent view and two confirmations serialize; the second finds nothing to publish and gets the wrong number refusal with `Tags to publish: 0, not N` (FR-NEW-013). `reject_if_operation_in_progress` is NOT called (FR-NEW-014). The span is `trace_remote_operation("git.remote_push_tags", ...)`. **Obliged by:** SPEC-0054/FR-NEW-013, FR-NEW-014. **Alternatives considered:** no lock for the preview (a preview racing a confirm could show a stale count; harmless but untestable); taking the lock only for the send (the count check would race). **Source:** existing code. **Evidence:** `git_tags.rs:560`.
- DEC-011 (SPEC-0054): Reading the push result. Ok with no rejection: every pushed tag is `published`. Per ref rejections (smart protocol): that tag is `refused by remote: <reason>`, the others are `published`. Lease lost on some refs (nothing was sent): re list once, move each lost tag to `differs (remote at <7>)` or, when the remote now holds the identical object, `up to date`, drop it from the push set and push the rest again; every round removes at least one tag, so at most t rounds, and in practice one. The lists of FR-NEW-020 are decided by the fresh listing, tags not due at the first listing keep their first outcome. Any other failure (open, transport) is the connection refusal and reports nothing as sent. **Obliged by:** SPEC-0054/FR-NEW-008, FR-NEW-012, FR-NEW-020. **Alternatives considered:** one global retry (leaves a second race); reporting the tags a dropped connection may have applied (unknowable, FR-NEW-012 reports none). **Source:** design.
- DEC-013 (SPEC-0054): `#[cfg(test)]` seams carry what a local file remote cannot produce, all keyed by remote URL and one shot so parallel tests never meet: `after_listing(url)` (exists, `git_tags.rs`, reused: moves or creates a remote tag between listing and push, E2E-054 to 056); `bulk_seams::reject(url, [(tag, reason)])` in `git/remote.rs` (the named tags are held back from the real push and reported as per ref rejections, E2E-016, 033, 041); `bulk_seams::drop_after(url, n)` (the real push carries only the first n refspecs, then reports a connection error, E2E-021). **Obliged by:** SPEC-0054/FR-NEW-008, FR-NEW-012, FR-NEW-020. **Alternatives considered:** a git hook on the bare remote (the local transport runs no hook); a fake smart HTTP server (a large harness for three tests). **Source:** existing code. **Evidence:** `git_tags.rs:745-770`, `git/remote.rs:739-750`.
- DEC-016 (SPEC-0054): Cost: the whole action shares one deadline (`remote_timeout_secs`), one listing, one push per round, and one probe connection per annotated tag whose object differs from the remote's but shares its commit. Identical objects short circuit without a probe, so 50 annotated tags already on the remote cost nothing extra (E2E-026). **Obliged by:** SPEC-0054/FR-NEW-017. **Alternatives considered:** batching the probes (DEC-006). **Source:** existing code. **Evidence:** `git_tags.rs:673-737`.
- DEC-015 (SPEC-0054): No existing test is modified; the 59 tests are all new. The assertions that change are counts, registry lengths and parity docs, as for every new tool. **Obliged by:** SPEC-0054/FR-NEW-016. **Alternatives considered:** none. **Source:** design.

### Data model
No schema change; no database write beyond what the single publish already does.

### Contracts
**`git.remote_push_tags`** (new in US-0096): parameters `mount_id` (string, required), `remote` (string, default `origin`, empty means `origin`), `confirm_count` (optional string or integer; absent or null is a preview). Result `{mount_id, remote, to_publish, published, sent, tags: [{name, outcome, commit, remote_commit, reason}], lines, message, auth}` with `outcome` one of `to publish`, `published`, `up to date`, `differs`, `commit not on remote`, `refused by remote`; `lines` hold the exact texts of the spec in byte order of the name. Refusals and codes: no remote `ERR_NOT_FOUND` `Cannot publish tags: this project tracks no remote.`; unknown remote `ERR_NOT_FOUND` `Cannot publish tags: this project does not track a remote named '<r>'.`; not a count `ERR_INVALID_ARGUMENT` `The number to confirm must be a whole number, zero or more.`; wrong number `ERR_NO_CLOBBER` `Tags to publish: <M>, not <N>. Review the list, then confirm again.` followed by one line per tag; connection, credential and timeout texts are the existing ones. Audit: action `git.tag_publish`, path `/`, `published tag '<n>' at <full 40>` per tag sent.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (`bulk_seams::drop_after`) |
| `crates/core/src/tools/git_tags.rs` | modify only if a test proves a gap |
| `crates/core/src/tools/git.rs` | modify (12 tests) |

### Existing patterns
`closing_remote`, `silent_remote`, `with_git_hosts_lock` and the SPEC-0053 tests `e2e_tag_032_remote_delete_*` (connection legs), `e2e_tag_030_remote_delete_*` (two members at once with the held write lock), `e2e_tag_029_remote_delete_*` (paused operations) in `crates/core/src/tools/git.rs`.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Test functions are named `e2e_tag_<number>_publish_all_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_publish_all_tags` with a `file://` bare remote (the registered tool for the non member and credential cases). Fixtures: project `proj-a`, remote `origin` at the host `github.ibm.com`, members Alice and Bob, commits `a1b2c3d`, `b2c3d4e`, `c3d4e5f` on the remote unless stated (use the real commit ids the fixtures produce).

### SPEC-0054/E2E-020: The remote is unreachable
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_020_publish_all_*`
- **Given** two tags due and the remote down. **When** Alice previews. **Then** she is refused with `fetch from host 'github.ibm.com' failed: connection refused` (the remote refuses connections). **And** nothing is reported as sent and the audit trail is unchanged.
- **Priority:** High

### SPEC-0054/E2E-021: The connection drops while sending, retry is safe
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_021_publish_all_*`
- **Given** two tags due, `v1.0` and `v1.1`. **When** Alice confirms stating 2 and the connection drops after the remote applied `v1.0` only. **Then** Alice is refused with `fetch from host 'github.ibm.com' failed: connection reset` (the remote drops the connection) and no tag is reported as sent. **And** a retry (preview then confirm stating 1) reports `v1.0 up to date`, `v1.1 published`, and the audit trail then has exactly one entry for `v1.1`.
- **Priority:** High

### SPEC-0054/E2E-022: Credential or answer problems
- Level / Driver / Location: functional e2e / registered tool, `e.call` / `crates/core/src/tools/git.rs`, `e2e_tag_022_publish_all_*`
- **Given** two tags due. **When** Alice previews with an expired credential, with no stored credential, with a remote that rate limits, and with a remote that does not answer within 1 s. **Then** the refusals are, in turn, `token expired for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, `no token for host github.ibm.com; authenticate with git.auth or git.token_set for host github.ibm.com`, `fetch from host 'github.ibm.com' failed: rate limit exceeded`, `remote timeout: host 'github.ibm.com' did not respond within 1s`. **And** nothing is sent.
- **Priority:** High

### SPEC-0054/E2E-026: Fifty tags, annotated and lightweight
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_026_publish_all_*`
- **Given** 25 annotated tags `a01` to `a25` (message `Release a01` and so on) already on the remote at the same commits with the same messages, and 25 lightweight tags `r26` to `r50` on remote commits, none on the remote. **When** Alice previews and confirms stating 25. **Then** neither answer is a timeout refusal; the report has 50 lines in name order, `a01 up to date` to `a25 up to date` then `r26 published` to `r50 published`. **And** the remote holds 50 tags and the audit trail holds 25 new entries.
- **Priority:** Medium

### SPEC-0054/E2E-029: A connection failure beats a wrong number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_029_publish_all_*`
- **Given** 2 tags due and the remote down. **When** Alice confirms stating 7. **Then** the answer is `fetch from host 'github.ibm.com' failed: connection refused` and not the wrong number text.
- **Priority:** Medium

### SPEC-0054/E2E-035: Two members confirm at once
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_035_publish_all_*`
- **Given** `v1.0` and `v1.1` due; Alice and Bob both previewed and saw `Tags to publish: 2`. **When** both confirm stating 2 at the same moment. **Then** exactly one of them receives `v1.0 published`, `v1.1 published`. **And** the other receives `Tags to publish: 0, not 2. Review the list, then confirm again.` followed by the lines `v1.0 up to date`, `v1.1 up to date`; the remote holds both tags once, and the audit trail has exactly two new entries, one per tag, both by the member who sent.
- **Priority:** High

### SPEC-0054/E2E-036: A paused merge does not block
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_036_publish_all_*`
- **Given** a merge paused on a conflict in `proj-a` and `v1.0` due. **When** Alice confirms stating 1. **Then** the report says `v1.0 published`. **And** the merge is still paused with the same conflicts.
- **Priority:** Medium

### SPEC-0054/E2E-043: A paused rebase does not block
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_043_publish_all_*`
- **Given** a rebase paused on a conflict in `proj-a` and `v1.0` due. **When** Alice confirms stating 1. **Then** the report is exactly `v1.0 published`. **And** the rebase is still paused with the same conflicts.
- **Priority:** Medium

### SPEC-0054/E2E-045: Fifty tags, all up to date
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_045_publish_all_*`
- **Given** 50 lightweight tags `r01` to `r50` held identically by the remote. **When** Alice previews. **Then** the answer is 50 lines `<name> up to date` in name order followed by `No tag to publish.`, without a timeout refusal.
- **Priority:** Medium

### SPEC-0054/E2E-046: A paused cherry pick does not block
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_046_publish_all_*`
- **Given** a cherry pick paused on a conflict in `proj-a` and `v1.0` due. **When** Alice confirms stating 1. **Then** the report is exactly `v1.0 published`. **And** the cherry pick is still paused.
- **Priority:** Medium

### SPEC-0054/E2E-047: Fifty tags due, preview only
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_047_publish_all_*`
- **Given** 50 lightweight tags `r01` to `r50` on remote commits, none on the remote. **When** Alice previews. **Then** the answer is 50 lines `<name> to publish` in name order followed by `Tags to publish: 50. Confirm by stating 50 to send them.`, without a timeout refusal. **And** the remote has no new tag.
- **Priority:** Medium

### SPEC-0054/E2E-050: A word instead of a number
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_050_publish_all_*`
- **Given** a project with no remote and two tags. **When** Alice confirms stating `two`. **Then** the answer is `Cannot publish tags: this project tracks no remote.` **And** with a remote tracked and two tags due, stating `two` gives `The number to confirm must be a whole number, zero or more.` **And** with the remote down, stating `two` gives `fetch from host 'github.ibm.com' failed: connection refused`.
- **Priority:** Medium

## Constraints
- Files not to touch: `crates/core/src/mcp/server.rs`, contract files, anything under `sdlc/specs/`.
- Dependencies not to add: none.
- Patterns to avoid: timing based assertions (hold the write lock instead of sleeping).
- Scope boundary: single publish regression and documentation (US-0100).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered by an earlier story may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every earlier SPEC-0054 test and every existing git test passes unmodified.
- [ ] Docs updated: none beyond code comments.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0099: connection failures, concurrency, paused operations and cost`, body `Spec: SPEC-0054` and the requirement ids.
