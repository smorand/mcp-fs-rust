# Datafusion and tantivy upgrade to remove zstd 0.13: technical debt design

> Id: SPEC-0055
> Nature: DEBT
> Depth: M
> Depth evidence: 11 files (4 manifest lines, 1 code line, 2 gate configs, 1 test file modified, 3 new test locations, 1 new doc), 12 requirements, no public contract change
> Status: implemented
> Generated on: 2026-10-27
> Behavior change: none
> From backlog: n/a (free text input; the deferred follow up is BL-0089)
> Depends on: none
> Security: internal finding
> CVSS: not scored
> Affected: zstd 0.13.3, zstd-safe 7.2.1, zstd-sys 2.0.13 as locked on main at 7a142ec (RUSTSEC-2026-0391, 0395, 0397, 0399, 0400, 0401, 0402, 0403, 0404, 0406, 0410)
> Fixed in: unreleased

## 1. Debt
### DBT-001: datafusion, arrow and parquet pin a vulnerable zstd
- **Current structure:** `Cargo.toml:123-125` declares datafusion 43, arrow 53 and parquet 53. Their zstd consumers are `datafusion`, `async-compression` and `parquet` (`cargo tree -i zstd@0.13.3 --all-features --depth 1`). The released datafusion 55.2.0 would only move to arrow 59, which still requires `zstd ^0.13`; the unreleased `main` of apache/datafusion declares arrow 60 and zstd 0.14 (`Cargo.toml` of that repository).
- **Cost paid today:** `cargo audit` reports 11 advisories on the zstd family (of which 3 vulnerabilities), so the `security` step of `make check` fails on main and the tests `make_security_passes`, `cargo_audit_reports_no_vulnerability_and_no_warning` and `deny_toml_drops_the_fixed_git2_ignores` in `crates/mcp-fs/tests/security_gate.rs` fail. Parquet files with zstd compressed pages read through the optional `db` tool family reach the vulnerable decoder.
- **Target structure:** datafusion from a pinned git revision (arrow 60, parquet 60, zstd 0.14.3, zstd-safe 8.2.0, zstd-sys 2.1.2).
- **Observable behavior:** unchanged.

### DBT-002: tantivy 0.26 pulls zstd 0.13 through tantivy-sstable
- **Current structure:** `Cargo.toml:88` declares `tantivy = "0.26"`; `tantivy-sstable 0.7.0` requires `zstd ^0.13` (crates.io metadata of `tantivy-sstable/0.7.0`), and it is the fourth consumer of the old zstd.
- **Cost paid today:** after DBT-001 alone, zstd 0.13.3 and zstd-safe 7.3.0 remain in the graph (measured on a scratch branch).
- **Target structure:** `tantivy = "0.27"` (its `tantivy-sstable` requires `zstd ^0.14`).
- **Observable behavior:** unchanged; search results and the on disk index of the optional search family keep working.

### DBT-003: gate configuration and documents follow the move
- **Current structure:** `deny.toml:40-42` denies every git source; `.cargo/audit.toml:7-8` ignores `instant` and `paste` "via datafusion 43"; no document records a pinned dependency.
- **Cost paid today:** the git source would be refused by `cargo deny`; two ignores become stale; the pin has no recorded reason or exit.
- **Target structure:** exactly one git source allowed; the two stale ignores removed; a short document states the pin, its reason and its exit (BL-0089).
- **Observable behavior:** unchanged.

## 2. Perimeter: the Current State (MANDATORY, every depth)
| File | Change | Occurrences |
|---|---|---|
| `Cargo.toml` | lines 88 (tantivy), 123 (datafusion), 124 (arrow), 125 (parquet) | 4 |
| `Cargo.lock` | regenerated for the four packages and their dependents (about 1900 changed lines on the scratch probe; the three `zstd*` package entries at `Cargo.lock:6887-6906` replaced) | 3 entries |
| `crates/core/src/tools/db.rs` | line 93 `NdJsonReadOptions` becomes `JsonReadOptions`; 24 lines reference datafusion or arrow items and were compiled unchanged | 1 |
| `deny.toml` | `[sources]` at lines 40-42 gains the allowed git source | 1 |
| `.cargo/audit.toml` | lines 7 and 8 removed; line 10 reason text re checked against the new graph | 2 |
| `crates/mcp-fs/tests/security_gate.rs` | test `accepted_advisories_are_recorded_once_with_a_reason` (line 149) expected list loses 2 ids; new tests added | 1 test modified |
| `crates/core/src/tools/db.rs` (tests module, from line 585) | characterization tests added | new |
| `crates/core/src/search/bm25_sqlite.rs` (tests module) and a fixture directory | tantivy 0.26 index fixture test added | new |
| `.agent_docs/dependencies.md`, `AGENTS.md` | new document and its index line | 2 |

