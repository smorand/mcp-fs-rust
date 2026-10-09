# mcp-fs — Archive Extraction In Place — Specification Document

> Generated on: 2026-10-08
> Id: SPEC-0015
> Nature: FEAT
> Depth: L
> Depth evidence: greenfield capability (no archive-extraction code exists anywhere in the tree,
> confirmed by grep in §2.1), 5 modules touched (`tools/`, `errors.rs`, `core/fs_ops.rs`, `api/`
> REST+OpenAPI, `mcp/server.rs`, `TOOL_CONTRACT.txt`/golden), 30 requirements, 4 new Cargo
> dependencies plus one new feature on an existing one. No de-escalation applied.
> Status: Draft
> Type: Evolution Specification
> From backlog: BL-0006
> Split: not split
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Executive Summary

Adds `fs.extract_archive`, a tool that extracts an archive already stored in a project's
filesystem directly into that filesystem, with no download/re-upload round trip. It supports
`.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2` and `.tar.xz`/`.txz`. Password-protected
zip and 7z archives are supported through a stateless retry: a missing or wrong password fails
with `ERR_PASSWORD_REQUIRED` and the caller simply re-issues the same call with `password` filled
in. Every entry is validated for path safety (zip-slip) and type (no symlink, hardlink, or device
entry survives) and the whole archive's declared size is charged against the write quota, all in
one pre-scan pass, before a single byte is written. `rar` is out of scope entirely, per an explicit
user decision recorded in the Decisions Log. The two doors are `fs.extract_archive` (MCP) and
`POST /api/fs/{mount_id}/extract-archive` (REST), both calling one `tools::archive::extract_archive`
function, mirroring the `fs.export_zip` precedent this spec reuses throughout.

## 2. Current State

### 2.1 How it works today

Nothing exists. `rg -i "tar::Archive|sevenz|bzip2|lzma|unrar|ZipArchive.*extract"
crates/core/src/` returns zero hits for any extraction capability (verified empty output during
this run; `fs.export_zip` is the only `zip` crate user and it only *writes* archives). There is no
`fs.extract_archive`, no `archive.rs` module, and no archive-extraction route anywhere in
`crates/core/src/`.

The nearest existing capability, and the structural template this spec copies, is
`fs.export_zip` (`crates/core/src/tools/export.rs:1-9`): one typed `pub(crate) async fn
export_zip(state: &AppState, mount_id: &str, paths: &[String]) -> Result<Value>`
(`crates/core/src/tools/export.rs:58`), called by the MCP `#[tool]` method
`McpServer::fs_export_zip` (`crates/core/src/mcp/server.rs:2075-2085`) and by the REST route
`POST /api/fs/{mount_id}/export-zip` (`crates/core/src/api/dataplane.rs:149`,
handler `export_zip` at `crates/core/src/api/dataplane.rs:633-648`), so the two doors share one
implementation. `export_zip` fully buffers the archive in memory (no size or count limit, per its
own `DEC-010`, `crates/core/src/tools/export.rs:67-69`) and has a private `ensure_no_escape` helper
(`crates/core/src/tools/export.rs:116-131`) layered on top of
`SafetyManager::normalize_path`, because `normalize_path` *clamps* an escaping path rather than
rejecting it — confirmed by reading `crates/core/src/safety.rs:89-106`: it runs
`PosixPath::normpath` and only rejects when the **normalized** result still starts with `/..`
(line 95), so a raw `/../etc` normalizes to `/etc` and passes clean. `ensure_no_escape` walks the
raw, pre-normalization path itself and rejects a net upward climb (`crates/core/src/tools/
export.rs:122-130`). This spec needs the mirror image of that helper — reject an **archive entry**
path that would climb outside the extraction destination — and reuses the same walking technique.

`VolumeClient` (`crates/core/src/storage/volume.rs`) exposes the primitives this spec's write pass
needs directly, the same way `export_zip` calls `client.read_bytes`/`client.blob.put` directly
rather than through `core::fs_ops`: `exists` (`volume.rs:79`), `walk` (`volume.rs:130`),
`write_bytes_atomic` (`volume.rs:157`), `makedirs` (`volume.rs:180`), `mkdir` (`volume.rs:184`),
and `touch_atime_mtime` (`volume.rs:62-68`, fire-and-forget). `core::fs_ops::write_bytes`
(`crates/core/src/core/fs_ops.rs:589-615`) wraps `write_bytes_atomic` with its own
`safety.charge_write` call at line 609; this spec's write pass calls `write_bytes_atomic`
directly instead, for the reason recorded in `DEC-006` below. `core::fs_ops::ensure_parents`
(`crates/core/src/core/fs_ops.rs:1199-1208`) is private (`fn`, not `pub(crate) fn`) and creates a
path's missing parent directories via `client.makedirs`; `FR-NEW-022` promotes it to
`pub(crate)` so this spec reuses it instead of duplicating its four lines, per the project's
"never reimplement an operation" convention (`AGENTS.md`, Conventions section).

`SafetyManager::charge_write` (`crates/core/src/safety.rs:131-142`) is **synchronous**, not async
(the backlog/governing-facts note describing it as async was wrong and is corrected here): it
takes `(&self, person: &str, project: &str, num_bytes: i64) -> Result<()>`, adds `num_bytes` to
the in-memory per-session `bytes_written` counter *only if* the new total stays under
`write_quota_bytes`, and returns `Err(ToolError::write_quota_exceeded(..))` **without mutating the
counter** otherwise (`safety.rs:134-138`). This is exactly the "nothing charged on failure"
property SC-010 and `FR-NEW-017` depend on, and it is true by reading the function, not assumed.
There is a matching `refund_write` (`safety.rs:149-154`), unused by this spec because
`charge_write`'s own fail-closed check already prevents an over-charge; nothing here is ever
charged and later needs to be given back.

`crates/core/src/errors.rs` defines exactly **14** `ERR_*` codes today (counted at
`errors.rs:9-22`; none named `PASSWORD_REQUIRED`), each with an explicit `http_status()` arm
(`errors.rs:96-124`) and three exhaustiveness tests: `every_code_has_an_http_status`
(`errors.rs:178-195`), `client_caused_codes_are_all_4xx` (`errors.rs:199-221`), and
`no_code_falls_through_to_an_accidental_500` (`errors.rs:225-246`), confirmed by their actual
names and line ranges rather than the paraphrase this run started from.

`TOOL_CONTRACT.txt` currently lists **149** tools, not the 101 `AGENTS.md`'s own count claims:
`grep -E '^(fs|admin|git|search)\.' TOOL_CONTRACT.txt | wc -l` returns `149` (`fs.`=75, `admin.`=21,
`git.`=49 including its `git.auth*` and `git.pr_*` subsets, `search.`=4), run during this session.
`AGENTS.md`'s count is stale: the project's own `specs/archived/` directory holds four specs
(`SPEC-0011` trash listing and recovery, `SPEC-0012` zip-export signed URL, `SPEC-0013` rmcp 3.x
migration prep, `SPEC-0014` auto-purge) that each added tools after that count was written, which
is also why this spec is numbered `SPEC-0015` rather than the `SPEC-0011` its own originating
instructions guessed: the highest `SPEC-NNNN` anywhere under `specs/`, archived included, is
`0014`, not the `0010` visible directly under `specs/`. `fs.extract_archive` will be the 150th
tool.

### 2.2 Existing specifications governing this area

- `SPEC-0003` (filesystem engine): owns `fs.*` tool conventions this spec follows (`mount_id`
  required, `state.authorize` then `normalize_path` then the engine call). Not modified; this spec
  adds one more `fs.*` tool under the same convention.
- `SPEC-0004` (REST data plane and OpenAPI): owns the `/api/fs/{mount_id}/*` route shape and the
  OpenAPI generation this spec extends with one more route and one more body schema. Not modified.
- `SPEC-0005` (multi-backend storage): owns `VolumeClient`/`SafetyManager`/quota; this spec is a
  pure consumer of `charge_write` and `write_bytes_atomic`, adding no new backend behavior. Not
  modified.
- `specs/archived/SPEC-0012_..._zip-export-signed-url` (the un-renumbered name for what is now read
  as the export-zip spec, archived because the project's spec lifecycle archives completed specs):
  the direct structural ancestor. Not modified; referenced throughout §2.1.
- No existing specification governs archive extraction, password-protected content handling, or a
  stateless-retry error pattern. This is new ground within an existing area (`fs.*` tools), hence
  `Type: Evolution Specification` rather than greenfield.

### 2.3 Existing test coverage

Zero. The feature does not exist, so there is nothing to regress. The exact test command, from
`AGENTS.md`'s own Key Commands section and `./test.sh` (read verbatim): `cargo test --workspace`
(`test.sh:5`), with the quality gate additionally requiring `cargo clippy --all-targets
--all-features -- -D warnings` and `cargo fmt --all -- --check` (`AGENTS.md`, "Quality gate"
section). `--all-features` matters here specifically because without it the workspace never
compiles the `postgres`/`sqlserver` backends, which is unrelated to this spec but is the existing
rule this spec's own test run must also satisfy.

## 3. Scope

### 3.1 In Scope

- One new tool, `fs.extract_archive`, extracting an archive already stored in a project's volume
  directly into that volume.
- Formats: `.zip`, `.7z`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tb2`, `.tar.xz`/`.txz`.
- Password-protected `.zip` (ZipCrypto legacy and AES, via the `zip` crate's `aes-crypto` feature)
  and `.7z` (via `sevenz-rust2`'s AES-256 support), with a stateless retry on
  `ERR_PASSWORD_REQUIRED`.
- Optional `destination` override; default destination derived by stripping the archive's
  recognized multi-part extension.
- `overwrite` flag (default `false`) governing whether an existing destination entry blocks the
  whole call.
- Per-entry path-safety (zip-slip) and type rejection (symlink/hardlink/device/special), all-or-
  nothing across the whole archive.
- Write-quota charging for the archive's total declared uncompressed size, in one pre-scan pass,
  before any write.
- The REST door `POST /api/fs/{mount_id}/extract-archive`, mirroring `export-zip`.
- The new `ERR_PASSWORD_REQUIRED` error code.

### 3.2 Out of Scope (Non-Goals)

- **RAR, in any form.** Not deferred, no backlog entry. User's exact words, recorded verbatim in
  `DEC-001`: "Remove completely rar, this was nice to have, not a requirement."
- **Creating or compressing archives.** Already covered by the existing `fs.export_zip`
  (`SPEC-0012`, archived); this spec only extracts.
- **Auto-recursing into archives found nested inside the extracted output.** A `.zip` written as
  part of extracting some other archive lands as an ordinary file; it is never auto-extracted.
- **Multi-volume archives** (`.7z.001`, `.zip.001`, split RAR parts, or any other split-archive
  convention). A multi-volume archive is handled like any other recognized-extension file whose
  bytes do not parse as that format: `ERR_INVALID_ARGUMENT`.
- **A stateful paused-operation mechanism.** No `continue`/`abort` tool pair, no new relational
  table. Every call is a fresh, fully stateless dry-run-then-write; a password retry is just the
  same tool call issued again with `password` filled in.

## 4. User Personas & Actors

- **Project member (human or LLM agent acting on a member's behalf)**: the only actor. Calls
  `fs.extract_archive` (or the REST equivalent) against an archive they can already read in a
  project they are a member of. There is no distinct admin or background-job actor for this
  capability: extraction is always a direct, synchronous, caller-initiated operation, like
  `fs.export_zip`.

## 4.5 Bounded Contexts

- **Archive parsing** (`tools::archive`, new): format detection, per-library entry enumeration and
  decoding, password handling. Owns no filesystem state; produces an in-memory decoded entry list.
- **Filesystem write** (existing `VolumeClient` + `SafetyManager`, consumed not owned): the
  pre-scan's conflict/quota checks and the write pass's `write_bytes_atomic`/`makedirs` calls.
  This spec is a pure consumer here, adding no new primitive.
- **Transport** (`mcp::server` + `api::dataplane` + `api::openapi`, extended not owned): the two
  doors and the OpenAPI schema, following the `export_zip`/`export-zip` pattern exactly.

These three contexts do not overlap: `tools::archive` never touches the filesystem directly except
through the write-pass primitives it calls at the very end, and the transport layer never parses
archive bytes itself.

## 5. Usage Scenarios

### SC-001: Extract a plain archive, default destination

- **Actor**: project member.
- **Preconditions**: `/uploads/report.tar.gz` exists in the volume and is a valid, unencrypted
  tar.gz archive containing `a.txt` and `sub/b.txt`.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/report.tar.gz")` with no
  `destination`, no `password`, default `overwrite=false`.
- **Postconditions**: `/uploads/report/a.txt` and `/uploads/report/sub/b.txt` exist with the
  archive's decoded bytes; response is `{"destination": "/uploads/report", "files_written": 2,
  "dirs_created": 1, "bytes_written": <sum>}`.
- **Exceptions**: none; this is the happy path.
- **Cross-scenario notes**: the destination-derivation rule exercised here is reused, overridden,
  in SC-002.

### SC-002: Extract with an explicit destination override

- **Actor**: project member.
- **Preconditions**: `/uploads/report.zip` exists and is valid; `/extracted` does not yet exist.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/report.zip",
  destination="/extracted")`.
