# US-0213: encrypted PDFs, cross reference, LZW and ASCII85 streams under the same limit

> Spec: SPEC-0089 (sdlc/specs/SPEC-0089-bound-pdf-stream-inflation-and-the-document-converter-output/)
> Nature: FEAT
> Priority: P1
> Depends on: US-0212
> min_tier: 1
> Files touched: 6
> Closes spec: no

## Objective
Close the three decode paths US-0212 leaves as today: a stream encoded with LZW or ASCII85 is measured like a Flate one, the cross reference stream lopdf inflates while reading the cross reference is measured before the load, and an encrypted PDF no longer goes through lopdf's encrypted loader, which inflates object streams with no filter and no type check. After this story no inflation in the PDF text pass escapes the stream limit.

## Requirements covered
- SPEC-0089/FR-NEW-002 (partial: LZW and ASCII85 decodes, the cross reference stream, the object streams of an encrypted document): IF a stream of a PDF decodes to more than 104 857 600 bytes THEN THE document text extraction SHALL stop decoding it and refuse with `ERR_INVALID_ARGUMENT` and the message `could not extract <path>: a PDF stream decodes to more than 104857600 bytes`.
- SPEC-0089/FR-NEW-009 (partial: encrypted, LZW and ASCII85 documents within the limits answered as before): THE document text extraction SHALL answer a PDF within every limit with the same text, page markers, page count, title, note, companion, preview, charge and audit line as before, SHALL answer from an up to date companion without reading the PDF when no refresh is asked, and SHALL extract every other format exactly as before.

## Independent Test
Run `cargo test -p mcp-fs-core --lib -- pdf_cross_reference_stream_bomb_is_refused pdf_encrypted_object_stream_bomb_is_refused pdf_encrypted_document_text_is_unchanged pdf_escaped_encrypt_key_is_found pdf_lzw_content_bomb_is_refused pdf_ascii85_then_flate_bomb_is_refused pdf_lzw_and_ascii85_text_is_unchanged`: 7 tests pass. Real assertions: the three bombs are refused with the stream limit message and nothing left; the three "unchanged" tests compare our pages and title with what the library's own pass (`pdf_extract::extract_text_from_mem_by_pages`, still public) returns for the same bytes. Then `cargo test -p mcp-fs-core --lib -- e2e_0089_` passes (US-0212's tests unchanged).

