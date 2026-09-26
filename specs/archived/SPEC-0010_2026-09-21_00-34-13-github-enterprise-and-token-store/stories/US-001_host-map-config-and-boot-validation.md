# US-001: The git.hosts map, its boot validation, and its single owner

> Parent Spec: `specs/archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 1
> Depends On: none
> Complexity: M
> min_tier: 1
> Files touched: 5

## Objective

An operator declares which git hosts this server trusts and what credential policy each carries, and a misdeclared map stops the server at boot rather than at first clone. This story creates `git/remote.rs` and gives it sole ownership of interpreting the map, so no later story has to move that logic. It does not resolve a URL yet: that is US-002.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/config.rs  # GitConfig at :447-457, ServerConfig::validate at :813-819
crates/mcp-fs/src/git/mod.rs  # declares the git submodules
crates/mcp-fs/src/git/remote.rs  # NEW: validate_hosts, resolve_host, Provider
```

### Existing Patterns
- Boot validation fires from `ServerConfig::validate` (`crates/mcp-fs/src/config.rs:813-819`), alongside the existing `validate_store("oauth", ...)` call at `:817`. Follow that shape exactly.
- `GitConfig` (`crates/mcp-fs/src/config.rs:447-457`) is the struct that gains `hosts`. Serde defaults there show how an absent key is handled.
- The one-module-owns-it rule mirrors `core::fs_ops`, the single implementation behind both the MCP and REST layers (see AGENTS.md conventions).

### Data Model (excerpt)
- `HostEntry` — in-memory, from YAML, keyed `host`. `{ host: String, provider: Provider }`. Immutable after boot.
- `Provider` — in-memory enum. `Github` | `Gitlab` | `Generic` | `Anonymous`.

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-002:** Providers stay `github` and `gitlab`, plus an explicit `generic`. No hardcoded host values. **Rationale:** GHES genuinely is provider `github`, so the credential shape and `git.auth`'s validation stay correct; `generic` covers self-hosted hosts without opening the set to arbitrary strings. **Alternatives considered:** adding `azure` with its own credential shape; a fully open config-defined set. **Implemented by:** FR-NEW-001. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/tools/git_auth.rs:160-162`
- **DEC-015:** Host matching is exact only; no wildcards. **Rationale:** wildcards reintroduce the precedence ambiguity substring matching caused. **Implemented by:** FR-NEW-004, FR-NEW-006. **Round:** 2b. **Code evidence:** n/a

### Applicable NFRs
- **Boot validation is total** (§7.4): FR-NEW-002 to FR-NEW-004. Every invalid entry is rejected at boot, not at first use.
- **Host resolution is constant-time relative to map size** (§7.1): a map lookup, not a scan.

### Bounded Context
**Host Resolution** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Implementation Drift To Close In This Story

> Carried verbatim from the specification's Section 19. These are resolved **during** this story, not later. An entry still open when the branch merges moves to `specs/BACKLOG.md`, and that move is a decision someone signs, not a silence.

#### DRIFT-003: The `url` crate is not a direct dependency
- **Spec says:** "No new external dependencies... `url` parsing is already in the tree" (§9.5, as originally written). FR-NEW-006 requires the hostname to be obtained by parsing the URL.
- **Code does:** `url` appears in `Cargo.lock` as a transitive dependency only. It is absent from `Cargo.toml` `[workspace.dependencies]` and from `crates/mcp-fs/Cargo.toml` `[dependencies]`, and no `url::` reference exists anywhere in `crates/mcp-fs/src`.
- **Nature:** missing capability
- **Resolution during implementation:** Add `url = "2.5"` to `[workspace.dependencies]` and `url = { workspace = true }` to `crates/mcp-fs/Cargo.toml`. Section 9.5 has already been corrected to state one new direct dependency.
- **Detected by:** `cargo build` fails with `error[E0432]: unresolved import url`.
- **Blocks which requirement:** FR-NEW-006, and through it FR-NEW-007, FR-NEW-041, FR-NEW-042.
- **Status:** resolved (`git::remote::validate_host_key` uses `url::Host::parse`; RED confirmed via `error[E0433]: cannot find module or crate \`url\`` with the dependency lines removed, then GREEN restored, this commit)

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-001 [EARS-U]: The host-to-provider map exists in configuration
> The mcp-fs server SHALL expose a configuration key `git.hosts` mapping a hostname to one of the providers `github`, `gitlab`, `generic` or `anonymous`.

