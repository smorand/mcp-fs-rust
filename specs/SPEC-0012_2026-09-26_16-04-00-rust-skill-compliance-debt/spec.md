# Rust coding-standards compliance — Technical Debt Specification

> Generated on: 2026-09-26
> Id: SPEC-0012
> Nature: DEBT
> Depth: M
> Depth evidence: 5 kept debt items across 2 workspace members (`crates/mcp-fs`, `crates/agent`) plus
>   root tooling files (`Makefile`, `deny.toml`, `rust-toolchain.toml`); no schema change; no new data
>   model; ~11 requirements. Two audited candidate items (rmcp migration, OpenTelemetry wiring) were
>   excluded at preflight because they fail the "no observable behavior change" test — see "Excluded
>   from this lot" below. Escalated from S to M because the crate split (DBT-002) touches every module
>   under `crates/mcp-fs/src/`, which is more than one module.
> Status: Draft
> Behavior change: none
> From backlog: n/a
> Split: not split
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

---

## Preflight guard verdict (per gap item, before Phase 1)

The audit against the personal `rust` coding-standards skill listed 7 gaps. Each is routed here
explicitly, with the evidence that decided it.

| # | Gap | Verdict | Evidence | Why |
|---|---|---|---|---|
| 1 | No Makefile | **KEPT — DBT-001** | `build.sh:1-5`, `test.sh:1-5`, `run.sh:1-22` exist and are the documented interface (`AGENTS.md:36-41`, `README.md:257`) | Wrapping the existing scripts in `make` targets adds no observable behavior: same underlying `cargo build/test/clippy/fmt` invocations, same scripts left untouched. Pure structural addition. |
| 2 | No `crates/core` | **KEPT — DBT-002** | `crates/mcp-fs/src/lib.rs` is 32 lines (`crates/mcp-fs/src/lib.rs:1-32`), `crates/mcp-fs/src/main.rs` is 9 lines (`crates/mcp-fs/src/main.rs:1-9`); the crate is `[lib] name = "mcp_fs"` (`crates/mcp-fs/Cargo.toml:14-16`) with `mod` declarations for `api`, `app`, `cli`, `config`, `core`, `docs`, `errors`, `git`, `identity`, `keys`, `logging`, `migrate`, `safety`, `search`, `state`, `storage`, `token_screen`, `tools`, `util` (`/bin/ls crates/mcp-fs/src`) | The lib/bin separation already exists inside one Cargo package; this is a mechanical package split (move the lib half into its own crate), not a redesign. `main.rs` at 9 lines is already a thin entry point, which is exactly the target shape. |
| 3 | No `rust-toolchain.toml` | **KEPT — DBT-003** | `Cargo.toml:8` declares `rust-version = "1.88"`, workspace-wide (`Cargo.toml:5-10`); no `rust-toolchain.toml` file exists at the repo root (`test -f rust-toolchain.toml` → absent) | Adding a toolchain pin that matches the already-declared MSRV changes no compiled behavior; it only removes ambiguity about which toolchain a fresh clone uses. |
| 4 | No `deny.toml` | **KEPT — DBT-004** | `deny.toml` absent at repo root (`test -f deny.toml` → absent); no `cargo-audit`/`cargo-deny` invocation anywhere in `build.sh`, `test.sh`, `run.sh`, or `AGENTS.md` | Adding a security-scanning config and a Makefile target that runs it is additive tooling; it does not touch application code or its observable behavior. |
| 5 | Hand-rolled JSON-RPC instead of `rmcp` | **EXCLUDED — not pure debt** | `crates/mcp-fs/src/mcp/mod.rs:1-16` documents `DEC-107`: the C# reference server runs the MCP SDK with `Stateless = true`, answering a bare `tools/call` with **no `initialize` handshake and no session header**; `rmcp` 3.1 "always requires `initialize` first (even with `NeverSessionManager`)", so it **cannot reproduce that contract**. The wire contract is frozen in `TOOL_CONTRACT.txt` and machine-checked by `tool-contract-golden.json` across 94 tools (per `AGENTS.md` overview). | Migrating to `rmcp` would force every client to send `initialize` before `tools/call`, which is an observable protocol change for every existing MCP client of this server. That fails the DEBT preflight table's first row ("any observable behavior changes... route to `/analysis` or `/spec-feat`") and the invariance test in Round 3 of `spec-core.md` ("You SHALL NOT smuggle a behavior change into the debt"). This is a legitimate, large migration, but it needs its own `/spec-feat` (or a deliberate, user-approved breaking-change DEBT entry with `Section 5` populated), never bundled into a "no behavior change" pass. |
| 6 | Hand-rolled `${VAR}`-expanding YAML config instead of `figment`/`etcetera` | **KEPT — DBT-005, scope narrowed** | `crates/mcp-fs/src/config.rs:841` (`ServerConfig`), `:860` (`load`), `:1076` (`expand_env`); resolution precedence hand-rolled in `crates/mcp-fs/src/cli.rs:233-293` (`resolve_config_path` / `resolve_config_path_with`), asserted by 8 existing unit tests at `crates/mcp-fs/src/cli.rs:332-421` | Swapping the *implementation* of path resolution and `${VAR}` expansion for `etcetera` (XDG dirs) and `figment` (file parsing) is pure debt **only if the precedence order and `${VAR}` semantics are preserved exactly**. The skill's own layered model (`defaults < TOML < env < CLI` merge via `figment`) is a *different, richer* resolution strategy than what exists today (today there is no merging of env vars into config values, only path selection plus in-file `${VAR}` substitution). Adopting that richer layering would be new capability, not debt. DBT-005 is therefore scoped to the mechanical swap only; the richer merge model is out of scope and backlogged (see Decisions). |
| 7 | No `tracing-opentelemetry`/OTLP wiring | **EXCLUDED — new capability, not debt** | `crates/mcp-fs/src/logging.rs:1-30` shows the current tracing setup (`tracing_subscriber::EnvFilter`, stderr only); no `opentelemetry`, `tracing_opentelemetry`, `otel` or `OTLP` string exists anywhere in `crates/` or `Cargo.toml` (`rg -n "opentelemetry\|tracing_opentelemetry\|otel\|OTLP" crates/ Cargo.toml -i` → zero matches) | Wiring an OTLP exporter, even config-gated, adds a new network egress path, new config keys (`otel_destination`, `otel_api_key` per the skill), and new spans traced. Per the DEBT preflight table: "A missing security capability (no authn, no secret rotation, no RBAC) → FEAT" and, by the same logic, a missing observability *capability* that this project never had is new observable surface (new outbound calls, new config surface), not an internal reshuffle. Recommend a separate `/spec-feat` for OpenTelemetry export; a bare rename of `logging.rs` to `telemetry.rs` with no OTLP layer would be pure debt but delivers none of the actual compliance gap, so it is not proposed here either. |

