# Lineage: the C# origin, and why parity is retired

This server began as a strict 1:1 port of a C# implementation (`../mcp-fs-csharp`).
**That constraint is retired.** The project now has its own lifecycle: a change is judged
on whether it is correct and useful here, not on whether the C# does the same thing.

Two things follow, and both matter when reading older code or comments:

* **A comment or doc that justifies a behaviour by "the C# does it this way" is stale.**
  The behaviour may still be right, but the reason no longer holds on its own. Do not
  preserve a shape solely because the reference had it.
* **Nothing requires reading `../mcp-fs-csharp` any more.** `TOOL_CONTRACT.txt` remains
  the captured tool surface and is still the authority on tool names, parameters and
  descriptions, because those are a client contract regardless of where they came from.

## What parity bought, and what is kept

The port produced a large, precise regression corpus, and that value survives its
retirement. The MCP wire framing, the 55 tool schemas, the `ERR_*` codes, the REST routes
and the git smart protocol were all pinned against a live reference rather than inferred,
and the tests that pin them still run.

| Kept | Why it stays |
|---|---|
| The 55 tool names, parameters and descriptions | a client and an LLM contract; `TOOL_CONTRACT.txt` is authoritative |
| Tool schema equality tests against `parity-golden.json` | the cheapest guard against accidental schema drift, and still green |
| The 14 `ERR_*` codes and their HTTP mapping | a client branches on these |
| The MCP wire framing and JSON-RPC behaviour | captured live with `curl`, encoded in `app.rs` assertions |
| The blob layouts, `{root}/{sha[..2]}/{sha}` and `git:{sha}` | an on disk contract that existing volumes rely on |
| `crates/parity-harness` | kept runnable and green, see below |

## What changed once parity was retired

### Schema divergences taken deliberately

Both were blocked by parity and are the reason it had to go. Neither is reversible by an
older reader of the same database.

**1. `volume_id` on `nodes`, `blob_refs` and the three `git_*` tables.**
SQLite gets one database file per volume, so a tenant column was unnecessary. PostgreSQL
and SQL Server cannot create a database per project on the fly, so one database holds
every volume and the tenant became the first column of the primary key
(`PRIMARY KEY (volume_id, path)`). Under SQLite the column is still written but constant
per file, so that deployment is unchanged in layout and behaviour.

*Consequence:* a database written by the current code carries a column the pre port code
does not know about. SQLite tolerates the extra column on read, but there is no automated
downgrade. `mcp-fs migrate` covers the forward direction only.

**2. Dialect binary typing for the OAuth token.**
`token_enc` was `BLOB`. It is now `BLOB`, `BYTEA` or `VARBINARY(MAX)` depending on the
engine. The encryption is untouched (AES-256-GCM keyed by `MCPFS_TOKEN_KEY`); only the
column type is rendered per dialect.

A third change is worth listing beside them because it is also schema shaped:
`ColumnType::TextKey(n)` renders as bounded `NVARCHAR(n)` on SQL Server, because SQL
Server cannot index `NVARCHAR(MAX)`. Keyed text therefore has a length limit there that
it does not have elsewhere.

### The bug that parity would have required preserving

