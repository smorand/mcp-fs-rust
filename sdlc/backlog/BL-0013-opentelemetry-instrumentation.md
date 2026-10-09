# Specification Backlog

Ideas raised during specification work and deliberately deferred. Each entry records
why it was deferred and which spec surfaced it. Nothing here is a commitment.

## BL-001: OpenTelemetry instrumentation

**Description.** Replace or supplement the current stderr-only `tracing` setup with an
OpenTelemetry pipeline: OTLP span export behind a config switch, plus rolling file logs.
Spans on the boundaries that matter here: MCP tool calls (tool name, outcome, duration),
relational queries (statement preview, rows, duration), blob operations, and identity
verification. Never on token contents, key material or DSN contents.

**Current state.** Not implemented. The only telemetry dependency in the workspace is
`tracing-subscriber` (`Cargo.toml:98`, `crates/mcp-fs/Cargo.toml:79`). `logging.rs:29-37`
installs a plain `fmt` subscriber writing to stderr, filtered by `RUST_LOG` with a default
of `info`. There is no OTLP exporter, no span export and no file appender.

**Rationale for deferral.** The platform foundation specification is a retro-specification
of shipped behaviour. Writing OTel requirements into it would convert a document that
describes the system into a document that changes it, and would put a large new dependency
surface inside a spec whose value is that it asserts nothing untrue about the code.

**Suggested by.** `SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md`, §7.5, DEC-005.

## BL-002: Specification of the `crates/agent` CLI client

**Description.** A retro-specification of the interactive CLI agent: its config schema, the
LLM streaming and tool-calling loop, the wrap-aware line editor, markdown-to-ANSI rendering,
and the terminal invariants it depends on.

**Current state.** Implemented and shipping (`crates/agent/src/`: `main.rs`, `mcp.rs`,
`llm.rs`, `input.rs`, `ui.rs`, `session.rs`, `spinner.rs`, `config.rs`), documented in
`.agent_docs/agent.md`.

**Rationale for deferral.** The agent is an MCP **client**, not part of the server product
(`crates/agent/src/main.rs:3-5`). Its real value is as the end-to-end exerciser of the tool
surface, which the server specs already assert directly. Specifying it competes for effort
with the seven remaining server specs.

**Suggested by.** The eight-spec split, DEC-001.

## Theme: Horizontal scaling

Everything needed for a deployment to safely run more than one `mcp-fs` process behind a
load balancer. Both items below are in-memory, per-process state with no cross-replica
coordination: BL-003 is the session/write-quota map, BL-014 is the wider picture including
the git write lock, which has the exact same shape.

### BL-003: Shared session state across replicas

**Description.** Make the write quota and the read-before-write guard shared rather than
per process, so that horizontal scaling does not silently change their semantics.

**Current state.** Session state is an in-memory map keyed by `(person, project_id)`
(`crates/mcp-fs/src/safety.rs:2-4,79-86`). With more than one replica, a caller's quota is
effectively multiplied by the replica count and the read guard can be satisfied on one
replica and enforced on another.

**Rationale for deferral.** This is a product decision about the intended deployment
topology, not a defect in the current single-process design. Recorded as TBD-001 in the
platform foundation spec rather than solved there.

**Suggested by.** `SPEC-0002_2026-09-18_17-37-46-platform-foundation/spec.md`, §15 TBD-001.

### BL-014: Horizontal scaling readiness (multiple server replicas)

**Description.** The full set of coordination gaps that block running more than one
`mcp-fs` replica: shared session/quota state (BL-003), a shared git write lock, and a
relational backend suited to multi-writer access.

**Current state.** Nothing in the codebase assumes more than one replica. The per-project
git write lock that serializes commits, pushes and pulls is an in-memory `tokio::sync::Mutex`
per process (`crates/mcp-fs/src/git/repo.rs:44`), so two replicas can each acquire their own
lock for the same repository and interleave writes to it, corrupting its history. SQLite, the
default relational backend, is a single file with one writer; PostgreSQL and SQL Server
already support concurrent writers.

**Risk analysis.** With more than one replica and no fix: a caller's write quota is
effectively multiplied by the replica count (BL-003); two replicas racing the git write lock
can corrupt a repository's history; a SQLite deployment cannot scale past one writer
regardless of any fix here.

**Rationale for deferral.** No current deployment runs more than one replica. Solving this
without a concrete need means guessing at the coordination primitive (distributed lock,
leader election, sticky routing) instead of building for the topology actually in use.