- **Inputs:** YAML under `git.hosts`, e.g. `github.ibm.com: github`.
- **Outputs:** An in-memory map available to every remote operation.
- **Business Rules:** The key is the bare hostname. The value is exactly one of the four accepted strings, lowercase.
- **Priority:** Must-have
- **Rationale:** Replaces substring detection at `crates/mcp-fs/src/tools/git.rs:683-691`.

#### FR-NEW-002 [EARS-E]: Boot rejects an unknown provider value
> WHEN the server starts and any `git.hosts` entry names a provider outside `github`, `gitlab`, `generic`, `anonymous`, THE mcp-fs server SHALL refuse to start, naming the offending host and listing the accepted values.

- **Inputs:** `git.hosts` with e.g. `github.ibm.com: githib`.
- **Outputs:** Boot failure, non-zero exit, message naming `github.ibm.com`.
- **Business Rules:** Consistent with the existing property that misconfiguration fails at boot (`README.md:140-142`).
- **Priority:** Must-have

#### FR-NEW-003 [EARS-E]: Boot rejects a duplicate host
> WHEN the server starts and the same hostname appears more than once in `git.hosts`, THE mcp-fs server SHALL refuse to start, naming the duplicated host.

- **Inputs:** Two entries for `github.com`.
- **Outputs:** Boot failure naming `github.com`.
- **Business Rules:** Precedence between duplicates is never inferred.
- **Priority:** Must-have

#### FR-NEW-004 [EARS-E]: Boot rejects a malformed host
> WHEN the server starts and any `git.hosts` key contains a scheme, a path, a port, or a wildcard character, THE mcp-fs server SHALL refuse to start, naming the offending key.

- **Inputs:** `https://github.com`, `github.com/org`, `github.com:443`, `*.acme.corp`.
- **Outputs:** Boot failure naming the key.
- **Business Rules:** Wildcards are rejected rather than supported, because precedence between overlapping patterns is exactly the ambiguity this requirement removes.
- **Priority:** Must-have

#### FR-NEW-005 [EARS-UB]: The map is immutable at runtime
> The mcp-fs server SHALL NOT expose any tool, route or API that modifies `git.hosts` after boot.

- **Inputs:** None.
- **Outputs:** None.
- **Business Rules:** A screen reads the map; nothing writes it.
- **Priority:** Must-have
- **Rationale:** A hot-editable trust routing table breaks the boot-validation property.

#### FR-NEW-065 [EARS-U]: Ownership of the host map is split between declaration and interpretation
> The mcp-fs server SHALL declare `git.hosts` as a field of `GitConfig` in `crates/mcp-fs/src/config.rs`, SHALL implement its validation and its resolution exactly once in `crates/mcp-fs/src/git/remote.rs` as `git::remote::validate_hosts(&GitConfig) -> Result<()>` and `git::remote::resolve_host(&str) -> Result<Provider>`, and `ServerConfig::validate` SHALL call `validate_hosts` rather than inspecting the entries itself.

- **Inputs:** The source tree after implementation.
- **Outputs:** `config.rs` contains the field declaration and one call to `validate_hosts`, and no other inspection of an entry's value. `remote.rs` is the only file that compares a hostname to the map.
- **Business Rules:** Boot validation still fires from `ServerConfig::validate` (`crates/mcp-fs/src/config.rs:813-819`), preserving the property that misconfiguration fails at boot.
- **Priority:** Must-have
- **Rationale:** Closes audit finding F-A8. FR-NEW-054 and E2E-NEW-197 required `git.hosts` to be read in exactly one file, which no implementation validating in `config.rs` could satisfy.

## Acceptance Tests

