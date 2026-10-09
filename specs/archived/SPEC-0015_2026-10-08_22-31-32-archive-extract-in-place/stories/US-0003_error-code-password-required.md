---
Parent Spec: SPEC-0015 — Archive Extraction In Place
Spec ID: SPEC-0015
Epic: n/a
Status: ready
Priority: P0
Depends On: none
Complexity: XS
min_tier: 2
Files touched: crates/core/src/errors.rs
---

# US-0003 — New error code `ERR_PASSWORD_REQUIRED`

## Objective

Add the `ERR_PASSWORD_REQUIRED` error code to the existing 14-code `ToolError` enum in
`crates/core/src/errors.rs`, with its constructor, HTTP status mapping, and inclusion in both
existing exhaustiveness tests. Pure infra, zero dependency on `archive.rs`; independently
testable today, before any archive-parsing code exists.

## Technical Context

### Stack

Rust 2024, no new dependency. This story touches only `crates/core/src/errors.rs`.

### Relevant File Structure

- `crates/core/src/errors.rs` — defines 14 `ERR_*` codes (`errors.rs:9-22`), each with an
  `http_status()` match arm (`errors.rs:96-124`), and three exhaustiveness tests:
  `every_code_has_an_http_status` (`errors.rs:178-195`), `client_caused_codes_are_all_4xx`
  (`errors.rs:199-221`), `no_code_falls_through_to_an_accidental_500` (`errors.rs:225-246`).

### Existing Patterns Referenced

Follow the exact shape of any existing client-caused code (e.g. `ERR_NOT_FOUND` or
`ERR_WRITE_QUOTA_EXCEEDED`) in `errors.rs`: a `code` module constant, a `ToolError::<name>(msg:
impl Into<String>) -> Self` constructor, an `http_status()` arm, and inclusion in both
`client_caused` arrays referenced by `client_caused_codes_are_all_4xx` and
`no_code_falls_through_to_an_accidental_500`.

### Decisions That Govern This Story

None specific to this story beyond the requirement itself (`FR-NEW-028`).

### Applicable NFRs

None beyond existing conventions: never log credentials (`AGENTS.md` inherited `rust` skill
rule) — not directly applicable here since this story adds no logging, but the error message
itself must never echo a password (enforced at the call site in later stories, not here).

### Bounded Context

Not explicitly one of the three `§4.5` bounded contexts (archive parsing / filesystem write /
transport); this is a pure infra addition to the shared error-code module all three contexts
read from.

## Functional Requirements

**FR-NEW-028** [EARS-U] The system SHALL add a new error code `ERR_PASSWORD_REQUIRED` to
`crates/core/src/errors.rs`'s `code` module, a constructor `ToolError::password_required(m: impl
Into<String>) -> Self`, an `http_status()` match arm returning `428`, and an addition of
`code::PASSWORD_REQUIRED` to the `client_caused` array in
`no_code_falls_through_to_an_accidental_500` (`errors.rs:225-246`) and to the assertion list in
`client_caused_codes_are_all_4xx` (`errors.rs:199-221`), bringing the total `ERR_*` code count from
14 to 15.

**Inputs/Outputs**: constructor takes any `impl Into<String>` message, returns a `ToolError`
whose `.code` is `"ERR_PASSWORD_REQUIRED"` and whose `.http_status()` is `428`.

**Business Rules**: this code is reused verbatim for both the missing-password and
wrong-password cases (spec §13 Consistency Notes) — no second code is ever introduced for the
same remediation path. That reuse happens in later stories (US-0006); this story only adds the
code itself.

## Acceptance Tests

### Test Data

| Input | Expected |
|---|---|
| `ToolError::password_required("x")` | `.code == "ERR_PASSWORD_REQUIRED"` |
| `ToolError::password_required("x")` | `.http_status() == 428` |
| `ToolError::password_required("bad guess")` | `.to_string() == "ERR_PASSWORD_REQUIRED: bad guess"` |

#### E2E-NEW-030 — `ERR_PASSWORD_REQUIRED` maps to HTTP 428

- Category: Failure
- Scenario: (FR-only)
- Requirements: FR-NEW-028
- Preconditions: none (unit-level, `errors.rs`'s own test module)
- Steps:
  - Given: `ToolError::password_required("x")`
  - When: `.http_status()` is read
  - Then: it equals `428`
  - And: `.code == "ERR_PASSWORD_REQUIRED"`
  - And: it is present in both `client_caused_codes_are_all_4xx`'s array and
    `no_code_falls_through_to_an_accidental_500`'s array (both existing tests still pass,
    extended, not rewritten)
- Cleanup: none
- Priority: P0

#### E2E-NEW-045 — `ToolError::password_required` Display format

- Category: Failure
- Scenario: (FR-only)
- Requirements: FR-NEW-028
- Preconditions: none
- Steps:
  - Given: `ToolError::password_required("bad guess")`
  - When: `.to_string()` is read
  - Then: it equals `"ERR_PASSWORD_REQUIRED: bad guess"`, mirroring the existing
    `display_is_code_colon_message` test's exact assertion style
- Cleanup: none
- Priority: P2

#### E2E-NEW-046 — `is_client_error()` is true for `password_required`

- Category: Edge
- Scenario: (FR-only)
- Requirements: FR-NEW-028
- Preconditions: none
- Steps:
  - Given: `ToolError::password_required("x")`
  - When: `.is_client_error()` is read
  - Then: it is `true`, added to the existing `client_caused_codes_are_all_4xx` test's array
    rather than as a freestanding assertion, mirroring how every other client-caused code is
    checked there
- Cleanup: none
- Priority: P2

## Constraints

- Files Not to Touch: `crates/core/src/tools/archive.rs` does not exist yet; this story does not
  create it.
- Dependencies Not to Add: none.
- Patterns to Avoid: do not invent a second error code for the wrong-password case; one code
  covers both missing and wrong password (spec §13).
- Scope Boundary: error code plumbing only. No archive logic, no tool registration.

## Non Regression

The three existing exhaustiveness tests (`every_code_has_an_http_status`,
`client_caused_codes_are_all_4xx`, `no_code_falls_through_to_an_accidental_500`) must still pass
after being extended with the new code — they are extended, not rewritten (spec §9.3).

## Self-Review Checklist

Tier 2: Full 4-axis self-review per `/implement` Phase 3.3 Step 5.
