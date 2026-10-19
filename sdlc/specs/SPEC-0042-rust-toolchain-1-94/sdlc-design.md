# Move the Rust toolchain to 1.94.1 and take the fixed aws-sdk-s3: technical debt design

> Id: SPEC-0042
> Nature: DEBT
> Depth: S
> Depth evidence: one debt item (toolchain floor), 1 module family of manifests, 0 source changes, no public contract change, no persisted change
> Status: implemented
> Generated on: 2026-10-19
> Behavior change: none
> From backlog: BL-0060
> Depends on: SPEC-0035 (implemented, `.cargo/audit.toml`)
> Security: RUSTSEC-2026-0002 (lru 0.12.5 unsound IterMut, via aws-sdk-s3; currently accepted)
> CVSS: not scored
> Affected: every commit whose Cargo.lock carries aws-sdk-s3 1.119.0 / lru 0.12.5, including main
> Fixed in: unreleased

## 1. Debt
### DBT-001: The 1.88 floor blocks the fixed aws-sdk-s3
- **Current structure:** `rust-version = "1.88"` (`Cargo.toml:8`), `channel = "1.88"` (`rust-toolchain.toml:2`); `Cargo.lock` resolves aws-sdk-s3 1.119.0 (`Cargo.lock:599-600`), which pulls lru 0.12.5 (`cargo tree -i lru@0.12.5`, 2026-10-19). `.cargo/audit.toml:10-11` accepts RUSTSEC-2026-0253 and RUSTSEC-2026-0002 "until BL-0060".
- **Cost paid today:** one standing accepted unsound advisory (RUSTSEC-2026-0002) and an aws SDK 33 minor versions behind, held back only by the MSRV (`cargo info aws-sdk-s3@1.152.0`: rust-version 1.94.1).
- **Target structure:** floor and pinned channel `1.94.1`; `Cargo.lock` at aws-sdk-s3 1.152.0 (lru 0.18.5 for the SDK, not affected by either advisory). Verified on a throwaway worktree: with the two pins at 1.94.1, `cargo update -p aws-sdk-s3 --precise 1.152.0` locks lru 0.12.5 -> 0.18.5, `cargo tree -i lru@0.12.5` is empty, and with `.cargo/audit.toml` removed `cargo audit` reports only RUSTSEC-2023-0071, 0367, 0368, 0369, 2024-0384, 2024-0436, 2026-0192 and 2026-0253 (lru 0.16.4 via tantivy). RUSTSEC-2026-0002 is gone.
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State
| File | Change | Occurrences |
|---|---|---|
| `Cargo.toml` | `:8` `rust-version = "1.88"` becomes `"1.94.1"`; comment `:100` "1.88 MSRV" reworded to the new floor | 2 |
| `rust-toolchain.toml` | `:2` `channel = "1.88"` becomes `"1.94.1"` | 1 |
| `Cargo.lock` | `cargo update -p aws-sdk-s3 --precise 1.152.0` (and its transitive moves, lru 0.18.5) | regenerated |
| `.cargo/audit.toml` | remove `:11` RUSTSEC-2026-0002; `:10` reason becomes "lru unsound pop, via tantivy 0.26.2, no fix" (drop "and aws-sdk-s3 (until BL-0060)") | 2 |
| `crates/mcp-fs/tests/security_gate.rs` | `:160` drop `"RUSTSEC-2026-0002"` from the expected list | 1 |
| `crates/mcp-fs/tests/toolchain_pin.rs` | none: compares the two pins to each other (`:12-33`), stays green | 0 |

**Non source occurrences:** `rg -n "1\.88"` (2026-10-19, excluding `target`, `.git`, `Cargo.lock`) finds only the rows above plus history in `sdlc/specs/`, `sdlc/stories/US-0042-zip-9-upgrade.md:37` and `sdlc/backlog/BL-0060-*` (all left untouched). No Dockerfile, no `.github/workflows`, none in Makefile, README, AGENTS.md or `.agent_docs`. `sevenz-rust2` stays pinned `0.20` (`Cargo.toml:97-101`); lifting that pin is out of scope.
**Contracts touched:** `rust-version` is a build contract for contributors (internal; this is a personal repository with no downstream crate consumers). No HTTP, CLI, env or persisted contract.
**Guardrail coverage:** the full workspace suite plus `security_gate.rs` and `toolchain_pin.rs`. **Test command:** `cargo test --workspace` (run through `make check`, which adds format, clippy `--all-features -D warnings`, and `cargo audit`/`cargo deny`).
**Total:** 5 files edited by hand plus `Cargo.lock`, 6 occurrences, recounted.

## 3. Class sweep
Not applicable: a dependency advisory removed by upgrade, no `--security` flag, no own code vulnerable.

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: Toolchain floor
> The workspace SHALL declare `rust-version = "1.94.1"` and `rust-toolchain.toml` SHALL pin `channel = "1.94.1"`.
- **Old form:** `1.88` in both. **New form:** `1.94.1` in both. **Applies to:** `Cargo.toml:8`, `rust-toolchain.toml:2`. **Casing convention:** n/a (version literal).

#### DR-002 [EARS-U]: aws-sdk-s3 version
> `Cargo.lock` SHALL resolve `aws-sdk-s3` to 1.152.0 and SHALL NOT contain `lru` 0.12.5.
- **Old form:** aws-sdk-s3 1.119.0, lru 0.12.5. **New form:** 1.152.0, lru 0.18.5. **Applies to:** `Cargo.lock`. The `Cargo.toml:57` requirement `version = "1"` is unchanged.

