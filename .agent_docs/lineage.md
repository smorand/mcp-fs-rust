# Lineage: the C# origin, and why it is no longer a reference

This server began as a strict 1:1 port of a C# implementation. **That is history.** The C#
is not a reference, not a tie breaker and not something to read: it has its own lifecycle,
its own deployment process, and it is diverging further by design. It will never support
PostgreSQL, for instance, because that is not its target. Comparing the two now produces
false signals, so the differential harness that used to enforce equality has been deleted.

Two rules follow:

* **Judge a change on its merits here**: correctness, the tool contract, the tests.
* **Never justify a behaviour with "the C# does it this way".** If a behaviour is right,
  say why it is right. A comment or doc that still argues from the reference is stale and
  should be rewritten when touched.

### About the ~290 remaining mentions in the source

They are deliberate, not an unfinished sweep. Most record *why* a shape is odd: a key
order, a 30 second clock skew, an `ERR_*` mapping, a `fnmatch` semantic. That history is
the only explanation those choices have, so deleting it would leave an arbitrary looking
constant with no rationale, which is worse than a stale name.

What was corrected instead are the statements that became **false** when parity retired:

| Was | Now |
|---|---|
| `lib.rs`: the crate promises "strict 1:1 external parity" | lineage stated as history, contract stated as ours |
| `blob/local.rs`: "a volume written by one must be readable by the other" | the shard layout is our own on disk contract, protecting existing deployments |
| `mcp/mod.rs`: hand rolling "guarantees the 1:1 parity this port requires" | it keeps the wire contract under our control |

The rule for everything else: when you touch a comment that argues from the C#, restate it
as a reason. Do not bulk rewrite them for their own sake.

## The tool contract survived, and is now ours

The one thing worth keeping from the port is the tool surface, because it is a client and
an LLM facing contract regardless of where it came from. It is frozen in two places:

| File | Role |
|---|---|
| `TOOL_CONTRACT.txt` | human readable reference for the 94 tools, their parameters and return shapes |
| `tool-contract-golden.json` | machine checked snapshot: names, descriptions and `inputSchema`, compared on every test run |

Three tests enforce it (`tools/mod.rs`, `tools/all.rs`, `tools/contract_golden.rs`),
including the serialized form, so a reordered schema key fails too. Regenerating the
snapshot is deliberate:

```bash
MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current
```

Review the resulting diff: a description edit should be one line, and 55 changed tools
means something went wrong.

## Schema shapes that a database reader needs to know

These three are load bearing when reading a database or planning an upgrade.

**1. `volume_id` on `nodes`, `blob_refs` and the three `git_*` tables.**
SQLite gets one file per volume, so a tenant column was unnecessary. PostgreSQL and SQL
Server cannot create a database per project on the fly, so one database holds every volume
and the tenant is the first column of the primary key (`PRIMARY KEY (volume_id, path)`).
Under SQLite the column is written but constant per file, so that deployment is unchanged
in layout and behaviour.

*Consequence:* there is no automated downgrade. `mcp-fs migrate` moves state forward
between engines, one direction only.

**2. Dialect binary typing for the OAuth token.**
`token_enc` renders as `BLOB`, `BYTEA` or `VARBINARY(MAX)` per engine. The encryption is
untouched (AES-256-GCM keyed by `MCPFS_TOKEN_KEY`); only the column type varies.

**3. `ColumnType::TextKey(n)` is a bounded key.**
It renders as `NVARCHAR(n)` on SQL Server, because SQL Server cannot index
`NVARCHAR(MAX)`. Keyed text therefore has a length ceiling there that it does not have on
SQLite or PostgreSQL, which is why a path length limit exists at all. See
[`backends.md`](backends.md).

## Design decisions worth remembering

Each of these is documented at its call site. They are listed here as rationale, stated on
their own terms rather than as a comparison.

### Errors carry a code a client can branch on

An error is useless if a caller cannot tell a missing file from a bad argument from a
crash, so every failure carries one of the 14 `ERR_*` codes and an HTTP status that
suggests the right remedy:

* a missing file is `ERR_NOT_FOUND` and **404**, because an absent file is not a server
  failure,
