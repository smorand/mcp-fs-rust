# US-0118: look up each listed tag once and close the spec

> Spec: SPEC-0061 (sdlc/specs/SPEC-0061-tag-object-import/)
> Nature: DEBT
> Priority: P2
> Depends on: US-0117
> min_tier: 2
> Files touched: 1
> Closes spec: yes

## Objective
Make the tag listing look each target up once instead of up to three times, then close the spec: prove the whole spec with the structural check and the before and after measurement.

## Requirements covered
- SPEC-0061/DR-003 [EARS-U]: `tag_entry` SHALL perform one `repo.find_object` per tag and SHALL derive the kind (`annotated` iff the object is a tag object), the peeled commit and the annotation details from that object, with no further object lookup in the listing path.
  - Old form: `tag_entry` calls `find_object`, then `peel_to_commit(repo, &target)` (a second `find_object`), then `annotation_details(repo, &target, map)` (`find_tag`, a third lookup). New form: `tag_entry` looks the target up once with `Oid::from_str(&target).ok().and_then(|oid| repo.find_object(oid, None).ok())`; `sha` is `object.peel_to_commit()` mapped to its id string, falling back to `target` when the object is missing or does not peel (same fallback as today); `annotation_details` takes `&git2::Tag` obtained with `object.as_tag()`. `peel_to_commit` stays for its other callers (`git_tags.rs:162,210,281,641,1077`).
- SPEC-0061/DR-004 [EARS-U]: The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; the object stored for an annotated tag has the same id, type, payload, blob key and index row as before.

## Independent Test
`cargo test -p mcp-fs-core --lib e2e_tag` stays green (212 tests plus the 2 of US-0117: 214). The structural commands of DT-007 print what the test says. The measurement DT-008 is run and its two tables go in the commit body. The listing output is unchanged byte for byte: the existing listing tests (`e2e_tag_020`, `022`, `025` and the annotated ones) are the non mock assertion.

## Design excerpt
### Decisions
- DEC-004 (SPEC-0061): the listing is given no speed target; the structural check counts lookups instead of asserting a timing.
- DEC-006: the measurement is evidence in the commit body, not a gating test.
### Data model
None. No stored format changes: blob key `git:{sha}` and the object index row are exactly as before.
### Contracts
No tool schema or output changes. `git.tags` entries keep the fields `name`, `full_ref`, `sha`, `object`, `kind` and, for annotated tags, `message`, `tagger`, `tagger_email`, `tagged_at`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/git_tags.rs` | modify (`tag_entry` lines 340-364, `annotation_details` line 366) |
### Existing patterns
Current code (`crates/core/src/tools/git_tags.rs:340-380`): `tag_entry` finds the object to learn `annotated`, calls `peel_to_commit(repo, &target)` (which calls `repo.find_object` again, lines 307-311) for `sha`, and for an annotated tag calls `annotation_details(repo, &target, map)` which does `repo.find_tag(oid)` (a third lookup). Target shape:
```rust
fn tag_entry(repo: &Repository, full_ref: String, target: String) -> Value {
    let name = full_ref.strip_prefix(TAGS_PREFIX).unwrap_or(&full_ref).to_string();
    let object = Oid::from_str(&target).ok().and_then(|oid| repo.find_object(oid, None).ok());
    let annotated = object.as_ref().is_some_and(|o| o.kind() == Some(ObjectType::Tag));
    let sha = object.as_ref()
        .and_then(|o| o.peel_to_commit().ok())
        .map_or_else(|| target.clone(), |c| c.id().to_string());
    // ... same json! as today; for annotated: if let Some(tag) = object.as_ref().and_then(|o| o.as_tag()) { annotation_details(tag, map) }
}
fn annotation_details(tag: &git2::Tag<'_>, map: &mut serde_json::Map<String, Value>) { /* body unchanged, minus the find_tag */ }
```
Edge cases to keep byte identical: a missing object or an invalid id text lists the tag as lightweight with `sha` equal to its own target; a tag on a tree or blob (peel fails) falls back to `target`; a tag of a tag peels fully.

## Acceptance tests
> 100% must pass, run through `make check`. Never run test files ad hoc. DEBT success criterion: the existing suite passes unmodified, exceptions: none.

### SPEC-0061/DT-007: structural, one lookup per listed tag and no whole import in tag_create
- Level / Driver / Location: structural / shell / `crates/core/src/tools/git_tags.rs`
- When `awk '/^async fn tag_create/,/^}/' crates/core/src/tools/git_tags.rs | rg 'import_from_repo'` runs
- Then it prints nothing (rg exit status 1); `rg -c 'import_object' crates/core/src/tools/git_tags.rs` prints `1`
- And `awk '/^fn tag_entry/,/^}/' crates/core/src/tools/git_tags.rs | rg -c 'find_object|find_tag|\bpeel_to_commit\(repo'` prints `1` (the pattern matches the free helper called as `peel_to_commit(repo, ...)` and not the `object.peel_to_commit()` method), and `awk '/^fn annotation_details/,/^}/' crates/core/src/tools/git_tags.rs | rg 'find_object|find_tag'` prints nothing

### SPEC-0061/DT-008: measurement, not gating
- Level / Driver / Location: measurement / temporary test, not committed / scratch in `crates/core/src/tools/git.rs`
- Given the temporary test (create N blobs with `repo.blob`, `import_from_repo`, then time 5 `tag(&e, json!({"name": ...}))` and 5 `tag(&e, json!({"name": ..., "message": "m"}))`, print milliseconds), for N in 500, 5 000, 20 000
- When run on the final tip of the chain
- Then at N = 20 000 each annotated sample is within 2 times the matching lightweight sample plus 20 ms; both tables (before from `SPEC-0061 Section 2`, after measured now) are in the commit body; the scratch test is reverted (`git checkout -- crates/core/src/tools/git.rs` before committing)

## Constraints
- Files not to touch: `crates/core/src/git/odb.rs`; the other callers of `peel_to_commit`.
- Dependencies not to add: none.
- Patterns to avoid: a timing assertion in a committed test; changing the listing output order or fields.
- Scope boundary: nothing else; BL-0110 and BL-0111 are separate.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code (mutation proof where the story says so; revert it with `git checkout -- <file>`).
- [ ] Every acceptance test above green; `make check` green (cargo test --workspace, clippy -D warnings, fmt --check); main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified, exceptions: none (0 existing tests modified). `cargo test -p mcp-fs-core --lib e2e_tag` stays green (212 tests today).
- [ ] DEBT: observable behavior unchanged (DR-004): the stored annotated tag object has the same id, type, payload, blob key and index row as before.
- [ ] Docs: none to update.
- [ ] Self review: time and space complexity of the critical path stated (listing is O(n log n) for the sort plus n object lookups, now exactly n (was up to 3n); space O(n) for the output), no `unwrap` outside tests, no dead code, no new dependency.
- [ ] Closing story: every requirement of SPEC-0061 (DR-001 to DR-004) cited by a story with status done or landed; the measurement tables are in the commit body.
- [ ] One commit `US-0118: look up each listed tag once and close the spec`, body `Spec: SPEC-0061` and the requirement ids.
