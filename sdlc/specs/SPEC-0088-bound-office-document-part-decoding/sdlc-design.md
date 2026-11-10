# SPEC-0088: Bound the decoded size of Office document parts, design

> Spec: sdlc/specs/SPEC-0088-bound-office-document-part-decoding/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: internal finding
> Designed on: 2026-11-10
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| zip part lookup and decoding | `zip` (`ZipArchive::index_for_name`, `by_index_raw(i).size()` for the declared size, `by_index(i)` to decode) | `zip = "9"` (`Cargo.toml:77`), locked 9.0.5 | DEC-001 |
| bounded read | `archive_single::read_capped` (`crates/core/src/tools/archive_single.rs:92-108`, `CHUNK` 65 536 at `:14`) | in tree (SPEC-0084 DEC-001) | DEC-001, DEC-004 |
| XML parsing | `quick-xml` (unchanged use) | `quick-xml = "0.41"` (`Cargo.toml:144`) | n/a |
| test fixtures (deflate segments, checksums) | `flate2` (`Compress` with `FlushCompress::Full`, `Crc::combine`) | `flate2 = "1"` (`Cargo.toml:82`) | DEC-011 |
| log capture in tests | `crate::logging::capture` (`crates/core/src/logging.rs:100-215`) | in tree | DEC-012 |

No new dependency, no manifest change. Versions from the manifests and `Cargo.lock`. The dev profile is
not touched (optimizing dependencies in tests is BL-0137).

Skills loaded: `rust` (edition 2024, no `unwrap` or `expect` on fallible production paths, typed
`ToolError`, clippy `--all-targets --all-features -D warnings`, `cargo fmt --check`, functional e2e
through the real entry points).

## 2. Approaches

### Chosen: one bounded part reader with a per call budget, plus a content counter
Replace `entry_text` (`crates/core/src/docs/extract.rs:563-568`), the one function every Office call
site uses to read a part, by `OfficeBudget::read_part`: it reads the part's declared size from the
central directory without decoding, refuses a part over 104 857 600 bytes, refuses when the declared
sizes read so far plus this one pass 268 435 456 bytes, then decodes through `read_capped` capped at the
declared size and refuses a part that yields more. The same budget counts the extracted content where
it grows (Markdown blocks, table cells, the Markdown table rendered cell by cell) and refuses past
268 435 456 bytes. Pros: occurrences 1 to 10 of the class sweep are closed at one site; occurrence 11 at
the three places content grows; zero decoded bytes for a part refused on its declared size; the limits
are checked in reading order, which is what FR-NEW-009 orders. Cons: every reader and helper that read a
part must now propagate a refusal (`Result<Option<String>>` instead of `Option<String>`), and
`md_table` is rewritten to render row by row. Affected scope: `crates/core/src/docs/extract.rs` only
for production behaviour; `mcp/server.rs` and `api/dataplane.rs` for the refusal log line (DEC-012).
Best suited when, as here, one reader serves every part.

### Rejected: decode each part whole, then check its size
Keeps the reader and adds a check after `read_to_end`. Rejected: FR-NEW-002 and FR-NEW-003 require the
refusal before decoding, and the exploit tests assert 0 decoded bytes held (DEC-001).

### Rejected: one global decoded byte budget, no declared size check
Counts decoded bytes as they come, like SPEC-0084 for a compressed tar. Rejected: a bomb part would be
decoded up to the limit before the refusal (104 857 600 bytes held where the spec asks 0), and the
messages of FR-NEW-002 carry the declared size, which this approach never reads (DEC-001).

### Rejected: limits from the configuration
Spec D-4 fixes the numbers; no `config.rs` key bounds built in extraction today (DEC-002).

## 3. Architecture

```
fs.extract_text (mcp/server.rs:2396-2422)        POST /api/fs/{mount}/extract-text (api/dataplane.rs:1484-1505)
        \  membership (AppState::authorize), path normalized       /
         +---- core::fs_ops::extract_document (fs_ops.rs:1720) ----+
                 prepare_extraction (extract.rs:318)
                   not a file -> ERR_NOT_FOUND                      FR-NEW-009 item 2
                   up to date companion, no refresh -> cached       (header check only, no part read)
                   Extractor::extract (extract.rs:151)
                     .docx -> docx(data)  .pptx/.pptm/.potx/.ppsx -> pptx(data)  .xlsx/.xlsm -> xlsx(data)
                       office(data, read):  OfficeBudget::new()
                         open_zip + SPEC-0041 header check          FR-NEW-009 item 3
                         read(zip, budget): parts in reading order
                           budget.read_part(zip, name)              item 4: part limit, call limit, size lie
                           budget.add_content(n) where content grows item 5: content limit
                         cfg(test): LAST_OFFICE_HELD = budget figures
                     cfg(test): OFFICE_HELD[filename] = LAST_OFFICE_HELD.take()
                   refusal -> ERR_INVALID_ARGUMENT "could not extract <path>: <reason>"   (extract.rs:366-374)
                 success only: charge, companion, artifacts, audit (fs_ops.rs:1771-1788)
        to_call_result / error_response: one INFO line per refused call (DEC-012)
```