* an edit that finds no match or an ambiguous match is **422**: the request was well
  formed, the content was not,
* an unsupported extraction format is `ERR_NOT_SUPPORTED` and **501**: asking to extract
  an `.mp3` is a missing capability, not a malformed request,
* a spent quota is **429**, a missing read before write is **428**, a duplicate project is
  **409**, because those three need three different remedies.

### One implementation per operation

`core::fs_ops` is the only place an operation is written; the MCP tool layer and the REST
data plane are thin adapters over it. This is not tidiness: when the REST route had its own
delete, it bypassed trash, audit and the `recursive` flag, so the REST door was the
destructive one. The V4A patch engine lives there for the same reason.

Consequences that are easy to get wrong if the rule is broken: an upload is quota charged
and audited like any other write; `git.remote_clone` charges the whole import up front, so
a repository that does not fit leaves the volume untouched; `git.checkout_file` is charged,
because restoring from history is a write.

### Git had to actually work

Smart HTTP is unforgiving, and three things are required rather than optional:
`upload-pack` must advertise `multi_ack_detailed`; a pack must be built from a revwalk fed
to `insert_walk`, because `insert_recursive` omits a commit's ancestry and produces an
incomplete pack for any repository with more than one commit; and the `receive-pack` report
must be band 1 framed once the client negotiated `side-band-64k`, or git aborts with
`bad band #117` after the push has already landed. The `unpack ok` line the protocol
requires is sent. Routes enforce project membership, not merely that the repo exists.
`max_pack_size_mb` is enforced.

Verified end to end: clone, commit, push, reclone.

### Document as a service (2026-09-16)

An external converter turns an uploaded document into its Markdown companion, driven by a
per call flag rather than by a hook on every write: the caller decides, because converting
on every eligible write would fire on edits and on internally generated files and would
make every write pay a multi second latency.

**The CLI converter is contained by construction, not by trust.** It is an arbitrary third
party binary, so every call gets a fresh `TempDir`; the input is staged inside it under a
sanitized single segment name keeping its extension; the child runs with that directory as
its working directory and is handed a **relative** `./name.ext`, so a tool that writes
beside its input (docling creates `name_docling/`) writes inside the sandbox; `TMPDIR`,
`TMP` and `TEMP` point there too, while `env_clear` is deliberately NOT used because a
converter legitimately needs `HOME`, `PATH` and its model cache; the command is an argv
**list**, never a shell string, so there is no redirection, no `&&` and no glob; only
stdout is read, and stderr is captured, capped at 8 KiB of its tail and surfaced only on
failure, because a progress bar must never reach a log line; on timeout the child is
killed and reaped **before** the directory is removed, so no process is left writing into
a directory being deleted. This is not a hard OS sandbox: a converter writing to an
absolute path or to `$HOME` escapes it, and real containment is the operator's call
through argv[0] (`sandbox-exec`, `bwrap`, `docker run`), which is precisely why the
command is a list.

**An upload is never rolled back when the conversion fails.** Eligibility, configuration
and size are all checked **before the first byte is written**, so a flag set on an
ineligible file stores nothing at all. Once the source bytes are committed the rule
inverts: a converter that crashes does not fail the call and does not delete the file. The
response carries `documentation.error` instead. Deleting a user's just uploaded document
because a third party binary segfaulted is worse than returning it without its companion,
and `fs.documentize` exists as the retry surface. The same reasoning gives the multipart
upload its all or nothing pre-pass: every file's eligibility is validated through the
engine's own gate before any of them is written, so a mixed batch fails cleanly rather
than half way.

The companion path is deliberately the one `fs.extract_text` already uses
(`report.pdf` -> `report.md`), so a doc service companion is served as a cache hit by the
built-in extractor at no extra cost, and the two features can never produce two files for
one document.

### Smaller ones

* `auth.jwt.algorithms` is honoured, and the HMAC family is refused on purpose: an `HS*`
  algorithm with a public key file would let anyone holding that key mint tokens.
* There is no custom libgit2 ODB backend, which `git2` cannot express from safe Rust. The
  blob store is the source of truth and is synced around libgit2 calls; stored bytes are
  identical.
