# US-0098: tags that are skipped, refused or changed on the remote

> Spec: SPEC-0054 (sdlc/specs/SPEC-0054-publish-all-tags/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0097
> min_tier: 1
> Files touched: 3
> Closes spec: no

## Objective
Give every skipped or refused tag its exact outcome and never let it stop the others: a remote tag that differs is reported `differs (remote at <7>)` and never replaced, a tag whose commit the remote lacks is skipped without sending the commit, a per tag refusal by the remote is reported while the others are sent, and a remote that changes between the listing and the send is detected by the lease, reclassified and the rest pushed. Carries the test seams for the refusal and for the drift.

## Requirements covered
- SPEC-0054/FR-NEW-004 (partial: only skipped tags, differing, commit not on remote): IF no local tag is to publish and the member states no number or the number 0, THEN THE Remote exchange context SHALL answer `No tag to publish.` with the list of FR-NEW-001 and send nothing.
- SPEC-0054/FR-NEW-005: The Remote exchange context SHALL NOT replace a remote tag that differs from the local tag of the same name when publishing all tags.
- SPEC-0054/FR-NEW-006: IF the commit of a local tag is not on the remote, THEN THE Remote exchange context SHALL skip that tag, report it `commit not on remote` and send no commit.
- SPEC-0054/FR-NEW-008: IF the remote refuses a tag, THEN THE Remote exchange context SHALL report it `refused by remote: <the reason the remote gave>`, send the other tags anyway and leave the refused local tag unchanged.
- SPEC-0054/FR-NEW-018: The Remote exchange context SHALL give each tag exactly one outcome, chosen in this order: `commit not on remote`, then `up to date`, then `differs (remote at <7 characters>)`, then `to publish`.
- SPEC-0054/FR-NEW-020: IF, between the listing taken by a confirmed action and its sending, the remote came to hold a tag of a name to publish that is not identical to the local tag (the FR-NEW-005 definition), THEN THE Remote exchange context SHALL NOT replace it, SHALL report that tag `differs (remote at <7 characters>)` where the 7 characters are the first 7 of the commit the remote tag points at now, and SHALL still send the other tags.

## Independent Test
`cargo test -p mcp-fs-core --lib publish_all` green; with the remote `v1.0` at another commit than the local one, confirming leaves `bare_ref_sha(remote, "refs/tags/v1.0")` unchanged while the other due tag arrives and exactly one audit entry exists.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0054): The push core `push_refs` (`git/remote.rs:609`) takes a slice of `PushRefs` and returns the per ref statuses; every existing caller (`push_to_remote`, `push_tag_to_remote`, `delete_tag_on_remote`) passes a one element slice and behaves as before. The status callback keeps a list of `(refname, message)` instead of one cell, and the lease check records every mismatched destination instead of the first. A creation refspec carries the lease "absent" (the zero object id) so a remote that gained the tag after the listing aborts the push before anything is sent. **Obliged by:** SPEC-0054/FR-NEW-002, FR-NEW-008, FR-NEW-020. **Rationale:** one connection for N tags and a stale listing detected without sending. **Alternatives considered:** a second push function duplicating the credential, deadline and callback setup (two copies to keep in step); no lease and trusting the remote to refuse an update (the local transport raises one hard error for the whole push, so no per tag report). **Source:** design. **Evidence:** `git/remote.rs:609-680`.
- DEC-006 (SPEC-0054): Classification, per local tag read from `entry.db.list_refs()` (prefix `refs/tags/`), sorted in Rust by the bytes of the name (never by the database collation). Order of checks, applied to the one listing (FR-NEW-018): commit not on remote (memoized per distinct commit with a `HashMap<Oid, bool>`, so tags sharing a commit cost one history walk), then identical (same object id, or two annotated tags on the same commit whose message the existing `probe_identical` compares), then differs (remote commit is the peeled `^{}` entry when advertised, shown with `short7`), then to publish. Time O(t log t + u d) for t tags, u distinct commits, d history depth, plus one probe connection per annotated lookalike. **Obliged by:** SPEC-0054/FR-NEW-001, FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-018. **Alternatives considered:** one probe fetch of all lookalikes at once (a larger refactor of `probe_remote_tag` for a case that is rare). **Source:** existing code. **Evidence:** `git_tags.rs:636-695`, `:986`.
- DEC-010 (SPEC-0054): The push set is the `to publish` tags whose name `git2::Reference::is_valid_name` accepts; a name it refuses (for example `a//b`, accepted by the branch rules, the SPEC-0052 case of an invalid reference name) is reported `refused by remote: invalid ref name` without a refspec, as the single publish refuses it before its push. Each tag ref is first written to the on disk repository (`repo.reference`), as the single publish does, because a push names refs. **Obliged by:** SPEC-0054/FR-NEW-008. **Alternatives considered:** pushing it and letting libgit2 fail the whole push. **Source:** existing code. **Evidence:** `git_tags.rs:688-695`.
- DEC-011 (SPEC-0054): Reading the push result. Ok with no rejection: every pushed tag is `published`. Per ref rejections (smart protocol): that tag is `refused by remote: <reason>`, the others are `published`. Lease lost on some refs (nothing was sent): re list once, move each lost tag to `differs (remote at <7>)` or, when the remote now holds the identical object, `up to date`, drop it from the push set and push the rest again; every round removes at least one tag, so at most t rounds, and in practice one. The lists of FR-NEW-020 are decided by the fresh listing, tags not due at the first listing keep their first outcome. Any other failure (open, transport) is the connection refusal and reports nothing as sent. **Obliged by:** SPEC-0054/FR-NEW-008, FR-NEW-012, FR-NEW-020. **Alternatives considered:** one global retry (leaves a second race); reporting the tags a dropped connection may have applied (unknowable, FR-NEW-012 reports none). **Source:** design.
- DEC-013 (SPEC-0054): `#[cfg(test)]` seams carry what a local file remote cannot produce, all keyed by remote URL and one shot so parallel tests never meet: `after_listing(url)` (exists, `git_tags.rs`, reused: moves or creates a remote tag between listing and push, E2E-054 to 056); `bulk_seams::reject(url, [(tag, reason)])` in `git/remote.rs` (the named tags are held back from the real push and reported as per ref rejections, E2E-016, 033, 041); `bulk_seams::drop_after(url, n)` (the real push carries only the first n refspecs, then reports a connection error, E2E-021). **Obliged by:** SPEC-0054/FR-NEW-008, FR-NEW-012, FR-NEW-020. **Alternatives considered:** a git hook on the bare remote (the local transport runs no hook); a fake smart HTTP server (a large harness for three tests). **Source:** existing code. **Evidence:** `git_tags.rs:745-770`, `git/remote.rs:739-750`.
- DEC-015 (SPEC-0054): No existing test is modified; the 59 tests are all new. The assertions that change are counts, registry lengths and parity docs, as for every new tool. **Obliged by:** SPEC-0054/FR-NEW-016. **Alternatives considered:** none. **Source:** design.