**Kept in this DEBT spec:** DBT-001 (Makefile), DBT-002 (`crates/core` split), DBT-003
(`rust-toolchain.toml`), DBT-004 (`deny.toml` + audit/deny wiring), DBT-005 (config internals swapped
to `figment`+`etcetera`, precedence preserved byte-for-byte).

**Excluded, and why, in one line each:** rmcp migration breaks the frozen no-`initialize` wire
contract (`DEC-107`) — needs its own `/spec-feat` or an explicit breaking-change DEBT with Section 5
filled in. OpenTelemetry/OTLP wiring adds new observable network and config surface — needs its own
`/spec-feat`.

---

## 1. Debt

### DBT-001: No Makefile
- **Current structure:** Build automation is three standalone scripts: `build.sh:1-5` (`cargo build
  --release`), `test.sh:1-5` (`cargo test --workspace`), `run.sh:1-22` (env bootstrap, keypair
  generation, `cargo run`). No single `make <target>` interface exists.
- **Why it is debt:** The personal `rust` skill's `make sync/run/test/lint/format/typecheck/
  security/check` interface is the standard entry point across every project in this house; a
  visiting agent or operator has to discover this project's three-script convention instead of
  running `make check` like everywhere else. That costs onboarding time and blocks reusing the
  skill's pre-commit hook, which shells out to `make`.
- **Target structure:** A root `Makefile` exposing `sync`, `build`, `test`, `lint`, `lint-fix`,
  `format`, `format-check`, `typecheck`, `security`, `check`, `run`, `clean` targets. Each target
  either invokes the existing `build.sh` / `test.sh` / `run.sh` unchanged, or the equivalent bare
  `cargo` command where no script exists yet (`clippy`, `fmt`, `audit`, `deny`).
- **Observable behavior:** unchanged. `build.sh`, `test.sh`, `run.sh` are not modified or removed;
  `make` is an additional, additive entry point.

### DBT-002: No `crates/core`
- **Current structure:** All logic lives in the single package `crates/mcp-fs` (`crates/mcp-fs/
  Cargo.toml:1-16`), compiled both as a library (`[lib] name = "mcp_fs"`, `crates/mcp-fs/Cargo.toml:
  14-16`) and a binary (`[[bin]] name = "mcp-fs" path = "src/main.rs"`, `crates/mcp-fs/Cargo.toml:
  10-12`). `crates/mcp-fs/src/main.rs` (9 lines) already only parses CLI args and calls into the
  library; every module (`api/`, `app.rs`, `cli.rs`, `config.rs`, `core/`, `docs/`, `errors.rs`,
  `git/`, `identity.rs`, `keys.rs`, `logging.rs`, `migrate.rs`, `safety.rs`, `search/`, `state.rs`,
  `storage/`, `token_screen.rs`, `tools/`, `util/`) lives under `crates/mcp-fs/src/`.
- **Why it is debt:** The skill mandates a workspace with a library crate holding all logic and a
  thin bin crate per binary. Here the lib/bin separation already exists *inside one Cargo package*,
  but the package boundary is missing, so nothing stops a future change from blurring the line, and
  the `crates/agent` binary cannot depend on `mcp-fs`'s logic without pulling in its own `[[bin]]`
  target as a dependency (a well-known Cargo anti-pattern).
- **Target structure:** A new package `crates/core` (directory name, per the skill's mandated
  layout), Cargo package name `mcp-fs-core`, library crate name `mcp_fs_core` (deliberately not
  `core`, to avoid shadowing the language's own `core` sysroot crate — see Decisions, DDEC-001).
  Every module currently under `crates/mcp-fs/src/` except `main.rs` moves to `crates/core/src/`.
  `crates/mcp-fs` keeps only `src/main.rs`, its own `Cargo.toml`, and a path dependency on
  `mcp-fs-core`.
- **Observable behavior:** unchanged. The compiled binary `mcp-fs` keeps the same name, same CLI,
  same `serve`/`keys`/`token`/`migrate`/`version` verbs (`crates/mcp-fs/src/cli.rs`), same tool
  contract, same REST surface.

### DBT-003: No `rust-toolchain.toml`
- **Current structure:** `Cargo.toml:8` declares `rust-version = "1.88"` (the MSRV) at the workspace
  level; no file pins the actual toolchain a contributor's `rustup`/`cargo` resolves to.
- **Why it is debt:** Two contributors (or CI and a laptop) can silently build with different stable
  toolchains above the MSRV floor, which is exactly the reproducibility gap `rust-toolchain.toml`
  exists to close.
- **Target structure:** `rust-toolchain.toml` at the repo root, `channel = "1.88"`, matching the
  already-declared MSRV in `Cargo.toml:8` (no new version decision introduced).
- **Observable behavior:** unchanged, assuming the toolchain currently in use by contributors and CI
  is already `>= 1.88` stable (true today, since the project already compiles under that MSRV).

### DBT-004: No `deny.toml` / security gate
- **Current structure:** No `deny.toml` file exists at the repo root; no `cargo-audit` or
  `cargo-deny` invocation exists in `build.sh`, `test.sh`, `run.sh`, or documented in `AGENTS.md`.
- **Why it is debt:** The skill's quality gate (`make check`) is `format-check && lint && typecheck &&
  security && test-cov && doc`; without `deny.toml` there is no automated dependency-advisory or
  license gate at all, and the eventual `make security` target would have nothing to run against.
- **Target structure:** `deny.toml` using the cargo-deny 0.20+ schema (no `version = 2` field,
  `unmaintained = "workspace"`), denying yanked crates and wildcard deps, warning on multiple
  versions, and an allowed-license list matching this project's `license = "MIT"`
  (`Cargo.toml:9`) and its actual dependency tree. Wired into the new `make security` target
  (`cargo audit && cargo deny check`).
- **Observable behavior:** unchanged. This adds a CI/local gate; it does not touch `crates/mcp-fs` or
  `crates/agent` source.

### DBT-005: Hand-rolled config resolution instead of `figment`/`etcetera`
- **Current structure:** `ServerConfig::load` (`crates/mcp-fs/src/config.rs:860`) reads a YAML file
  at a path resolved by `resolve_config_path`/`resolve_config_path_with`
  (`crates/mcp-fs/src/cli.rs:233-293`), in this exact precedence: explicit `--config` path (even if
  absent) → `$MCP_FS_CONFIG` env var (even if absent) → `$XDG_CONFIG_HOME` (or `$HOME/.config`)
  `/mcp-fs/config.yaml` when it exists → `${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}
  .yaml`. The loaded YAML text is then passed through a hand-rolled `${VAR}` / `${VAR:-default}`
  substitution (`crates/mcp-fs/src/config.rs:1076`, `expand_env`) before `serde_yaml` deserializes
  it into `ServerConfig` (`crates/mcp-fs/src/config.rs:841`).
- **Why it is debt:** The skill mandates `figment` (layered `serde` providers) plus `etcetera`
  (cross-platform XDG base directories) as the one approved config stack; the current code
  reimplements XDG resolution by hand (`crates/mcp-fs/src/cli.rs:242-282`) instead of using
  `etcetera::choose_base_strategy()`, and re-implements environment substitution by hand instead of
  a `figment::providers::Env` merge.
- **Target structure:** `resolve_config_path_with`'s XDG step uses `etcetera::choose_base_strategy()`
  in place of the current hand-rolled `$XDG_CONFIG_HOME`/`$HOME` logic
  (`crates/mcp-fs/src/cli.rs:242-282`). `ServerConfig::load` uses `figment::providers::Yaml` to parse
  the file and a **custom pre-processing step that reproduces `expand_env`'s exact semantics**
  (`${VAR}` and `${VAR:-default}`) rather than adopting `figment`'s own `Env` provider merge, which
  has different semantics (see Decisions, DDEC-002: scope narrowed to avoid smuggling the richer
  `defaults < file < env < CLI` layering model as a feature).
- **Observable behavior:** unchanged. Same precedence order, same `${VAR}`/`${VAR:-default}` syntax
  and defaulting rules, same error when nothing is found.

---

## 2. Perimeter — the Current State (mandatory, every depth)

| File | Change | Occurrences |
|---|---|---|
| `Makefile` (new) | created | 1 file, 0 pre-existing occurrences |
| `build.sh` | referenced by `make build`/`make release`, not modified | `build.sh:1-5` (2 lines of substance) |
| `test.sh` | referenced by `make test`, not modified | `test.sh:1-5` (2 lines of substance) |
| `run.sh` | referenced by `make run`/`make run-dev`, not modified | `run.sh:1-22` |
| `crates/mcp-fs/Cargo.toml` | `[lib]` section removed, `mcp-fs-core` path dependency added | `crates/mcp-fs/Cargo.toml:14-16` (the `[lib]` block, 3 lines) |
| `crates/mcp-fs/src/main.rs` | unchanged in content; now the only file left in the package | `crates/mcp-fs/src/main.rs:1-9` |
| `crates/mcp-fs/src/{api,app.rs,cli.rs,config.rs,core,docs,errors.rs,git,identity.rs,keys.rs,logging.rs,migrate.rs,safety.rs,search,state.rs,storage,token_screen.rs,tools,util}` | moved verbatim to `crates/core/src/` (renamed package `mcp-fs-core`) | 18 top-level module entries (`/bin/ls crates/mcp-fs/src` minus `main.rs` and `lib.rs`) |
| `crates/mcp-fs/src/lib.rs` | content (32 lines, `crates/mcp-fs/src/lib.rs:1-32`) moves to `crates/core/src/lib.rs`, becoming the new crate's public API surface | 1 file |
| every `use crate::...` inside the moved modules | rewritten to `use mcp_fs_core::...` | mechanical; exact count produced by the compiler at build time, not hand-counted (see Section 7, step 2) |
| `rust-toolchain.toml` (new) | created | 1 file |
| `deny.toml` (new) | created | 1 file |
| `crates/mcp-fs/src/cli.rs` | `resolve_config_path_with`'s XDG branch (`:242-282`) swapped to use `etcetera` | `crates/mcp-fs/src/cli.rs:242-282` (41 lines) |
| `crates/mcp-fs/src/config.rs` | `ServerConfig::load` (`:860`) and `expand_env` (`:1076`) swapped to a `figment`-based loader preserving `expand_env`'s exact substitution semantics as a custom provider/pre-processor | `crates/mcp-fs/src/config.rs:841,860,1076` |
| `Cargo.toml` (workspace) | `[workspace.dependencies]` gains `etcetera`, `figment`, plus dev-dependencies for `cargo-deny`/`cargo-audit` are external tools, not crate deps | `Cargo.toml:1-10` (workspace block) |

**Non-source occurrences:** `AGENTS.md` (documents `build.sh`/`test.sh`/`run.sh` as the "Key
commands", needs a companion `make` row added, not a replacement), `README.md:257` (documents
`cargo clippy --all-targets --all-features -- -D warnings` directly; add the `make lint` equivalent),
`.env.example` (references `run.sh`'s bootstrap, unaffected), `.gitignore` (references `run.sh`
artifacts, unaffected), `FUNCTIONAL_TESTS.md` (references `test.sh`, unaffected — no rename),
`tests/functional/README.md` and `tests/functional/run_all.sh` (reference the scripts by name,
unaffected), `.agent_docs/config.md`, `.agent_docs/testing.md`, `.agent_docs/agent.md` (document the
config resolution order and the test command; both need the `crates/core` module path update and a
one-line note that the underlying resolver now uses `etcetera`/`figment` internally with unchanged
behavior), `config/agent_test.yaml` (references `run.sh`, unaffected), `plan/2026-09-16_07-18-21-je.md`
(historical planning note, no update needed — it is a dated record, not living documentation).

**Contracts touched:**
- `build.sh` / `test.sh` / `run.sh` invocation contract — **public** (documented, operator-facing
  commands in `README.md`/`AGENTS.md`). Not renamed or removed; `make` is additive. No compatibility
  plan needed (Section 4 has no row for it: nothing breaks or dual-maintains, the old form simply
  keeps working unchanged).
- `mcp-fs` binary CLI (`serve`/`keys`/`token`/`migrate`/`version` verbs, `--config` flag,
  `$MCP_FS_CONFIG`/`$MCP_FS_CONFIG_DIR`/`$MCP_FS_CONFIG_NAME`/`$XDG_CONFIG_HOME` env vars) —
  **public** (operator-facing, deployment-facing). Precedence and semantics preserved exactly; see
  Section 4.
- `mcp_fs` library crate name / `crate::` internal module paths — **internal only** (nothing outside
  this repository's own two workspace members references `mcp_fs::` as a library; `crates/agent` is
  a separate MCP *client*, per `AGENTS.md`'s own description, and does not depend on `mcp-fs`'s lib
  crate — confirmed: `crates/agent/Cargo.toml:1-20` has no `mcp-fs` or `mcp_fs` dependency line).
  Free to rename to `mcp_fs_core` with no compatibility plan required.
- 94-tool MCP contract (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`) and the REST `/api/fs`
  surface — **public**, but **untouched by every kept item in this lot**. DBT-002 through DBT-005 are
  pure internal moves; none of them touch `crates/mcp-fs/src/mcp/` or `crates/mcp-fs/src/api/`
  content, only their location on disk.

