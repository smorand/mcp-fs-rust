---
Parent Spec: SPEC-0015
Spec ID: SPEC-0015
Status: ready
Depends On: US-0012
min_tier: 2
Files touched: crates/core/src/tools/archive.rs
---
# US-0013 [converge] 7z missing and wrong password are tested

Converge round 1 gap: FR-NEW-010 and FR-NEW-011 are tested on zip only. Add tests on a real
AES-encrypted 7z fixture (built with sevenz-rust2 in the test): no password gives
`ERR_PASSWORD_REQUIRED` "password required to extract this archive"; a wrong password gives
`ERR_PASSWORD_REQUIRED` "incorrect password for this archive"; nothing is written in either case.
Fix the code if either test is red.

Orchestrator check (mechanical grep of all 61 spec test ids) also found these spec tests never
written, all in the password flow, so they belong here: E2E-NEW-004 (AES 7z, correct password,
extracts), E2E-NEW-005 (AES and legacy ZipCrypto zip, correct password, extract), E2E-NEW-007
(retry after a missing password, now supplied, succeeds), E2E-NEW-009 (retry after a wrong
password, now correct, succeeds), E2E-NEW-058 (`password: ""` against an encrypted zip is
"incorrect password", not "password required").
