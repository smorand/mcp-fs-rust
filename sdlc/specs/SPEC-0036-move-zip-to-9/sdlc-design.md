# Move the direct zip dependency to 9.x: technical debt design

> Id: SPEC-0036
> Nature: DEBT
> Depth: M
> Depth evidence: one area (zip readers: 2 production sites, 7 test files), 6 requirements, no persisted change, one declared observable delta on archive entry names (Section 7)
> Status: implemented
> Generated on: 2026-10-18
> Behavior change: BREAKING on `fs.extract_archive` entry name decoding and deduplication, and on compressed archive bytes of the zip writers, see Section 7
> From backlog: BL-0058
> Depends on: SPEC-0035 (implemented; `.cargo/audit.toml` exists)
> Security: RUSTSEC-2026-0367, RUSTSEC-2026-0368, RUSTSEC-2026-0369 (zip, direct copy fixed; build time copy stays accepted)
> CVSS: RUSTSEC-2026-0367 CVSS:3.1/AV:L/AC:L/PR:L/UI:R/S:U/C:H/I:H/A:H; RUSTSEC-2026-0369 CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:H/A:H; RUSTSEC-2026-0368 not scored
> Affected: every commit whose Cargo.lock pins zip 2.4.2, including main at 4ebc36e
> Fixed in: unreleased

## 1. Debt

### DBT-001: The direct zip dependency is a release line with open advisories
- **Current structure:** `zip = { version = "2", default-features = false, features = ["deflate", "aes-crypto"] }` (`Cargo.toml:87`, used through `crates/core/Cargo.toml:68`) resolves to zip 2.4.2 (`Cargo.lock:7026-7027`). zip 2.4.2 carries RUSTSEC-2026-0367, 0368 and 0369 (patched `>= 9.0.0`, `cargo audit --json` on 2026-10-17). They are accepted in `.cargo/audit.toml:13-15` and `deny.toml:19-23` with "until BL-0058".
- **Cost paid today:** three accepted vulnerabilities on a direct dependency; the acceptance reason is "never called", a property nothing tests.
- **Target structure:** zip 9.x for the direct dependency; the three ids stay accepted only for the build time copy (zip 3.0.0 via `utoipa-swagger-ui` 9, `Cargo.lock:7050-7051`), whose newest release 10.0.1 still requires `zip ^8.6` (crates.io API, 2026-10-18), so BL-0059 cannot remove it either.
- **Observable behavior:** unchanged except the declared break of Section 7.

## 2. Perimeter: the Current State

Probe (2026-10-18, scratch worktree off main): `zip = "9"` then `cargo update -p zip@2.4.2` updates zip 2.4.2 to 9.0.5, constant_time_eq 0.3.1 to 0.6.1, pbkdf2 0.12.2 to 0.13.0, adds aes 0.9.2, cipher 0.5.2, cpubits 0.1.1, inout 0.2.4, nix 0.31.3, typed-path 0.12.3, removes zeroize_derive 1.5.0 (all selected for Rust 1.88; zip 9.0.5 declares `rust-version = "1.88"`). This set is informative only: the implementation accepts whatever `cargo update -p zip@2.4.2` resolves, provided DT-030 passes, `make security` exits 0 and the build stays on Rust 1.88. `cargo check --workspace --all-targets --all-features` then fails with 13 errors over 12 sites (`docs/extract.rs:553` raises 2), every one caused by `ZipFile::name()` and `ZipArchive::file_names()` returning `ZipResult<Cow<str>>` instead of `&str` (`zip-9.0.5/src/read/zipfile.rs:83`, `zip-9.0.5/src/read/zip_archive.rs:468`).