### Data model
No schema change; no database write beyond what the single publish already does.

### Contracts
**`git.remote_push_tags`** (new in US-0096): parameters `mount_id` (string, required), `remote` (string, default `origin`, empty means `origin`), `confirm_count` (optional string or integer; absent or null is a preview). Result `{mount_id, remote, to_publish, published, sent, tags: [{name, outcome, commit, remote_commit, reason}], lines, message, auth}` with `outcome` one of `to publish`, `published`, `up to date`, `differs`, `commit not on remote`, `refused by remote`; `lines` hold the exact texts of the spec in byte order of the name. Refusals and codes: no remote `ERR_NOT_FOUND` `Cannot publish tags: this project tracks no remote.`; unknown remote `ERR_NOT_FOUND` `Cannot publish tags: this project does not track a remote named '<r>'.`; not a count `ERR_INVALID_ARGUMENT` `The number to confirm must be a whole number, zero or more.`; wrong number `ERR_NO_CLOBBER` `Tags to publish: <M>, not <N>. Review the list, then confirm again.` followed by one line per tag; connection, credential and timeout texts are the existing ones. Audit: action `git.tag_publish`, path `/`, `published tag '<n>' at <full 40>` per tag sent.

### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (outcome mapping, lease lost round, invalid ref name) |
| `crates/core/src/git/remote.rs` | modify (per ref status, lease for creation, `bulk_seams::reject`) |
| `crates/core/src/tools/git.rs` | modify (14 tests) |

