# The `/api/fs` REST plane

Mounted only when `api.enabled` is true (default). Two families share one guard:
a **bytes plane** for a UI or a script (browse, upload, download, zip) and
**endpoint for endpoint parity with the `fs.*` tools**, which calls the very same
`core::fs_ops` functions the tools call. 36 routes, asserted against the
documentation table by a test, so a route cannot ship undocumented.

Implementation: `api/dataplane.rs` (routes), `api/openapi.rs` (spec plus UI).

## Auth rules

| Route group | Bearer | Membership |
|---|---|---|
| `/api/fs/roots` | required | none (the answer *is* the ACL for the caller) |
| `/api/fs/{mount_id}/**` | required | `require_member(mount_id, person)` |
| `/api/swagger.json`, `/api/docs`, `/api/docs/*` | none | none |

The bearer comes from the configured header (`auth.jwt.header`, default
`X-Forwarded-Authorization`) then `Authorization`; `Basic` is accepted with the
token as the password. A 401 body is `{"error": "ERR_UNAUTHENTICATED", "detail":
"ERR_UNAUTHENTICATED: ..."}`; `detail` repeats the code, matching the reference.

## Bytes plane

| Method | Path | Parameters | Purpose |
|---|---|---|---|
| GET | `/api/fs/roots` | none | `{person, roots[{mount_id, owner}]}` |
| GET | `/api/fs/{mount_id}/list` | query `path` (default `/`) | listing for a browser: directories first, then names caselessly; entries carry `name`, `kind`, `size`, `mtime` |
| POST | `/api/fs/{mount_id}/mkdir` | body `path` | create with parents, idempotent |
| POST | `/api/fs/{mount_id}/delete` | body `path` | **hard** delete (no trash), file or subtree; 404 `{detail}` when absent |
| POST | `/api/fs/{mount_id}/move` | body `source`, `destination` | rename or relocate; 404 `{detail}` when the source is absent |
| POST | `/api/fs/{mount_id}/upload` | multipart: file parts, field `directory` (destination, default `/`), repeated field `paths` (per file relative path) | `{written[], count}` |
| GET | `/api/fs/{mount_id}/download` | query `path` (required) | raw bytes as an attachment, MIME guessed |
| GET | `/api/fs/{mount_id}/download-zip` | query `path` (default `/`) | subtree as a zip, entry names relative to that root |

Two deliberate differences from the matching tools, both inherited from the
reference: the REST `delete` never uses the trash (a UI owns its own undo story)
and the REST `move` has no no clobber flag. Attachment responses send the
filename twice (sanitized ASCII plus RFC 5987 `filename*`) so unicode names
survive.

## Tool parity, GET

Query parameter names are the tool parameter names, and the payload is the tool
payload (see `.agent_docs/tools.md` for the return keys). `mount_id` is the path
segment; `path` is required unless stated.

| Path | Query parameters | Tool |
|---|---|---|
| `read` | `path`, `offset_lines=0`, `limit_lines=2000`, `line_numbered=true` | `fs.read` |
| `read-bytes` | `path`, `offset_bytes=0`, `length_bytes=65536` | `fs.read_bytes` |
| `read-lines` | `path`, `start_line`, `end_line` (both required) | `fs.read_lines` |
| `read-section` | `path`, `anchor_line` (required), `max_lines=200` | `fs.read_section` |
| `head` / `tail` | `path`, `lines=20` | `fs.head` / `fs.tail` |
| `stat` | `path` | `fs.stat` |
| `exists` | `path` | `fs.exists` |
| `hash` | `path`, `algo=sha256` | `fs.hash` |
| `count-lines` | `path` | `fs.count_lines` |
| `glob` | `pattern` (required), `root=/`, `exclude_patterns` (repeatable) | `fs.glob` |
| `grep` | `pattern` (required), `root=/`, `include_glob`, `exclude_glob`, `regex=true`, `case_sensitive=true`, `output_mode=content`, `context_lines=0`, `max_matches=100` | `fs.grep` |
| `tree` | `path=/`, `max_depth=3`, `exclude_patterns` (repeatable), `with_sizes=false` | `fs.tree` |
| `find-definition` | `name` (required), `root=/`, `kind` | `fs.find_definition` |
| `find-references` | `name` (required), `root=/` | `fs.find_references` |
| `audit-log` | `since` (number), `limit=20` | `fs.audit_log` |

A missing required value or an unparsable one is `ERR_INVALID_ARGUMENT` (400),
never a silent default. Booleans accept `true/false/1/0`. A repeatable parameter
is read with every occurrence in order (`?exclude_patterns=a&exclude_patterns=b`).

## Tool parity, POST

JSON body, snake_case, same names and defaults as the tool minus `mount_id`.