## Design excerpt
### Decisions
- **DEC-003 (SPEC-0089), this story's part:** `LZWDecode` is measured through `weezl` configured as lopdf configures it (`EarlyChange` from `DecodeParms`, default 1: `Decoder::with_tiff_size_switch(BitOrder::Msb, 8)`, else `Decoder::new(BitOrder::Msb, 8)`, `lopdf-0.42.0/src/object.rs:818-846`), output counted from `decode_bytes` into a 65 536 byte buffer (`weezl-0.1.12/src/decode.rs:208`, `:329`), an error ending the step with what it produced (lopdf keeps the partial output); `ASCII85Decode` by a streaming copy of `decode_ascii85` (`object.rs:874-920`: `z` gives 4 zero bytes, whitespace skipped, a byte outside `!` to `u` ends the input, a trailing `~>` removed, a final partial group gives `count - 1` bytes); chains pipe each step's output into the next step's measurer, each step's output counted (the decoded size is the sum over steps). The stream limit is checked after every chunk on the running size.
- **DEC-005 (SPEC-0089):** before the load: `xref_guard` locates the cross reference sections as lopdf does (the `%PDF-` offset, `startxref` searched before the last `%%EOF` of the last 512 bytes, `reader.rs:1262-1278`; then `Prev` and `XRefStm` chains with a seen set, `:765-791`), parses each object section with `Reader::get_object` over a cross reference holding that one offset (the public parser, so the dictionary is the one lopdf will see), and measures its stream when its `Length` is direct (an indirect one makes lopdf read it empty, `parser/mod.rs:344-359`); over the stream limit: the FR-NEW-002 refusal before any object is read. Any discovery failure is ignored: lopdf then fails or succeeds as today. A classic section's trailer is parsed the same way from a synthetic `0 0 obj` holding its dictionary bytes, located by a minimal tokenizer that skips strings, hex strings, names and comments. When the main trailer holds an `/Encrypt` key (at depth 1, found by that tokenizer), the bytes are copied and that key renamed in place to a name of the same length (`/NoCrypt` for the plain spelling, `#xx` escapes decoded by the tokenizer, offsets unchanged); the copy is checked by re parsing its trailer with lopdf's parser, and when that parse still holds `/Encrypt` the document fails closed as unreadable (`could not extract <path>: pdf: the encryption dictionary could not be isolated`); else the copy is loaded with the filter of DEC-004 (lopdf's encrypted loader, which ignores the filter, is never entered); then the key is renamed back in `doc.trailer` and today's states are reproduced: if `doc.authenticate_password("")` fails, every object but the encryption dictionary is dropped (what `load_encrypted_document` leaves, `reader.rs:832-856`, `:1147-1166`) and `doc.decrypt("")` gives today's error; else `doc.decrypt("")` decrypts every object (it skips our renamed object streams, `document.rs:477-482`), and the deferred object streams are then handled as in DEC-004, inserted the way the encrypted loader does (only numbers the cross reference places in that container, replacing, `reader.rs:887-897`). Known differences with today, accepted: an object whose encrypted bytes contain `endobj` is now read whole (the encrypted loader cuts it, `extract_raw_object`); trailer key order.
- **DEC-019 (SPEC-0089), this story's part:** `encrypted(doc)` builds the encrypted fixture with lopdf's `Document::encrypt` and an `EncryptionVersion::V1` state, user password empty (`lopdf-0.42.0/src/document.rs:417-433`, `src/encryption.rs:139-150`), then appends a cross reference stream listing the objects of the object stream as compressed, `/Prev` the classic section. ASSUMED: if lopdf refuses that shape, the implementer writes the RC4 encryption with `md-5` 0.10.6 from `Cargo.lock` as a dev dependency and reports it. `lzw_run(letter, n)`: weezl's encoder over a 1 000 000 byte buffer repeated, cached per process; `ascii85(bytes)`: a plain encoder in the testkit.
- **DEC-021 (SPEC-0089):** `weezl = "0.1"` is added to `[workspace.dependencies]` and to `crates/core/Cargo.toml`; it resolves to the 0.1.12 already in `Cargo.lock` for lopdf, so no new crate is downloaded.
- **DEC-027 (SPEC-0089):** US-0212 left LZW and ASCII85 streams armed unmeasured, the cross reference streams unmeasured and encrypted files on lopdf's encrypted loader; this story removes the three gaps.
### Data model
No change beyond US-0212.
### Contracts
`crates/core/src/docs/pdf_bound.rs`: `fn xref_guard(data: &[u8], budget: &mut PdfBudget) -> Result<Option<TrailerSpan>>` (the refusal, or the main trailer's byte span and whether it holds `/Encrypt` at depth 1); `fn unlock_encrypted(data: &[u8], span: &TrailerSpan) -> Result<Vec<u8>>` (the copy with the key renamed: the tokenizer decodes `#xx` name escapes, so `/Encr#79pt` is found too, and its bytes are replaced by a name of the same length that decodes to something else, `/NoCrypt` for the plain spelling; when lopdf's own parse of the patched trailer still holds `/Encrypt`, the document is treated as unreadable and fails closed with `ToolError::invalid_argument("pdf: the encryption dictionary could not be isolated")`, a refusal of the FR-NEW-009 family `could not extract <path>: ...`, never handed to the encrypted loader); the measurer gains `Step::Lzw { early_change: bool }` and `Step::Ascii85`. Refusal text unchanged: `a PDF stream decodes to more than 104857600 bytes`.
`Cargo.toml` `[workspace.dependencies]`: `weezl = "0.1"`; `crates/core/Cargo.toml` `[dependencies]`: `weezl = { workspace = true }`.
### Paths
| Path | Action |
|---|---|
| `Cargo.toml` | modify |
| `crates/core/Cargo.toml` | modify |
| `Cargo.lock` | modify (generated: `mcp-fs-core` gains the `weezl` edge, no new package) |
| `crates/core/src/docs/pdf_bound.rs` | modify |
| `crates/core/src/tools/pdf_testkit.rs` | modify |
| `crates/core/src/tools/pdf_bound_e2e.rs` | modify |
### Existing patterns
US-0212's `measure`, `PdfWriter`, `zlib_run`, `nothing_left`. lopdf's discovery to mirror: `Reader::read` (`lopdf-0.42.0/src/reader.rs:739-818`), `get_xref_start` (`:1262-1286`), `get_object` (`:1099-1129`, a `Reader` value is built with its public fields, as `load_mem_with_options` does at `:98-108`), `xref::decode_xref_stream` (`src/xref.rs:169`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target `make test`). Never run test files ad hoc for the final run.

No spec acceptance test belongs to this story: the spec's bombs are all Flate streams in unencrypted files with a classic cross reference or an uncompressed one. These design tests guard the decisions above.

### Design test: cross reference stream bomb
- Level / Driver / Location: exploit (functional e2e) / typed engine call `core::fs_ops::extract_document` after `f.state.authorize(MOUNT, person)` / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_cross_reference_stream_bomb_is_refused`
- Given `/docs/xref.pdf`, a one page document whose only cross reference is a stream `<< /Type /XRef /Size 6 /W [1 4 2] /Filter /FlateDecode /Length <stored> >>` whose decoded content is its six entries followed by spaces up to 200 000 000 bytes / When the member asks for its text / Then the refusal is `could not extract /docs/xref.pdf: a PDF stream decodes to more than 104857600 bytes` / And nothing left
- Red before the change because: lopdf inflates the cross reference stream while loading and the extraction succeeds
- Cleanup: none

### Design test: encrypted object stream bomb
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_encrypted_object_stream_bomb_is_refused`
- Given `/docs/locked.pdf`, the document of E2E-SEC-002 encrypted with an empty user password, its object 1 listed as compressed in object stream 5 / When the member asks for its text / Then the refusal is the stream limit message naming `/docs/locked.pdf` / And nothing left
- Red before the change because: US-0212 loads an encrypted file through lopdf's encrypted loader, which inflates object stream 5 whole (`reader.rs:889`), and the extraction succeeds
- Cleanup: none

### Design test: encrypted document within the limits
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_encrypted_document_text_is_unchanged`
- Given `/docs/secret.pdf`, the two page document of E2E-001 (`Quarterly results`, `Outlook`, title `Report 2026` in an object stream) encrypted with an empty user password / When the member asks for its text / Then the preview is `*[Page 1]*\n\nQuarterly results\n\n---\n*[Page 2]*\n\nOutlook`, 2 pages, title `Report 2026`, and the pages equal what `pdf_extract::extract_text_from_mem_by_pages` returns for the same bytes, trimmed
- Red before the change because: the fixture builder does not exist; once it does, the test is green with US-0212's code (the encrypted loader decodes it) and must stay green through the change: record the baseline, then the mutation below
- Cleanup: none

### Design test: LZW content bomb
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_lzw_content_bomb_is_refused`
- Given `/docs/lzw.pdf` whose page 1 content is `<< /Filter /LZWDecode /Length <stored> >>` over the LZW code of 200 000 000 spaces / When the member asks for its text / Then the stream limit refusal naming `/docs/lzw.pdf` / And `decoded_held` at most 104 933 136; nothing left
- Red before the change because: US-0212 arms an LZW stream unmeasured: the reader decodes 200 000 000 bytes and the extraction succeeds
- Cleanup: none

### Design test: ASCII85 then Flate bomb
- Level / Driver / Location: exploit (functional e2e) / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_ascii85_then_flate_bomb_is_refused`
- Given `/docs/a85.pdf` whose page 1 content is `<< /Filter [/ASCII85Decode /FlateDecode] /Length <stored> >>` over the ASCII85 form of the zlib bomb stream / When the member asks for its text / Then the stream limit refusal / And nothing left
- Red before the change because: US-0212 arms a chain holding ASCII85 unmeasured and the extraction succeeds
- Cleanup: none

### Design test: LZW and ASCII85 documents within the limits
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_lzw_and_ascii85_text_is_unchanged`
- Given `/docs/old.pdf`, two pages whose contents are `BT /F1 12 Tf 72 700 Td (Old LZW) Tj ET` under `/LZWDecode` and `BT /F1 12 Tf 72 700 Td (Old A85) Tj ET` under `/ASCII85Decode` / When the member asks for its text / Then the preview is `*[Page 1]*\n\nOld LZW\n\n---\n*[Page 2]*\n\nOld A85`, equal to the library's own pass
- Red before the change because: green with US-0212 (armed unmeasured); kept green through the change: baseline, then the mutation below
- Cleanup: none

### Design test: an escaped encryption key is found
- Level / Driver / Location: functional e2e / typed engine call / crates/core/src/tools/pdf_bound_e2e.rs
- Function: `pdf_escaped_encrypt_key_is_found`
- Given `/docs/escaped.pdf`, the bytes of `/docs/secret.pdf` whose trailer key `/Encrypt` is rewritten `/Encr#79pt` (and the offsets after it shifted by 2 in the cross reference) / When the member asks for its text / Then the preview is that of `pdf_encrypted_document_text_is_unchanged`, equal to the library's own pass
- Green on arrival: US-0212's code passes it (lopdf's encrypted loader decodes the escape); it must stay green through step 4: baseline, then the mutation below
- Cleanup: none

