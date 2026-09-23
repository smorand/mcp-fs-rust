# Tool reference (94 tools)

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

## git (39, registered only when `git.enabled`)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `git.init` | make the volume a git repository | (`mount_id`) | `mount_id`, `initialized`, `message` | member |
| `git.status` | HEAD, branch, refs, in-progress operation | (`mount_id`) | `mount_id`, `head`, `branch`, `refs[{name, sha}]`, `operation` (null, or `{op_type, source_ref, current_step, total_steps, remaining_conflicts[], continue_with, abort_with}`) | member |
| `git.branches` | branches with their sha, the current marker and the divergence against `refs/remotes/origin/{name}` | (`mount_id`) | `mount_id`, `branches[{name, full_ref, sha, current, upstream, ahead, behind}]` (`upstream`/`ahead`/`behind` are null with no tracking ref, never 0) | member |
| `git.branch_create` | create a branch at a start point | `name`, `start_point=""`, `checkout:bool=false` | `branch`, `sha`, `checked_out` | member |
| `git.branch_switch` | move HEAD to an existing branch and rewrite the volume to its tree; refuses a dirty volume | `name` | `branch`, `sha`, `changed`, `files_changed` | member |
| `git.branch_delete` | remove `refs/heads/{name}`; refuses the checked-out branch, and an unmerged one without `force` | `name`, `force:bool=false` | `branch`, `sha`, `forced` | member |
| `git.branch_reset` | force-move a branch pointer; rewrites the volume only for the checked-out branch | `name`, `target_commit`, `force:bool=false` | `branch`, `old_sha`, `new_sha`, `checked_out`, `files_changed` | member |
| `git.reset` | move the CURRENT branch's pointer; `soft` moves the ref alone and never touches the volume (`hard` rewrites the volume, US-018) | `target_ref`, `mode` (`soft`\|`hard`, required, no `mixed`) | `mode`, `old_sha`, `new_sha`, `files_changed` (0 for soft) | member |
| `git.tags` | tags | (`mount_id`) | `mount_id`, `tags[{name, full_ref, sha}]` | member |
| `git.log` | commits from a ref | `ref_name=null`, `limit:int=20`, `path=null` | `mount_id`, `commits[]` | member |
| `git.show` | one commit plus its diff | `commit_sha` | `commit{}`, `diff` | member |
| `git.diff` | diff two refs, or a ref and the volume | `from_ref`, `to_ref=null`, `path=null` | `mount_id`, `from`, `to`, `diff` | member |
| `git.commit` | commit the current volume state | `message`, `author_name=null`, `author_email=null` | `commit_sha`, `message`, `author`, `timestamp` | member |
| `git.checkout_file` | restore a file from a commit | `commit_sha`, `path` | `path`, `commit`, `size` | member |
| `git.blame` | last change per line | `path`, `ref_name=null` | `path`, `lines[{line, commit, author, email, date}]` | member |
| `git.merge` | merge a ref into the checked-out branch, surfacing conflicts | `source_ref`, `squash:bool=false`, `message=null` | `status` (`merged`\|`already_up_to_date`), `merge_commit`, `fast_forward`, `squashed`, `files_changed`; on a conflict the shape below instead | member |
| `git.merge_resolve` | finish a conflicted merge, per file by strategy or by literal content | `resolutions[{path, strategy?, content?}]` | `status` (`merged`\|`conflict`), `merge_commit`, `remaining_conflicts[]`, `resolved_count`, `files_changed` | member |
| `git.merge_abort` | abandon a conflicted merge, discarding every recorded resolution | (`mount_id`) | `status` (`aborted`), `operation` | member |
| `git.rebase` | validate an interactive rebase plan and replay it onto another commit; the whole todo is checked before any commit is created | `onto`, `todo[{action, sha, message?}]` | `status` (`up_to_date`), `branch`, `new_tip`, `replayed`, `dropped`, `squashed` | member |
| `git.rebase_continue` | resume the paused rebase once every conflicting path of the paused commit is resolved | `resolutions[{path, strategy?, content?}]?` | the `git.rebase` shape, or the conflict shape when it pauses again | member |
| `git.rebase_abort` | abandon the paused rebase, restoring the branch exactly | (`mount_id`) | `status` (`aborted`), `operation` | member |
| `git.cherry_pick` | apply one commit's change onto the checked-out branch as a NEW commit, keeping the original author | `commit_sha`, `mainline:int=0` | `status` (`committed`\|`already_present`), `new_sha` (null when nothing was created), `source_sha`; on a conflict the shared conflict shape instead | member |
| `git.cherry_pick_continue` | finish the paused cherry-pick, per file by strategy or by literal content | `resolutions[{path, strategy?, content?}]?` | `status` (`committed`), `new_sha`, `source_sha` | member |
| `git.cherry_pick_abort` | abandon the paused cherry-pick, restoring the branch and every byte | (`mount_id`) | `status` (`aborted`), `operation`, `restored_sha` | member |
| `git.revert` | undo one commit by adding a NEW commit carrying the exact inverse change, leaving the original in history | `commit_sha`, `mainline:int=0` | `status` (`committed`\|`already_present`), `new_sha` (null when nothing was created), `reverted_sha`; on a conflict the shared conflict shape instead | member |
| `git.revert_continue` | finish the paused revert, per file by strategy or by literal content | `resolutions[{path, strategy?, content?}]?` | `status` (`committed`), `new_sha`, `reverted_sha` | member |
| `git.revert_abort` | abandon the paused revert, restoring the branch and every byte | (`mount_id`) | `status` (`aborted`), `operation`, `restored_sha` | member |
| `git.stash_save` | snapshot the dirty volume as a commit under `refs/stash/*`, then revert the volume to HEAD's tree | `message=null` | `stash_id`, `sha`, `message`, `base_sha`, `branch`, `created_at`, `files_stashed` | member |
| `git.stash_list` | every stash entry of the volume, newest first; read only, so it works while an operation is in progress | (`mount_id`) | `mount_id`, `stashes[{stash_id, message, base_sha, branch, created_at}]`, `count` | member |
| `git.stash_drop` | delete one entry by id; the volume and the commit object are untouched | `stash_id` | `stash_id`, `dropped` | member |
| `git.stash_apply` | replay one entry onto whatever is checked out now, keeping the entry | `stash_id` | `stash_id`, `status` (`applied`\|`conflict`), `files_changed`, `dropped` (always `false`); on a conflict the shared conflict shape carries those keys too | member |
| `git.stash_pop` | the same replay, deleting the entry only once every byte has landed; a conflict keeps it and reports `dropped: false` | `stash_id` | `stash_id`, `status`, `files_changed`, `dropped` | member |
| `git.remote_add` | record a named remote; https only, no embedded credential, host declared in `git.hosts`; a duplicate name is refused, never upserted | `name`, `url` | `name`, `url`, `host`, `provider` | member |
| `git.remote_remove` | delete a remote and every ref under `refs/remotes/{name}/`; an unknown name is `ERR_NOT_FOUND`, never a silent no-op | `name` | `name`, `removed` | member |
| `git.remote_list` | every remote of the volume, empty list when there is none; read-only, so it stays available while an operation is paused | (`mount_id`) | `mount_id`, `remotes[{name, url, host, provider}]`, `count` | member |
| `git.remote_clone` | clone a remote into the volume | `url`, `branch=null`, `depth:int=0` | `mount_id`, `url`, `branch`, `commit`, `commit_message`, `files_imported`, `commits_imported`, `depth`, `auth`, `skipped?` | member |
| `git.remote_push` | push a local branch to a named remote, fast-forward unless a lease is supplied | `branch`, `remote` (default `origin`), `remote_branch`, `force`, `expected_remote_sha` | `branch`, `remote`, `created`, `up_to_date`, `remote_sha`, `auth` (credential class, never a value), `forced`; plus `overwritten_sha` when forced and `remote_branch` when it differs from the local name | member |
| `git.remote_fetch` | fetch objects, update `refs/remotes/origin/*` only, never a working file | (`mount_id`) | updated refs, `refs_stale[]`, objects downloaded | member |
| `git.remote_pull` | fetch, then fast-forward or three-way merge the checked-out branch; a conflict pauses as a `merge` | `branch` | fast-forward, merge outcome, or the shared conflict response | member |