- Bounded context "document text extraction" owns `OfficeBudget` and the four refusals; it lives in
  `crates/core/src/docs/extract.rs` next to the readers (the readers' private helpers). Project storage
  (companion, artifacts, allowance, audit) is untouched: a refusal returns before `land_conversion`
  (`crates/core/src/core/fs_ops.rs:1739-1748`), as every extraction failure does today.
- `md_table` (`extract.rs:468-490`) is shared with CSV: CSV keeps its output, through the same function
  with a budget that never refuses (DEC-007).

## 4. Data model

Nothing persisted. In memory, per call:

- `OfficeBudget { declared: u128, decoded: u64, content: u64, content_limit: Option<u64> }`: the sum of
  the declared sizes of the parts read (`u128`, never wraps), the decoded part bytes (hook figure), the
  extracted content counted so far, and the content limit (`None` for CSV).
- `#[cfg(test)] OfficeHeld { parts: u64, content: u64 }` and two thread locals (DEC-008).

## 5. Contracts and interfaces

`crates/core/src/docs/extract.rs` (all `pub(crate)` or private, no public API change):

```rust
pub(crate) const PART_LIMIT: u64 = 104_857_600;
pub(crate) const CALL_LIMIT: u64 = 268_435_456;
pub(crate) const CONTENT_LIMIT: u64 = 268_435_456;

struct OfficeBudget { .. }
impl OfficeBudget {
    fn office() -> Self;            // content limit CONTENT_LIMIT
    fn unbounded() -> Self;         // CSV: counts, never refuses
    /// Ok(None): absent, unreadable or damaged part (unchanged, BL-0144).
    fn read_part(&mut self, zip: &mut Zip, name: &str) -> Result<Option<String>>;
    fn add_content(&mut self, bytes: usize) -> Result<()>;
}
fn office(data: &[u8], read: impl FnOnce(&mut Zip, &mut OfficeBudget) -> Result<ExtractResult>) -> Result<ExtractResult>;
fn docx(data: &[u8]) -> Result<ExtractResult>;   // signature unchanged
fn pptx(data: &[u8]) -> Result<ExtractResult>;   // signature unchanged
fn xlsx(data: &[u8]) -> Result<ExtractResult>;   // signature unchanged
fn md_table(rows: &[Vec<String>], budget: &mut OfficeBudget) -> Result<String>;
```

Refusals, `ToolError::invalid_argument` (`ERR_INVALID_ARGUMENT`), message without prefix; the existing
wrapper at `extract.rs:366-374` adds `could not extract <path>: `:

| Rule | Message after the prefix |
|---|---|
| FR-NEW-002 | `document part '<part>' is <D> bytes once decoded, over the limit of 104857600 bytes` |
| FR-NEW-003 | `the document parts read total more than 268435456 bytes once decoded` |
| FR-NEW-004 | `document part '<part>' decodes to more bytes than declared <D>` |
| FR-NEW-005 | `the extracted content exceeds 268435456 bytes` |

`<part>` is the name looked up (`word/document.xml`), `<D>` the `u64` declared size printed with `{}`.

Logging (DEC-012): `crates/core/src/mcp/server.rs` `to_call_result` calls
`crate::logging::log_tool_failure(tool, &e)` on `Err`; `crates/core/src/api/dataplane.rs`
`error_response` calls `crate::logging::log_tool_failure("rest", err)`. Message: `tool call failed:
ERR_INVALID_ARGUMENT: could not extract ...` at INFO for every 4xx `ToolError`, ERROR otherwise
(`logging.rs:79-85`, unchanged).

`TOOL_CONTRACT.txt` and `tool-contract-golden.json`: unchanged (DEC-016).

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/docs/extract.rs` | modify: `OfficeBudget`, limits, `read_part` replaces `entry_text`, readers propagate refusals, `office()` wrapper, hook; content counting and `md_table` rendered row by row | FR-NEW-001 to FR-NEW-006, FR-NEW-009 | US-0205, US-0206, US-0207, US-0208 |
| `crates/core/src/tools/office_bound_e2e.rs` | create: the `e2e_0088_*` tests through the typed engine call | every requirement | US-0205 to US-0210 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod office_bound_e2e;` | test layout | US-0205 |
| `crates/core/src/tools/archive_testkit.rs` | modify: Office fixture, extract helper, hook readers, `deflate_run`, `zip_raw`, bomb parts, `heavy()`, `nothing_left`, `THIRD` | fixtures (DEC-011) | US-0205 |
| `crates/core/src/mcp/server.rs` | modify: `to_call_result` logs a failure once | FR-NEW-007 BR, NFR 7.6 (DEC-012) | US-0210 |
| `crates/core/src/api/dataplane.rs` | modify: `error_response` logs a failure once (US-0210); tests `e2e_0088_004`, `014`, `029` (US-0211) | FR-NEW-010, NFR 7.6 | US-0210, US-0211 |
| `crates/core/src/logging.rs` | modify (test code only): `capture` records each event's level as the field `level` | E2E-035 | US-0210 |
| `.agent_docs/architecture.md` | modify: error logging split now wired at both doors; Office extraction bound paragraph | DEC-015 | US-0210 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-015) | US-0211 |

