# US-0003: Signed URL download — route, atomic consume, concurrency, unconditional mount

> Parent Spec: specs/SPEC-0012_2026-10-07_20-08-29-zip-export-signed-url/spec.md
> Spec ID: SPEC-0012
> Epic: n/a
> Status: ready
> Priority: 3
> Depends On: US-0001, US-0002
> Complexity: L
> min_tier: 1
> Files touched: 3

> **Sizing note:** this story carries 8 FRs and 30 tests, well above the tier-2 nominal budget. It
> was kept whole because every FR here is a branch of **one handler function**
> (`GET /exports/{token}`): happy path, replay, expired, concurrent race, in-flight-crossing-expiry,
> no-auth-required, no-revocation-surface, and unconditional mounting are not separable
> implementations — they are one atomic delete-then-serve operation observed under different timing
> and input conditions. A story containing only "reject replay" has nothing to test without the rest
> of the handler existing. `min_tier` is set to 1. Reviewed and confirmed with the user at the Phase 3
> slicing gate.

## Objective
Implement the one route a recipient (who holds no system credential at all) uses to actually
download an export: `GET /exports/{token}`. This story owns the entire lifecycle of *consuming* a
token — atomically, exactly once, with no authentication check — and the mounting fix (FR-NEW-009b)
that keeps this route alive even when the REST data plane is disabled.

## Technical Context

### Stack
Rust 2024, axum, `tokio`.

### Relevant File Structure
```
crates/core/src/
  exports.rs                        <- NEW: GET /exports/{token} handler + its own router()
  app.rs                            <- merge exports::router() unconditionally
  api/dataplane.rs                  <- promote attachment() to pub(crate) (one-line visibility change)
  storage/meta.rs                   <- read-only reference: US-0001's atomic delete-and-fetch + sweep-query accessors
  storage/volume.rs                 <- read-only reference: VolumeClient.blob (public field)
```

### Existing Patterns
- **Unconditional router merge, the exact precedent to copy:** `app.rs:136-138` merges
  `crate::deleted_projects_screen::router(state.clone())` and
  `crate::trash_screen::router(state.clone())` *outside* the `if state.config.api.enabled { ... }`
  block (`app.rs:141-144`), the same way `/health` (`app.rs:134`) is always mounted. `exports::router`
  must be merged the same way, in the same unconditional block — **not** inside `crate::api::router()`
  (`api/dataplane.rs`), which only merges when `api.enabled` is true. This is exactly FR-NEW-009b,
  found by the spec's own Phase 6 round-3 audit: placing the route inside the REST plane would make
  every minted link permanently dead on an MCP-only deployment.
- `attachment()` helper: `crates/core/src/api/dataplane.rs:635`, signature `fn attachment(data:
  Vec<u8>, mime: &str, name: &str) -> Response`. It has no dependency on `AppState` or auth — promote
  it to `pub(crate)` (one-line change) and call it from `exports.rs` exactly as `download_zip` already
  does (`dataplane.rs:623`): `attachment(bytes, "application/zip", "export.zip")`.
- Atomic consume: US-0001's atomic delete-and-fetch accessor (`storage/meta.rs`) is the entire
  mechanism for FR-NEW-009 (happy path), FR-NEW-010 (replay/unknown — both produce "not found" from
  this same accessor), FR-NEW-011 (expired — the accessor's `expires_at>now` predicate naturally
  excludes it), and FR-NEW-013 (concurrency — the underlying `DELETE ... WHERE token=? AND
  expires_at>?` plus `rows_affected()==1` check is atomic at the database level on every dialect, so
  two concurrent callers racing the same token can never both see success).
- Blob read-then-delete: after the atomic row delete succeeds, read `client.blob.get("export:{token}")`
  then `client.blob.delete("export:{token}")` — `VolumeClient.blob` is public
  (`storage/volume.rs:15`), exactly as `git/odb.rs:42-44` reads/writes/deletes `git:{sha}` with no
  wrapper.