> **100% must pass.** The story is not implemented while one test fails. Loop fix, run, check until zero failures. Run the suite through the project's chain: `./test.sh` (`cargo test --workspace`). Never run a test binary directly or with an ad hoc command. `--all-features` matters: without it the two optional relational drivers are never compiled.

### Test Data

| Data | Description | Source | Status |
|------|-------------|--------|--------|
| `alice@test.com` | Primary person fixture, used by every test below | fixture | ready |
| `bob@test.com` | Second person, for isolation assertions | fixture | ready |
| `proj-a` | Mount id used by every volume-scoped test | fixture | ready |
| `ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH` | GitHub-shaped token fixture | fixture | ready |
| `glpat_1111222233334444555566667777` | GitLab-shaped token fixture | fixture | ready |
| Reference host map | `github.com: github`, `github.ibm.com: github`, `gitlab.acme.corp: gitlab`, `git.acme.internal: generic`, `public.example.org: anonymous` | fixture | ready |
| `git.unknown.test` | Host deliberately absent from the map | fixture | ready |

### The 17 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-001** — Valid map boots and resolves every provider class
- **Category:** Feature · **Scenario:** SC-001 · **Requirements:** FR-NEW-001, FR-NEW-006 · **Priority:** Critical
- Given the reference map above
- When the server starts and each of the five hosts is resolved
- Then the server boots successfully
- And `github.com` resolves to `github`, `github.ibm.com` to `github`, `gitlab.acme.corp` to `gitlab`, `git.acme.internal` to `generic`, `public.example.org` to `anonymous`

**E2E-NEW-002** — Two hosts may share one provider
- **Category:** Feature · **Scenario:** SC-001 · **Requirements:** FR-NEW-001 · **Priority:** Critical
- Given `github.com: github` and `github.ibm.com: github`
- When the server starts
- Then it boots and both hosts resolve to `github` independently

**E2E-NEW-003** — Unknown provider value fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-002 · **Priority:** Critical
- Given `git.hosts` containing `github.ibm.com: githib`
- When the server starts
- Then startup fails
- And the error message contains `github.ibm.com` and lists `github`, `gitlab`, `generic`, `anonymous`

**E2E-NEW-004** — Uppercase provider value fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-002 · **Priority:** High
- Given `github.com: GitHub`
- When the server starts
- Then startup fails naming `github.com`, because provider values are lowercase-exact

**E2E-NEW-005** — Empty provider value fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-002 · **Priority:** High
- Given `github.com: ""`
- When the server starts
- Then startup fails naming `github.com`

**E2E-NEW-006** — Duplicate host fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-003 · **Priority:** Critical
- Given `github.com: github` appearing twice, the second time as `github.com: anonymous`
- When the server starts
- Then startup fails and the error names `github.com`
- And the server does not silently choose either entry

**E2E-NEW-007** — Host with a scheme fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `https://github.com: github`
- When the server starts
- Then startup fails naming `https://github.com`

**E2E-NEW-008** — Host with a path fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `github.com/org: github`
- When the server starts
- Then startup fails naming `github.com/org`

**E2E-NEW-009** — Host with a port fails boot
- **Category:** Error · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `github.com:443: github`
- When the server starts
- Then startup fails naming the key

**E2E-NEW-010** — Wildcard host fails boot
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-004 · **Priority:** Critical
- Given `*.acme.corp: gitlab`
- When the server starts
- Then startup fails naming `*.acme.corp`
- And the failure message does not suggest wildcards are supported

**E2E-NEW-013** — Empty map boots
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001 · **Priority:** High
- Given `git.hosts: {}`
- When the server starts
- Then it boots successfully
- And a subsequent clone of `https://github.com/o/r.git` fails as an undeclared host

**E2E-NEW-014** — Absent map section boots
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-001 · **Priority:** High
- Given a `git` config with no `hosts` key
- When the server starts
- Then it boots, and behaves as E2E-NEW-013

**E2E-NEW-017** — The map cannot be mutated at runtime
- **Category:** Security · **Scenario:** SC-001 · **Requirements:** FR-NEW-005 · **Priority:** Critical
- Given a running server with the reference map
- When the registered tool list and HTTP route table are enumerated
- Then no tool and no route accepts input that changes a host-to-provider entry