Affected existing tests: the 14 Office reader tests of `crates/core/src/docs/extract.rs`
(`rg -n 'fn (docx|pptx|xlsx|non_zip)\w*\(\)' crates/core/src/docs/extract.rs`) keep their code and
stay green: `docx`, `pptx` and `xlsx` keep their signature (DEC-008). Kept unchanged and named because
they sit next to the change: `csv_extraction_produces_a_markdown_table`, `md_table_pads_escapes_and_flattens`
(`extract.rs:1349`, `:1394`, CSV through the unbounded budget), the SPEC-0041 extraction tests
(`crates/core/src/core/fs_ops.rs:4895-5110`), the 16 tests of `crates/core/src/tools/document.rs`, the
`logging.rs` tests (`:337-350`) and `e2e_new_028`, `e2e_new_029` (`tools/archive.rs:1886-1945`, capture
users). Any other existing test that changes is a story failure: stop and report it.

Affected documentation: `README.md` (document extraction paragraph near `:180`), `AGENTS.md`
(`docs/extract.rs` bullet), `.agent_docs/tools.md:336` (the "known remaining gap" sentence about Office
text extraction is now closed) and `:404-410` (`fs.extract_text` limits, dated SPEC-0088 paragraph),
`.agent_docs/testing.md` (filter `e2e_0088_`, `heavy()`, the builders), `.agent_docs/architecture.md`
(`:349-357`, error logging now wired).

## 7. Compatibility and risks

- Declared changes (spec): a document whose part declares over 104 857 600 bytes, whose parts read
  declare over 268 435 456 bytes together, whose part yields more than it declares, or whose content
  passes 268 435 456 bytes is refused; a lying Excel sheet or PowerPoint slide is refused where it was
  skipped.
- Undeclared but documented: every failed tool call now leaves one log line (INFO for a 4xx, ERROR
  otherwise), at both doors, which `.agent_docs/architecture.md:349-357` already describes. Nothing
  logs a password: `ToolError` messages never carry one (`e2e_new_028` and `e2e_new_029`,
  `crates/core/src/tools/archive.rs:1886-1945`, guard the archive messages).
- Memory beyond the counted figures, not in this spec's definitions and filed: the assembly copies of
  the text (`of_blocks` join and trim, per cell trim, the character cut) and a lossy UTF-8 repair of an
  invalid part hold transient copies (BL-0145); the readers decode on the async worker thread (BL-0146).
- Ordering: US-0205 builds the reader and the hook with the exploit tests red first; US-0206 to US-0208
  add one rule each on the same file, one writer at a time.
- Rollback: revert the story commits; no data or contract change.

## 8. Security design

- Threat surface: a stored Office document is untrusted: the member who uploaded it (or an attacker who
  got a member to upload it) chooses its parts' declared sizes, its compressed data, its relationships
  (which parts are read, how many times) and its cell references. Sinks: the in memory decode of a part
  (`extract.rs:563-568`, `read_to_end` into an unbounded `Vec`), the copy of a shared text per cell
  (`extract.rs:1116`), the uncapped grid and the Markdown table (`extract.rs:997-999`, `:462-490`).
- Occurrences 1 to 10 (`extract.rs:716`, `:871`, `:879`, `:896`, `:897`, `:939`, `:992`, `:1006`,
  `:1046`, `:1048`, all through `entry_text` at `:563-568`): fix site `entry_text`, replaced by
  `OfficeBudget::read_part`; mechanism: allow list on the declared size read from the central directory
  before any decoding (part limit, then call limit), then a capped read at the declared size (DEC-001,
  DEC-003, DEC-004). Exploit tests E2E-SEC-001 to E2E-SEC-010 (US-0205).
- Occurrence 11 (`extract.rs:1112-1118` and the Markdown table): counter charged where content grows,
  checked after each charge, so at most one piece (one cell, one paragraph) past the limit is ever held
  (DEC-007). Exploit test E2E-SEC-011 (US-0208).
- Occurrences 12 and 13 (declared sizes from 2^63): compared as `u64`, summed as `u128`, printed with
  `{}`; never cast to a signed type (DEC-003). Exploit tests E2E-SEC-012, E2E-SEC-013 (US-0205).