**Guardrail coverage:** the config-resolution precedence that DBT-005 must preserve is already
directly asserted by 8 existing unit tests in `crates/mcp-fs/src/cli.rs:332-421`
(`explicit_path_wins_over_everything`, `explicit_path_is_used_even_when_it_does_not_exist`,
`env_config_wins_over_xdg_and_the_dir_name_pair`, `env_config_is_used_even_when_it_does_not_exist`,
`xdg_config_home_is_probed_before_the_dir_name_pair`,
`home_supplies_the_xdg_base_when_the_variable_is_unset`,
`an_absent_xdg_file_falls_through_to_the_dir_name_pair`, `dir_and_name_compose_the_default`), plus
`blank_env_values_fall_back_to_defaults` and `nothing_found_reports_every_path_tried`
(`crates/mcp-fs/src/cli.rs:433,451`) — 10 tests total covering this exact surface. `expand_env`'s
substitution semantics are covered by 5 unit tests at `crates/mcp-fs/src/config.rs:1217-1246+`
(`expand_env_uses_variable_when_set`, `expand_env_uses_default_when_unset`,
`expand_env_variable_overrides_default`, `expand_env_missing_without_default_is_empty`,
`expand_env_leaves_non_variables_alone`). This is strong, already-existing guardrail coverage: this
refactor is verifiable as is, no new tests are required before starting, though DT- tests are added
below to lock the structural claims (crate split, Makefile equivalence).

