# Replace the deprecated serde_yaml with serde_norway: technical debt design

> Id: SPEC-0040
> Nature: DEBT
> Depth: S
> Depth evidence: one debt item (YAML config parsing), 2 crates, 2 production call sites, no public contract change, no persisted change
> Status: implemented
> Generated on: 2026-10-19
> Behavior change: none
> From backlog: BL-0061
> Depends on: SPEC-0035 (implemented, `.cargo/audit.toml`)
> Security: RUSTSEC-2024-0449 (unsafe-libyaml unmaintained, via serde_yaml; currently accepted)
> CVSS: not scored
> Affected: every commit whose Cargo.lock carries serde_yaml 0.9 / unsafe-libyaml, including main at bc2c572
> Fixed in: unreleased

## 1. Debt
### DBT-001: Config is parsed with a deprecated crate whose parser is unmaintained
- **Current structure:** `serde_yaml = "0.9"` (`Cargo.toml:25`), used by `crates/core/Cargo.toml:39` and `crates/agent/Cargo.toml:18`; production parses at `crates/core/src/config.rs:917` (server config) and `crates/agent/src/config.rs:91` (agent config). It pulls `unsafe-libyaml` (RUSTSEC-2024-0449, unmaintained), accepted at `.cargo/audit.toml:12` "until BL-0061" and pinned in `crates/mcp-fs/tests/security_gate.rs:160`.
- **Cost paid today:** one standing accepted advisory, and a deprecated crate in the config path.
- **Target structure:** `serde_norway` 0.9.42, a drop in fork with the same API, using `unsafe-libyaml-norway` (no RustSec advisory for either, checked 2026-10-19); the advisory id leaves the accepted list.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `Cargo.toml` | `:25` `serde_yaml = "0.9"` becomes `serde_norway = "0.9"` | 1 |
| `crates/core/Cargo.toml` | `:39` dependency renamed | 1 |
| `crates/agent/Cargo.toml` | `:18` dependency renamed | 1 |
| `Cargo.lock` | `serde_yaml` and `unsafe-libyaml` replaced by `serde_norway`, `unsafe-libyaml-norway` | regenerated |
| `crates/core/src/config.rs` | production `:917`, comment `:906`, tests `:1217`, `:1234`, `:1244`, `:1251`, `:1253`, `:1255`, `:1264`, `:1274`, `:1277`, `:1285`, `:1288`, `:1296`, `:1300`, `:1418`, `:1587` | 17 (1 code, 1 comment, 15 test) |
| `crates/agent/src/config.rs` | `:91` | 1 |
| `crates/core/src/cli.rs` | comment `:779` | 1 |
| `crates/core/src/git/remote.rs` | comment `:47` | 1 |
| `.cargo/audit.toml` | remove the `:12` line RUSTSEC-2024-0449 | 1 |
| `crates/mcp-fs/tests/security_gate.rs` | `:160` drop the id from the expected list; new DT-001, DT-002 | 1 + new |
| `.agent_docs/config.md` | `:375` | 1 |
| `.agent_docs/git.md` | `:169` | 1 |

