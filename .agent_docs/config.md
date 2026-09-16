# Configuration reference

One YAML file drives the server. Schema and defaults live in `config.rs` and are
identical to the reference implementation, key for key, so the same file drives
both. Every section and every key is optional: an absent section takes its
defaults, and unknown keys are ignored (there is no strict mode).

## `server`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `host` | string | `0.0.0.0` | bind address |
| `port` | int | `5002` | bind port |
| `mcp_path` | string | `/mcp` | path of the MCP endpoint; must start with `/`, otherwise the boot fails |

## `auth.jwt`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `public_key_path` | string | `""` | RSA public key (PEM) used to verify bearers; empty means every token is rejected |
| `header` | string | `X-Forwarded-Authorization` | header checked first; `Authorization` is the fallback |
| `algorithms` | list | `[RS256]` | parsed for parity and unused: verification is always RS256 |
| `audience` | string | null | when set, `aud` must match; when absent, `aud` is not validated |
| `issuer` | string | `web-a2a` | when set, `iss` must match |
| `username_claim` | string | `email` | claim carrying the caller identity, lowercased on read |

Expiry and `nbf` are always validated with a 30 second leeway, matching the
reference clock skew.

## `auth`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `admins` | list of strings | `[]` | platform admins, matched caselessly. Grants project and membership management, **not** file access (see `.agent_docs/tools.md`) |

## Relational stores: `infra.meta`, `infra.admin`, `infra.git`, `infra.oauth`

Four stores hold the server's relational state, each configured independently and each
accepting the same three backends. Details of the layer are in
[`backends.md`](backends.md).

| Backend | Compiled | Needs |
|---|---|---|
| `sqlite` | always | a path, no dsn |
| `postgres` | `--features postgres` | a dsn |
| `sqlserver` | `--features sqlserver` | a dsn |

Keys shared by all four sections:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string | `sqlite` | `sqlite`, `postgres` or `sqlserver`. Anything else is `ERR_INVALID_ARGUMENT` naming the accepted values |
| `dsn` | string | `""` | PostgreSQL and SQL Server only. **Secret**: inject it with `${VAR}` |
| `schema` | string | `public` | PostgreSQL only: the schema holding our tables |
| `pool.max_connections` | int | `10` | server engines only; SQLite keeps one serialized connection by design |
| `pool.acquire_timeout_secs` | int | `30` | server engines only: how long a request waits for a pooled connection |

Plus one SQLite path key per section:

| Section | Key | Default | Holds |
|---|---|---|---|
| `infra.meta` | `dir` | `state/volumes` | one db per volume, `{dir}/{project_id}.db`. Its parent is the state root, from which the git and oauth paths are derived |
| `infra.admin` | `path` | `state/admin.db` | ACL registry (`project`, `project_member`) |
| `infra.git` | (derived) | `{state_root}/git/{project_id}.db` | git index (`git_objects`, `git_refs`, `git_remotes`) |
| `infra.oauth` | (derived) | `{state_root}/oauth.db` | encrypted OAuth tokens |

`infra.git` and `infra.oauth` are new sections. They were previously hardcoded to those
derived paths, which is still exactly what an absent section means, so no existing config
changes behaviour.

### Validation at boot

All four are checked when the config loads, because a silent misconfiguration here is the
real risk. Each failure is `ERR_INVALID_ARGUMENT` naming the offending key:

* `backend` is not one of the three accepted values;
* `backend` is `postgres` or `sqlserver` and `dsn` is empty;
* `dsn` is set while `backend` is `sqlite`, which never reads a dsn. Rejected rather than
  ignored, so a dsn that does nothing cannot look like it is in use;
* `backend` names an engine this binary was not built with. The message names the cargo
  feature to rebuild with, since the failure is a build choice and not a typo.

Pointing several sections at the same dsn is supported and is the expected deployment:
the table names do not collide, and one pool is shared per distinct backend, dsn and
schema triple.

### Example

```yaml
infra:
  meta:
    backend: postgres
    dsn: "${MCPFS_META_DSN}"
    schema: mcpfs
    pool: { max_connections: 20, acquire_timeout_secs: 10 }
  admin:
    backend: postgres
    dsn: "${MCPFS_META_DSN}"
    schema: mcpfs
  git:
    backend: postgres
    dsn: "${MCPFS_META_DSN}"
  oauth:
    backend: sqlite
```