| File | Change | Occurrences |
|---|---|---|
| `Cargo.toml` | `:87` version `"2"` becomes `"9"`, features unchanged | 1 |
| `Cargo.lock` | zip 2.4.2 entry becomes 9.0.5 plus the probe's additions and moves | 1 entry plus probe set |
| `crates/core/src/docs/extract.rs` | `:553` `zip.file_names().map(str::to_string)` adapts to the `Result` item | 1 production |
| `crates/core/src/tools/archive.rs` | `:397` `file.name().to_string()` adapts (production); pre scan loop `:395-410` gains the decoded name duplicate refusal (DR-008, production); `:288`, `:289`, `:380`, `:381`, `:1120`, `:1626` comments cite `zip-2.4.2` source paths; tests `:952`, `:1287`, `:1291` | 1 production, 6 comments, 3 test |
| `crates/core/src/api/dataplane.rs` | tests `:2510`, `:2639`, `:2911` | 3 test |
| `crates/core/src/docs/docx.rs` | test `:378` | 1 test |
| `crates/core/src/exports.rs` | test `:249` | 1 test |
| `crates/core/src/tools/export.rs` | test `:286` | 1 test |
| `crates/core/src/tools/artifacts.rs` | test `:4154` | 1 test |
| `.cargo/audit.toml` | `:13-15` reasons for RUSTSEC-2026-0367/0368/0369 name the build time copy and BL-0059 instead of BL-0058 | 3 |
| `deny.toml` | `:19-23`: 2 comment lines and 3 reasons, same change | 5 lines |
| `crates/mcp-fs/tests/security_gate.rs` | DT-030, DT-031, DT-036 | new |
| `crates/core/src/docs/docx.rs` (writer) | `:46-60` `pack`, reached through `core/fs_ops.rs:1791`: no edit, output declared DR-007; test DT-038 | 0 edit |
| `crates/core/src/api/dataplane.rs` (writer) | `:643-645` directory zip download: no edit, output declared DR-007 | 0 edit |
| `crates/core/src/tools/export.rs` (writer) | `:80-82` `fs.export_zip`: no edit, output declared DR-007 | 0 edit |

