---
Id: BL-0119
Title: The upload-pack answer ignores which options the client chose (multi_ack, side-band)
Nature: FEAT
Source: converge audit of SPEC-0067 (US-0141)
Created: 2026-10-31
---

## What
`negotiate::parse` drops the options of the first `want` line, so the answer is always the `multi_ack_detailed` form (`ACK <id> common`, `ready`, `NAK`) with the pack on side-band channel 1. A client that offers only `multi_ack`, or no `side-band-64k`, would get lines or framing it did not ask for; git's own server answers such a client differently (`ACK <id> continue`, a raw pack). The service advertises `multi_ack` (`UPLOAD_PACK_CAPABILITIES`, `crates/core/src/git/http/mod.rs:66-67`) but only `multi_ack_detailed` is honoured. Real git clients always send `multi_ack_detailed` and `side-band-64k` over HTTP.

## Why
No requirement of SPEC-0067 covers it (spec 3.3 says the reference runs with the options the service offers), and the existing behavior before SPEC-0067 was the same (always side-band, always NAK).

## Evidence
`crates/core/src/git/http/negotiate.rs:52` (options dropped); the advertisement at `crates/core/src/git/http/mod.rs:66-67`.

## Notes
Either stop advertising `multi_ack` and `side-band` (the 64k form is the only one real clients use over HTTP) or honour the choice. Needs a spec: observable behavior for old clients.
