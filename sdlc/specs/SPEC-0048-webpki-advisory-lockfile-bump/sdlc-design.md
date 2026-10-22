# Bump rustls-webpki and webpki-roots for two RustSec advisories: technical debt design

> Id: SPEC-0048
> Nature: DEBT
> Depth: S
> Depth evidence: 1 module (the lockfile), 2 occurrences, no contract change, no persisted data
> Status: implemented
> Generated on: 2026-10-22
> Behavior change: none
> From backlog: BL-0071
> Depends on: none
> Security: RUSTSEC-2026-0382, RUSTSEC-2026-0381
> CVSS: not scored
> Affected: Cargo.lock of main at commit 1abe7f6 (rustls-webpki 0.103.15, webpki-roots 1.0.9)
> Fixed in: unreleased

## 1. Debt

### DBT-001: two vulnerable transitive crates fail the security gate
- **Current structure:** `Cargo.lock:4939-4940` pins `rustls-webpki` 0.103.15 (RUSTSEC-2026-0382, "Certificate verification may accept distrusted intermediates", fixed in `>=0.103.16, <0.104.0-alpha.1`). `Cargo.lock:6686-6687` pins `webpki-roots` 1.0.9 (RUSTSEC-2026-0381, "Bundled root certificates allow trust in distrusted emSign intermediates", fixed in `>=1.0.10`). Neither crate appears in any `Cargo.toml`: `rustls-webpki` comes through `rustls` 0.23.46, `webpki-roots` through `hyper-rustls` 0.27.10 and `reqwest` 0.12.28 (`cargo tree -i`).
- **Cost paid today:** `make security` fails (`cargo audit`: "2 vulnerabilities found"), so `make check` is red on `main`, and three existing tests fail in `crates/mcp-fs/tests/security_gate.rs`: `cargo_audit_reports_no_vulnerability_and_no_warning`, `make_security_passes`, `deny_toml_drops_the_fixed_git2_ignores`. Stories landed under a "no worse than main" gate since 2026-10-22.
- **Target structure:** the same dependency graph with the two packages at their fixed releases.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `Cargo.lock` | `rustls-webpki` 0.103.15 to 0.103.16 (version and checksum lines); `webpki-roots` 1.0.9 to 1.0.10 (version and checksum lines) | 2 packages |
| `crates/mcp-fs/tests/security_gate.rs` | add one test (DT-001); no existing test changes | 1 addition |

**Non source occurrences:** `rg -n 'rustls-webpki|webpki-roots' Cargo.toml crates/*/Cargo.toml deny.toml` finds only the comment at `deny.toml:11` about `rustls-webpki 0.101.7`, a different, eliminated version; nothing in CI, Makefile, container files, scripts or docs pins either crate. `Cargo.lock` holds exactly one `rustls-webpki` entry (`rg -c '^name = "rustls-webpki"' Cargo.lock` gives 1).
**Contracts touched:** `Cargo.lock` is internal (build input of this repository only).
**Guardrail coverage:** 13 tests in `crates/mcp-fs/tests/security_gate.rs` (10 pass, 3 fail today), the whole suite 2843 tests in `mcp-fs-core` and 113 in `agent`. **Test command:** `cargo test --workspace --no-fail-fast` (the Makefile `test` target is `cargo test --workspace`, which stops at the first failing crate).
**Total:** 2 files, 2 package occurrences plus 1 test addition, all recounted.

## 3. Class sweep

Class: a vulnerable release of a third party crate reaching the build through the lockfile (CWE-1395, dependency on vulnerable third party component). The flaw lives in the crates, not in project code, so the occurrences are lockfile entries.
Searches run: (1) `cargo audit` reports 2 vulnerabilities, ids RUSTSEC-2026-0382 and RUSTSEC-2026-0381, no other (`cargo audit 2>&1 | rg -c '^ID:'` gives 2); idiom: advisory database match. (2) `rg -A1 '^name = "(rustls-webpki|webpki-roots)"' Cargo.lock` lists one version each, 0.103.15 and 1.0.9; idiom: lockfile text. Not searched: project source (no code calls these crates directly: `rg -n 'webpki' crates --glob '*.rs'` is not needed for a transitive pin).