**Non source occurrences:** no Makefile, CI, script or container file names zip (`rg -n -i "\bunzip\b|zipfile|\.zip\b" Makefile scripts .github` returns nothing). `AGENTS.md` names `zip` in the stack line only, version free: unchanged.
**Contracts touched:** `fs.extract_archive` and `POST /api/fs/{mount_id}/extract-archive` (public, same engine `tools/archive.rs:386`): entry names, Section 7. Archive acceptance is unchanged except entry deduplication, which moves from decoded name (`zip-2.4.2/src/read.rs:72-74`, keyed by the decoded `file_name`) to raw name bytes (`zip-9.0.5/src/read/zip_archive.rs:82-85`), DR-008. Otherwise: zip 9 validates local against central headers only when `ZipReadOptions::enforce_header_consistency(true)` is set (`zip-9.0.5/src/read/zip_archive.rs:736`), the option derives `Default` as `false` (`zip-9.0.5/src/read/mod.rs:251`, `:270`), and `by_index`, `by_index_decrypt` and `by_name` all pass `ZipReadOptions::new()` (`zip_archive.rs:664`, `:685`, `:690`); this lot never enables it. `fs.extract_text` on OOXML (`docs/extract.rs:540-554`): member names are matched against fixed ASCII names (`word/document.xml` and similar) or against the `ppt/slides/slide*.xml` prefix and suffix (`docs/extract.rs:904-908`); an ASCII name decodes identically in 2.x and 9.x, so the output is unchanged for every name the matching selects. Three writers deflate archives: `fs.export_zip` (`tools/export.rs:80-82`), the REST directory zip download (`api/dataplane.rs:643-645`) and docx generation (`docs/docx.rs:46-60`, via `core/fs_ops.rs:1791`, stored as a file). The `deflate` feature backend changes (`zip-2.4.2/Cargo.toml:78-82` `flate2/rust_backend` plus zopfli; 9.0.5 zlib-rs plus zopfli), so compressed bytes, size and content hash can differ; entries, order and decompressed contents do not (DR-007, Section 7). Name matching filters that select members exclude non zip `entry.name()` calls: `archive.rs:513`, `:526` (sevenz), `artifacts.rs:4043` and `git/http/mod.rs:337` (git2).
**Guardrail coverage:** archive extraction tests in `crates/core/src/tools/archive.rs` (tests module from `:679`), export tests `tools/export.rs:275-300`, `exports.rs:240-255`, REST tests `api/dataplane.rs:2500-2920`, OOXML extraction `docs/extract.rs` tests. **Test command:** `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
**Total:** 11 files plus the lockfile; 3 production edits, 10 test edits, 6 comments, 8 config lines; recounted from the probe output and `rg -n "\.name\(\)|file_names\(\)" crates --type rust` filtered to zip receivers.

## 3. Class sweep

**Class:** untrusted archive entry metadata of any archive format (entry name, unix mode bits, symlink or hardlink target) reaching a filesystem write or a permission change (CWE-22, CWE-59, CWE-732).
**Searches run:**
1. `rg -n "\.extract\(|extract_unwrapped|set_permissions|from_mode|symlink|enclosed_name|unix_mode\(" -g '!target' -g '!sdlc' -g '!*.md' .` : the sink idiom (library extraction helpers, permission application, symlink creation).
2. `rg -n "ZipArchive::new|ZipStreamReader|read_zipfile_from_stream|by_name\(|by_index" crates scripts -g '*.rs' -g '*.py' -g '*.sh'` : the source idiom (every place an archive is opened or an entry read).
3. `rg -n -i "\bunzip\b|zipfile|\.zip\b" Makefile scripts .github` : non source extraction (none).
4. `rg -n "ensure_entry_path_safe|sevenz_special_type|EntryType::" crates/core/src/tools/archive.rs` : the tar and 7z decoders writing into the same volume.
Not searched: archive formats the server does not decode (none beyond zip, tar, 7z: `tools/archive.rs` dispatches by extension to `decode_zip`, `decode_tar` `:591`, `decode_sevenz` `:468`).

| # | Occurrence | Verdict | Disposition |
|---|---|---|---|
| 1 | `crates/core/src/tools/archive.rs:386` zip extraction for `fs.extract_archive` | safe: writes only into the simulated volume, never the host filesystem; symlink entries refused at `archive.rs:398-402`; every entry path checked by `ensure_entry_path_safe` (`archive.rs:351`, called `:403`) which rejects `..` escapes; no unix mode is applied to the stored node | DR-001 upgrade plus guardrails DT-032, DT-033 |
| 2 | `crates/core/src/docs/extract.rs:540` OOXML reader | safe: reads members by fixed name as text (`:546`), writes nothing | DR-001 upgrade, covered by DT-037 |
| 3 | `utoipa-swagger-ui-9.0.2/build.rs:93-133` (zip 3.0.0, build dependency) | build time only, input is the crate's own vendored Swagger UI zip, not attacker controlled; sets permissions at `:133` from that trusted archive | stays accepted (DR-004), removal tracked by BL-0059 |
| 4 | `crates/core/src/docs/service.rs:469-500` converter output collection | out of class: reads a local folder the external converter produced, not archive entry metadata; a replaced output folder is refused (`:484-489`, test `:988`), symlinks inside are skipped (`:496`) | none, not touched by this lot |
| 5 | `crates/core/src/tools/archive.rs:468` 7z pre scan | safe: path check `:513`, special entry refusal `:514` (`sevenz_special_type` `:565`), writes only into the simulated volume | none, existing 7z tests are the guardrail |
| 6 | `crates/core/src/tools/archive.rs:591` tar decoder | safe: path check `:639`, symlink and hardlink entries refused (`:670-671`), writes only into the simulated volume | none, existing tar tests are the guardrail (`:1210`, `:1263`) |
| 7 | test only readers (`api/dataplane.rs`, `docs/docx.rs`, `exports.rs`, `tools/export.rs`, `tools/artifacts.rs`, `tools/archive.rs` tests) | read archives the tests built themselves | API adaptation only (Section 8.4) |

The originating occurrence (the direct dependency) is fixed in this lot by DR-001.

## 4. Requirements (EARS)

### 4.1 Structural
#### DR-001 [EARS-U]: Direct zip on 9.x
> The workspace SHALL depend directly on `zip` 9.x with features `deflate` and `aes-crypto` and default features off, and Cargo.lock SHALL contain no zip release below 9 among normal (non build) dependencies.
- **Old form:** `zip = { version = "2", default-features = false, features = ["deflate", "aes-crypto"] }`
- **New form:** `zip = { version = "9", default-features = false, features = ["deflate", "aes-crypto"] }`
- **Applies to:** `Cargo.toml:87`, `Cargo.lock`

#### DR-002 [EARS-U]: Entry names read through the 9.x API
> The two production readers (`docs/extract.rs` entry listing, `tools/archive.rs` pre scan) SHALL obtain entry names through the `Result` returning `name()` / `file_names()` and SHALL map an `Err` to the same error the surrounding code already returns for an unreadable entry: `corrupt("zip", e)` in `tools/archive.rs`, and omission of that name from the listing in `docs/extract.rs`.
- **Old form:** `file.name().to_string()`, `zip.file_names().map(str::to_string)`
- **New form:** `file.name().map_err(|e| corrupt("zip", e))?.into_owned()`, `zip.file_names().filter_map(Result::ok).map(Cow::into_owned)`
- **Casing convention:** Rust snake_case, unchanged

#### DR-003 [EARS-UB]: No stale source citation
> `crates/core/src/tools/archive.rs` SHALL NOT cite `zip-2.4.2` source paths; the comments at `:288-289`, `:380-381`, `:1120` and `:1626` cite the zip 9.0.5 locations of the same functions (`by_index_raw`, `encrypted`, `is_symlink`, `unix_mode`, the writer and reader lines cited) or drop the path.

#### DR-004 [EARS-U]: Acceptance narrowed to the build time copy
> `.cargo/audit.toml` and `deny.toml` SHALL keep RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369 with a reason naming zip 3.0.0 as a build dependency of utoipa-swagger-ui and BL-0059, and SHALL NOT name BL-0058; `make security` SHALL exit 0.

### 4.2 Invariance
#### DR-005 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change, except the declared breaks DR-006, DR-007 and DR-008, and `tool-contract-golden.json` SHALL be unchanged.

### 4.3 Compatibility
#### DR-006 [EARS-O]: Declared entry name decoding
> IF a zip entry name is valid UTF-8 THEN `fs.extract_archive` SHALL write it under its UTF-8 decoding whatever the entry's UTF-8 flag, and IF it is not valid UTF-8 THEN `fs.extract_archive` SHALL write it under its CP437 decoding whatever the flag.
- Before: flag set, lossy UTF-8 (`U+FFFD` for invalid bytes); flag clear, CP437 (`zip-2.4.2/src/read.rs:1313-1316`).
- After: `zip-9.0.5/src/types.rs:123-131`.

#### DR-007 [EARS-E]: Declared compressed byte change
> WHEN `fs.export_zip`, `POST /api/fs/{mount_id}/export-zip`, the `/api/fs` directory zip download or docx generation produces an archive THE workspace SHALL keep its entry names, entry order and decompressed contents identical to the 2.x output; its compressed bytes, size and content hash are outside the invariance of DR-005.

#### DR-008 [EARS-O]: Declared entry deduplication change
> IF two central directory entries of a zip have identical raw name bytes THEN `fs.extract_archive` SHALL extract only the entry zip 9 keeps under that key (the last one, `IndexMap::insert`, `zip-9.0.5/src/read/zip_archive.rs:84`), whatever their UTF-8 flags.

#### DR-009 [EARS-O]: Decoded name duplicates refused
> IF two entries of a zip with distinct raw name bytes decode to the same name THEN the `fs.extract_archive` pre scan SHALL fail with `ERR_INVALID_ARGUMENT` and the message `archive is corrupt or not a valid zip file: duplicate entry name '<name>'` (through `corrupt("zip", ..)`, `tools/archive.rs:334-337`), before any write.
- The check runs inside the pre scan loop for entry i, after `ensure_entry_path_safe(&raw_path)?` and before `entries.push`, so for one entry a symlink or path refusal takes precedence, and the first offending entry in central directory order decides the error.
- It is a `HashSet` of decoded names: O(n) time, O(n) space over entries already held in `entries`.
- The existing destination collision check (`tools/archive.rs:173-181`) is unchanged; it compares against the volume, not within the archive.

## 5. Compatibility plan

| Contract | Kind | Public | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `fs.extract_archive` / REST extract-archive entry names | tool and HTTP | yes | break now (DEC-002) | none | n/a |
| `fs.extract_archive` entry deduplication | tool and HTTP | yes | break now (DEC-006) | none | n/a |
| zip writer bytes (export, directory download, docx) | tool, HTTP, stored file | yes | break now (DEC-005) | none | n/a |

**Dual maintenance semantics:** none; DEC-002 records why.

## 6. Decisions
- **DEC-001:** Move to zip 9.x (9.0.5 at probe time), not 8.x. **Alternatives considered:** zip 8.x to match utoipa-swagger-ui 10 (rejected: patched range is `>= 9.0.0`, 8.x keeps the three advisories). **Rationale:** the only release line with the fix. **Implemented by:** DR-001. **Evidence:** advisory `[versions] patched = [">= 9.0.0"]` (rustsec advisory-db `crates/zip/RUSTSEC-2026-0368.md`).
- **DEC-002:** Declare the name decoding change as a break rather than reimplementing zip 2's decoding. Owner decision 2026-10-18. **Alternatives considered:** rebuild the 2.x decoding from `flags()` and `name_raw()` (rejected: keeps mojibake for UTF-8 names from tools that omit the flag, and adds code to preserve a defect).
  The header consistency checks of 9.x stay off (their default); enabling them would be a separate behavior change, not part of this lot. **Rationale:** the new decoding is the more correct one. **Implemented by:** DR-006. **Evidence:** `zip-9.0.5/src/types.rs:123-131`.
- **DEC-003:** Keep the three ids accepted for the build time zip 3.0.0. **Alternatives considered:** move utoipa-swagger-ui to 10 in this lot (rejected: 10.0.1 requires `zip ^8.6`, still affected, and pulls utoipa 6, a separate migration, BL-0059). **Rationale:** the build copy reads only its vendored archive. **Implemented by:** DR-004. **Evidence:** crates.io `utoipa-swagger-ui/10.0.1/dependencies` (2026-10-18).
- **DEC-004:** An `Err` from `name()` maps to existing error paths (DR-002), never a new error code. **Alternatives considered:** `unwrap` (rejected: a panic in a request path). **Rationale:** `Err` is unreachable in 9.0.5 (it only arises from bytes below 0x80 failing UTF-8, which cannot happen, `zip-9.0.5/src/cp437.rs:17-27`), so the mapping is a defensive path with no new contract. **Implemented by:** DR-002. **Evidence:** cited.
- **DEC-005:** Accept compressed byte changes of the three writers. Owner default applied 2026-10-18 (recommendation stated, no objection). **Alternatives considered:** pin the 2.x deflate backend (rejected: possible through `deflate-flate2` plus `flate2/rust_backend` instead of `deflate` (`zip-9.0.5/Cargo.toml` features), but zip 9's own writer and zopfli choices still differ from 2.4.2, so byte identity is not guaranteed either way, and it keeps a backend the crate moved off). **Rationale:** only unzip tools read these bytes; stored files keep their bytes. **Implemented by:** DR-007. **Evidence:** `zip-2.4.2/Cargo.toml:78-82`.
- **DEC-006:** Raw byte dedup is taken as zip 9 does it (DR-008); a decoded name clash is refused (DR-009). Owner decision 2026-10-18. **Alternatives considered:** reproduce 2.x decoded name dedup (rejected: silently drops an entry the user put in the archive); enable `enforce_header_consistency` to refuse raw duplicates too (rejected here: a wider behavior change, filed as BL-0063); refuse with the destination collision error (rejected: its message tells the user to pass `overwrite=true`, which cannot help). **Rationale:** the affected archives are malformed or contrived; refusing a clash is the safe outcome, writing two entries to one path is unspecified. **Implemented by:** DR-008, DR-009. **Evidence:** `zip-9.0.5/src/read/zip_archive.rs:82-85`, `tools/archive.rs:173-181`.
- **Verified (probe worktree, 2026-10-18):** with the probe lockfile and the current `.cargo/audit.toml`/`deny.toml`, `cargo audit --json` gives `warnings {}` and no vulnerability, and `cargo deny check` ends `advisories ok, bans ok, licenses ok, sources ok`. If a live advisory changes that before implementation, the implementation STOPS (blocked); it never adds an ignore, because `accepted_advisories_are_recorded_once_with_a_reason` (`crates/mcp-fs/tests/security_gate.rs:149`) pins the exact id set.

## 7. Declared breaks
Not pure debt: one observable delta, authorized by DEC-002 (owner, 2026-10-18).
- **Contract:** `fs.extract_archive` and `POST /api/fs/{mount_id}/extract-archive`.
- **Who is affected:** a user extracting a zip whose entry names are UTF-8 without the UTF-8 flag (now readable instead of CP437 mojibake), or flagged UTF-8 with invalid bytes (now CP437 instead of `U+FFFD`).
- **What they must do:** nothing; a file already extracted keeps its stored name.
- **Contract:** zip writers (`fs.export_zip`, REST export-zip, REST directory zip download, docx generation; DEC-005).
- **Who is affected:** a consumer comparing exported zip or generated docx bytes, hashes or sizes across versions, and project quota usage for a newly generated docx.
- **What they must do:** nothing; files stored before the upgrade keep their bytes.
- **Contract:** `fs.extract_archive` entry deduplication (DEC-006).
- **Who is affected:** a user extracting a zip whose entries collide only after name decoding (now refused, DR-009) or only in raw bytes with different flags (now one entry instead of two, DR-008).
- **What they must do:** rebuild the archive with distinct names.

## 8. Tests

### 8.1 Non regression: the success criterion
The existing test suite passes, unmodified except the rows of 8.4:

    MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check

### 8.2 Compatibility tests
#### DT-034: UTF-8 name without the flag extracts readable
- **Validates:** DR-006, DR-005
- **Location:** `crates/core/src/tools/archive.rs` tests, through the same harness the existing `fs.extract_archive` tests use.
- **Given** a zip built in the test whose single stored entry has raw name bytes `C3 A9 2E 74 78 74` (`é.txt`), UTF-8 flag (general purpose bit 11) cleared in both the local file header and the central directory entry by patching the bytes after writing (names are read from the central directory, `zip-9.0.5/src/read/zip_archive.rs:664-690`), content `hello`
- **When** the owner extracts it into `/out`
- **Then** the call succeeds and `/out/é.txt` exists with content `hello`
- **Red before the fix because:** zip 2.4.2 decodes CP437, the file lands at `/out/├⌐.txt`, so `/out/é.txt` is not found.

#### DT-035: invalid UTF-8 flagged name extracts under CP437
- **Validates:** DR-006
- **Location:** same harness.
- **Given** a zip whose single entry has raw name bytes `FF 2E 74 78 74` with the UTF-8 flag set, content `x`
- **When** extracted into `/out`
- **Then** `/out/\u{a0}.txt` exists with content `x` (CP437 `0xFF` is U+00A0)
- **Red before the fix because:** zip 2.4.2 decodes lossy UTF-8, the name is `/out/\u{fffd}.txt`.

#### DT-038: writer output keeps entries and contents
- **Validates:** DR-007
- **Location:** `crates/core/src/docs/docx.rs` tests
- **Given** markdown `"# Hello\n\nbody\n"`
- **When** `render_markdown_to_docx(md, None)` runs
- **Then** the archive opened with `ZipArchive` lists exactly `[Content_Types].xml`, `_rels/.rels`, `word/_rels/document.xml.rels`, `word/document.xml`, `word/styles.xml` in that order, and `word/document.xml` decompresses to the string `build_body(md, None)` returns (`docx.rs:69`)
- **Red before the fix because:** not red; guardrail. Mutation check: dropping one entry from `pack` (`docx.rs:50-56`) makes it fail.

#### DT-039: entry deduplication follows raw bytes, decoded clashes refused
- **Validates:** DR-008, DR-009
- **Location:** `crates/core/src/tools/archive.rs` tests, same harness as DT-034; archives built in the test and patched in both headers
- **Case a:** entries `FF 2E 74 78 74` (content `1`) and `FE 2E 74 78 74` (content `2`), both flagged UTF-8. **Then** extraction succeeds with exactly 2 files, `/out/\u{a0}.txt` = `1` and `/out/\u{25a0}.txt` = `2`. **Red before because:** 2.x decodes both to `\u{fffd}.txt` and yields 1 file.
- **Case b:** raw `C3 A9 2E 74 78 74` twice, first flagged (content `first`), second unflagged (content `second`). **Then** exactly 1 file, `/out/é.txt` = `second`. **Red before because:** 2.x yields 2 files (`é.txt`, `├⌐.txt`).
- **Case c:** raw `C2 A0 2E 74 78 74` unflagged and `FF 2E 74 78 74` flagged, both decoding to `\u{a0}.txt`. **Then** `ERR_INVALID_ARGUMENT`, message `archive is corrupt or not a valid zip file: duplicate entry name '\u{a0}.txt'`, **And** `fs.list_dir /` (second channel) shows no `out`. **Red before because:** 2.x yields 2 distinct files.

**Raw name construction (DT-034, DT-035, DT-039):** write each entry with an ASCII placeholder name of the same byte length (`X.txt` for 5 bytes, `XX.txt` for 6) and `SimpleFileOptions::default()`; then overwrite the placeholder bytes in both the local file header (name at offset 30) and the central directory entry (name at offset 46) with the target raw bytes, and set or clear general purpose bit 11 (local offset 6, central offset 8) as the case states. One helper `zip_with_raw_names(&[(&[u8], bool, &[u8])])` in the archive tests module serves all three.

### 8.3 Structural and security tests
#### DT-030: lockfile and graph carry zip 9 only for normal dependencies
- **Validates:** DR-001
- **Location:** `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** Cargo.lock lists exactly two zip versions, `3.0.0` and one starting with `9.`; `cargo tree -i zip@2.4.2 --all-features` exits non zero and its stderr contains `did not match any packages` (observed on the probe, 2026-10-18); every line of `cargo tree --workspace --all-features -e normal --prefix none` that starts with `zip v` starts with `zip v9.`, and at least one such line exists
- **Red before the fix because:** Cargo.lock lists `2.4.2`.