- Occurrences 14, 15, 18, 19: safe (spec 14.2). 16 and 17: BL-0142, BL-0143.
- Where the exploit tests run: `crates/core/src/tools/office_bound_e2e.rs`, through
  `core::fs_ops::extract_document` after `f.state.authorize(MOUNT, person)`, the function both doors
  call. Second channels for the absence of effect: the decoded bytes held and the content held read
  through the `OFFICE_HELD` hook (DEC-008, 0 when no part was decoded); the listing of `/docs` and the
  table and image artifact lists (`tools::artifacts::list_tables`, `list_images`); the audit trail
  (`SafetyManager::audit`); the session allowance read through a following real write of 52 428 800
  bytes (`core::fs_ops::write_bytes`). Each exploit test carries its literal payload: a part declaring
  and holding exactly 200 000 000 bytes (`bomb_part`), `9223372036854775808`, `18446744073709551615`
  (`zip_declaring`), a 1 000 000 letter shared text referred to by 400 cells.
- Red before the fix: the exploit tests are written and run first in their story, with the hook wired
  but no limit, and fail for the reason the spec names (the held figure; the message for 012 and 013).
  Spec 14.3 marks 012 and 013 ASSUMED (the zip reader accepting a part holding fewer bytes than it
  declares): DEC-017 says what to do when the library refuses the fixture.
- Red run cost: before the fix each of E2E-SEC-001 to 010 decodes its 200 000 000 byte part whole
  (about 1 GB peak with the text copies) and E2E-SEC-011 builds 400 copies of a 1 000 000 byte text
  (about 2 GB peak). Every SPEC-0088 test that builds or decodes a part of 10 MB or more holds the
  process wide `heavy()` lock, so they run one at a time (DEC-011); payloads are literal (no scaled
  limits, DEC-002).
- Secrets: none involved. Nothing new is traced but the refusal message, which names the path, the part
  and the sizes.

## 9. Test strategy

Functional e2e through the typed engine call `core::fs_ops::extract_document` after
`f.state.authorize(MOUNT, person)` (both doors call that function: `mcp/server.rs:2396-2422`,
`api/dataplane.rs:1484-1505`), in `crates/core/src/tools/office_bound_e2e.rs`; E2E-035 through the
assistant tool method `McpServer::fs_extract_text` (the door that logs), on a dedicated runtime under
`crate::logging::capture::lock_for_test`; E2E-004, 014 and 029 through the HTTP door in the tests of
`crates/core/src/api/dataplane.rs` (`Harness`, its project `proj` and member `owner@test.com` stand for
`atlas` and `mia@example.com`, as `e2e_0085_005` does). Names `e2e_0088_<id>_<title in snake case>`,
cut at 50 characters, trailing underscore dropped; filter `e2e_0088_`. No feature: `./test.sh`'s first
command runs them all (DEC-010).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | office_bound_e2e.rs | US-0208 |
| E2E-002 | functional e2e | typed engine call | office_bound_e2e.rs | US-0208 |
| E2E-003 | functional e2e | typed engine call | office_bound_e2e.rs | US-0208 |
| E2E-004 | functional e2e | HTTP door | api/dataplane.rs tests | US-0211 |
| E2E-005 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-006 | functional e2e | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-007 | functional e2e | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-008 | functional e2e | typed engine call | office_bound_e2e.rs | US-0206 |
| E2E-009 | functional e2e | typed engine call | office_bound_e2e.rs | US-0206 |
| E2E-010 | functional e2e | typed engine call | office_bound_e2e.rs | US-0206 |
| E2E-011 | functional e2e | typed engine call | office_bound_e2e.rs | US-0206 |
| E2E-012 | functional e2e | typed engine call | office_bound_e2e.rs | US-0206 |
| E2E-013 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-014 | functional e2e | HTTP door | api/dataplane.rs tests | US-0211 |
| E2E-015 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-016 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-017 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-018 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-019 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-020 | functional e2e | typed engine call | office_bound_e2e.rs | US-0208 |
| E2E-021 | functional e2e | typed engine call | office_bound_e2e.rs | US-0208 |
| E2E-022 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-023 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-024 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-025 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-026 | functional e2e | typed engine call | office_bound_e2e.rs | US-0207 |
| E2E-027 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-028 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-029 | functional e2e | HTTP door | api/dataplane.rs tests | US-0211 |
| E2E-030 | functional e2e | typed engine call | office_bound_e2e.rs | US-0210 |
| E2E-031 | functional e2e | typed engine call | office_bound_e2e.rs | US-0210 |
| E2E-032 | functional e2e | typed engine call | office_bound_e2e.rs | US-0210 |
| E2E-033 | functional e2e | typed engine call | office_bound_e2e.rs | US-0210 |
| E2E-034 | functional e2e | typed engine call | office_bound_e2e.rs | US-0210 |
| E2E-035 | functional e2e | assistant tool method `McpServer::fs_extract_text`, log capture | office_bound_e2e.rs | US-0210 |
| E2E-036 | functional e2e | typed engine call, three calls joined | office_bound_e2e.rs | US-0210 |
| E2E-037 | functional e2e | typed engine call | office_bound_e2e.rs | US-0209 |
| E2E-SEC-001 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-002 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-003 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-004 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-005 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-006 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-007 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-008 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-009 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-010 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-011 | exploit | typed engine call | office_bound_e2e.rs | US-0208 |
| E2E-SEC-012 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |
| E2E-SEC-013 | exploit | typed engine call | office_bound_e2e.rs | US-0205 |

