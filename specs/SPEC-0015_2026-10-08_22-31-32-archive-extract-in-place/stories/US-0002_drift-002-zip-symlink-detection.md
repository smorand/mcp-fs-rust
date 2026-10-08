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

# US-0002 — [drift] Resolve DRIFT-002 — zip entry symlink bit detection

## Objective

This story is a drift-closure check, not independent implementation work. It exists so the open
drift entry `DRIFT-002` (spec §19) has a tracked home and is not silently dropped from the plan.
The actual verification and any resulting code land inside **US-0007** (symlink/hardlink/device
rejection), at the point where the pinned `zip` crate's real `external_attributes`/`is_symlink()`
surface is read before writing the check.

## Drift entry (copied verbatim from spec §19)

- **DRIFT-002**
  - Spec says: `FR-NEW-013` — a zip entry's symlink-ness is readable via Unix mode bits in
    `external_attributes`, high 16 bits, `S_IFLNK`.
  - Code does: unknown; same not-yet-a-dependency situation as DRIFT-001.
  - Nature: missing capability (unverified until the dependency lands).
  - Resolution during implementation: read the pinned version's `ZipFileData`/`ZipFile` field
    names for `external_attributes` or any native `is_symlink()` method it may already expose
    (the crate's release history suggests this was added in some 2.x release; use it directly if
    present, falling back to the bit-mask approach only if not).
  - Detected by: `E2E-NEW-018` (wrong runtime behavior) or `cargo build` (API mismatch).
  - Blocks which requirement: FR-NEW-013.
  - Status: open.

## Depends On

None.

## Consumed by

US-0007 (symlink/hardlink/device rejection, zip-slip entry-path rejection, void-on-disqualify
ordering). That story must re-verify this drift entry against the actually-pinned `zip` crate
version before implementing the symlink check, preferring a native `is_symlink()` method if the
pinned version exposes one, falling back to the `(external_attributes >> 16) & 0xF000 ==
0xA000` (`S_IFLNK`) bit-mask check otherwise, and close this entry's "Status: open" in its own
completion notes.

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5, applied to US-0007 since that
is where this drift's resolution actually happens. This story itself has no code to review.