- **Postconditions**: every archive entry lands under `/extracted/...`; the archive's own
  multi-part extension is never consulted for the destination.
- **Exceptions**: none.
- **Cross-scenario notes**: `destination` is normalized through `SafetyManager::normalize_path`
  exactly like any other `fs.*` destination parameter, never through the stricter entry-escape
  check reserved for archive entries (SC-008).

### SC-003: Password-protected archive, correct password on the first call

- **Actor**: project member who already knows the password.
- **Preconditions**: `/uploads/secret.7z` is AES-256 encrypted with password `"swordfish"`.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/secret.7z",
  password="swordfish")`.
- **Postconditions**: extraction succeeds exactly as SC-001; the response contains no password
  echo.
- **Exceptions**: none.
- **Cross-scenario notes**: distinguishes from SC-004/SC-005 only by whether the password supplied
  on this one call happens to be correct; there is no multi-call state anywhere.

### SC-004: Password-protected archive, no password given, then retried

- **Actor**: project member who does not yet know a password is needed.
- **Preconditions**: `/uploads/secret.zip` is AES encrypted.
- **Flow**: call 1: `fs.extract_archive(mount_id, path="/uploads/secret.zip")`, no `password`.
  Fails `ERR_PASSWORD_REQUIRED`, message "password required to extract this archive". Call 2:
  identical, now with `password="correct"`. Succeeds.
- **Postconditions**: after call 1, nothing exists on disk under any derived or explicit
  destination. After call 2, extraction is complete exactly as SC-001.
- **Exceptions**: `ERR_PASSWORD_REQUIRED` on call 1, by design, not a failure of the system.
- **Cross-scenario notes**: the two calls are entirely independent; call 2 re-runs the full
  pre-scan, including conflict and quota checks, from scratch.

### SC-005: Retry with a wrong password

- **Actor**: project member who mistypes the password.
- **Preconditions**: same as SC-004.
- **Flow**: call 1: `password="wrong"`. Fails `ERR_PASSWORD_REQUIRED`, message "incorrect password
  for this archive". Call 2: `password="correct"`. Succeeds.
- **Postconditions**: nothing written after call 1; call 2 behaves exactly as SC-003.
- **Exceptions**: `ERR_PASSWORD_REQUIRED` on call 1, same code as SC-004, distinguished only by
  message text, because the remediation (retry with a correct password) is identical either way.
- **Cross-scenario notes**: both SC-004 and SC-005 return the same `code`; only the message
  differs, which is itself a tested contract (`FR-NEW-019`, `FR-NEW-020`).

### SC-006: Destination collision, `overwrite=false` (default)

- **Actor**: project member.
- **Preconditions**: `/uploads/report.zip` contains `a.txt` and `sub/b.txt`; `/uploads/report/
  a.txt` already exists on disk with different content.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/report.zip")`, no
  `overwrite`.
- **Postconditions**: the WHOLE operation is rejected before any write. `/uploads/report/a.txt`
  keeps its original content; `/uploads/report/sub/b.txt` is never created, even though it had no
  collision of its own.
- **Exceptions**: `ERR_NO_CLOBBER`, message naming `/uploads/report/a.txt` (the first colliding
  path in archive entry order).
- **Cross-scenario notes**: proves the all-or-nothing guarantee also holds for the conflict check,
  not only for path-safety/type rejection (SC-008/SC-009).

### SC-007: Same collision, `overwrite=true`

- **Actor**: project member.
- **Preconditions**: identical to SC-006.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/report.zip",
  overwrite=true)`.
- **Postconditions**: `/uploads/report/a.txt` now holds the archive's content;
  `/uploads/report/sub/b.txt` is created. If `/uploads/report/sub` had pre-existed as a directory,
  it is reused, not recreated or rejected.
- **Exceptions**: none.
- **Cross-scenario notes**: the only difference from SC-006 is the flag; the pre-scan pass runs
  identically in both.

### SC-008: Zip-slip entry (path-escape attempt)

- **Actor**: a maliciously or accidentally crafted archive, invoked by an otherwise ordinary
  project member.
- **Preconditions**: `/uploads/evil.zip` contains `good.txt` (benign) and
  `../../etc/passwd` (an entry path climbing above the destination).
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/evil.zip")`.
- **Postconditions**: the WHOLE extraction is rejected before any write. `good.txt`'s otherwise
  valid content is never written anywhere, proving one bad entry voids benign entries in the same
  archive.
- **Exceptions**: `ERR_PATH_OUT_OF_BOUNDS`, message naming the escaping entry
  (`../../etc/passwd`).
- **Cross-scenario notes**: tested together with SC-009 using one archive carrying both a path
  escape and a symlink entry alongside benign ones, per the cross-scenario note in the governing
  facts.

### SC-009: Symlink (or hardlink/device/special) entry

- **Actor**: same as SC-008.
- **Preconditions**: `/uploads/evil.tar` contains `good.txt` (benign, a regular file) and `link`
  (a symlink entry, tar `EntryType::Symlink`, pointing at `/etc/passwd`).
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/evil.tar")`.
- **Postconditions**: the WHOLE extraction is rejected before any write, same as SC-008.
- **Exceptions**: `ERR_NOT_SUPPORTED`, message naming the entry (`link`) and its type
  (`symlink`).
- **Cross-scenario notes**: see SC-008; also exercised against a hardlink tar entry
  (`EntryType::Link`) and, for `.zip`, an entry whose Unix mode bits in `external_attributes`
  mark it `S_IFLNK`, per `FR-NEW-013`.

### SC-010: Quota exceeded

- **Actor**: project member whose session is near its write-quota ceiling.
- **Preconditions**: `write_quota_bytes` configured low enough that the session has, say, 100
  bytes of headroom left; `/uploads/big.zip`'s entries declare 10,000 total uncompressed bytes.
- **Flow**: caller invokes `fs.extract_archive(mount_id, path="/uploads/big.zip")`.
- **Postconditions**: nothing is written anywhere; the session's `bytes_written` counter is
  numerically identical before and after the call (verified by reading it through a second,
  unrelated small write that must still succeed afterward within the original headroom).
- **Exceptions**: `ERR_WRITE_QUOTA_EXCEEDED`.
- **Cross-scenario notes**: exercises `SafetyManager::charge_write`'s fail-closed, non-mutating
  rejection (`safety.rs:134-138`), not a bespoke check this spec writes itself.

### SC-011: Unsupported or corrupt archive

- **Actor**: project member.
- **Preconditions**: two files: `/uploads/data.rar` (an unrecognized extension) and
  `/uploads/fake.zip` (a `.zip`-named file whose bytes are not a valid zip, e.g. truncated or
  plain text).
- **Flow**: caller invokes `fs.extract_archive` against each in turn.
- **Postconditions**: neither call writes anything.
- **Exceptions**: `/uploads/data.rar` → `ERR_NOT_SUPPORTED`, message naming the extension
  (`.rar`). `/uploads/fake.zip` → `ERR_INVALID_ARGUMENT`, message stating the archive is corrupt
  or not a valid zip file. These are two distinct exceptions of one scenario (neither writes
  anything), not two scenarios, per the governing facts.
- **Cross-scenario notes**: a password supplied against a tar-family archive is a related but
  separate exception, covered by `FR-NEW-008` and its own tests, not folded into SC-011 because it
  is a different precondition (format family, not corruption).

## 6. Functional Requirements

### New Requirements (FR-NEW-001..030)

