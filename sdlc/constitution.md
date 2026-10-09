# Constitution (DRAFT, written by /sdlc-retro-spec on 2026-10-09, review before relying on it)

> Status: draft

| Rule | Source | Enforced by |
|---|---|---|
| `make check` (fmt-check, clippy -D warnings, typecheck, security, test) passes before commit | `Makefile:20-40`, `AGENTS.md:42,48,190-191` | nothing (no CI workflow exists in this repo; convention only) |
| A store never speaks a driver directly; SQL goes through `RelationalDb`/`Dialect` | `AGENTS.md` Conventions section; `crates/core/src/storage/rel/` (all 3 engine files follow the same trait) | nothing automated beyond code review; no lint rule forbids importing a driver type outside `storage/rel/` |
| `volume_id` appears in every `WHERE` clause touching `nodes`/`blob_refs`/`git_*` | `AGENTS.md` Conventions section; `crates/core/src/storage/meta.rs`, `git/db.rs` (consistent across all call sites) | nothing (convention only; a missing `volume_id` predicate is not mechanically checked) |
| The MCP surface and the REST plane share one engine implementation (`core::fs_ops`), never a second implementation in the tool layer or `api/dataplane.rs` | `AGENTS.md` Conventions section; `crates/core/src/tools/*.rs` and `crates/core/src/api/dataplane.rs` both call into `core::fs_ops` | `tools/contract_golden.rs` checks tool schemas, not call-graph duplication; no automated check for this rule specifically |
| `tool-contract-golden.json` is the machine-checked source of truth for every tool's name/description/schema | `AGENTS.md`; `crates/core/src/tools/contract_golden.rs` | `cargo test` (3 golden-contract tests), part of `make check` |
| No dashes as punctuation (hyphen/en dash/em dash) anywhere in code, comments or output | `AGENTS.md` Conventions section | nothing automated; style convention only |

Every gate reads this file, so a rule needs two independent sources or an enforcing tool. Only
the tool-contract rule currently has a real enforcing mechanism (a passing test); every other row
above is convention-only, confirmed by a single source (`AGENTS.md` plus the code pattern it
describes), not independently enforced. Treat this draft as a starting point, not a verified set
of gates.