#### DT-031: no zip 2.4.2 citation
- **Validates:** DR-003
- **Location:** `crates/mcp-fs/tests/security_gate.rs`
- **Method:** test reads `crates/core/src/tools/archive.rs` text and asserts it does not contain `zip-2.4.2`.
- **Red before the fix because:** `:288`, `:289`, `:380`, `:381`, `:1120`, `:1626` contain it.

#### DT-036: acceptance narrowed and gate green
- **Validates:** DR-004
- **Location:** `crates/mcp-fs/tests/security_gate.rs`
- **Asserts:** in `.cargo/audit.toml` and `deny.toml`, each line holding RUSTSEC-2026-0367, 0368 or 0369 contains `utoipa-swagger-ui` and `BL-0059` and not `BL-0058`; neither file contains the string `BL-0058` anywhere; the existing `make_security_passes` stays green.
- **Red before the fix because:** `.cargo/audit.toml:13-15` read `until BL-0058` and `deny.toml:21-23` lack `utoipa-swagger-ui`.

#### DT-037: guardrail
- **Validates:** DR-005
- The 8.1 command green; `git diff main -- tool-contract-golden.json` empty.

### 8.4 Existing tests to modify
Each change only adapts to `name()` / `file_names()` returning a `Result` (append `.unwrap()` before the existing conversion, or compare `.unwrap()` with the same literal); every asserted value is unchanged. Justification for all rows: the 9.x API returns `ZipResult<Cow<str>>`, so the old expression does not compile (probe, Section 2).

