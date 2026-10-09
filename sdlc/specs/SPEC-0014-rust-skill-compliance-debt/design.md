> Id: SPEC-0014
> Nature: DEBT
> Status: Resolved
> Migrated from: specs/archived/SPEC-0012_2026-09-26_16-04-00-rust-skill-compliance-debt/spec.md on 2026-10-09
> Renumbered: legacy id was SPEC-0012; new id SPEC-0014 (collision avoidance)

# Rust coding-standards compliance — Technical Debt Design

## 1. Components

- **Root `Makefile`** (DBT-001 / DR-001): wraps `build.sh`, `test.sh`, `run.sh` plus bare `cargo clippy`/`cargo fmt`/`cargo audit`/`cargo deny` invocations behind `make sync/build/release/test/lint/lint-fix/format/format-check/typecheck/security/check/run/run-dev/clean/help`.
- **`crates/core` package** (DBT-002 / DR-002): Cargo package `mcp-fs-core`, library crate `mcp_fs_core`, holding every module formerly under `crates/mcp-fs/src/` except `main.rs`. `crates/mcp-fs` retains only `src/main.rs` and a path dependency on `mcp-fs-core`.
- **`rust-toolchain.toml`** (DBT-003 / DR-003): pins `channel = "1.88"`, matching the workspace `rust-version`.
- **`deny.toml`** (DBT-004 / DR-004): cargo-deny 0.20+ schema, denies yanked crates and wildcard deps, warns on duplicate versions, allowed-license list including `MIT`. Wired into `make security` (`cargo audit && cargo deny check`).
- **Config resolution internals** (DBT-005 / DR-005, partially applied — see Decisions DEC-002): `resolve_config_path_with` in `crates/core/src/cli.rs` resolves the XDG base directory via `etcetera::choose_base_strategy()` instead of hand-rolled `$XDG_CONFIG_HOME`/`$HOME` logic. `ServerConfig::load`/`from_yaml` in `crates/core/src/config.rs` was **not** swapped to `figment::providers::Yaml`; it still calls `serde_yaml::from_str` directly, preserving the custom `HostMap` `Visitor`'s duplicate-key rejection in `crates/core/src/git/remote.rs`.

## 2. Flows

1. **Build/test entry point:** a contributor or agent runs `make check` → fmt-check → clippy (`-D warnings`) → typecheck → security (`cargo audit` + `cargo deny check`) → test-cov → doc, instead of discovering three standalone scripts.
2. **Crate resolution:** `crates/mcp-fs/src/main.rs` parses CLI args via clap and calls into `mcp_fs_core::*`; all logic (api, app, cli, config, core, docs, errors, git, identity, keys, logging, migrate, purge, safety, search, state, storage, tools, util, screens) resolves from the `mcp-fs-core` library crate, not from a `mod` tree inside the binary package.
3. **Config load:** `mcp-fs serve` → `resolve_config_path_with` (explicit `--config` → `$MCP_FS_CONFIG` → `etcetera`-resolved XDG dir → `${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}.yaml`) → read file text → `expand_env` substitutes `${VAR}`/`${VAR:-default}` → `serde_yaml::from_str` deserializes into `ServerConfig`, with `HostMap`'s custom `Visitor` rejecting a duplicate `git.hosts` key at `git/remote.rs:168`.

## 3. Interfaces

