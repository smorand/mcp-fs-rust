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

## `infra.meta`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string | `sqlite` | only `sqlite` is implemented; anything else is `ERR_INVALID_ARGUMENT` |
| `dir` | string | `state/volumes` | one db per volume: `{dir}/{project_id}.db`. Its parent is the state root, from which the git and oauth paths are derived |

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

## `infra.admin`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `backend` | string | `sqlite` | only `sqlite` is implemented |
| `path` | string | `state/admin.db` | ACL registry (`project`, `project_member`) |

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

Only `infra.meta.dir`, `infra.blob.dir` and `infra.admin.path` are configurable;
everything else is derived, so moving `infra.meta.dir` moves the whole state tree.

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

`mcp-fs serve` picks the file in this order:

1. `--config PATH` (also `-c`),
2. `$MCP_FS_CONFIG` (a full path),
3. `${MCP_FS_CONFIG_DIR:-config}/${MCP_FS_CONFIG_NAME:-local}.yaml`.

A missing file is `ERR_INVALID_ARGUMENT` with the resolved path in the message; a
YAML error reports the parse failure. The keypair is unrelated to this resolution:
`mcp-fs keys --dir .keys` writes `jwt.key` and `jwt.pub`, and
`auth.jwt.public_key_path` must point at the `.pub` file.
