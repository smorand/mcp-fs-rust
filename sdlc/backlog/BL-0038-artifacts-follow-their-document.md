---
Id: BL-0038
Title: Document artifacts follow their document through file operations
Nature: FEAT
Source: deferred from SPEC-0019
Created: 2026-10-09
---

## What
Make the images and tables of a converted document (SPEC-0019) follow it instead of being discarded:
move and rename, copy (with quota), trash and restore (including restore rename on collision),
permanent purge by the existing sweep, folder operations, copy or move over a converted document,
version control operations and project export.

## Why
SPEC-0019 discards artifacts on any change to the document, so a member must convert again after a
move or a restore. Following them saves that conversion.

## Notes
Cut from SPEC-0019 after five gate rounds did not converge: each operation touching a file needed its
own rule. Specify it operation by operation; the SPEC-0019 drafts of rounds 1 to 5 (FR-NEW-009 to 012,
024, 025, 029, 030) and the auditor findings on per path purge (no such operation exists,
`crates/core/src/cli.rs:114-123`), directory copy charging (`crates/core/src/core/fs_ops.rs:1177-1183`)
and restore rename (`crates/core/src/tools/trash.rs:235`) are the starting material.