Subtree read, subtree delete and rename built `path LIKE '{prefix}/%'` with an unescaped
prefix. A file named `a_b` matched its sibling `axb`, so a subtree delete could remove
unrelated rows. Under parity the correct move would have been to reproduce it. It is
fixed instead: all three sites route through one `descendant_pattern` helper that escapes
`\`, `%`, `_` and, on SQL Server, `[`.

### The differential harness

`crates/parity-harness` is **no longer a gate**, because there is no longer a reference to
be equal to. It is kept runnable and green because the corpus is a genuine 128 step
regression suite over the MCP surface, the REST plane and every error path, and deleting
it would throw away coverage that the unit tests do not replicate.

Use it as a regression check against a previous build of this server:

```bash
cargo run -p parity-harness -- capture --base http://127.0.0.1:5002 --token "$T" \
  --owner admin@example.com --out baseline.json
# make a change, restart, then
cargo run -p parity-harness -- compare --base http://127.0.0.1:5002 --token "$T" \
  --owner admin@example.com --golden baseline.json
```

`parity-golden.json` remains the committed C# baseline. It still backs the tool schema
equality tests, which is why it is not deleted, but a difference against it in the
harness is now information rather than a failure.

## The divergence record

The list below is why parity was worth abandoning: each entry is a place where copying
the reference would have copied a defect. It is kept as the rationale behind current
behaviour. Each is also documented at its call site.

### Errors carry a usable code and a usable status

| Case | Reference | Here | Why |
|---|---|---|---|
| missing file, bad argument, read a directory, traversal | `"An error occurred invoking 'fs.read'."`, **no code** | `ERR_NOT_FOUND` / `ERR_INVALID_ARGUMENT` | the reference storage layer raised a bare `IOException`, so the SDK emitted a generic sentence and a client could not tell a missing file from a bad argument from a crash |
| missing file over REST | HTTP 500 | HTTP 404 | an absent file is not a server failure |
| edit with no match, ambiguous match | HTTP 400 | HTTP 422 | the request was well formed and the content was not; the reference defaulted every unmapped code to 400 |
| unsupported extraction format | HTTP 400, `ERR_INVALID_ARGUMENT` | HTTP 501, `ERR_NOT_SUPPORTED` | asking to extract an `.mp3` is a missing capability, not a malformed request |
| quota, read guard, duplicate project | HTTP 400 for all three | 429, 428, 409 | a spent budget, a missing precondition and a name conflict need three different remedies |

### Behaviour that was simply incorrect

| Case | Reference | Here |
|---|---|---|
| list a file | HTTP 200 with `{"entries": []}` | `ERR_INVALID_ARGUMENT`: inventing an empty directory hides a caller bug |
| REST delete | `{"deleted": true}`, no trash, no audit, recursive regardless | the tool payload, honouring `recursive` and `trash`; the REST door was the destructive one because it bypassed the engine |
| upload | no quota charge, no audit | charged and audited, like every other write |
| REST mkdir | `parents` and `exist_ok` ignored, no audit | same parameters and audit as the tool |
| `git.remote_clone` | wrote every file with no charge, no audit | whole import charged up front, then audited, so a repository that does not fit leaves the volume untouched |
| `git.checkout_file` | audited, not charged | charged: restoring from history is a write |
| `fs.move` with `overwrite: true` | always `ERR_NO_CLOBBER` | replaces the destination, GCing what it referenced; the flag was dead code |
| `fs.tree` at exactly the node cap | one node short, flagged `truncated` | complete, `truncated: false` |
| `find_refs` ordering | `[3, 2]`, a traversal artifact | ascending by line |
| `swagger.json` | `/api/fs/roots` missing | documented |

### Git, where the reference could not actually clone or push

Three defects, each verified against both servers, made real git use impossible:
`upload-pack` did not advertise `multi_ack_detailed`, which smart HTTP requires; the pack
was built with `insert_recursive`, which omits a commit's ancestry, so any repository with
more than one commit produced an incomplete pack; and the `receive-pack` report was raw
pkt-lines even when the client had negotiated `side-band-64k`, so git aborted with
`bad band #117` after the push had landed. Fixed with the detailed capability, a revwalk
fed to `insert_walk`, and band 1 framing. Verified end to end: clone, commit, push,
reclone.

Also on git: the `unpack ok` report line the protocol requires is sent; routes enforce
project membership, where the reference only checked that the repo existed, so any
verified token could read or write any project; `max_pack_size_mb` is enforced rather than
parsed and ignored; and pushed objects are really indexed instead of stubbed.

### Remaining smaller ones

* `auth.jwt.algorithms` is honoured. The reference parsed it and hardcoded RS256, so a
  configured policy was silently ignored. The HMAC family is refused on purpose: an `HS*`
  algorithm with a public key file would let anyone holding that key mint tokens.
* No custom libgit2 ODB backend, which `git2` cannot express from safe Rust. The blob
  store is the source of truth and is synced around libgit2 calls; stored bytes are
  identical.
* Generated `.docx` keeps numbered list markers and renders fenced code as monospaced
  paragraphs instead of leaking backtick lines.
* `is_owner` and `is_admin` are computed caselessly, matching the checks that authorize.
* An invalid `fs.grep` regex is `ERR_INVALID_ARGUMENT` rather than an escaped exception.
* One implementation per operation, shared by MCP and REST (`core::fs_ops`), including the
  V4A patch engine, which used to live in the tool layer and forced the REST route to
  dispatch back through the tool registry.

## Working on this codebase now

* **Judge a change on its merits here.** Correctness, the tool contract, and the tests.
* **`TOOL_CONTRACT.txt` still wins** on tool names, parameters and descriptions.
* **Do not add a "the C# does X" justification.** If a behaviour is right, say why it is
  right.
* **Keep the harness green.** When a change is a deliberate improvement over the recorded
  baseline, recapture and commit the new baseline with the change, and note it here.
