# US-0242: keep the damage note on the stored text copy

> Spec: SPEC-0094 (sdlc/specs/SPEC-0094-mark-damaged-pdf-page-content/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0241
> min_tier: 1
> Files touched: 7
> Closes spec: no

## Objective
A repeated extraction of a PDF is answered from the stored text copy and must carry the same damage note as the extraction that stored it, without reading or decoding the PDF. The note is recorded with the conversion's artifact set and read back by the hash of the stored text; the refresh option re-examines the PDF.

## Requirements covered
- SPEC-0094/FR-NEW-005: WHEN a member extracts the text of a PDF and the answer comes from its stored text copy, THE text extraction SHALL include in that answer a note equal to the note of the extraction that stored the copy: the damage note, or the empty text. The note is present in the answer even when it is empty.
- SPEC-0094/FR-NEW-006: WHEN a member extracts the text of a PDF with the refresh option, THE text extraction SHALL read the PDF again and answer exactly as for a first extraction, damage note included.
- SPEC-0094/FR-NEW-003 (partial: the cached answer of a clean PDF has an empty note): THE text extraction SHALL NOT write a damage note for a PDF in which every page content is complete, nor for any document that is not a PDF.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- e2e_0094_002 e2e_0094_004 e2e_0094_007 e2e_0094_008 e2e_0094_020` (5 e2e tests) and the store conformance for the note (`cargo test -p mcp-fs-core --lib -- storage::conformance`, SQLite engine; PostgreSQL with `MCPFS_TEST_PG_DSN` set when available). Real assertions: the cached answer's `note` key and value, `cached` true or false, and the stored row read back by `current_set_note`. Also `env -u MCPFS_TEST_PG_DSN cargo test -p mcp-fs-core --lib -- e2e_0089_ e2e_0092_ --test-threads=1` (a cache hit still decodes nothing: `e2e_0089_020`, `e2e_0092_011`).

## Design excerpt
### Decisions
- **DEC-005 (SPEC-0094):** persist the note in a nullable `note` column of `doc_artifact_sets`, written by `commit_artifact_set` in the same transaction as the set and its pointer. Rejected: re reading and walking the PDF on every cache hit (`e2e_0089_020` forbids decoding on a hit), writing the note into the stored text, a sidecar file, a new table, reusing `report`.
- **DEC-006 (SPEC-0094):** on a `.pdf` cache hit, hash the companion text just read (`VolumeClient::sha256_hex`, `storage/volume.rs:48`) and call `current_set_note(path, sha)`; answer `note` as that text or `""`; no row, a different hash or `client.trash == None` gives `""`. Other extensions are not looked up and get no `note` key.
- **DEC-007 (SPEC-0094):** `FreshExtraction::stored_note()` returns `Some(note)` only when the format is `pdf` and `md_path()` is `Some` (non empty text, so the note is the damage note, never the scanned message), else `None`.
### Data model
`doc_artifact_sets` gains `note TEXT NULL`. Declared in the table (`storage/meta.rs:237-253`, `Column::new("note", ColumnType::Text)`) and as a `ColumnMigration { table: "doc_artifact_sets", column: "note", ty: ColumnType::Text, not_null: false, default: "NULL" }` after the `nodes.rev` one (`meta.rs:314-320`). Rollback: additive, nullable; an older binary never names it. `migrate.rs:84-90` copies by declared columns.
### Contracts
- `NewArtifactSet.note: Option<String>` (`meta.rs:388`), inserted as the tenth column of the `INSERT INTO doc_artifact_sets` at `meta.rs:893` (placeholders `?1..?10`, `.bind(set.note.clone())`).
- `RelationalMetaStore::current_set_note(&self, path: &str, text_sha: &str) -> Result<Option<String>>`: `SELECT s.note FROM doc_artifact_current c JOIN doc_artifact_sets s ON s.volume_id = c.volume_id AND s.set_id = c.set_id WHERE c.volume_id = ?1 AND c.path = ?2 AND s.text_sha = ?3` through `self.db.query_opt`, `row.opt_text(0)` flattened; volume scoped like every query in the file.
- `PendingSet::with_note(self, note: Option<String>) -> Self`; `commit` copies it into `NewArtifactSet` (`capture.rs:197`).
- `fs_ops.rs:1760`: `PendingSet::builtin(...)` becomes `PendingSet::builtin(...).with_note(fresh.stored_note().map(str::to_string))`.
- Cache branch (`extract.rs:621-648`): for `ext == ".pdf"` insert `"note"` into the payload built by `doc_payload` (`:674-690`), after reading `cached` and hashing it. No other key changes.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/meta.rs` | modify |
| `crates/core/src/docs/artifacts/capture.rs` | modify |
| `crates/core/src/core/fs_ops.rs` | modify |
| `crates/core/src/docs/extract.rs` | modify |
| `crates/core/src/storage/conformance.rs` | modify (`note: None` on the literals at `:896`, `:943`, `:972`, `:991`; a note case in the artifact conformance function) |
| `crates/core/src/purge.rs` | modify (`note: None` on the test literals at `:1993`, `:2064`) |
| `crates/core/src/tools/damage_e2e.rs` | modify (five tests) |

### Existing patterns
- A nullable column migration: `crates/core/src/storage/meta.rs:314-320` (`nodes.rev`).
- The set commit transaction: `meta.rs:872-955`; the conversion text hash: `capture.rs:158-164` (`VolumeClient::sha256_hex(self.text.as_bytes())`, `text_sha` is `None` for empty text).
- The artifact conformance function (`storage/conformance.rs:885-1010`): add after the first `commit_artifact_set`: the set carries `note: Some("n1".into())`, `text_sha: Some("sha-text")`; assert `m.current_set_note("/b.xlsx", "sha-text") == Some("n1")`, with `"other"` as the hash `None`, on the `other` volume store `None`, and after a later set with `note: None` and a new `text_sha` that `current_set_note` with the old hash is `None`.
- The cache branch to extend, `crates/core/src/docs/extract.rs:621-648`, and `Prepared::Cached`.
- Tests that seed a companion by hand: `nesting_e2e.rs:748-760` (`seed(&f, "/docs/loop.md", b"kept")`).
### Fixtures
Reuse `pdf_pages` from US-0241. E2E-008 seeds `/docs/report-3p.md` through `seed` with the text a first extraction would store, after the PDF (so its mtime is not older than the PDF's, `extract.rs:625`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `./test.sh`).

### SPEC-0094/E2E-002: Clean PDF is untouched
- **Category:** requirement
- **Scenario:** SC-002
- **Requirements:** FR-NEW-003, FR-NEW-005, FR-NEW-007
- **Given** `clean-3p.pdf` (pages `Alpha`, `Beta`, `Gamma`, all complete)
- **When** the member extracts its text, then extracts it again
- **Then** both answers hold the text of the three pages (the second one as a preview) and an empty note, and the second answer comes from the stored text copy
- **And** neither note contains `damaged` nor `scanned`, and the second answer holds a note that is exactly the empty text
- Level / Driver / Location: functional e2e / `extract_text` helper, twice / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_002_a_clean_pdf_is_untouched`
- Red before the code: the second answer has no `note` key (`v["note"]` is null). Clean PDF `clean-3p.pdf` pages `Alpha`, `Beta`, `Gamma`. The first answer `cached` false, the second true.

### SPEC-0094/E2E-004: Twenty five damaged pages
- **Category:** edge
- **Scenario:** SC-004
- **Requirements:** FR-NEW-002, FR-NEW-005
- **Given** a 25 page PDF where every page is cut like page 2 of `report-3p.pdf`
- **When** the member extracts its text, then extracts it again
- **Then** both notes are exactly `pdf: damaged or cut content on pages 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20 and 5 more; the text may be incomplete`
- **And** the second answer comes from the stored text copy
- Level / Driver / Location: functional e2e / `extract_text` helper, twice / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_004_twenty_five_damaged_pages`
- Red before the code: the second answer has no `note` key. 25 pages built by `pdf_pages`, each cut like page 2 of `report-3p.pdf`.

### SPEC-0094/E2E-007: Repeating the extraction keeps the note
- **Category:** side effect
- **Scenario:** SC-003
- **Requirements:** FR-NEW-005, FR-NEW-006
- **Given** `report-3p.pdf` extracted once (E2E-001)
- **When** the member extracts it a second time, then a third time with the refresh option
- **Then** the second answer comes from the stored text copy with the note `pdf: damaged or cut content on page 2; the text may be incomplete`; the third answer does not come from the stored copy and carries the same note
- Level / Driver / Location: functional e2e / `extract_text` helper, three calls, the third with `refresh` true / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_007_repeating_keeps_the_note`
- Red before the code: the second answer has no `note` key.

### SPEC-0094/E2E-008: A stored copy with no damage record
- **Category:** state transition
- **Scenario:** SC-003
- **Requirements:** FR-NEW-005, FR-NEW-006
- **Given** `report-3p.pdf` and a stored text copy for it that is newer than the PDF and was written without any damage record (as copies written before this change are)
- **When** the member extracts its text, then extracts it with the refresh option
- **Then** the first answer comes from the stored copy with a note that is present and exactly the empty text; the second answer carries `pdf: damaged or cut content on page 2; the text may be incomplete`
- Level / Driver / Location: functional e2e / `extract_text` helper after `seed` of the companion / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_008_a_stored_copy_with_no_damage_record`
- Red before the code: the first answer has no `note` key (the spec wants it present and empty). The damage note of the refreshed answer comes from the detection of US-0241, already on the branch.

### SPEC-0094/E2E-020: The note does not depend on the length asked for
- **Category:** edge
- **Scenario:** SC-003
- **Requirements:** FR-NEW-002, FR-NEW-005
- **Given** `report-3p.pdf` extracted once with a maximum length of 20 characters
- **When** the member extracts it again with a maximum length of 1000
- **Then** the answer comes from the stored text copy and the note is exactly `pdf: damaged or cut content on page 2; the text may be incomplete`
- Level / Driver / Location: functional e2e / `fs_ops::extract_document` called directly with `max_chars` 20 then 1000 (`archive_testkit.rs:489-500` shape) / `crates/core/src/tools/damage_e2e.rs`
- Function: `e2e_0094_020_the_note_does_not_depend_on_the_length_asked_for`
- Red before the code: the second answer has no `note` key. The stored copy holds the text cut at 20 characters; the second answer is `cached` true with the damage note.

## Constraints
- Files not to touch: `mcp/server.rs`, `api/dataplane.rs`, `docs/pdf_bound.rs`, `tool-contract-golden.json`, `TOOL_CONTRACT.txt`.
- Dependencies not to add: none.
- Patterns to avoid: reading the PDF source or the artifact blobs on a cache hit; a `note` key on a non PDF cached answer; a hand written `ALTER TABLE` (use `ColumnMigration`); a new table.
- Scope boundary: the fresh answer and the detection are US-0241; other formats unchanged and documentation are US-0243.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; SQLite conformance green, PostgreSQL conformance green when `MCPFS_TEST_PG_DSN` is available (say which ran).
- [ ] Non regression: a fresh database and a database opened after the column was added both work (the conformance `connect` twice rule); `e2e_0089_020`, `e2e_0092_011`, `office_bound_e2e` cached tests pass unmodified; `purge.rs` and `conformance.rs` change only by the new field.
- [ ] Docs: none in this story (US-0243).
- [ ] Self review: the lookup is volume scoped and bound to the hash; the note and the set land in one transaction; an empty extraction writes no stored copy so no cached note exists for it; no extra blob read on a hit.
- [ ] One commit `US-0242: keep the damage note on the stored text copy` on the work branch, body with `Summary:`, `Spec: SPEC-0094` and the requirement ids.
