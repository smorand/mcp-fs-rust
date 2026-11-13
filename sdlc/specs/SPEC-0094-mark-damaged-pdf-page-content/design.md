# SPEC-0094: Tell a member when a PDF page is damaged or cut, design

> Spec: sdlc/specs/SPEC-0094-mark-damaged-pdf-page-content/spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-12
> Target tier: 2 (default, no `--tier`, no constitution field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Language | Rust, edition 2024 (existing) | per `Cargo.toml` workspace | none, existing |
| PDF reader | `pdf-extract` 0.12 over `lopdf` 0.42 (existing, locked) | `Cargo.lock:3677` lopdf 0.42.0 | DEC-001 |
| Relational store | `RelationalDb` over SQLite or PostgreSQL (existing) | n/a | DEC-005 |
| Tests | `cargo test`, functional e2e through `fs_ops::extract_document` (existing) | n/a | DEC-008 |

No new dependency, no version looked up: `Content::decode_strict` already ships in the locked lopdf (`lopdf-0.42.0/src/parser_aux.rs:25-27`).
Skills loaded: `rust`. Effect: `make` targets and the `make check` gate (the project's `./test.sh`, clippy `-D warnings`, `cargo fmt --check` all stay as in `AGENTS.md`), thiserror style errors through the existing `ToolError`, tests at the functional e2e level, `tracing` untouched (no new boundary). Where the skill and the project differ the project wins (house style `AGENTS.md`: the tests run through `./test.sh`, the layout `crates/core/src/tools/*_e2e.rs`).

## 2. Approaches

### Detection

#### Chosen: strict decode first, lenient decode only on failure
At the two places where page content is parsed (`pdf_bound.rs:1514` the page, `:1604` a drawn form) call `Content::decode_strict` first. `Ok` gives exactly the operations the lenient decode gives (one grammar, `parser/mod.rs:721-742`), so a clean page parses once and the walk is unchanged. `Err` marks the page damaged and the walk falls back to the lenient `Content::decode`, which is what it runs today.
Pros: no behavior change for the walk, a clean PDF pays nothing, the damaged page pays one more parse. Cons: a damaged page parses twice (bounded by the same stream limits, the content was already measured). Affected scope: `docs/pdf_bound.rs` only. Best suited when the library already exposes the strict grammar, as here.

#### Rejected: lenient decode then a second strict pass on every page
Pays a second parse of every clean page. Rejected by DEC-001.

#### Rejected: our own content validator
Duplicates a grammar that already exists and would drift from what the reader accepts. Rejected by DEC-001.

### Persisting the note for the stored text copy

#### Chosen: a nullable `note` column on the artifact set row
`doc_artifact_sets` already records one row per conversion, written in the same transaction as the pointer `doc_artifact_current`, keyed by the conversion text's `text_sha`. A cache hit reads the companion text, hashes it, and asks the store for the note of the current set whose `text_sha` equals that hash. Pros: atomic with the conversion, exact binding to the stored text, a replaced or hand written companion cannot show a stale note, schema change is one additive column through the existing migration mechanism. Cons: the note exists only where a relational store backs the volume (every production backend; a test double has none and answers an empty note). Affected scope: `storage/meta.rs`, `docs/artifacts/capture.rs`, `core/fs_ops.rs`, `docs/extract.rs`.

#### Rejected: read the PDF again on every cached call
Would cost a blob read plus a full walk per cached answer and would re decode a PDF the cached answer must not decode (`e2e_0089_020` asserts the cache hit decodes nothing). Rejected by DEC-005.

#### Rejected: write the note into the stored text
Changes the bytes of the stored copy and its preview; the spec forbids it unless declared (14.3). Rejected by DEC-005.

#### Rejected: a sidecar file or a new table
A sidecar adds a visible file and a lifecycle to keep in step; a new table duplicates `doc_artifact_sets`' keys and sweep. Rejected by DEC-005.

## 3. Architecture

```
extract_document (fs_ops.rs:1719)
  prepare_extraction (extract.rs:602)
    cache hit (.pdf)  -> read companion text -> sha256 -> store.current_set_note(path, sha) -> payload.note
    fresh             -> Extractor::extract_owned -> pdf() (extract.rs:793)
                           pdf_bound::extract -> walk() per page -> Walked.damaged -> PdfText.damaged
                           damage_note(&damaged) + scanned message -> ExtractResult.note
  land_conversion (fs_ops.rs:867) -> PendingSet::builtin(...).with_note(stored note) -> commit_artifact_set (note column)
```

- Document reading (`docs/pdf_bound.rs`, `docs/extract.rs`): judges pages, builds the note, composes it with the scanned message.
- Stored text copies (`storage/meta.rs`, `docs/artifacts/capture.rs`, `core/fs_ops.rs`, `docs/extract.rs` cache branch): keeps and returns the note of the extraction that wrote the copy.
- The MCP tool (`mcp/server.rs:2402`) and the REST route (`api/dataplane.rs:171,1487`) are untouched: both call `fs_ops::extract_document` (`AGENTS.md` conventions), which gives FR-NEW-009 for free.

## 4. Data model

One additive column. Before: `doc_artifact_sets(volume_id, set_id, path, node_rev, text_sha, text_len, report, superseded_at, created_at)` (`storage/meta.rs:237-253`). After: the same plus `note TEXT NULL`.
- Declared in the table (fresh databases) and as a `ColumnMigration { table: "doc_artifact_sets", column: "note", ty: Text, not_null: false, default: "NULL" }` (existing databases), the same shape as `nodes.rev` (`storage/meta.rs:314-320`).
- Value: the damage note text of the extraction that wrote the set, `NULL` when none (and for every set written before this change).
- Migration: `ALTER TABLE ... ADD COLUMN` rendered per dialect (`storage/rel/dialect.rs:335`); PostgreSQL probes `DeclaredObjects.columns`. `mcp-fs migrate` copies by the declared columns (`migrate.rs:84-90`), so the column travels.
- Rollback: additive and nullable. An older binary never names the column in its `INSERT` (`meta.rs:893`) or `SELECT`, so it keeps working on a migrated database.
- Not irreversible, not destructive: category 4 of ASK ONLY FOR does not apply.

## 5. Contracts and interfaces

Signatures (all `pub(crate)` or private, nothing public):
- `docs/pdf_bound.rs`: `PdfText.damaged: Vec<u32>` (page numbers from 1, ascending, one entry per damaged page); private `Walked.damaged: bool`; private `PageWalk.damaged: bool`.
- `docs/extract.rs`: `fn damage_note(pages: &[u32]) -> String` (empty string for no pages); constants `DAMAGE_LIST_CAP: usize = 20`; `FreshExtraction::stored_note(&self) -> Option<&str>`.
- `docs/artifacts/capture.rs`: `PendingSet::with_note(self, note: Option<String>) -> Self`.
- `storage/meta.rs`: `NewArtifactSet.note: Option<String>`; `RelationalMetaStore::current_set_note(&self, path: &str, text_sha: &str) -> Result<Option<String>>`.

The note, exact wording (FR-NEW-002, FR-NEW-007):
- one page: `pdf: damaged or cut content on page 2; the text may be incomplete`
- several: `pdf: damaged or cut content on pages 1, 3; the text may be incomplete`
- more than 20: the first 20 numbers, then ` and <M> more`, M = count minus 20: `pdf: damaged or cut content on pages 1, 2, ..., 20 and 5 more; the text may be incomplete`
- with no text: `<damage note> no extractable text layer (scanned PDF?); enable a multimodal OCR provider` (one space).

Answer shape (no new key): a fresh PDF answer carries the existing `note` (`extract.rs:594`). A PDF answer from the stored copy gains `"note": "<damage note or empty string>"` and keeps every key it has (`path`, `md_path`, `format`, `chars`, `cached`, `preview`). A non PDF cached answer is untouched, no `note` (FR-NEW-005 rule).
Refusals are unchanged: `could not extract <path>: <message>` (`extract.rs:657-662`).
Config: none. Audit line: unchanged.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/docs/pdf_bound.rs` | modify: strict first at `:1514` and `:1604`, `damaged` on `PageWalk`/`Walked`/`PdfText`, collect in `extract` | FR-NEW-001, 004 | US-0241 |
| `crates/core/src/docs/extract.rs` | modify: `damage_note`, `pdf()` note join, unit tests | FR-NEW-001, 002, 007 | US-0241 |
| `crates/core/src/tools/damage_e2e.rs` | create: fixtures and 21 tests, in three stories | all | US-0241, US-0242, US-0243 |
| `crates/core/src/tools/mod.rs` | modify: `mod damage_e2e;` | all | US-0241 |
| `crates/core/src/storage/meta.rs` | modify: column, migration, `NewArtifactSet.note`, insert, `current_set_note` | FR-NEW-005 | US-0242 |
| `crates/core/src/docs/artifacts/capture.rs` | modify: `with_note`, pass to `NewArtifactSet` | FR-NEW-005 | US-0242 |
| `crates/core/src/core/fs_ops.rs` | modify: `.with_note(fresh.stored_note())` at `:1760` | FR-NEW-005 | US-0242 |
| `crates/core/src/docs/extract.rs` | modify again: cache branch note lookup, `stored_note` | FR-NEW-005, 006 | US-0242 |
| `crates/core/src/storage/conformance.rs` | modify: `note: None` on 4 literals (`:896,943,972,991`), one `current_set_note` conformance case | FR-NEW-005 | US-0242 |
| `crates/core/src/purge.rs` | modify: `note: None` on 2 test literals (`:1993,2064`) | FR-NEW-005 | US-0242 |
| `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | documentation rule | US-0243 |

`README.md`: no change (it lists no extraction answer detail; checked by `rg -n 'extract_text|note' README.md` in US-0243).
Affected existing tests: all keep. Fresh PDF answers keep `"note":""` for a clean PDF (`offload_e2e.rs:1109`, `pdf_bound_e2e.rs:111`). The cached PDF tests assert single keys only (`pdf_bound_e2e.rs:539`, `nesting_e2e.rs:756`) and the cache hit still decodes nothing (`e2e_0089_020`). The six literals above are mechanical (a new field), not a behavior change: they are not "tests to modify" in the sense of the spec (no assertion changes).
Affected documentation: `AGENTS.md` (pdf_bound paragraph, test commands), `.agent_docs/tools.md` (`fs.extract_text` row and the damage note), `.agent_docs/testing.md` (`e2e_0094_` command).

## 7. Compatibility and risks

- Consumers: a fresh PDF answer's `note` may now be non empty (a documented, asked for change). A cached PDF answer gains a `note` key (empty when none). Nothing else changes; no tool schema, description or golden JSON is touched (`tool-contract-golden.json` stays byte identical).
- Text, page markers, page count, title and refusals are identical (FR-NEW-008): the lenient decode still feeds the walk and the reader.
- Pages the reader never reaches are not examined (DEC-003): when the reader stops at a page error (`pdf_bound.rs:240-244`) the pages after it are neither extracted nor walked today. Walking them only to judge them could raise a bound refusal that is not raised today, which FR-NEW-008 forbids. Their damage is therefore not listed. This is narrower than FR-NEW-001's "every damaged page of the whole PDF"; it concerns only a document the reader already abandons, and is reported as a concern.
- Race: an extraction writes the stored copy, then commits the set (`land_conversion`, `fs_ops.rs:905-909`). A cached read between the two sees a set whose `text_sha` differs (or none) and answers an empty note once. Benign, the same window the artifacts have.
- A volume without a relational store (test doubles, `client.trash == None`) never records a note and answers an empty one on a cache hit.
- Stored copies written before this change have no row note: empty note until `refresh` (declared in the spec).
- Rollback: revert the commit; the extra column is harmless to older code.
- Dependencies: none added, none removed.

## 9. Test strategy

Level for all `E2E-` tests: functional e2e, driven through `fs_ops::extract_document` (`archive_testkit::extract_text`, `max_chars` 200000) on a `nesting_fixture`, the door SPEC-0092 uses. Exceptions stated per test. Location `crates/core/src/tools/damage_e2e.rs`, function prefix `e2e_0094_NNN_`. Unit tests (not in the spec's table) for `damage_note` in `docs/extract.rs` and for `current_set_note` in `storage/conformance.rs`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | `extract_text` helper | `tools/damage_e2e.rs` | US-0241 |
| E2E-002 | functional e2e | `extract_text`, twice | `tools/damage_e2e.rs` | US-0242 |
| E2E-003 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-004 | functional e2e | `extract_text`, twice | `tools/damage_e2e.rs` | US-0242 |
| E2E-005 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-006 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-007 | functional e2e | `extract_text`, three calls | `tools/damage_e2e.rs` | US-0242 |
| E2E-008 | functional e2e | `extract_text` after seeding a companion | `tools/damage_e2e.rs` | US-0242 |
| E2E-009 | functional e2e | `extract_text`, replace file | `tools/damage_e2e.rs` | US-0243 |
| E2E-010 | functional e2e | `extract_text`, twice | `tools/damage_e2e.rs` | US-0241 |
| E2E-011 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-012 | functional e2e | `extract_text`, refusal | `tools/damage_e2e.rs` | US-0241 |
| E2E-013 | functional e2e | `extract_text` on txt, docx, xlsx | `tools/damage_e2e.rs` | US-0243 |
| E2E-014 | functional e2e | MCP `fs_extract_text` and REST `extract-text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-015 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-016 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-017 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-018 | functional e2e | `extract_text` | `tools/damage_e2e.rs` | US-0241 |
| E2E-019 | functional e2e | `fs_ops::extract_document` with `max_chars` 20 | `tools/damage_e2e.rs` | US-0241 |
| E2E-020 | functional e2e | `extract_document` with `max_chars` 20 then 1000 | `tools/damage_e2e.rs` | US-0242 |
| E2E-021 | functional e2e | `extract_text`, twice | `tools/damage_e2e.rs` | US-0241 |

Test data: generated in `damage_e2e.rs` with `pdf_testkit::PdfWriter` (`pages_writer` builds at most two pages, so a local `pdf_pages(contents)` builder lays out N pages: object 1 catalog, 2 page tree, 5 font, page `10 + 2i`, content `11 + 2i`), forms with `nesting_testkit::form`. docx and xlsx for E2E-013 from `archive_testkit::office_zip`. No fixture files, no user provided data.
Baselines for "the text equals what the same file gave before this change": the test builds the expected body by hand from the content strings (`*[Page N]*` markers per `pdf_bound.rs:224`) and the story's red run on the current code records it.
Test hazard (E2E-009, E2E-008): the cache rule is `companion mtime >= source mtime` (`extract.rs:625`); a replacement written in the same instant as the stored copy reads as "up to date". The test advances the source with `touch_atime_mtime` after a short real wait (as `offload_e2e.rs:855` waits) before the second call.
Red evidence: BUG style red does not apply (FEAT); each new test is recorded failing on the pre story code, the green on arrival ones (E2E-013, E2E-016, E2E-017 on arrival, E2E-009 on arrival) carry a mutation proof in their story.

## 10. Decisions log

- **DEC-001:** Detect damage with `Content::decode_strict` first and fall back to the lenient `Content::decode` on error, at both parse sites (`pdf_bound.rs:1514`, `:1604`). **Obliged by:** SPEC-0094/FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-008. **Rationale:** one grammar for both calls (`lopdf parser/mod.rs:721-742`), so a clean page yields the same operations in one parse and the walk is unchanged; damaged pages cost one more parse. **Alternatives considered:** a second strict pass over every page (double parse on the common path); an own validator (grammar drift); a hook inside pdf-extract (not exposed). **Source:** existing code `crates/core/src/docs/pdf_bound.rs:1514`, `:1604`; spec 14.3. **Evidence:** `lopdf-0.42.0/src/parser_aux.rs:20-27`, `lopdf-0.42.0/src/parser/mod.rs:721-742`.
- **DEC-002:** Carry the finding as `damaged: bool` per page (`PageWalk` to `Walked`) and `damaged: Vec<u32>` on `PdfText`; a page is flagged once however many parts or forms are damaged. **Obliged by:** SPEC-0094/FR-NEW-004. **Rationale:** a boolean per page makes "named once" true by construction; the list is ascending because pages are walked in order (`pdf_bound.rs:209`). **Alternatives considered:** a set of page numbers (same result, heavier); recording the offset (spec 3.2 excludes it). **Source:** design. **Evidence:** `crates/core/src/docs/pdf_bound.rs:209-212`, `:1319-1325`.
- **DEC-003:** Do not walk pages the reader does not reach (after a reader page error); their damage is not listed. **Obliged by:** SPEC-0094/FR-NEW-008 (a walk of an extra page could raise a new refusal), limiting FR-NEW-001. **Rationale:** the pages carry no returned text and today are never walked; a refusal from them would change an outcome. **Alternatives considered:** walk them with a damage only pass (needs a second copy of the bound logic, or refuses); document the limit (chosen). **Source:** existing code `crates/core/src/docs/pdf_bound.rs:240-244`. **Evidence:** the `break` on `page.is_err()`. Reported as a concern.
- **DEC-004:** Build the note in `docs/extract.rs` next to the scanned message: `damage_note(&[u32])` with `DAMAGE_LIST_CAP = 20`, and compose in `pdf()` once: `damage`, then a space, then the scanned message when the text is empty (`with_note` overwrites, `extract.rs:89-92`). **Obliged by:** SPEC-0094/FR-NEW-002, FR-NEW-007. **Rationale:** the wording lives where the existing PDF messages live; one place builds the final string. **Alternatives considered:** build in `pdf_bound` (mixes text judging with message wording); a `Vec<String>` of notes joined later (no second note kind exists). **Source:** existing code `crates/core/src/docs/extract.rs:793-810`. **Evidence:** the exact strings in spec FR-NEW-002 and FR-NEW-007.
- **DEC-005:** Persist the note in a nullable `note` column of `doc_artifact_sets`, written by `commit_artifact_set` in the same transaction as the set and its pointer. **Obliged by:** SPEC-0094/FR-NEW-005, FR-NEW-006. **Rationale:** atomic with the conversion; bound to the stored text by `text_sha`; no extra file or table; the migration mechanism exists (`meta.rs:314-320`). **Alternatives considered:** re read and walk the PDF on every cache hit (cost, and `e2e_0089_020` forbids decoding on a hit); embed the note in the stored text (changes the copy, spec 14.3); a sidecar file; a new table; reuse the `report` column (visible as `artifacts_report`, a different meaning). **Source:** existing code `crates/core/src/storage/meta.rs:237-253`, `:872-955`. **Evidence:** `crates/core/src/storage/meta.rs:314-320`, `crates/core/src/tools/pdf_bound_e2e.rs:531-545`.
- **DEC-006:** On a PDF cache hit, hash the companion text that was just read (`VolumeClient::sha256_hex`, `volume.rs:48`) and look up `current_set_note(path, sha)`; answer `note` as the stored text or the empty string; a missing row, a different hash or no relational store gives the empty string. Non PDF extensions are not looked up and get no `note` key. **Obliged by:** SPEC-0094/FR-NEW-005. **Rationale:** the hash ties the note to exactly the stored bytes, so a replaced, edited or hand written copy cannot show a stale note (E2E-008); no source read. **Alternatives considered:** key by `node_rev` (the node of the companion, not the PDF; rev of the source needs another read); trust the mtime rule only (stale note after a hand edit). **Source:** existing code `crates/core/src/docs/extract.rs:610-648`. **Evidence:** `crates/core/src/docs/artifacts/capture.rs:158-164` (`text_sha` is the hex sha256 of the text).
- **DEC-007:** Store only the damage note: `FreshExtraction::stored_note()` returns the result note when the format is `pdf` and a stored copy is written (`md_path()` is `Some`, so the text is not empty and the note holds no scanned message), else `None`. **Obliged by:** SPEC-0094/FR-NEW-005 (the cached note is the damage note or empty; an empty text has no stored copy), FR-NEW-007. **Rationale:** `md_path()` already filters empty text (`extract.rs:556-558`); the scanned message never reaches the store. **Alternatives considered:** store the whole note for every format (would give non PDF cached answers a note, which FR-NEW-005 forbids). **Source:** existing code `crates/core/src/docs/extract.rs:556-570`. **Evidence:** same lines.
- **DEC-008:** Test every `E2E-` as a functional e2e through `fs_ops::extract_document` on a `nesting_fixture` in a new `tools/damage_e2e.rs`; E2E-014 also calls the MCP tool and the REST route of the served app. **Obliged by:** SPEC-0094/FR-NEW-009 and spec section 9. **Rationale:** the door SPEC-0092 uses, without `in_child` because no test here can overflow the stack (pages with at most 66 forms, refused before the reader). **Alternatives considered:** unit tests on `pdf_bound::extract` only (would not cover the cached path or both doors). **Source:** existing code `crates/core/src/tools/nesting_e2e.rs:96-102`, `crates/core/src/tools/archive_testkit.rs:481-502`. **Evidence:** same.
- **DEC-009:** The existing `FORM_DEPTH_LIMIT` refusal stays first: `walk` returns `Err` before any note is built, so `extract` returns the refusal and no stored copy. **Obliged by:** SPEC-0094/FR-NEW-008. **Rationale:** the flag is read only after `walk` returns `Ok`. **Alternatives considered:** none needed. **Source:** existing code `crates/core/src/docs/pdf_bound.rs:1593-1603`. **Evidence:** same lines.
- **DEC-010:** Documentation goes in the closing story: `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md`; `README.md` unchanged. **Obliged by:** n/a (no code impact): project documentation rule. **Rationale:** the house closing pattern (SPEC-0093 US-0240). **Alternatives considered:** document with each story (the docs would describe a half built feature on the branch). **Source:** `AGENTS.md` "Documentation Pattern"; `sdlc/stories/US-0240-*.md`. **Evidence:** n/a.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0241 | report cut pages in a fresh PDF extraction | P1 | FR-NEW-001, 002, 003, 004, 007, 008, 009 | main | 2 | no |
| US-0242 | keep the damage note on the stored text copy | P1 | FR-NEW-005, 006 | US-0241 | 1 | no |
| US-0243 | other formats and replaced files stay as they are, and the documentation | P2 | FR-NEW-003, 005 (partial), 006 (partial) | US-0242 | 2 | yes |