| Test file | Line | Change |
|---|---|---|
| `crates/core/src/api/dataplane.rs` | 2510 | `.name().unwrap().to_string()` |
| `crates/core/src/api/dataplane.rs` | 2639 | compare `entry.name().unwrap()` |
| `crates/core/src/api/dataplane.rs` | 2911 | `file_names().map(Result::unwrap)` with owned or borrowed `&str` as the asserted vector needs |
| `crates/core/src/docs/docx.rs` | 378 | `.name().unwrap().to_string()` |
| `crates/core/src/exports.rs` | 249 | `.name().unwrap().to_string()` |
| `crates/core/src/tools/export.rs` | 286 | `.name().unwrap().to_string()` |
| `crates/core/src/tools/archive.rs` | 952 | compare `.name().unwrap()` |
| `crates/core/src/tools/archive.rs` | 1287 | compare `.name().unwrap()` with `"link"` |
| `crates/core/src/tools/archive.rs` | 1291 | compare `.name().unwrap()` with `"good.txt"` |
| `crates/core/src/tools/artifacts.rs` | 4154 | `.name().unwrap().to_string()` |

Any other existing test that fails after the bump is a finding, not an edit.

### 8.5 Security tests
These occurrences were never exploitable here (Section 3), so the two extraction tests are guardrails pinning the controls across the upgrade; the red before signal for the advisories themselves is DT-030.

