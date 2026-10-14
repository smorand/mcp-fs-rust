# Bump rustls past RUSTSEC-2026-0346..0349 so the security gate is green: technical debt design

> Id: SPEC-0029
> Nature: DEBT
> Depth: S
> Depth evidence: one debt item, one file (`Cargo.lock`, 1 entry), no public contract change, no persisted data change
> Status: specified
> Generated on: 2026-10-14
> Behavior change: none
> From backlog: BL-0012
> Depends on: none
> Security: RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348, RUSTSEC-2026-0349 (no CVE id verified)
> CVSS: not scored
> Affected: every commit whose `Cargo.lock` pins rustls 0.23.45, including main at 764e84f
> Fixed in: unreleased

## 1. Debt

### DBT-001: Vulnerable rustls version pinned in the lockfile
- **Current structure:** `Cargo.lock:4837-4838` pins `rustls` `0.23.45`, the only rustls entry in the lockfile. It is transitive: aws-sdk-s3 through `aws-smithy-http-client` (`cargo tree -i rustls@0.23.45`), and reqwest with `rustls-tls` (`Cargo.toml:109`, `crates/agent/Cargo.toml:15`).
- **Cost paid today:** `make security` (`Makefile:46-48`) fails: `cargo audit --ignore RUSTSEC-2023-0071` prints `error: 4 vulnerabilities found!`, and `cargo deny check` prints `advisories FAILED, bans ok, licenses ok, sources ok`. So `make check` (`Makefile:51`) stops before the tests, and `crates/mcp-fs/tests/security_gate.rs::make_security_passes` is red (`test result: FAILED. 1 passed; 1 failed`). The four advisories all say `Solution: Upgrade to >=0.23.46`.
- **Target structure:** `Cargo.lock` pins `rustls` `0.23.46`. Nothing else in the lockfile moves (`cargo update -p rustls --dry-run`: `Updating rustls v0.23.45 -> v0.23.46`, a single package).
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State

| File | Change | Occurrences |
|---|---|---|
| `Cargo.lock` | `version = "0.23.45"` becomes `version = "0.23.46"` under `name = "rustls"`, checksum updated by cargo | 1 package entry |

**Non source occurrences:** none. `Cargo.toml` names no rustls version (only features: `Cargo.toml:40`, `:49`, `:109`). `deny.toml:17-21` and `Makefile:47` ignore no rustls advisory, so neither changes.
**Contracts touched:** none public. The TLS client behavior is the dependency's own; this is a patch release of the same minor.
**Guardrail coverage:** 2 tests in `crates/mcp-fs/tests/security_gate.rs`, plus the full suite. **Test command:** see 8.1.
**Total:** 1 file, 1 occurrence, recounted with `rg -n -A1 '^name = "rustls"$' Cargo.lock`.

## 3. Class sweep

**Class:** a third party component version flagged by a published security advisory reaches the built binaries (CWE-1395, dependency on a vulnerable third party component).
**Searches run:**
1. `cargo audit --ignore RUSTSEC-2023-0071`: the RustSec database checked against `Cargo.lock`, every package of every target.
2. `cargo deny check` (`deny.toml:1-2`, `all-features = true`): the same database through the feature graph, with this repository's ignore list.
Idioms not searched: vendored or copied third party source. `git ls-files` holds no vendor directory, and both tools read only the lockfile graph.

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | rustls 0.23.45, RUSTSEC-2026-0346/0347/0348/0349 (`Cargo.lock:4837`) | vulnerable, fails the gate | DR-001, this lot (the triggering occurrence) |
| 2 | git2 0.20.4, RUSTSEC-2026-0183/0184 (unsound) | accepted risk, ignored with a reason at `deny.toml:19-20`, no fixed release | BL-0053 |
| 3 | rsa, RUSTSEC-2023-0071 | accepted risk, ignored at `deny.toml:18` and `Makefile:47`, no fixed release | BL-0053 |
| 4 | lru 0.12.5 and 0.16.4, RUSTSEC-2026-0253, RUSTSEC-2026-0002 (unsound) | warning only, does not fail the gate | BL-0053 |
| 5 | instant, paste, ttf-parser, unsafe-libyaml, yoke-derive (unmaintained) | warning only, does not fail the gate | BL-0053 |