**Non source occurrences:** `rg -il zstd --glob '!Cargo.lock' --glob '!target' --glob '!sdlc' .` returns no file: no CI file, Makefile target, container file, script or document names zstd. Mentions of datafusion in `config/local.yaml:83`, `tests/functional/scenarios/08_database.sh:12` and `FUNCTIONAL_TESTS.md:370` are prose, not versions, and need no edit. `Makefile:47-48` runs `cargo audit` and `cargo deny check`. No `.github/workflows` directory exists.
**Contracts touched:** all internal. The `db.*` tools and the `search.*` tools keep their parameters and results (DR-009, DR-010); the dependency versions are not part of any wire contract.
**Guardrail coverage:** 8 tests in `crates/core/src/tools/db.rs` (3 behavioural, CSV only: `query_csv_in_volume`, `schema_returns_column_types`, `sample_returns_n_rows`); nothing exercises parquet, JSON, `db.profile` or `db.convert`. Search: 161 e2e tests in `crates/core/src/search/e2e.rs`, 7 in `bm25_sqlite.rs`, 12 in `indexer.rs`. Security gate: 14 tests in `crates/mcp-fs/tests/security_gate.rs`. **Coverage of the db family is thin, so Section 4 (DR-009) requires characterization tests first.** **Test command:** `cargo test --workspace --no-fail-fast`; gate: `make check`.
**Total:** 11 files, 14 line level occurrences, all recounted by command (`cargo tree`, `rg -c`, `rg -n`).

