---
Id: BL-0118
Title: SPEC-0067 says an unknown held id appears in no line, git's own server answers `ACK <unknown id> ready`
Nature: FEAT
Source: converge audit of SPEC-0067 (US-0141), origin US-0137
Created: 2026-10-31
---

## What
SPEC-0067 FR-NEW-004 treats an unknown held id as not stated, and E2E-038 and E2E-011 say the unknown ids "appear in no line". FR-NEW-001 and the reference answer say the opposite for one case: git's own server, run on 2026-10-31 (git 2.50.1) with the request `want T, have U1, have T, have U2, done`, answers `ACK T common`, `ACK U2 ready`, `ACK T`: an id the project lacks, met after a held commit that already satisfies every want, is answered with `ACK <that id> ready`.

## Why
The implementation follows the reference (`git/http/negotiate.rs` `acknowledge`, DEC-004 of the design) because the spec makes the reference answer the contract. The two statements cannot both hold. A client accepts both forms; the visible difference is one extra `ready` line naming the unknown id.

## Evidence
`crates/core/src/git/http/negotiate.rs:253-268`; test `e2e_038_up_to_date_request_with_unknown_ids_besides_the_held_tip` in `crates/core/src/git/http/mod.rs` expects the `ready` line; `ready_is_withheld_like_git_does` in `crates/core/tests/git_negotiation_e2e.rs` compares with the reference.

## Notes
Amend E2E-038 and FR-NEW-004 in a `/sdlc-spec` run (the reference wins), or add a declared difference that suppresses the `ready` naming an unknown id (the client then sees `ACK <last common> ready` or none).