* An unescaped `LIKE` prefix in subtree read, subtree delete and rename made `a_b` match
  its sibling `axb`, so a subtree delete could remove unrelated rows. All three route
  through one `descendant_pattern` helper that escapes `\`, `%`, `_` and, on SQL Server,
  `[`.
* A conflict is a SUCCESS, not an error. The pull's only conflict path used to be
  `ToolError::internal("the merge left unresolved conflicts that 'ours'/'theirs' cannot
  represent")`, so there was no conflict response to copy anywhere in the tree (DRIFT-001).
  `git.merge` introduces it as new code: `Ok` carrying `status: "conflict"`, both sides'
  content per file, and the completion pair named in the response, with nothing applied to
  the volume. The pull keeps its own error until its rework.
* Every combine response is a Rust type in `git/merge.rs`, never a hand built `json!` per
  call site: the field names of the conflict response drifted four times between
  requirements and tests before they were pinned, so a single serializer turns that class
  of drift into a compile error. `conflicting_paths`, `resolve_with`, `step`, `target_ref`,
  `present`, `size`, `commit_sha` and `parents` are emitted by no tool.
* A refused atomic apply gives the write quota back (`merge::charge_and_apply`, 2026-09-22).
  The charge has to come first so an over-quota operation is rejected before a byte moves,
  but pass 1 of the apply pre-checks every target, so a refusal there provably wrote
  nothing and must not consume quota: charge, apply, refund on error. `SafetyManager::
  refund_write` is clamped at zero and is sound ONLY behind that pre-check.
* `git.merge_resolve` writes every resolved path, including the ones whose resolved bytes
  already equal the tip's (a `ours` strategy on a file only the other side changed), so
  `files_changed` counts every file the resolution settled rather than only the volume
  delta. The caller stated the final content of those files; the volume is made to match
  it explicitly instead of trusted to already.
* The `git_operations` row of a paused merge pins the SOURCE commit sha in `onto_sha`, not
  the tip (which `original_tip_sha` already holds). The resolve happens in a later call,
  possibly after a restart and after the source branch moved, and it must finish the merge
  that was actually reported.
* `fs.move` with `overwrite: true` replaces the destination and GCs what it referenced; the
  flag used to be dead code.
* Listing a file is `ERR_INVALID_ARGUMENT`, because inventing an empty directory hides a
  caller bug.
* An invalid `fs.grep` regex is `ERR_INVALID_ARGUMENT`, not an escaped exception.
* `is_owner` and `is_admin` are computed caselessly, matching the checks that authorize.
* Generated `.docx` keeps numbered list markers and renders fenced code as monospaced
  paragraphs.
* A cherry-pick reports `already_present` for BOTH ways a change can already be in the
  branch: the commit is an ancestor (by sha) and replaying it yields the tree the branch
  already has (by content). The story's own test bodies spelled those two as
  `already_applied` and `empty` with a `message` key; the specification's response table
  fixes the key set at `status`, `new_sha`, `source_sha` and the status set at `committed`,
  `conflict`, `already_present`, and the table wins. Both cases mean the same thing to a
  caller: no commit was created and none should be.
* Resuming a conflicted `git.stash_apply` / `git.stash_pop` through `git.merge_resolve`
  creates NO commit and moves NO ref (DRIFT 2026-09-22, closed 2026-09-23). Both stash op
  types report `continue_with() == "git.merge_resolve"` because a stash application IS a
  three way merge against HEAD, but that speaks to the ALGORITHM, not to committing the
  result: a stash holds uncommitted work. The completion path therefore branches on
  `row.op_type`, the value `require_merge_operation` already validated, so nothing new is
  threaded through the pause and a restart mid conflict still resolves correctly. The
  response stays `merge::ResolveResponse`: `status` is the `applied` the non conflicting
  stash tools return, and `merge_commit` is null rather than a fabricated sha, which the
  field already allowed since the partial response uses it. A resolved POP drops its entry
  there, after every resolved byte has landed, which is the same charge, apply, drop order
  the clean pop uses; the conflicted pop deliberately deferred that delete (E2E-NEW-463).