### Mutation evidence for tests green on arrival
| Test | Mutation |
|---|---|
| `pdf_encrypted_document_text_is_unchanged` | in the encrypted path, skip `doc.decrypt("")` after the load: the strings and contents stay encrypted, the preview differs, the test fails |
| `pdf_escaped_encrypt_key_is_found` | in the tokenizer, compare name bytes without decoding `#xx`: the key is not renamed, the call fails closed with `pdf: the encryption dictionary could not be isolated`, the test fails |
| `pdf_lzw_and_ascii85_text_is_unchanged` | in the ASCII85 measurer, return 0 and leave the stream unarmed: page 2 is read as stored bytes, its text differs, the test fails |

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `crates/core/src/tools/archive_testkit.rs`
- Dependencies not to add: anything but `weezl` (already in `Cargo.lock`, 0.1.12); `md-5` only under the ASSUMED fallback of DEC-019, as a dev dependency, reported
- No `unsafe`; the in place rename keeps the byte length (`/Encrypt` and `/NoCrypt` are 8 bytes); never decrypt by hand
- No dash as punctuation in code, comments or docs
- Scope boundary: load total and object limit are US-0214

## Definition of done
- [ ] RED evidence recorded for `pdf_cross_reference_stream_bomb_is_refused`, `pdf_encrypted_object_stream_bomb_is_refused`, `pdf_lzw_content_bomb_is_refused`, `pdf_ascii85_then_flate_bomb_is_refused` (step 1: fixtures and tests on US-0212's code, under `heavy()`), before the production code (step 2: weezl, LZW and ASCII85 steps; step 3: cross reference guard; step 4: encrypted path).
- [ ] Green on arrival tests (`pdf_encrypted_document_text_is_unchanged`, `pdf_escaped_encrypt_key_is_found`, `pdf_lzw_and_ascii85_text_is_unchanged`): baseline run recorded first, each proven to fail under its mutation, mutation reverted.
- [ ] Every acceptance test above green; `make check` green on the spec branch after this commit (`make security` included: `cargo deny` accepts `weezl`, MIT or Apache 2.0); main only receives the whole spec.
- [ ] Non regression: every `e2e_0089_` test of US-0212 and `cargo test -p mcp-fs-core --lib docs::extract tools::artifacts` pass.
- [ ] Docs updated: none in this story (the closing story records the `weezl` dependency in `AGENTS.md`).
- [ ] Self review: every new branch has a test; no `unwrap` or `expect` on fallible production paths; no `unsafe`; failure modes listed in the commit body (an unparseable trailer, an `/Encrypt` hidden from the tokenizer).
- [ ] One commit `US-0213: encrypted PDFs, cross reference, LZW and ASCII85 streams under the same limit` on the spec branch, body with `Summary:`, `Spec: SPEC-0089` and the requirement ids (SPEC-0089/FR-NEW-002, FR-NEW-009).