**Exact test command:** `cargo test --workspace` (per `test.sh:5`) plus, once DBT-004 lands,
`cargo clippy --all-targets --all-features -- -D warnings` (per `AGENTS.md:42` and `README.md:257`,
both already documented as the quality gate).

**Total:** 12 files/locations changed or added across the perimeter, 10 non-source documentation
files referencing the changed commands, 0 removed public commands, 0 renamed public contracts.

---

## 3. Requirements

### 3.1 Structural

#### DR-001 [EARS-U]: Makefile wrapper
> The `mcp-fs` repository SHALL provide a root `Makefile` exposing the targets `sync`, `build`,
> `release`, `test`, `lint`, `lint-fix`, `format`, `format-check`, `typecheck`, `security`, `check`,
> `run`, `run-dev`, `clean`, `help`, where `build` runs `build.sh`, `test` runs `test.sh`, and `run`/
> `run-dev` run `run.sh`, without modifying the scripts' contents.

- **Old name / form:** direct invocation of `./build.sh`, `./test.sh`, `./run.sh`.
- **New name / form:** `make build`, `make test`, `make run` (delegating to the same scripts).
- **Applies to:** the root `Makefile` (new file); no change to `build.sh`, `test.sh`, `run.sh`.

#### DR-002 [EARS-U]: `crates/core` extraction
> The workspace SHALL contain a library package at `crates/core` (Cargo package name
> `mcp-fs-core`, library crate name `mcp_fs_core`) holding every module currently under
> `crates/mcp-fs/src/` except `main.rs`, and `crates/mcp-fs` SHALL retain only `src/main.rs` plus a
> path dependency on `mcp-fs-core`.

- **Old name / form:** package `mcp-fs` (`crates/mcp-fs/Cargo.toml:1`), library crate `mcp_fs`
  (`crates/mcp-fs/Cargo.toml:14-16`), modules under `crates/mcp-fs/src/{api,app,cli,config,core,
  docs,errors,git,identity,keys,logging,migrate,safety,search,state,storage,token_screen,tools,
  util}`.