Test data: generated in the tests (DEC-011), no committed fixture, nothing user provided.

## 10. Decisions log

- **DEC-001:** one reader, `OfficeBudget::read_part(zip, name) -> Result<Option<String>>`, replaces
  `entry_text` at every call site: `index_for_name(name)` (absent: `Ok(None)`), declared size from
  `by_index_raw(i)?.size()` (central directory, `u64`, no decompressor built), part limit then call limit
  (DEC-003), then `by_index(i)` and `read_capped(file, declared)`; any open error other than a refusal
  stays `Ok(None)` as today. **Obliged by:** SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004,
  FR-NEW-008. **Rationale:** one fix site covers occurrences 1 to 10; the declared size is known before
  any byte is decoded, so a refusal holds 0 bytes; `index_for_name` is what `by_name` uses
  (`zip-9.0.5/src/read/zip_archive.rs:545-551`), so the parts found are the same. **Alternatives
  considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:563-568`,
  `crates/core/src/tools/archive_single.rs:92-108`, `zip-9.0.5/src/read/zip_archive.rs:551`, `:699`.
- **DEC-002:** the limits are `pub(crate) const` in `extract.rs` (`PART_LIMIT` 104 857 600,
  `CALL_LIMIT` 268 435 456, `CONTENT_LIMIT` 268 435 456), not configuration, and there is no test seam
  that scales them: every acceptance test runs against the real limits with payloads as small as the
  limits permit. **Obliged by:** SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-005, spec D-4.
  **Rationale:** the refusal messages and the held figures the spec asserts are literal
  (`104857600`, `268435456`, `104 923 136`, `268 500 992`, `373 293 056`); a scaled limit would assert a
  different message and prove nothing about the shipped one. The cost is bounded by `heavy()` and the
  cheap fixture builder (DEC-011). **Alternatives considered:** a `#[cfg(test)]` limits struct passed
  down (rejected: changes the asserted messages, adds a parameter to every reader). **Source:** spec.
  **Evidence:** spec section 9.
- **DEC-003:** arithmetic: declared sizes are `u64` compared unsigned with `>` (equal accepted); the call
  total is a `u128` (`declared + size as u128 > CALL_LIMIT as u128`), so `18446744073709551615` never
  wraps; the part limit is checked before the call limit for the part about to be read; the size enters
  the total at its check, before decoding (a part read twice counts twice; a part that then reads as
  absent still counted). Messages print the `u64` with `{}`. **Obliged by:** SPEC-0088/FR-NEW-002 business
  rule, FR-NEW-003 business rule, FR-NEW-009 item 4. **Rationale:** exact digits and no wrap for the two
  largest declarable sizes (E2E-SEC-012, E2E-SEC-013). **Alternatives considered:** `checked_add` on
  `u64` (equivalent, one more branch). **Source:** spec 14.3. **Evidence:** n/a.
- **DEC-004:** a size lie is `ReadError::CapExceeded` from `read_capped`, or a zip `InvalidData` error
  whose text contains `larger than its declared` (zip enforces the declared size itself,
  `zip-9.0.5/src/crc32.rs:43-51`, `src/types.rs` `size_limit`), mapped to `document part '<part>'
  decodes to more bytes than declared <D>`; every other read error (bad checksum, damaged stream,
  encrypted part) keeps reading as absent. **Obliged by:** SPEC-0088/FR-NEW-004, FR-NEW-011 business
  rule (BL-0144). **Rationale:** the same mapping as `zip_read_error` (`crates/core/src/tools/archive.rs:571-578`);
  at most the declared size plus one 65 536 byte chunk is held. **Alternatives considered:** trusting
  zip's own limit alone (true today, not guaranteed). **Source:** SPEC-0085 DEC-005. **Evidence:** as cited.
- **DEC-005:** refusals are `ToolError::invalid_argument` with the messages of section 5 and no prefix;
  `prepare_extraction` keeps adding `could not extract <path>: ` (`extract.rs:366-374`), so both doors
  and every format share the text; the existing `docx: word/document.xml is missing` and not a zip
  messages are unchanged. **Obliged by:** SPEC-0088/FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-NEW-005,
  FR-NEW-010, spec D-5. **Rationale:** one wrapper, no new code. **Alternatives considered:** a new error
  code (spec D-5 rules it out). **Source:** existing code. **Evidence:** `crates/core/src/docs/extract.rs:366-374`.
