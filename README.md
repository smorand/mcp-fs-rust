# mcp-fs (Rust)

A **streamable-HTTP MCP server** exposing a **simulated multi-project filesystem**:
110 `fs.*` / `admin.*` / `git.*` tools, a parallel REST data plane at `/api/fs` with
OpenAPI docs, and an optional Git HTTP smart server. Runs from a single binary with
**no external service by default**.

Server state (metadata tree, ACL, git index, OAuth tokens) = **SQLite** by default, or
**PostgreSQL** or **SQL Server** chosen per store in the config. File bytes = **local
filesystem** (default) or **MinIO/S3**, content-addressed by sha256. Auth = verified
**RS256 bearer JWT**.

This began as a Rust port of a C# implementation. **That is history**: the port constraint
is gone, the two projects have separate lifecycles and deployment targets, and they are
diverging by design (the C# has no PostgreSQL support, because that is not its target).
The tool surface is still a contract and is still pinned by tests. See
[`.agent_docs/lineage.md`](.agent_docs/lineage.md) for the lineage and the design
decisions worth knowing.

## Cargo features

A default build is SQLite only and carries no database driver beyond bundled `rusqlite`.

| Feature | Adds | Driver |
|---|---|---|
| (default) | SQLite | `rusqlite`, bundled |
| `postgres` | PostgreSQL | `sqlx` |
| `sqlserver` | SQL Server | `tiberius-ng` + `bb8` |
| `all-backends` | both | |

`tiberius-ng` is a young, low traffic fork, chosen because `sqlx` has no MSSQL driver and
the alternative (`odbc-api`) would require a system driver manager on every host. It is
pinned, optional, and confined to one file. The full comparison and the exit plan are in
[`.agent_docs/backends.md`](.agent_docs/backends.md#decision-which-sql-server-driver-2026-09-15).

```bash
cargo build --release --features postgres
cargo build --release --features all-backends
```

SQL Server needs a second driver because **sqlx removed its MSSQL support in 0.7** and the
rewrite has never shipped. Both are optional so a default build carries neither the
dependency nor its TLS stack.

## Quickstart

```bash
git clone git@github.com:smorand/mcp-fs-rust.git && cd mcp-fs-rust
./run.sh            # generates keys, bootstraps config/local.yaml, builds, serves :5002
```

In another terminal:

```bash
TOKEN=$(./target/release/mcp-fs token you@example.com --key .keys/jwt.key)

curl http://127.0.0.1:5002/health

# list the tools
curl -s -X POST http://127.0.0.1:5002/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "X-Forwarded-Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# call one
curl -s -X POST http://127.0.0.1:5002/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "X-Forwarded-Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call",
       "params":{"name":"admin.list_projects","arguments":{}}}'
```

Interactive API docs: <http://127.0.0.1:5002/api/docs> (spec at `/api/swagger.json`).

## CLI

```
mcp-fs serve [--config PATH]      run the server
mcp-fs keys  [--dir DIR]          generate an RS256 keypair (default .keys)
mcp-fs token <email> [--key PATH] [--ttl SECONDS]
mcp-fs migrate --from A.yaml --to B.yaml   copy relational state between backends
mcp-fs version
```

Config resolution, highest priority first: `--config`, else `$MCP_FS_CONFIG`, else
`~/.config/mcp-fs/config.yaml` when it exists, else
`${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}.yaml`. The first two are used
as given; the last two are probed, and when neither exists the error lists every path
tried.

### Switching backend with data in place

`migrate` is how you move an existing deployment onto PostgreSQL or SQL Server. It copies
every row the server owns, from the engines one config names to the engines the other
names.

```bash
mcp-fs migrate --from config/sqlite.yaml --to config/postgres.yaml
```

**Run it with the server stopped**: it takes no locks against a live writer, so a
concurrent write would be missed. The copy is row for row rather than a replay through the
store API, so every `mtime` and `ctime` is preserved exactly.

Two things are deliberately not moved. **Blob bytes** stay put, because the blob store is
configured separately and a relational change does not affect it; every `sha256` is
instead checked against the destination blob store and reported when missing. **OAuth
tokens** are skipped, because they are session state encrypted with `MCPFS_TOKEN_KEY` and
copying ciphertext to a deployment with a different key would produce rows that never
decrypt. A device flow re establishes them.

## Configuration

`config/local.yaml` is your personal working copy and is **gitignored**. Two templates
are tracked, copy one:

```bash
cp config/local.yaml.template config/local.yaml   # SQLite + local blobs, zero services
cp config/minio.yaml.template config/local.yaml   # SQLite + MinIO/S3 blobs
```

Each of the four relational stores is configured independently under `infra`, so you can
move them one at a time:

```yaml
infra:
  meta:                          # the per volume file tree
    backend: postgres            # sqlite | postgres | sqlserver
    dsn: "${MCPFS_META_DSN}"     # secret, required for a server engine
    schema: mcpfs                # postgres only
    pool: { max_connections: 20, acquire_timeout_secs: 10 }
  admin: { backend: postgres, dsn: "${MCPFS_META_DSN}" }   # projects and ACL
  git:   { backend: postgres, dsn: "${MCPFS_META_DSN}" }   # git index
  oauth: { backend: sqlite }                               # OAuth tokens
```

Pointing several stores at one dsn is the expected setup: table names do not collide and
one connection pool is shared. Misconfiguration fails at boot, not on first use: a missing
dsn, a dsn set on a `sqlite` store, an unknown backend name, or a backend whose cargo
feature was not compiled in are each rejected with the offending key named.

`${VAR}` and `${VAR:-default}` are expanded from the environment before the YAML is
parsed, so **secrets never live in a committed file**. Put them in a gitignored `.env`
(template: `.env.example`), which `run.sh` sources:

| Variable | Purpose |
|---|---|
| `MCPFS_MINIO_SECRET_KEY` | S3/MinIO secret key |
| `MCPFS_GITHUB_CLIENT_SECRET` | GitHub App secret for the `git.auth` device flow |
| `MCPFS_TOKEN_KEY` | 32 byte base64 key; when set, OAuth tokens are persisted encrypted (AES-256-GCM). Unset means in-memory only |
| any name you pick | a store `dsn`, for example `${MCPFS_META_DSN}` |

A dsn normally carries a password, so it is treated as a secret: the type redacts itself in
both `Debug` and `Display`, and a test asserts it never reaches a log line, a boot banner
or an error message.

Full schema in [`.agent_docs/config.md`](.agent_docs/config.md).

## What lives where

| Data | Backend | Location under the SQLite default |
|---|---|---|
| File tree metadata | `infra.meta` | `state/volumes/{project}.db` |
| File bytes | local fs or S3 | `state/blobs/{bucket}/{sha[..2]}/{sha}` or bucket `mcpfs-{project}` |
| Projects and ACL | `infra.admin` | `state/admin.db` |
| Git objects and refs | blob store + `infra.git` | key `git:{sha}`, `state/git/{project}.db` |
| OAuth tokens (opt-in) | `infra.oauth`, encrypted | `state/oauth.db` |
| Git working directories | always on disk | `state/git-repos/{project}/` |

`state/` is the whole database under the default. Back it up, and note that in MinIO mode
the bytes live in the bucket while the metadata stays in `state/`: a project needs **both
halves**.

On PostgreSQL or SQL Server one database holds every volume, discriminated by a
`volume_id` column, since creating a database per project on the fly is not viable. Blob
bytes never move into the relational database, and `state/git-repos/` stays on disk
because libgit2 needs a real working directory.

## Document as a service

An upload can carry its own Markdown companion. Set `trigger_documentation_service` on a
byte upload and the file is stored **and** converted by an external, stateless document
service, the result landing beside the source at the exact path `fs.extract_text` reads
(`toto.pptx` -> `toto.md`), so the built-in extractor then serves it as a cache hit.
PowerPoint, Word, PDF, audio and video only. The feature is **off by default**:

```yaml
doc_service:
  enabled: true
  mode: cli                      # cli | api
  extensions: []                 # empty = the built-in eligible set
  max_input_bytes: 536870912     # 512 MiB
  cli:
    command: ["doc-convert", "--quiet", "-o", "{outdir}", "{document}"]
    timeout_secs: 900
  api:
    url: "https://converter.internal/convert"
    auth_header: Authorization   # header NAME, not necessarily a bearer
    auth_token: "Bearer ${MCPFS_DOC_SERVICE_TOKEN}"   # secret, sent verbatim
    file_field: file             # name of the multipart part carrying the document
    response_field: ""           # empty = the response body IS the markdown
    timeout_secs: 900
```

In `cli` mode the converter is an **argv list, never a shell string**, and every call runs
in a fresh temporary directory: the input is staged inside it under a sanitized name, the
child's working directory and `TMPDIR` point there, `{document}` expands to a relative
`./name.ext`, and the directory is removed on every exit path including a timeout, which
kills and reaps the child first. That contains a tool which writes beside its input; it is
not a hard OS sandbox, so wrap argv[0] in `sandbox-exec`, `bwrap` or `docker run` if you
need one. In `api` mode the request is `POST {url}`, `multipart/form-data`, one part named
`file`; `scripts/doc_service_fake.py` implements exactly that contract for local
development (`--bundle` answers the JSON bundle below).

The converter may return a **bundle** rather than bare Markdown. In `cli` mode an optional
`{outdir}` placeholder (the default command uses it) names a fresh empty directory inside
the sandbox: the server reads `{outdir}/document.md` as the Markdown, every regular file
below it (symlinks not followed, total capped by `max_input_bytes`), and
`{outdir}/artifacts.json` as the manifest; without `{outdir}` stdout is the Markdown, as
before. In `api` mode an `application/json` answer (with `response_field` empty) is
`{"markdown": "...", "files": {"figures/f1.png": "<base64>"}, "manifest": {...}}`. Pipe
tables of the Markdown are kept as table artifacts (`fs.list_tables`, `CSV quality:
approximate`); each line holding one `![alt](target)` whose target is a bundle file in
`png`, `jpeg`, `gif`, `webp`, `tiff` or `bmp` is kept as an image artifact, captioned and
paged by the manifest (`fs.list_images`, `fs.get_image`); every conversion result carries
an `artifacts_report`.

The manifest `{outdir}/artifacts.json` (or the bundle's `manifest`) is optional:
`{"pictures": {"figures/f1.png": {"caption": "...", "caption_failed": false, "page": 3}}}`;
a missing field means an empty caption, no failure marker, no page.

The doc service surfaces, two tools and two routes:

| Surface | Purpose |
|---|---|
| `fs.write_bytes` | write raw bytes (base64), with the `trigger_documentation_service` flag |
| `fs.documentize` | convert a file already stored in the volume, the retry surface |
| `POST /api/fs/{mount_id}/write-bytes` | the JSON mirror of `fs.write_bytes` |
| `POST /api/fs/{mount_id}/documentize` | the JSON mirror of `fs.documentize` |

**Document artifacts.** Each conversion keeps the document's images and tables beside its
Markdown sibling, readable by five read only tools and their REST twins, one implementation,
byte identical payloads:

| Tool | Route | Returns |
|---|---|---|
| `fs.list_images` | `GET /api/fs/{mount_id}/images?path=&marker=` | `images[{id,caption,page}]`, `marker` |
| `fs.get_image` | `GET .../image?path=&id=` | `id`, `format`, `caption`, `page`, `base64` |
| `fs.list_tables` | `GET .../tables?path=&marker=` | `tables[{id,caption,rows,columns,csv_quality}]`, `marker` |
| `fs.get_table` | `GET .../table?path=&id=&format=` | `id`, `format` (`markdown` or `csv`), `content` |
| `fs.get_document_view` | `GET .../document-view?path=&captions=&table_mode=` | `path`, `text` with image lines and tables (`markdown`, `csv-reference`, `both`) |

Lists page by 100; the continuation marker is valid for its project, list and document,
for any member. A document changed since its conversion has no artifacts until it is
converted again. The set is charged to the session write quota together with the sibling,
before anything is written; artifacts are not files and never show in listings or exports.
The real `doc-convert` writes no `artifacts.json` yet (no pages, no caption failure marker),
emits PNG only and deduplicates identical figures.

The existing multipart `POST /api/fs/{mount_id}/upload` takes the same flag as a form field
(`"true"` or `"1"`), applied to every file of the form, and answers with a `documentation`
array holding one entry per file.

Two rules worth knowing, both deliberate. **Eligibility is checked before the first byte is
written**, so a flag set on a `.txt`, or on a mixed batch, fails with `ERR_NOT_SUPPORTED`
and nothing is stored. **A failed conversion never rolls the upload back**: the file stays,
the response carries `documentation.error`, and `fs.documentize` retries. Conversion is
synchronous and slow (tens of seconds for a small PDF, minutes for a video), so set your
client's read timeout accordingly.

## Comparing two files

`fs.diff_files` (and `GET /api/fs/diff?mount_id_a=&path_a=&mount_id_b=&path_b=`) compares two
text files, in one project or two, and returns a unified diff with 3 lines of context by default; the optional `context_lines`
(a whole number from 0 to 10000000, as a number or as digit text) chooses another number of
unchanged lines around each change (2026-10-20, SPEC-0046). Anything else is refused with
`ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000`. The
caller must be a member of both projects. Each file is capped at `safety.max_diff_file_bytes`
(5 MiB by default); a binary or non UTF-8 file is refused.

```bash
{"name": "fs.diff_files", "arguments": {"mount_id_a": "alpha", "path_a": "/a.txt", "mount_id_b": "beta", "path_b": "/b.txt"}}
# -> {"identical": false, "diff": "--- alpha:/a.txt\n+++ beta:/b.txt\n@@ -1 +1 @@\n-a\n+b\n"}
```

## Extracting archives in place

`fs.extract_archive` (and `POST /api/fs/{mount_id}/extract-archive`) unpacks an archive that
is already stored in the project, with no download and re upload through the client:

```bash
# MCP: extract /uploads/report.tar.gz into /uploads/report
{"name": "fs.extract_archive", "arguments": {"mount_id": "proj", "path": "/uploads/report.tar.gz"}}
# -> {"destination": "/uploads/report", "files_written": 2, "dirs_created": 1, "bytes_written": 10}
```

Formats, by extension: `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2`,
`.tar.xz`/`.txz`, all decoded in pure Rust. `destination` defaults to the archive path minus
its suffix; `overwrite` defaults to `false`. **Passwords are a plain retry**: an encrypted zip
or 7z answered without (or with a wrong) `password` fails with `ERR_PASSWORD_REQUIRED` (HTTP
428) and the caller sends the same call again with `password`; nothing is stored between
calls and the password is never logged. **All or nothing**: the whole archive is decoded and
checked before the first write, so a symlink or device entry, an entry escaping the
destination (zip slip), a collision without `overwrite`, or a declared total above the write
quota rejects the call with nothing written. A zip whose entry headers disagree with its central directory
(name, sizes, checksum, compression, encryption, modification time), that holds two entries of
the same name, or whose entries share bytes is refused the same way (`ERR_INVALID_ARGUMENT`,
`archive is corrupt or not a valid zip file: header inconsistency detected: ...`), after the
password answer for an encrypted one.

## Project quota enforcement

A platform admin caps a project's storage with `admin.set_project_quota` (`max_mb`, or
none for unlimited). Usage is the sum of the declared sizes of the project's files, each path
counted once even when content is shared, trashed files included; export zips, version
control history and conversion artifacts do not count. Every operation that grows the
project (writes, edits, appends, uploads, copies, archive extraction, conversion siblings,
clone, pull, merge, switch, cherry pick, stash pop, file restore) is checked on its net
change and refused, with nothing written, by `ERR_PROJECT_QUOTA_EXCEEDED` (HTTP 507):

```
project storage quota of 1 MB exceeded: 1040000 bytes used, 40000 bytes requested
```

The project limit is checked before the per session write quota, so a refusal spends no
session budget. A shrinking or same size change always passes; reaching the limit exactly is
allowed. Moves, deletes to the trash, trash restores, version control aborts and pushes are
never checked. A delete keeps the bytes counted until the trash is purged permanently. A
limit lowered below the current usage is accepted and removes nothing; every growing
operation is refused until usage drops. Concurrent operations on one project are admitted
one at a time, so they never leave usage above the limit.

`admin.get_project_quota` (also `GET /api/admin/projects/{project_id}/quota`) returns
`{project_id, max_mb, used_bytes}`, `max_mb` null when unlimited; it answers a platform
admin or the project owner only.

`admin.get_project_disk_usage` (also `GET /api/admin/projects/{project_id}/disk-usage?limit=`)
lists a project's files from the largest to the smallest, so an owner refused by the limit
can find what to delete. It answers the same two callers with the same refusals as the quota
read, and returns `{project_id, max_mb, used_bytes, file_count, truncated, entries}`; each
entry is `{path, size, state, original_path}` with `state` `Active` or `In trash` (a trashed
file also shows the path it had before its delete). `limit` is a whole number from 1 to 500
(default 100); anything else is refused with `limit must be between 1 and 500`. Equal sizes
are ordered by path in character code order. The figures come from one moment: the read takes
the project's admission lock, so it never shows half of a write.

## Security model

- **Authentication**: RS256 JWT, signature, issuer and expiry verified (30s clock skew,
  matching the reference implementation). Read from `X-Forwarded-Authorization`, then
  `Authorization`. Basic auth is accepted with the token as the password, for git CLI use.
- **Authorization**: a project has an owner and members. Every `fs.*` and `git.*` tool
  requires membership.
- **Separation of duties**: a *platform admin* (`auth.admins`) manages projects and
  membership and can list everything, but does **not** get implicit access to a
  project's files. An admin who needs the files adds itself as a member.
- **Safety rails**: path normalization (NUL rejected, traversal contained), must-read
  before-write, per-session write quota, capped audit log, soft delete to
  `.mcp_trash/` unless hard delete is enabled.
- Secrets are read from the environment and never logged.

## Build and test

```bash
make build                       # ./build.sh -> cargo build --release
make test                        # ./test.sh -> cargo test --workspace
make lint                        # cargo clippy --all-targets --all-features -- -D warnings
make check                       # format-check + lint + typecheck + security + test
```

`make` wraps the existing scripts unchanged (`make help` lists every target); the scripts
themselves still work exactly as before for anyone who prefers them directly:

```bash
./build.sh                       # cargo build --release
./test.sh                        # cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
```

The suite is the quality gate and must be green before any commit. Use `--all-features` on
clippy, otherwise the two optional drivers are never compiled and their warnings never
surface.

The default run needs no database and no Docker. To exercise PostgreSQL and SQL Server,
bring the services up and pass their dsn; the cases skip themselves when the variables are
absent:

```bash
docker compose -f docker-compose.test.yml up -d

MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs \
MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' \
  cargo test --workspace --all-features

docker compose -f docker-compose.test.yml down -v
```

One conformance suite runs the same assertions against every engine, which is what proves a
store behaves identically on all three. See
[`.agent_docs/testing.md`](.agent_docs/testing.md).

## Interactive CLI agent

`crates/agent` builds an `agent` binary that drives the 110 tools through an LLM. It is a
**client**, so it exercises the real MCP wire protocol the way any other client would.

```bash
mkdir -p .agent_keys
./target/release/mcp-fs token you@example.com \
    --key .keys/jwt.key > .agent_keys/you                   # one raw JWT per file
export IBM_ICA_MODEL_KEY=...                                # or put it in .env
./agent.sh --user you
```

`agent.sh` starts the server itself when nothing answers on the configured endpoint, logging
to `mcp_<datetime>.log`, and stops it again when the agent exits. A server that was already
running is left alone. A watchdog keyed on the agent's pid covers the case where the wrapper
is killed outright.

Configured by `config/agent_test.yaml`: MCP endpoint, token directory, and any OpenAI
compatible chat endpoint. `--conversation ID` resumes a transcript, `/help` lists the
commands. It also works non interactively, which makes it scriptable:

```bash
printf 'Liste mes projets.\nexit\n' | ./target/release/agent --user you
```

`.agent_keys/` (bearer tokens) and `.agent_history/` (transcripts) are gitignored. See
[`.agent_docs/agent.md`](.agent_docs/agent.md) for the terminal invariants it relies on.

The line editor is verified on a real pty, because terminal geometry bugs pass every unit
test: a wrong prompt width is a mistake at the call site, not in the width function.

```bash
cargo build -p agent -p mcp-fs && python3 scripts/pty_check.py
```

## The tool contract is frozen

The 94 tool names, descriptions and `inputSchema` values are a client and an LLM facing
contract, so they are snapshotted in `tool-contract-golden.json` and compared on every test
run, serialized form included, which means even a reordered schema key fails the build.
`TOOL_CONTRACT.txt` is the human readable companion.

Changing the contract is deliberate, never a hand edit:

```bash
MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs-core --lib tool_contract_golden_is_current
```

Then review the diff: a description edit is one line, and 57 changed tools means something
went wrong.

## Design decisions worth knowing

Stated on their own terms; the full record is in
[`.agent_docs/lineage.md`](.agent_docs/lineage.md), and each is documented at its call site.

**Errors are usable.** Every failure carries one of the 16 `ERR_*` codes, and the REST
status suggests a remedy rather than a generic 400: a missing file is 404, a spent quota
429, a missing read before write or a missing archive password 428, a duplicate project 409, an unsupported extraction
format 501, a project over its storage limit 507, and an edit that matched nothing or matched ambiguously 422.

**A real `git clone` and `git push` work.** Smart HTTP is unforgiving: `upload-pack`
advertises `multi_ack_detailed`, the pack is built from a revwalk so a commit's ancestry is
included (otherwise any repository with more than one commit is incomplete), and the
`receive-pack` report is side-band framed once the client negotiated it, or git aborts after
the push has already landed. `unpack ok` is sent, project membership is enforced on every
git route, `max_pack_size_mb` is enforced, and pushed objects are really indexed.
Verified end to end: clone, commit, push, reclone.

**One implementation per operation.** `core::fs_ops` is the only place an operation is
written; the MCP tool layer and the REST plane are thin adapters over it, including the V4A
patch engine. This is why the REST `delete` honours trash, `allow_hard_delete` and
`recursive`, why `upload` is quota charged and audited like any other write, and why
`git.remote_clone` charges the whole import up front so a repository that does not fit
leaves the volume untouched instead of half populated. `git.checkout_file` is charged too:
restoring from history is a write.

**Correctness.** `auth.jwt.algorithms` is honoured, with unsupported names logged at
startup and the HMAC family refused on purpose (an `HS*` algorithm with a public key file
would let anyone holding that key mint tokens). Listing a file is a 400 rather than an
invented empty listing. `fs.tree` at exactly the node cap returns every node. Symbol
references come back ordered by line. An invalid `fs.grep` regex is a stable 400. A
subtree `LIKE` pattern is escaped, so a file named `a_b` no longer matches its sibling
`axb` and a subtree delete cannot remove unrelated rows. No custom libgit2 ODB backend
(`git2` cannot express one from safe Rust): the blob store is the source of truth, with
identical stored bytes.

## Not supported

Audio and video extraction (needs a speech model) and legacy binary Office formats
(`.doc`, `.xls`, `.ppt`). `object_format: sha256` is accepted and ignored: the bundled
libgit2 is sha1 only. RAR archives are not extracted (no pure Rust decoder; out of scope
by decision), nor multi volume archives (`.7z.001`, `.zip.001`).

On the relational side: there is no online migration (`mcp-fs migrate` is offline, run with
the server stopped) and no automated downgrade, since a database written by the current
code carries a `volume_id` column older code does not know about. Blob bytes are never
stored in the relational database.

## Documentation

`AGENTS.md` is the compact index. Details live in [`.agent_docs/`](.agent_docs/):
architecture, tools, api, git, config, backends, testing, lineage, agent.

## License

MIT