#### DR-003 [EARS-U]: Advisory record
> `.cargo/audit.toml` SHALL NOT list RUSTSEC-2026-0002, and the reason of RUSTSEC-2026-0253 SHALL name only tantivy 0.26.2.
- **Old form:** the two lines of `.cargo/audit.toml:10-11`. **New form:** `"RUSTSEC-2026-0253", # lru unsound pop, via tantivy 0.26.2, no fix`; line 11 deleted.

### 4.2 Invariance (MANDATORY)
#### DR-004 [EARS-U]: Behavior invariance
> The server SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.

### 4.3 Compatibility
None: no public contract moves (Section 5).

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `rust-version` 1.88 -> 1.94.1 | build floor | no (internal, single owner) | break now | none | n/a |

**Dual maintenance semantics:** none. Contributors need `rustup toolchain install 1.94.1`; `rust-toolchain.toml` makes rustup select it.

## 6. Decisions
- **DEC-001:** pin exactly `1.94.1`, not `stable`. **Alternatives considered:** `1.94` (rejected: aws-sdk-s3 1.152.0 declares 1.94.1, so 1.94.0 fails); `stable` (rejected: `toolchain_pin.rs` requires channel equal to `rust-version`, and a moving channel breaks reproducible builds). **Rationale:** smallest floor that satisfies the SDK, same convention as today. **Implemented by:** DR-001. **Evidence:** `crates/mcp-fs/tests/toolchain_pin.rs:30-33`.
- **DEC-002:** lock aws-sdk-s3 with `--precise 1.152.0`, not a bare `cargo update`. **Alternatives considered:** full `cargo update` (rejected: unrelated version churn, wider risk, not this debt). **Rationale:** a bare `-p aws-sdk-s3` stayed on 1.119.0 in the experiment. **Implemented by:** DR-002. **Evidence:** n/a, experiment of 2026-10-19.
- **Assumption:** RUSTSEC-2026-0253 stays accepted because lru 0.16.4 via tantivy remains (`Cargo.lock:3574-3575` in the experiment); it is not ours to fix here.
- **Assumption:** aws-sdk-s3 1.152.0 keeps the S3 API surface used by `infra.blob` (`crates/core/src/storage/blob/s3.rs`); proven only by compile plus the MinIO suite, see DT-003.

## 7. Declared breaks
None. (Raising the MSRV is a contributor facing build change, recorded in Section 5, not an observable product break.)

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified** except the one justified row of 8.4:
    `make check`
and, because the S3 client version changes, the real object store suite described in `.agent_docs/testing.md`: `make test-e2e-full`.

### 8.2 Compatibility tests
None (no public contract).

### 8.3 Structural tests
#### DT-001: Toolchain floor
- **Asserts:** both pins read `1.94.1`. **Method:** extend `crates/mcp-fs/tests/toolchain_pin.rs` with `assert_eq!(channel, "1.94.1")` (the existing equality test is kept).
#### DT-002: lru 0.12.5 absent
- **Asserts:** `Cargo.lock` has no `name = "lru"` block with `version = "0.12.5"` and has aws-sdk-s3 `1.152.0`. **Method:** file text assertion in `crates/mcp-fs/tests/security_gate.rs`.
#### DT-003: S3 backend still works
- **Asserts:** blob put, get, delete against MinIO behave as before. **Method:** `make test-e2e-full` (existing tests, unmodified).

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/mcp-fs/tests/security_gate.rs:160` | audit ignore list test | remove `"RUSTSEC-2026-0002"` from `expected` | DR-003 removes the id; the test pins the list exactly, so it must follow. It tests the record, not product behavior. |

## 9. Implementation order
1. Install 1.94.1 (`rustup toolchain install 1.94.1`), edit both pins and the `Cargo.toml:100` comment (DR-001), add DT-001. Green: `cargo build`, `toolchain_pin`.
2. `cargo update -p aws-sdk-s3 --precise 1.152.0` (DR-002), add DT-002. Green: `cargo test --workspace`, clippy `--all-features`.
3. Edit `.cargo/audit.toml` and the `security_gate.rs` expected list together (DR-003); they must land in the same step or `security_gate` fails. Green: `make security`.
4. Run `make check` and `make test-e2e-full` (DT-003).

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE (inline checklist, all citations opened, counts recounted 2026-10-19) |

**Amendments applied:** none.

## Design additions (2026-10-19)
### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `Cargo.toml` | modify `:8`, `:100` | DR-001 | US-0050 |
| `rust-toolchain.toml` | modify `:2` | DR-001 | US-0050 |
| `Cargo.lock` | regenerate | DR-002 | US-0050 |
| `.cargo/audit.toml` | modify `:10-11` | DR-003 | US-0050 |
| `crates/mcp-fs/tests/security_gate.rs` | modify `:160`, add DT-002 | DR-002, DR-003 | US-0050 |
| `crates/mcp-fs/tests/toolchain_pin.rs` | add DT-001 | DR-001 | US-0050 |
| `.agent_docs/testing.md` or `AGENTS.md` | note the 1.94.1 floor only if either names the old one (`rg 1\.88` found neither) | n/a | US-0050 |

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001 | integration | direct file read | `crates/mcp-fs/tests/toolchain_pin.rs` | US-0050 |
| DT-002 | integration | direct file read | `crates/mcp-fs/tests/security_gate.rs` | US-0050 |
| DT-003 | functional e2e | existing suite | `make test-e2e-full` | US-0050 |

- **DEC-003:** one story for the whole spec. **Obliged by:** DR-001..DR-004. **Rationale:** steps 1 to 3 of Section 9 cannot land separately on a green main (the audit list test and the lock must move together; the pin test DT-001 fails between step 1 and the new floor otherwise); 5 files fits tier 2. **Alternatives considered:** four stories along Section 9 (rejected: each intermediate state fails `make check`). **Source:** design. **Evidence:** `crates/mcp-fs/tests/security_gate.rs:150-168`.