**E2E-NEW-111** — The screen cannot modify the host map
- **Category:** Edge · **Scenario:** SC-006 · **Requirements:** FR-NEW-005 · **Priority:** Critical
- When every request the screen can issue is enumerated
- Then none changes a host-to-provider entry

**E2E-NEW-224** — Validation and resolution live in one module
- **Category:** Integration · **Scenario:** Cross-cutting · **Requirements:** FR-NEW-065 · **Priority:** Critical
- When the source tree is scanned after implementation
- Then `git::remote::validate_hosts` and `git::remote::resolve_host` exist in `crates/mcp-fs/src/git/remote.rs`
- And `config.rs` contains the `hosts` field declaration and exactly one call to `validate_hosts`, and no other inspection of an entry's value

**E2E-NEW-225** — Boot validation still fires from `ServerConfig::validate`
- **Category:** Feature · **Scenario:** SC-001 · **Requirements:** FR-NEW-065 · **Priority:** Critical
- Given a map with an unknown provider value
- When the server starts
- Then boot fails, proving `validate_hosts` is reached from `ServerConfig::validate`

**E2E-NEW-226** — A valid map passes validation from the same path
- **Category:** Edge · **Scenario:** SC-001 · **Requirements:** FR-NEW-065 · **Priority:** High
- Given the reference map
- When the server starts
- Then it boots and every host resolves through `git::remote::resolve_host`

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Do not validate entries inside `config.rs` (FR-NEW-065 forbids it: `ServerConfig::validate` calls `git::remote::validate_hosts` and inspects nothing itself).
- Do not accept wildcards, and do not add a `*.example.com` fallback (DEC-015).

### Scope Boundary
Declare and validate the map, and expose `resolve_host`. Do NOT wire resolution into `git.remote_clone`; that is US-002. Do NOT add `git.remote_timeout_secs`; that is US-014.

## Non Regression

### Existing Tests That Must Pass
- The existing boot-validation property holds: a server with a valid config still starts, and `validate_store` calls at `crates/mcp-fs/src/config.rs:817` are unaffected.
- A config with no `git.hosts` key at all still boots (EXC-001d). Remote operations then fail per US-002, which is a later story: do not make the absent map a boot failure.
- The whole suite stays green: `cargo test --workspace` with `--all-features`, plus `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check`.

### Behaviors That Must Not Change
- Platform admin manages projects and membership and gains **no** implicit file or token access.
- `volume_id` scopes every row of `nodes`, `blob_refs` and `git_*`; it belongs in EVERY `WHERE` clause.
- Secrets come from the environment only. Never log a token or a key.
- A store never speaks a driver: it speaks `storage::rel::RelationalDb`. Never block a request thread on the database.

### API Contracts to Preserve
- `mount_id` stays required on every `fs.*` and `git.*` tool.
- Errors stay `ToolError::<code>` carrying a stable `ERR_*` from the closed set at `crates/mcp-fs/src/errors.rs:9-22`.
- Tool parameter names stay snake_case; parameter descriptions are frozen LLM-facing docs.

## Self-Review Checklist

Full 4-axis self-review per `/implement` Phase 3.3 Step 5. In addition, before declaring this story done:

1. **Spec compliance.** Every FR above is satisfied by code you can point at, and every test above passes by its own assertion, not by a weakened one.
2. **One implementation.** You added no second copy of host resolution, URL validation, credential supply or timeout. `git2::RemoteCallbacks` is still constructed in exactly one file; `git.hosts` is still read in exactly one file.
3. **No invented decision.** Every choice you made traces to a `DEC-` entry quoted above or to an explicit FR. If you had to decide something this story does not settle, you stopped and said so rather than choosing.
4. **No token anywhere.** No token value reaches a response, a log line, a tracing record, an audit entry, an error message or an HTTP body.
5. **Drift closed.** DRIFT-003 is resolved in this branch, by the resolution quoted above, and you can name the test that proves it.