#### DT-032: symlink plus traversal archive refused, nothing written
- **Covers:** occurrence 1, `tools/archive.rs:398` and `:403`
- **Driver:** direct call to the `fs.extract_archive` engine through the existing archive test harness
- **Given** a zip with entries, in order: `ok.txt` (content `fine`), `link` as a symlink entry (unix mode `0o120777`) whose target is `../../../etc/passwd`, `../../evil.txt` (content `pwned`)
- **When** the owner extracts it into `/out`
- **Then** the call fails with `ERR_NOT_SUPPORTED` and message `archive entry 'link' is a symlink, which is not supported`
- **And** a directory listing of `/` (second channel) shows no `/out`, and no node named `evil.txt` or `ok.txt` exists anywhere in the volume (glob `**/*.txt` returns none)
- **Red before the fix because:** not red; guardrail (controls predate this lot). Its mutation check: removing the `is_symlink` branch makes it fail on the error code.

#### DT-033: setuid mode bits are not carried onto the stored file
- **Covers:** occurrence 1, mode handling (RUSTSEC-2026-0367 class)
- **Given** a zip built in the test with one regular entry `tool.sh`, content `#!/bin/sh`, written with `SimpleFileOptions::default()`; the test then patches the central directory entry's external attributes to `0o104755 << 16` and its "version made by" high byte to `3` (Unix), because the writer masks modes to `0o777` (`zip-9.0.5/src/write/options.rs:334-335`); before extraction it asserts `ZipArchive::new(..).by_index_raw(0).unwrap().unix_mode() == Some(0o104755)`, proving the setuid bit is in the input
- **When** extracted into `/out`
- **Then** the call succeeds
- **And** `fs.stat /out/tool.sh` (second channel) reports `mode` `0o644`, the mode of a file written by `fs.write` (`core/fs_ops.rs:251`, asserted at `:2338`)
- **Red before the fix because:** not red; guardrail. Mutation check: copying the entry's unix mode onto the node makes it fail.