**Non source occurrences:** `.agent_docs/config.md:375`, `.agent_docs/git.md:169` (above); none in Makefile, scripts, CI, `deny.toml` (`rg -n serde_yaml`, 2026-10-19).
**Contracts touched:** none public; the YAML config file format read by the server and the agent is unchanged (DR-003).
**Guardrail coverage:** the config tests in `crates/core/src/config.rs` (module from `:1166`), agent config tests in `crates/agent`. **Test command:** `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
**Total:** 12 files plus the lockfile, 28 occurrences, recounted with `rg -c serde_yaml`.

## 3. Class sweep
**Class:** an unmaintained memory-unsafe parser reaching untrusted or operator supplied YAML (CWE-1104, use of unmaintained third party component).
**Searches run:** `rg -n serde_yaml --type rust --type toml` (every Rust and manifest use); `cargo tree -i unsafe-libyaml --all-features` (every path to the unsafe parser, transitive included).
**Not searched:** other YAML parsers, none in the manifest.

| Occurrence | Verdict | Disposition |
|---|---|---|
| `crates/core/src/config.rs:917` server config | operator supplied file, parsed by the unmaintained parser | DR-001 |
| `crates/agent/src/config.rs:91` agent config | operator supplied file | DR-001 |
| test only parses in `config.rs` (15 lines) | test literals | DR-001 (path rename, 8.4) |

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: serde_norway everywhere
> The workspace SHALL parse and emit YAML through `serde_norway` 0.9, and SHALL NOT name `serde_yaml` in `Cargo.toml`, `crates/` or `.agent_docs/`.
- **Old form:** `serde_yaml` (crate name in manifests; path prefix `serde_yaml::` in code)
- **New form:** `serde_norway` (same items: `from_str`, `to_value`, `from_value`, `Value`)
- **Casing convention:** Rust snake_case

#### DR-002 [EARS-UB]: Advisory no longer accepted
> `Cargo.lock` SHALL NOT contain `serde_yaml` nor `unsafe-libyaml`, `.cargo/audit.toml` SHALL NOT list RUSTSEC-2024-0449, and `cargo audit` SHALL report no vulnerability and no warning.

### 4.2 Invariance
#### DR-003 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change; every tracked config file (`config/agent_test.yaml`, `config/local.yaml.template`, `config/minio.yaml.template`) SHALL deserialize to an equal value, the two templates read after the same `${VAR}` expansion the server applies; `tool-contract-golden.json` SHALL be unchanged.

### 4.3 Compatibility
None (no public contract).

## 5. Compatibility plan
None.

## 6. Decisions
- **DEC-001:** `serde_norway` 0.9.42. Owner decision 2026-10-19. **Alternatives considered:** `serde_yaml_ng` 0.10 (rejected: still depends on `unsafe-libyaml`, the advisory stays); `serde_yml` (rejected: RUSTSEC-2025-0068, unsound, no fix); `serde-saphyr` 2.0.0 (rejected for now: pure Rust and active, but a different API and YAML 1.2 semantics that can parse existing configs differently, a behavior change). **Implemented by:** DR-001, DR-002. **Evidence:** crates.io dependency lists, 2026-10-19.
- **DEC-002:** Rename the crate everywhere instead of aliasing `serde_yaml = { package = "serde_norway" }`. Owner decision 2026-10-19. **Alternatives considered:** the alias (rejected: zero test edits but a misleading name left in the code). **Implemented by:** DR-001. **Evidence:** n/a.

## 7. Declared breaks
None.

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the 8.4 rows:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
#### DT-003: config parity
- **Validates:** DR-003
- **Location:** `crates/core/src/config.rs` tests (server) and `crates/agent` tests (agent)
- **Given** `config/local.yaml.template`, `config/minio.yaml.template` (server, after `${VAR}` expansion with fixed test values) and `config/agent_test.yaml` (agent) / **When** each is parsed into its config type / **Then** parsing succeeds, and the `Debug` rendering of the parsed value equals a snapshot recorded on the current code before the swap (stored as a test literal).
- **Red before:** guardrail; recorded on serde_yaml, then must stay equal on serde_norway.

### 8.3 Structural tests
#### DT-001: no serde_yaml, no unsafe-libyaml
- **Validates:** DR-001, DR-002
- **Location:** `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** Cargo.lock has no `name = "serde_yaml"` and no `name = "unsafe-libyaml"`; `cargo tree -i unsafe-libyaml --all-features` exits non zero with `did not match any packages`; `Cargo.toml`, `crates/**/*.rs`, `crates/**/Cargo.toml`, `.agent_docs/*.md` contain no `serde_yaml`.
- **Red before because:** Cargo.lock lists `serde_yaml` 0.9 and `unsafe-libyaml`.

#### DT-002: advisory leaves the accepted list
- **Validates:** DR-002
- **Location:** `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** `.cargo/audit.toml` does not contain `RUSTSEC-2024-0449`; the existing `cargo_audit_reports_no_vulnerability_and_no_warning` and `make_security_passes` stay green.
- **Red before because:** `.cargo/audit.toml:12` lists it.

#### DT-004: guardrail
- 8.1 green; `git diff main -- tool-contract-golden.json` empty.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/core/src/config.rs` | the 15 test lines `:1217` to `:1587` listed in Section 2 | `serde_yaml::` path becomes `serde_norway::`; asserted values unchanged | the crate is renamed (DEC-002) |
| `crates/mcp-fs/tests/security_gate.rs` | `accepted_advisories_are_recorded_once_with_a_reason` (`:160`) | drop `RUSTSEC-2024-0449` from the expected id list | the advisory is no longer accepted (DR-002) |

## 9. Implementation order
1. Write DT-003 snapshots on serde_yaml (green), DT-001 and DT-002 (RED).
2. Rename in manifests, `cargo update`, rename every occurrence, drop the audit id and the expected list entry, update the two docs.
3. 8.1 green. One story; nothing compiles between the manifest rename and the code rename.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 1 | IMPLEMENTABLE after amending A-1 (`config/local.yaml` is untracked, `git ls-files config/`: parity now covers the tracked `agent_test.yaml`, `local.yaml.template`, `minio.yaml.template`); inline, depth S: both sides of the rename spelled; perimeter recounted (`rg -c serde_yaml`: config.rs 17, agent config 1, cli 1, remote 1, manifests 3, docs 2, audit 1, security gate 1); 2 test exceptions declared with reasons; non regression command literal; sweep with two searches, every hit disposed; security field set (advisory driven upgrade); no open question |

**Amendments applied:** none.

## Design additions (2026-10-19)

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001, DT-002 | structural | lockfile text, `cargo tree`, file text | `crates/mcp-fs/tests/security_gate.rs` | US-0047 |
| DT-003 | unit | parse the tracked configs, compare `Debug` snapshots | `crates/core/src/config.rs` tests, `crates/agent/src/config.rs` tests | US-0047 |
| DT-004 | guardrail | full suite | `make check` | US-0047 |

### Additional decisions
- **DEC-003:** One story at `min_tier: 2`. **Obliged by:** SPEC-0040/DR-001..DR-003. **Rationale:** 12 files but a mechanical rename (decision density low); nothing compiles between the manifest rename and the code rename, so it cannot be split. **Alternatives considered:** two stories (rejected: the first would not build). **Source:** design. **Evidence:** Section 2.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0047 | replace serde_yaml with serde_norway | P1 | DR-001, DR-002, DR-003 | main | 2 | yes |
