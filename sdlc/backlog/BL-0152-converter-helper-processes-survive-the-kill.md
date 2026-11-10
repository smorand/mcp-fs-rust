---
Id: BL-0152
Title: A converter's own child processes survive the kill on the cap or the timeout
Nature: BUG
Source: /sdlc-design of SPEC-0089, residual of DEC-015
origin: found while designing SPEC-0089
Created: 2026-11-10
Decision: filed by the SPEC-0089 design under the user's authorization of the unattended run; SPEC-0089 stops the converter program itself, which is what its requirements and tests name
---

## What
On the converter limit (SPEC-0089) or the timeout (today) the server kills and reaps the program it started. A converter that forked helpers (a shell running a pipeline, a launcher starting a worker) leaves them running: they die only when they next write into a closed pipe, or never if they write into files. Their working folder is removed under them.

## Why
Resource leak and possible writes after the call (CWE-404); the operator chooses the converter, so the trust is partial.

## Evidence
`crates/core/src/docs/service.rs:287-295` (`child.kill().await` on the direct child only); killing a process group needs `killpg` (no safe API in the tree; `unsafe_code` is forbidden). Related: BL-0150 (a helper growing a bundle file).

## Notes
Options: `Command::process_group(0)` (safe, std) plus a safe group kill through a crate already in `Cargo.lock` (`rustix` if present with its `process` feature), or document that a converter must not fork. Decide with BL-0150.
