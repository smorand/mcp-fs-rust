---
Id: BL-0158
Title: A size limit for remote git clone and fetch, against the project storage limit
Nature: FEAT
Source: SPEC-0093 side finding, validated by the owner on 2026-11-12
Created: 2026-11-12
---

## What
Cloning or fetching from a remote git repository (`crates/core/src/git/remote.rs`, `clone_to_temp`, `fetch_from_remote`) is bounded by a time limit only (`deadline`, wired into libgit2's `transfer_progress`, and the caller's `tokio::time::timeout`). Nothing limits the number of bytes received. A very large or hostile repository can fill the server disk (temporary clone) or the project store before the deadline, bypassing the project storage limit that every other write respects.

## Why
The project storage limit and the session write allowance are the product's quota promises (SPEC-0020); a remote clone is a way to write far more than they allow. It only applies when git remotes are configured and a member can name a remote.

## Evidence
`git/remote.rs` (`remote.rs:278-283` the deadline note, `:398-440` the transfer callbacks; no byte counter); SPEC-0093 side findings; `quota.rs` admission functions used by the file writes.

## Notes
Proposal for the spec: count received bytes in the transfer progress callback and abort the transfer with a quota refusal (same codes and messages as the other writes: project side first, tie to the project) when the received pack plus what the project already stores would exceed the remaining storage, or the session allowance; a fixed ceiling for the temporary clone directory; nothing left behind on refusal (the temporary folder removed). Tests need a local git server stand-in that streams a large pack.