- Expiry is checked **exactly once**, inside the atomic delete-and-fetch call, at request start. Do
  not add a second expiry check anywhere later in the handler (e.g. before writing the response body)
  — that is precisely what FR-NEW-012 forbids, and what makes the 5-minute window safe to keep short
  (a slow transfer is never interrupted, only a slow *arrival* is rejected).
- Error-shape identity: FR-NEW-010 requires the "never existed", "already consumed", and "already
  swept" cases to produce byte-identical `404` responses. Since all three collapse to "the
  delete-and-fetch accessor found no matching row," this falls out for free from reusing the one
  accessor — do not special-case any of the three.

### Data Model (excerpt)
Same `export_links` table and `export:{token}` blob key from US-0001/US-0002. This story **deletes**
rows (never inserts; never lists beyond what the atomic accessor returns).

### Decisions That Govern This Story
- **DEC-004:** The signed URL is self-authorizing — no bearer/membership check on the download route.
  The point of a hand-off link is that the recipient has no system credential at all. Rejected
  requiring the original caller's bearer token on download (defeats the purpose of a shareable link).
  **Implemented by:** FR-NEW-017.
- **DEC-005 / DEC-018:** No revocation capability in v1; short TTL makes it low-value. **Implemented
  by:** FR-NEW-018.
- **DEC-006:** Expiry checked once at request start; an in-flight buffered download completes even
  past the 5-minute mark, because the response is always fully buffered before transmission in this
  codebase's existing pattern. **Implemented by:** FR-NEW-012.
- **DEC-011:** Download route is `GET /exports/{token}`, a wholly new top-level path sibling to
  `/health`/`/api`/`/git`/`/app`, **not** nested under `/api/fs/*` — revised during Phase 6 round 2
  after the auditor found `/api/fs/export/{token}` would collide with `/api/fs/{mount_id}/{action}`
  for any project literally named `export` (axum's router prefers a static segment over a parameter
  at the same tree position). **Implemented by:** FR-NEW-009.
- **DEC-012:** Single-use semantics — first `GET` serves and deletes; replay returns `404`. Rejected
  repeatable-until-expiry. **Implemented by:** FR-NEW-010, FR-NEW-013.
- **DEC-017:** Zip filename returned to the recipient is fixed `export.zip` (no caller-supplied
  name). **Implemented by:** FR-NEW-009.

### Applicable NFRs
- **§7.2 Security:** `Security: internal finding` — this route is the one deliberate authorization
  omission the spec's class sweep (§2.4) covers. Mitigations this story must preserve: UUIDv4 token
  entropy (122 bits, already generated by US-0002), single-use, 5-minute window, no enumeration
  endpoint. Traces/logs on download outcome must not record the full token value, only
  truncated/hashed.
- **§7.5 Observability:** INFO on download outcome: served / expired / replayed (token
  truncated/hashed, never logged in full).
- **§7.4 Reliability:** this route's correctness does not depend on the background sweep (US-0004);
  expired-token cleanup on `GET` (FR-NEW-011) is independent of it.

### Bounded Context
**Signed delivery**: "Unauthenticated, single-use resource access." Key entities: `export_links` row
(consumed/expired transition — this story owns the transition), blob at `export:{token}`.

## Functional Requirements

### FR-NEW-009 [EARS-E]: Signed URL download, happy path
- **EARS:** WHEN `GET /exports/{token}` arrives for a `token` whose `export_links` row exists and
  whose `expires_at` has not passed at the moment of lookup THE system SHALL, in one atomic
  operation, delete the `export_links` row, read the blob bytes at key `export:{token}`, delete
  those bytes, and return them as an `application/zip` attachment named `export.zip` via the
  `attachment()` helper (promoted to `pub(crate)`).
- **Exact names:** route `GET /exports/{token}`; response header `Content-Type: application/zip`;
  `Content-Disposition: attachment; filename="export.zip"`