## `infra.blob`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string | `local` | `local`, or `minio` / `s3` |
| `dir` | string | `state/blobs` | local only: layout `{dir}/{bucket}/{sha[..2]}/{sha}` |
| `endpoint` | string | `""` | S3 only: endpoint URL; empty means the AWS default |
| `access_key` | string | `""` | S3 only |
| `secret_key` | string | `""` | S3 only. **Secret**: inject it with `${MCPFS_MINIO_SECRET_KEY}` |
| `bucket_prefix` | string | `mcpfs-` | bucket per volume: `{bucket_prefix}{project_id}` (also the local subdirectory name) |
| `region` | string | `us-east-1` | S3 only |

Path style addressing is always forced on S3 because MinIO requires it.


## `safety`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `write_quota_bytes` | int | `52428800` (50 MiB) | per session (`person` + project) write budget, charged before each write |
| `trash_dir` | string | `.mcp_trash` | soft delete destination inside the volume |
| `read_guard` | bool | `true` | require a read of a file in this session before editing it |
| `allow_hard_delete` | bool | `false` | when false, `fs.delete(trash=false)` is `ERR_NOT_SUPPORTED` |
| `max_read_lines` | int | `2000` | hard ceiling on `fs.read`; `limit_lines` is clamped to it |

## `api`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | mounts `/api/fs/**` plus `/api/swagger.json` and `/api/docs`. False means MCP only and all three 404 |

## `extract.ocr`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `provider` | string | `none` | `none` (images yield an empty result plus a note) or `multimodal` |
| `endpoint` | string | `""` | OpenAI compatible `chat/completions` URL |
| `model` | string | `""` | model name sent to that endpoint |
| `api_key_env` | string | `MCP_FS_OCR_KEY` | **name of the environment variable** holding the key, never the key itself |
| `prompt` | string | `Transcribe this document faithfully into Markdown.` | instruction sent with the image |

The prompt and the key are request material only: they are never logged, never
echoed in an error and never returned to the caller, because tool output flows
straight into an LLM context.

## `doc_service`

An external, stateless document to Markdown converter, in two interchangeable
forms. It backs the `trigger_documentation_service` flag on byte uploads and the
`fs.documentize` tool: the companion lands at the exact path `fs.extract_text`
looks at (`toto.pptx` -> `toto.md`), so it is then served as a cache hit by the
built-in extractor. **Off by default**, so no existing deployment changes.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `false` | when false, the upload flag answers `ERR_NOT_SUPPORTED` rather than silently storing a file without its companion |
| `mode` | string | `cli` | `cli` or `api`. Anything else is `ERR_INVALID_ARGUMENT` naming the accepted values |
| `extensions` | list | `[]` | overrides the built-in eligible set when non empty, so a deployment can narrow it to what its converter really handles |
| `max_input_bytes` | int | `536870912` (512 MiB) | refuse to convert beyond this; checked **before** any byte is written |
| `cli.command` | list | `[doc-convert, --stdout, --quiet, {document}]` | argv, never a shell string. Exactly one `{document}` placeholder across the whole list |
| `cli.timeout_secs` | int | `900` | wall clock ceiling for one conversion; on expiry the child is killed and reaped |
| `api.url` | string | `""` | `POST` target; must start with `http://` or `https://` |
| `api.auth_header` | string | `Authorization` | header **NAME**, not necessarily a bearer scheme |
| `api.auth_token` | string | `""` | **Secret**: inject it with `${VAR}`. Sent **verbatim**, so a bearer is written `auth_token: "Bearer ${TOKEN}"`. Omitted entirely when empty |
| `api.file_field` | string | `file` | name of the `multipart/form-data` part carrying the document, because a third party endpoint names it whatever it likes |
| `api.response_field` | string | `""` | empty means the response body IS the Markdown; otherwise the body is JSON and this names the field carrying it |
| `api.timeout_secs` | int | `900` | request timeout of the shared HTTP client, built once at boot |

The built-in eligible set is PowerPoint (`.pptx .pptm .potx .ppsx .ppt`), Word
(`.docx .doc`), PDF, audio (`.mp3 .m4a .wav .ogg .flac .aac .opus .wma`) and
video (`.mp4 .mov .mkv .webm .avi .m4v .mpeg .mpg`). `.xlsx` and images are
deliberately absent: `fs.extract_text` already covers them.

### Validation at boot

Checked only when `enabled` is true, because an unfinished draft must not stop a
boot that does not use the feature. Each failure is `ERR_INVALID_ARGUMENT` naming
the offending key:

* `mode` is not one of the two accepted values;
* `mode: cli` and `cli.command` is empty, or carries anything other than exactly
  one `{document}` placeholder (none means the converter is never told which file
  to read, two means it is handed a document it was never asked to merge);
* `mode: api` and `api.url` is empty or is not `http://` / `https://`;
* `api.auth_token` is set while `api.auth_header` is empty, which would silently
  drop the token and leave the operator believing the endpoint is authenticated;
* `api.file_field` is empty, which every server rejects on the first upload.

### Example, cli mode

```yaml
doc_service:
  enabled: true
  mode: cli
  max_input_bytes: 536870912
  cli:
    command: ["doc-convert", "--stdout", "--quiet", "{document}"]
    timeout_secs: 900
```

### Example, api mode

```yaml
doc_service:
  enabled: true
  mode: api
  extensions: [".pdf", ".pptx", ".docx"]
  api:
    url: https://convert.internal/v1/markdown
    auth_header: Authorization
    auth_token: "Bearer ${MCPFS_DOC_SERVICE_TOKEN}"
    file_field: file
    response_field: ""
    timeout_secs: 900
```

The request is `POST url`, `multipart/form-data`, one part named by
`api.file_field` (`file` by default) carrying the original filename and the MIME
guessed from its extension (`application/octet-stream` when unknown), Markdown
back on 2xx. Only the part name and the response shape are negotiable, which is
what `api.file_field` and `api.response_field` are for: a third party endpoint
needs no adapter of ours.

### Sandboxing, and its limit

In `cli` mode the converter is an arbitrary third party binary, so containment is
by construction: every call gets a fresh temporary directory; the input is written
inside it under a sanitized single segment name keeping the original extension;
the child runs with that directory as its working directory and `{document}`
expands to a **relative** `./name.ext`, so a converter that writes beside its
input (as `doc-convert` does, it creates `toto_docling/`) writes inside the
sandbox; `TMPDIR`, `TMP` and `TEMP` point there too; the command is an argv list
so there is no shell, no redirection, no `&&` and no glob; only stdout is read;
stderr is captured, capped at 8 KiB keeping the tail, and surfaced only on
failure; and the directory is removed on every exit path, including a timeout,
where the child is killed and reaped **before** the removal so no process is left
writing into a directory being deleted.

**This is not a hard OS sandbox, and it is not sold as one.** Containment is cwd
plus a relative path plus `TMPDIR` plus argv-without-shell. A converter that
writes to an absolute path or into `$HOME` escapes it and the cleanup will not
catch it. Full containment is the operator's call and is deliberately left there,
because a general purpose jail would break a converter that legitimately needs
network access and a model cache: wrap argv[0] in your own jail, which is exactly
why `command` is a list.

```yaml
doc_service:
  cli:
    command: ["docker", "run", "--rm", "-i", "my/doc-convert", "--stdout", "{document}"]
```

`env_clear` is deliberately NOT used: a converter legitimately needs `HOME`,
`PATH` and its model caches.

## `git`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `false` | registers the 14 git tools and the git HTTP routes |
| `object_format` | string | `sha1` | `sha256` is accepted and ignored (the bundled libgit2 is sha1 only) |
| `anonymous_read` | bool | `false` | allow unauthenticated clone and fetch |
| `max_pack_size_mb` | int | `512` | push body ceiling, enforced (413) |
| `github_client_id` | string | `""` | GitHub App client id for the `git.auth` device flow |
| `github_client_secret_env` | string | `MCPFS_GITHUB_CLIENT_SECRET` | name of the env var holding the GitHub secret |
| `gitlab_client_id` | string | `""` | GitLab application id |
| `gitlab_client_secret_env` | string | `GITLAB_CLIENT_SECRET` | name of the env var holding the GitLab secret |
| `gitlab_instance_url` | string | `https://gitlab.com` | base URL for a self hosted GitLab |

Details in [`.agent_docs/git.md`](git.md).

## Derived paths

Under the SQLite backend, only `infra.meta.dir`, `infra.blob.dir` and
`infra.admin.path` are configurable; everything else is derived, so moving
`infra.meta.dir` moves the whole state tree. A store on PostgreSQL or SQL Server uses
its dsn instead and ignores these paths, except `git_repo_dir`, which stays on disk
because libgit2 needs a real working directory.