| Occurrence | Verdict | Disposition |
|---|---|---|
| `Cargo.lock:4939` rustls-webpki 0.103.15 | vulnerable | DR-001 |
| `Cargo.lock:6686` webpki-roots 1.0.9 | vulnerable | DR-002 |

## 4. Requirements (EARS)

### 4.1 Structural

#### DR-001 [EARS-U]: rustls-webpki at a fixed release
> The lockfile SHALL pin `rustls-webpki` at 0.103.16 or a later 0.103 release.
- **Old form:** `name = "rustls-webpki"`, `version = "0.103.15"`
- **New form:** `version = "0.103.16"` with the checksum of that release
- **Applies to:** `Cargo.lock`
- **Casing convention:** crate names lowercase with hyphens, as in the lockfile.

#### DR-002 [EARS-U]: webpki-roots at a fixed release
> The lockfile SHALL pin `webpki-roots` at 1.0.10 or a later 1.0 release.
- **Old form:** `version = "1.0.9"`
- **New form:** `version = "1.0.10"` with the checksum of that release
- **Applies to:** `Cargo.lock`
- **Casing convention:** as DR-001.

#### DR-003 [EARS-UB]: nothing else moves
> The change SHALL NOT alter the version of any other package in the lockfile, nor any manifest, nor `deny.toml`.
- **Applies to:** `Cargo.lock`, every `Cargo.toml`, `deny.toml`.

### 4.2 Invariance

#### DR-004 [EARS-U]: Behavior invariance
> The server and the agent SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility

None: no public contract is touched.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `Cargo.lock` package versions | internal | no | replace in place | none | none |

**Dual maintenance semantics:** not applicable.

## 6. Decisions

- **DEC-001:** Run `cargo update -p rustls-webpki -p webpki-roots`, which locks exactly two packages (dry run of 2026-10-22: "Locking 2 packages", `rustls-webpki v0.103.15 -> v0.103.16`, `webpki-roots v1.0.9 -> v1.0.10`). **Alternatives considered:** a full `cargo update` (moves every package that has a newer release, against DR-003); pinning the crates in a manifest (they are transitive, which adds a dependency edge for no reason); a `[patch]` section (same objection, plus it outlives the need); ignoring the advisories in `deny.toml` (the fix exists, an ignore hides a live flaw). **Rationale:** the smallest lockfile change that closes both advisories. **Implemented by:** DR-001, DR-002, DR-003. **Evidence:** `Cargo.lock:4939`, `Cargo.lock:6686`.
- **DEC-002:** DT-001 is a new test in `security_gate.rs`, reusing the file's `lock_packages` helper (`crates/mcp-fs/tests/security_gate.rs:41-56`). **Alternatives considered:** relying on `cargo audit` alone (it passes for any fixed version, but would also pass once the advisory database drops the entry, so it does not pin the fix). **Rationale:** the lockfile is the artifact the requirement names. **Implemented by:** DR-001, DR-002. **Evidence:** `crates/mcp-fs/tests/security_gate.rs:41`.
- **DEC-003:** DT-002 and DT-003 are checks run in the story, not permanent tests (they compare the lockfile and manifests with `main`, which a test in the tree cannot see after the merge). **Alternatives considered:** a committed expected list of packages (goes stale at the next dependency update). **Implemented by:** DR-003. **Evidence:** n/a
- **Assumption:** the fixed releases build with the pinned toolchain (`rust-toolchain.toml`); the dry run resolved them "to highest Rust 1.94.1 compatible versions".

## 7. Declared breaks

None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    cargo test --workspace --no-fail-fast

Expected: `mcp-fs-core` lib 2843+ passed, `agent` 113 passed, `security_gate` 13 passed (3 of them fail before the change: `cargo_audit_reports_no_vulnerability_and_no_warning`, `make_security_passes`, `deny_toml_drops_the_fixed_git2_ignores`, tolerated exceptions listed and counted: 0 modified tests, 3 tests turning green). Then `make check` is green.

### 8.2 Compatibility tests
None (no public contract).

### 8.3 Structural tests

