# US-0150: word a publish all failure as a push of tags

> Spec: SPEC-0072 (sdlc/specs/SPEC-0072-tag-publish-error-wording/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0149
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
Publishing all tags reads `fetch from host` when the remote cannot be reached. Name the push of tags, adapt the five existing assertions that checked the old text, and document the wording.

## Requirements covered
- SPEC-0072/FR-NEW-002: WHEN the connection to the remote fails while a member publishes all tags THE Remote exchange context SHALL refuse with `push of tags to host '<host>' failed: <cause>`.
- SPEC-0072/FR-MOD-001 (partial: publish all; one tag publish is US-0149): WHEN a connection fails during a tag publish THE Remote exchange context SHALL use FR-NEW-001 or FR-NEW-002 instead of `fetch from host '<host>' failed: <cause>`.

## Independent Test
`cargo test -p mcp-fs-core --lib git` is green including E2E-001, 003, 011 and the adapted assertions; publishing all tags to a missing remote reads `push of tags to host '<host>' failed: `.

## Design excerpt
### Decisions
- DEC-001: wording is chosen by an enum argument `ConnectionWording`, default `Fetch`. Obliged by: SPEC-0072/FR-NEW-001, SPEC-0072/FR-NEW-002, SPEC-0072/FR-MOD-001. Rationale: FR-MOD-001 and E2E-006 need the same callee to word differently per caller. Alternatives considered: per function copies of `connection_error` (rejected: 11 call sites). Source: design. Evidence: `remote.rs:1568-1571`.
- DEC-005: two stories: US-0149 (one tag: enum with `Fetch` and `PublishTag`, the vanished tag, eight tests), US-0150 (publish all: `PublishTags`, the adapted assertions, docs, four tests). Obliged by: n/a (no code impact): partitioning only. Rationale: the adapted assertions must change together with the production wording they check, and only publish all uses them. Source: design.
- DEC-006: the preview of publish all (`git_tags.rs:1028`, `:1234` listings) is a publish step and takes `PublishTags`. Obliged by: SPEC-0072/FR-NEW-002. Source: spec business rule.
### Data model
None.
### Contracts
`ConnectionWording::PublishTags`; the listings of `publish_all_on_repo` (`git_tags.rs:1028`) and the lease refresh (`:1234`) pass it; `push_tags_to_remote` (`remote.rs:882`, `:896`) uses it. Adapt `assert_connection_refused` (`git.rs:29491`) to `publish_tags_prefix(url)` for `e2e_tag_020`, `e2e_tag_022`, `e2e_tag_029`, `e2e_tag_050` and the `lost` string at `:29528`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/git/remote.rs` | modify (`PublishTags`, `push_tags_to_remote`) |
| `crates/core/src/tools/git_tags.rs` | modify (publish all listings) |
| `crates/core/src/tools/git.rs` | modify (tests, `publish_tags_prefix`, five adapted assertions) |
| `.agent_docs/git.md` | modify (wording table) |
### Existing patterns
the work of US-0149 in the same files; `assert_connection_refused` at `git.rs:29491-29495`; `bulk_seams::drop_after` used at `git.rs:29526`.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc.


### SPEC-0072/E2E-001: publishing still works
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** Alice, local tags `v1.0` and `v1.1` not yet on a reachable remote. **When** she publishes `v1.0`, then all tags. **Then** both succeed with the answers they gave before. **And** the remote holds both tags.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-003: all tags, remote down
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** Alice, tags `v1.0`, `v1.1` due, a remote that does not exist. **When** she publishes all tags. **Then** the refusal starts `push of tags to host '<host>' failed: `. **And** the audit log is unchanged.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-011: publish all, drop after the first tag
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Given** two due tags and a remote that drops after the first. **When** Alice publishes all with the number 2. **Then** the refusal starts `push of tags to host '<host>' failed: ` and the remote holds only `v1.0`.
- Cleanup: temp directories are dropped with the test.

### SPEC-0072/E2E-MOD-001: the publish all connection assertions (were the publish all connection checks of `e2e_tag_020`, `e2e_tag_022`, `e2e_tag_029`, `e2e_tag_050`, and the drop assertion at `git.rs:29528`)
- Level / Driver / Location: functional e2e / direct tool call on `Env` / `crates/core/src/tools/git.rs` tests
- **Validated before:** the refusal starts `fetch from host '<host>' failed: `.
- **Validates now:** it starts `push of tags to host '<host>' failed: `, same cause text.
- **Given / When / Then:** as the originals; only the expected prefix changes.
- Cleanup: temp directories are dropped with the test.

## Constraints
- Files not to touch: any `sdlc-spec.md`, receive-pack and upload-pack code, `tool-contract-golden.json`.
- Dependencies not to add: none.
- Patterns to avoid: dashes as punctuation in code, comments or docs.
- Scope boundary: the other stories of the queue.

## Definition of done
- [ ] RED evidence recorded for E2E-003 and E2E-011 and for the adapted assertions (they fail on the old text once the expected prefix is changed); E2E-001 guards behavior that exists and is proven by the mutation "publish all always fails".
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: all tests of `cargo test -p mcp-fs-core --lib git` keep passing; the only existing tests changed are the five assertions of E2E-MOD-001, each by its expected prefix only.
- [ ] Docs updated: `.agent_docs/git.md` (error wording of the tag section).
- [ ] Self review: full.
- [ ] Closes spec: the spec's full acceptance set green and every requirement of SPEC-0072 cited by at least one story with status done or landed.
- [ ] One commit `US-0150: word a publish all failure as a push of tags`, body `Spec: SPEC-0072` and the requirement ids.