### Existing patterns
`push_refs`, `delete_seams` (`#[cfg(test)]`, `forced_rejection`) in `crates/core/src/git/remote.rs`; `after_listing` in `crates/core/src/tools/git_tags.rs`; the SPEC-0053 tests `e2e_tag_019_remote_delete_*` and `e2e_tag_031_remote_delete_*` for the seam shapes.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc. Test functions are named `e2e_tag_<number>_publish_all_<slug>` in the tests module of `crates/core/src/tools/git.rs`. Level: functional e2e; driver: the direct helper `call_publish_all_tags` with a `file://` bare remote (the registered tool for the non member and credential cases). Fixtures: project `proj-a`, remote `origin` at the host `github.ibm.com`, members Alice and Bob, commits `a1b2c3d`, `b2c3d4e`, `c3d4e5f` on the remote unless stated (use the real commit ids the fixtures produce).

### SPEC-0054/E2E-013: A differing remote tag is skipped, the others sent
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_013_publish_all_*`
- **Given** local `v1.0` at `a1b2c3d` while the remote `v1.0` is at `f6a7b8c`; local `v2.0` at `b2c3d4e`, not on the remote. **When** Alice previews, then confirms stating 1. **Then** the preview is exactly `v1.0 differs (remote at f6a7b8c)`, `v2.0 to publish`, `Tags to publish: 1. Confirm by stating 1 to send them.`; the report is exactly `v1.0 differs (remote at f6a7b8c)`, `v2.0 published`. **And** the remote `v1.0` is still at `f6a7b8c`, and the audit trail has one new entry (`v2.0`).
- **Priority:** Critical

### SPEC-0054/E2E-014: A differing kind is skipped
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_014_publish_all_*`
- **Given** the local tag `v1.0` is lightweight at `a1b2c3d`, the remote `v1.0` is annotated at `a1b2c3d`. **When** Alice previews. **Then** the answer is exactly `v1.0 differs (remote at a1b2c3d)` then `No tag to publish.` **And** the remote tag is unchanged.
- **Priority:** High

### SPEC-0054/E2E-015: A tag whose commit the remote lacks is skipped
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_015_publish_all_*`
- **Given** local `wip-alice` at `d4e5f6a`, a commit not on the remote; local `v2.0` at `b2c3d4e`. **When** Alice previews, then confirms stating 1. **Then** the report is exactly `v2.0 published`, `wip-alice commit not on remote`. **And** the remote does not hold the commit `d4e5f6a` nor a `wip-alice` tag.
- **Priority:** Critical

### SPEC-0054/E2E-016: The remote refuses one tag
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_016_publish_all_*`
- **Given** local `v1.0`, `refused-tag` and `v2.0`, all on remote commits; the remote refuses `refused-tag` with the reason `protected tag name`. **When** Alice confirms stating 3. **Then** the report is exactly `refused-tag refused by remote: protected tag name`, `v1.0 published`, `v2.0 published`. **And** the local `refused-tag` is unchanged and the audit trail has two new entries (`v1.0`, `v2.0`).
- **Priority:** High

### SPEC-0054/E2E-023: Nothing is sendable
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_023_publish_all_*`
- **Given** local `v1.0` (remote at `f6a7b8c`, differs) and `wip-alice` (commit not on the remote). **When** Alice previews, then asks to confirm stating 0. **Then** both answers are exactly `v1.0 differs (remote at f6a7b8c)`, `wip-alice commit not on remote`, `No tag to publish.` **And** the remote and the audit trail are unchanged.
- **Priority:** High

### SPEC-0054/E2E-033: No audit for a tag the remote refused
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_033_publish_all_*`
- **Given** the state of E2E-016. **When** Alice confirms stating 3. **Then** no audit entry names `refused-tag`.
- **Priority:** High

### SPEC-0054/E2E-037: A missing commit wins over a differing remote tag
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_037_publish_all_*`
- **Given** local `v1.0` at `d4e5f6a`, a commit not on the remote, while the remote holds `v1.0` at `f6a7b8c`. **When** Alice previews. **Then** the answer is exactly `v1.0 commit not on remote`, `No tag to publish.` and never `differs`.
- **Priority:** High

### SPEC-0054/E2E-040: Two tags whose commit the remote lacks
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_040_publish_all_*`
- **Given** local `wip-alice` and `wip-bob` both at `d4e5f6a`, not on the remote. **When** Alice previews. **Then** the answer is exactly `wip-alice commit not on remote`, `wip-bob commit not on remote`, `No tag to publish.` **And** nothing is sent.
- **Priority:** Medium