### FR-NEW-009b [EARS-U]: Route registration independent of the REST data plane
- **EARS:** THE system SHALL register `GET /exports/{token}` unconditionally in
  `crates/core/src/app.rs`, merged the same way as
  `crate::deleted_projects_screen::router`/`crate::trash_screen::router` (`app.rs:136-138`) rather
  than inside `crate::api::router()` (`api/dataplane.rs`), so the route stays reachable even when the
  REST data plane is disabled (`api.enabled: false`, `app.rs:141-144`), since `fs.export_zip`
  (FR-NEW-001) has no such dependency and a link it mints must not become permanently unreachable.
- **Exact names:** new module `crates/core/src/exports.rs`, exposing `pub fn router(state:
  Arc<AppState>) -> Router`, merged in `app.rs` alongside `deleted_projects_screen`/`trash_screen`
  (`app.rs:136-138`)

### FR-NEW-010 [EARS-O]: Replay or unknown token rejected
- **EARS:** IF a `GET /exports/{token}` request arrives for a `token` with no existing
  `export_links` row (never existed, already consumed, or already swept) THEN THE system SHALL
  respond `404 Not Found`, with no response distinguishing which of those three cases applied.
- **Business Rules:** the three causes MUST produce byte-identical response bodies and the same
  status code.

### FR-NEW-011 [EARS-O]: Expired, never-downloaded token rejected
- **EARS:** IF the row exists but `expires_at` has already passed at lookup THEN THE system SHALL
  respond `404 Not Found` (same response shape as FR-NEW-010) and SHALL delete the row and its blob
  bytes as part of handling this request, independent of the background sweep.

### FR-NEW-012 [EARS-S]: In-flight download survives expiry
- **EARS:** WHILE a request's expiry check (FR-NEW-009) has already passed at the moment the
  request began processing THE system SHALL still complete transmitting the already-buffered zip
  bytes to the client, since the response is fully buffered before transmission and `expires_at` is
  evaluated exactly once at request start.

### FR-NEW-013 [EARS-E]: Atomic single-use consumption
- **EARS:** WHEN two concurrent `GET /exports/{token}` requests race for the same `token` THE system
  SHALL serve the zip to exactly one of them (whichever wins the atomic row-delete) and respond `404
  Not Found` to the other, per FR-NEW-010.

### FR-NEW-017 [EARS-UB]: No bearer/membership check on download
- **EARS:** THE system SHALL NOT require an `Authorization`/forwarded-identity header or
  project-membership check on `GET /exports/{token}`; the token itself is the sole authorization.