- `mcp-fs` binary CLI surface unchanged: `serve`/`keys`/`token`/`migrate`/`purge`/`version` verbs, `--config` flag, `$MCP_FS_CONFIG`/`$MCP_FS_CONFIG_DIR`/`$MCP_FS_CONFIG_NAME`/`$XDG_CONFIG_HOME` env vars, exact precedence order preserved.
- `make <target>` is additive; `build.sh`/`test.sh`/`run.sh` remain callable directly, unmodified.
- `mcp_fs_core` library crate (package `mcp-fs-core`, directory `crates/core`) is the one place every `fs.*`/`git.*`/REST handler's logic lives; `crates/mcp-fs` is a 9-line thin entry point.
- 102-tool MCP contract (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`) and REST `/api/fs` surface: untouched by this lot (confirmed: neither `mcp/` nor `api/` content changed, only their location on disk after the crate move).

## 4. Data and state

No schema change, no new data model, no new persisted state. Config file format, `${VAR}` substitution syntax, and YAML deserialization semantics (including the duplicate-key rejection) are unchanged.

## 5. Configuration

- New: `rust-toolchain.toml` (`channel = "1.88"`), `deny.toml` (license allow-list, advisory/ban rules).
- `[workspace.dependencies]` gained `etcetera = "0.11"`. `figment` was added then removed (see DEC-002 / DDRIFT-002): not present in the final dependency tree.
- No new config *keys* for `ServerConfig` itself; XDG resolution mechanism changed internally (etcetera) but exposed env vars (`MCP_FS_CONFIG`, `MCP_FS_CONFIG_DIR`, `MCP_FS_CONFIG_NAME`, `XDG_CONFIG_HOME`) are unchanged.

## 6. Observability

No change. `logging.rs` (stderr `tracing_subscriber::EnvFilter`) was explicitly excluded from this lot (OpenTelemetry/OTLP wiring is new capability, not debt — see Findings).

## 7. Decisions

### DEC-001: `crates/core` package named `mcp-fs-core` / `mcp_fs_core`, not `core`
**Decision:** directory `crates/core` (matches skill-mandated layout), but Cargo package name `mcp-fs-core` and library crate name `mcp_fs_core`.
**Rationale:** avoids ambiguous/shadowed references to Rust's own implicit sysroot `core` crate, which is part of every crate's extern prelude.
**Alternatives rejected:** naming the package literally `core` and relying only on `[lib] name` to disambiguate — rejected because the package name itself still participates in dependency resolution and error messages.
**Status:** implemented. Confirmed on disk: `crates/core/` directory, package `mcp-fs-core`.

### DEC-002: `figment` not adopted for `ServerConfig::load`; XDG swap to `etcetera` kept
**Decision:** `resolve_config_path_with`'s XDG branch uses `etcetera::choose_base_strategy()` (applied). `ServerConfig::load`/`from_yaml` keeps `serde_yaml::from_str` directly rather than `figment::providers::Yaml` (not applied; `figment` dependency removed).
**Rationale:** `figment::Figment::extract()` always materializes an intermediate keyed `Value` before handing data to `Deserialize`, which collapses a duplicate YAML mapping key to its last value — before `git/remote.rs`'s custom `HostMap` `Visitor` (which deliberately keeps every entry to detect a duplicate `git.hosts` key) ever sees more than one entry. This broke `a_duplicate_git_hosts_key_in_yaml_fails_boot` and `e2e_new_006_duplicate_host_fails_boot`, both asserting that a config declaring the same host twice fails boot loudly — a genuine security-relevant guard, not incidental test fragility. Since `figment`'s only real benefit (the `defaults < file < env < CLI` layering) was already explicitly out of scope per the original DR-005 narrowing, adopting it bought nothing while regressing that guard.
**Alternatives rejected:** a `git.hosts`-specific pre-scan feeding a `figment`-based parse for the rest of the file — rejected as added complexity for a library offering no used capability.
**Status:** resolved, won't-implement. Documented in `drift/2026-09-26_17-19-40.md` (DDRIFT-002), closed 2026-09-27. No further action; a future pass may formally drop the `figment` clause from DR-005's canonical wording.

### DEC-003: `rmcp` SDK migration and OpenTelemetry/OTLP wiring excluded from this lot
**Decision:** both audited gaps excluded from the DEBT spec.
**Rationale:** `rmcp` migration would force every client to send `initialize` before `tools/call`, breaking the frozen no-handshake wire contract (`DEC-107`) that the 102-tool contract (now `TOOL_CONTRACT.txt`/`tool-contract-golden.json`) depends on — an observable behavior change, not pure debt. OpenTelemetry/OTLP wiring adds new network egress and new config keys — new capability, not a reshuffle.
**Status:** still excluded as of 2026-10-09. Verified: `crates/core/src/mcp.rs` and `crates/core/src/mcp/` contain no `rmcp`-handshake-mandatory framing change; `rg "opentelemetry|tracing_opentelemetry|otel|OTLP"` across `crates/` and `Cargo.toml` returns no matches (not independently re-run in this pass, but no such dependency appears in `Cargo.toml`). See Findings for backlog recommendation.

## 8. Requirement to code map

| Req | Code | Status |
|---|---|---|
| DR-001 (Makefile) | `Makefile` (root) | Done — verified present on disk |
| DR-002 (`crates/core` split) | `crates/core/` (package `mcp-fs-core`), `crates/mcp-fs/src/main.rs` (9 lines, only file) | Done — verified: `crates/mcp-fs/src/` contains only `main.rs`; `crates/core/src/` holds all logic modules |
| DR-003 (toolchain pin) | `rust-toolchain.toml` | Done — verified `channel = "1.88"` |
| DR-004 (security gate) | `deny.toml`, `make security` | Done — verified `deny.toml` present |
| DR-005 (config internals) | `crates/core/src/cli.rs:333` (`etcetera::choose_base_strategy()`), `crates/core/src/config.rs` (`serde_yaml::from_str`, unchanged) | Partially done, by deliberate decision (DEC-002): XDG half applied, YAML-parser half won't-implement |
| DR-006 (behavior invariance) | all of the above | Done — no declared break; duplicate-host guard explicitly preserved |
| DR-007 (config precedence preserved) | `crates/core/src/cli.rs` tests `explicit_path_wins_over_everything`, `xdg_config_home_is_probed_before_the_dir_name_pair`, etc. | Done |
| DR-008 (no wire-contract surface touched) | `crates/core/src/mcp.rs`, `crates/core/src/api/` | Done — moved verbatim, not modified |

## 9. Legacy mapping

Source: specs/archived/SPEC-0012_2026-09-26_16-04-00-rust-skill-compliance-debt/spec.md (pre-move, renumbered to SPEC-0014); plus its drift/ directory

| Old id | New id | Note |
|---|---|---|
| DBT-001 | DEC (component, §1) | No numeric debt-item renumbering needed; folded into Components/Requirement map, status Done |
| DBT-002 | DEC-001 | Crate-naming decision kept verbatim rationale; status Done |
| DBT-003 | (component, §1) | Status Done |
| DBT-004 | (component, §1) | Status Done |
| DBT-005 | DEC-002 | Scope-narrowing decision superseded by won't-implement decision on the `figment` half (DDRIFT-002); XDG half Done |
| DR-001..DR-008 | §8 Requirement to code map | All renumbered rows kept verbatim ids, statuses updated from spec-time "Draft" to current verified state |
| DDEC-001 | DEC-001 | Verbatim rationale carried over |
| DDEC-002 | DEC-002 | Superseded in part by DDRIFT-002's later resolution; both folded into one decision entry |
| DDEC-003 | DEC-003 | Verbatim rationale carried over; re-flagged as still-outstanding backlog candidate |
| DT-001..DT-012 | n/a | Test-level detail not reproduced here; verified structurally (file layout checks) rather than by re-running `cargo test` in this read-only pass |
| DDRIFT-001 (etcetera/figment version pinning) | n/a | Resolved at implementation time (`etcetera = "0.11"` pinned); no longer open |
| DDRIFT-002 (figment duplicate-host collapse) | DEC-002 | Resolution and rationale folded into DEC-002 |
