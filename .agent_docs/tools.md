# Tool reference (63 tools)

Facts below come from `TOOL_CONTRACT.txt` (captured from the running reference
server) and the `tools/` modules. Parameters are listed as
`name:type=default`; no `=` means required. `mount_id` is required on every
`fs.*` and `git.*` tool.

Authorization column: **member** = project membership (`AppState::authorize`),
**admin** = platform admin, **owner/admin** = project owner or platform admin,
**auth** = any verified identity, no project involved. Details in
[Authorization model](#authorization-model).

## fs read (8)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.read` | line numbered, paged window over a text file | `path`, `offset_lines:int=0`, `limit_lines:int=2000`, `line_numbered:bool=true` | `content`, `total_lines`, `truncated`, `next_offset` | member |
| `fs.read_bytes` | raw bytes, base64, with MIME type | `path`, `offset_bytes:int=0`, `length_bytes:int=65536` | `base64`, `mime_type`, `length` | member |
| `fs.read_lines` | inclusive line range | `path`, `start_line:int`, `end_line:int` | `content`, `total_lines` | member |
| `fs.read_section` | indentation block around an anchor line | `path`, `anchor_line:int`, `max_lines:int=200` | `content`, `start_line`, `end_line` | member |
| `fs.read_many` | batch read, per file error isolation | `paths:array`, `per_file_cap_lines:int=500` | `files[]` of `{path, content, truncated}` or `{path, error}` | member |
| `fs.head` | first N lines | `path`, `lines:int=20` | `content` | member |
| `fs.tail` | last N lines | `path`, `lines:int=20` | `content` | member |
| `fs.count_lines` | line count without content | `path` | `total_lines` | member |

## fs write (4)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.write` | create or overwrite, atomic | `path`, `content`, `overwrite:bool=false`, `create_parents:bool=true` | `path`, `bytes_written`, `overwritten`, `diff` | member |
| `fs.append` | append, optionally create | `path`, `content`, `create:bool=false` | `path`, `bytes_appended` | member |
| `fs.create_empty` | touch | `path`, `exist_ok:bool=false` | `path`, `created` | member |
| `fs.write_bytes` | write raw bytes, base64 | `path`, `base64`, `overwrite:bool=false`, `create_parents:bool=true`, `trigger_documentation_service:bool=false` | `path`, `bytes_written`, `overwritten`, `documentation` | member |

`fs.write` on an existing file needs a prior read in the session and returns the
unified diff of the change. Overwriting without `overwrite=true` is
`ERR_NO_CLOBBER`.

`fs.write_bytes` is the only way to put binary content in a volume through MCP;
`base64` is the standard alphabet with padding, the one `fs.read_bytes` returns,
and anything else is `ERR_INVALID_ARGUMENT`. With
`trigger_documentation_service` the file is also converted to Markdown by the
configured `doc_service` and the companion stored beside it (`deck.pptx` ->
`deck.md`): `documentation` is then `{md_path, bytes_written}`, or
`{error: {code, message}}` when the conversion failed, and `null` when the flag
was off. Eligibility (PowerPoint, Word, PDF, audio, video) and the size cap are
checked **before anything is written**, so a flag on a `.txt` is
`ERR_NOT_SUPPORTED` with no file stored; a conversion that fails afterwards never
rolls the upload back.

## fs edit (5)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.edit` | replace a unique string | `path`, `old_string`, `new_string`, `replace_all:bool=false`, `dry_run:bool=false` | `path`, `applied`, `diff` | member |
| `fs.multi_edit` | several edits, all or nothing | `path`, `edits:array`, `dry_run:bool=false` | `path`, `applied`, `edits`, `diff` | member |
| `fs.search_replace` | replace a multi line block | `path`, `search_block`, `replace_block`, `fuzzy:bool=false` | `path`, `applied`, `diff` | member |
| `fs.insert_at_line` | insert before a 1 based line | `path`, `line:int`, `content` | `path`, `applied`, `line` | member |
| `fs.apply_patch` | multi file V4A patch in one volume | `patch_text` | `files[]` of `{path, op, moved_to?}` (`op`: add / update / delete) | member |

Every tool here enforces the read guard (`fs.apply_patch` on its update and
delete operations, not on an add, which creates a new file). A non unique
`old_string` is `ERR_AMBIGUOUS_MATCH`, an absent one `ERR_NO_MATCH`. `edits[]` items are
`{old_string, new_string, replace_all?}`. `dry_run` returns the diff and writes
nothing.

## fs search (4)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.glob` | files by glob, newest first, cap 100 | `pattern`, `root:string="/"`, `exclude_patterns=null` | `matches`, `truncated` | member |
| `fs.grep` | content search | `pattern`, `root="/"`, `include_glob=null`, `exclude_glob=null`, `regex:bool=true`, `case_sensitive:bool=true`, `output_mode="content"`, `context_lines:int=0`, `max_matches:int=100` | `content`: `matches[{path,line,text,context}]` + `truncated`; `files`: `files[]`; `count`: `count`, `files` | member |
| `fs.find_definition` | symbol definition, language aware | `name`, `root="/"`, `kind=null` | `definitions[{path,name,kind,line}]` | member |
| `fs.find_references` | identifier references | `name`, `root="/"` | `references[{path,line,kind}]` | member |

Walks prune `.git`, `node_modules`, `target`, `dist`, `.build`, `coverage`,
`.mcp_trash` and stop at 5000 files. Symbol lookup uses tree-sitter where a
grammar is bundled and falls back to a lexical index otherwise.

## fs listing (2)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.list_dir` | flat listing | `path="/"`, `include_hidden:bool=false`, `sort_by="name"` (or `size`), `with_sizes:bool=false` | `path`, `entries[{name,kind,size?}]`, `total` | member |
| `fs.tree` | recursive JSON tree | `path="/"`, `max_depth:int=3`, `exclude_patterns=null`, `with_sizes:bool=false` | `path`, `tree[]`, `truncated` | member |

`fs.tree` stops at 2000 nodes. Its exclude set does not hide the trash
directory, matching the reference.

## fs metadata (3)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.stat` | POSIX metadata | `path` | `path`, `size`, `mode` (e.g. `"0o644"`), `kind`, `mtime`, `ctime`, `atime`, `uid`, `gid` | member |
| `fs.exists` | probe path and kind | `path` | `exists`, `kind` (null when absent) | member |
| `fs.hash` | content hash | `path`, `algo="sha256"` (md5, sha1, sha256, sha512) | `path`, `algo`, `hash`, `size` | member |

`uid`/`gid` are the synthetic constants 1000/1000: a volume has no real POSIX
owner.

## fs lifecycle (6)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.mkdir` | create a directory | `path`, `parents:bool=true`, `exist_ok:bool=true` | `path`, `created` | member |
| `fs.delete` | delete, trash by default | `path`, `recursive:bool=false`, `trash:bool=true` | `path`, `trashed`, `trash_path` | member |
| `fs.move` | rename or relocate | `source`, `destination`, `overwrite:bool=false` | `source`, `destination` | member |
| `fs.copy` | copy a file or tree | `source`, `destination`, `overwrite:bool=false`, `recursive:bool=false` | `source`, `destination` | member |
| `fs.list_allowed_roots` | volumes the caller can reach | (`mount_id` only) | `person`, `roots[{mount_id, root, owner}]` | member |
| `fs.audit_log` | mutations recorded this session | `since:number=null`, `limit:int=20` | `entries[{timestamp, op, path, detail}]` | member |

`trash=false` needs `safety.allow_hard_delete` on the server, otherwise
`ERR_NOT_SUPPORTED`. `fs.list_allowed_roots` and `fs.audit_log` still take and
check `mount_id` even though they never open the volume.

## fs document (3)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `fs.extract_text` | document to Markdown, stored as a companion `.md` | `path`, `max_chars:int=200000`, `preview_chars:int=4000`, `ocr:bool=true`, `refresh:bool=false` | `path`, `md_path`, `format`, `chars`, `cached`, `preview` | member |
| `fs.write_docx` | render Markdown into a `.docx` | `path`, `markdown`, `title=null`, `overwrite:bool=false` | `path`, `bytes_written`, `overwritten` | member |
| `fs.documentize` | convert a stored document through the external `doc_service` | `path`, `overwrite:bool=false` | `path`, `md_path`, `bytes_written`, `overwritten` | member |

`fs.documentize` is the retry surface of the document service: it reads the file
already in the volume, converts it and writes the companion at the same path
`fs.extract_text` uses, so the built-in extractor then answers `cached: true`.
Unlike an upload it honours the caller's `overwrite`, so an existing companion is
`ERR_NO_CLOBBER`. It answers `ERR_NOT_SUPPORTED` when `doc_service` is disabled or
the extension is not eligible, and `ERR_NOT_FOUND` when the path is not a file.

`fs.extract_text` reuses an up to date companion (`cached: true`, nothing
written) and handles PDF, DOCX, PPTX, XLSX, HTML, CSV, images (OCR through a
configured multimodal provider, disabled by default) and text. Audio and video
are unsupported (`ERR_NOT_SUPPORTED`). `fs.write_docx` requires a `.docx` path.

## admin (10)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `admin.create_project` | create a project and provision its volume | `project_id`, `owner` | `project_id`, `owner`, `created_at` | admin |
| `admin.delete_project` | delete a project and tear down its volume | `project_id` | `project_id`, `deleted` | owner/admin |
| `admin.list_projects` | projects the caller can access | (none) | `projects[{project_id, owner, created_at, index_mode, is_owner}]` | auth |
| `admin.list_all_projects` | every project | (none) | `projects[{project_id, owner, created_at, index_mode}]` | admin |
| `admin.list_users` | every known person plus platform admins | (none) | `users[{person, is_admin}]` | admin |
| `admin.add_member` | add a member | `project_id`, `person` | `project_id`, `person`, `role` | owner/admin |
| `admin.remove_member` | remove a member | `project_id`, `person` | `project_id`, `person`, `removed` | owner/admin |
| `admin.list_members` | members of a project | `project_id` | `project_id`, `members[{person, role, added_by}]` | member or admin |
| `admin.set_index_mode` | set the search index mode and wipe or rebuild the index | `project_id`, `mode` | `project_id`, `index_mode`, `previous_mode`, `reindex_started` | owner/admin |
| `admin.get_index_mode` | read the search index mode | `project_id` | `project_id`, `index_mode` | member or admin |

`project_id` must be 3 to 32 characters of lowercase letters, digits and hyphens,
with alphanumeric first and last characters. Creation provisions the volume and
rolls the ACL row back if provisioning fails. Deletion also purges
`state/git/{id}.db` and the bare repo directory when git is enabled, so a
recreated id never inherits stale refs.

## git (14, registered only when `git.enabled`)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `git.init` | make the volume a git repository | (`mount_id`) | `mount_id`, `initialized`, `message` | member |
| `git.status` | HEAD, branch, refs | (`mount_id`) | `mount_id`, `head`, `branch`, `refs[{name, sha}]` | member |
| `git.branches` | branches with their sha | (`mount_id`) | `mount_id`, `branches[{name, full_ref, sha}]` | member |
| `git.tags` | tags | (`mount_id`) | `mount_id`, `tags[{name, full_ref, sha}]` | member |
| `git.log` | commits from a ref | `ref_name=null`, `limit:int=20`, `path=null` | `mount_id`, `commits[]` | member |
| `git.show` | one commit plus its diff | `commit_sha` | `commit{}`, `diff` | member |
| `git.diff` | diff two refs, or a ref and the volume | `from_ref`, `to_ref=null`, `path=null` | `mount_id`, `from`, `to`, `diff` | member |
| `git.commit` | commit the current volume state | `message`, `author_name=null`, `author_email=null` | `commit_sha`, `message`, `author`, `timestamp` | member |
| `git.checkout_file` | restore a file from a commit | `commit_sha`, `path` | `path`, `commit`, `size` | member |
| `git.blame` | last change per line | `path`, `ref_name=null` | `path`, `lines[{line, commit, author, email, date}]` | member |
| `git.remote_clone` | clone a remote into the volume | `url`, `branch=null`, `depth:int=0` | `mount_id`, `url`, `branch`, `commit`, `commit_message`, `files_imported`, `commits_imported`, `depth`, `auth`, `skipped?` | member |
| `git.remote_push` | push a local branch to `origin`, fast-forward only, no force | `branch` | push outcome: remote sha, `created`, `up_to_date` | member |
| `git.remote_fetch` | fetch objects, update `refs/remotes/origin/*` only, never a working file | (`mount_id`) | updated refs, `refs_stale[]`, objects downloaded | member |
| `git.remote_pull` | fetch, then fast-forward or (with `on_conflict`) three-way merge the checked-out branch | `branch`, `on_conflict=null` (`ours`\|`theirs`) | fast-forward or merge outcome | member |

A commit object is `{sha, short_sha, message, author, author_email, timestamp,
date, parents[]}`. `git.remote_clone` uses the OAuth token stored for the
detected provider/host when there is one; an empty remote returns
`{mount_id, url, files_imported: 0, message}`. `git.remote_push`,
`git.remote_fetch` and `git.remote_pull` take no `url`: they resolve the
volume's stored `origin`. See [`git.md`](git.md) for the full remote pipeline,
the `git.hosts` host map and the `on_conflict` merge semantics.

## git.auth (4, registered only when `git.enabled`)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `git.auth` | start the OAuth device flow | `provider` (`github` or `gitlab`), `host=null`, `instance_url=null` | `status` (`pending`), `provider`, `user_code`, `verification_uri`, `expires_in`, `message` | auth |
| `git.auth_status` | authentication status | `provider=null`, `host=null` | one host: `authenticated`, `provider`, plus `scopes`, `expires_at` when authenticated; omitted: `statuses[]` | auth |
| `git.auth_revoke` | drop the stored token | `provider=null`, `host=null` | `provider`, `revoked` | auth |
| `git.token_set` | seed a PAT already held for a declared host, skipping the device flow | `host`, `token`, `expires_at=null` | confirmation, never echoes the token | auth |

`git.auth` returns as soon as the provider issues a user code; a detached task
polls the token endpoint, so the client waits by calling `git.auth_status`. A
token belongs to a person plus host (not provider alone, and not a mount): see
[`git.md`](git.md#token-identity-is-per-person-host).

## Authorization model

| Gate | Implementation | Applies to |
|---|---|---|
| Verified identity | `IdentityResolver::verify`, RS256, `iss`, `exp`/`nbf`, 30s leeway | every tool call, checked before dispatch |
| Membership | `AdminBackend::require_member` | every `fs.*` and `git.*` tool |
| Platform admin | caseless match against `auth.admins` | `admin.create_project`, `admin.list_all_projects`, `admin.list_users` |
| Owner or platform admin | `AppState::require_owner_or_admin` | `admin.delete_project`, `admin.add_member`, `admin.remove_member`, `admin.set_index_mode` |
| Member or platform admin | inline in `tools/admin.rs` | `admin.list_members`, `admin.get_index_mode` |

**Separation of duties.** A platform admin manages projects and membership and
can list everything, but membership is never implied: `AppState::authorize` is
membership only, with no admin bypass, so a platform admin gets `ERR_FORBIDDEN`
on `fs.read` of a project it is not a member of. An admin who needs the files
adds itself as a member, which leaves a `project_member` row and an `added_by`
trail. The same rule holds on the REST plane and on the git HTTP routes.

`is_owner` and `is_admin` in tool output are computed caselessly, exactly like
the checks that authorize, so a reported flag can never contradict the gate.