- **DEC-006:** propagation and reading order: `docx` uses `read_part(..)?` then the missing message on
  `None`; `pptx_slide_order` returns `Result<Vec<String>>` (presentation part or its relationships
  absent: numeric fallback as today; refused: the refusal); `pptx_notes_part` returns
  `Result<Option<String>>`; `xlsx_shared_strings` and `xlsx_sheets` return `Result<..>` (the workbook
  relationships refusal is no longer swallowed by `unwrap_or_default`); a slide or sheet that reads as
  absent is still skipped with `continue`. The reading order of spec 2.1 is unchanged. **Obliged by:**
  SPEC-0088/FR-NEW-002, FR-NEW-009 item 4, FR-NEW-011. **Rationale:** a refusal must not degrade into
  "absent" (E2E-SEC-004 would otherwise fall back to the numeric order and succeed). **Alternatives
  considered:** none. **Source:** existing code. **Evidence:** `extract.rs:716-717`, `:868-881`,
  `:894-930`, `:936-942`, `:985-1048`.
- **DEC-007:** content counting: `OfficeBudget::add_content(n)` adds `n` bytes, then refuses with
  `the extracted content exceeds 268435456 bytes` when the total is over `CONTENT_LIMIT` (equal
  accepted). Charged where content grows: each Markdown line pushed to `lines` (Word paragraph line,
  slide heading, slide paragraph, notes line, sheet heading), each grid cell when it is captured (Excel
  `xlsx_cell_text` result before the copy is kept; Word table cell at `</w:tc>`), and the Markdown table
  rendered row by row by `md_table`, which charges each cell plus its ` | ` separator before appending it
  and no longer builds a full `cleaned` copy of the grid. The peak is at most one piece over the limit, a
  piece being at most one part, so the content held stays under `CONTENT_LIMIT + PART_LIMIT` =
  373 293 056. CSV calls the same `md_table` with `OfficeBudget::unbounded()` (counts, never refuses).
  **Obliged by:** SPEC-0088/FR-NEW-005, FR-NEW-006, FR-NEW-009 item 5. **Rationale:** a table row can
  join many shared texts, so a per row or per block charge would overshoot by more than one part; per
  cell charging keeps the bound. **Alternatives considered:** counting only the final text (the grid
  multiplication of occurrence 11 would land before any check); capping the grid (changes SPEC-0019's
  artifacts). **Source:** spec 14.3. **Evidence:** `extract.rs:462-490`, `:1071-1118`, `:768-812`.
- **DEC-008:** measurement hook, test only: `docx`, `pptx`, `xlsx` keep `fn(&[u8]) -> Result<ExtractResult>`
  (the 14 existing tests unchanged) and run through `office(data, read)`, which builds the budget, opens
  the package (header check included), calls `read` and, on every outcome, stores
  `OfficeHeld { parts, content }` in the thread local `LAST_OFFICE_HELD`; `Extractor::extract` moves it,
  with no await in between, into the thread local map `OFFICE_HELD` keyed by the filename it was given.
  `parts` is the sum of the decoded part bytes of the call (`read_capped` output lengths; the fixed
  65 536 byte chunk buffer is outside the figure, as `LAST_HIGH_WATER` in SPEC-0084 DEC-006), an upper
  bound of what is held at once; `content` is the counter's final value (it only grows). A call that
  reads no part (cached, refused before `Extractor::extract`, `.docm`) leaves no entry and reads 0.
  The readers run on the calling thread (`extract.rs:157-178`, no `spawn_blocking`), so in a single
  threaded `#[tokio::test]` the thread local belongs to the test. **Obliged by:** SPEC-0088/FR-NEW-006,
  NFR 7.1, NFR 7.6 (measurement hook). **Rationale:** a per path map lets E2E-036 read three concurrent
  calls; no production signature changes. **Alternatives considered:** reusing the single slot
  `LAST_HIGH_WATER` (three calls overwrite each other); an `Arc<AtomicU64>` threaded through
  `prepare_extraction` (changes the production call path for a test). **Source:** SPEC-0084 DEC-006.
  **Evidence:** `crates/core/src/tools/archive_single.rs:55-61`.