**FR-NEW-001** [EARS-U] The system SHALL expose a tool named `fs.extract_archive` taking
`mount_id:string` (required), `path:string` (required, the archive's absolute POSIX path),
`destination:string` (optional), `overwrite:boolean` (optional, default `false`), and
`password:string` (optional), served identically by the MCP `#[tool]` method
`McpServer::fs_extract_archive` and the REST route `POST /api/fs/{mount_id}/extract-archive`,
both calling one `pub(crate) async fn extract_archive(state: &AppState, mount_id: &str, path:
&str, destination: Option<&str>, overwrite: bool, password: Option<&str>) -> Result<Value>` in a
new module `crates/core/src/tools/archive.rs`, mirroring `tools::export::export_zip`
(`export.rs:58-104`).

**FR-NEW-002** [EARS-E] WHEN `fs.extract_archive` is called THE system SHALL call
`state.authorize(mount_id, person)` (or the REST plane's equivalent `authorize_only`) before any
other step, exactly as every other `fs.*` tool does.

**FR-NEW-003** [EARS-E] WHEN `path` is normalized THE system SHALL use
`SafetyManager::normalize_path`, then look up the node via `client.meta.get`; IF the node does not
exist THEN THE system SHALL fail with `ERR_NOT_FOUND`, message `"'{path}' not found"`.

**FR-NEW-004** [EARS-O] IF the node at the normalized `path` is a directory THEN THE system SHALL
fail with `ERR_INVALID_ARGUMENT`, message `"'{path}' is a directory, not an archive file"`.

**FR-NEW-005** [EARS-U] The system SHALL determine the archive format from the normalized `path`'s
filename extension, matched case-insensitively, using exactly this mapping, compound suffixes
checked before the plain `.tar` suffix: `.zip` → zip; `.7z` → sevenz; `.tar.gz` or `.tgz` →
tar+gzip; `.tar.bz2` or `.tb2` → tar+bzip2; `.tar.xz` or `.txz` → tar+xz; `.tar` → tar
(uncompressed).

**FR-NEW-006** [EARS-O] IF the normalized `path`'s extension matches none of the suffixes in
`FR-NEW-005` THEN THE system SHALL fail with `ERR_NOT_SUPPORTED`, message naming the exact
extension found, e.g. `"unsupported archive extension: '.rar'"`.

**FR-NEW-007** [EARS-O] IF the archive's bytes, once read, do not parse as the format selected by
`FR-NEW-005` (corrupt, truncated, or wrong magic bytes) THEN THE system SHALL fail with
`ERR_INVALID_ARGUMENT`, message stating the archive is corrupt or not a valid file of that
format, distinct from the `ERR_NOT_SUPPORTED` of `FR-NEW-006`.

**FR-NEW-008** [EARS-O] IF the format selected by `FR-NEW-005` is `tar` in any of its four
variants (plain, gzip, bzip2, xz) AND `password` is supplied THEN THE system SHALL fail with
`ERR_INVALID_ARGUMENT`, message `"password is not applicable to this archive format"`, checked
before the archive's bytes are opened at all.

**FR-NEW-009** [EARS-U] The system SHALL list every entry of a `zip` or `sevenz` archive —
relative path, declared type (regular file / directory / other), declared uncompressed size for a
regular file, and whether the entry is password-encrypted — without requiring a correct password,
except when the format's own header encryption (an entirely header-encrypted 7z archive) makes
the entry list itself unreadable without the password, in which case `FR-NEW-019`/`FR-NEW-020`
apply to that failure instead of this listing step.

**FR-NEW-010** [EARS-O] IF any entry in a `zip` or `sevenz` archive reports itself encrypted AND
no `password` was supplied THEN THE system SHALL fail with `ERR_PASSWORD_REQUIRED`, message
`"password required to extract this archive"`, before any entry is decoded or written.

**FR-NEW-011** [EARS-O] IF any entry in a `zip` or `sevenz` archive reports itself encrypted AND
a `password` was supplied AND decoding any such entry with that password fails specifically due to
an incorrect password (as distinguished from a general corruption error by the decoding library)
THEN THE system SHALL fail with `ERR_PASSWORD_REQUIRED`, message `"incorrect password for this
archive"`.

**FR-NEW-012** [EARS-U] The system SHALL decode every regular-file entry of the archive fully into
memory during the pre-scan pass (the same fully-buffered model `fs.export_zip` uses, per
`export.rs:67-69`'s own `DEC-010`), so that password correctness, entry safety, and the full
write pass all operate on already-decoded bytes with no second decode.

**FR-NEW-013** [EARS-O] IF an entry's declared type, as reported by the format library, is a
symlink, hardlink, device, FIFO, or socket (for `tar`: any `tar::EntryType` other than `Regular` or
`Directory`; for `zip`: an entry whose Unix mode bits in `external_attributes` mark it `S_IFLNK`,
or an equivalent native symlink indicator if the pinned `zip` crate version exposes one; for `7z`:
an entry whose `sevenz_rust2` attributes mark it as a non-regular, non-directory file) THEN THE
system SHALL fail the WHOLE call with `ERR_NOT_SUPPORTED`, message naming the entry's path and its
type, e.g. `"archive entry 'link' is a symlink, which is not supported"`.

**FR-NEW-014** [EARS-O] IF an entry's path, as recorded in the archive, is absolute (begins with
`/`) OR, when split on `/` and walked component by component treating `..` as a one-level
decrease and any other non-empty, non-`.` component as a one-level increase, the running total
reaches a decrease below zero at any point, THEN THE system SHALL fail the WHOLE call with
`ERR_PATH_OUT_OF_BOUNDS`, message naming the escaping entry's path, using the same walking
technique `tools::export::ensure_no_escape` already uses (`export.rs:122-130`), applied to entry
paths rather than caller-supplied `paths`.

**FR-NEW-015** [EARS-UB] The system SHALL NOT write any byte of any entry, nor create any
directory, until every entry of the archive has passed the checks in `FR-NEW-013`, `FR-NEW-014`,
`FR-NEW-017`, and `FR-NEW-018`: a single disqualifying entry anywhere in the archive voids the
WHOLE call, even when most entries are benign.

**FR-NEW-016** [EARS-U] The system SHALL compute the extraction destination as: the normalized
form of the caller-supplied `destination` (via `SafetyManager::normalize_path`) when given;
otherwise the normalized `path` with its matched extension from `FR-NEW-005` stripped (e.g.
`/uploads/report.tar.gz` → `/uploads/report`; `/uploads/data.zip` → `/uploads/data`), preserving
the stem's original case.

**FR-NEW-017** [EARS-O] IF `overwrite` is `false` (the default) AND any entry's destination path
(the destination from `FR-NEW-016` joined with the entry's relative path, normalized) already
exists as either a file or a directory THEN THE system SHALL fail the WHOLE call with
`ERR_NO_CLOBBER`, message naming the first such colliding destination path in archive entry
order, before any write.

**FR-NEW-018** [EARS-O] IF `overwrite` is `true` THEN THE system SHALL permit writing over an
existing file at an entry's destination path and SHALL reuse (not reject, not recreate) an
existing directory at an entry's destination path.

**FR-NEW-019** [EARS-U] The system SHALL sum the declared uncompressed size (`FR-NEW-009`) of
every regular-file entry across the whole archive and SHALL charge that single total via
`SafetyManager::charge_write(person, mount_id, total)` exactly once, before any write; IF that call
returns `Err` THEN THE system SHALL fail the WHOLE call with `ERR_WRITE_QUOTA_EXCEEDED` and SHALL
NOT have mutated the session's `bytes_written` counter, relying on `charge_write`'s own
fail-closed behavior (`safety.rs:134-138`) rather than a bespoke check.

**FR-NEW-020** [EARS-UB] The system SHALL NOT write, for any regular-file entry, more decoded
bytes than that entry's declared uncompressed size (`FR-NEW-009`); IF decoding an entry produces
more bytes than declared THEN THE system SHALL fail the WHOLE call with `ERR_INVALID_ARGUMENT`,
message naming the entry and stating it decompressed to more bytes than declared.

**FR-NEW-021** [EARS-U] Once every check in `FR-NEW-003` through `FR-NEW-020` has passed, THE
system SHALL perform the write pass as follows, calling `VolumeClient` primitives directly (never
`core::fs_ops::write_bytes`, per `DEC-006`): first, `client.makedirs(destination, true)`
unconditionally (idempotent, also covers a zero-entry archive); then, for every directory entry in
archive order, `client.makedirs(entry_destination_path, true)`; then, for every regular-file entry
in archive order, `core::fs_ops::ensure_parents` (promoted `pub(crate)` by `FR-NEW-022`) followed by
`client.write_bytes_atomic(entry_destination_path, &decoded_bytes)` and
`client.touch_atime_mtime(entry_destination_path)`.

**FR-NEW-022** [EARS-U] The system SHALL promote `core::fs_ops::ensure_parents`
(`fs_ops.rs:1199-1208`) from a private `fn` to a `pub(crate) fn`, with no change to its body or
behavior, so `tools::archive` reuses it instead of duplicating its parent-directory-creation
logic.

**FR-NEW-023** [EARS-U] The system SHALL count `files_written` as the number of regular-file
entries written in `FR-NEW-021`, `dirs_created` as the number of directories created in
`FR-NEW-021` (the unconditional destination directory plus every directory entry) that did **not**
already exist immediately before this call, and `bytes_written` as the sum of decoded bytes
actually written across all regular-file entries.

**FR-NEW-024** [EARS-U] On success, THE system SHALL return `{"destination": <string, the
destination from FR-NEW-016>, "files_written": <integer>, "dirs_created": <integer>,
"bytes_written": <integer>}`.

**FR-NEW-025** [EARS-U] On success, THE system SHALL call `safety.record_audit(person, mount_id,
"extract_archive", <normalized path>, <a detail string containing the destination, files_written,
and bytes_written>)` exactly once per call, mirroring the one-audit-entry-per-operation pattern
`core::fs_ops::write_bytes` uses (`fs_ops.rs:613`).

**FR-NEW-026** [EARS-E] WHEN a call to `fs.extract_archive` succeeds THE system SHALL emit one
`tracing::info!` event carrying the fields `mount_id`, `path`, `destination`, `files_written`, and
`bytes_written`.

**FR-NEW-027** [EARS-UB] The system SHALL NOT log the `password` field, in full or in part, at any
tracing level, under any circumstance, including on failure.

**FR-NEW-028** [EARS-U] The system SHALL add a new error code `ERR_PASSWORD_REQUIRED` to
`crates/core/src/errors.rs`'s `code` module, a constructor `ToolError::password_required(m: impl
Into<String>) -> Self`, an `http_status()` match arm returning `428`, and an addition of
`code::PASSWORD_REQUIRED` to the `client_caused` array in
`no_code_falls_through_to_an_accidental_500` (`errors.rs:225-246`) and to the assertion list in
`client_caused_codes_are_all_4xx` (`errors.rs:199-221`), bringing the total `ERR_*` code count from
14 to 15.

**FR-NEW-029** [EARS-U] The system SHALL add the REST route `POST /api/fs/{mount_id}/
extract-archive` to `crates/core/src/api/dataplane.rs`'s router (alongside the existing
`export-zip` entry at `dataplane.rs:149`) and to its `REST_ROUTES` list (`dataplane.rs:87`-area),
backed by a handler that parses a JSON body `{"path": string, "destination": string|null,
"overwrite": boolean|null, "password": string|null}` and calls the same `tools::archive::
extract_archive` function as the MCP tool, named `ExtractArchiveBody` in
`crates/core/src/api/openapi.rs`'s schema list and `extract-archive` in its `Op` list, mirroring
`ExportZipBody` and the `export-zip` `Op` entry (`openapi.rs:1016-1021`, `openapi.rs:1259-1268`).

**FR-NEW-030** [EARS-U] The system SHALL regenerate `TOOL_CONTRACT.txt` and
`tool-contract-golden.json` to include `fs.extract_archive` (bringing the tool count from 149 to
150), via `MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib
tool_contract_golden_is_current`, and SHALL review the diff before committing, per `AGENTS.md`'s
own instruction for that command.

## 7. Non-Functional Requirements

### 7.1 Performance

No new performance target beyond the existing fully-buffered model `fs.export_zip` already
accepts (`DEC-010` there): this spec charges one write-quota total up front (`FR-NEW-019`), which
is the project's only existing resource-consumption guard for a bulk operation, and relies on it
rather than introducing a second, bespoke limit (entry count, nesting depth, or a time budget).

### 7.2 Security

Covered functionally throughout §6: zip-slip rejection (`FR-NEW-014`), symlink/hardlink/device
rejection (`FR-NEW-013`), all-or-nothing enforcement (`FR-NEW-015`), size-mismatch (zip-bomb
amplification) rejection (`FR-NEW-020`), and the password-never-logged prohibition (`FR-NEW-027`).
This is `Security: n/a` per the governing facts: these are hardening requirements built into a new
capability from the start, not a fix to an existing vulnerability, so the `--security` class-sweep
machinery in the shared process (S1-S3) does not apply.

### 7.3 Usability

The stateless password retry (`SC-004`/`SC-005`) is the whole usability design: no second tool,
no session state for the caller to track, just "call again with `password` filled in."

### 7.4 Reliability

All-or-nothing semantics (`FR-NEW-015`) mean a failed call never leaves partial state on disk;
there is nothing to roll back because nothing was written.

### 7.5 Observability

`FR-NEW-026` and `FR-NEW-027` are the complete observability surface for this spec: one INFO
tracing event on success with the five named fields, and an absolute prohibition on logging the
password at any level, matching this project's existing
"MUST / NEVER trace a credential, token, or secret" convention (`AGENTS.md`'s inherited `rust`
skill rules). No DB-query-level or file-mutation-level DEBUG tracing is added beyond what
`write_bytes_atomic`'s own call sites already emit, since this spec adds no new storage primitive.

### 7.6 Deployment

No change. No new infrastructure, no new config section; `fs.extract_archive` is always available
whenever the server is, exactly like `fs.export_zip`.

### 7.7 Scalability

No change. The per-session write-quota mechanism (`FR-NEW-019`) is the existing scaling guard and
this spec adds no new one.

## 8. Data Model

No new persisted schema. No new table, no new column. `fs.extract_archive` reads existing
`nodes`/`blob_refs` rows through `VolumeClient` and writes new ones through the same primitives
every other write-capable `fs.*` tool uses.

## 9. Impact Analysis

### 9.1 Affected Components

| File/Module | Impact | Description |
|---|---|---|
| `crates/core/src/tools/archive.rs` | New | `extract_archive`, format detection, per-format decode, entry safety checks. |
| `crates/core/src/tools/mod.rs` | Modified | Register the new `archive` module (mirroring how `export` is registered). |
| `crates/core/src/core/fs_ops.rs` | Modified | `ensure_parents` promoted `pub(crate)` (`FR-NEW-022`); no other change. |
| `crates/core/src/errors.rs` | Modified | New `ERR_PASSWORD_REQUIRED` code, constructor, `http_status()` arm, three exhaustiveness-test updates. |
| `crates/core/src/mcp/server.rs` | Modified | New `ExtractArchiveArgs` struct, new `#[tool] fs_extract_archive` method. |
| `crates/core/src/api/dataplane.rs` | Modified | New route registration, new `extract_archive` handler, new `REST_ROUTES` entry. |
| `crates/core/src/api/openapi.rs` | Modified | New `Op` entry, new `ExtractArchiveBody` schema entry. |
| `Cargo.toml` (workspace) | Modified | New dependencies: `tar`, `flate2`, `bzip2`, `lzma-rs`, `sevenz-rust2`; `zip`'s `features` gains `aes-crypto`. |
| `crates/core/Cargo.toml` | Modified | Workspace-true references to the five crates above. |
| `TOOL_CONTRACT.txt`, `tool-contract-golden.json` | Modified (generated) | +1 tool, 149 → 150. |
| `AGENTS.md` | Modified | Documentation index / tool-family description touch-up to mention `fs.extract_archive`; the stale "101 tools" count in the Overview section is corrected to the real, re-derived count at commit time. |

### 9.2 Affected Requirements

| Spec | Requirement ID | Impact | Description |
|---|---|---|---|
| `SPEC-0003` | (tool-family convention, not a numbered requirement) | None (followed, not changed) | This spec's `fs.extract_archive` follows the same authorize-then-normalize-then-engine shape every other `fs.*` tool in `SPEC-0003` follows. |
| `SPEC-0004` | (REST route convention, not a numbered requirement) | None (followed, not changed) | New route added under the existing `/api/fs/{mount_id}/*` shape; OpenAPI generation extended, not altered. |

No existing numbered requirement in any specification is invalidated or modified.

### 9.3 Affected Tests

| Area | Impact |
|---|---|
| `crates/core/src/tools/archive.rs` (new) | New unit/integration tests, specified in §12. |
| `crates/core/src/errors.rs` | Three existing exhaustiveness tests (`every_code_has_an_http_status`, `client_caused_codes_are_all_4xx`, `no_code_falls_through_to_an_accidental_500`) gain one more code each; they are **extended**, not rewritten, and must still pass. |
| `crates/core/src/mcp/server.rs` tests | New tests for `fs_extract_archive` through the real production handler, mirroring the existing `fs_export_zip` test block (`server.rs:4638-4650`-area). |
| `crates/core/src/api/dataplane.rs` tests | New tests for `extract-archive`, mirroring the existing `export-zip` test block (`dataplane.rs:2688-2778`-area). |
| `crates/core/src/mcp/server.rs` tool-count tests | The existing total-tool-count assertions (which already track a running count, e.g. the comments at `server.rs:3526`/`3539` bumping 36→37 for a prior spec) gain one more bump, 149→150. |

### 9.4 Affected Documentation

| File | Change |
|---|---|
| `AGENTS.md` | Tool count corrected; one-line mention of `fs.extract_archive` added to the tool-family description. |
| `TOOL_CONTRACT.txt` | Regenerated, +1 entry (`FR-NEW-030`). |
| `.agent_docs/tools.md` | New entry for `fs.extract_archive`: parameters, authorization, error codes, following the existing per-tool entry format. |
| `.agent_docs/api.md` | New entry for `POST /api/fs/{mount_id}/extract-archive`. |

### 9.5 Dependencies & Risks

**New dependencies** (all pure-Rust, no system library pulled in, verified live during this run
rather than recalled from memory):

| Crate | Version | Feature(s) | Purpose |
|---|---|---|---|
| `zip` (existing) | `2` (unchanged major) | `deflate` (existing) + `aes-crypto` (new) | AES-encrypted zip decrypt; legacy ZipCrypto decrypt needs no extra feature in zip 2.x. |
| `tar` | `0.4` | default | Pure-Rust tar reader. |
| `flate2` | `1` | default (pure-Rust `miniz_oxide` backend; no system `zlib` is pulled in by the default feature set) | `.tar.gz`/`.tgz` decompression. |
| `bzip2` | `0.6` | default (`libbz2-rs-sys`, the pure-Rust backend, confirmed as this crate's default since its 0.6.0 release) | `.tar.bz2`/`.tb2` decompression. |
| `lzma-rs` | `0.3` | default | `.tar.xz`/`.txz` decompression (LZMA/LZMA2/XZ, decode-only). |
| `sevenz-rust2` | `0.20` | `util`, `aes256` | `.7z` decompression, with `decompress_with_password`/equivalent entry decode. |

**Risk**: `sevenz-rust2` is a fork maintained to replace the now-unmaintained `sevenz-rust`
(confirmed live: `sevenz-rust` carries an advisory for being unmaintained, its repository
deleted). Pinning `sevenz-rust2` instead avoids inheriting that advisory; this is itself the
reason to pin `sevenz-rust2` and not its predecessor, recorded as `DEC-007`.

**Risk**: 7z's own format has no first-class, universally-implemented symlink representation,
so `FR-NEW-013`'s 7z branch is lower-fidelity than its tar/zip branches by construction, not by
oversight; `DEC-008` records this as an accepted, stated limitation rather than a silent gap.

**No breaking change for any existing consumer**: this spec adds one tool, one route, and one
error code; nothing existing is renamed, removed, or given new default behavior.

## 10. Documentation Requirements

- `.agent_docs/tools.md`: new `fs.extract_archive` entry (parameters, error codes, authorization),
  added at commit time, in the existing per-tool format.
- `.agent_docs/api.md`: new `POST /api/fs/{mount_id}/extract-archive` entry, same format as the
  existing `export-zip` entry.
- `AGENTS.md`: tool-family description and tool-count correction (§9.4).

## 11. Traceability Matrix

| Scenario | Functional Req | E2E Happy | E2E Failure | E2E Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001..003, 005, 009, 012, 016, 019, 021, 023, 024, 025, 026 | E2E-NEW-001 | — | E2E-NEW-002 (format matrix) |
| SC-002 | FR-NEW-016 | E2E-NEW-003 | — | E2E-NEW-051 (nested override) |
| SC-003 | FR-NEW-009, 010, 011, 012 | E2E-NEW-004, E2E-NEW-005 | — | E2E-NEW-052 (zero-byte entry) |
| SC-004 | FR-NEW-010, 019 (nothing charged) | E2E-NEW-007 (retry succeeds) | E2E-NEW-006 | E2E-NEW-057 (repeated failure is stable) |
| SC-005 | FR-NEW-011 | E2E-NEW-009 (retry succeeds) | E2E-NEW-008 | E2E-NEW-058 (empty string is wrong, not absent) |
| SC-006 | FR-NEW-017 | E2E-NEW-053 (no-collision baseline) | E2E-NEW-010, E2E-NEW-049 (dir-vs-file collision) | E2E-NEW-011 (first-colliding-path naming) |
| SC-007 | FR-NEW-018 | E2E-NEW-012 | — | E2E-NEW-013 (dir reuse), E2E-NEW-050 (no-op overwrite) |
| SC-008 | FR-NEW-014, 015 | E2E-NEW-054 (no-escape baseline) | E2E-NEW-014, E2E-NEW-040 (absolute path), E2E-NEW-041 (backslash) | E2E-NEW-015 (benign entry not written) |
| SC-009 | FR-NEW-013, 015 | E2E-NEW-055 (no-special-entry baseline) | E2E-NEW-016, E2E-NEW-017 (hardlink), E2E-NEW-018 (zip symlink) | E2E-NEW-019 (benign entry not written) |
| SC-010 | FR-NEW-019 | — | E2E-NEW-020 | E2E-NEW-021 (counter unchanged, probed via a follow-up write) |
| SC-011 | FR-NEW-006, 007, 008 | — | E2E-NEW-022, 023, 024, 034, 035, 036, 037, 038, 039 (format/corruption/password matrix) | E2E-NEW-056 (case-insensitive extension) |
| (n/a, requirement-only) | FR-NEW-004 | — | E2E-NEW-025, E2E-NEW-059 (nested dir) | E2E-NEW-060 (root `/`) |
| (n/a, requirement-only) | FR-NEW-020 | — | E2E-NEW-026, E2E-NEW-042 (tar), E2E-NEW-043 (7z) | — |
| (n/a, requirement-only) | FR-NEW-022 | — | — | E2E-NEW-027 (structural: `ensure_parents` reused, no duplicate loop), E2E-NEW-013, E2E-NEW-051 (nested parent creation exercised live) |
| (n/a, requirement-only) | FR-NEW-027 | — | E2E-NEW-028 | E2E-NEW-029, E2E-NEW-044 (field-completeness, both outcomes) |
| (n/a, requirement-only) | FR-NEW-028 | — | E2E-NEW-030 (428 mapping), E2E-NEW-045 (Display) | E2E-NEW-046 (is_client_error) |
| (n/a, requirement-only) | FR-NEW-029 | E2E-NEW-031, E2E-NEW-047 (OpenAPI) | E2E-NEW-032 | — |
| (n/a, requirement-only) | FR-NEW-030 | E2E-NEW-033, E2E-NEW-048 (golden json), E2E-NEW-061 (count bump) | — | — |

## 12. End-to-End Test Suite

### 12.1 Test Summary

> Designed in this same run (depth L calls for an independent sub-agent test designer, per the
> shared process; no sub-agent-spawning tool is available in this execution environment, so this
> table was authored by the same pass that wrote the requirements — a deviation recorded as
> `DEC-009`). A first pass under-counted per-requirement and per-scenario coverage against the
> sufficiency rules in §4.5 of the shared process; that under-count was caught by this run's own
> Phase 5.5/6 self-audit (§18) and closed by adding the tests below, numerically verified rather
> than eyeballed, rather than left as a registered gap.

| Test ID | Action | Category | Scenario | FR refs | Priority |
|---|---|---|---|---|---|
| E2E-NEW-001 | Extract `.tar.gz` with default destination | Happy | SC-001,SC-010,SC-011 | 001,002,003,005,009,012,016,019,021,022,023,024,025,026 | P0 |
| E2E-NEW-002 | Extract `.tgz`, `.tb2`, `.txz`, `.tar`, `.zip`, `.7z` each, default destination | Edge | SC-001 | 001,002,003,005 | P0 |
| E2E-NEW-003 | Extract `.zip` with explicit `destination` override | Happy | SC-002 | 001,002,003,016 | P0 |
| E2E-NEW-004 | Extract AES-encrypted `.7z` with correct password | Happy | SC-003 | 001,002,003,009,010,011,012,016,021,023,024,025,026 | P0 |
| E2E-NEW-005 | Extract AES-encrypted and legacy ZipCrypto `.zip` with correct password | Happy | SC-003 | 001,002,003,009,012,016,021,023,024,025,026 | P0 |
| E2E-NEW-006 | Encrypted archive, no password | Failure | SC-004 | 001,002,003,010 | P0 |
| E2E-NEW-007 | Retry after E2E-NEW-006 with correct password succeeds | Happy | SC-004 | 001,002,003,010,019 | P1 |
| E2E-NEW-008 | Encrypted archive, wrong password | Failure | SC-005 | 001,002,003,011 | P0 |
| E2E-NEW-009 | Retry after E2E-NEW-008 with correct password succeeds | Happy | SC-005 | 001,002,003,011 | P1 |
| E2E-NEW-010 | Destination collision, `overwrite` omitted (defaults false) | Failure | SC-006 | 001,002,003,017 | P0 |
| E2E-NEW-011 | Collision naming: message names first colliding path in archive order | Edge | SC-006 | 001,002,003,017 | P1 |
| E2E-NEW-012 | Same collision, `overwrite=true` | Happy | SC-007 | 001,002,003,016,018,021,023,024,025,026 | P0 |
| E2E-NEW-013 | `overwrite=true` against a pre-existing directory entry | Edge | SC-007 | 001,002,003,018,022 | P1 |
| E2E-NEW-014 | Archive with one zip-slip entry and one benign entry | Failure | SC-008 | 001,002,003,014,015 | P0 |
| E2E-NEW-015 | Same archive: benign entry's content never appears anywhere on disk | Edge | SC-008 | 001,002,003,015 | P0 |
| E2E-NEW-016 | Archive with a tar symlink entry and one benign entry | Failure | SC-009 | 001,002,003,013,015 | P0 |
| E2E-NEW-017 | Archive with a tar hardlink entry | Failure | SC-009 | 001,002,003,013 | P1 |
| E2E-NEW-018 | Zip entry with Unix mode `S_IFLNK` | Failure | SC-009 | 001,002,003,013 | P1 |
| E2E-NEW-019 | Symlink-carrying archive: benign entry's content never appears anywhere on disk | Edge | SC-009 | 001,002,003,015 | P0 |
| E2E-NEW-020 | Archive total declared size exceeds remaining quota | Failure | SC-010 | 001,002,003,019 | P0 |
| E2E-NEW-021 | After E2E-NEW-020's rejection, a small unrelated write within original headroom still succeeds | Edge | SC-010 | 001,002,003,019 | P0 |
| E2E-NEW-022 | `.rar` extension | Failure | SC-011 | 001,002,003,006 | P0 |
| E2E-NEW-023 | `.zip`-named file with non-zip bytes | Failure | SC-011 | 001,002,003,007 | P0 |
| E2E-NEW-024 | Password supplied against a `.tar.gz` archive | Failure | SC-011 | 001,002,003,008 | P0 |
| E2E-NEW-025 | `path` points at a directory, not a file | Failure | (FR-only) | 001,002,003,004 | P1 |
| E2E-NEW-026 | Crafted zip entry whose decoded bytes exceed its declared size | Failure | (FR-only) | 001,002,003,009,012,020 | P1 |
| E2E-NEW-027 | Structural: `tools::archive` calls `core::fs_ops::ensure_parents`, no duplicate loop | Edge | (FR-only) | 022 | P2 |
| E2E-NEW-028 | Failing call (wrong password) logs no literal password string | Failure | (FR-only) | 001,002,003,011,027 | P0 |
| E2E-NEW-029 | Succeeding call's log line also contains no literal password string | Edge | (FR-only) | 001,002,003,009,012,026,027 | P0 |
| E2E-NEW-030 | `ERR_PASSWORD_REQUIRED` maps to HTTP 428 on the REST plane | Failure | (FR-only) | 028 | P0 |
| E2E-NEW-031 | REST `POST extract-archive` happy path | Happy | (FR-only) | 001,002,003,005,016,021,023,024,025,026,029 | P0 |
| E2E-NEW-032 | REST route: malformed body is 400; unauthenticated/stranger matches `fs.read` | Failure | (FR-only) | 001,002,029 | P0 |
| E2E-NEW-033 | `TOOL_CONTRACT.txt` lists `fs.extract_archive` with the exact schema | Happy | (FR-only) | 001,030 | P1 |
| E2E-NEW-034 | Bare `.gz` (not `.tar.gz`) is an unsupported extension | Failure | SC-011 | 001,002,003,006 | P1 |
| E2E-NEW-035 | Multi-volume-style `.zip.001` is an unsupported extension (Non-Goal) | Failure | SC-011 | 001,002,003,006 | P1 |
| E2E-NEW-036 | Truncated `.7z` is corrupt | Failure | SC-011 | 001,002,003,007 | P1 |
| E2E-NEW-037 | Corrupt gzip header inside a `.tar.gz` is corrupt | Failure | SC-011 | 001,002,003,007 | P1 |
| E2E-NEW-038 | Password supplied against a plain `.tar` | Failure | SC-011 | 001,002,003,008 | P1 |
| E2E-NEW-039 | Password supplied against a `.tar.bz2` | Failure | SC-011 | 001,002,003,008 | P1 |
| E2E-NEW-040 | Zip entry with an absolute path `/etc/passwd` | Failure | SC-008 | 001,002,003,014,015 | P1 |
| E2E-NEW-041 | Backslash-style traversal entry, mirroring `export.rs`'s own windows-style test | Failure | SC-008 | 001,002,003,014 | P1 |
| E2E-NEW-042 | Tar entry whose header size is smaller than its actual content | Failure | (FR-only) | 001,002,003,020 | P2 |
| E2E-NEW-043 | 7z entry size-mismatch | Failure | (FR-only) | 001,002,003,020 | P2 |
| E2E-NEW-044 | Tracing event has exactly the five named fields; password absent on success and failure | Edge | (FR-only) | 001,002,003,011,026,027 | P1 |
| E2E-NEW-045 | `ToolError::password_required` Display format | Failure | (FR-only) | 028 | P2 |
| E2E-NEW-046 | `is_client_error()` is true for `password_required` | Edge | (FR-only) | 028 | P2 |
| E2E-NEW-047 | OpenAPI includes the `extract-archive` path and `ExtractArchiveBody` schema | Happy | (FR-only) | 001,029 | P1 |
| E2E-NEW-048 | `tool-contract-golden.json` also includes `fs.extract_archive` | Happy | (FR-only) | 030 | P1 |
| E2E-NEW-049 | No-clobber conflict against a pre-existing directory (not a file) | Failure | SC-006 | 001,002,003,017 | P1 |
| E2E-NEW-050 | `overwrite=true` with zero actual collisions is a no-op success | Edge | SC-007 | 001,002,003,018 | P2 |
| E2E-NEW-051 | Destination override pointing at a nested, not-yet-existing path two levels deep | Edge | SC-002 | 001,002,003,016 | P1 |
| E2E-NEW-052 | AES zip with a zero-byte entry, password correct | Edge | SC-003 | 001,002,003,009,012 | P2 |
| E2E-NEW-053 | Same archive shape as E2E-NEW-010 but with no pre-existing collision succeeds | Happy | SC-006 | 001,002,003,017 | P1 |
| E2E-NEW-054 | Same archive shape as E2E-NEW-014 but with no escaping entries succeeds | Happy | SC-008 | 001,002,003,014,015 | P1 |
| E2E-NEW-055 | Same archive shape as E2E-NEW-016 but with no special entries succeeds | Happy | SC-009 | 001,002,003,013,015 | P1 |
| E2E-NEW-056 | Extension matched case-insensitively (`.ZIP` uppercase) succeeds | Edge | SC-011 | 001,002,003,005 | P1 |
| E2E-NEW-057 | Second wrong-password retry after E2E-NEW-006 still fails identically | Edge | SC-004 | 001,002,003,010 | P2 |
| E2E-NEW-058 | Empty-string password is treated as wrong, not as absent | Edge | SC-005 | 001,002,003,011 | P1 |
| E2E-NEW-059 | `path` points at a directory nested two levels deep | Failure | (FR-only) | 001,002,003,004 | P2 |
| E2E-NEW-060 | `path` is `/` itself (the volume root), rejected the same way | Edge | (FR-only) | 001,002,003,004 | P2 |
| E2E-NEW-061 | The running tool-count assertion in `mcp/server.rs` is bumped 149→150 | Happy | (FR-only) | 030 | P2 |

**Coverage statistics**: 61 tests. By category: Happy = 15, Failure = 29, Edge = 17. **Happy :
Failure ratio = 15 : 29 ≈ 1 : 1.93, which beats 1:1.** Every one of the 30 requirements has at
least 3 tests referencing its id, verified by script against this exact table (minimum observed:
3, for `FR-NEW-004`, `FR-NEW-028`, and `FR-NEW-030`). Every scenario has at least `1 happy, 1
failure, 1 edge` present in the matrix wherever that scenario's own §5 Exceptions field names a
real failure mode; for the four scenarios whose own Exceptions field reads "none" (SC-001,
SC-002, SC-003, SC-007), no dedicated Failure-category row is tagged to that scenario specifically,
by the scenario's own definition — their failure modes are the cross-cutting ones (bad format,
quota, auth) already owned by SC-004/005/006/008/009/010/011, and duplicating those under every
scenario that could theoretically also trip them would be the padding the shared process
forbids. Per-scenario totals: SC-001=2, SC-002=2, SC-003=3, SC-004=3, SC-005=3, SC-006=4,
SC-007=3, SC-008=5, SC-009=5, SC-010=3, SC-011=11. Several of these sit below the shared process's
generic "5 per scenario" design guidance (`§4.5`); `DEC-011` records why, for this spec's
narrow, single-mechanism scenarios, that generic guidance is satisfied by the happy/failure/edge
presence check instead, rather than by inventing scenario-specific failure modes that do not
exist.

**Modified/removed tests**: none (feature does not exist today, §2.3).

### 12.2 New Test Specifications

**E2E-NEW-001** — Category: Happy. Scenario: SC-001. Requirements: FR-NEW-001,002,003,005,009,
012,016,019,021,022,023,024,025,026. Driver: `tools::archive::extract_archive`, called directly
(unit-level) and once more through `McpServer::fs_extract_archive` (integration-level) in the same
test module, mirroring `export.rs`'s own dual-level pattern. Preconditions: fixture volume `proj`
owned by `owner@test.com`; a `.tar.gz` archive built in the test (via the `tar`+`flate2` crates
themselves, so the fixture never depends on a binary test asset) containing `a.txt` = `b"hello"`
and `sub/b.txt` = `b"world"`, written to `/uploads/report.tar.gz`. Steps: **Given** the fixture
above, **When** `extract_archive(&state, "proj", "/uploads/report.tar.gz", None, false, None)` is
awaited, **Then** the result is `Ok` with `destination == "/uploads/report"`, `files_written == 2`,
`dirs_created == 1`, `bytes_written == 10` (5+5), **And** `client.read_bytes("/uploads/report/
a.txt").await.unwrap() == b"hello"`, **And** `client.read_bytes("/uploads/report/sub/
b.txt").await.unwrap() == b"world"`. Cleanup: fixture dropped at end of test (in-memory/tempdir
backend). Priority: P0.

**E2E-NEW-002** — Category: Edge. Scenario: SC-001. Requirements: FR-NEW-005. Driver:
`tools::archive::extract_archive`, parameterized over the six format family members. Preconditions:
one fixture per format, each containing one entry `only.txt = b"x"`, built with the matching
crate (`zip`, `sevenz_rust2`, `tar` plain, `tar`+`flate2`, `tar`+`bzip2`, `tar`+`lzma_rs`) at
`/u/f.<ext>` for each of `.zip`, `.7z`, `.tar`, `.tgz`, `.tb2`, `.txz`. Steps: **Given** each
fixture in turn, **When** `extract_archive` is called with no destination, **Then** the result is
`Ok` **And** `/u/f/only.txt` exists with content `b"x"` for every one of the six. Cleanup: as
above. Priority: P0.

**E2E-NEW-003** — Category: Happy. Scenario: SC-002. Requirements: FR-NEW-016. Preconditions: a
`.zip` at `/uploads/report.zip` with one entry `x.txt = b"y"`; `/extracted` does not exist.
Steps: **Given** the fixture, **When** `extract_archive(.., path="/uploads/report.zip",
destination=Some("/extracted"), ..)`, **Then** `destination == "/extracted"` **And**
`/extracted/x.txt` exists with content `b"y"` **And** `/uploads/report` (the would-be default) does
not exist. Priority: P0.

**E2E-NEW-004** — Category: Happy. Scenario: SC-003. Requirements: FR-NEW-009,010,011,012.
Preconditions: a `.7z` built with `sevenz_rust2`'s AES-256 password encryption, password
`"swordfish"`, one entry `s.txt = b"secret"`, at `/uploads/secret.7z`. Steps: **Given** the
fixture, **When** `extract_archive(.., password=Some("swordfish"), ..)`, **Then** `Ok` **And**
`/uploads/secret/s.txt == b"secret"`. Priority: P0.

**E2E-NEW-005** — Category: Happy. Scenario: SC-003. Requirements: FR-NEW-009,012. Preconditions:
two zips: one AES-encrypted (via `zip`'s `aes-crypto` feature), one legacy-ZipCrypto-encrypted,
both password `"p"`, one entry `z.txt = b"z"` each. Steps: for each, **Given** the fixture,
**When** `extract_archive(.., password=Some("p"), ..)`, **Then** `Ok` **And** the entry's content
matches. Priority: P0.

**E2E-NEW-006** — Category: Failure. Scenario: SC-004. Requirements: FR-NEW-010. Preconditions: an
AES-encrypted `.zip` at `/uploads/secret.zip`, password `"correct"`, one entry. Steps: **Given**
the fixture, **When** `extract_archive(.., password=None, ..)`, **Then** `Err` with `code ==
"ERR_PASSWORD_REQUIRED"` **And** `message == "password required to extract this archive"` **And**
nothing exists under `/uploads/secret` (verified via `client.exists`). Priority: P0.

**E2E-NEW-007** — Category: Happy. Scenario: SC-004. Requirements: FR-NEW-010,019. Preconditions:
continuation of E2E-NEW-006 in the same test, same fixture state (no write happened). Steps:
**Given** the state right after E2E-NEW-006's failure, **When** `extract_archive(.., password=
Some("correct"), ..)`, **Then** `Ok` **And** the entry exists with its expected content. Priority:
P1.

**E2E-NEW-008** — Category: Failure. Scenario: SC-005. Requirements: FR-NEW-011. Preconditions:
same fixture shape as E2E-NEW-006, password `"correct"`. Steps: **Given** the fixture, **When**
`extract_archive(.., password=Some("wrong"), ..)`, **Then** `Err` with `code ==
"ERR_PASSWORD_REQUIRED"` **And** `message == "incorrect password for this archive"` **And**
nothing exists under the destination. Priority: P0.

**E2E-NEW-009** — Category: Happy. Scenario: SC-005. Requirements: FR-NEW-011. Preconditions:
continuation of E2E-NEW-008. Steps: **Given** the state right after E2E-NEW-008's failure, **When**
`extract_archive(.., password=Some("correct"), ..)`, **Then** `Ok` **And** the entry exists with
its expected content. Priority: P1.

**E2E-NEW-010** — Category: Failure. Scenario: SC-006. Requirements: FR-NEW-017. Preconditions: a
`.zip` at `/uploads/report.zip` with entries `a.txt = b"new"` and `sub/b.txt = b"new2"`; prior
state has `/uploads/report/a.txt = b"old"` already written (different content), `/uploads/report/
sub` does not yet exist. Steps: **Given** the fixture, **When** `extract_archive(..,
overwrite=false, ..)` (or omitted), **Then** `Err` with `code == "ERR_NO_CLOBBER"` **And**
`message` contains `"/uploads/report/a.txt"` **And** `client.read_bytes("/uploads/report/
a.txt").await.unwrap() == b"old"` (unchanged) **And** `client.exists("/uploads/report/sub/
b.txt").await.unwrap() == false`. Priority: P0.

**E2E-NEW-011** — Category: Edge. Scenario: SC-006. Requirements: FR-NEW-017. Preconditions: same
as E2E-NEW-010 but the archive's entry order is `sub/b.txt` first, `a.txt` second, and
`/uploads/report/sub/b.txt` is the pre-existing collision instead. Steps: as above. **Then** the
error message names `/uploads/report/sub/b.txt`, not `/uploads/report/a.txt`, proving "first
colliding path in archive entry order" rather than lexicographic or any other order. Priority:
P1.

**E2E-NEW-012** — Category: Happy. Scenario: SC-007. Requirements: FR-NEW-018. Preconditions:
identical to E2E-NEW-010. Steps: **Given** the fixture, **When** `extract_archive(..,
overwrite=true, ..)`, **Then** `Ok` **And** `/uploads/report/a.txt == b"new"` **And**
`/uploads/report/sub/b.txt == b"new2"`. Priority: P0.

**E2E-NEW-013** — Category: Edge. Scenario: SC-007. Requirements: FR-NEW-018. Preconditions: a
`.zip` with a directory entry `sub/` and a file `sub/c.txt = b"c"`; `/uploads/report/sub` already
exists as a directory (created independently, not by this archive) with an unrelated file
`/uploads/report/sub/other.txt` already inside it. Steps: **Given** the fixture, **When**
`extract_archive(.., overwrite=true, ..)`, **Then** `Ok` **And** `/uploads/report/sub/other.txt`
still exists (the directory was reused, not recreated/emptied) **And** `/uploads/report/sub/
c.txt == b"c"`. Priority: P1.

**E2E-NEW-014** — Category: Failure. Scenario: SC-008. Requirements: FR-NEW-014,015.
Preconditions: a `.tar` containing `good.txt = b"benign"` and an entry literally named
`../../etc/passwd` with content `b"pwned"`, at `/uploads/evil.tar`. Steps: **Given** the fixture,
**When** `extract_archive(..)`, **Then** `Err` with `code == "ERR_PATH_OUT_OF_BOUNDS"` **And**
`message` contains `"../../etc/passwd"`. Priority: P0.

**E2E-NEW-015** — Category: Edge. Scenario: SC-008. Requirements: FR-NEW-015. Preconditions: same
as E2E-NEW-014. Steps: **Given** the fixture, **When** `extract_archive(..)` fails, **Then**
`client.exists("/uploads/evil/good.txt").await.unwrap() == false` **And** a direct walk of the
volume's blob store confirms the literal content `"pwned"` was never written anywhere
(verification channel: blob-store walk, not the tool's own response). Priority: P0.

**E2E-NEW-016** — Category: Failure. Scenario: SC-009. Requirements: FR-NEW-013,015.
Preconditions: a `.tar` built with the `tar` crate's symlink-entry API, containing `good.txt =
b"benign"` and a symlink entry `link` → `/etc/passwd`, at `/uploads/evil.tar`. Steps: **Given**
the fixture, **When** `extract_archive(..)`, **Then** `Err` with `code == "ERR_NOT_SUPPORTED"`
**And** `message` contains `"link"` and `"symlink"`. Priority: P0.

**E2E-NEW-017** — Category: Failure. Scenario: SC-009. Requirements: FR-NEW-013. Preconditions:
same shape as E2E-NEW-016 but the special entry is a tar hardlink (`EntryType::Link`) instead.
Steps: as E2E-NEW-016, **Then** `message` names the entry and a type naming it as a hardlink (not
the word "symlink"). Priority: P1.

**E2E-NEW-018** — Category: Failure. Scenario: SC-009. Requirements: FR-NEW-013. Preconditions: a
`.zip` with one entry whose `external_attributes` encode Unix mode `0o120777` (`S_IFLNK`), content
being a link-target string, named `link`, plus a benign `good.txt`. Steps: **Given** the fixture,
**When** `extract_archive(..)`, **Then** `Err` with `code == "ERR_NOT_SUPPORTED"` **And**
`message` contains `"link"`. Priority: P1.

**E2E-NEW-019** — Category: Edge. Scenario: SC-009. Requirements: FR-NEW-015. Preconditions: same
as E2E-NEW-016. Steps: as E2E-NEW-015's verification, applied to this fixture: the benign entry's
content never appears anywhere on disk after the failure. Priority: P0.

**E2E-NEW-020** — Category: Failure. Scenario: SC-010. Requirements: FR-NEW-019. Preconditions:
`ServerConfig.safety.write_quota_bytes` set to `100` in the fixture's config; a `.zip` at
`/uploads/big.zip` with one entry declaring (and actually containing) `10_000` bytes. Steps:
**Given** the fixture, **When** `extract_archive(..)`, **Then** `Err` with `code ==
"ERR_WRITE_QUOTA_EXCEEDED"` **And** `client.exists("/uploads/big/<entry>").await.unwrap() ==
false`. Priority: P0.

**E2E-NEW-021** — Category: Edge. Scenario: SC-010. Requirements: FR-NEW-019. Preconditions:
continuation of E2E-NEW-020, same session. Steps: **Given** the state right after E2E-NEW-020's
failure, **When** a small, unrelated `fs.write(bytes=50)` call is made in the same session,
**Then** it succeeds (`50 <= 100` headroom), proving the failed extraction charged nothing.
Priority: P0.

**E2E-NEW-022** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-006. Preconditions: a
file at `/uploads/data.rar` (any bytes; extension alone drives this check). Steps: **Given** the
fixture, **When** `extract_archive(..)`, **Then** `Err` with `code == "ERR_NOT_SUPPORTED"` **And**
`message` contains `".rar"`. Priority: P0.

**E2E-NEW-023** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-007. Preconditions: a
file at `/uploads/fake.zip` containing the plain-text bytes `b"not a zip file at all"`. Steps:
**Given** the fixture, **When** `extract_archive(..)`, **Then** `Err` with `code ==
"ERR_INVALID_ARGUMENT"` **And** `message` states the archive is corrupt or not a valid zip.
Priority: P0.

**E2E-NEW-024** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-008. Preconditions: a
valid, unencrypted `.tar.gz` at `/uploads/plain.tar.gz`. Steps: **Given** the fixture, **When**
`extract_archive(.., password=Some("anything"), ..)`, **Then** `Err` with `code ==
"ERR_INVALID_ARGUMENT"` **And** `message == "password is not applicable to this archive format"`.
Priority: P0.

**E2E-NEW-025** — Category: Failure. Requirements: FR-NEW-004. Preconditions: `/uploads/adir`
exists as a directory. Steps: **Given** the fixture, **When** `extract_archive(..,
path="/uploads/adir", ..)`, **Then** `Err` with `code == "ERR_INVALID_ARGUMENT"` **And**
`message` contains `"'/uploads/adir' is a directory"`. Priority: P1.

**E2E-NEW-026** — Category: Failure. Requirements: FR-NEW-020. Preconditions: a hand-crafted zip
(built via the `zip` crate's low-level writer, patched post-write) whose central-directory entry
declares `uncompressed_size = 1` for an entry that actually decompresses to a larger buffer, OR
a unit-level test that calls the internal decode-and-verify step with a mocked declared size of
`1` against real decoded bytes of length `10`. Steps: **Given** the mismatch, **When**
`extract_archive(..)`, **Then** `Err` with `code == "ERR_INVALID_ARGUMENT"` **And** `message`
names the entry and states it decompressed to more bytes than declared. Priority: P1.

**E2E-NEW-027** — Category: Edge (structural). Requirements: FR-NEW-022. Driver: a `#[test]`
(non-async, no fixture) reading `crates/core/src/tools/archive.rs`'s source text. Steps: **Given**
the file at `crates/core/src/tools/archive.rs`, **When** read as a string, **Then** it contains the
literal substring `ensure_parents` (proving reuse) **And** it does not define a function whose body
re-implements a parent-directory-walk loop (grepped for the literal substring `rfind('/')`
combined with a `makedirs` call outside of a call to `ensure_parents` — absence asserted).
Priority: P2.

**E2E-NEW-028** — Category: Failure. Requirements: FR-NEW-027. Preconditions: a `tracing_test`-
style subscriber (or an in-memory `tracing` layer collecting formatted event text) installed for
the duration of the test; the E2E-NEW-008 fixture (wrong password `"wrong"` against a password
`"correct"` archive). Steps: **Given** the subscriber, **When** `extract_archive(..,
password=Some("wrong"), ..)` fails, **Then** none of the collected log lines contain the literal
substring `"wrong"`. Priority: P0.

**E2E-NEW-029** — Category: Edge. Requirements: FR-NEW-027. Preconditions: same subscriber
mechanism, the E2E-NEW-004 fixture (correct password `"swordfish"`). Steps: **Given** the
subscriber, **When** `extract_archive(.., password=Some("swordfish"), ..)` succeeds, **Then** none
of the collected log lines contain the literal substring `"swordfish"`. Priority: P0.

**E2E-NEW-030** — Category: Failure. Requirements: FR-NEW-028. Driver: `crates/core/src/
errors.rs`'s own test module, extended. Steps: **Given** `ToolError::password_required("x")`,
**Then** `.http_status() == 428` **And** `.code == "ERR_PASSWORD_REQUIRED"` **And** it is present
in both `client_caused_codes_are_all_4xx`'s array and
`no_code_falls_through_to_an_accidental_500`'s array (both existing tests still pass, extended).
Priority: P0.

**E2E-NEW-031** — Category: Happy. Requirements: FR-NEW-029. Driver: `crates/core/src/api/
dataplane.rs`'s own REST test harness (the same `h.post`/fixture helpers the `export-zip` tests
use). Preconditions: same fixture shape as E2E-NEW-001, served over the REST plane. Steps:
**Given** the fixture, **When** `h.post(&u("extract-archive"), json!({"path": "/uploads/
report.tar.gz"}))`, **Then** HTTP 200 **And** the JSON body matches `{"destination": "/uploads/
report", "files_written": 2, "dirs_created": 1, "bytes_written": 10}`. Priority: P0.

**E2E-NEW-032** — Category: Failure. Requirements: FR-NEW-029. Preconditions: as E2E-NEW-031.
Steps: (a) **Given** a body missing `path`, **When** posted, **Then** HTTP 400. (b) **Given** no
bearer token, **When** posted, **Then** the same status and `error` code `fs.read` would return
under the same condition (cross-checked through the real router, not hardcoded, mirroring
`e2e_new_041_unauthenticated_call_matches_other_tools` in `export.rs`). (c) **Given** a bearer for
a person who is not a member of the project, **When** posted, **Then** `ERR_FORBIDDEN`'s status.
Priority: P0.

**E2E-NEW-033** — Category: Happy (structural). Requirements: FR-NEW-030. Driver: a `#[test]`
reading `TOOL_CONTRACT.txt`. Steps: **Given** the regenerated file, **When** searched for the line
`fs.extract_archive`, **Then** it is present, with a `params:` line listing `mount_id:string,
path:string, destination:string=null, overwrite:boolean=false, password:string=null` (exact
parameter list and defaults matching `FR-NEW-001`). Priority: P1.

**E2E-NEW-034** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-006. Preconditions: a
file at `/uploads/data.gz` (plain gzip, not a tar). Steps: **Given** the fixture, **When**
`extract_archive(..)`, **Then** `Err` with `code == "ERR_NOT_SUPPORTED"` **And** `message`
contains `".gz"`. Priority: P1.

**E2E-NEW-035** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-006. Preconditions: a
file at `/uploads/parts.zip.001` (a split-archive-style name). Steps: **Given** the fixture,
**When** `extract_archive(..)`, **Then** `Err` with `code == "ERR_NOT_SUPPORTED"` **And**
`message` contains `".001"`. Priority: P1.

**E2E-NEW-036** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-007. Preconditions: a
valid `.7z` whose last 200 bytes are truncated before being written to `/uploads/broken.7z`.
Steps: **Given** the fixture, **When** `extract_archive(..)`, **Then** `Err` with `code ==
"ERR_INVALID_ARGUMENT"` **And** `message` states the archive is corrupt. Priority: P1.

**E2E-NEW-037** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-007. Preconditions: a
valid `.tar.gz` whose first four bytes (the gzip magic) are overwritten with `b"\x00\x00\x00\x00"`
before being written to `/uploads/broken.tar.gz`. Steps: as E2E-NEW-036. Priority: P1.

**E2E-NEW-038** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-008. Preconditions: a
valid, unencrypted plain `.tar` at `/uploads/plain.tar`. Steps: **Given** the fixture, **When**
`extract_archive(.., password=Some("x"), ..)`, **Then** `Err` with `code ==
"ERR_INVALID_ARGUMENT"` **And** `message == "password is not applicable to this archive format"`.
Priority: P1.

**E2E-NEW-039** — Category: Failure. Scenario: SC-011. Requirements: FR-NEW-008. Preconditions: a
valid, unencrypted `.tar.bz2` at `/uploads/plain.tar.bz2`. Steps: as E2E-NEW-038. Priority: P1.

**E2E-NEW-040** — Category: Failure. Scenario: SC-008. Requirements: FR-NEW-014,015.
Preconditions: a `.zip` with entries `good.txt = b"benign"` and an entry whose stored name is
literally `/etc/passwd` (absolute). Steps: **Given** the fixture, **When** `extract_archive(..)`,
**Then** `Err` with `code == "ERR_PATH_OUT_OF_BOUNDS"` **And** `message` contains `"/etc/passwd"`
**And** `good.txt`'s content is never written anywhere. Priority: P1.

**E2E-NEW-041** — Category: Failure. Scenario: SC-008. Requirements: FR-NEW-014. Preconditions: a
`.zip` with one entry whose stored name is `..\\..\\windows\\system32\\evil.dll`, mirroring
`tools::export::ensure_no_escape`'s own windows-style test case
(`export.rs`'s `e2e_new_009_traversal_attempts_are_out_of_bounds`). Steps: **Given** the fixture,
**When** `extract_archive(..)`, **Then** `Err` with `code == "ERR_PATH_OUT_OF_BOUNDS"`. Priority:
P1.

**E2E-NEW-042** — Category: Failure. Requirements: FR-NEW-020. Preconditions: a `.tar` entry whose
header declares `size = 1` but whose actual tar data block (padded to the 512-byte boundary, as
the tar format requires) is hand-crafted to make the reader yield more than 1 byte for that entry.
Steps: **Given** the fixture, **When** `extract_archive(..)`, **Then** `Err` with `code ==
"ERR_INVALID_ARGUMENT"` **And** `message` names the entry and states a size mismatch. Priority:
P2.

**E2E-NEW-043** — Category: Failure. Requirements: FR-NEW-020. Preconditions: a `.7z` entry whose
folder-header declares a smaller uncompressed size than its actual decompressed output (hand-
crafted via `sevenz_rust2`'s low-level writer, patched post-write). Steps: as E2E-NEW-042.
Priority: P2.

**E2E-NEW-044** — Category: Edge. Requirements: FR-NEW-011,026,027. Preconditions: the subscriber
mechanism of E2E-NEW-028/029, run against both the E2E-NEW-001 (success) and E2E-NEW-008
(failure, wrong password `"wrong"`) fixtures. Steps: **Given** each fixture in turn, **When**
`extract_archive` completes, **Then** the collected structured fields on the relevant event are
exactly `mount_id`, `path`, `destination`, `files_written`, `bytes_written` for the success case
(`FR-NEW-026`) — no `password` field key present at all, not merely an empty one — **And** no
event at any level carries a `password` key for the failure case either. Priority: P1.

**E2E-NEW-045** — Category: Failure. Requirements: FR-NEW-028. Driver: `errors.rs`'s own test
module. Steps: **Given** `ToolError::password_required("bad guess")`, **Then**
`.to_string() == "ERR_PASSWORD_REQUIRED: bad guess"`, mirroring the existing
`display_is_code_colon_message` test's exact assertion style. Priority: P2.

**E2E-NEW-046** — Category: Edge. Requirements: FR-NEW-028. Driver: `errors.rs`'s own test module.
Steps: **Given** `ToolError::password_required("x")`, **Then** `.is_client_error() == true`,
added to the existing `client_caused_codes_are_all_4xx` test's array rather than as a freestanding
assertion, mirroring how every other client-caused code is checked there. Priority: P2.

**E2E-NEW-047** — Category: Happy. Requirements: FR-NEW-029. Driver: a `#[test]` calling the
OpenAPI spec-generation function directly (the same one `GET /api/swagger.json` serves). Steps:
**Given** the generated spec, **When** inspected, **Then** it contains a path entry for
`/api/fs/{mount_id}/extract-archive` with method `POST` **And** a schema named
`ExtractArchiveBody` with required field `path` and optional fields `destination`, `overwrite`,
`password`. Priority: P1.

**E2E-NEW-048** — Category: Happy. Requirements: FR-NEW-030. Driver: a `#[test]` reading
`tool-contract-golden.json` (the machine-checked twin of `TOOL_CONTRACT.txt`). Steps: **Given**
the regenerated file, **When** parsed as JSON and searched for a tool named `fs.extract_archive`,
**Then** it is present with an `inputSchema` whose required array is `["mount_id", "path"]`.
Priority: P1.

**E2E-NEW-049** — Category: Failure. Scenario: SC-006. Requirements: FR-NEW-017. Preconditions: a
`.zip` with one entry `d/` (a directory entry) and `d/x.txt = b"x"`; `/uploads/report/d` already
exists on disk as a **file** (not a directory), from an unrelated prior write. Steps: **Given**
the fixture, **When** `extract_archive(.., overwrite=false, ..)`, **Then** `Err` with `code ==
"ERR_NO_CLOBBER"` **And** `message` contains `/uploads/report/d` **And** nothing under
`/uploads/report` changes. Priority: P1.

**E2E-NEW-050** — Category: Edge. Scenario: SC-007. Requirements: FR-NEW-018. Preconditions: a
`.zip` with entries `a.txt = b"a"` and `b.txt = b"b"`; destination `/uploads/report` does not
exist at all (nothing to collide with). Steps: **Given** the fixture, **When**
`extract_archive(.., overwrite=true, ..)`, **Then** `Ok` **And** both files exist with their
expected content, proving `overwrite=true` changes nothing about the happy path when there is
genuinely no collision. Priority: P2.

**E2E-NEW-051** — Category: Edge. Scenario: SC-002. Requirements: FR-NEW-016. Preconditions: a
`.zip` with one entry `x.txt = b"y"`; `destination = "/a/b/c"`, none of `/a`, `/a/b`, `/a/b/c`
exist yet. Steps: **Given** the fixture, **When** `extract_archive(.., destination=
Some("/a/b/c"), ..)`, **Then** `Ok` **And** `/a/b/c/x.txt == b"y"` **And** `dirs_created >= 1`
for the destination itself (intermediate parents `/a` and `/a/b` are created by
`client.makedirs`'s own recursive behavior, not counted individually against `dirs_created`, which
counts only the archive's own directory-shaped entries plus the destination root per
`FR-NEW-023`). Priority: P1.

**E2E-NEW-052** — Category: Edge. Scenario: SC-003. Requirements: FR-NEW-009,012. Preconditions:
an AES-encrypted `.zip`, password `"p"`, with one entry `empty.txt` of zero bytes. Steps:
**Given** the fixture, **When** `extract_archive(.., password=Some("p"), ..)`, **Then** `Ok`
**And** `client.read_bytes(".../empty.txt").await.unwrap() == b""` **And** `bytes_written == 0`
**And** `files_written == 1`. Priority: P2.

**E2E-NEW-053** — Category: Happy. Scenario: SC-006. Requirements: FR-NEW-017. Preconditions: the
same `.zip` shape as E2E-NEW-010 (`a.txt`, `sub/b.txt`) but `/uploads/report` does not exist at
all beforehand (no collision). Steps: **Given** the fixture, **When** `extract_archive(..,
overwrite=false, ..)` (default), **Then** `Ok`, proving the conflict-detection machinery itself
does not misfire when nothing actually collides. Priority: P1.

**E2E-NEW-054** — Category: Happy. Scenario: SC-008. Requirements: FR-NEW-014,015. Preconditions:
the same `.tar` shape as E2E-NEW-014 (`good.txt`) but with no `../../etc/passwd`-style entry at
all. Steps: **Given** the fixture, **When** `extract_archive(..)`, **Then** `Ok` **And**
`good.txt`'s content is written normally, proving the path-safety check itself does not misfire
on an entirely benign archive. Priority: P1.

**E2E-NEW-055** — Category: Happy. Scenario: SC-009. Requirements: FR-NEW-013,015. Preconditions:
the same `.tar` shape as E2E-NEW-016 (`good.txt`) but with no symlink entry at all. Steps:
**Given** the fixture, **When** `extract_archive(..)`, **Then** `Ok`, proving the entry-type check
itself does not misfire on an archive containing only regular files. Priority: P1.

**E2E-NEW-056** — Category: Edge. Scenario: SC-011. Requirements: FR-NEW-005. Preconditions: a
valid zip archive at `/uploads/REPORT.ZIP` (uppercase extension), one entry `x.txt = b"y"`.
Steps: **Given** the fixture, **When** `extract_archive(.., path="/uploads/REPORT.ZIP", ..)`,
**Then** `Ok` **And** the format is correctly resolved as `zip` despite the uppercase extension,
per `FR-NEW-005`'s case-insensitive matching clause. Priority: P1.

**E2E-NEW-057** — Category: Edge. Scenario: SC-004. Requirements: FR-NEW-010. Preconditions:
continuation of E2E-NEW-006 (no password given, failed). Steps: **Given** the state right after
E2E-NEW-006's failure, **When** `extract_archive(.., password=None, ..)` is called a second time,
identically, **Then** it fails identically (`ERR_PASSWORD_REQUIRED`, same message), proving the
failure is stateless and repeatable rather than drifting on retry. Priority: P2.

**E2E-NEW-058** — Category: Edge. Scenario: SC-005. Requirements: FR-NEW-011. Preconditions: the
E2E-NEW-008 fixture (password `"correct"`). Steps: **Given** the fixture, **When**
`extract_archive(.., password=Some(""), ..)` (an explicit empty string, not `None`), **Then**
`Err` with `code == "ERR_PASSWORD_REQUIRED"` **And** `message == "incorrect password for this
archive"` (the wrong-password message, not the missing-password one), because an explicitly
supplied empty string is a supplied password that happens to be wrong, distinct from `None`.
Priority: P1.

**E2E-NEW-059** — Category: Failure. Requirements: FR-NEW-004. Preconditions: `/uploads/a/b`
exists as a directory, two levels deep. Steps: **Given** the fixture, **When**
`extract_archive(.., path="/uploads/a/b", ..)`, **Then** `Err` with `code ==
"ERR_INVALID_ARGUMENT"` **And** `message` contains `"'/uploads/a/b' is a directory"`. Priority:
P2.

**E2E-NEW-060** — Category: Edge. Requirements: FR-NEW-004. Preconditions: the project volume's
root `/` (always a directory). Steps: **Given** the fixture, **When** `extract_archive(..,
path="/", ..)`, **Then** `Err` with `code == "ERR_INVALID_ARGUMENT"`, the same rejection as any
other directory path, proving the root is not special-cased. Priority: P2.

**E2E-NEW-061** — Category: Happy. Requirements: FR-NEW-030. Driver: `crates/core/src/mcp/
server.rs`'s own running tool-count test (the one whose comments already track prior bumps, e.g.
`server.rs:3526`/`3539`). Steps: **Given** the test's existing assertion of the total tool count,
**When** `fs.extract_archive` is added, **Then** the asserted total is `150`, with a comment
dated to this spec mirroring the existing comment style for prior bumps. Priority: P2.

## 13. Consistency Notes

- The response key is `destination`, matching the parameter name `destination`, never
  `extracted_to` or any other alias: one name, one meaning, throughout.
- `ERR_PASSWORD_REQUIRED` is reused verbatim for both the missing-password and wrong-password
  cases (`FR-NEW-010`/`FR-NEW-011`); nothing in this spec invents a second code for the same
  remediation.
- Every "whole call rejected" requirement (`FR-NEW-013`, `FR-NEW-014`, `FR-NEW-017`, `FR-NEW-019`,
  `FR-NEW-020`) is phrased identically ("the WHOLE call") so an implementer cannot read one as
  entry-scoped and another as archive-scoped by accident.

## 14. Migration & Implementation Notes

Suggested implementation order, because two requirements would break each other if built in the
wrong sequence:

1. `FR-NEW-028` (new error code) first: every later requirement's test needs `ERR_PASSWORD_REQUIRED`
   to exist and compile.
2. `FR-NEW-022` (`ensure_parents` promoted `pub(crate)`) next: a small, isolated, zero-behavior-
   change diff, safe to land and test alone before `tools/archive.rs` depends on it.
3. `FR-NEW-005` through `FR-NEW-012` (format detection and decode) before `FR-NEW-013` through
   `FR-NEW-020` (entry safety, conflict, quota): the safety/conflict/quota checks all operate on
   the decoded entry list the earlier group produces.
4. `FR-NEW-021` (write pass) only after every check in step 3 is in place and tested, per
   `FR-NEW-015`'s ordering guarantee.
5. `FR-NEW-001`/`002`/`029` (the two transport doors) last, once `extract_archive` itself is fully
   tested in isolation, mirroring how `export_zip` was clearly built bottom-up (its own doc
   comment states the typed function is called by both doors, not the reverse).
6. `FR-NEW-030` (contract regeneration) absolutely last, after every other change, since it is
   generated from the finished tool definition.

## 15. Open Questions & TBDs

None. Every ambiguity surfaced during drafting (destination derivation, conflict semantics, quota
charging point, password error-code reuse, symlink detection per format, zip-bomb size-mismatch
handling) was resolved into a numbered requirement rather than left open, per the governing
decisions already made and the additional decisions recorded below.

## 16. Glossary

| Term | Definition | Context |
|---|---|---|
| Zip-slip | An archive entry whose path, when extracted naively, climbs outside the intended destination directory (e.g. via `../` components or an absolute path), letting an attacker overwrite arbitrary files. | `FR-NEW-014`, SC-008. |
| ZipCrypto | The legacy, weak password-encryption scheme built into the original zip format, as opposed to the stronger AES extension. | `FR-NEW-009`, `FR-NEW-011`. |
| Declared uncompressed size | The size of an entry's decompressed content as recorded in the archive's own metadata (zip central directory, 7z folder headers, tar header), read before decoding. | `FR-NEW-009`, `FR-NEW-019`, `FR-NEW-020`. |
| All-or-nothing | This spec's guarantee that a disqualifying condition anywhere in an archive voids the entire extraction, with no partial write. | `FR-NEW-015`, SC-006, SC-008, SC-009. |
| Pre-scan pass | The in-memory phase (format detection, entry decode, safety checks, conflict checks, quota charge) that completes entirely before the write pass begins. | §6, `FR-NEW-012`, `FR-NEW-015`, `FR-NEW-021`. |

## 17. Decisions Log

- **DEC-001** [Round 1]: RAR is entirely out of scope, not deferred, no backlog entry. Rationale:
  explicit user instruction, "Remove completely rar, this was nice to have, not a requirement."
  Alternatives considered: deferring RAR to a backlog entry (rejected — user said "remove
  completely", not "defer"). Implemented by: §3.2 (Non-Goals).
- **DEC-002** [Round 1]: no stateful paused-operation table; password handling is fully stateless,
  a missing/wrong password is `ERR_PASSWORD_REQUIRED` and the caller retries the same call.
  Rationale: avoids a new relational table and a `continue`/`abort` tool pair for a condition the
  caller can resolve by itself with no server-side memory. Alternatives considered: a
  `git_operations`-style paused-row mechanism (rejected as unnecessary complexity for a
  single-parameter retry). Implemented by: FR-NEW-010, FR-NEW-011.
- **DEC-003** [Round 1]: `destination` defaults to the archive's path with its recognized
  multi-part extension stripped. Rationale: predictable, matches the common "extract here" mental
  model without requiring the caller to name a destination for the common case. Alternatives
  considered: always requiring an explicit `destination` (rejected as needless friction).
  Implemented by: FR-NEW-016.
- **DEC-004** [Round 1]: conflict handling defaults to `overwrite=false`, rejecting the WHOLE
  call on any collision. Rationale: silent partial overwrite is the more dangerous default;
  an explicit `overwrite=true` is the deliberate, informed choice. Alternatives considered:
  per-entry overwrite negotiation (rejected — adds a third knob for no clear benefit over an
  all-or-nothing boolean). Implemented by: FR-NEW-017, FR-NEW-018.
- **DEC-005** [Round 1]: quota is charged once, as the archive's total declared uncompressed size,
  in the pre-scan pass, rather than per-entry during the write pass. Rationale: a zip-bomb-style
  archive must be rejected before any byte lands on disk, not after partially writing. Alternatives
  considered: per-entry charging during the write pass (rejected — would let a large early entry
  succeed before a later one triggers the quota failure, breaking all-or-nothing). Implemented by:
  FR-NEW-019.
- **DEC-006** [Round 2, this run]: the write pass calls `VolumeClient::write_bytes_atomic` and
  `client.makedirs` directly, never `core::fs_ops::write_bytes`. Rationale: `core::fs_ops::
  write_bytes` calls `safety.charge_write` internally (`fs_ops.rs:609`); since this spec already
  charges the whole archive's total once up front (`FR-NEW-019`), routing every per-entry write
  through `core::fs_ops::write_bytes` too would charge twice and could spuriously fail mid-
  extraction after some files already landed, which breaks the all-or-nothing guarantee this spec
  is built around. This mirrors `tools::export::export_zip`'s own precedent of calling
  `client.read_bytes`/`client.blob.put` directly rather than through `core::fs_ops`, for files that
  do not fit that layer's per-call quota-charging shape. Alternatives considered: refunding the
  per-entry charge immediately after each `core::fs_ops::write_bytes` call (rejected — needless
  churn on the session counter for no behavioral gain, and a window where the counter briefly
  overshoots). Implemented by: FR-NEW-019, FR-NEW-021.
- **DEC-007** [Round 2, this run]: pin `sevenz-rust2`, not `sevenz-rust`. Rationale: `sevenz-rust`
  carries an unmaintained-crate advisory with its repository deleted (confirmed via live search
  during this run); `sevenz-rust2` is the actively maintained fork. Alternatives considered: none
  genuinely competitive — `sevenz-rust2` is the documented successor. Implemented by: §9.5
  dependency table.
- **DEC-008** [Round 2, this run]: 7z symlink/hardlink/device detection is accepted as
  lower-fidelity than the tar and zip branches of `FR-NEW-013`, because the 7z format has no
  universally-implemented symlink representation. Rationale: stated honestly rather than invented
  false precision; the risk is low because 7z archives carrying POSIX symlinks are rare in
  practice and any entry this branch cannot positively identify as a symlink is instead written as
  an ordinary regular file, which is a safe (non-destination-escaping) default already covered by
  `FR-NEW-014`'s independent path-safety check. Alternatives considered: refusing to support `.7z`
  extraction at all until a library-level symlink-detection guarantee exists (rejected — out of
  proportion to the actual risk, given the independent zip-slip guard already in place).
  Implemented by: FR-NEW-013, §9.5 Risks.
- **DEC-009** [Round 2, this run]: this run's Phase 4.0 test design and Phase 5.5/6 gates were
  executed by the same agent and context that wrote the requirements, rather than by a
  fresh-context sub-agent, because no sub-agent-spawning tool is available in this execution
  environment (the available tool set is bash/edit/find/grep/read/write/ls plus MCP proxies; no
  `Task`-equivalent). Rationale: the shared process mandates a fresh-context spawn at depth L for
  exactly the bias this deviation reintroduces (the author of the requirements also designing the
  tests, and auditing their own implementability). Mitigation taken: the test table was built
  mechanically against the explicit sufficiency rules (every requirement ≥3 tests, every scenario
  ≥5, failure:happy beating 1:1) and counted rather than eyeballed (§12.1), and the Phase 6 audit
  below (§18) was performed as a distinct, deliberately skeptical re-read of the finished document
  from disk, approximating (not replacing) a fresh-context pass. This is recorded as a process
  deviation, not hidden. Implemented by: §12, §18.
- **DEC-010** [Round 2, this run]: the quota charge in `FR-NEW-019` uses each entry's **declared**
  uncompressed size, not the actual decoded length, with a separate guard (`FR-NEW-020`) rejecting
  any entry whose actual decoded length exceeds its declared size. Rationale: this is the governing
  facts' own instruction ("sums every entry's declared uncompressed size"), and it additionally
  closes a size-mismatch zip-bomb-amplification vector a naive "charge actual decoded size" design
  would still be vulnerable to during the decode step itself (decoding happens before the quota
  check in `FR-NEW-012`/`FR-NEW-019`'s ordering, so an entry lying about its declared size to look
  small, then decoding to something huge, must be caught independently of the quota charge).
  Alternatives considered: charging actual decoded size instead (rejected — contradicts the
  explicit governing instruction and does not, by itself, bound decode-time memory use before the
  charge even runs). Implemented by: FR-NEW-019, FR-NEW-020.
- **DEC-011** [Round 2, this run]: for this spec's 11 usage scenarios, each a narrow,
  single-mechanism parameter or safety check rather than a multi-step end-user journey, the
  shared process's generic "5 tests per scenario" design guidance (§4.5) is satisfied by the
  presence of a happy, a failure, and an edge test wherever that scenario's own §5 Exceptions
  field names a real failure mode, rather than by inventing scenario-specific failure modes that
  do not exist for scenarios whose own Exceptions field reads "none" (SC-001, SC-002, SC-003,
  SC-007). Rationale: this run's own self-audit (§18) first found several requirements and
  scenarios under the sufficiency minimums and closed the requirement-level gaps exhaustively
  (every FR-NEW-XXX now has ≥3 tests, verified by script against the file on disk); duplicating
  the cross-cutting failure modes (bad format, quota, wrong auth) under every scenario that could
  theoretically also trip them, just to hit a literal "5" on narrow scenarios like SC-002
  ("destination override"), would be exactly the padding the shared process elsewhere forbids.
  Alternatives considered: inventing a bespoke failure mode per narrow scenario (rejected —
  fabricated, not real); leaving the counts low and registering the shortfall as an A finding
  (rejected — this is a test-plan sufficiency question the author can and did resolve directly,
  not a code/spec alignment question an A finding is for). Implemented by: §12.1.

## 18. Implementability Gate

> No sub-agent-spawning tool is available in this execution environment (`DEC-009`). The audit
> below was performed as a distinct, skeptical, cold re-read of the finished document and the
> cited source lines, applying the depth-S inline checklist (shared process §6.4) in full as the
> closest available approximation of the mandated fresh-context sub-agent audit, since no literal
> fresh-context spawn was possible. This is a recorded deviation, not a silent downgrade.

| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 2 | IMPLEMENTABLE-WITH-DRIFT |

**Round 1 findings:**

- **A finding 1**: `FR-NEW-009`'s claim that zip entry metadata (name, size, encryption flag) is
  readable without a password is asserted but not verified against the pinned `zip` crate's exact
  API at this run's effort level (no `cargo doc` build of the exact pinned version was performed;
  only a web search confirming the `aes-crypto` feature's existence). Code does: unknown until the
  dependency is actually added and `cargo doc` or the crate's docs.rs page is read against the
  pinned `2.x` version string resolved at `cargo update` time. Nature: unverified capability
  claim, not yet contradicted by any citation. **Detected by**: `cargo build` will fail to compile
  against a non-existent API immediately if the assumption is wrong, and `E2E-NEW-002`/
  `E2E-NEW-005` will fail if entry metadata actually requires the password in the pinned version.
  Resolution during implementation: if the pinned `zip` version's encrypted-entry metadata read
  does require the password, `FR-NEW-009`'s "without requiring a correct password" clause for zip
  specifically is false for that version, and the implementer must either pin a version where it
  holds or fold the zip branch into the same "password required to even list" path 7z's header-
  encryption case already covers in `FR-NEW-009`'s own carve-out clause — no new requirement is
  needed, the existing carve-out already covers this exact failure mode.
- **A finding 2**: `FR-NEW-013`'s zip-symlink detection names `S_IFLNK` bit `0xA000` informally in
  §9.5/E2E-NEW-018 prose but the exact `external_attributes` bit-shift (Unix mode lives in the
  high 16 bits per the Info-ZIP convention: `(external_attributes >> 16) & 0xFFFF`) is stated in
  `FR-NEW-013` itself but not independently re-derived against the pinned `zip` crate's actual
  field name/type at this effort level. Code does: unknown until the dependency is added. Nature:
  unverified capability claim. **Detected by**: `E2E-NEW-018` is red if the bit-extraction is
  wrong, and `cargo build` fails if the field name assumed does not exist on the pinned version's
  `ZipFile`/`ZipFileData` type.

Both A findings are the same shape: a claim about a not-yet-added dependency's exact API surface,
made at the behavioral level with a named fallback or carve-out already written into the
requirement that would absorb the finding if the primary mechanism turns out to be wrong. Neither
is a product decision two implementers could resolve differently — both are "read the crate's
actual docs.rs page once it is vendored," which is ordinary implementation work with a compiler
and a dedicated test as the oracle, exactly the Q2 (alignment) shape, not Q1 (functional). No F
finding exists: every scenario, every error code, every response field, and every ordering
constraint is pinned to a specific decision with no two-implementer divergence left.

**Amendments applied**: none required — both A findings already resolve through an existing
requirement's own stated fallback/carve-out, so no textual amendment to the requirements
themselves was needed; they are registered below for the implementer's benefit rather than left
unlabeled.

**Drift registered**: DRIFT-001, DRIFT-002 (§19).

## 19. Implementation Drift Register

- **DRIFT-001**
  - Spec says: `FR-NEW-009` — zip entry metadata (name, declared size, encryption flag) is
    readable from a `zip::ZipArchive` without a correct password.
  - Code does: unknown; the `zip` crate is not yet a dependency with the `aes-crypto` feature
    enabled in this tree (confirmed absent, §2.1/§9.5). Must be verified against the exact pinned
    version once added.
  - Nature: missing capability (unverified until the dependency lands).
  - Resolution during implementation: read the pinned version's docs.rs page (or run
    `cargo doc --open -p mcp-fs-core` locally) for `ZipArchive`/`ZipFile` before writing the
    decode loop; if metadata truly requires the password for an AES-encrypted entry, treat that
    exactly like 7z's header-encryption carve-out already written into `FR-NEW-009` — no new
    requirement needed, just route that case through `FR-NEW-010`/`FR-NEW-011` at the "can't even
    list" point instead of the "can list, can't decode" point.
  - Detected by: `cargo build` (API mismatch) or `E2E-NEW-002`/`E2E-NEW-005` (wrong runtime
    behavior).
  - Blocks which requirement: FR-NEW-009, FR-NEW-010, FR-NEW-011.
  - Status: not-reproducible (drift_001_zip_entry_metadata_readable_without_password green on first run: zip 2.4.2 exposes name, size and encrypted flag without a password via ZipArchive::by_index_raw, read.rs:1097; confirmed by the user 2026-10-09).
- **DRIFT-002**
  - Spec says: `FR-NEW-013` — a zip entry's symlink-ness is readable via Unix mode bits in
    `external_attributes`, high 16 bits, `S_IFLNK`.
  - Code does: unknown; same not-yet-a-dependency situation as DRIFT-001.
  - Nature: missing capability (unverified until the dependency lands).
  - Resolution during implementation: read the pinned version's `ZipFileData`/`ZipFile` field
    names for `external_attributes` or any native `is_symlink()` method it may already expose
    (the crate's release history suggests this was added in some 2.x release; use it directly if
    present, falling back to the bit-mask approach only if not).
  - Detected by: `E2E-NEW-018` (wrong runtime behavior) or `cargo build` (API mismatch).
  - Blocks which requirement: FR-NEW-013.
  - Status: resolved (E2E-NEW-018 red before, green after; `ZipFile::is_symlink()` at
    zip-2.4.2/src/read.rs:1746-1749, built on `unix_mode()` at zip-2.4.2/src/types.rs:555-562;
    SPEC-0015_US-0007).
