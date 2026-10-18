# US-0042: zip 9 with declared decoding, dedup and writer changes

> Spec: SPEC-0036 (sdlc/specs/SPEC-0036-move-zip-to-9/)
> Nature: DEBT
> Priority: P1
> Depends on: main
> min_tier: 1
> Files touched: 10
> Closes spec: no

## Objective
Move the direct zip dependency from 2.4.2 to 9.x (RUSTSEC-2026-0367/0368/0369), adapt the two production readers and ten existing tests to `name()`/`file_names()` returning `ZipResult<Cow<str>>`, refuse decoded name clashes, and pin the declared breaks with tests.

## Requirements covered
- SPEC-0036/DR-001: The workspace SHALL depend directly on `zip` 9.x with features `deflate` and `aes-crypto` and default features off, and Cargo.lock SHALL contain no zip release below 9 among normal (non build) dependencies.
- SPEC-0036/DR-002: The two production readers (`docs/extract.rs` entry listing, `tools/archive.rs` pre scan) SHALL obtain entry names through the `Result` returning `name()` / `file_names()` and SHALL map an `Err` to `corrupt("zip", e)` in `tools/archive.rs`, and omission of that name from the listing in `docs/extract.rs`.
- SPEC-0036/DR-005 (partial: this story's diff): byte identical observable output for every input the existing suite covers, except DR-006, DR-007, DR-008; `tool-contract-golden.json` unchanged.
- SPEC-0036/DR-006: IF a zip entry name is valid UTF-8 THEN `fs.extract_archive` SHALL write it under its UTF-8 decoding whatever the flag, and IF it is not valid UTF-8 THEN under its CP437 decoding whatever the flag.
- SPEC-0036/DR-007: WHEN `fs.export_zip`, REST export-zip, the `/api/fs` directory zip download or docx generation produces an archive THE workspace SHALL keep entry names, entry order and decompressed contents identical; compressed bytes, size and hash are outside DR-005.
- SPEC-0036/DR-008: IF two central directory entries have identical raw name bytes THEN `fs.extract_archive` SHALL extract only the entry zip 9 keeps (the last one), whatever their UTF-8 flags.
- SPEC-0036/DR-009: IF two entries with distinct raw name bytes decode to the same name THEN the pre scan SHALL fail with `ERR_INVALID_ARGUMENT` and message `archive is corrupt or not a valid zip file: duplicate entry name '<name>'` (through `corrupt("zip", ..)`), before any write. The check runs inside the pre scan loop for entry i, after `ensure_entry_path_safe(&raw_path)?` and before `entries.push`; a `HashSet` of decoded names, O(n).

## Independent Test
`cargo test -p mcp-fs-core --lib tools::archive` passes DT-032..DT-035 and DT-039, `cargo test -p mcp-fs --test security_gate` passes DT-030, `cargo test -p mcp-fs-core --lib docs::docx` passes DT-038; then the non regression command.

## Design excerpt
### Decisions
- DEC-001: zip 9.x, not 8.x (patched `>= 9.0.0`).
- DEC-002: declare the name decoding change, do not reimplement 2.x decoding; header consistency checks stay off (default).
- DEC-004: an `Err` from `name()` maps to existing error paths, never `unwrap` in production (unreachable in 9.0.5, `zip-9.0.5/src/cp437.rs:17-27`).
- DEC-005: accept compressed byte changes of the three writers.
- DEC-006: raw byte dedup as zip 9 does; decoded clash refused with the corrupt error, not the collision error (its message suggests `overwrite=true`).
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | `:87` `version = "2"` to `"9"`, features unchanged |
| `Cargo.lock` | `cargo update -p zip@2.4.2`; any resolved set accepted if DT-030 and `make security` pass on Rust 1.88 |
| `crates/core/src/docs/extract.rs` | `:553` `zip.file_names().filter_map(Result::ok).map(Cow::into_owned).collect()` |
| `crates/core/src/tools/archive.rs` | `:397` `file.name().map_err(\|e\| corrupt("zip", e))?.into_owned()`; DR-009 check; tests |
| `crates/core/src/api/dataplane.rs`, `docs/docx.rs`, `exports.rs`, `tools/export.rs`, `tools/artifacts.rs` | test adaptations below; DT-038 in `docx.rs` |
| `crates/mcp-fs/tests/security_gate.rs` | DT-030 |
### Existing patterns
- Archive tests harness and existing extraction tests: `crates/core/src/tools/archive.rs` tests module from `:679` (e.g. `e2e_new_018_zip_symlink_entry_is_not_supported` `:1249`, writer helper `:1117-1140`).
- Lockfile parsing helper `lock_packages` and `cargo tree` pattern: `crates/mcp-fs/tests/security_gate.rs:41-95`.
- `corrupt`: `tools/archive.rs:334-337`. `fs.stat` mode `0o644`: `core/fs_ops.rs:251`, asserted `:2338`.
- **Raw name construction (DT-034, DT-035, DT-039):** write each entry with an ASCII placeholder of the same byte length (`X.txt` 5 bytes, `XX.txt` 6) and `SimpleFileOptions::default()`; overwrite the placeholder bytes in the local file header (name at offset 30) and the central directory entry (name at offset 46); set or clear general purpose bit 11 (local offset 6, central offset 8). One helper `zip_with_raw_names(&[(&[u8], bool, &[u8])])`.

## Acceptance tests
> 100% must pass, run through `make check`.

### SPEC-0036/DT-030: lockfile and graph carry zip 9 only for normal dependencies
- Location: `crates/mcp-fs/tests/security_gate.rs`
- Asserts: Cargo.lock lists exactly two zip versions, `3.0.0` and one starting with `9.`; `cargo tree -i zip@2.4.2 --all-features` exits non zero and stderr contains `did not match any packages`; every line of `cargo tree --workspace --all-features -e normal --prefix none` starting with `zip v` starts with `zip v9.`, and at least one exists.
- Red before because: Cargo.lock lists `2.4.2`.

### SPEC-0036/DT-032: symlink plus traversal archive refused, nothing written
- Given a zip with `ok.txt` (`fine`), `link` symlink entry (mode `0o120777`) targeting `../../../etc/passwd`, `../../evil.txt` (`pwned`) / When the owner extracts into `/out` / Then `ERR_NOT_SUPPORTED`, message `archive entry 'link' is a symlink, which is not supported` / And `fs.list_dir /` shows no `out`, glob `**/*.txt` returns none.
- Guardrail; mutation RED: removing the `is_symlink` branch fails it on the error code.

### SPEC-0036/DT-033: setuid mode bits are not carried onto the stored file
- Given a zip with regular entry `tool.sh` (`#!/bin/sh`) written with defaults, then central directory external attributes patched to `0o104755 << 16` and "version made by" high byte `3`; the test first asserts `ZipArchive::new(..).by_index_raw(0).unwrap().unix_mode() == Some(0o104755)` / When extracted into `/out` / Then success / And `fs.stat /out/tool.sh` reports mode `0o644`.
- Guardrail; mutation RED: copying the entry mode onto the node fails it.

### SPEC-0036/DT-034: UTF-8 name without the flag extracts readable
- Given one entry raw name `C3 A9 2E 74 78 74`, bit 11 cleared in both headers, content `hello` / When extracted into `/out` / Then `/out/é.txt` = `hello`.
- Red before because: 2.4.2 decodes CP437, file lands at `/out/├⌐.txt`.

### SPEC-0036/DT-035: invalid UTF-8 flagged name extracts under CP437
- Given one entry raw name `FF 2E 74 78 74`, bit 11 set, content `x` / Then `/out/\u{a0}.txt` = `x`.
- Red before because: 2.4.2 gives `/out/\u{fffd}.txt`.

### SPEC-0036/DT-038: writer output keeps entries and contents
- Location: `crates/core/src/docs/docx.rs` tests
- Given markdown `"# Hello\n\nbody\n"` / When `render_markdown_to_docx(md, None)` / Then entries are exactly `[Content_Types].xml`, `_rels/.rels`, `word/_rels/document.xml.rels`, `word/document.xml`, `word/styles.xml` in order, and `word/document.xml` equals `build_body(md, None)`.
- Guardrail; mutation RED: dropping one entry from `pack` fails it.

### SPEC-0036/DT-039: dedup follows raw bytes, decoded clashes refused
- Case a: `FF 2E 74 78 74` (`1`) and `FE 2E 74 78 74` (`2`), both flagged. Then 2 files, `/out/\u{a0}.txt` = `1`, `/out/\u{25a0}.txt` = `2`. Red before: 1 file.
- Case b: `C3 A9 2E 74 78 74` flagged (`first`) then unflagged (`second`). Then 1 file, `/out/é.txt` = `second`. Red before: 2 files.
- Case c: `C2 A0 2E 74 78 74` unflagged and `FF 2E 74 78 74` flagged. Then `ERR_INVALID_ARGUMENT`, message `archive is corrupt or not a valid zip file: duplicate entry name '\u{a0}.txt'`, and `fs.list_dir /` shows no `out`. Red before: 2 files.

## Constraints
- Files not to touch: `.cargo/audit.toml`, `deny.toml`, the `zip-2.4.2` comments (US-0043), `tool-contract-golden.json`.
- Dependencies not to add: anything but what `cargo update -p zip@2.4.2` resolves; never `enforce_header_consistency(true)` (BL-0063).
- Never add an audit ignore: if `make security` goes red on a new package, STOP (blocked).
- No `unwrap` on `name()` in production code.

## Definition of done
- [ ] RED evidence recorded (DT-030, DT-034, DT-035, DT-039 assertion RED on zip 2; DT-032, DT-033, DT-038 mutation RED).
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: 10, each only adapting to the `Result` return with asserted values unchanged: `api/dataplane.rs:2510`, `:2639`, `:2911`; `docs/docx.rs:378`; `exports.rs:249`; `tools/export.rs:286`; `tools/archive.rs:952`, `:1287`, `:1291`; `tools/artifacts.rs:4154`. Any other failing existing test is a finding, not an edit.
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] One commit `US-0042: zip 9 with declared decoding, dedup and writer changes`, body `Spec: SPEC-0036` and the DR ids.
