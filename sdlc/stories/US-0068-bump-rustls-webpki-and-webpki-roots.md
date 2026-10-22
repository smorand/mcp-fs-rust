# US-0068: bump rustls-webpki and webpki-roots for two RustSec advisories

> Spec: SPEC-0048 (sdlc/specs/SPEC-0048-webpki-advisory-lockfile-bump/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 2
> Closes spec: yes

## Objective
`make security` fails on `main` because `Cargo.lock` pins `rustls-webpki` 0.103.15 (RUSTSEC-2026-0382) and `webpki-roots` 1.0.9 (RUSTSEC-2026-0381). Move exactly those two packages to their fixed patch releases, with a test that pins the fix, so `make check` is green again with no behavior change.

## Requirements covered
- SPEC-0048/DR-001: The lockfile SHALL pin `rustls-webpki` at 0.103.16 or a later 0.103 release.
- SPEC-0048/DR-002: The lockfile SHALL pin `webpki-roots` at 1.0.10 or a later 1.0 release.
- SPEC-0048/DR-003: The change SHALL NOT alter the version of any other package in the lockfile, nor any manifest, nor `deny.toml`.
- SPEC-0048/DR-004: The server and the agent SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

## Independent Test
Run `cargo test -p mcp-fs --test security_gate`: 14 tests pass (the 13 existing, 3 of which fail before this story, plus DT-001). Then `cargo audit` prints no vulnerability and exits 0, and `make check` is green.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0048): Run `cargo update -p rustls-webpki -p webpki-roots`, which locks exactly two packages (`rustls-webpki v0.103.15 -> v0.103.16`, `webpki-roots v1.0.9 -> v1.0.10`). Not a full `cargo update`, not a manifest pin, not a `[patch]`, not a `deny.toml` ignore.
- DEC-002 (SPEC-0048): DT-001 is a new test in `security_gate.rs`, reusing `lock_packages` (`crates/mcp-fs/tests/security_gate.rs:41`).
- DEC-003 (SPEC-0048): DT-002 and DT-003 are checks run in this story, not permanent tests.
- DEC-004 (SPEC-0048): one story carries the whole debt: DT-001 first (red), then the lockfile update.
### Data model
None.
### Contracts
None. `Cargo.lock` is internal.
### Paths
| Path | Action |
|---|---|
| `Cargo.lock` | modify by `cargo update` only, never by hand |
| `crates/mcp-fs/tests/security_gate.rs` | modify: add one test |
### Existing patterns
`lock_packages(name) -> Vec<String>` returns every locked version of a package (`crates/mcp-fs/tests/security_gate.rs:41-56`). Follow the style of the neighbouring lockfile test `lockfile_pins_git2_0_21_and_libgit2_sys_unchanged` (line 59).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0048/DT-001: the fixed releases are locked
- Level / Driver / Location: integration / `lock_packages` over `Cargo.lock` / `crates/mcp-fs/tests/security_gate.rs`
- Given the lockfile after the update
- When the test reads the versions of `rustls-webpki` and `webpki-roots`
- Then `lock_packages("rustls-webpki")` has exactly one entry and it is `0.103.16` or later in the 0.103 line; `lock_packages("webpki-roots")` has exactly one entry and it is `1.0.10` or later in the 1.0 line (compare `major.minor.patch` numerically)
- Red before the update because: `["0.103.15"]` and `["1.0.9"]` fail the comparison (assertion message names the crate and the found version).

### SPEC-0048/DT-002: only the two packages move
- Level / Driver: structural check, shell. Not a permanent test.
- Run: `pairs() { awk '/^name = /{n=$3} /^version = /{print n, $3}'; }; diff <(git show main:Cargo.lock | pairs | sort) <(pairs < Cargo.lock | sort)`
- Then the diff shows exactly 2 removed and 2 added lines: `"rustls-webpki" "0.103.15"` to `"0.103.16"` and `"webpki-roots" "1.0.9"` to `"1.0.10"`. Paste the output in the commit body.

### SPEC-0048/DT-003: no manifest or deny file change
- Level / Driver: structural check, shell. Not a permanent test.
- Run: `git diff --stat main -- '*.toml' deny.toml`
- Then it prints nothing.

### SPEC-0048/DT-020: the advisory scan is clean
- Level / Driver / Location: integration / `cargo audit --json` and `make security` through the existing tests, unmodified / `crates/mcp-fs/tests/security_gate.rs`
- Given the locked packages
- When `cargo_audit_reports_no_vulnerability_and_no_warning`, `make_security_passes` and `deny_toml_drops_the_fixed_git2_ignores` run
- Then all three pass: 0 vulnerabilities, warnings `{}`, exit 0, and `cargo deny check` passes
- Red before the update because: `assertion left == right failed: vulnerabilities: ... "count":2`.

## Constraints
- Files not to touch: every `Cargo.toml`, `deny.toml`, any source file, any existing test.
- Dependencies not to add: none; no other package may change version.
- Patterns to avoid: a full `cargo update`; editing `Cargo.lock` by hand; adding an advisory to the `deny.toml` ignore list.
- Scope boundary: the `rustls-webpki 0.101.7` remark in `deny.toml:11` concerns an old version already eliminated; leave it.

## Definition of done
- [ ] RED evidence recorded for DT-001 (and the three `security_gate` tests failing) before the update.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: the existing suite passes unmodified: `cargo test --workspace --no-fail-fast`. Exceptions: 0 modified tests; 3 tests turn green (`cargo_audit_reports_no_vulnerability_and_no_warning`, `make_security_passes`, `deny_toml_drops_the_fixed_git2_ignores`).
- [ ] Docs updated: none required (`rg -n 'rustls-webpki|webpki-roots' README.md AGENTS.md .agent_docs` finds nothing).
- [ ] Self review: DT-001 would fail on the old lock; the diff touches only the two files; no dash used as punctuation in new code or comments.
- [ ] Closes spec: the spec's full acceptance set green (DT-001, DT-002, DT-003, DT-020), and DR-001 to DR-004 each cited by this story.
- [ ] One commit `US-0068: bump rustls-webpki and webpki-roots for two RustSec advisories`, body `Spec: SPEC-0048` and the requirement ids.