## 9. Implementation order
1. Write DT-030, DT-031, DT-034, DT-035, DT-036 on zip 2 and record RED; write DT-039 and record RED; write DT-032, DT-033, DT-038 and record their mutation RED.
2. `Cargo.toml:87` to `"9"`, `cargo update -p zip@2.4.2`; the two production edits (DR-002) plus the DR-009 refusal and the 10 test adaptations (8.4) in the same step, since nothing compiles in between.
3. Comments (DR-003), audit and deny reasons (DR-004).
4. `make check` (8.1).
One story suffices; the build is broken between the bump and the adaptations, so they cannot be split.

## 10. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 2 | 4 | NOT-IMPLEMENTABLE: F-1, F-2 rested on header checks that are off by default (`zip-9.0.5/src/read/mod.rs:251`, `:270`), reclassified A, the false claim removed from Sections 2, 5, 6, 7; A-1 (DT-030 command), A-2 (deny.toml count, DT-036 red cause), A-3 (DT-031 location) applied; A-4 rejected, `archive.rs:351` verified |
| 2 | 3 | 2 | NOT-IMPLEMENTABLE: F-1 DT-033 input could not carry setuid (writer masks to 0o777), now patched and asserted; F-2, F-3 sweep rows 4 to 6 added (test readers renumbered 7), class widened to every archive format, search 4 added; A-1 DT-030 assertion, A-2 OOXML slide prefix match, applied |
| 3 | 2 | 3 | NOT-IMPLEMENTABLE: F-1 undeclared writer byte change, resolved by a scope decision (DEC-005, DR-007, Section 7); F-2 DT-038 added; A-1 error count, A-2 DT-034 both headers, A-3 non zip `name()` exclusions applied |
| 4 | 1 | 1 | NOT-IMPLEMENTABLE: F-1 entry deduplication moves from decoded name (2.x) to raw bytes (`zip-9.0.5/src/read/zip_archive.rs:82-85`), undeclared; A-1 applied. STOPPED: second consecutive round of only new F findings, scope decision needed |
| 5 | | | after owner decision (DEC-006): DR-008, DR-009, DT-039 added; audit F-1 (check order) and F-2 (probe audit run, verified clean) applied as precision, A-1, A-2 applied |
| 6 | 0 | 2 | IMPLEMENTABLE: A-1 raw name construction helper, A-2 lockfile set informative, applied |