### SPEC-0054/E2E-041: The remote refuses every tag
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_041_publish_all_*`
- **Given** `v1.0` and `v1.1` due and the remote refusing every tag with the reason `protected tag name`. **When** Alice confirms stating 2. **Then** the report is exactly `v1.0 refused by remote: protected tag name`, `v1.1 refused by remote: protected tag name`. **And** the audit trail is unchanged and the local tags are unchanged.
- **Priority:** High

### SPEC-0054/E2E-051: A differing message makes an annotated tag differ
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_051_publish_all_*`
- **Given** the local annotated tag `v2.0` with the message `Release 2.0` at `a1b2c3d`, and the remote annotated tag `v2.0` at `a1b2c3d` with the message `Other`. **When** Alice previews. **Then** the answer is exactly `v2.0 differs (remote at a1b2c3d)`, `No tag to publish.`
- **Priority:** Medium

### SPEC-0054/E2E-054: A remote tag appears between the listing and the send
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_054_publish_all_*`
- **Given** local `v1.0` at `a1b2c3d` and `v2.0` at `b2c3d4e`, none on the remote; Alice previewed `Tags to publish: 2`. **When** Alice confirms stating 2 and, after the product listed the remote for this confirmation and before it sends, the remote receives `v1.0` at `f6a7b8c` from another source. **Then** the report is exactly `v1.0 differs (remote at f6a7b8c)`, `v2.0 published`. **And** the remote `v1.0` is still at `f6a7b8c` and the audit trail has one new entry (`v2.0`).
- **Priority:** High

### SPEC-0054/E2E-055: A remote tag moves between the listing and the send
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_055_publish_all_*`
- **Given** local `v1.0` at `a1b2c3d` and the remote `v1.0` also at `a1b2c3d` at the listing, so `v1.0 up to date`; and `v2.0` at `b2c3d4e` due. **When** Alice confirms stating 1 and, after the product listed the remote for this confirmation and before it sends, the remote moves `v1.0` to `f6a7b8c`. **Then** the report is exactly `v1.0 up to date`, `v2.0 published` (the line states the outcome of the listing; it is not examined again). **And** the remote `v1.0` is at `f6a7b8c` and the audit trail has one new entry (`v2.0`).
- **Priority:** Medium

### SPEC-0054/E2E-056: A due tag is created on the remote by another member before the send
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_056_publish_all_*`
- **Given** local `v1.0` at `a1b2c3d` and `v2.0` at `b2c3d4e`, none on the remote; Alice previewed 2. **When** Alice confirms stating 2 and, after the product listed the remote for this confirmation and before it sends, the remote receives `v1.0` at `a1b2c3d` (the same tag). **Then** the report is exactly `v1.0 up to date`, `v2.0 published`. **And** the audit trail has one new entry (`v2.0`).
- **Priority:** Medium

### SPEC-0054/E2E-057: A name the remote refuses as a reference
- Level / Driver / Location: functional e2e / direct helper `call_publish_all_tags`, `file://` bare remote / `crates/core/src/tools/git.rs`, `e2e_tag_057_publish_all_*`
- **Given** local `a//b` and `v1.0`, on remote commits, none on the remote. **When** Alice previews, then confirms stating 2. **Then** the preview is exactly `a//b to publish`, `v1.0 to publish`, `Tags to publish: 2. Confirm by stating 2 to send them.` and the report is exactly `a//b refused by remote: invalid ref name`, `v1.0 published`. **And** the audit trail has one new entry (`v1.0`).
- **Priority:** Medium

## Constraints
- Files not to touch: `crates/core/src/mcp/server.rs`, contract files, anything under `sdlc/specs/`.
- Dependencies not to add: none.
- Patterns to avoid: a retry that pushes a tag the fresh listing shows differing; production code reading a test seam outside `#[cfg(test)]`.
- Scope boundary: connection failures, concurrency, pause, cost (US-0099); single publish regression and docs (US-0100).

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (a test that exercises behaviour already delivered by an earlier story may pass at once: say so).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every US-0096 and US-0097 test and every existing git test passes unmodified; the branch push, tag push and remote delete tests pass unmodified.
- [ ] Docs updated: none beyond code comments.
- [ ] Self review: full (performance, idiomatic Rust per the rust skill, no `unwrap` on fallible paths, no dash punctuation).
- [ ] One commit `US-0098: tags that are skipped, refused or changed on the remote`, body `Spec: SPEC-0054` and the requirement ids.
