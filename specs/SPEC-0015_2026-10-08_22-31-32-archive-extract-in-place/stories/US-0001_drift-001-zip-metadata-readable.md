---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: none
Complexity: XS
min_tier: 2
Files touched: crates/core/src/tools/archive.rs
---

# US-0001 — [drift] Resolve DRIFT-001 — zip metadata readable without password

## Objective

This story is a drift-closure check, not independent implementation work. It exists so the open
drift entry `DRIFT-001` (spec §19) has a tracked home and is not silently dropped from the plan.
The actual verification and any resulting code land inside **US-0006** (zip/7z entry listing),
at the point where the pinned `zip` crate's real API is read against the pinned version's
`ZipArchive`/`ZipFile` docs. State this folding explicitly when implementing US-0006: before
writing the entry-decode loop, read the pinned `zip` crate's docs.rs page and confirm whether
entry metadata (name, declared size, encryption flag) is readable without a correct password.

## Drift entry (copied verbatim from spec §19)

- **DRIFT-001**
  - Spec says: `FR-NEW-009` — zip entry metadata (name, declared size, encryption flag) is
    readable from a `zip::ZipArchive` without a correct password.
  - Code does: unknown; the `zip` crate is not yet a dependency with the `aes-crypto` feature
    enabled in this tree (confirmed absent, §2.1/§9.5). Must be verified against the exact pinned
    version once added.
  - Nature: missing capability (unverified until the dependency lands).
  - Resolution during implementation: read the pinned version's docs.rs page (or run
    `cargo doc --open -p mcp-fs-core` locally) for `ZipArchive`/`ZipFile` before writing the
    decode loop; if metadata truly requires the password for an AES-encrypted entry, treat that
    exactly like 7z's header-encryption carve-out already written into `FR-NEW-009` — no new
    requirement needed, just route that case through `FR-NEW-010`/`FR-NEW-011` at the "can't even
    list" point instead of the "can list, can't decode" point.
  - Detected by: `cargo build` (API mismatch) or `E2E-NEW-002`/`E2E-NEW-005` (wrong runtime
    behavior).
  - Blocks which requirement: FR-NEW-009, FR-NEW-010, FR-NEW-011.
  - Status: open.

## Depends On

None.

## Consumed by

US-0006 (zip/7z entry listing, corrupt-archive detection, password required/incorrect handling).
That story must re-verify this drift entry against the actually-pinned `zip` crate version before
implementing the decode loop, and close this entry's "Status: open" to "resolved" (or "accepted
limitation" with the carve-out routing described above) in its own completion notes.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5, applied to US-0006 since that
is where this drift's resolution actually happens. This story itself has no code to review.
