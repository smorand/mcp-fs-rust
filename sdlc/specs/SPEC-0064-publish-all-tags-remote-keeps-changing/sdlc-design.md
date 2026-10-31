# SPEC-0064: Publish all tags, the remote keeps changing during the send, design

> Spec: sdlc/specs/SPEC-0064-publish-all-tags-remote-keeps-changing/sdlc-spec.md
> Nature: FEAT
> Depth: S
> Security: n/a
> Designed on: 2026-10-30
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language and test runner | Rust 2024, `cargo test` (the project's manifests, unchanged) | as in `Cargo.toml` | none |
| Libraries | none added | n/a | DEC-001 |

Skills loaded: `rust` (earlier in this session), effect: project gate is `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `make check`; tests live in the in-crate `mod tests`; no `unwrap` in production code.

Project state: existing (`git ls-files | wc -l` = 508 earlier in this session).

## 2. Approaches
### Chosen: tests plus a compiled out test seam, no production behavior change
The refusal and the loop already behave as the spec says (`crates/core/src/tools/git_tags.rs:1160-1222`). What is missing is a way to reproduce a remote that changes between the listing and the send, and the tests. A test seam in the push core lets a test run a closure just before and just after each sending attempt, and count sending attempts and listings. Pros: no production line changes; follows the existing `bulk_seams` pattern (`git/remote.rs:942-975`). Cons: the seam is test only code in a production file. Best suited when the behavior exists and only its verification is missing.
### Rejected: change the production loop to make it injectable
Would put test concerns in the production path and risk the behavior the spec pins (DEC-002).

## 3. Architecture
`send_due_tags` (`git_tags.rs:1160`) calls `push_tags_to_remote` (`git/remote.rs:837`) once per round and `reclassify_lost` (`git_tags.rs:1227`), which calls `list_advertised_refs` (`git/remote.rs:1057`). The seam sits in those two remote functions, behind `#[cfg(test)]`:
- before `push_refs` in `push_tags_to_remote`: count one sending attempt for the URL, then run the registered hook with `PushPhase::Before(attempt)`;
- after `push_refs` returns: run the hook with `PushPhase::After(attempt)`;
- at the top of `list_advertised_refs`: count one listing for the URL.
A test hook mutates the bare remote on disk (create or delete a tag ref), so the lease check (every tag is a creation leased on "absent", `remote.rs:826-836`) fails for real and the re-listing sees the real state.

## 4. Data model
None.

## 5. Contracts and interfaces
- Refusal text, code and set of names: unchanged (`git_tags.rs:1217-1222`). The refusal keeps the "no overwrite" error code (`ToolError::no_clobber`, `errors.rs:74`).
- Test seam (compiled out of production), in `crates/core/src/git/remote.rs` `bulk_seams`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PushPhase { Before(usize), After(usize) }   // usize = attempt index for this url, from 0
/// Run `hook` around every bulk tag push to `url` (replaces a previous hook for it).
pub(crate) fn around_push(url: &str, hook: impl FnMut(PushPhase) + Send + 'static)
/// How many bulk tag pushes and how many remote listings `url` has seen since `reset(url)`.
pub(crate) fn counts(url: &str) -> (usize, usize)   // (pushes, listings)
pub(crate) fn reset(url: &str)                       // clears hook and counters of `url`
```
State is keyed by URL in `static` `Mutex<Vec<..>>` like the existing seams, so parallel tests never see each other's.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/git/remote.rs` | modify: `bulk_seams` gains `PushPhase`, `around_push`, `counts`, `reset`; `push_tags_to_remote` and `list_advertised_refs` call them under `#[cfg(test)]` | FR-NEW-001, FR-NEW-002 (testability) | US-0127 |
| `crates/core/src/tools/git.rs` (tests module, starts at line 7665) | modify: 8 tests | FR-NEW-001, FR-NEW-002 | US-0127, US-0128 |
Affected existing tests: none (keep all). Affected documentation: `.agent_docs/git.md` gets two lines on the seam (US-0128).

## 7. Compatibility and risks
No consumer sees a change: production code paths are untouched (`#[cfg(test)]` only). Risk: a hook running inside `push_tags_to_remote` holds no lock the test needs; the hook is taken out of its slot while it runs and put back, so a hook that registers a hook cannot deadlock. The existing one shot seams (`reject`, `drop_after`) keep their behavior.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001, E2E-002, E2E-006 | functional e2e at the tool boundary | `call_publish_all_tags` / `confirm_all`, hooks on the bare remote | `crates/core/src/tools/git.rs` `mod tests` | US-0127 |
| E2E-003, E2E-004, E2E-005, E2E-007, E2E-008 | functional e2e at the tool boundary | same | same | US-0128 |
Test data: generated (bare remotes through `three_commit_remote`, tags through `tag`, remote tags through `plain_tag_bare_remote`). The behavior exists, so a RED run before production code does not apply: the red proof is a mutation of the production code (stated per story).

## 10. Decisions log
- **DEC-001:** no new dependency, no production line changed. **Obliged by:** SPEC-0064/FR-NEW-001, FR-NEW-002 (they pin existing behavior). **Rationale:** the spec documents what the code does. **Alternatives considered:** a mock transport crate (rejected: a category 2 dependency for a hook the existing seam pattern already covers). **Source:** design. **Evidence:** `git/remote.rs:942-975`.
- **DEC-002:** the seam is `#[cfg(test)]` only and lives in `bulk_seams` beside the existing seams. **Obliged by:** SPEC-0064/FR-NEW-001 (testability of the refusal). **Rationale:** the file remote cannot make a tag appear and vanish (BL-0093); closures run around the real push make it happen for real, so the lease check fires in libgit2 and the production code is exercised unchanged. **Alternatives considered:** a seam that forces `lease_lost` without touching the remote (rejected: it would bypass the code that builds the lost list and the re-listing against a real remote state); a hook only before the push (rejected: the flapping tag must be gone again before the re-listing, which needs the After phase). **Source:** design. **Evidence:** `git/remote.rs:826-836,942-975`, `git_tags.rs:1227-1261`.
- **DEC-003:** counters keyed by URL and a `reset(url)` the tests call first. **Obliged by:** SPEC-0064/FR-NEW-002 (E2E-006, E2E-007 count attempts and listings). **Rationale:** the URL of a bare remote is unique per test (`file://` of a temp dir), so keyed state is parallel safe like the existing seams. **Alternatives considered:** thread locals (rejected: the push runs on a blocking thread, `on_git_thread`, so a thread local would not be visible). **Source:** existing code. **Evidence:** `tools/git.rs` `on_git_thread` use in `git_tags.rs:72-76`.
- **DEC-004:** two stories: the seam with the three tests that cannot exist without it (E2E-001, 002, 006), then the five remaining tests and the close. **Obliged by:** SPEC-0064/FR-NEW-001, FR-NEW-002. **Alternatives considered:** one story of 8 tests and a seam (rejected: two files and a harness addition in one review); one story per test (rejected: tier 3 shape). **Source:** design.
- **DEC-005:** red proof by mutation, recorded in each story. **Obliged by:** SPEC-0064/FR-NEW-001, FR-NEW-002. **Rationale:** the behavior pre exists. **Source:** design.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0127 | reproduce a remote that keeps changing and test the refusal | P1 | FR-NEW-001, FR-NEW-002 (partial) | main | 2 | no |
| US-0128 | test which tags the refusal names, the retry and the order of refusals | P1 | FR-NEW-001, FR-NEW-002 | US-0127 | 2 | yes |