| Helper | Value |
|---|---|
| `volume_meta_path(p)` | `{infra.meta.dir}/{p}.db` |
| `volume_bucket(p)` | `{infra.blob.bucket_prefix}{p}` |
| `admin_db_path()` | `infra.admin.path` |
| `state_root()` | parent of `infra.meta.dir` (so `state/` by default) |
| `git_db_path(p)` | `{state_root}/git/{p}.db` |
| `git_repo_dir(p)` | `{state_root}/git-repos/{p}/` |
| `oauth_db_path()` | `{state_root}/oauth.db` |

## `${VAR}` expansion

Expansion runs on the **raw text before the YAML is parsed** (`config::expand_env`),
so any value in the file can come from the environment:

| Form | Result |
|---|---|
| `${VAR}` | the value of `VAR`, or the empty string when unset or empty |
| `${VAR:-default}` | the value of `VAR`, or `default` when unset **or empty** |
| `$notavar`, `${}`, `${1BAD}` | left untouched (a name must be alphanumeric plus `_` and must not start with a digit) |

An empty variable counts as unset, which is why `MCPFS_MINIO_ACCESS_KEY=` in a
`.env` still falls back to the template default.

## Secrets policy

Secrets never live in a committed file. They reach the process through the
environment only: either expanded into the YAML with `${VAR}`, or read at the
composition root by name.

| Variable | Used by |
|---|---|
| `MCPFS_MINIO_SECRET_KEY` | `infra.blob.secret_key` in `config/minio.yaml.template` |
| `MCPFS_GITHUB_CLIENT_SECRET` | the `git.auth` GitHub device flow (name configurable) |
| `MCPFS_TOKEN_KEY` | when set, OAuth tokens are persisted encrypted (AES-256-GCM, 32 byte base64 key); unset means memory only |
| any name you choose | `doc_service.api.auth_token`, for example `${MCPFS_DOC_SERVICE_TOKEN}` |
| any name you choose | the `dsn` of a relational store, for example `${MCPFS_META_DSN}` |

A **dsn is a secret**, because it usually carries a password. `dsn` is typed as `Dsn`,
not `String`, and that type redacts itself in both `Debug` and `Display`, printing
`Dsn(redacted)` or `redacted`. So a dsn cannot reach a log line, a boot banner or an
error message even by accident, and there is a test asserting it. Never format a dsn
with `{:?}` expecting to see it, and never add a field that prints one.

`doc_service.api.auth_token` is typed `Secret`, the same newtype contract for a
credential that is not a connection string: `Debug` prints `Secret(redacted)` or
`Secret(unset)`, `Display` prints `redacted` or `unset`, and `expose()` is the
single audited read point, used once where the request header is built. A test
asserts the token does not appear in a `{:?}` of the whole config.

* `.env` is **gitignored**; `.env.example` is tracked as the template. `run.sh`
  sources `.env` when present.
* `config/local.yaml` is **gitignored** (personal working copy). Two templates are
  tracked: `config/local.yaml.template` (SQLite plus local blobs, zero external
  service) and `config/minio.yaml.template` (SQLite plus MinIO/S3 blobs, secret
  through `${MCPFS_MINIO_SECRET_KEY}`). `run.sh` bootstraps the first one on a
  fresh checkout.
* `state/` and `.keys/` are gitignored too.
* No secret is ever logged: tokens redact themselves in `Debug` output.

## Config path resolution

The config file is mandatory. `mcp-fs serve` picks it in this order, highest priority
first, which is the same CLI then environment then file pattern the rest of the CLI
follows:

| # | Source | Behaviour |
|---|---|---|
| 1 | `--config PATH` (also `-c`) | used as given |
| 2 | `$MCP_FS_CONFIG` (a full path) | used as given |
| 3 | `~/.config/mcp-fs/config.yaml` | probed, used only when it exists |
| 4 | `${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}.yaml` | probed, used only when it exists |

Cases 1 and 2 are explicit, so a missing file is reported by the name you gave. Cases 3
and 4 are probed, and when neither exists the error lists every path tried rather than
naming just one.

Step 3 honours `$XDG_CONFIG_HOME` when it is set and falls back to `$HOME/.config`. It
is new, and because it only applies when the file exists it cannot disturb a checkout
that relies on the working directory relative `config/local.yaml`.

A missing or unreadable file is `ERR_INVALID_ARGUMENT`; a YAML error reports the parse
failure. The keypair is unrelated to this resolution: `mcp-fs keys --dir .keys` writes
`jwt.key` and `jwt.pub`, and `auth.jwt.public_key_path` must point at the `.pub` file.
