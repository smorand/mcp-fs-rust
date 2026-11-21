# US-0267: the settle delay setting

> Spec: SPEC-0100 (sdlc/specs/SPEC-0100-derived-metadata-rebuilds-after-the-file-settles/)
> Nature: FEAT
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 4
> Closes spec: no

## Objective
Add `search.settle_delay_seconds` to the server configuration: a whole number of seconds, 0 or more, 30 when absent or empty, refused at start with the spec text otherwise, whether or not search is enabled.

## Requirements covered
- SPEC-0100/FR-NEW-014 (partial: the key and its parsing; the default is used by US-0268): The system SHALL read the settle delay as a whole number of seconds, 0 or more, from the key `search.settle_delay_seconds` of the server configuration.
- SPEC-0100/FR-NEW-016: IF the configured settle delay is negative or not a whole number THEN THE system SHALL refuse to start with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`.

## Independent Test
Run `cargo test -p fsa-core --lib -- e2e_0100_025 e2e_0100_071 e2e_0100_074 config::`. They load YAML configs with the values `-5`, `2.5`, `abc`, empty, absent and `0` through `ServerConfig::from_yaml` plus `validate()` and assert the exact error text or the parsed delay.

## Design excerpt
### Decisions
- DEC-013 (SPEC-0100): `SearchConfig.settle_delay_seconds` is a `String` filled by a deserializer that accepts a YAML integer, float or string, and is parsed in `validate()` before the `enabled` early return. **Obliged by:** SPEC-0100/FR-NEW-014, FR-NEW-016. **Rationale:** `2.5` and `abc` must reach the spec's message, not a serde error; the check applies when search is disabled. **Alternatives considered:** `u64` field (serde error text). **Source:** existing code. **Evidence:** `config.rs:428` (`serde_norway::Value` deserialize precedent), `config.rs:1173-1175`.
### Data model
None.
### Contracts
YAML `search: { settle_delay_seconds: 30 }` (integer, float or string scalar accepted by the deserializer; `${VAR}` expanded before parsing). `SearchConfig::settle_delay() -> u64` returns the parsed value after validation. Error: `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`. Absent or empty: 30.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/config.rs` | modify: field `settle_delay_seconds: String` with a scalar deserializer, `settle_delay()`, check in `validate()` before the `!cfg.enabled` return of `validate_search` |
| `config/local.yaml.template` | modify: commented example under `search:` |
| `.agent_docs/config.md` | modify: key, default, error |
| `crates/core/src/search/derived_e2e.rs` | modify: the three config tests |
### Existing patterns
Deserializer precedent from a `serde_norway::Value`: `config.rs:420-440`. Validation of the search block: `config.rs:1173-1195` (the early return for a disabled search is why the check must go before it).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

### SPEC-0100/E2E-025: invalid settle delay
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_025_*`)
- **Given** a server configuration with settle delay `-5`, then `1.5`, then `abc`
- **When** the operator starts the server
- **Then** each start stops with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`
- **And** with the setting absent the server starts with 30 s
- **Priority:** High

### SPEC-0100/E2E-071: an invalid settle delay with search disabled
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_071_*`)
- **Given** the server configuration has search disabled and a settle delay of `-5`
- **When** the operator starts the server
- **Then** the start stops with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`
- **And** with the delay corrected the server starts
- **Priority:** High

### SPEC-0100/E2E-074: a settle delay that is not a whole number
- Level / Driver / Location: functional e2e / `McpServer` handler on the shared fixture, clock paused / `crates/core/src/search/derived_e2e.rs` (`e2e_0100_074_*`)
- **Given** the server configuration has a settle delay of `2.5`, then of `abc`
- **When** the operator starts the server each time
- **Then** each start stops with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`
- **And** a delay of `0` starts the server
- **Priority:** Medium

## Constraints
- Do not read the value anywhere except `settle_delay()`; the scheduler uses it in US-0268.
- Do not add a CLI flag.
- `0` is valid.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code.
- [ ] Every acceptance test above green; `make check` green on the work branch after this commit; main only receives the whole squash at landing.
- [ ] Non regression: the tests of the touched modules keep passing unchanged, except those named under Paths as modified.
- [ ] Docs updated: `.agent_docs/config.md` and the YAML template.
- [ ] Self review: every requirement above is met by a named test; every error text equals the design contract; no dash used as punctuation in code, comments or docs; no secret logged.

- [ ] One commit `US-0267: the settle delay setting` on the work branch, body with `Summary:` (2 to 4 plain lines), `Spec: SPEC-0100` and the requirement ids.
