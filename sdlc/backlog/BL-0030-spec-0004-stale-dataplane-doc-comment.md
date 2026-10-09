---
Id: BL-0030
Title: Fix stale module doc comment and docs gaps in the REST data plane
Nature: DEBT
Source: retro-spec 2026-10-09, SPEC-0004 (rest-data-plane) split
Created: 2026-10-09
Severity: Low
Fingerprint: stale-doc:crates/core/src/api/dataplane.rs:module-doc-comment
---

## What
Three small documentation gaps found while retro-specifying the REST data plane:
1. The module-level doc comment at `crates/core/src/api/dataplane.rs:1-18` still describes
   `mkdir`/`delete`/`move` as part of "the bytes plane" that "talks to the `VolumeClient`
   directly," contradicting the actual implementation (they call `core::fs_ops` through the
   engine, per the in-code rationale comments on each handler).
2. The `read-section` route is present in the router and manifest (`dataplane.rs:123`) but was
   never mentioned in any prose description of the route families.
3. A legacy "unmeasured per-request OpenAPI generation cost" concern was dropped as stale/
   unverified rather than carried forward blindly.

## Why
A stale module comment misleads a future reader about which routes bypass the engine.

## Evidence
`crates/core/src/api/dataplane.rs:1-18,123`.

## Notes
Fix 1 and 2 together (update the module doc comment to the current 4-route storage-direct set
vs. 36-route engine-parity set, and mention `read-section`). Item 3: if the OpenAPI generation
cost is still a live concern, re-raise it as a fresh backlog item with a measurement task rather
than reviving the old TBD.