Every combine operation reports a conflict with one shape: `{status:
"conflict", operation, operation_id, source_ref, current_step, total_steps,
conflicts[{path, ours, theirs, base, binary, type_change}], continue_with,
abort_with}`, where each of `ours`, `theirs` and `base` is `{exists, content}`
and `content` is null for a deleted or binary side. `current_step` and
`total_steps` are zero-based and null for a single step operation. A conflict
applies nothing: no file, no commit, no ref move, and never a conflict marker
in the volume.

A conflict is finished with `git.merge_resolve` or abandoned with
`git.merge_abort` (the pair `continue_with`/`abort_with` names). Each
resolution carries exactly one of `strategy` (`ours`, the checked-out branch's
side, or `theirs`, the side being merged in, each taken whole) or `content`
(the literal bytes, which is how a caller merges the two sides itself); an
empty `content` is a valid resolution to an empty file, a missing one is an
error. A call is all-or-nothing: a path that is not in conflict, a duplicate
path, an unknown strategy or an empty list rejects the whole call and changes
nothing. Resolving only some paths keeps the operation in progress, records
the decisions in the `git_operations` row (so they survive a restart) and
writes nothing to the volume; resolving the last one charges the write quota,
applies every resulting file atomically and only then advances the ref.

A commit object is `{sha, short_sha, message, author, author_email, timestamp,
date, parents[]}`. `git.remote_clone` uses the OAuth token stored for the
detected provider/host when there is one; an empty remote returns
`{mount_id, url, files_imported: 0, message}`. `git.remote_push`,
`git.remote_fetch` and `git.remote_pull` take no `url`: they resolve the
volume's stored `origin`. See [`git.md`](git.md) for the full remote pipeline,
the `git.hosts` host map and the pull merge semantics.

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

