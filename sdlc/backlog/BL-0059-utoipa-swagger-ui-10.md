---
Id: BL-0059
Title: Drop the last zip acceptances once utoipa-swagger-ui ships with zip 9
Nature: DEBT
Source: deferred from SPEC-0035, re evaluated 2026-10-18 (/sdlc-spec BL-0059)
Created: 2026-10-16
Severity: Low
Decision: parked by the owner on 2026-10-18 until upstream releases
Trigger: a utoipa-swagger-ui release whose build dependency on zip is >= 9.0.0
---

## What
`utoipa-swagger-ui = { version = "9", features = ["axum"] }` (`Cargo.toml:116`) build depends on zip 3.0.0, which carries RUSTSEC-2026-0367/0368/0369 (patched `>= 9.0.0`), accepted in `.cargo/audit.toml:13-15` and `deny.toml:21-23` until this entry lands.

## Why parked
- zip 3.0.0 was un-yanked on crates.io; only the three advisories remain.
- The latest release, 10.0.1, still requires `zip ^8.6` (crates.io dependencies API, 2026-10-18), so moving to it keeps all three advisories and also forces utoipa 6 (we are on 5, `Cargo.toml:115`).
- Upstream master already uses `zip = "9.0"` (`utoipa-swagger-ui/Cargo.toml:58`), unreleased.
- Exposure is nil: build time only, unpacking the crate's own vendored Swagger UI archive.

## When the trigger fires
Bump utoipa-swagger-ui (and utoipa if required), remove the three ids from `.cargo/audit.toml` and `deny.toml`, update `security_gate.rs` tests `accepted_advisories_are_recorded_once_with_a_reason` and `zip_acceptances_name_the_build_time_copy`. Suggested: `/sdlc-spec --debt BL-0059`.

## Rejected alternative
Serving a vendored Swagger UI ourselves (`api/openapi.rs:1483` is the only use): larger change for a build time acceptance whose fix is already upstream.
