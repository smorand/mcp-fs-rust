# US-0171: refuse unsafe RAR entries, oversized entries and collisions

> Spec: SPEC-0080 (sdlc/specs/SPEC-0080-extract-rar-archives/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0170
> min_tier: 2
> Files touched: 3
> Closes spec: no

## Objective
A RAR archive holding a symbolic link, a name that escapes the destination, or an entry whose data does not match its declared size is refused whole with nothing written; an existing file is refused unless `overwrite`.

## Requirements covered
- SPEC-0080/FR-NEW-006: IF a RAR archive holds a symbolic link entry THEN THE archive extraction SHALL refuse the whole archive with `ERR_NOT_SUPPORTED`, name the offending entry in the message, and write nothing. A hard link entry is extracted as an empty regular file (SPEC-0080 erratum 2).
- SPEC-0080/FR-NEW-007: IF a RAR entry name, once resolved, lies outside the destination, whether through `..` or an absolute name, THEN THE archive extraction SHALL refuse the whole archive with `ERR_PATH_OUT_OF_BOUNDS` and write nothing.
- SPEC-0080/FR-NEW-008: IF the data of a RAR entry does not match its declared size and checksum THEN THE archive extraction SHALL refuse the whole archive with `ERR_INVALID_ARGUMENT`, name the offending entry in the message, and write nothing.
- SPEC-0080/FR-NEW-009: IF an entry targets a path that already holds a file AND `overwrite` is false THEN THE archive extraction SHALL refuse with `ERR_NO_CLOBBER` naming the first colliding path in archive order, and write nothing.

## Independent Test
Run `cargo test -p mcp-fs-core --lib --features rar -- e2e_0080_008 e2e_0080_009 e2e_0080_010 e2e_0080_012 e2e_0080_019 e2e_0080_032 e2e_0080_033 e2e_0080_034 e2e_0080_035 e2e_0080_036 e2e_0080_037 e2e_0080_044`: all 12 tests pass. Refused calls leave the store without the destination folder, read back with `client.exists`.

## Design excerpt
### Decisions
- **DEC-004:** `decode_rar` runs in this order, matching FR-NEW-015: open for listing (with the password if given); a `MissingPassword` or `BadPassword` at `When::Read` means encrypted names, mapped by `password_required_message(password)`; other open or header errors map to `corrupt("rar", e)` (item 5); entry rules in archive order, link or device then escaping name (item 6); then if any entry `is_encrypted()` and no password was given, `password_required_message(None)` (item 7, nothing read yet, so nothing partial); then each file entry is read in order (solid archives need order); a `BadPassword` or `BadData` on an encrypted entry maps to `password_required_message(password)`; a read size different from `unpacked_size` maps to `ERR_INVALID_ARGUMENT` naming the entry (item 8). An empty `Some("")` password on an encrypted archive is treated as given, so the message is the wrong password one. **Obliged by:** SPEC-0080/FR-NEW-002 to FR-NEW-008, FR-NEW-015, FR-NEW-016, FR-NEW-020. **Rationale:** the pipeline then continues with destination, collision, admission, which are items 9 to 12. **Alternatives considered:** read first, check entries after (violates FR-NEW-015 item 6 over 7). **Source:** spec. **Evidence:** spec FR-NEW-015, `archive.rs:154-205`.
- **DEC-005:** a lying declared size (header smaller than the data) is detected as a read error or size mismatch and reported as `ERR_INVALID_ARGUMENT` `archive entry '<name>' decompressed to more bytes than declared <declared>` (same wording as `archive.rs:339-347` where the count is known, the declared size alone where it is not). **Obliged by:** SPEC-0080/FR-NEW-008. **Rationale:** the C library stops at the declared size. **Alternatives considered:** none that stays safe. **Source:** design. **Evidence:** `archive.rs:325-347`.
- **DEC-006:** entry kinds: `FileHeader::is_directory()` is a folder; a Unix symlink is `file_attr & 0o170000 == 0o120000`; a Windows reparse point is `file_attr & 0x400 != 0`; both are refused with the symlink wording. A RAR5 hard link and a device entry cannot be seen through the safe API; US-0171 starts with a spike on the E2E-008 and E2E-032 fixtures; if the spike shows a hard link extracts as an ordinary file, the story ends BLOCKED_ON_SPEC with an entry in `design-issues.md` (options: accept raw FFI via `unrar_sys`, which needs `unsafe`, or amend E2E-032). **Obliged by:** SPEC-0080/FR-NEW-006. **Rationale:** no `unsafe` in this project. **Alternatives considered:** raw `RARReadHeaderEx` via `unrar_sys` (unsafe); `unrar-ng` (same limit). **Source:** design. **Evidence:** `unrar-0.5.8/src/open_archive.rs:582-640`, `unrar-ng-0.7.7/src/open_archive.rs:929-937`, `unrar_sys-0.5.8/src/lib.rs:163-166`.
- **DEC-008:** the shared pipeline stays as is: collision scan, `admit_paths` (project net growth first, session gross after), writes. No pre decode admission. **Obliged by:** SPEC-0080/FR-NEW-009, FR-NEW-014, FR-NEW-022. **Rationale:** the refusal outcomes are the same; admitting before decoding is a hardening outside this spec (BL-0125). **Alternatives considered:** pre decode admission on declared sizes. **Source:** existing code. **Evidence:** `archive.rs:183-205`, `quota.rs:96-158`.
### Data model
None.
### Contracts
Entry rules in `decode_rar` (archive order, before any read): folder = `is_directory()`; symlink = `file_attr & 0o170000 == 0o120000` or `file_attr & 0x400 != 0`, refused with `ERR_NOT_SUPPORTED` `archive entry '<name>' is a symlink, which is not supported`; hard link: extracted as an empty file (SPEC-0080 erratum 2); devices are not classified; escaping name: `ensure_entry_path_safe(&name)` (`archive.rs:359-385`) gives `ERR_PATH_OUT_OF_BOUNDS`; size: a data read whose length differs from `unpacked_size`, or a read error on a plain entry, gives `ERR_INVALID_ARGUMENT` `archive entry '<name>' decompressed to more bytes than declared <declared>`; collision: the existing pipeline scan (`archive.rs:166-176`). SPIKE FIRST: build the E2E-008, E2E-032 and E2E-012 fixtures with the store builder (symlink redirection record, hard link redirection record, a header declaring 10 bytes over 1 000 000 bytes of data) and see what the safe `unrar` API reports; if the hard link or the oversize case cannot be observed, stop BLOCKED_ON_SPEC and write `sdlc/specs/SPEC-0080-extract-rar-archives/design-issues.md` (options: raw FFI through `unrar_sys`, which needs `unsafe`; or amend E2E-032 and E2E-012).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify |
| `crates/core/src/tools/archive_rar_fixtures.rs` | modify |
| `crates/core/src/tools/archive.rs` | modify |
### Existing patterns
`ensure_entry_path_safe` (`archive.rs:359-385`), the tar entry type refusal (`archive.rs:670-674`), `ensure_decoded_size_matches` (`archive.rs:339-347`), the collision scan (`archive.rs:166-176`). Test helpers: `Fixture::with_config`, `seed_project`, `stores.client(MOUNT)` as in `fixture()` and `extract()` at `crates/core/src/tools/archive.rs:721-735`; the typed call is `extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password)`; a test that checks membership first calls `f.state.authorize(MOUNT, person)` before it, as both doors do.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

### SPEC-0080/E2E-008: Symbolic link entry
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-006
- **Given** `/uploads/links.rar` holding `a.txt` and a symbolic link `l` pointing to `/etc/passwd`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED` and the message names the entry `l`
- **And** `/uploads/links` does not exist
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_008_symbolic_link_entry`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-009: Entry named `../../etc/passwd`
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-007
- **Given** `/uploads/evil.rar` holding the entry `../../etc/passwd` containing `pwned`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`
- **And** read through the project's file listing: no `/etc/passwd` and no `/uploads/etc` exist, and `/uploads/evil` does not exist
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_009_entry_named_etc_passwd`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-010: Collision names the first colliding path
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-009
- **Given** `/uploads/report.rar` as in E2E-001, and `/uploads/report/data/a.txt` and `/uploads/report/data/b.txt` already existing
- **When** the member extracts with overwrite false
- **Then** the refusal is `ERR_NO_CLOBBER` naming `/uploads/report/data` (the first colliding path in archive order)
- **And** both existing files keep their content
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_010_collision_names_the_first_colliding`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-012: Entry that decodes larger than declared
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-008
- **Given** `/uploads/bomb.rar` whose entry `big.bin` declares 10 bytes and decodes to 1 000 000 bytes
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` and the message names `big.bin`
- **And** `/uploads/bomb` does not exist, and the remaining storage and the session write allowance are unchanged
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_012_entry_that_decodes_larger_than_decla`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-019: Overwrite true replaces
- **Category:** edge; **Scenario:** SC-001; **Requirements:** FR-NEW-009
- **Given** `/uploads/report.rar` as in E2E-001 and `/uploads/report/data/a.txt` containing `old`
- **When** the member extracts with overwrite true
- **Then** the answer reports 2 files written and `/uploads/report/data/a.txt` reads `hello`
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_019_overwrite_true_replaces`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-032: Hard link entry extracts as an empty file
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-006
- **Given** `/uploads/hl.rar` holding `a.txt` and a hard link `b.txt` to it
- **When** the member extracts it
- **Then** the extraction succeeds, `/uploads/hl/a.txt` has its content and `/uploads/hl/b.txt` exists with 0 bytes
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_032_hard_link_entry`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-033: Link after valid entries writes nothing
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-006
- **Given** `/uploads/mix.rar` holding `ok1.txt`, `ok2.txt`, then a symbolic link `l`
- **When** the member extracts it
- **Then** the refusal is `ERR_NOT_SUPPORTED`; `/uploads/mix` does not exist, so neither `ok1.txt` nor `ok2.txt` was written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_033_link_after_valid_entries_writes_noth`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-034: Absolute entry name
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-007
- **Given** `/uploads/abs.rar` holding the entry `/etc/cron.d/x` containing `* * * * * root id`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`
- **And** `/etc/cron.d/x` does not exist in the project and `/uploads/abs` does not exist
- **Priority:** Critical
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_034_absolute_entry_name`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-035: Traversal in the middle of a name
- **Category:** failure, security; **Scenario:** SC-003; **Requirements:** FR-NEW-007
- **Given** `/uploads/mid.rar` holding `a/../../b.txt` containing `x`
- **When** the member extracts it
- **Then** the refusal is `ERR_PATH_OUT_OF_BOUNDS`; `/uploads/b.txt` and `/b.txt` do not exist
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_035_traversal_in_the_middle_of_a_name`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-036: One oversized entry among honest ones
- **Category:** failure; **Scenario:** SC-003; **Requirements:** FR-NEW-008
- **Given** `/uploads/one.rar` with `ok.txt` honest and `liar.bin` declaring 10 bytes and decoding to 11
- **When** the member extracts it
- **Then** the refusal is `ERR_INVALID_ARGUMENT` naming `liar.bin`; `ok.txt` was not written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_036_one_oversized_entry_among_honest_one`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-037: Declared 10 bytes, decoded 10 bytes
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-008
- **Given** `/uploads/exact.rar` with `exact.bin` declaring 10 bytes and decoding to 10 bytes
- **When** the member extracts it
- **Then** 1 file and 10 bytes are written
- **Priority:** High
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_037_declared_10_bytes_decoded_10_bytes`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

### SPEC-0080/E2E-044: Collision under an explicit destination
- **Category:** edge; **Scenario:** SC-003; **Requirements:** FR-NEW-009
- **Given** `/uploads/report.rar` as in E2E-001 and `/out/data/b.txt` already existing
- **When** the member extracts it to the destination `/out` with overwrite false
- **Then** the refusal is `ERR_NO_CLOBBER` naming `/out/data`; `/out/data/a.txt` does not exist
- **Priority:** Medium
- Level / Driver / Location: functional e2e / typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership / crates/core/src/tools/archive_rar.rs tests, `#[cfg(feature = "rar")]`
- Function: `e2e_0080_044_collision_under_an_explicit_destinat`
- RED before the change: the test fails (the behavior does not exist yet) and goes green with this story
- Cleanup: none, the fixture is dropped with the in memory store

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt` (the tool contract does not change)
- Dependencies not to add: none; no `unsafe` (a need for it is a spec issue, not a decision of this story)
- Scope boundary: limits, passwords, multi volume, order belong to later stories

## Definition of done
- [ ] RED evidence recorded for every new test not listed as green on arrival, before the production code.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: `cargo test -p mcp-fs-core --lib tools::archive` passes unchanged except the tests this story lists as modified; no change to the tool contract (`tool-contract-golden.json`, `TOOL_CONTRACT.txt`).
- [ ] DEBT only: not applicable (FEAT).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible paths; no dash as punctuation in code, comments or docs; failure modes of the new code listed in the commit body.
- [ ] Does not close its spec.
- [ ] One commit `US-0171: refuse unsafe RAR entries, oversized entries and collisions`, body `Spec: SPEC-0080` and the requirement ids.

## Resolved

The three spec questions of the first run were settled with the recommended options and recorded as SPEC-0080 erratum 2 (hard link extracts as an empty file; FR-NEW-008 reworded to a size and checksum mismatch; E2E-010 and E2E-044 name the colliding folder). Devices are not classified: the Windows attribute bits 0x1000 and 0x2000 alias the Unix FIFO and character types. A Windows reparse flag counts as a symlink unless the type bits say regular file or folder.