Deferral of rows 2 to 5 authorized by the owner on 2026-10-14 (answer "ok" to the batched deferral question).

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-U]: rustls at a fixed version
> The workspace lockfile SHALL pin exactly one `rustls` package, at version `0.23.46` or later within `0.23`.
- **Old form:** `name = "rustls"` / `version = "0.23.45"`
- **New form:** `name = "rustls"` / `version = "0.23.46"`
- **Applies to:** `Cargo.lock` only
- **Casing convention:** the lockfile's own (lowercase crate name)

#### DR-003 [EARS-UB]: No masking
> The change SHALL NOT edit `Cargo.toml`, any `crates/*/Cargo.toml`, `deny.toml` or `Makefile`, and SHALL NOT add an advisory ignore.

### 4.2 Invariance
#### DR-002 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract is touched.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| none | | | | | |

**Dual maintenance semantics:** not applicable.

## 6. Decisions
- **DEC-001:** Lockfile only patch bump (`cargo update -p rustls`). **Alternatives considered:** a `Cargo.toml` floor such as `rustls = ">=0.23.46"` as a direct dependency, rejected because rustls is transitive and a new direct dependency is speculative; `cargo update` of the whole tree, rejected because it moves about 100 unrelated packages (`note: ... 100 unchanged dependencies behind latest`); ignoring the four advisories, rejected because a fix exists. **Rationale:** smallest change that clears the gate. **Implemented by:** DR-001, DR-003. **Evidence:** `cargo update -p rustls --dry-run` output in Section 1.
- **DEC-002:** The ten warning level advisories are out of this lot (BL-0053). **Alternatives considered:** clearing them here, rejected because none fails the gate and several have no fixed release (`deny.toml:18-20`). **Rationale:** YAGNI, owner decision 2026-10-14. **Implemented by:** n/a, no code impact. **Evidence:** `cargo audit` `warning: 10 allowed warnings found`.
- **Assumption:** the advisory database stays as fetched on 2026-10-14. `make_security_passes` reads it live, so a new advisory published before implementation can turn it red again for an unrelated crate; that would be a new backlog entry, not a scope change of this lot.

## 7. Declared breaks
None.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
None: no public contract.

### 8.3 Structural tests
#### DT-010: Manifests untouched
- **Asserts:** `git diff --name-only main...HEAD` lists `Cargo.lock` and nothing else outside `sdlc/`; `git diff main...HEAD -- deny.toml Makefile Cargo.toml crates/*/Cargo.toml` is empty.
- **Method:** grep assertion on the diff (DR-003).

### 8.4 Existing tests to modify

| Test file | Test | Change | Justification |
|---|---|---|---|

Empty.

### 8.5 Security tests
#### DT-020: The security gate passes
- **Covers:** DR-001, sweep occurrence 1
- **Driver:** existing test `crates/mcp-fs/tests/security_gate.rs::make_security_passes` (runs `make security`), via `cargo test -p mcp-fs --test security_gate`
- **Red before the fix because:** `make security failed: ... error: 4 vulnerabilities found!` (observed 2026-10-14, `test result: FAILED. 1 passed; 1 failed`)
- Given the lockfile at main / When `make security` runs with the advisory ids RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348, RUSTSEC-2026-0349 in the database / Then it exits 0 and `cargo audit` reports no vulnerability / And DT-021 holds.

#### DT-021: The vulnerable version is gone from the graph
- **Covers:** DR-001, sweep occurrence 1, read through the lockfile rather than the audit
- **Driver:** CLI
- **Red before the fix because:** `cargo tree -i rustls@0.23.45 --all-features` prints the dependency tree (exit 0) on main
- Given the changed lockfile / When `cargo tree -i rustls@0.23.45 --all-features` runs / Then it fails with `did not match any packages` / And `rg -n -A1 '^name = "rustls"$' Cargo.lock` prints exactly one entry, `version = "0.23.46"`.

## 9. Implementation order
1. `cargo update -p rustls` (only `Cargo.lock` changes; build stays green).
2. Run DT-021 and DT-010, then DT-020, then 8.1 in full.

## 10. Implementability gate

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

Inline checklist (depth S): 1 PASS, 2 PASS (both sides of the version spelled), 3 n/a (DEBT), 4 PASS, 5 n/a (DEBT), 6 PASS, 7 PASS, 8 PASS (no behavior change), 9 PASS (counts recounted), 10 PASS, 11 PASS, 12 PASS (Security field set), 13 PASS (two searches, every hit disposed, the triggering occurrence in the lot, exploit tests with literal advisory ids and a second channel), 14 PASS.

**Amendments applied:** none.