#### DT-001: the fixed releases are locked
- **Validates:** DR-001, DR-002
- **Asserts:** `lock_packages("rustls-webpki")` is exactly one version and it is `0.103.16` or later in the 0.103 line; `lock_packages("webpki-roots")` is exactly one version and it is `1.0.10` or later in the 1.0 line. Red before the change: `["0.103.15"]` and `["1.0.9"]` fail the comparison.
- **Method:** parse the lockfile text with the file's helper, compare numeric components of `major.minor.patch`.

#### DT-002: only the two packages move
- **Validates:** DR-003
- **Asserts:** the sorted list of `name version` pairs of `Cargo.lock` after the change differs from `main`'s by exactly two pairs removed and two added, for `rustls-webpki` and `webpki-roots`.
- **Method:** `diff <(git show main:Cargo.lock | <pairs>) <(<pairs> < Cargo.lock)` where `<pairs>` is `awk '/^name = /{n=$3} /^version = /{print n, $3}'`; run in the story, output shown.

#### DT-003: no manifest or deny file change
- **Validates:** DR-003
- **Asserts:** `git diff --stat main -- '*.toml' deny.toml` lists nothing.
- **Method:** command in the story, output shown.

### 8.4 Existing tests to modify
None. Empty.

### 8.5 Security tests

#### DT-020: the advisory scan is clean
- **Covers:** DR-001, DR-002, sweep occurrences `Cargo.lock:4939` and `Cargo.lock:6686`
- **Driver:** direct call (`cargo audit --json`), through the existing `cargo_audit_reports_no_vulnerability_and_no_warning` and `make_security_passes` tests, unmodified.
- **Red before the fix because:** `assertion left == right failed: vulnerabilities: ... "count":2` (the audit lists RUSTSEC-2026-0382 and RUSTSEC-2026-0381).
- Given the locked packages / When `cargo audit --json` runs / Then it reports 0 vulnerabilities and `{}` warnings and exits 0 / And `cargo deny check` passes (read through `make security`, a different channel than the JSON report).

## 9. Implementation order

1. Add DT-001 to `security_gate.rs` and run it: red (`["0.103.15"]`).
2. Run `cargo update -p rustls-webpki -p webpki-roots`; the build stays green between steps because both fixes are patch releases of the same lines.
3. Run DT-001 (green), DT-002, DT-003, then `cargo test --workspace --no-fail-fast` and `make check` (green).

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 (inline checklist; the auditor subagent is unavailable while the provider weekly limit lasts) | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none.

## Design additions (2026-10-22)

Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`). Skill loaded: `rust` (`make check` is the gate, tests are real, no unit test stands in for the audit).

### File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `Cargo.lock` | modify (2 packages) | DR-001, DR-002, DR-003 | US-0068 |
| `crates/mcp-fs/tests/security_gate.rs` | modify (add DT-001; no existing test changes) | DR-001, DR-002 | US-0068 |

Affected existing tests: keep all 13 of `security_gate.rs`; three of them turn green. Affected documentation: none (`rg -n 'rustls-webpki|webpki-roots' README.md AGENTS.md .agent_docs` finds no mention to update).

### Test strategy

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001 | integration | lockfile text parsed by `lock_packages` | `crates/mcp-fs/tests/security_gate.rs` | US-0068 |
| DT-002 | structural check | shell diff of lockfile package pairs against `main` | story procedure | US-0068 |
| DT-003 | structural check | `git diff --stat main` | story procedure | US-0068 |
| DT-020 | integration | `cargo audit --json` and `make security` through the existing tests, unmodified | `crates/mcp-fs/tests/security_gate.rs` | US-0068 |

Test data: none to generate; the lockfile is the fixture.

- **DEC-004:** One story carries the whole debt: DT-001 first (red), then the lockfile update. **Obliged by:** DR-001, DR-002, DR-003. **Rationale:** the implementation order of section 9 has two steps that cannot be separated into landable halves, since a red test alone breaks `make check` on `main` and the lockfile alone leaves the pin unasserted. **Alternatives considered:** two stories (test, then bump), rejected because the first would land a red test. **Source:** design. **Evidence:** section 9 above.