## Design additions (2026-10-18)

### File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `Cargo.toml`, `Cargo.lock` | zip `"2"` to `"9"` | DR-001 | US-0042 |
| `crates/core/src/docs/extract.rs` | `:553` Result adaptation | DR-002 | US-0042 |
| `crates/core/src/tools/archive.rs` | `:397` adaptation, DR-009 refusal in `:395-410`, tests `:952`, `:1287`, `:1291`, new DT-032..035, DT-039, helper `zip_with_raw_names` | DR-002, DR-006, DR-008, DR-009 | US-0042 |
| `crates/core/src/api/dataplane.rs`, `docs/docx.rs`, `exports.rs`, `tools/export.rs`, `tools/artifacts.rs` | 8.4 test adaptations; DT-038 in `docx.rs` | DR-005, DR-007 | US-0042 |
| `crates/core/src/tools/archive.rs` | 6 `zip-2.4.2` comment citations | DR-003 | US-0043 |
| `.cargo/audit.toml`, `deny.toml` | reasons name utoipa-swagger-ui and BL-0059 | DR-004 | US-0043 |
| `crates/mcp-fs/tests/security_gate.rs` | DT-030 (US-0042), DT-031, DT-036 (US-0043) | DR-001, DR-003, DR-004 | both |

### Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| DT-030 | structural | lockfile text + `cargo tree` | `security_gate.rs` | US-0042 |
| DT-032, DT-033, DT-034, DT-035, DT-039 | integration | `fs.extract_archive` engine, archive test harness | `tools/archive.rs` tests | US-0042 |
| DT-038 | unit | `render_markdown_to_docx` | `docs/docx.rs` tests | US-0042 |
| DT-031, DT-036 | structural | file text, `make security` | `security_gate.rs` | US-0043 |
| DT-037 | guardrail | full suite | `make check` | US-0043 |

### Additional decisions
- **DEC-007:** Two stories: (1) bump plus every compile forced edit plus behavior tests, (2) comments and audit reasons. **Obliged by:** SPEC-0036/DR-001..DR-009. **Rationale:** Section 9 step 2 cannot be split (nothing compiles in between); steps 3 and 4 can follow with main green, since DT-036 and DT-031 are only added in story 2 and the old reasons keep `make security` green. **Alternatives considered:** one story of 12 paths (above every tier budget). **Source:** design. **Evidence:** Section 9.
- **DEC-008:** US-0042 at `min_tier: 1`: 10 paths and 7 new tests including byte patched archives exceed the tier 2 file budget, and cannot be split. US-0043 at `min_tier: 2`. **Obliged by:** SPEC-0036/DR-001, DR-002, DR-009. **Alternatives considered:** tier 2 for both (file budget broken). **Source:** design. **Evidence:** table above.

### Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0042 | zip 9 with declared decoding, dedup and writer changes | P1 | DR-001, DR-002, DR-005, DR-006, DR-007, DR-008, DR-009 | main | 1 | no |
| US-0043 | narrow the zip acceptances and drop stale citations | P1 | DR-003, DR-004, DR-005 | US-0042 | 2 | yes |