**Suggested by.** User request, 2026-09-21, grouping the horizontal-scaling gaps as one theme
alongside BL-003.

### BL-015: Two horizontal-scaling strategies, and the subsystem breakdown for the stateless one

**Description.** A comparison, surfaced while evaluating whether a shared parallel filesystem
(IBM Storage Scale, JuiceFS) could sit under `mcp-fs` as a backend, of two different ways to run
more than one replica, plus the full list of per-subsystem work the fully-stateless strategy
requires. Recorded here for whoever picks up BL-003/BL-014 next, so the option space does not
need to be rediscovered.

**Strategy A: shard by project, not fully stateless.** `mcp-fs` already isolates every project's
state: its own relational data (one SQLite file, or one `volume_id` partition), its own bare git
repository on disk, its own blob bucket, its own search index. A routing layer in front of N
independent replicas, keyed by `project_id` (consistent hashing or an explicit registry), gets
multi-tenant capacity without touching any subsystem: each replica keeps exactly the single-node
design it has today. The limit is that one project's throughput stays bounded by one replica,
which is rarely the actual constraint (a single MCP session rarely saturates a node). This is the
cheaper strategy and needs no new infrastructure class.

**Strategy B: any replica can serve any project (what BL-003/BL-014 describe).** This needs four
separate fixes, not one:

1. **Relational backend** (BL-014's third gap). PostgreSQL/SQL Server already give concurrent
   writers, but remain single-primary: they scale reads (replicas), not writes, across nodes.
   Genuine multi-node write scaling needs distributed SQL (Citus, CockroachDB, YugabyteDB, TiDB)
   or a distributed KV metadata engine (TiKV, FoundationDB), the same class of engine JuiceFS
   Community lets you plug in, and the same problem JuiceFS Enterprise's proprietary Raft engine
   exists to solve. Plain PostgreSQL may still be enough in practice; distributed SQL is a step to
   take only once PostgreSQL's single-primary write throughput is the measured bottleneck.
2. **Git bare repositories on local disk.** `state/git-repos/{project_id}/` is a hard local-
   filesystem dependency: libgit2 requires a real filesystem, not an API. No database change
   touches this. Two options: keep project-affinity routing for git operations specifically
   (Strategy A, scoped to just this subsystem), or put `state/git-repos/` on a filesystem shared
   across all replicas. This second option is the one place in the whole evaluation where a shared
   parallel/clustered POSIX filesystem (Storage Scale, JuiceFS) would have a concrete, scoped job:
   nothing else in `mcp-fs` needs one.
3. **Search index (Tantivy BM25 + vector).** Local, on-disk, per-process; not distributed. Needs
   either a distributed search/vector backend (Elasticsearch/OpenSearch/Meilisearch for BM25;
   Qdrant/Milvus/pgvector-on-Citus for vectors) behind the existing `search.*` tools, or per-shard
   indexes under Strategy A. IBM's Content-Aware Storage was evaluated as one example of an
   external, ACL-aware, incrementally-updated vector index that could sit behind `search.*`
   without touching the rest of the engine; noted as a reference architecture, not a
   recommendation, since it requires standing up Storage Scale and Fusion underneath it.
4. **In-memory state.** OAuth tokens (in-memory by default) and the session/write-quota map
   (BL-003) must move to a store every replica can read and write: Redis, or the same relational
   backend as (1). Left as-is, a token issued on one replica is invisible to another, and a
   caller's quota is multiplied by the replica count exactly as BL-003 already describes.

**Why this belongs next to BL-003/BL-014 rather than replacing them.** Strategy B, done properly,
re-derives problems that IBM Storage Scale and JuiceFS already solve for a general-purpose
parallel filesystem: distributed metadata, a data plane shared across nodes, distributed search.
Neither product offers a data-plane API matching what `mcp-fs` needs (read/write/edit/patch/glob/
grep) though: both expose management-plane and, in IBM's case, vector-search-plane APIs only. Any
adoption of either as a backend would mean reimplementing `core/fs_ops.rs` against a mounted
filesystem instead of the current relational-plus-blob storage, losing the current
content-addressed dedup and refcount GC unless rebuilt on top, and reconciling `mcp-fs`'s
project-based ACL (a JWT claim plus a membership table, no OS identity involved) with POSIX
UID/GID and, in practice, an AD/LDAP-backed identity federation, since both products enforce
access through OS-level identity. That reconciliation can be automated with OAuth2 plus an
identity-translation service in front of the directory, but the directory dependency itself does
not go away, and group membership (not user identity) is the harder half to keep synchronized,
since a project's membership table has no POSIX equivalent to map onto cleanly.

**Rationale for deferral.** Same as BL-014: no current deployment needs more than one replica.
Recorded so the strategy choice (A vs B) and, if B, the subsystem-by-subsystem plan are available
when the need becomes concrete, rather than re-litigated then.

**Suggested by.** User request, 2026-09-21, following a comparison of `mcp-fs` against IBM Storage
Scale, IBM Content-Aware Storage and JuiceFS as possible backends.

## Theme: Git full support

BL-004 through BL-013 are the pieces still missing between the current git surface and what
"full git support" would mean: force push, pull request creation, squash merge, real conflict
resolution, remote management beyond `origin`, refspec flexibility, discarding changes,
pluggable credentials, Azure DevOps, and the consolidated divergence register (BL-013) that
ties the set together. Grouped here so they get reviewed together rather than picked up
piecemeal; none is a commitment.

### BL-004: Force push on `git.remote_push`

**Description.** Add `force: bool` to `git.remote_push`, allowing a non-fast-forward push to
overwrite the remote ref.

**Current state.** Push is fast-forward only (FR-NEW-024). A non-fast-forward is refused with a
distinct error stating force is not supported.

**Risk analysis.** A force push destroys commits on the remote irreversibly. The pusher here is
frequently an autonomous agent, authenticating with a person's personal token, against a
repository the person did not necessarily choose. A mistaken force push is attributed to the
person and is not recoverable from the server side. If implemented, the recommendation is:
`force` defaults to false; it is rejected outright for any ref matching a configured
protected-branch pattern; every forced push records an audit entry naming the overwritten sha
so the prior tip can be recovered from the reflog; and it is gated by a server-level config
flag that is off by default, so a deployment must opt in before any caller can use it.

**Rationale for deferral.** Deliberate scope exclusion at interview (DEC-013). The recovery
paths that make force safe are themselves a body of work.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-013.

### BL-005: Pull request creation

**Description.** A tool creating a pull or merge request on the provider, after a push.

**Current state.** Not implemented. The provider REST APIs are not called at all; the only
outbound traffic is the git transport.

**Rationale for deferral.** Requires per-provider REST clients, a token scope beyond repository
read/write, and a result model that differs between GitHub and GitLab. Out of scope at
interview.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-014 discussion.

### BL-006: Squash merge

**Description.** Collapse a branch's commits into one before or during merge.

**Current state.** Not implemented. The merge path creates an ordinary merge commit with two
parents (FR-NEW-034).

**Rationale for deferral.** Out of scope at interview. Interacts with BL-005, since squash is
usually a property of how a pull request is merged rather than of a local operation.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-014 discussion.

### BL-007: Real merge conflict resolution

**Description.** Per-file or interactive conflict resolution, rather than one global `ours` or
`theirs` applied to every conflicting file.

**Current state.** `git.remote_pull` accepts `on_conflict` of `ours` or `theirs` and applies it
to every conflict (FR-NEW-031). Conflict markers are prohibited from entering the volume
(FR-NEW-032).

**Rationale for deferral.** The volume is the working tree and there is no index, so representing
an unresolved conflict requires deciding what a half-merged simulated filesystem looks like to
`fs.read`, `fs.write` and `git.status`. That is a subsystem, not a parameter.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-024.

### BL-008: Remote management tools

**Description.** `git.remote_add`, `git.remote_remove`, `git.remote_list`, exposing the existing
storage API, plus support for remotes other than `origin`.

**Current state.** `git_remotes` holds `(volume_id, name, url)` and `add_remote`,
`remove_remote` and `list_remotes` are implemented and tested
(`crates/mcp-fs/src/git/db.rs:69-75`, `:281-305`, `:429-442`). After FR-NEW-020 the only writer
is `git.remote_clone`, which records exactly one remote named `origin`. Push, fetch and pull
resolve `origin` and take no `url`.

**Consequence today.** A volume created by `git.init` has no `origin` and therefore cannot push,
fetch or pull at all. Its only route to a remote is to be cloned instead.

**Rationale for deferral.** Excluded at interview (DEC-021) to keep the remote surface to one
unambiguous target.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-021.

### BL-009: Remote branch name distinct from the local branch

**Description.** A `remote_branch` parameter on `git.remote_push`, allowing `local:remote`
refspecs.

**Current state.** The remote branch name always equals the local one (FR-NEW-022).

**Rationale for deferral.** Deferred at interview as a later refinement.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-014.

### BL-010: `git.discard_changes`

**Description.** Reset a volume's files to its current branch tip: overwrite modified files,
restore deleted ones, remove added ones.

**Current state.** `git.remote_pull` refuses a dirty volume and instructs the caller to commit or
discard (FR-NEW-029). Committing works, because divergence is then resolvable by merge. Discarding
has no single-call implementation: the nearest tool is `git.checkout_file`
(`crates/mcp-fs/src/tools/git.rs:222`), which restores one file per call and cannot remove a file
the volume added.

**Rationale for deferral.** Once `on_conflict` merge existed (DEC-024), committing became a
working escape, so discard stopped being the only way out of a dirty volume. Still worth having.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-025.

### BL-011: Pluggable credential providers

**Description.** A `CredentialProvider` trait so a provider can supply a credential shape other
than `oauth2:<token>`.

**Current state.** Every provider uses `git2::Cred::userpass_plaintext("oauth2", &t)`
(`crates/mcp-fs/src/tools/git.rs:1011-1013`), which works for read and write on both GitHub and
GitLab, and on their enterprise deployments.

**Rationale for deferral.** Rejected as approach C at interview: a trait with four
implementations that all return the identical credential is indirection with no current payer.
It becomes worthwhile the moment a second credential shape genuinely exists, which is the same
moment BL-012 becomes relevant.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-034.

### BL-012: Azure DevOps support

**Description.** Support Azure DevOps as a provider.

**Current state.** Excluded. The provider set is `github`, `gitlab`, `generic`, `anonymous`
(FR-NEW-001). Azure DevOps suffers the same substring-detection defect this specification fixes,
and would work today as a `generic` host if its credential convention matched.

**Rationale for deferral.** Azure DevOps expects the PAT as the password with an arbitrary or
empty username, not the literal `oauth2`. Supporting it properly means BL-011 first.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, DEC-002.

### BL-013: Divergences from standard git, for review

**Description.** A consolidated register of every way this server's git behaviour differs from
ordinary git. Each was a deliberate trade, but nobody has reviewed them as a set, and together
they define what "git support" means here.

**The divergences, as of this specification:**

| Area | Standard git | Here | Where decided |
|---|---|---|---|
| Working tree | Index plus working tree plus HEAD | No index. The volume **is** the working tree. | Pre-existing |
| Staging | `git add` selects what to commit | `git.commit` commits the whole volume state | Pre-existing, `git.rs:194` |
| Dirty tree | Many operations warn or stash | Pull refuses; no stash exists | FR-NEW-029 |
| Discarding changes | `git checkout -- .`, `git restore` | No single-call equivalent; see BL-010 | DEC-025 |
| Merge conflicts | Conflict markers, manual resolution, `git mergetool` | One global `ours`/`theirs`; markers prohibited | DEC-024, FR-NEW-032 |
| Rebase, cherry-pick, revert | Available | Not implemented | Never specified |
| Stash | Available | Not implemented | Never specified |
| Push refspecs | Arbitrary `local:remote`, multiple refs, tags | One branch, same name both sides, no tags | DEC-014, BL-009 |
| Force push | `--force`, `--force-with-lease` | Refused outright | DEC-013, BL-004 |
| Remotes | Many, freely managed | Exactly one, named `origin`, created only by clone | DEC-021, BL-008 |
| Transport | HTTPS, SSH, git, file, local paths | HTTPS only | DEC-009, FR-NEW-041 |
| Credentials in URLs | Accepted by git | Rejected | FR-NEW-042 |
| Unknown host | Just tries, anonymously | Error unless declared | DEC-010, FR-NEW-007 |
| Credential helpers | Pluggable, per host | One shape, `oauth2:<token>` | DEC-009, BL-011 |
| Token lifetime | Helper's concern | No refresh or rotation | DEC-009 |
| Submodules | Supported | Not supported | Never specified |
| Partial apply | `git checkout` is atomic | Clone tolerates per-file failure and reports `skipped`; pull is atomic | FR-NEW-035 vs `git.rs:790-795` |

**Rationale for deferral.** Each divergence is individually justified. The review question is
whether the set is coherent, and which gaps matter enough to close. That is a product
conversation, not a defect.

**Suggested by.** `archived/SPEC-0010_2026-09-21_00-34-13-github-enterprise-and-token-store/spec.md`, user request during
Round 2b.