## git.pr (6, registered only when `git.enabled`)

| Tool | Purpose | Parameters | Returns | Auth |
|---|---|---|---|---|
| `git.pr_create` | open a pull request (GitHub) or merge request (GitLab) on a declared remote | `mount_id`, `base`, `head`, `title`, `body=null`, `draft=false`, `remote=origin` | the normalized pull request (21 keys, see below) | member |
| `git.pr_list` | list the pull requests of a declared remote, filtered by state | `mount_id`, `state=open` (`open`\|`closed`\|`merged`\|`all`), `remote=origin` | `pull_requests[]` of the normalized shape, plus `count` | member |
| `git.pr_get` | read one pull request, enriched with its review state and its check state; an unknown state is a failure, never `none` | `mount_id`, `pr_number`, `remote=origin` | the normalized pull request | member |
| `git.pr_diff` | read the unified diff of one pull request, bounded by `git.max_pr_diff_mb` | `mount_id`, `pr_number`, `remote=origin` | `diff`, `truncated`, plus the pull request identity | member |
| `git.pr_merge` | merge a pull request on the provider with `merge`, `squash` or `rebase` | `mount_id`, `pr_number`, `strategy`, `commit_title=null`, `commit_message=null`, `remote=origin` | the normalized pull request in state `merged`, plus `note` naming `git.remote_fetch` | member |
| `git.pr_review` | submit a review verdict: `approve`, `request_changes` or `comment`; a provider refusal is surfaced as the provider's own status and message | `mount_id`, `pr_number`, `verdict`, `body=null`, `remote=origin` | the submitted review, plus the pull request identity | member |

Every `git.pr_*` tool returns ONE shape, defined once in
`git/provider/model.rs` as `PullRequest`, in this key order: `provider`, `host`,
`number`, `title`, `body`, `state` (`open`/`closed`/`merged`), `draft` (a
separate boolean, never folded into `state`), `base`, `head`, `author`, `url`,
`created_at`, `updated_at`, `commits`, `changed_files`, `additions`,
`deletions`, `review_state`, `checks_state`, `mergeable` (tri state: `true`,
`false`, `null` when the provider has not computed it), `raw` (the provider
payload untouched, already scrubbed of the credential). Details and the mapping
rules: [`git.md`](git.md#the-pull-request-surface-gitpr_-us-025).

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
