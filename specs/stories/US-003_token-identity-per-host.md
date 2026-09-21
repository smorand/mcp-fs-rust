# US-003: Token identity becomes (person, host), in memory, on disk, and across backends

> Parent Spec: `specs/2026-09-21_00-34-13-github-enterprise-and-token-store.md`
> Epic: n/a (collapsed at tier 1)
> Status: ready
> Priority: 3
> Depends On: US-001
> Complexity: L
> min_tier: 1
> Files touched: 6

## Objective

One person holds distinct tokens for `github.com` and `github.ibm.com`, both provider `github`, which the current `(person, provider)` key makes impossible. The in-memory key, the persisted primary key on all three engines, and the `migrate` verb all move to `(person, host)` in one change, because a server whose memory key and table key disagree cannot load its own state.

## Technical Context

### Stack
Rust 2024, axum + tokio, git2 0.20.4 (libgit2), rusqlite (bundled), sqlx (PostgreSQL, optional), tiberius-ng + bb8 (SQL Server, optional), jsonwebtoken + rsa, aes-gcm, serde, tracing. Build and gate: `./test.sh` (`cargo test --workspace`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

### Relevant File Structure
```
crates/mcp-fs/src/git/oauth/store.rs  # key() at :65-67, store_token :115-123, get_token :148-150
crates/mcp-fs/src/git/oauth/persistence.rs  # primary key at :42, columns :33-34, upsert :103-108, delete :130-135
crates/mcp-fs/src/storage/rel/schema.rs  # :115-116 states the no-DDL-change limit; :237-241 renders CREATE TABLE IF NOT EXISTS
crates/mcp-fs/src/storage/rel/dialect.rs  # :399-420 renders ALTER TABLE ADD COLUMN
crates/mcp-fs/src/migrate.rs  # :200-212 project copy, :231-247 nodes/blob_refs, :273-277 git TABLES
```

### Existing Patterns
- `OAuthTokenStore` keys entries `format!("{person}:{provider}")`, lowercased (`crates/mcp-fs/src/git/oauth/store.rs:65-67`). That function is the single place the key shape is decided.
- `host` must be `TextKey`-bounded like the existing `person` and `provider` columns (`crates/mcp-fs/src/git/oauth/persistence.rs:33-34`), because SQL Server cannot index `NVARCHAR(MAX)`.
- The copy loop with its row-count check is at `crates/mcp-fs/src/migrate.rs:204-211`. Reuse that shape for `oauth_tokens`.
- Backend parity is proved by the one suite in `crates/mcp-fs/src/storage/conformance.rs`, which every engine runs.

### Data Model (excerpt)
- `OAuthSession` — gains `host`; `provider` becomes a non-key attribute. `access_token`, `scopes`, `expires_at`, `instance_url` unchanged (`crates/mcp-fs/src/git/oauth/store.rs:24-32`).
- `oauth_tokens` row — primary key becomes **`(person, host)`**, was `(person, provider)` (`persistence.rs:42`). `host` is `TextKey`-bounded. All legacy rows dropped (FR-NEW-011).

### Decisions That Govern This Story
Quoted verbatim from the specification's decisions log (Section 17). These are settled; do not re-litigate them.

- **DEC-001:** Scope is the complete solution: host map, seeding tool, and web token screen. **Rationale:** blocking dependency for Graph Studio; partial delivery leaves its code reader degraded. **Alternatives considered:** two defects with the screen deferred; seeding only; mapping only. **Implemented by:** FR-NEW-001 to FR-NEW-045. **Round:** 1. **Code evidence:** n/a
- **DEC-003:** Token identity becomes per host, not per provider. **Rationale:** a person may hold distinct tokens for `github.com` and `github.ibm.com`, both provider `github`. **Implemented by:** FR-NEW-009, FR-NEW-010. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/git/oauth/store.rs:65-67`, `crates/mcp-fs/src/git/oauth/persistence.rs:42`
- **DEC-004:** The key is `(person, host)`; provider is an attribute of the row. **Alternatives considered:** `(person, provider, host)`, which permits two tokens per host with no known use. **Implemented by:** FR-NEW-009. **Round:** 1. **Code evidence:** n/a
- **DEC-005:** Pre-existing `oauth_tokens` rows are dropped at upgrade, not migrated. **Rationale:** the old rows carry no host, and inferring one from `provider` writes a guess into a primary key. **Alternatives considered:** inferring `github.com`/`gitlab.com`; reading the existing `instance_url` column. **Implemented by:** FR-NEW-011. **Round:** 1. **Code evidence:** `crates/mcp-fs/src/git/oauth/persistence.rs:42`

### Applicable NFRs
- **Backend parity** (§7.4): FR-NEW-010 and FR-NEW-012 behave identically on SQLite, PostgreSQL and SQL Server, exercised by the single conformance suite.
- **Encryption unchanged** (§7.2): only the bearer token is encrypted (`crates/mcp-fs/src/git/oauth/persistence.rs:3`). Re-keying changes no encryption behaviour.
- **Scalability** (§7.7): `host` is a bounded `TextKey` precisely so SQL Server can index it.

### Bounded Context
**Token Custody** — see the specification's Section 4.5. Terminology hazard it exists to prevent: "provider" in Host Resolution is a credential policy, whereas "provider" in Token Custody is an attribute recorded on a stored session; they carry the same values but are not the same concept. "remote" in Remote Operations means the stored `origin` row, never a libgit2 `Remote` handle.

## Implementation Drift To Close In This Story

> Carried verbatim from the specification's Section 19. These are resolved **during** this story, not later. An entry still open when the branch merges moves to `specs/BACKLOG.md`, and that move is a decision someone signs, not a silence.

#### DRIFT-005: The relational schema layer cannot change a primary key
- **Spec says:** FR-NEW-010 requires `oauth_tokens` to be keyed `(person, host)`; FR-NEW-011 requires every pre-existing row to be dropped and the table rebuilt.
- **Code does:** Migration renders only `CREATE TABLE IF NOT EXISTS` (`crates/mcp-fs/src/storage/rel/schema.rs:237-241`) and `ALTER TABLE ... ADD COLUMN` (`crates/mcp-fs/src/storage/rel/dialect.rs:399-420`), a limit stated outright at `schema.rs:115-116`. There is no `DROP TABLE` renderer and no primary-key migration in any dialect. Applied against an existing table, the new `SchemaSet` is a **silent no-op**: the old `(person, provider)` key and the missing `host` column both survive.
- **Nature:** missing capability
- **Resolution during implementation:** Add a dialect-rendered drop-and-recreate step to the relational schema layer, guarded so it fires only when the live `oauth_tokens` lacks a `host` column. **The guard is not optional:** without it every restart destroys valid tokens. Exercise it on all three engines through `crates/mcp-fs/src/storage/conformance.rs`.
- **Detected by:** E2E-NEW-037 fails on SQLite. If the guard is omitted, the existing `survives_reopen_on_disk` test (`crates/mcp-fs/src/git/oauth/persistence.rs:361`) fails.
- **Blocks which requirement:** FR-NEW-010, FR-NEW-011, and therefore FR-NEW-009 in persisted mode.
- **Status:** open

#### DRIFT-004: The `migrate` verb does not copy `oauth_tokens` at all
- **Spec says:** §9.1 lists `crates/mcp-fs/src/migrate.rs` as a **Moderate** impact that "carries the new `oauth_tokens` key". FR-NEW-012 requires the verb to copy the table under the new key.
- **Code does:** `migrate()` copies `project` and `project_member` (`crates/mcp-fs/src/migrate.rs:200-212`), `nodes` and `blob_refs` (`:231-247`), and `crate::git::db::TABLES` (`:273-277`). `oauth_tokens` is never opened and never copied; `crates/mcp-fs/src/git/oauth/persistence.rs:28` `schema()` has no caller in `migrate.rs`.
- **Nature:** missing capability
- **Resolution during implementation:** FR-NEW-012 is new behaviour, not a re-key. Open the source and destination oauth stores, apply `git::oauth::persistence::schema()` to both, and copy the table using the same row-count check as `migrate.rs:204-211`. Correct the §9.1 impact from "Moderate" to "New behaviour".
- **Detected by:** E2E-NEW-041 fails, because the destination holds zero rows.
- **Blocks which requirement:** FR-NEW-012.
- **Status:** open

## Functional Requirements

Quoted verbatim from the specification's Section 6. The EARS statement is the contract.

#### FR-NEW-009 [EARS-U]: Token identity is per host
> The mcp-fs server SHALL key every stored credential on the pair `(person, host)`, case-insensitively on both parts, holding at most one token per pair.

- **Inputs:** `store_token("alice@test.com", "github.ibm.com", …)`.
- **Outputs:** An entry retrievable by `("ALICE@TEST.COM", "GitHub.IBM.com")`.
- **Business Rules:** Replaces `format!("{person}:{provider}")` at `crates/mcp-fs/src/git/oauth/store.rs:65-67`. The provider becomes an attribute of the stored session, not part of its identity.
- **Priority:** Must-have
- **Rationale:** One Person holds distinct tokens for `github.com` and `github.ibm.com`, both provider `github`.

#### FR-NEW-010 [EARS-U]: The persisted primary key is per host
> The mcp-fs server SHALL persist credentials in `oauth_tokens` with primary key `(person, host)` on SQLite, PostgreSQL and SQL Server alike.

- **Inputs:** Schema definition.
- **Outputs:** A table whose key columns are `person` and `host`, both `TextKey`.
- **Business Rules:** Replaces `vec!["person", "provider"]` at `crates/mcp-fs/src/git/oauth/persistence.rs:42`. `host` is `TextKey` bounded, because SQL Server cannot index `NVARCHAR(MAX)`.
- **Priority:** Must-have

#### FR-NEW-011 [EARS-E]: Migration drops legacy rows
> WHEN the server starts against an `oauth_tokens` table keyed `(person, provider)`, THE mcp-fs server SHALL drop every pre-existing row and rebuild the table keyed `(person, host)`.

- **Inputs:** An existing table with rows for `(alice@test.com, github)`.
- **Outputs:** A table on the new key containing zero rows.
- **Business Rules:** No host is inferred from a legacy provider value. Every Person re-authenticates once.
- **Priority:** Must-have
- **Rationale:** DEC-005. Inferring `github.com` from `provider=github` would write a guess into a primary key.

#### FR-NEW-012 [EARS-E]: The migrate verb carries the new key
> WHEN `mcp-fs migrate` copies relational state between backends, THE mcp-fs server SHALL copy `oauth_tokens` under the `(person, host)` key and SHALL drop rows found in the legacy format.

- **Inputs:** `mcp-fs migrate --from sqlite.yaml --to postgres.yaml`.
- **Outputs:** A destination table on the new key.
- **Priority:** Must-have

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

### The 13 tests this story owns

Quoted verbatim from the specification's Section 12. Each test is owned by exactly one story.

**E2E-NEW-032** — Two hosts of the same provider hold independent tokens
- **Category:** Data Integrity · **Scenario:** SC-002 · **Requirements:** FR-NEW-009 · **Priority:** Critical
- When tokens `ghp_PUBLIC…` and `ghp_ENTERPRISE…` are seeded for `github.com` and `github.ibm.com`
- Then both are retrievable independently
- And a clone from `github.com` supplies `ghp_PUBLIC…` while one from `github.ibm.com` supplies `ghp_ENTERPRISE…`
- *This is the collision that the old `(person, provider)` key made impossible.*

**E2E-NEW-033** — Host is matched case-insensitively on seeding
- **Category:** Edge · **Scenario:** SC-002 · **Requirements:** FR-NEW-009 · **Priority:** High
- When a token is seeded for `GitHub.IBM.com`
- Then `git.auth_status` reports it under `github.ibm.com`
- And a clone of `https://github.ibm.com/o/r.git` uses it

**E2E-NEW-036** — The persisted key is `(person, host)`
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-010 · **Priority:** Critical
- When the schema is created
- Then `oauth_tokens` has primary key columns `person` and `host`, both `TextKey`-bounded
- And `provider` is a non-key column

**E2E-NEW-037** — Legacy rows are dropped on upgrade
- **Category:** State Transition · **Scenario:** SC-008 · **Requirements:** FR-NEW-011 · **Priority:** Critical
- Given an `oauth_tokens` table keyed `(person, provider)` holding rows for `(alice@test.com, github)` and `(bob@test.com, gitlab)`
- When the server starts on the new version
- Then the table is keyed `(person, host)` and contains zero rows
- And `git.auth_status` reports no tokens for either person

**E2E-NEW-038** — Migration is identical on all three backends
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-010, FR-NEW-012 · **Priority:** Critical
- When the conformance suite runs on SQLite, PostgreSQL and SQL Server
- Then each produces a table keyed `(person, host)` with `host` indexable
- And the observable behaviour of store, get, revoke and list is identical across all three

**E2E-NEW-039** — A failing migration fails boot, naming the backend
- **Category:** Error · **Scenario:** SC-008 · **Requirements:** FR-NEW-011 · **Priority:** High
- Given a backend that rejects the schema change
- When the server starts
- Then boot fails and the message names the backend

**E2E-NEW-040** — A fresh install performs no drop
- **Category:** Edge · **Scenario:** SC-008 · **Requirements:** FR-NEW-011 · **Priority:** High
- Given no `oauth_tokens` table
- When the server starts
- Then the table is created on the new key and boot succeeds

**E2E-NEW-041** — The migrate verb carries the new key
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-012 · **Priority:** High
- Given a source holding tokens keyed `(person, host)`
- When `mcp-fs migrate --from a.yaml --to b.yaml` runs
- Then the destination holds the same rows under the same key, values intact and still decryptable

**E2E-NEW-042** — The migrate verb drops legacy-format source rows
- **Category:** Error · **Scenario:** SC-008 · **Requirements:** FR-NEW-012 · **Priority:** High
- Given a source in the old `(person, provider)` format
- When `migrate` runs
- Then the destination table is on the new key and contains no rows inferred from provider values

**E2E-NEW-043** — Keying is caseless on both parts after the change
- **Category:** Edge · **Scenario:** SC-008 · **Requirements:** FR-NEW-009 · **Priority:** High
- When a token is stored for `("Alice@Test.COM", "GitHub.IBM.com")`
- Then it is retrievable as `("alice@test.com", "github.ibm.com")` and as `("ALICE@TEST.COM", "GITHUB.IBM.COM")`
- And it is not retrievable as `("alice@test.com", "github.com")`

**E2E-NEW-082** — Push uses the pusher's own token, not the cloner's
- **Category:** Security · **Scenario:** SC-009 · **Requirements:** FR-NEW-009 · **Priority:** Critical
- Given `proj-a` cloned by `alice@test.com`, and `bob@test.com` a member holding his own token for the same host
- When bob pushes
- Then the credential supplied is bob's token, never alice's

**E2E-NEW-162** — Two people hold independent tokens for one host
- **Category:** Security · **Scenario:** SC-006, SC-008 · **Requirements:** FR-NEW-009, FR-NEW-040 · **Priority:** Critical
- Given alice and bob each seed a different token for `github.ibm.com`
- Then both are stored and retrievable independently
- And a clone by alice supplies alice's token and one by bob supplies bob's
- And neither person's `git.auth_status` or token screen reveals the other's

**E2E-NEW-163** — Revocation by one person does not affect another
- **Category:** Data Integrity · **Scenario:** SC-008 · **Requirements:** FR-NEW-009 · **Priority:** Critical
- Given the state of E2E-NEW-162
- When alice revokes `github.ibm.com`
- Then bob's token for the same host remains and still authenticates a clone

## Constraints

### Files Not to Touch
- `crates/mcp-fs/src/core/fs_ops.rs` — untouched by this specification (§9.1).
- `crates/mcp-fs/src/api/dataplane.rs` — git has no REST surface (§9.1).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json` — regenerated once, last, in US-019.

### Dependencies Not to Add
- No new crate beyond `url = "2.5"`, which US-001 promotes to a direct dependency (DRIFT-003). `git2` 0.20.4 already provides merge.
- No `globset`: glob matching goes through `util::text::Fnmatch`, the single implementation in the tree.

### Patterns to Avoid
- Do not infer a host from a legacy `provider` value. Legacy rows are dropped, never migrated (DEC-005, FR-NEW-011).
- Do not let the drop-and-recreate run unguarded: it fires only when the live `oauth_tokens` lacks a `host` column (DRIFT-005).

### Scope Boundary
The key change and its persistence, everywhere it lands, including `migrate`. Do NOT build `git.token_set` (US-004) and do NOT change the auth tool schemas (US-006, US-007).

## Non Regression

### Existing Tests That Must Pass
- These existing tests move to the new key and must pass: `store_then_get` (store.rs:200), `keying_is_caseless_on_both_parts` (:214), `store_overwrites_the_same_key_regardless_of_casing` (:225), `has_valid_token_respects_expiry` (:260), `revoke_removes_the_session` (:274), `persistent_store_loads_and_writes_through` (:308), `tokens_are_per_person` (git_auth.rs:559).
- `providers_are_independent` (store.rs:234) becomes `hosts_are_independent` and must additionally prove two hosts of the same provider are independent — this is E2E-MOD-001.
- These persistence tests move to the new primary key and must pass: `upsert_then_load` (:231), `upsert_replaces_the_row_for_the_same_key` (:247), `one_row_per_person_provider_pair`→`one_row_per_person_host_pair` (:264), `delete_removes_one_row_and_is_idempotent` (:279), `token_is_not_stored_in_clear` (:291), `rows_encrypted_with_another_key_are_skipped_not_fatal` (:306), `corrupt_blobs_and_bad_timestamps_are_skipped` (:329), `scopes_round_trip_including_empty` (:349).
- **`survives_reopen_on_disk` (`crates/mcp-fs/src/git/oauth/persistence.rs:361`) is the guard test for DRIFT-005.** If the drop-and-recreate step fires unconditionally, this test fails and every restart destroys valid tokens.
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
5. **Drift closed.** DRIFT-005, DRIFT-004 is resolved in this branch, by the resolution quoted above, and you can name the test that proves it.
