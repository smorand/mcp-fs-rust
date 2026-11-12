---
Id: BL-0154
Title: A damaged or truncated PDF page content extracts as less text instead of being reported as damaged
Nature: DEBT
Source: observed while implementing SPEC-0092 (US-0226), validated by the owner on 2026-11-12
Created: 2026-11-12
---

## What
Text extraction of a PDF goes through `pdf_extract::content::Content::decode` (lopdf based), which accepts almost any truncated or damaged page content: it reads what it can and returns that without an error. A cut or corrupted page therefore extracts as less text, or as an empty page, and the member is not told the document is damaged. A parse failure only surfaces for narrow cases (for example an inline image with no data, `BI ID`).

## Why
The answer looks like a success. A member can index or rely on an incomplete document without any signal. It has no security consequence: the SPEC-0089 and SPEC-0092 bounds apply before the reader, so a damaged page cannot bypass a limit.

## Evidence
`crates/core/src/tools/nesting_e2e.rs` (`parity_do_unparsable_form`: the fixture `(unclosed` decodes fine, `BI ID` is needed to make `Content::decode` refuse), SPEC-0092 US-0226 report. The library code is in `pdf-extract-0.12.1` and `lopdf-0.42.0` (`content.rs`, a lenient parser).

## Notes
Nothing to fix on our side without changing the library or adding a validation pass of our own. Options if it ever matters: (a) a strict check in the SPEC-0089 walk that the page content parses to its end (report the first unparsed offset); (b) a "may be incomplete" marker in the extraction answer when the walk saw trailing garbage; (c) leave as is and document it. Not urgent.