- **New name / form:** package `mcp-fs-core` at `crates/core`, library crate `mcp_fs_core`, same
  module tree relocated under `crates/core/src/`. The compiled binary keeps its existing name
  `mcp-fs` (`crates/mcp-fs/Cargo.toml:11`), unaffected by the library rename.
- **Applies to:** every file listed in Section 2's perimeter table under the `crates/mcp-fs/src/`
  row; every `use crate::` import inside those files, rewritten to `use mcp_fs_core::`.

#### DR-003 [EARS-U]: Toolchain pin
> The repository SHALL contain a `rust-toolchain.toml` at its root with `channel = "1.88"`, matching
> the workspace `rust-version` declared at `Cargo.toml:8`.

- **Old name / form:** no toolchain file; the ambient `rustup` default toolchain is used.
- **New name / form:** `rust-toolchain.toml` pinning `channel = "1.88"`.
- **Applies to:** repository root (new file only).

#### DR-004 [EARS-U]: Security gate config
> The repository SHALL contain a `deny.toml` at its root using the cargo-deny 0.20+ schema (no
> `version = 2` field), denying yanked crates and wildcard dependencies, warning on duplicate
> dependency versions, and declaring an allowed-license list including at minimum `MIT` (the
> project's own license, `Cargo.toml:9`) and every license actually present in the resolved
> dependency tree; the Makefile's `security` target SHALL run `cargo audit` followed by
> `cargo deny check`.

- **Old name / form:** no security gate exists.
- **New name / form:** `deny.toml` (new file) plus `make security`.
- **Applies to:** repository root (new file); `Makefile`'s `security` target.

#### DR-005 [EARS-U]: Config resolution internals
> The `resolve_config_path_with` function (`crates/mcp-fs/src/cli.rs:242`) SHALL resolve the XDG
> config-home base directory via `etcetera::choose_base_strategy()` instead of its current hand-rolled
> `$XDG_CONFIG_HOME`/`$HOME` fallback logic, and `ServerConfig::load` (`crates/mcp-fs/src/config.rs:
> 860`) SHALL parse the YAML file via a `figment::providers::Yaml` provider fed by text that has
> already been passed through the existing `expand_env` substitution logic (or a byte-for-byte
> equivalent reimplementation of it), while preserving the exact precedence order asserted by the
> tests at `crates/mcp-fs/src/cli.rs:332-421`.

- **Old name / form:** hand-rolled XDG resolution (`crates/mcp-fs/src/cli.rs:242-282`) and hand-rolled
  YAML-text `${VAR}` substitution (`crates/mcp-fs/src/config.rs:1076`).
- **New name / form:** `etcetera`-backed XDG resolution; `figment`-backed YAML parsing, still fed by
  the same `${VAR}` substitution semantics.
- **Applies to:** `crates/mcp-fs/src/cli.rs:242-282` (after DR-002, this file lives at
  `crates/core/src/cli.rs`), `crates/mcp-fs/src/config.rs:841-1076` (after DR-002, at
  `crates/core/src/config.rs`); `Cargo.toml`'s `[workspace.dependencies]` gains `etcetera` and
  `figment`.

### 3.2 Invariance (mandatory)

#### DR-006 [EARS-U]: Behavior invariance
> The `mcp-fs` system SHALL produce, for every input covered by the existing test suite
> (`cargo test --workspace`), byte-identical observable output before and after this change,
> including: the `mcp-fs` binary's CLI surface, its 94-tool MCP contract, its REST `/api/fs` surface,
> its config file resolution precedence, and its `${VAR}`/`${VAR:-default}` substitution behavior.

### 3.3 Compatibility

#### DR-007 [EARS-E]: Config resolution precedence preserved
> WHEN `mcp-fs` is invoked with any combination of an explicit `--config`/`-c` path, the
> `MCP_FS_CONFIG` environment variable, `XDG_CONFIG_HOME` (or `HOME`), `MCP_FS_CONFIG_DIR`, and
> `MCP_FS_CONFIG_NAME`, THE system SHALL resolve the config file path using the exact precedence
> order asserted by the tests at `crates/mcp-fs/src/cli.rs:332-421` (renumbered `crates/core/src/
> cli.rs` after DR-002), with no behavioral difference before and after DR-005.

#### DR-008 [EARS-UB]: No wire-contract surface touched
> This remediation SHALL NOT modify the JSON-RPC/SSE wire framing in `crates/mcp-fs/src/mcp/`
> (relocated to `crates/core/src/mcp/` under DR-002, moved verbatim), the 94-tool schema frozen in
> `TOOL_CONTRACT.txt` and `tool-contract-golden.json`, or the REST `/api/fs` surface in
> `crates/mcp-fs/src/api/` (relocated verbatim under DR-002).

---

## 4. Compatibility Plan

| Contract | Kind | Public? | Plan | Deprecation window | Removal condition |
|---|---|---|---|---|---|
| `mcp-fs` CLI config resolution (`--config`, `MCP_FS_CONFIG`, `XDG_CONFIG_HOME`, `MCP_FS_CONFIG_DIR`, `MCP_FS_CONFIG_NAME`) | env vars + CLI flag | yes | Preserve exactly (no rename, no new precedence, no dual maintenance needed — nothing changes from the caller's point of view) | n/a — nothing deprecated | n/a |
| `build.sh` / `test.sh` / `run.sh` | shell scripts | yes | Preserve exactly; `Makefile` targets are additive wrappers, not replacements | n/a — nothing deprecated | n/a |
| `mcp_fs` library crate name (`crate::` internal paths) | Rust crate name | no (internal only — confirmed no external or `crates/agent` dependency, `crates/agent/Cargo.toml:1-20`) | Rename freely to `mcp_fs_core`; no compatibility plan required | n/a | n/a |

**Dual-maintenance semantics:** not applicable — every public contract in this lot keeps its exact
current form; nothing is renamed, so there is no old-form/new-form pair to dual-maintain and no
warning to emit. (Contrast with DBT-005's file-internal swap of the *implementation* behind an
unchanged contract, which is precisely why it is debt and not a breaking change.)

---

## 5. Declared Breaks

None. This is pure debt: every public contract keeps its exact current form; the internal package
layout, toolchain pin, security tooling, and config-resolution implementation change, but nothing a
caller or operator observes moves.

---

## 6. Tests

### 6.1 Non-regression — the success criterion
**The existing test suite passes unmodified.** Command:
```
cargo test --workspace
```
Once DBT-004 lands, the full quality gate additionally requires:
```
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
All three are already the documented gate at `AGENTS.md:42,141` and unaffected in command form by
this remediation (only their invocation gains an optional `make` wrapper).

### 6.2 Compatibility tests

#### DT-001: Config precedence, explicit path wins
- **Validates:** DR-005, DR-007
- **Contract:** `mcp-fs` CLI config resolution
- Given the existing test `explicit_path_wins_over_everything` (`crates/core/src/cli.rs:332` after
  the move), When it runs unmodified against the `figment`/`etcetera`-backed implementation, Then it
  asserts the identical result it asserts today: `PathBuf::from("/tmp/x.yaml")` wins over every env
  var. No new assertion is required; this test's continued pass **is** the compatibility proof.

#### DT-002: Config precedence, XDG probed before dir/name pair
- **Validates:** DR-005, DR-007
- **Contract:** `mcp-fs` CLI config resolution
- Given the existing test `xdg_config_home_is_probed_before_the_dir_name_pair`
  (`crates/core/src/cli.rs:383` after the move), When it runs unmodified against the
  `etcetera`-backed XDG resolution, Then it asserts the identical result: `PathBuf::from(
  "/xdg/mcp-fs/config.yaml")`.

#### DT-003: `${VAR}` substitution semantics preserved
- **Validates:** DR-005, DR-006
- **Contract:** config file `${VAR}`/`${VAR:-default}` expansion
- Given the 5 existing tests at `crates/core/src/config.rs:1217-1246+` (`expand_env_uses_variable_
  when_set`, `expand_env_uses_default_when_unset`, `expand_env_variable_overrides_default`,
  `expand_env_missing_without_default_is_empty`, `expand_env_leaves_non_variables_alone`), When they
  run unmodified against the `figment`-fed implementation, Then each asserts its existing exact
  string output unchanged.

### 6.3 Structural tests

#### DT-010: `crates/mcp-fs` contains no logic modules
- **Asserts:** after DR-002, `crates/mcp-fs/src/` contains exactly one file, `main.rs`, and no
  `mod` declarations beyond what `main.rs` needs to call into `mcp_fs_core`.
- **Method:** a `#[test]` in `crates/mcp-fs/tests/` (new) that reads `crates/mcp-fs/src/` via
  `std::fs::read_dir` and asserts the file list is exactly `["main.rs"]`.

#### DT-011: `crates/core` builds and tests independently
- **Asserts:** `cargo test -p mcp-fs-core` succeeds standalone (proves the extraction is a real
  package boundary, not merely a `mod` re-export).
- **Method:** documented as part of the non-regression command in Section 6.1; `cargo test
  --workspace` already exercises this since `mcp-fs-core` is a workspace member.

#### DT-012: `Makefile` targets resolve to the documented commands
- **Asserts:** `make build` invokes `build.sh`, `make test` invokes `test.sh`, `make run` invokes
  `run.sh`, with no divergent flags.
- **Method:** `make -n build`, `make -n test`, `make -n run` (dry-run print) each contain the literal
  substring `build.sh`, `test.sh`, `run.sh` respectively; asserted by a `tests/functional/`
  (existing directory, per Section 2) bats-style or shell assertion, or inline in `test.sh` itself as
  a pre-check. Given this project's shell tooling already lives in `tests/functional/`, this can be a
  one-line addition to `tests/functional/run_all.sh` rather than a new framework.

### 6.4 Existing tests to modify

| Test file | Test name | Change | Justification |
|---|---|---|---|
| every `#[cfg(test)] mod tests` block under `crates/mcp-fs/src/**` | all (relocated, not rewritten) | import paths change from `use crate::...` to `use mcp_fs_core::...`; **no assertion, no test data, and no expected value changes** | required mechanically by DR-002's package split; every file physically moves from `crates/mcp-fs/src/` to `crates/core/src/`, so every `#[cfg(test)]` block inside those files moves with it. This is relocation, not modification of test intent, and is called out here for transparency rather than left silent. |

No other existing test changes assertions, expected values, or test data. This list is intentionally
the only row, and it is a batch of purely mechanical import-path rewrites rather than logic changes —
the closest thing to "empty" this particular debt item (a package split) can produce.

---

## 7. Implementation Order

Sequence chosen so the build and `cargo test --workspace` stay green after every step; each step is
independently committable.

1. **DBT-003 (toolchain pin) + DBT-004 (security gate config).** Add `rust-toolchain.toml` and
   `deny.toml`. Zero code touched; `cargo build --workspace` and `cargo test --workspace` unaffected.
   Run `cargo deny check` once to confirm the allow-list matches the real dependency tree; adjust
   `deny.toml` until it passes clean, since this is new tooling with nothing yet to fix in the code.
2. **DBT-001 (Makefile).** Add the root `Makefile` wrapping `build.sh`/`test.sh`/`run.sh` and the bare
   `cargo clippy`/`cargo fmt`/`cargo audit`/`cargo deny` commands. Verify `make check` runs clean
   end to end. No source file touched.
3. **DBT-002 (crate split), in three sub-steps to keep the suite green throughout:**
   a. Create `crates/core/Cargo.toml` (package `mcp-fs-core`, `[lib] name = "mcp_fs_core"`), copy
      every module except `main.rs` from `crates/mcp-fs/src/` to `crates/core/src/`, rewrite every
      `use crate::` to `use mcp_fs_core::` inside the copied files, add `mcp-fs-core` as a
      `[workspace.dependencies]` path entry. Add `mcp-fs-core` to `[workspace] members`.
   b. Reduce `crates/mcp-fs/src/` to `main.rs` only; add the `mcp-fs-core` path dependency to
      `crates/mcp-fs/Cargo.toml`; update `main.rs`'s `use` statements to pull from `mcp_fs_core`.
      Run `cargo build --workspace` — this is the step most likely to surface a missed import;
      fix path by path until it compiles clean.
   c. Delete the now-empty module files from `crates/mcp-fs/src/` (they were copied, not moved, in
      3a, so the working tree carries both until this sub-step; delete only after 3b compiles).
      Run `cargo test --workspace`. Add DT-010 and DT-011.
4. **DBT-005 (config internals).** Only after the crate split lands (its file paths, e.g.
   `crates/core/src/cli.rs`, are the ones DR-005 names). Add `etcetera`/`figment` to
   `[workspace.dependencies]`. Swap `resolve_config_path_with`'s XDG branch to `etcetera`. Run the 8
   existing precedence tests (now at `crates/core/src/cli.rs`) — they must pass unmodified before
   touching `ServerConfig::load`. Then swap `ServerConfig::load` to the `figment`-based provider,
   re-run the 5 existing `expand_env` tests plus the 8 precedence tests. Add DT-012 last, once the
   Makefile (step 2) and the config swap (this step) both exist, since it exercises both.
5. **Documentation.** Update `AGENTS.md` and `README.md`'s "Key commands" sections to add the `make`
   targets alongside (not replacing) `build.sh`/`test.sh`/`run.sh`; update `.agent_docs/config.md`,
   `.agent_docs/testing.md`, `.agent_docs/agent.md` to reference `crates/core` module paths and the
   `etcetera`/`figment` internals, per this project's own AGENTS.md rule ("On every implementation,
   keep up to date: README.md ... and AGENTS.md").

Between every numbered step, `cargo build --workspace` and `cargo test --workspace` are green. Step 3
is the only one with sub-steps, because it is the only one that moves files the compiler must resolve
in a specific order (create the new crate before deleting the old modules).

---

## 8. Decisions & Assumptions

- **DDEC-001:** Name the new package's directory `crates/core` (matching the skill's mandated layout
  literally) but its Cargo package name `mcp-fs-core` and library crate name `mcp_fs_core`, not `core`.
  **Rationale:** Rust's implicit sysroot crate is itself named `core` and is part of every crate's
  extern prelude; a workspace member literally named `core` risks ambiguous or shadowed references to
  `::core::` paths used throughout the codebase (`std`/`core` primitives). **Alternatives considered:**
  name the package `core` and rely on `[lib] name = "..."` alone to disambiguate the compiled artifact
  — rejected, because the *package* name `core` in `Cargo.toml` still participates in dependency
  resolution and error messages in a way that invites confusion for no benefit. **Implemented by:**
  DR-002. **Code evidence:** `crates/mcp-fs/src/**` uses ordinary Rust code throughout with no
  existing `::core::` qualification convention that would make a name collision safe to introduce.
- **DDEC-002:** Scope DBT-005 to swapping the *implementation* behind `resolve_config_path_with` and
  `ServerConfig::load` (XDG resolution via `etcetera`, YAML parsing via `figment::providers::Yaml`),
  while explicitly **not** adopting `figment`'s full `defaults < file < env < CLI` layered-merge
  model. **Rationale:** the current system has no notion of individual config *values* being
  overridden by environment variables (only the config *file's path* is env-selected, and `${VAR}`
  substitutes inside the file's text); adopting figment's value-level env/CLI merge would let an
  operator override, e.g., `server.port` via an env var where they cannot today — that is new
  observable capability, which fails the DEBT invariance test. **Alternatives considered:** adopt the
  richer figment layering as originally implied by the skill — rejected for this lot; recorded as a
  candidate for a future `/spec-feat` (config value-level overrides via env/CLI), not built here.
  **Implemented by:** DR-005. **Code evidence:** `crates/mcp-fs/src/config.rs:860-872` (today,
  `load` reads exactly one file, expands `${VAR}` in its text, and deserializes once — no merge step
  exists to preserve or extend).
- **DDEC-003:** Exclude the `rmcp` SDK migration (skill gap #5) and the OpenTelemetry/OTLP wiring
  (skill gap #7) from this specification. **Rationale:** both fail the Q1 functional test in
  `spec-core.md` Phase 6 — two competent implementing agents each resolving "migrate to rmcp" their
  own way would produce observably different results for any MCP client that today calls `tools/call`
  without an `initialize` handshake (per `DEC-107`), and "add OTLP wiring" necessarily adds new
  config keys and new network egress, which is new capability rather than a reshuffle.
  **Alternatives considered:** bundle them anyway "since the audit asked for all 7" — rejected per the
  task's own preflight instruction to flag anything that isn't pure debt rather than force it in.
  **Implemented by:** n/a — no code impact in this spec. **Code evidence:**
  `crates/mcp-fs/src/mcp/mod.rs:1-16`, `crates/mcp-fs/src/logging.rs:1-30`.
- **Assumption:** the toolchain currently used by contributors and any CI (none found:
  `.github/workflows` is absent from this repository) is already stable `>= 1.88`, since the project
  already builds under that MSRV (`Cargo.toml:8`). Labeled as an assumption because no CI
  configuration exists to verify this mechanically today; `rust-toolchain.toml`'s addition is exactly
  what closes that gap going forward.
- **Assumption:** the exact `cargo-deny` license allow-list and advisory exceptions (if any advisory
  currently affects a dependency) are decided by whoever runs `cargo deny check` for the first time
  during Implementation Order step 1, since the real dependency tree's licenses were not enumerated as
  part of this specification (that enumeration is an implementation-time, not specification-time,
  activity — the requirement (DR-004) only mandates the mechanism, not the exact resulting list).

---

## 9. Implementability Gate

This run had no live sub-agent auditor available (delegated, non-interactive execution). The check
below was run **inline against the file as written on disk**, per `spec-core.md` Phase 6.4's
depth-S procedure, applied here as the closest available substitute at depth M given the tooling
constraint. This substitution is itself logged as an open risk in Section 10.

| Round | F (functional, blocking) | A (drift, traced) | Verdict |
|---|---|---|---|
| 1 (inline self-review) | 0 | 1 | IMPLEMENTABLE-WITH-DRIFT |

**Inline checklist applied (spec-core.md 6.4, items 1–15):**
1. No orphan decision — every `DDEC-XXX` names its `DR-XXX`. PASS.
2. No buried change — every substantial change (Makefile, crate split, toolchain, deny.toml, config
   swap) owns its own `DR-XXX`. PASS.
3. Every name spelled — package names, crate names, file paths, target names all literal. PASS.
4. No forced choice — precedence order, license list mechanism, and crate naming are all decided
   (crate naming explicitly, license list mechanism deferred to implementation time as a non-normative
   detail, see DDEC-003's assumption; this is not a forced choice with observably different results,
   since `cargo deny check` fails loudly on an unlisted license regardless of who runs it first).
5. Order stated — Section 7 is explicit and step-ordered.
6. Every test specified — DT-001 through DT-012 each name Given/When/Then or an explicit method.
7. No out-of-spec prerequisite — `etcetera` and `figment` versions are not pinned here; this is a
   drift entry (DDRIFT-001 below), not a blocker, since `make sync`/`cargo build` will resolve
   compatible versions and the requirement (DR-005) does not depend on a specific version.
8. EARS clean — every `DR-XXX` matches a listed pattern, no forbidden modal used.
9. Right tool — DEBT confirmed at preflight for every kept item; DDEC-003 documents why 2 candidate
   items were routed elsewhere instead of smuggled in.
10. Length within budget for depth M (this document runs long because the perimeter and evidence
    citations are extensive, which the template treats as unavoidable at every depth for a DEBT spec).
11. Every claim about the code cited `file:LINE`, recounted directly in Section 2's preflight table.
12. No requirement rests on a capability the code lacks — DR-005 explicitly narrows scope for this
    exact reason (DDEC-002); DR-002 confirmed lib/bin already exists inside the package (evidence
    cited).
13. Security misclassification check — this document describes no authn/authz bypass, no injection,
    no data exposure, no deserialization/traversal flaw, and no advisory-driven dependency bump.
    `Security: n/a` is correct.
14. N/A (no `--security` flag, check 13 did not fire).
15. DEBT-specific: perimeter is exhaustive including non-source files (Section 2's non-source list);
    both sides of every rename spelled (DR-001, DR-002, DR-005); every public contract has a plan
    (Section 4); non-regression command is literal (`cargo test --workspace`); existing-tests-to-modify
    list is a single, fully-justified mechanical-relocation row (Section 6.4).

**Amendments applied:** none required; the one A finding below was registered rather than fixed in
place, since the correct version numbers are an implementation-time decision, not a specification-time
fact to get wrong.

**Drift registered:** DDRIFT-001 (below).

---

## 10. Drift Register

#### DDRIFT-001: Exact `etcetera`/`figment` versions not pinned in this specification
- **Doc says:** DR-005 requires `etcetera`-backed XDG resolution and `figment`-backed YAML parsing,
  without naming exact crate versions.
- **Code does:** `Cargo.toml:1-10`'s `[workspace.dependencies]` currently names an exact or
  range version for every other dependency (e.g. `rusqlite = { version = "0.37", ... }`,
  `Cargo.toml` header block), establishing this project's convention of pinning a version per
  dependency.
- **Nature:** missing capability (the specification does not itself resolve the version; that
  resolution happens at implementation time via `context7`/crates.io, per this project's own
  `AGENTS.md`-inherited "Context7 Workflow" rule).
- **Resolution during implementation:** before adding `etcetera`/`figment` to
  `[workspace.dependencies]`, resolve their current published versions via context7 or
  `https://crates.io/api/v1/crates/<name>`, per the standing house rule ("Before adding/upgrading a
  dependency... context7 is mandatory"), and pin them the same way every other dependency in this
  file is pinned.
- **Detected by:** `cargo build --workspace` fails immediately if an unresolvable or yanked version
  is named; `cargo deny check` (DBT-004, already in this same lot) fails on a yanked crate the
  moment it is added.
- **Status:** open

---

## Open risks (not drift, flagged for the user)

1. **No sub-agent delegation was available during this run.** `spec-core.md` mandates independent
   test-designer and auditor sub-agents at depth M/L; this execution substituted an inline
   self-review (Section 9) because no sub-agent spawning tool was available in this environment.
   The self-review is by the same author as the requirements, which is exactly the bias the
   sub-agent step exists to remove. **Recommend:** before implementation, have a second agent (fresh
   context) run the Phase 6.2 audit contract against this file, passing only the file path, per the
   protocol's own instructions.
2. **No CI configuration exists in this repository** (`.github/workflows` absent). DBT-004's
   `make security` target and DBT-001's `make check` have nothing to run them automatically today;
   this spec does not propose adding CI, since that is outside the 7 audited gaps and would be its
   own decision.
3. **The excluded `rmcp` migration (skill gap #5) is the single largest compliance gap** and remains
   unaddressed. It is deliberately out of this lot (DDEC-003) but is real technical debt that
   conflicts with a documented, frozen wire contract (`DEC-107`). Recommend a dedicated `/spec-feat`
   or an explicit breaking-change DEBT entry, with the user asked directly whether breaking the
   no-`initialize` contract is acceptable.
4. **The excluded OpenTelemetry wiring (skill gap #7)** likewise remains unaddressed; recommend a
   dedicated `/spec-feat`.