## 3. Class sweep
**Class:** attacker supplied compressed data reaching a memory unsafe decompression context of a third party native library (CWE-416 use after free, CWE-119 buffer errors).
**Searches run:**
1. `rg -n '^name = "zstd' -A1 Cargo.lock` : lockfile idiom; 3 hits.
2. `cargo tree -i zstd@0.13.3 --all-features --depth 1` : dependency graph idiom; 4 consumers.
3. `rg -n -i 'zstd' crates --glob '*.rs'` : direct source idiom; 0 hits.
4. `rg -il zstd --glob '!Cargo.lock' --glob '!target' --glob '!sdlc' .` : non source idiom (CI, scripts, docs, manifests); 0 hits.
5. `rg -n 'flate2|bzip2|lz4|snap|xz|lzma|brotli|zip =|tar =' Cargo.toml crates/*/Cargo.toml` : other decompression libraries idiom; 6 manifest lines (`zip`, `tar`, `flate2`, `bzip2`, `lzma-rs`).
Not searched: network transport compression (no gzip or zstd content encoding is configured in this repository's manifests) and compression inside container images (no container file exists).

| Occurrence | Verdict | Disposition |
|---|---|---|
| `Cargo.lock:6887` zstd 0.13.3 via `datafusion` 43, `async-compression` 0.4.19, `parquet` 53.4.1 | vulnerable | DR-001, DR-004 |
| `Cargo.lock:6887` zstd 0.13.3 via `tantivy-sstable` 0.7.0 | vulnerable | DR-002, DR-004 |
| `Cargo.lock:6896` zstd-safe 7.2.1 (via `zstd`, `async-compression`) | vulnerable | DR-004 |
| `Cargo.lock:6905` zstd-sys 2.0.13 (via `zstd-safe`) | vulnerable | DR-004 |
| direct use in `crates/**/*.rs` | none (search 3: 0 hits) | no action |
| `zip`, `tar`, `flate2`, `bzip2`, `lzma-rs` in `Cargo.toml:73-83` | safe for this class: `cargo audit` reports none of them among the 11 advisories (output of `cargo audit` on main); their advisories, if any, are tracked by SPEC-0036 and `.cargo/audit.toml:11-13` | no action |

Lockfile line numbers refer to main at 7a142ec; DT-020 matches by package name, not by line.

The occurrence that triggered this entry (the zstd consumer of the `db` family, `datafusion`/`parquet`) is fixed in this lot (DR-001).

## 4. Requirements (EARS)
### 4.1 Structural
#### DR-001 [EARS-U]: datafusion, arrow and parquet versions
> The workspace SHALL depend on datafusion from the git repository `https://github.com/apache/datafusion` at revision `6f1436fe1407495019d8afec8a9f55663f418d9a`, on arrow `60` and on parquet `60`.
- **Old form:** `datafusion = { version = "43", features = ["parquet"] }`, `arrow = "53"`, `parquet = { version = "53", features = ["arrow"] }` (`Cargo.toml:123-125`).
- **New form:** `datafusion = { git = "https://github.com/apache/datafusion", rev = "6f1436fe1407495019d8afec8a9f55663f418d9a", features = ["parquet"] }`, `arrow = "60"`, `parquet = { version = "60", features = ["arrow"] }`.
- **Applies to:** `Cargo.toml` and `Cargo.lock`.
- **Casing convention:** Cargo manifest keys, lower case.

#### DR-002 [EARS-U]: tantivy version
> The workspace SHALL depend on tantivy `0.27`.
- **Old form:** `tantivy = "0.26"` (`Cargo.toml:88`).
- **New form:** `tantivy = "0.27"`.
- **Applies to:** `Cargo.toml`, `Cargo.lock`.
- **Casing convention:** as DR-001.

#### DR-003 [EARS-U]: renamed datafusion option type
> The `db` tool family SHALL register NDJSON files with `JsonReadOptions`.
- **Old form:** `NdJsonReadOptions::default()` (`crates/core/src/tools/db.rs:93`).
- **New form:** `JsonReadOptions::default()`.
- **Applies to:** `crates/core/src/tools/db.rs`.
- **Casing convention:** Rust type names, UpperCamelCase.

#### DR-004 [EARS-UB]: no vulnerable zstd in the lockfile
> The lockfile SHALL NOT contain a `zstd` package below 0.14.3, a `zstd-safe` package below 8.2.0 or a `zstd-sys` package below 2.1.2.
- **Old form:** `zstd 0.13.3`, `zstd-safe 7.2.1`, `zstd-sys 2.0.13+zstd.1.5.6` (`Cargo.lock:6887-6906`).
- **New form:** `zstd 0.14.3`, `zstd-safe 8.2.0`, `zstd-sys 2.1.2+zstd.1.5.7` (measured with DR-001 and DR-002 applied).
- **Applies to:** `Cargo.lock`.
- **Casing convention:** crate names lower case, hyphenated.

#### DR-005 [EARS-U]: one allowed git source
> The dependency policy file SHALL allow the git source `https://github.com/apache/datafusion` and no other git source.
- **Old form:** `[sources]` with `unknown-git = "deny"` and no allowance (`deny.toml:40-42`).
- **New form:** the same section plus `allow-git = ["https://github.com/apache/datafusion"]`.
- **Applies to:** `deny.toml`.
- **Casing convention:** TOML keys, kebab case as in `cargo-deny`.

#### DR-006 [EARS-U]: stale advisory ignores removed
> The advisory acceptance file SHALL NOT ignore `RUSTSEC-2024-0384` or `RUSTSEC-2024-0436`, and every remaining ignore SHALL match an advisory present in the audited graph.
- **Old form:** both ids ignored "via datafusion 43" (`.cargo/audit.toml:7-8`).
- **New form:** both lines absent; the reason text of an ignore that names a version that no longer applies (`.cargo/audit.toml:10`, "via tantivy 0.26.2") is corrected to the version in the lockfile.
- **Applies to:** `.cargo/audit.toml`.
- **Casing convention:** advisory ids upper case.

#### DR-007 [EARS-E]: documentation of the pin
> WHEN the workspace depends on a git revision, THE repository SHALL carry `.agent_docs/dependencies.md` stating the revision, the reason, the single allowed source and the exit condition (BL-0089), and `AGENTS.md` SHALL index it.
- **Old form:** none.
- **New form:** `.agent_docs/dependencies.md` and one line in the documentation index of `AGENTS.md`.
- **Applies to:** `.agent_docs/dependencies.md`, `AGENTS.md`.
- **Casing convention:** Markdown.

#### DR-008 [EARS-U]: follow up entry
> The change SHALL leave a backlog entry `sdlc/backlog/BL-0089-datafusion-from-crates-io.md` for the move to a released datafusion.
- **Old form:** none. **New form:** the entry created by this document's commit.
- **Applies to:** `sdlc/backlog/`. **Casing convention:** as the other backlog entries.

### 4.2 Invariance (MANDATORY)
#### DR-009 [EARS-U]: Behavior invariance
> The system SHALL produce, for every input covered by the existing test suite and by the characterization tests DT-001 to DT-011, byte identical observable output before and after this change, including the column type names `db.schema` reports.

#### DR-010 [EARS-U]: Search index compatibility
> The search family SHALL open a tantivy index directory written by tantivy 0.26 and answer the same query with the same top result.

### 4.3 Compatibility
#### DR-011 [EARS-O]: Incompatible index
> IF DT-030 shows that a tantivy 0.26 index does not open under 0.27, THEN the change SHALL NOT land and the incompatibility SHALL be handled as a declared break decided by the user (Section 7).

#### DR-012 [EARS-E]: Audit gate
> WHEN `make check` runs the security step, THE gate SHALL pass: `cargo audit` reports no vulnerability and no warning, and `cargo deny check` reports no error.

## 5. Compatibility plan
| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `db.query`, `db.schema`, `db.profile`, `db.sample`, `db.convert` results | tool results | public (MCP) | invariant, proven by DT-001 to DT-011 recorded on datafusion 43 first | none | none |
| tantivy on disk index of the search family | persisted data | internal to the deployment, persisted | invariant, proven by DT-030 | none | none |
| `Cargo.toml` git dependency | build input | internal | one allowed source, pinned revision | until a released datafusion carries arrow 60 | BL-0089 |

**Dual maintenance semantics:** none; no name is renamed for callers. If a datafusion difference changes a characterized output, the code restores the old output (for example the string column type name); changing a test expectation is not allowed (DEC-006).

## 6. Decisions
- **DEC-001:** Depend on datafusion by git revision `6f1436fe1407495019d8afec8a9f55663f418d9a` (apache/datafusion main, committed 2026-10-27, workspace version 55.1.0) with arrow 60 and parquet 60. **Alternatives considered:** (a) datafusion 55.2.0 from crates.io plus advisory ignores (rejected: it keeps zstd 0.13 because arrow 59 requires `zstd ^0.13`; nothing is removed); (b) vendored `[patch]` copies of `arrow-ipc` and `parquet` with zstd 0.14 (rejected: large copies in the repository); (c) waiting for a release (rejected: the audit gate is red on main now). **Rationale:** the only way to remove the vulnerable code today, chosen by the user. **Implemented by:** DR-001. **Evidence:** `Cargo.toml:123`; crates.io `datafusion` max version 55.2.0; apache/datafusion `Cargo.toml` (`arrow = 60.0.0`, `zstd = 0.14`).
- **DEC-002:** arrow and parquet are direct dependencies at 60 so they unify with the versions datafusion main uses. **Alternatives considered:** letting datafusion bring them in (rejected: `crates/core/Cargo.toml:89-90` depends on both directly). **Implemented by:** DR-001. **Evidence:** `Cargo.toml:124-125`.
- **DEC-003:** Upgrade tantivy to 0.27. **Alternatives considered:** `default-features = false` without `zstd-compression` (rejected: it would change which stored field compressors can be read; the upgrade keeps all); staying on 0.26 with an ignore (rejected: the user asked to remove zstd 0.13). **Implemented by:** DR-002. **Evidence:** `Cargo.toml:88`; tantivy 0.27.0 CHANGELOG (breaking changes concern APIs this code does not use: it compiled unchanged).
- **DEC-004:** Allow exactly one git source in `deny.toml`, the repository URL without a wildcard; the revision stays pinned by `Cargo.toml` and `Cargo.lock`. **Alternatives considered:** `unknown-git = "warn"` (rejected: weakens the gate for every future dependency). **Implemented by:** DR-005. **Evidence:** `deny.toml:40-42`.
- **DEC-005:** Remove the two stale ignores and modify the one existing test that lists them (declared exception, Section 8.4). **Alternatives considered:** leaving the ignores (rejected: `cargo audit` may flag unused ignores and the recorded reason would be false). **Implemented by:** DR-006. **Evidence:** `.cargo/audit.toml:7-8`; `security_gate.rs:149-182`.
- **DEC-006:** The characterization tests are written first, recorded on datafusion 43, and become the oracle: where datafusion 55 differs, the code restores the old output; a test expectation is never edited. A difference that cannot be restored is an F finding for the user (a declared break). **Alternatives considered:** accepting new outputs (rejected: smuggles behaviour change into a DEBT). **Implemented by:** DR-009. **Evidence:** `crates/core/src/tools/db.rs:585-703` (8 tests, thin).
- **DEC-007:** `cargo update` is limited to the four packages (`cargo update -p datafusion -p arrow -p parquet -p tantivy`) so unrelated crates keep their locked versions. **Alternatives considered:** a full `cargo update` (rejected: the probe changed about 1900 lockfile lines and mixes unrelated upgrades into a security move). **Implemented by:** DR-001, DR-002. **Evidence:** probe diff on a scratch branch.
- **DEC-008:** Security tests are structural plus a rejection guard because the effect of the class (uninitialized or freed memory exposure) is not observable by an assertion. The observable instrumentation is DR-004 and DR-012. **Alternatives considered:** a fuzzing harness (rejected: out of proportion for a dependency bump). **Implemented by:** DR-004, DR-012. **Evidence:** advisories text (`cargo audit` output).
- **DEC-009:** Record the pin in `.agent_docs/dependencies.md` and create BL-0089 for the exit. **Alternatives considered:** a comment in `Cargo.toml` only (rejected: no index, no exit tracking). **Implemented by:** DR-007, DR-008. **Evidence:** `AGENTS.md` documentation index.
- **Assumption:** the line count of the lockfile change (about 1900 lines) comes from one probe run and is not binding.

## 7. Declared breaks
None. DR-011 reserves the case where a tantivy 0.26 index would not open under 0.27: then the change stops and the user decides (rebuild of the derived index is the likely answer, since the search index is rebuilt from volume content).

## 8. Tests
### 8.1 Non regression: the success criterion
The existing test suite passes **unmodified**, with one declared exception (8.4):
    cargo test --workspace --no-fail-fast
    make check

### 8.2 Compatibility tests
#### DT-030: tantivy 0.26 index still opens
- **Validates:** DR-010, DR-011 on the persisted search index
- The test copies `fixtures/tantivy_0_26/` into `{tempdir}/vol/` (the layout `{tantivy_dir}/{volume_id}/` of `crates/core/src/search/bm25_sqlite.rs:65-79`) and builds the backend with `TantivyBm25Backend::new(tempdir)`; it never opens the fixture in place, since opening for write would modify it.
- Given a small index directory written by tantivy 0.26 with documents `alpha.md` ("hello world") and `beta.md` ("goodbye moon"), committed as a fixture before the upgrade, When the search family opens it under tantivy 0.27 and queries `hello`, Then the top result is `alpha.md` with the same score ordering as recorded on 0.26, And the query `moon` returns `beta.md`.
- **Location:** tests module of `crates/core/src/search/bm25_sqlite.rs`, fixture under `crates/core/src/search/fixtures/tantivy_0_26/`.

#### DT-031: search e2e unmodified
- **Validates:** DR-009 on the search family
- Given the 161 tests of `crates/core/src/search/e2e.rs`, When they run on the new dependency set, Then all pass with no edit.

### 8.3 Structural tests
#### DT-001 to DT-011: characterization of the db family (written first, green on datafusion 43 and on the new set)
- **Validates:** DR-009. **Location:** tests module of `crates/core/src/tools/db.rs`. **Method:** direct call through the registered tools with files stored in a volume; expected values recorded verbatim from datafusion 43.
- DT-001: `db.query` on a CSV with columns `id` (1, 2, 3), `name` (`Alice`, `Bob`, `Carol`), `score` (1.5, 2.5, 4.0): `SELECT count(*) AS n, sum(score) AS total FROM t` returns `n` 3 and `total` 8.0.
- DT-002: `db.query` on a Parquet file produced by `db.convert` from that CSV returns the same rows.
- DT-003: `db.query` on an NDJSON file with the same 3 rows returns them.
- DT-004: `db.schema` on the CSV, the Parquet and the NDJSON file returns the exact column type names recorded on 43 (string, integer, float columns).
- DT-005: `db.profile` on the CSV returns the recorded profile (row count, null counts, min and max per column).
- DT-006: `db.profile` on the Parquet file returns the same profile.
- DT-007: `db.sample` with n 2 on the NDJSON file returns 2 rows.
- DT-008: `db.convert` CSV to Parquet to CSV returns bytes equal to the recorded CSV text.
- DT-009: `db.convert` to NDJSON returns the recorded NDJSON text.
- DT-010: `db.query` with `SELEKT 1` returns the recorded error text.
- DT-011: with `max_rows` 2, `db.query` on the 3 row CSV returns `rows` with 2 entries and `row_count` 2, and a payload with no key other than `columns`, `rows` and `row_count`.

The literal expected values of DT-001 to DT-011 are written into the test source in step 1 of Section 9, before any dependency change, and the test source is their record.

#### DT-020: no old zstd in the lockfile
- **Asserts:** DR-004 : no `zstd` below 0.14.3, `zstd-safe` below 8.2.0, `zstd-sys` below 2.1.2. **Method:** parse `Cargo.lock` as `lock_packages` does in `security_gate.rs:41`.
#### DT-021: audit reports nothing
- **Asserts:** DR-012 : `cargo audit --json` has zero vulnerabilities and an empty warnings object (the existing `cargo_audit_reports_no_vulnerability_and_no_warning`, `security_gate.rs:103`, goes green; no edit).
#### DT-022: one git source allowed
- **Asserts:** DR-005 : `deny.toml` contains exactly one `allow-git` entry equal to `https://github.com/apache/datafusion`. **Method:** text parse in `security_gate.rs`.
#### DT-023: no stale ignore
- **Asserts:** DR-006 : the raw text of `.cargo/audit.toml` contains neither `RUSTSEC-2024-0384` nor `RUSTSEC-2024-0436`, every id still listed appears in the output of `cargo audit --json` as an ignored or present advisory, and the stderr of `cargo deny check` does not contain `advisory-not-detected`. **Method:** text parse plus two commands.
#### DT-024: renamed option type
- **Asserts:** DR-003 : `rg -n 'NdJsonReadOptions' crates` finds no occurrence. **Method:** grep assertion in `security_gate.rs` style test.
#### DT-025: pin documented
- **Asserts:** DR-007, DR-008 : `.agent_docs/dependencies.md` exists and names the revision and `BL-0089`; `AGENTS.md` links it; `sdlc/backlog/BL-0089-datafusion-from-crates-io.md` exists. **Method:** file assertions.

### 8.4 Existing tests to modify
| Test file | Test | Change | Justification |
|---|---|---|---|
| `crates/mcp-fs/tests/security_gate.rs` | `accepted_advisories_are_recorded_once_with_a_reason` (line 149) | remove `RUSTSEC-2024-0384` and `RUSTSEC-2024-0436` from the expected id list | DR-006 removes exactly those two ignores because `instant` and `paste` leave the dependency graph with datafusion 55; the test asserts the ignore list equals the recorded list |

### 8.5 Security tests
#### DT-040: zstd compressed parquet is read
- **Covers:** DR-001, sweep occurrence `Cargo.lock:6887` (datafusion and parquet path)
- **Driver:** direct call through `db.query`
- **Red before the fix because:** EFFECT NOT OBSERVABLE: the class corrupts or exposes memory inside the decoder and no assertion reads it; the observable instrumentation is DT-020 (lockfile) and DT-021 (audit), which are red on main now: `cargo audit` reports 3 vulnerabilities and `deny_toml_drops_the_fixed_git2_ignores` fails.
- The fixture is written by the test with `parquet::file::properties::WriterProperties` using `Compression::ZSTD`, not through `db.convert`.
- Given a Parquet file whose pages are zstd compressed with the rows (1, `Alice`) and (2, `Bob`), When `db.query` runs `SELECT id, name FROM t ORDER BY id`, Then the rows come back exactly, And the same query run a second time returns the same bytes.

#### DT-041: truncated legacy zstd frame is rejected
- **Covers:** DR-004, sweep occurrence `Cargo.lock:6896` (advisory on a failing streaming decompression of a legacy frame)
- **Driver:** direct call through `db.query`
- **Red before the fix because:** EFFECT NOT OBSERVABLE: as DT-040; the rejection is the expected behaviour on both versions, so this test is a guard, not the proof.
- The fixture is the file of DT-040 with the bytes of the first data page replaced in place, the page header and the file length unchanged.
- Given a Parquet file whose first data page is replaced by the literal bytes `22 B5 2F FD 00 00` (a legacy zstd magic number followed by a truncated header), When `db.query` runs `SELECT * FROM t`, Then the call returns an error and no rows, And a following `db.query` on the valid file of DT-040 returns its rows unchanged.

#### DT-042: the security step passes
- **Covers:** DR-012, all occurrences. **Driver:** the existing `make_security_passes` (`security_gate.rs:21`) goes green on the new set; no edit.

## 9. Implementation order
1. Characterization first, on the current versions: DT-001 to DT-011, DT-030 with its fixture generated by tantivy 0.26, DT-040 and DT-041. All green on main. (`make check` stays red only at the security step, as on main today.)
2. Bump tantivy to 0.27 (`cargo update -p tantivy`): DR-002. DT-030, DT-031 and the search suite stay green. Audit still red (datafusion path).
3. Bump datafusion, arrow and parquet (`cargo update -p datafusion -p arrow -p parquet`), rename the option type (DR-001, DR-003), allow the git source (DR-005). DT-001 to DT-011 stay green; code restores any changed output (DEC-006). After steps 2 and 3, DT-020 and DT-021 go green.
4. Remove the stale ignores and modify the declared test (DR-006), add DT-020 to DT-025, write `.agent_docs/dependencies.md` and index it (DR-007), keep BL-0089 (DR-008), and run `make check` fully green (DR-012). Steps 1 and 2 may land before 3; the security step turns green only once 3 and 4 have landed.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 8 minor | IMPLEMENTABLE |

**Amendments applied:** A-1 test count 14; A-2 prose mentions of datafusion; A-3 lockfile lines are for main at 7a142ec; A-4 DT-011 asserts the real payload keys; A-5 literal expected values live in the test source; A-6 fixture construction of DT-040 and DT-041; A-7 DT-030 copies the fixture to a tempdir; A-8 DT-023 concrete commands. Auditor: claude-sonnet-5-5 (not the model that wrote the document), one round.

## Design additions (2026-10-27)

### Test strategy and stories
All tests are functional or structural tests through the real functions of the crate or the real repository files; none is a mock of its own output. Target tier 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-001 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-002 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-003 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-004 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-005 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-006 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-007 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-008 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-009 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-010 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-011 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-020 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0103 |
| DT-021 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0103 |
| DT-022 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0103 |
| DT-023 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0104 |
| DT-024 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0103 |
| DT-025 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0104 |
| DT-030 | integration | direct call to the search backend | `crates/core/src/search/bm25_sqlite.rs` tests module and search e2e | US-0101 |
| DT-031 | integration | direct call to the search backend | `crates/core/src/search/bm25_sqlite.rs` tests module and search e2e | US-0102 |
| DT-040 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-041 | functional e2e | direct call through the registered tools | `crates/core/src/tools/db.rs` tests module | US-0101 |
| DT-042 | structural | file and command assertion | `crates/mcp-fs/tests/security_gate.rs` | US-0103 |

Test data: generated (CSV, Parquet and NDJSON files built by the tests), plus one committed fixture directory for the tantivy 0.26 index (DT-030), generated once by tantivy 0.26 in the first story.

- **DEC-010:** Four stories along the implementation order of Section 9: characterization first, tantivy, datafusion with the zstd removal, then cleanup and documentation. Each leaves main green except the security step, which is red on main today and turns green in US-0103 (DR-012). **Obliged by:** SPEC-0055/DR-009, DR-012. **Alternatives considered:** one story for everything (rejected: 11 files and a lockfile regeneration in one unverifiable unit); datafusion before tantivy (rejected: the order in Section 9 puts the smaller, independently testable bump first). **Source:** design. **Evidence:** Section 9.
- **DEC-011:** Test locations: db characterization and zstd guards in the tests module of `crates/core/src/tools/db.rs`; the tantivy fixture test in the tests module of `crates/core/src/search/bm25_sqlite.rs` with the fixture under `crates/core/src/search/fixtures/tantivy_0_26/`; structural gate tests in `crates/mcp-fs/tests/security_gate.rs` next to the SPEC-0035 and SPEC-0036 tests. **Obliged by:** SPEC-0055/DR-009, DR-010, DR-004. **Alternatives considered:** a new integration test crate (rejected: the helpers are private to those modules). **Source:** existing code. **Evidence:** `crates/core/src/tools/db.rs:585`, `crates/mcp-fs/tests/security_gate.rs:41`.