| Path | Body | Tool |
|---|---|---|
| `read-many` | `paths` (required), `per_file_cap_lines=500` | `fs.read_many` |
| `write` | `path`, `content`, `overwrite=false`, `create_parents=true` | `fs.write` |
| `append` | `path`, `content`, `create=false` | `fs.append` |
| `create-empty` | `path`, `exist_ok=false` | `fs.create_empty` |
| `copy` | `source`, `destination`, `overwrite=false`, `recursive=false` | `fs.copy` |
| `edit` | `path`, `old_string`, `new_string`, `replace_all=false`, `dry_run=false` | `fs.edit` |
| `multi-edit` | `path`, `edits` (array), `dry_run=false` | `fs.multi_edit` |
| `search-replace` | `path`, `search_block`, `replace_block`, `fuzzy=false` | `fs.search_replace` |
| `insert-at-line` | `path`, `line`, `content` | `fs.insert_at_line` |
| `apply-patch` | `patch_text` | `fs.apply_patch` |
| `extract-text` | `path`, `max_chars=200000`, `preview_chars=4000`, `ocr=true`, `refresh=false` | `fs.extract_text` |
| `write-docx` | `path`, `markdown`, `title=null`, `overwrite=false` | `fs.write_docx` |

Bodies go through the same tolerant accessors as `tools/call` arguments, so a
REST body and an MCP argument object behave identically (a string array accepts an
array, a bare string, or a comma separated string).

`apply-patch` is the one endpoint that does not call `fs_ops` directly: there is
no `fs_ops::apply_patch`, so it dispatches to the registered `fs.apply_patch`
tool (a documented TODO at the call site) and answers `ERR_NOT_SUPPORTED` if that
tool is not registered. No logic is duplicated either way.

The `admin.*` and `git.*` tools have no REST counterpart.

## Error mapping

Body is always `{"error": "ERR_*", "detail": "ERR_*: message"}`, except the bytes
plane short circuits which answer `{"detail": "..."}` with 404, matching the
reference.

| Code | Status |
|---|---|
| `ERR_UNAUTHENTICATED` | 401 |
| `ERR_FORBIDDEN` | 403 |
| `ERR_PROJECT_NOT_FOUND`, `ERR_NOT_FOUND` | 404 |
| `ERR_NO_CLOBBER` | 409 |
| `ERR_PATH_OUT_OF_BOUNDS`, `ERR_INVALID_ARGUMENT` | 400 |
| `ERR_EDIT_WITHOUT_PRIOR_READ`, `ERR_AMBIGUOUS_MATCH`, `ERR_NO_MATCH`, `ERR_WRITE_QUOTA_EXCEEDED`, `ERR_NOT_SUPPORTED`, `ERR_PROJECT_EXISTS`, `ERR_INTERNAL_ERROR` | 500 (no explicit mapping, fallback) |

The 500 fallback row is a faithful copy of the reference map and part of the
pinned contract; it is why a quota rejection reaches a REST client as 500.

## Body limits

| Limit | Value | Why |
|---|---|---|
| JSON endpoints | 30 MiB | axum defaults to 2 MiB, far below what the reference (Kestrel `MaxRequestBodySize`) accepts, so a large `write` would fail on one server only |
| `upload` (multipart) | 128 MiB | the ASP.NET Core `MultipartBodyLengthLimit` the reference runs with |

## OpenAPI surface

* `GET /api/swagger.json`: OpenAPI 3.0.1, built per request from the live state.
* `GET /api/docs` (and `/api/docs/`): Swagger UI, assets served from the copy
  embedded in `utoipa-swagger-ui`, so the page works with no network access.
* Both are **public**: the reference guards only the MCP prefix, and an
  unauthenticated client still needs the docs to learn how to authenticate.

**Descriptions are inherited from the tool registry, never written on the
endpoint.** For each route, `api/openapi.rs` maps `/api/fs/{mount_id}/{sub}` to
the tool `fs.{sub with hyphens replaced by underscores}` (overrides: `list` to
`fs.list_dir`, `roots` to `fs.list_allowed_roots`), then copies the tool
description into `summary`/`description` and each tool parameter description onto
the matching REST parameter or request body field. Consequences, all deliberate:

* documenting a tool parameter documents the REST endpoint at the same time;
* **the Swagger page doubles as an audit of the LLM facing tool docs**: a blank
  description there means a blank description in the tool schema;
* **REST parameter names must equal the tool parameter names**, which is why
  `/read-bytes` takes `offset_bytes` and `length_bytes` rather than `offset` and
  `length`. Rename one side and the description silently disappears.

The bytes plane routes have no tool, so they carry the small explicit summaries in
`REST_ONLY` / `REST_ONLY_PARAMS`. `/health` is documented without a tool. The
three git HTTP paths appear only when `git.enabled`, mirroring the router. The
route and body shapes themselves live in the `OPERATIONS` and `SCHEMAS` tables,
because the REST surface is not the tool surface (`mkdir` takes only `path` while
`fs.mkdir` also takes `parents` and `exist_ok`).