### FR-NEW-018 [EARS-UB]: No revocation surface
- **EARS:** THE system SHALL NOT expose any tool, route, or CLI verb to invalidate an
  `export_links` row before its natural expiry or consumption. (This story's half: no `DELETE
  /exports/{token}` route exists. US-0002 already verified no tool/CLI/admin.* entry exists.)

## Acceptance Tests

> **100% must pass.** Run through `cargo test --workspace`. Loop fix/run/check until zero failures.

### Test Data
| Data | Description | Source | Status |
|------|-------------|--------|--------|
| token `T` | created via US-0002's `fs.export_zip` (or directly via US-0001's insert accessor if US-0002 isn't runnable in isolation) | test fixture | ready |
| `T1`/`T2`/`T3` | never-created / expired-via-direct-DB-update / already-downloaded, for the cross-case identity test | test fixture | ready |
| `stranger@test.com` | confirmed non-member via `admin.list_members` | test fixture | ready |
| server instance with `api.enabled: false` | for FR-NEW-009b's tests | test fixture | ready |

### E2E-NEW-015: SC-002 happy path, correct content-type and filename
- **Category:** Happy path / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- Given export `T` exists, unconsumed, unexpired / When `GET /exports/T` with no `Authorization`
  header / Then status is `200 OK`, `content-type: application/zip`,
  `content-disposition: attachment; filename="export.zip"` / And the body, parsed as a zip, has
  exactly 2 entries `src/main.rs` and `docs/readme.md` with byte-exact content
- **Priority:** Critical

### E2E-NEW-016: side effect, export_links row deleted atomically on download
- **Category:** Side effect / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- Given export `T` row confirmed present / When `GET /exports/T` returns `200` / Then `SELECT
  COUNT(*) FROM export_links WHERE token='T'` is `0` immediately after
- **Priority:** Critical

### E2E-NEW-017: side effect, blob bytes deleted on successful download
- **Category:** Side effect / **Scenario:** SC-002 / **Requirements:** FR-NEW-009
- Given blob `export:T` confirmed present / When `GET /exports/T` returns `200` / Then a direct
  read of `export:T` after the request returns the not-found variant
- **Priority:** Critical

### E2E-NEW-018: security, download succeeds with zero auth header and with a garbage header
- **Category:** Security / **Scenario:** SC-002 / **Requirements:** FR-NEW-017
- Given two independently-created tokens `T_a`, `T_b` / When `GET /exports/T_a` with no
  `Authorization` header, and `GET /exports/T_b` with `Authorization: Bearer garbage-token` / Then
  both return `200 OK` with correct zip content; neither ever returns `401`/`403`
- **Priority:** Critical

### E2E-NEW-019: security, a non-member of the mount can still consume a valid token
- **Category:** Security / **Scenario:** SC-002 / **Requirements:** FR-NEW-017
- Given `owner@test.com` created token `T`; `stranger@test.com` confirmed non-member / When the
  download `GET` carries `stranger@test.com`'s bearer token / Then response is `200 OK` with correct
  zip content
- **Priority:** High

### E2E-NEW-020: failure, token that never existed returns 404
- **Category:** Failure / **Scenario:** SC-002 / **Requirements:** FR-NEW-010
- Given no export ever created for this token / When `GET
  /exports/00000000-0000-0000-0000-000000000000` / Then status is exactly `404`, body contains none
  of `"export_links"`, `"expired"`
- **Priority:** Critical

### E2E-NEW-021: failure, malformed token returns 404 not 400/500
- **Category:** Failure / **Scenario:** SC-002 / **Requirements:** FR-NEW-010
- Given no setup needed / When `GET /exports/not-a-valid-uuid-at-all` / Then status is exactly `404`
  (not `400`, not `500`)
- **Priority:** High

### E2E-NEW-023: SC-003 setup, first download succeeds
- **Category:** Happy path (setup) / **Scenario:** SC-003 / **Requirements:** FR-NEW-010
- Given token `T` created / When `GET /exports/T` once / Then status is `200 OK`
- **Priority:** Critical

### E2E-NEW-024: SC-003 replay, second GET on same token returns 404
- **Category:** Failure / **Scenario:** SC-003 / **Requirements:** FR-NEW-010
- Given `T` already consumed (E2E-NEW-023) / When `GET /exports/T` a second time / Then status is
  exactly `404`, body byte-identical to E2E-NEW-020's captured body
- **Priority:** Critical

### E2E-NEW-025: side effect, replay does not re-delete or error on absent row
- **Category:** Side effect / **Scenario:** SC-003 / **Requirements:** FR-NEW-009, FR-NEW-010
- Given `T` already consumed, row/blob already gone / When `GET /exports/T` a second time / Then no
  panic, no `500`, exactly `404` / And `export_links` row count for `T` remains `0`
- **Priority:** High

### E2E-NEW-026: data integrity, 404 body identical across never-existed/expired/replayed
- **Category:** Data integrity / **Scenario:** SC-003, SC-004 / **Requirements:** FR-NEW-010
- Given `T1` (never created), `T2` (created, `expires_at` set to the past via direct DB update),
  `T3` (created, then downloaded once) / When `GET` is issued for each / Then all three return `404`
  with byte-identical bodies (same content-length, same bytes)
- **Priority:** High

### E2E-NEW-027: SC-004 setup, unexpired never-downloaded token succeeds
- **Category:** Happy path / **Scenario:** SC-004 / **Requirements:** FR-NEW-011
- Given token `T` created, `expires_at` unmodified (5 min future) / When `GET /exports/T`
  immediately / Then status is `200 OK` with correct zip content
- **Priority:** Critical

### E2E-NEW-028: SC-004 failure, expired never-downloaded token returns 404
- **Category:** Failure / **Scenario:** SC-004 / **Requirements:** FR-NEW-011
- Given token `T`, `expires_at` directly updated to `now - 1s` / When `GET /exports/T` / Then status
  is exactly `404`, body shape identical to E2E-NEW-020
- **Priority:** Critical

### E2E-NEW-029: side effect, expired-token GET deletes row and blob immediately
- **Category:** Side effect / **Scenario:** SC-004 / **Requirements:** FR-NEW-011
- Given same setup as E2E-NEW-028, row/blob confirmed present before the GET / When `GET
  /exports/T` returns `404` / Then row for `T` is gone (count 0) and blob at `export:T` is gone,
  both immediately after the response (not deferred to the sweep)
- **Priority:** Critical

### E2E-NEW-030: boundary, GET exactly at and exactly after the expiry instant
- **Category:** Edge / **Scenario:** SC-004 / **Requirements:** FR-NEW-011
- Given token `T` with `expires_at` set to `now + 100ms`; token `T2` created identically / When (a)
  `GET /exports/T` immediately; (b) sleep 150ms then `GET /exports/T2` / Then (a) returns `200`; (b)
  returns `404`
- **Priority:** High

### E2E-NEW-037: concurrency, two concurrent GETs race on one token, exactly one wins
- **Category:** Concurrency / **Scenario:** SC-002 / **Requirements:** FR-NEW-013
- Given token `T`, row/blob present / When two `GET /exports/T` requests are issued concurrently
  (`tokio::join!` on two `oneshot` calls against the same `Arc<AppState>`) / Then exactly one
  response is `200` with correct zip content, the other is exactly `404` / And after both complete,
  row count for `T` is `0` and blob is gone
- **Priority:** Critical

### E2E-NEW-038: SC-002 edge, in-flight download crossing the expiry boundary
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-012
- Given token `T`, `expires_at` set to `now + 50ms`; the blob-read step is delayed ~150ms (test
  seam) so `expires_at` elapses while the body is being produced / When `GET /exports/T` is issued
  before expiry, and the atomic delete commits before expiry / Then the response still completes
  with `200` and the complete, correct zip bytes, and the test asserts its own injected delay
  genuinely crossed `expires_at`
- **Priority:** Critical
- **Implementation note:** if a reliable test seam for injecting delay between the atomic delete and
  byte transmission proves awkward, prefer proving the ordering via code inspection (the delete must
  happen before any byte is read) plus a smaller deterministic unit test on the handler's internal
  sequencing, over a flaky wall-clock race.

### E2E-NEW-046: edge, expiry is checked exactly once at request start, never re-checked mid-transmission
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-012
- Given a token whose atomic delete-and-read has already committed / When the handler proceeds to
  transmit the bytes, with `expires_at` artificially rewound to the past on a stale in-memory clone /
  Then transmission still completes with `200` and full correct bytes, proving no second expiry
  check exists anywhere after the atomic step
- **Priority:** High

### E2E-NEW-047: edge, an expired token's request is rejected even while an unrelated in-flight download is still transmitting
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-012
- Given token `T_live` mid-transmission (artificially delayed) and token `T_expired` whose
  `expires_at` has already passed, never downloaded / When `GET /exports/T_expired` is issued while
  `T_live`'s response is still being written / Then `T_expired`'s request returns `404` immediately,
  unaffected by `T_live`'s in-flight state
- **Priority:** Medium

### E2E-NEW-048: concurrency, three concurrent GETs race on one token, exactly one wins
- **Category:** Concurrency / **Scenario:** SC-002 / **Requirements:** FR-NEW-013
- Given token `T`, row/blob present / When three `GET /exports/T` requests are issued concurrently /
  Then exactly one is `200` with correct content, the other two are exactly `404`
- **Priority:** Medium

### E2E-NEW-049: side effect, the race's losing requests see a truly absent row
- **Category:** Side effect / **Scenario:** SC-002 / **Requirements:** FR-NEW-013
- Given the race from E2E-NEW-037/048 / When both/all losing requests return `404` / Then a direct
  DB query immediately after confirms zero rows for `T`
- **Priority:** Medium

### E2E-NEW-052: security, download succeeds even with a forwarded-identity header present and invalid
- **Category:** Security / **Scenario:** SC-002 / **Requirements:** FR-NEW-017
- Given a valid token `T` / When `GET /exports/T` with header `X-Forwarded-Authorization: Bearer
  garbage` / Then status is exactly `200` with correct zip content
- **Priority:** Medium

### E2E-NEW-054: security (structural), no DELETE/invalidate route exists for an export
- **Category:** Security (structural) / **Scenario:** SC-002 / **Requirements:** FR-NEW-018
- Given a valid token `T` / When `DELETE /exports/T` is issued / Then the response is `404` or `405`
  (no such route/method exists), and a subsequent `GET /exports/T` still succeeds with `200`
- **Priority:** Medium

### E2E-NEW-055: edge, a replay attempt with a case-differing token also returns 404
- **Category:** Edge / **Scenario:** SC-003 / **Requirements:** FR-NEW-010
- Given token `T` already consumed (lowercase UUID string) / When `GET /exports/{T.to_uppercase()}`
  is issued / Then status is exactly `404` (no case-insensitive fallback lookup exists)
- **Priority:** Low

### E2E-NEW-057: edge, download route reachable when api.enabled=false
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-009b
- Given a server instance with `api.enabled: false` (MCP-only), token `T` created via
  `fs.export_zip` / When `GET /exports/T` / Then status is `200 OK` with correct zip content (not
  `404`, which is what it would be if the route lived inside `api::router()`)
- **Priority:** Critical

### E2E-NEW-058: structural, export download router is merged unconditionally
- **Category:** Structural / **Scenario:** SC-002 / **Requirements:** FR-NEW-009b
- Given `crates/core/src/app.rs`'s router assembly / When checking where `exports::router` is
  merged / Then it is merged alongside `deleted_projects_screen::router`/`trash_screen::router`
  (unconditional block), never inside the `if state.config.api.enabled { ... }` block
- **Priority:** High

### E2E-NEW-059: edge, full create-then-download round trip succeeds on an MCP-only deployment
- **Category:** Edge / **Scenario:** SC-002 / **Requirements:** FR-NEW-009b
- Given a server instance with `api.enabled: false` / When `fs.export_zip({"mount_id": "proj",
  "paths": ["/src/main.rs"]})` returns a URL, then that URL is downloaded / Then the download
  succeeds with `200` and correct content, proving the whole feature works end to end on an MCP-only
  server
- **Priority:** Critical

## Constraints

### Files Not to Touch
- `crates/core/src/tools/export.rs` — US-0002's file, do not duplicate creation logic here.
- `crates/core/src/storage/meta.rs` — call US-0001's accessors, do not add a second delete path.
- `crates/core/src/api/dataplane.rs` — touch **only** `attachment()`'s visibility (`pub` ->
  `pub(crate)`). Do not add the download route here.

### Dependencies Not to Add
- No new crate.

### Patterns to Avoid
- Do not add a second expiry check anywhere after the atomic delete-and-fetch call (violates
  FR-NEW-012).
- Do not special-case "never existed" vs "consumed" vs "swept" into different response bodies
  (violates FR-NEW-010).
- Do not add a `DELETE`/invalidate route or admin tool (violates FR-NEW-018 / DEC-005).
- Do not merge `exports::router` inside `crate::api::router()` or inside the `if api.enabled` block.

### Scope Boundary
- This story does not implement the purge sweep (US-0004) — expired, never-downloaded tokens are
  cleaned up by the sweep independently; this story only cleans up an expired token when a `GET`
  happens to hit it (FR-NEW-011), which is opportunistic, not the primary cleanup path.

## Non Regression

### Existing Tests That Must Pass
- `download`/`download_zip`'s existing tests, unmodified.
- Every existing `app.rs` router test (notably the `/health` test at `app.rs:478`).

### Behaviors That Must Not Change
- `/health`, `/api/*`, `/git/*`, `/app/*` routing is unaffected by the new unconditional merge.
- `attachment()`'s behavior is unchanged; only its visibility modifier changes.

### API Contracts to Preserve
- No existing route's shape changes.

## Self-Review Checklist
Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