- **DEC-009:** a decoded part becomes its `String` with `String::from_utf8(bytes)` (no copy), and only an
  invalid UTF-8 part goes through `String::from_utf8_lossy` as today. **Obliged by:** SPEC-0088/FR-NEW-006
  (spec 14.3 asks whether the copy counts), FR-NEW-011. **Rationale:** a valid part is then held once, so
  the held figure is what is held; the lossy repair copy of an invalid part is not counted and is filed
  (BL-0145). **Alternatives considered:** counting the copy (the figure would double for every valid
  part, which never copies); refusing invalid UTF-8 (changes today's text, FR-NEW-011). **Source:**
  design. **Evidence:** `extract.rs:418-420`.
- **DEC-010:** test layout: `crates/core/src/tools/office_bound_e2e.rs` (`#[cfg(test)] mod office_bound_e2e;`
  in `tools/mod.rs`), every test `#[tokio::test]` (single threaded) through `fs_ops::extract_document`
  after `f.state.authorize(MOUNT, person)`, except E2E-035 (a plain `#[test]` running a dedicated
  current thread runtime under `crate::logging::capture::lock_for_test`, calling
  `McpServer::new(state, person).fs_extract_text(Parameters(..))`, the model being
  `crates/core/src/tools/archive.rs:1862-1865`) and the three HTTP tests in `api/dataplane.rs`. Names
  `e2e_0088_<id>_<snake title>` cut at 50 characters. No cargo feature. **Obliged by:** n/a (no code
  impact): test layout. **Rationale:** SPEC-0085 DEC-011 and SPEC-0087 DEC-008 conventions; `./test.sh`
  runs the default build first (`test.sh:5`). **Alternatives considered:** tests inside
  `docs/extract.rs` (no access to the project, quota and audit channels). **Source:** existing code.
  **Evidence:** `test.sh:5-7`, `crates/core/src/tools/mod.rs:70-110`.
- **DEC-011:** fixtures, all in `crates/core/src/tools/archive_testkit.rs`: `office_fixture()` and
  `office_fixture_allowance(n)` (project `atlas`, member `mia@example.com`, folder `/docs`);
  `THIRD` = `ola@example.com` with `add_third_member`; `extract_text(f, person, path, refresh)`;
  `office_zip(parts: &[(&str, &[u8])])` (zip writer, deflate); `zip_raw(entries: &[RawPart])` writing
  given compressed bytes, checksum and declared size (`zip_declaring` becomes a thin wrapper over it,
  same output); `deflate_run(prefix, letter, n, suffix) -> (Vec<u8>, flate2::Crc)` building the deflate
  stream of `prefix + letter * n + suffix` from one per process cached segment of 1 000 000 letters
  (each segment compressed by its own `flate2::Compress` with `FlushCompress::Full`, so segments end on a
  byte boundary, reference nothing before them and concatenate into one valid stream; the checksum is
  combined with `Crc::combine`), so a 200 000 000 byte part costs about 200 KB and no 200 MB
  compression; `bomb_part(name) -> RawPart` (exactly 200 000 000 bytes, markup included: the letter `a`
  as text for `word/document.xml`, slides, notes and `xl/sharedStrings.xml`; as the value of a root
  attribute `x` for `ppt/presentation.xml`, every `.rels` part, `xl/workbook.xml` and worksheets);
  `padded_part(name, xml_head, total)` (a part of an exact size padded through a root attribute, for
  E2E-006 to E2E-011); `office_held_reset()`, `parts_held(path)`, `content_held(path)` (0 when no
  entry); `nothing_left(f, doc, companion_before)`; `heavy()` returning the guard of a process wide
  `tokio::sync::Mutex<()>`, taken first by every test building or decoding a part of 10 MB or more
  (each can hold about 1 GB; in parallel they would exhaust memory). **Obliged by:** SPEC-0088 section 9
  (fixtures built by the design), FR-NEW-002, FR-NEW-006. **Rationale:** no library writes a lying or
  zip64 header; compressing 200 MB per test in an unoptimized debug build would cost seconds each.
  **Alternatives considered:** committed fixture files (binary blobs for 40 cases, unreadable);
  `flate2::write::DeflateEncoder` per test (slow); a dev profile that optimizes dependencies (BL-0137).
  **Source:** design. **Evidence:** `crates/core/src/tools/archive_testkit.rs:236-305` (`zip_declaring`),
  `crates/core/src/docs/zip_check.rs:44-120` (`craft::rename_local`, reused by E2E-022).
- **DEC-012:** the refusal log line: `to_call_result` (`crates/core/src/mcp/server.rs:46-58`) calls
  `crate::logging::log_tool_failure(tool, &e)` before building the error result; `error_response`
  (`crates/core/src/api/dataplane.rs:259-264`) calls `crate::logging::log_tool_failure("rest", err)`.
  The test capture records each event's level as the field `level` (`logging.rs`, test module only).
  **Obliged by:** SPEC-0088/FR-NEW-007 business rule ("logged once at the informational level"),
  FR-NEW-010 (same effects at both doors), NFR 7.6. **Rationale:** mismatch found while designing: spec
  2.1 row 8 and `.agent_docs/architecture.md:349-357` say a 4xx is logged once at INFO, but
  `log_tool_failure` has no production caller (`rg -n 'log_tool_failure' crates/core/src` finds only
  `logging.rs`), so E2E-035 fails today; the requirement is satisfied by wiring the documented
  classifier at the two doors, not by a log line in the extractor (which would log twice once the doors
  log). **Alternatives considered:** an `info!` inside `prepare_extraction` (extraction only, other tools
  stay silent, contradicts the documented split). **Source:** existing code. **Evidence:** as cited.
- **DEC-013:** story order: the reader, the part limit and the hook with every exploit test of
  occurrences 1 to 10, 12, 13 red first (US-0205), then the call total (US-0206), the size lie and the
  order of refusals (US-0207), the content limit with E2E-SEC-011 red first (US-0208), the guards of
  unread parts and unchanged formats (US-0209), no trace and the log line (US-0210), the HTTP door and
  the documentation (US-0211). Global order: after US-0204, the last queued row. **Obliged by:** n/a (no
  code impact): story order. **Rationale:** exploit tests red before their fix; one writer at a time on
  `extract.rs`. **Alternatives considered:** the content limit first (independent, but SEC-011's red run
  then needs the reader rewrite anyway). **Source:** security design section 8. **Evidence:** n/a.
- **DEC-014:** tests whose rule an earlier story or today's code already holds are green on arrival and
  each is proven by a mutation reasoned to change its observable outcome (listed per story).
  **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent SPEC-0085 DEC-013.
  **Source:** design. **Evidence:** US-0189 to US-0193.
- **DEC-015:** documentation: US-0210 updates `.agent_docs/architecture.md` (the error logging split is
  now wired at both doors; a dated "Office extraction bound" paragraph: the three limits, the size lie,
  reading order); the closing story US-0211 updates `README.md` (Office extraction limits),
  `AGENTS.md` (`docs/extract.rs` bullet: `OfficeBudget`, the three limits), `.agent_docs/tools.md`
  (`:336` gap closed, `fs.extract_text` limits and messages, dated SPEC-0088 paragraph) and
  `.agent_docs/testing.md` (filter `e2e_0088_`, `heavy()`, `deflate_run`, `bomb_part`, the hook).
  **Obliged by:** n/a (no code impact): project convention (`AGENTS.md`, spec 14.3 docs list).
  **Rationale:** `.agent_docs/tools.md:336` names this gap as remaining. **Alternatives considered:**
  none. **Source:** AGENTS.md. **Evidence:** as cited.
- **DEC-016:** the tool contract is unchanged: `fs.extract_text`'s description does not mention the
  limits; they are documented in `.agent_docs/tools.md` and `README.md`. **Obliged by:**
  SPEC-0088/FR-NEW-011 (the rest unchanged). **Rationale:** spec 14.3 leaves the choice; a contract change
  would regenerate `tool-contract-golden.json` for no behaviour a caller needs to plan for.
  **Alternatives considered:** adding the limits to the description. **Source:** design.
  **Evidence:** `TOOL_CONTRACT.txt`.
- **DEC-017:** the ASSUMED zip64 short part (spec 14.3, E2E-SEC-012, E2E-SEC-013): the first step of
  US-0205 builds both fixtures with `zip_declaring` and runs today's code. If the extraction succeeds on
  the 5 real bytes, the spec's red reason holds. If the zip reader refuses the short read, the part reads
  as absent: still red (the message differs), and the fix makes it green because the refusal comes
  before decoding. If `ZipArchive::new` or the SPEC-0041 header check refuses the package, that refusal
  is the control: record it in the commit body, keep the test as a guard asserting the observed refusal
  (`not a valid OOXML package:` prefix) and 0 held, and report it in the story's DONE_WITH_CONCERNS.
  **Obliged by:** SPEC-0088/FR-NEW-002 (E2E-SEC-012, E2E-SEC-013). **Rationale:** the spec's own rule for
  an ASSUMED fixture. **Alternatives considered:** none. **Source:** spec 14.3. **Evidence:**
  `zip-9.0.5/src/crc32.rs:43-51` (only the excess is checked).

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0205 | refuse an Office part declared over the part limit | P1 | SPEC-0088/FR-NEW-001, FR-NEW-002, FR-NEW-006, FR-NEW-007 | US-0204 | 1 | no |
| US-0206 | count the parts read together | P1 | SPEC-0088/FR-NEW-003, FR-NEW-006, FR-NEW-011 | US-0205 | 2 | no |
| US-0207 | refuse a part larger than it declares, in reading order | P1 | SPEC-0088/FR-NEW-004, FR-NEW-009 | US-0206 | 2 | no |
| US-0208 | bound the extracted content | P1 | SPEC-0088/FR-NEW-005, FR-NEW-006, FR-NEW-011 | US-0207 | 2 | no |
| US-0209 | unread parts, cached companions and other formats unchanged | P2 | SPEC-0088/FR-NEW-008, FR-NEW-011 | US-0208 | 2 | no |
| US-0210 | a refusal leaves no trace and one log line | P2 | SPEC-0088/FR-NEW-002, FR-NEW-006, FR-NEW-007, FR-NEW-011 | US-0209 | 2 | no |
| US-0211 | Office limits through the HTTP door and documentation | P2 | SPEC-0088/FR-NEW-010 | US-0210 | 2 | yes |

US-0205 is `min_tier` 1: the reader rewrite crosses ten call sites and three closures whose absent
versus refused semantics must not blur, and the fixture builder (segmented deflate, combined checksum,
raw zip writer) has no in tree model; no thinner slice is independently verifiable (the exploit tests
need the reader, the hook and the builder together).
