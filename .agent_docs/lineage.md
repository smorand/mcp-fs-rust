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
| `TOOL_CONTRACT.txt` | human readable reference for the 55 tools, their parameters and return shapes |
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
* `fs.move` with `overwrite: true` replaces the destination and GCs what it referenced; the
  flag used to be dead code.
* Listing a file is `ERR_INVALID_ARGUMENT`, because inventing an empty directory hides a
  caller bug.
* An invalid `fs.grep` regex is `ERR_INVALID_ARGUMENT`, not an escaped exception.
* `is_owner` and `is_admin` are computed caselessly, matching the checks that authorize.
* Generated `.docx` keeps numbered list markers and renders fenced code as monospaced
  paragraphs.
