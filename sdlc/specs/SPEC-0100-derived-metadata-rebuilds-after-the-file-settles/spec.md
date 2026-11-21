# Search entries are rebuilt after the file stops changing: functional specification

> Id: SPEC-0100
> Nature: FEAT
> Depth: L
> Depth evidence: FEAT / depth L: 2 areas (search indexing, the new waiting and job mechanism) plus the tool surface, project lifecycle and server settings, 20 requirements, pending work now survives a restart
> Status: designed
> Generated on: 2026-11-20
> Project: existing
> From backlog: BL-0174
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
The search entries of a file are today rebuilt at once on every write, in the background, with no way to see, order or survive a restart. This increment rebuilds them only after the file has been quiet for a settle delay, so a file saved many times is rebuilt once. A project member can also rebuild a file's entries at once, and can see what is waiting, running, done or failed. The mechanism is built to carry other kinds of derived data later: converted documents (BL-0176) and chunk sets (BL-0168); git operations and archive extraction join as triggers in BL-0177; the metadata screen is BL-0175.

## 2. Current State (MANDATORY, every depth)
Nothing waits, queues or remembers pending work. Every write of an indexed file starts its indexing at once in the background, and two quick writes to one file are not coalesced. A server restart forgets work in progress. Nobody can see what is happening. The tool `search.index` indexes one path at once, by hand, without any record. Every member of a project can read and write every file of it: there is no read only role.

### 2.1 What users can do today
- Write, edit or patch a file: if the project indexes its files, the new content becomes searchable shortly after, with no way to see or order it (14.1 rows 1 to 3).
- Delete, move, copy or restore a file: its search entries follow (14.1 row 4).
- Index one path by hand with `search.index` (14.1 row 5).
- Files created by a git operation or an archive extraction are not indexed until written again (BL-0177).

### 2.2 Specs governing this area
- SPEC-0008/FR-017: a write never waits for the index; the business rule says the index lags the volume by one backend call. The "never waits" part is kept (FR-NEW-004); the lag rule is modified (FR-MOD-001).
- SPEC-0008/FR-018: an index failure never fails the write. Kept (FR-NEW-012).
- SPEC-0008/E2E-023: the write answers well under 2 s, the new chunks are not counted at once, they are counted after 5 s. Deviation: counted after the settle delay (E2E-MOD-001, E2E-MOD-002).

### 2.3 What is verified today
Journeys exercised by existing tests: a write followed by a search finds the content, a delete empties it, a failing embedding service does not fail the write, volumes are isolated. No test exercises a delay, a restart, a rebuild by hand or a job listing (14.1 row 6).

## 3. Scope
### 3.1 In scope
- A settle delay, one value for the server, after which the search entries of a changed file are rebuilt.
- One kind of derived data now: `search_index`.
- Changes that schedule it: write, edit, patch, upload, copy, restore from the trash.
- Waiting rebuilds that survive a server restart.
- A tool to rebuild a file's entries at once, and a tool to list jobs.
- Job states `waiting`, `running`, `done`, `failed`, `superseded`.
- Cancelling or moving jobs when a file is deleted or moved, the index mode changes, or a project is deleted or restored.

### 3.2 Out of scope, and why
- Converted documents (companion, images, tables) rebuilt after their source changes: BL-0176, because the origin of a companion, in request conversion and quota charging need their own decisions.
- Git operations and archive extraction as triggers: BL-0177, because they call no indexing hook today.
- Chunk sets as a kind: BL-0168.
- A per project delay: arrives with the per project profile (BL-0170).
- A marker "out of date" on every read tool answer: a waiting job already says so, and the metadata screen (BL-0175) shows it.
- A maximum wait: the owner wants a file that never stops changing to stay untouched.
- Automatic retry of failed rebuilds: a person triggers it.
- A read only role: none exists; every member can rebuild and list.

### 3.3 Constraints imposed by the context
- The rule that a write never waits for derived data stays (SPEC-0008/FR-017), imposed by the project.
- Authorization is project membership, as for every file tool.

## 4. Actors
| Actor | Role |
|---|---|
| Member | member of a project: writes files, rebuilds their entries by hand, lists jobs |
| Non member | a signed in person who is not a member of the project: refused |
| Owner | a member who owns the project |
| Operator | sets the settle delay and starts and restarts the server |
| Platform admin | creates, deletes and restores projects, sets index modes of any project |

## 4.5 Bounded contexts (MANDATORY at M and L)
| Context | What it owns | Key concepts |
|---|---|---|
| Derived data | what is built from a file and when | kind, rebuild, settle delay, job, state |
| Search | the searchable entries of a file | project index mode, search entries |

## 5. Usage scenarios
### SC-001: a file saved many times is rebuilt once
**Actor:** Member
**Preconditions:** project `proj` indexes its files; settle delay 30 seconds; Alice (`alice@example.com`) is a member; `/notes.md` exists and is indexed.
**Flow:**
1. Alice saves `/notes.md` at 10:00:00 → the write succeeds at once; a rebuild of its search entries is waiting.
2. She saves it again at 10:00:20 and 10:00:40 → each save succeeds at once; the wait restarts each time; still one waiting rebuild.
3. She stops. At 10:01:10 (30 seconds after the last save) → the rebuild starts and ends.
**Postconditions:** search finds the last content of `/notes.md`; the job list shows one `done` job for `search_index`.
**Exceptions:**
- EXC-001a: the index service is down at 10:01:10 → the job is `failed` with a reason; the earlier entries stay (SC-007).

### SC-002: a delay of zero keeps today's behavior
**Actor:** Operator, Member
**Preconditions:** settle delay is 0; `/notes.md` indexed.
**Flow:**
1. Alice saves the file → the write succeeds at once and the rebuild starts right after, without waiting.
**Postconditions:** the job ends `done` shortly after; no job stays `waiting`.
**Exceptions:**
- EXC-002a: the delay setting is negative or not a whole number → the server refuses to start.

### SC-003: a copy or a restore schedules its destination
**Actor:** Member
**Preconditions:** project indexes its files; settle delay 30 seconds.
**Flow:**
1. Alice copies `/a.md` to `/a2.md` → the copy succeeds at once; a rebuild for `/a2.md` waits; none for `/a.md`.
2. She restores `/d.md` from the trash → a rebuild for `/d.md` waits.
**Postconditions:** after the delay both destinations are searchable.
**Exceptions:**
- EXC-003a: the project does not index its files → nothing is scheduled.

### SC-004: delete and move while rebuilds wait or run
**Actor:** Member
**Flow:**
1. Alice saves `/a.md` (a rebuild waits), then deletes it → no rebuild runs; every job of `/a.md` is removed and its entries are removed as today.
2. Alice saves `/b.md` (a rebuild waits), then moves it to `/c.md` → the waiting rebuild now targets `/c.md` with the time already waited.
**Postconditions:** no job names `/a.md` or `/b.md`; one waiting job names `/c.md`.
**Exceptions:**
- EXC-004a: the rebuild is running at the delete or the move → it ends `superseded` and its result is discarded.

### SC-005: the server restarts
**Actor:** Operator
**Flow:**
1. Alice saves `/notes.md`; a rebuild waits with 20 seconds left; the operator restarts the server → the rebuild is still listed as `waiting`, with the full 30 seconds left.
2. After 30 quiet seconds → the rebuild runs.
**Exceptions:**
- EXC-005a: a rebuild was running at the restart → its job is `failed` with `interrupted by a server restart`, and a new waiting job exists.

### SC-006: Alice rebuilds by hand and looks at jobs
**Actor:** Member
**Flow:**
1. A rebuild of `/notes.md` is waiting. Alice asks for a rebuild of `search_index` for `/notes.md` → the waiting one is cancelled, a job starts at once, the answer names the job and the state `running` (or `done` when it finished).
2. She lists the jobs of `/notes.md` → she sees the job, its kind, state, cause `manual`, and when it ended.
**Postconditions:** the file has at most one `waiting` job and at most one `running` job at a time.
**Exceptions:**
- EXC-006a: she asks for a kind that does not exist → refused with `ERR_INVALID_ARGUMENT`.
- EXC-006b: the person is not a member → refused with `ERR_FORBIDDEN`; the project does not exist → `ERR_PROJECT_NOT_FOUND`.
- EXC-006c: the file does not exist → refused with `ERR_NOT_FOUND`.
- EXC-006d: the project does not index its files → refused with `ERR_NOT_SUPPORTED`.

### SC-007: a rebuild fails
**Actor:** Member, Operator
**Flow:**
1. The rebuild of `/notes.md` runs and fails (index service down) → the job is `failed` with the reason `embedding service unreachable`; the write that caused it is unaffected; the earlier search entries stay.
2. Alice saves again once the service is back → a new job runs and ends `done`.

### SC-008: the file changes while its rebuild runs
**Actor:** Member
**Flow:**
1. A rebuild of `/notes.md` is running. Alice saves the file → the write succeeds; the running rebuild ends but its result is not kept; its job becomes `superseded`; a new waiting job exists.
**Postconditions:** after the next quiet period the content of the last save is indexed.

### SC-009: reading while a rebuild waits or runs
**Actor:** Member
**Flow:**
1. Alice saves `/notes.md`; a rebuild waits. She searches → she gets the previous entries, unchanged. She lists jobs → the job is `waiting` with the time left. When the rebuild ends, the entries are replaced in one step.

### SC-010: the index mode or the project changes
**Actor:** Platform admin, Member
**Flow:**
1. A rebuild waits; the project's index mode changes → the waiting rebuild is removed and the index follows today's mode change.
2. A project with jobs is deleted, then restored → no job remains.

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: a change schedules a rebuild after a quiet period
> WHEN the content of a file is created or replaced by a write, an edit, a patch, an upload, a copy or a restore from the trash THE system SHALL schedule one rebuild of its search entries, starting after the settle delay with no further change.

- **Business rules:** Kind `search_index` applies when the file's project indexes its files (the project's index mode is not `none`); otherwise nothing is scheduled.
- **Business rules:** A copy and a restore schedule the destination path only. A move and a delete are not changes of content (FR-NEW-005, FR-NEW-006).
- **Business rules:** A waiting job created by this requirement, by FR-NEW-006 or by FR-NEW-017 has cause `change`.
- **Business rules:** Not in this increment: git operations and archive extraction (BL-0177), converted documents (BL-0176). The tool `search.index` is unchanged: it indexes one path at once, without a job.
- **Scenario:** SC-001, SC-003. **Priority:** Must

#### FR-NEW-002 [EARS-E]: each change restarts the wait
> WHEN a file changes THE system SHALL restart the wait of its waiting rebuild, keeping one waiting rebuild per file.

- **Business rules:** The waiting job keeps its `job_id` and its `created_at` (the time of the first change of the wait); only `seconds_left` restarts.
- **Scenario:** SC-001, SC-008. **Priority:** Must

#### FR-NEW-003 [EARS-S]: a delay of zero rebuilds right after the change
> WHILE the settle delay is 0 THE system SHALL start the rebuild of a changed file right after the change, leaving no waiting job.

- **Business rules:** When a rebuild of that file is already running, the change supersedes it at its end (FR-NEW-013) and the next rebuild starts as soon as the running one has ended; it is not listed as `waiting` in the meantime, unlike a rebuild by hand (FR-NEW-008).
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-004 [EARS-UB]: a write never waits for a rebuild
> The system SHALL NOT make a write, edit, patch, move, copy or delete wait for a rebuild, a waiting rebuild or a failed rebuild.

- **Business rules:** Keeps SPEC-0008/FR-017.
- **Scenario:** SC-001, SC-002, SC-007. **Priority:** Must

#### FR-NEW-005 [EARS-E]: deleting a file removes its jobs
> WHEN a file is deleted THE system SHALL cancel its waiting rebuild, end its running rebuild as `superseded` with its result discarded, and remove every job of that path, finished ones included.

- **Business rules:** Derived data of the deleted file is removed as today. Deleting a folder applies this to every file under it. A job removed while running leaves no record when its computation ends. The running job is listed as `superseded` from the moment of the delete until it is removed with its computation.
- **Scenario:** SC-004, SC-007. **Priority:** Must

#### FR-NEW-006 [EARS-E]: moving a file moves its jobs
> WHEN a file is moved THE system SHALL make its waiting rebuild name the new path with the time already waited, end its running rebuild as `superseded` with its result discarded and schedule a new waiting rebuild with the full settle delay, and make its finished jobs name the new path.

- **Business rules:** Moving a folder applies this to every file under it. When the move replaces an existing file, every job of the replaced path is removed first, as in FR-NEW-005. A running rebuild is listed as `superseded` from the moment of the move, its computation may continue but its result is discarded; with a settle delay of 0 the new rebuild starts as soon as that computation has stopped. The search entries of the moved file are not searchable under the old path from the moment the move answers, and are searchable under the new path within 5 seconds of it; this creates no job. With a settle delay of 0 the new rebuild starts right after the move and is not listed as `waiting`.
- **Scenario:** SC-004. **Priority:** Must

#### FR-NEW-007 [EARS-U]: waiting rebuilds survive a restart
> The system SHALL keep every waiting rebuild across a server restart, each restarting with the full settle delay.

- **Scenario:** SC-005. **Priority:** Must

#### FR-NEW-008 [EARS-E]: rebuild by hand
> WHEN a member of a project asks for the rebuild of a kind for a file of that project THE system SHALL cancel the waiting rebuild of that file, create a job at once, and answer with the job id, the path, the kind and the state.

- **Business rules:** The tool is `fs.recalculate` with `mount_id`, `path` and `kind` (only `search_index` in this increment). Each call creates a new job.
- **Business rules:** The state in the answer is the state at the moment of the answer; a caller must not assume `running`. When no rebuild of that file is running, the new job starts at once: `state` is `running`, or `done` or `failed` (with `reason`) when it had already ended; the call is not an error. When one is running, the running job ends `superseded` and the answer names the new job in state `waiting` with `seconds_left` 0; it starts as soon as the running one has ended. At most one job of a file is `running`.
- **Scenario:** SC-006. **Priority:** Must

#### FR-NEW-009 [EARS-O]: refusals of a rebuild by hand
> IF the request is not allowed or not applicable THEN THE system SHALL refuse it, start nothing, and change no waiting rebuild.

- **Business rules:** Order of the refusals: project absent or deleted, not a member, search not enabled for the server, kind unknown, path absent, path is a folder, indexing off.
- **Business rules:** Exact texts: project absent or deleted `ERR_PROJECT_NOT_FOUND` (the existing text); not a member `ERR_FORBIDDEN: '<person>' is not a member of '<mount_id>'`; search not enabled for the server `ERR_NOT_SUPPORTED: search is not enabled; set search.enabled: true in config`; kind unknown `ERR_INVALID_ARGUMENT: kind must be 'search_index'`; path absent `ERR_NOT_FOUND: file not found: <path>`; path is a folder `ERR_INVALID_ARGUMENT: path must be a file`; indexing off `ERR_NOT_SUPPORTED: search indexing is not active for project <mount_id>`.
- **Scenario:** SC-006. **Priority:** Must

#### FR-NEW-010 [EARS-E]: jobs can be listed
> WHEN a member of a project asks for its jobs THE system SHALL list, for one file or for the whole project, each job with its path, kind, state, cause (`change` or `manual`), creation time, the seconds left when `waiting`, its end time when finished, and its failure reason when `failed`.

- **Business rules:** The tool is `fs.list_derived_jobs` with `mount_id`, an optional `path` and an optional `marker`. A job has `job_id`, `path`, `kind`, `state`, `cause`, `created_at` (UTC, RFC 3339 with seconds), `seconds_left` (whole seconds, rounded up, only when `waiting`), `ended_at` (UTC, RFC 3339 with seconds, only when finished) and `reason` (only when `failed`). The answer has `jobs` and `marker` (absent when no more).
- **Business rules:** States: `waiting`, `running`, `done`, `failed`, `superseded`. The last finished job of each file and kind is kept, so a finished job stays listed until a newer job of that file ends. At most 100 jobs per answer; order: `waiting` jobs first (fewest seconds left first), then `running`, then finished jobs (newest end first); ties by path, then kind. A marker continues the order of the first page as it was then; a job whose state changed between two pages can appear on two pages or on none, and a job whose state did not change appears exactly once. A `path` that names a folder lists the jobs of the files under it. An unusable marker is refused with `ERR_INVALID_ARGUMENT: marker is not valid`. A `path` that does not exist, nothing to list, or search not enabled for the server: an empty list. A non member and an absent project are refused as in FR-NEW-009.
- **Scenario:** SC-006, SC-009. **Priority:** Must

#### FR-NEW-011 [EARS-S]: previous data is served while a rebuild waits or runs
> WHILE a rebuild of a file waits or runs THE system SHALL keep serving the previous search entries of that file unchanged.

- **Business rules:** Applies to the rebuilds of this increment; a change of index mode keeps today's behavior (FR-NEW-015).
- **Scenario:** SC-007, SC-008, SC-009. **Priority:** Must

#### FR-NEW-012 [EARS-O]: a failed rebuild keeps the old entries and the write
> IF a rebuild fails THEN THE system SHALL set its job to `failed` with the reason, keep the previous search entries, leave the write that caused it successful, and SHALL NOT retry it on its own.

- **Business rules:** Keeps SPEC-0008/FR-018. A later change or a rebuild by hand creates a new job; the one exception to "no retry" is the rescheduling of FR-NEW-017. The reason is `embedding service unreachable` when the embedding service cannot be reached, and otherwise the error text of the failure prefixed by its `ERR_` code when it has one.
- **Scenario:** SC-006, SC-007, SC-009. **Priority:** Must

#### FR-NEW-013 [EARS-E]: a change during a running rebuild discards its result
> WHEN a running rebuild ends THE system SHALL discard its result and set its job to `superseded` if its file changed since it started or a rebuild by hand was asked for during it.

- **Business rules:** The change itself scheduled the next rebuild (FR-NEW-001). At most one job of a file is `running`.
- **Scenario:** SC-002, SC-006, SC-008. **Priority:** Must

#### FR-NEW-014 [EARS-U]: the settle delay is a server setting
> The system SHALL read the settle delay as a whole number of seconds, 0 or more, from the key `search.settle_delay_seconds` of the server configuration.

- **Business rules:** When the setting is absent or empty, the delay is 30 seconds.
- **Scenario:** SC-001, SC-002. **Priority:** Must

#### FR-NEW-015 [EARS-E]: an index mode change cancels the jobs
> WHEN the index mode of a project changes THE system SHALL cancel and remove its waiting jobs and end its running jobs as `superseded` with their results discarded.

- **Business rules:** The index itself follows today's mode change behavior: its entries are removed and, when the new mode is active, the project is rebuilt in full (SPEC-0008). That rebuild creates no job.
- **Scenario:** SC-010. **Priority:** Must

#### FR-NEW-016 [EARS-O]: an invalid settle delay stops the start
> IF the configured settle delay is negative or not a whole number THEN THE system SHALL refuse to start with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`.

- **Business rules:** The check applies whether or not search is enabled.
- **Scenario:** SC-002. **Priority:** Must

#### FR-NEW-017 [EARS-E]: a restart ends running rebuilds
> WHEN the server starts THE system SHALL end each rebuild that was running at its last stop as `failed` with the reason `interrupted by a server restart` and schedule a new waiting rebuild for its file with the full settle delay.

- **Business rules:** With a settle delay of 0 the new rebuild starts right after the start of the server and is not listed as `waiting`.
- **Scenario:** SC-005. **Priority:** Must

#### FR-NEW-018 [EARS-E]: deleting a project removes its jobs
> WHEN a project is deleted THE system SHALL cancel and remove all its jobs.

- **Business rules:** A job removed while running leaves no record when its computation ends.
- **Scenario:** SC-010. **Priority:** Must

#### FR-NEW-019 [EARS-E]: a restored project lists no old job
> WHEN a deleted project is restored THE system SHALL list no job for it.

- **Scenario:** SC-010. **Priority:** Must

#### FR-NEW-020 [EARS-E]: a rebuild replaces the entries in one step
> WHEN a rebuild succeeds THE system SHALL replace the previous search entries of the file in one step.

- **Scenario:** SC-009. **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-E]: indexing follows the settle delay (modifies the lag rule of SPEC-0008/FR-017)
> WHEN a file of an indexing project is written THE system SHALL make its new content searchable once the file has been unchanged for the settle delay and the rebuild has ended.

- **Before:** SPEC-0008/FR-017 business rule: the index lags the volume by the duration of one backend call; SPEC-0008/E2E-023: the write answers well under 2 s, the new chunks are not counted at once and are counted after 5 s.
- **After:** searchable after the settle delay plus the indexing time; with a delay of 0, as before.
- **Reason:** avoid rebuilding an often modified file at every save.
- **Scenario:** SC-001, SC-002, SC-007

### 6.3 Removed
None.

## 7. Non functional requirements (user observable)
### 7.1 Performance
- A write, edit, patch, move, copy or delete answers as fast as before; the delay and the jobs add no waiting (FR-NEW-004).
- Listing the jobs of a project with 10 000 jobs answers in under 2 seconds, 100 jobs per answer.
### 7.2 Security and access
- Rebuilding by hand and listing jobs need membership of the project; a non member sees nothing and cannot name a job (FR-NEW-009, FR-NEW-010).
### 7.5 Reliability
- A restart loses no waiting rebuild (FR-NEW-007).
- A failed rebuild never removes the previous entries (FR-NEW-012).
### 7.6 Operability
- An operator can see, for any file, what is waiting, since when, and why a rebuild failed (FR-NEW-010).

## 8. Edge cases
| Category | Case | Expected behavior | Test |
|---|---|---|---|
| Empty states | no job exists | empty list | E2E-016 |
| Boundary values | change just before the delay ends, and just after | the first restarts the wait, the second lets the rebuild start | E2E-009 |
| Boundary values | settle delay 0, empty or invalid | immediate rebuild, 30 s, refused start | E2E-002, E2E-052, E2E-025 |
| Boundary values | 10 000 jobs | 100 per answer with a marker | E2E-020 |
| Concurrent actions | two rebuilds by hand a second apart | two jobs, at most one running, the first `superseded` | E2E-017 |
| Concurrent actions | two files changed together | independent jobs | E2E-018 |
| Concurrent actions | a rebuild by hand while one runs | the running one ends `superseded` | E2E-039, E2E-057 |
| Access states | a person who is not a member; a project that does not exist | `ERR_FORBIDDEN`; `ERR_PROJECT_NOT_FOUND` | E2E-012, E2E-027, E2E-059 |
| Data variations | file name with accents and spaces | the job names it exactly | E2E-019 |
| Partial operations | rebuild fails halfway | previous entries kept | E2E-007, E2E-041 |
| Dependency failures | index service down | `failed` with reason | E2E-007, E2E-051 |
| Ordering and timing | change during a running rebuild | `superseded`, one waiting job | E2E-008, E2E-044, E2E-045 |
| Ordering and timing | restart with waiting and running rebuilds | full delay again; running ones `failed` | E2E-011, E2E-034, E2E-054, E2E-067 |
| Partial operations | delete or move during a rebuild; move over an existing file | `superseded`, jobs removed or moved | E2E-030, E2E-033 |
| Navigation | a copy of a folder | every file under it | E2E-068 |
| Exhaustion, Slow connectivity | not applicable: no quota is consumed by a rebuild of entries, no interactive flow | | |

## 9. Acceptance tests (the contract)
> Behavioral and technology agnostic. /sdlc-design maps each test to a driver. An implementing agent uses this as its definition of done.
> Timing convention: stub services answer at once unless a test says otherwise; a rebuild starts within 2 seconds after its delay ends and ends within 5 seconds after it starts; every stated time is read with a tolerance of 2 seconds. A "save" is a write of new content to the file. Searches and job lists read the state at the stated second.

### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | journey | SC-001 | FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-014 | Critical |
| E2E-002 | New | requirement | SC-002 | FR-NEW-003, FR-NEW-014 | High |
| E2E-004 | New | journey | SC-006 | FR-NEW-008, FR-NEW-010 | Critical |
| E2E-005 | New | journey | SC-009 | FR-NEW-010, FR-NEW-011, FR-NEW-020 | High |
| E2E-007 | New | failure | SC-007 | FR-NEW-004, FR-NEW-011, FR-NEW-012 | Critical |
| E2E-008 | New | journey | SC-008 | FR-NEW-002, FR-NEW-011, FR-NEW-013 | High |
| E2E-009 | New | edge | SC-001 | FR-NEW-002 | High |
| E2E-010 | New | failure | SC-004 | FR-NEW-005 | High |
| E2E-011 | New | edge | SC-005 | FR-NEW-007 | Critical |
| E2E-012 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-013 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-014 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-016 | New | edge | SC-006 | FR-NEW-010 | Medium |
| E2E-017 | New | edge | SC-006 | FR-NEW-008, FR-NEW-013 | High |
| E2E-018 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-019 | New | edge | SC-006 | FR-NEW-010 | Medium |
| E2E-020 | New | edge | SC-006 | FR-NEW-010 | Medium |
| E2E-021 | New | failure | SC-001 | FR-NEW-001 | High |
| E2E-023 | New | journey | SC-002 | FR-NEW-003, FR-NEW-004 | Critical |
| E2E-024 | New | journey | SC-004 | FR-NEW-006 | High |
| E2E-025 | New | failure | SC-002 | FR-NEW-016 | High |
| E2E-026 | New | edge | SC-006 | FR-NEW-008 | High |
| E2E-027 | New | failure | SC-006 | FR-NEW-010 | High |
| E2E-028 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-029 | New | edge | SC-002 | FR-NEW-003, FR-NEW-013 | High |
| E2E-030 | New | failure | SC-004 | FR-NEW-005 | High |
| E2E-031 | New | journey | SC-004 | FR-NEW-005 | High |
| E2E-032 | New | journey | SC-004 | FR-NEW-006 | Medium |
| E2E-034 | New | failure | SC-005 | FR-NEW-017 | High |
| E2E-035 | New | journey | SC-005 | FR-NEW-007 | Medium |
| E2E-036 | New | journey | SC-007 | FR-NEW-012 | High |
| E2E-039 | New | edge | SC-006 | FR-NEW-008, FR-NEW-013 | High |
| E2E-040 | New | edge | SC-009 | FR-NEW-011 | High |
| E2E-041 | New | edge | SC-009 | FR-NEW-011, FR-NEW-020 | High |
| E2E-042 | New | failure | SC-007 | FR-NEW-005 | High |
| E2E-043 | New | edge | SC-007 | FR-NEW-012 | High |
| E2E-044 | New | failure | SC-008 | FR-NEW-011, FR-NEW-013 | High |
| E2E-045 | New | edge | SC-008 | FR-NEW-002, FR-NEW-013 | High |
| E2E-046 | New | journey | SC-010 | FR-NEW-015 | High |
| E2E-047 | New | failure | SC-010 | FR-NEW-018, FR-NEW-019 | High |
| E2E-048 | New | edge | SC-010 | FR-NEW-015 | High |
| E2E-049 | New | edge | SC-001 | FR-NEW-001 | Medium |
| E2E-051 | New | failure | SC-007 | FR-NEW-012, FR-MOD-001 | High |
| E2E-052 | New | edge | SC-002 | FR-NEW-014 | Medium |
| E2E-053 | New | journey | SC-002 | FR-NEW-014 | Medium |
| E2E-054 | New | edge | SC-005 | FR-NEW-007, FR-NEW-017 | Medium |
| E2E-056 | New | failure | SC-006 | FR-NEW-008, FR-NEW-012 | High |
| E2E-057 | New | edge | SC-006 | FR-NEW-008, FR-NEW-013 | High |
| E2E-059 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-060 | New | failure | SC-010 | FR-NEW-015 | High |
| E2E-061 | New | edge | SC-010 | FR-NEW-018 | High |
| E2E-062 | New | journey | SC-010 | FR-NEW-019 | Medium |
| E2E-015 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-033 | New | edge | SC-004 | FR-NEW-006 | Medium |
| E2E-063 | New | journey | SC-003 | FR-NEW-001 | High |
| E2E-064 | New | failure | SC-003 | FR-NEW-001 | High |
| E2E-065 | New | edge | SC-010 | FR-NEW-019 | Medium |
| E2E-066 | New | failure | SC-010 | FR-NEW-018 | High |
| E2E-067 | New | edge | SC-005 | FR-NEW-007 | Medium |
| E2E-068 | New | edge | SC-003 | FR-NEW-001 | Medium |
| E2E-069 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-070 | New | edge | SC-006 | FR-NEW-010 | Medium |
| E2E-071 | New | failure | SC-002 | FR-NEW-016 | High |
| E2E-072 | New | failure | SC-005 | FR-NEW-017 | High |
| E2E-073 | New | failure | SC-009 | FR-NEW-011, FR-NEW-012 | High |
| E2E-074 | New | edge | SC-002 | FR-NEW-016 | Medium |
| E2E-075 | New | failure | SC-004 | FR-NEW-006 | High |
| E2E-076 | New | failure | SC-006 | FR-NEW-009 | High |
| E2E-077 | New | edge | SC-009 | FR-NEW-011, FR-NEW-020 | High |
| E2E-MOD-001 | Modified | journey | SC-001 | FR-MOD-001 | Critical |
| E2E-MOD-002 | Modified | journey | SC-002 | FR-MOD-001 | High |

Coverage: happy 15 (journey 14, requirement 1), failure 27, side effect 0, edge 27, state transition 0, security 0 as a category (the access rules are in the failure tests E2E-012 and E2E-027). 69 new tests and 2 modified. Happy to failure ratio 1:1.8.

### 9.2 New tests
#### E2E-001: three saves, one rebuild
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-014
- **Given** project `proj` indexing its files, settle delay 30 seconds, `/notes.md` indexed with `alpha`, Alice (`alice@example.com`) a member
- **When** Alice saves `/notes.md` with `beta` at 0 s, with `gamma` at 20 s and with `delta` at 40 s
- **Then** each save answers at once; at 67 s search finds no `delta`; at 80 s search finds `delta` and no `alpha`
- **And** the job list for `/notes.md` shows exactly one job of kind `search_index`, state `done`, cause `change`, whose id is the id seen in the job list after the first save and whose creation time is the time of the first save
- **Priority:** Critical

#### E2E-002: delay 0 rebuilds at once
- **Category:** requirement. **Scenario:** SC-002. **Requirements:** FR-NEW-003, FR-NEW-014
- **Given** settle delay 0, `/notes.md` indexed with `alpha`
- **When** Alice saves `/notes.md` with `beta`
- **Then** the save answers at once; at 10 s search finds `beta`
- **And** no job of `/notes.md` is `waiting`
- **Priority:** High

#### E2E-004: rebuild by hand, then list
- **Category:** journey. **Scenario:** SC-006. **Requirements:** FR-NEW-008, FR-NEW-010
- **Given** settle delay 30 s; Alice saved `/notes.md` with `beta` 5 s ago (a rebuild waits)
- **When** Alice asks `fs.recalculate` for kind `search_index` on `/notes.md`, then lists the jobs of `/notes.md`
- **Then** the answer names `/notes.md`, kind `search_index`, a job id and a state `running` (or `done` if it already ended); search finds `beta` within 7 s, without waiting the 30 s
- **And** the list shows one job of that kind, cause `manual`, and no `waiting` job
- **Priority:** Critical

#### E2E-005: previous entries served while a rebuild waits
- **Category:** journey. **Scenario:** SC-009. **Requirements:** FR-NEW-010, FR-NEW-011, FR-NEW-020
- **Given** `/notes.md` indexed with `alpha`; settle delay 30 s
- **When** Alice saves it with `beta`, searches `alpha` and `beta` at 5 s, and lists the jobs
- **Then** the search for `alpha` finds `/notes.md`; the search for `beta` finds nothing
- **And** the job list shows one `waiting` job with between 21 and 28 seconds left; after it ends, `alpha` is gone and `beta` is found
- **Priority:** High

#### E2E-007: index failure keeps the old entries and the write
- **Category:** failure. **Scenario:** SC-007. **Requirements:** FR-NEW-004, FR-NEW-011, FR-NEW-012
- **Given** `/notes.md` indexed with `alpha`; the embedding service is down; settle delay 2 s
- **When** Alice saves `/notes.md` with `beta`, then waits 12 s
- **Then** the save answered successfully; search for `alpha` still finds `/notes.md`
- **And** the job is `failed` with the reason `embedding service unreachable`; no new job starts by itself during the next 60 s
- **Priority:** Critical

#### E2E-008: change during a running rebuild
- **Category:** journey. **Scenario:** SC-008. **Requirements:** FR-NEW-002, FR-NEW-011, FR-NEW-013
- **Given** settle delay 3 s; a rebuild of `/notes.md` is running (the embedding service answers in 10 s); the file held `beta`
- **When** Alice saves `/notes.md` with `gamma` while it runs, then the first rebuild ends and the settle delay passes
- **Then** the first job ended `superseded` (visible until the next job of that file and kind ends); search never found `beta` as new content; after the quiet period search finds `gamma`
- **And** a new job was `waiting` between the save and the second rebuild
- **Priority:** High

#### E2E-009: the boundary of the delay
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** settle delay 30 s; instant services
- **When** case A: Alice saves at 0 s and at 29 s and at 58 s; case B: she saves at 0 s and at 45 s
- **Then** case A: nothing starts before 86 s, one rebuild runs and ends by 97 s and the job list shows one `done` job for the three saves; case B: the first rebuild starts between 30 s and 32 s and ends by 37 s, and a second starts between 75 s and 77 s
- **And** in case B the job list shows one `done` job (the last finished job per file and kind)
- **Priority:** High

#### E2E-010: a delete cancels the waiting rebuild
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-005
- **Given** a rebuild of `/a.md` waits
- **When** Alice deletes `/a.md`, then waits past the settle delay
- **Then** no job names `/a.md`; search never finds its content
- **And** no rebuild runs for `/a.md` (the job count of the project stays 0)
- **Priority:** High

#### E2E-011: a restart keeps the waiting rebuild
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-007
- **Given** Alice saved `/notes.md` with `beta`; the rebuild waits with 20 s left
- **When** the operator restarts the server
- **Then** the job list shows `/notes.md` `waiting` with between 28 and 30 s left (the full delay)
- **And** after 40 quiet seconds search finds `beta` and the job is `done`
- **Priority:** Critical

#### E2E-012: a non member cannot rebuild
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** Dave (`dave@example.com`) is a member of project `other` only; a rebuild of `/notes.md` waits in `proj`
- **When** Dave asks `fs.recalculate` for `search_index` on `/notes.md` of `proj`
- **Then** the answer is `ERR_FORBIDDEN: 'dave@example.com' is not a member of 'proj'`
- **And** the waiting job is unchanged and no job started
- **Priority:** High

#### E2E-013: unknown kind
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** Alice, a member of `proj`
- **When** she asks for kind `chunks`
- **Then** the answer is `ERR_INVALID_ARGUMENT: kind must be 'search_index'`
- **And** no job started
- **Priority:** High

#### E2E-014: unknown file
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** Alice; `/nope.md` does not exist
- **When** she asks for `search_index` on `/nope.md`
- **Then** the answer is `ERR_NOT_FOUND: file not found: /nope.md`
- **And** no job started
- **Priority:** High

#### E2E-016: no job yet
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-010
- **Given** project `proj` with no write since the server started
- **When** Alice lists the jobs of `proj`, then of `/notes.md`
- **Then** both lists are empty
- **And** no marker is returned
- **Priority:** Medium

#### E2E-017: two rebuilds by hand a second apart
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-008, FR-NEW-013
- **Given** `/notes.md` indexed; the embedding service answers in 5 s; Carol (`carol@example.com`) is a member
- **When** Alice and Carol each ask for `search_index` on `/notes.md`, Carol 1 s after Alice
- **Then** the two answers name two different job ids
- **And** at no time are two jobs of that file and kind `running`; between 5 s and 8 s the list shows Alice's job `superseded` and Carol's `waiting` or `running`; after 15 s the list shows one finished job, Carol's, `done`, and Alice's job is no longer listed
- **Priority:** High

#### E2E-018: two files, independent jobs
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** settle delay 30 s; instant services
- **When** Alice saves `/a.md` at 0 s and `/b.md` at 20 s
- **Then** `/a.md` is rebuilt about 30 s after its save and `/b.md` about 50 s, each within 2 s
- **And** two `done` jobs, one per file
- **Priority:** Medium

#### E2E-019: names with accents and spaces
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-010
- **Given** a file `/notes été.md` indexed
- **When** Alice rebuilds it by hand and lists its jobs
- **Then** the job names `/notes été.md` exactly
- **And** the list without a path shows it too
- **Priority:** Medium

#### E2E-020: a long job history is paged
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-010
- **Given** 10 000 files each with a finished job
- **When** Alice lists the jobs of the project, then asks for the next page with the returned marker
- **Then** the first answer holds 100 jobs, newest end first, and a marker; the second holds the next 100
- **And** the first answer arrives in under 2 s
- **Priority:** Medium

#### E2E-021: a project that does not index schedules nothing
- **Category:** failure. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** project `plain` whose index mode is `none`
- **When** Alice saves `/x.md` and waits past the settle delay
- **Then** the list of jobs of `plain` is empty
- **And** the save answered successfully
- **Priority:** High

#### E2E-023: a write never waits
- **Category:** journey. **Scenario:** SC-002. **Requirements:** FR-NEW-003, FR-NEW-004
- **Given** the embedding service answers in 30 s; settle delay 0
- **When** Alice saves `/notes.md`, edits it, patches it, moves it and deletes it
- **Then** each call answers in under 1 s
- **And** the rebuild of `/notes.md` starts right after the first save and ends in the background, after the calls answered
- **Priority:** Critical

#### E2E-024: a move keeps the wait
- **Category:** journey. **Scenario:** SC-004. **Requirements:** FR-NEW-006
- **Given** Alice saved `/b.md` 10 s ago; settle delay 30 s
- **When** she moves it to `/c.md`
- **Then** one `waiting` job names `/c.md` with 20 s left (within 2 s), none names `/b.md`
- **And** 40 s after the original save search finds the content under `/c.md`
- **Priority:** High

#### E2E-025: invalid settle delay
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-016
- **Given** a server configuration with settle delay `-5`, then `1.5`, then `abc`
- **When** the operator starts the server
- **Then** each start stops with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`
- **And** with the setting absent the server starts with 30 s
- **Priority:** High

#### E2E-026: a rebuild by hand replaces the waiting one
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-008
- **Given** `/notes.md` has a waiting job (cause `change`)
- **When** Alice rebuilds by hand
- **Then** the waiting job is gone; one job exists with cause `manual`
- **And** after the delay no extra job starts
- **Priority:** High

#### E2E-027: a non member cannot list jobs
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-010
- **Given** Dave is a member of `other` only; `proj` has jobs
- **When** Dave lists the jobs of `proj`
- **Then** the answer is `ERR_FORBIDDEN: 'dave@example.com' is not a member of 'proj'`
- **And** listing the jobs of `other` still works and shows only `other` jobs
- **Priority:** High

#### E2E-028: search_index where indexing is not active
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** project `plain` (index mode `none`); `/x.md` exists
- **When** Alice asks `search_index` on `/x.md`
- **Then** the answer is `ERR_NOT_SUPPORTED: search indexing is not active for project plain`
- **And** no job started
- **Priority:** High

#### E2E-029: a burst of saves with a delay of 0
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-003, FR-NEW-013
- **Given** settle delay 0; the embedding service answers in 3 s
- **When** Alice saves `/notes.md` with `v1`, `v2` and `v3`, one second apart
- **Then** every save answers at once; at most one job of that file and kind is `running` at any time; at 20 s search finds `v3` and neither `v1` nor `v2`
- **And** no job is `waiting` at any time; only a rebuild that had started can have ended `superseded`; the job list ends with one finished job, `done`
- **Priority:** High

#### E2E-030: a delete during a running rebuild
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-005
- **Given** a rebuild of `/a.md` is running (the embedding service answers in 10 s)
- **When** Alice deletes `/a.md`, and the rebuild ends
- **Then** search never finds the content of `/a.md`
- **And** no job names `/a.md` in the job list
- **Priority:** High

#### E2E-031: deleting a file with nothing pending
- **Category:** journey. **Scenario:** SC-004. **Requirements:** FR-NEW-005
- **Given** `/a.md` indexed, no job waiting, one finished job
- **When** Alice deletes `/a.md`
- **Then** search no longer finds it
- **And** no job names `/a.md`
- **Priority:** High

#### E2E-032: moving a file with nothing pending
- **Category:** journey. **Scenario:** SC-004. **Requirements:** FR-NEW-006
- **Given** `/b.md` indexed, no job waiting, one finished job
- **When** Alice moves it to `/c.md`
- **Then** a search read 2 s after the move answered finds nothing under `/b.md`, and within 5 s it finds the content under `/c.md`; no waiting job is created
- **And** the finished job now names `/c.md`
- **Priority:** Medium

#### E2E-034: a restart while a rebuild runs
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-017
- **Given** a rebuild of `/notes.md` is running (the embedding service answers in 60 s)
- **When** the operator restarts the server
- **Then** the interrupted job is `failed` with the reason `interrupted by a server restart`
- **And** a new job for the same file and kind is `waiting` with the full settle delay left, and after it ends the content is searchable
- **Priority:** High

#### E2E-035: a restart with nothing pending
- **Category:** journey. **Scenario:** SC-005. **Requirements:** FR-NEW-007
- **Given** all jobs `done`
- **When** the operator restarts the server
- **Then** no job is `waiting` or `running`
- **And** the finished jobs are still listed
- **Priority:** Medium

#### E2E-036: recovery after a failure
- **Category:** journey. **Scenario:** SC-007. **Requirements:** FR-NEW-012
- **Given** the rebuild of `/notes.md` failed (embedding service down)
- **When** the service is back and Alice saves `/notes.md` again
- **Then** a new waiting job starts and ends `done`; search finds the latest content
- **And** the earlier failed job is replaced in the list by the new finished job (one finished job per file and kind)
- **Priority:** High

#### E2E-039: a rebuild by hand while one runs
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-008, FR-NEW-013
- **Given** a rebuild of `/notes.md` is running (the embedding service answers in 10 s)
- **When** Alice asks for a rebuild by hand of `search_index` on `/notes.md`
- **Then** the answer names a new job in state `waiting` with `seconds_left` 0; the running job ends `superseded`; the new job (cause `manual`) starts as soon as it has ended
- **And** at no time are two jobs of that file and kind `running`
- **Priority:** High

#### E2E-040: new content is not searchable before its rebuild ends
- **Category:** edge. **Scenario:** SC-009. **Requirements:** FR-NEW-011
- **Given** `/notes.md` indexed with `alpha`; settle delay 30 s
- **When** Alice saves it with `beta` and searches `beta` at 15 s
- **Then** search finds nothing for `beta`, and `alpha` is still found
- **And** the job list says `waiting` with about 15 s left
- **Priority:** High

#### E2E-041: entries are replaced in one step
- **Category:** edge. **Scenario:** SC-009. **Requirements:** FR-NEW-011, FR-NEW-020
- **Given** `/big.md` indexed as 4 chunks with `old`; a rebuild produces 6 chunks with `new` and takes 10 s
- **When** Alice searches `old` and `new` repeatedly every second during the rebuild
- **Then** every answer shows only old chunks or only new chunks, never both
- **And** after the rebuild, `old` is gone and `new` finds 6 chunks
- **Priority:** High

#### E2E-042: a failed job is removed when the file is deleted
- **Category:** failure. **Scenario:** SC-007. **Requirements:** FR-NEW-005
- **Given** the rebuild of `/notes.md` ended `failed`
- **When** Alice deletes `/notes.md`
- **Then** no job names `/notes.md`
- **And** listing the project jobs shows no `failed` job for it
- **Priority:** High

#### E2E-043: two failures in a row keep one finished job
- **Category:** edge. **Scenario:** SC-007. **Requirements:** FR-NEW-012
- **Given** the embedding service is down; settle delay 2 s
- **When** Alice saves `/notes.md`, waits 10 s, saves again and waits 10 s
- **Then** the job list shows one finished job for `/notes.md` and kind `search_index`, `failed`
- **And** its end time is later than the first failure
- **Priority:** High

#### E2E-044: a discarded result is never visible
- **Category:** failure. **Scenario:** SC-008. **Requirements:** FR-NEW-011, FR-NEW-013
- **Given** settle delay 3 s; a rebuild of `/notes.md` is running with the content `beta` (the embedding service answers in 10 s for the first rebuild and at once for the next ones); the previous content was `alpha`
- **When** Alice saves `gamma`, and the first rebuild ends
- **Then** search finds `alpha` (the previous entries) and not `beta`, until the second rebuild ends (by 25 s after the start of the first)
- **And** the second rebuild then makes `gamma` searchable
- **Priority:** High

#### E2E-045: two saves during one running rebuild
- **Category:** edge. **Scenario:** SC-008. **Requirements:** FR-NEW-002, FR-NEW-013
- **Given** a rebuild of `/notes.md` is running (10 s); settle delay 30 s
- **When** Alice saves twice during it, 2 s apart
- **Then** after the running job ends `superseded`, exactly one job of that file and kind is `waiting`
- **And** after the quiet period one rebuild runs and ends `done`
- **Priority:** High

#### E2E-046: the index mode becomes none
- **Category:** journey. **Scenario:** SC-010. **Requirements:** FR-NEW-015
- **Given** a `search_index` rebuild of `/notes.md` waits in `proj`
- **When** the platform admin sets the index mode of `proj` to `none`
- **Then** the waiting job is removed
- **And** after the settle delay no job exists for `proj`
- **Priority:** High

#### E2E-047: a project is deleted, then restored
- **Category:** failure. **Scenario:** SC-010. **Requirements:** FR-NEW-018, FR-NEW-019
- **Given** `proj` has one waiting, one running and one finished job
- **When** the platform admin deletes the project, then restores it
- **Then** while `proj` is deleted, listing its jobs answers `ERR_PROJECT_NOT_FOUND`; after the restore the list is empty
- **And** no rebuild runs for it after the settle delay
- **Priority:** High

#### E2E-048: the index mode changes between two active modes
- **Category:** edge. **Scenario:** SC-010. **Requirements:** FR-NEW-015
- **Given** a `search_index` rebuild of `/notes.md` waits; the mode is `rag`
- **When** the platform admin changes the mode to `both`
- **Then** the waiting job is removed
- **And** within 20 s of the mode change (a project of 1 file) the project is rebuilt in full in the mode `both` and search finds the content of `/notes.md` there
- **Priority:** High

#### E2E-049: a copy and a restore schedule the destination only
- **Category:** edge. **Scenario:** SC-001. **Requirements:** FR-NEW-001
- **Given** `/a.md` indexed; `/trash` holds a deleted `/d.md`
- **When** Alice copies `/a.md` to `/a2.md` and restores `/d.md`
- **Then** one waiting job names `/a2.md` and one names `/d.md`
- **And** none names `/a.md`
- **Priority:** Medium

#### E2E-051: an index failure never fails the write
- **Category:** failure. **Scenario:** SC-007. **Requirements:** FR-NEW-012, FR-MOD-001
- **Given** the embedding service is down; settle delay 0
- **When** Alice writes `/notes.md` with `beta`
- **Then** the write answers successfully with its byte count; reading the file returns `beta`
- **And** search status shows no new chunks and the job is `failed`
- **Priority:** High

#### E2E-052: an empty settle delay setting means the default
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-014
- **Given** a server configuration with the settle delay present but empty
- **When** the operator starts the server
- **Then** the server starts and uses 30 s
- **And** the job list of a saved file shows `waiting` with about 30 s left
- **Priority:** Medium

#### E2E-053: a configured delay is used
- **Category:** journey. **Scenario:** SC-002. **Requirements:** FR-NEW-014
- **Given** settle delay 5 s, `/notes.md` indexed with `alpha`
- **When** Alice saves `/notes.md` with `beta`
- **Then** search finds no `beta` at 1 s and finds `beta` at 14 s
- **And** one `done` job
- **Priority:** Medium

#### E2E-054: two running rebuilds at a restart
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-007, FR-NEW-017
- **Given** a rebuild of `/a.md` runs and one of `/b.md` waits with 10 s left
- **When** the operator restarts the server
- **Then** the job of `/a.md` is `failed` with `interrupted by a server restart` and a new waiting job exists for it; the job of `/b.md` is `waiting` with the full delay
- **And** after the delay both files are rebuilt; with a settle delay of 0 instead, the new rebuild of `/a.md` starts right after the start and is not listed as `waiting`
- **Priority:** Medium

#### E2E-056: a failed manual rebuild is reported in the answer
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-008, FR-NEW-012
- **Given** the embedding service is down; `/notes.md` indexed
- **When** Alice asks for a rebuild by hand and waits for the job to end
- **Then** the first answer is not an error and its state is `running`, or `failed` with the reason `embedding service unreachable` if it already failed; within 5 s the job list shows `failed` with that reason
- **And** the previous entries are still searchable
- **Priority:** High

#### E2E-057: a second rebuild by hand while the first runs
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-008, FR-NEW-013
- **Given** `/notes.md` indexed; the embedding service answers in 10 s
- **When** Alice asks for `search_index` on `/notes.md`; Carol asks 2 s later
- **Then** the first job ends `superseded`; Carol's answer names a different job id with state `waiting` and `seconds_left` 0, which then runs
- **And** at no time are two jobs of that file and kind `running`
- **Priority:** High

#### E2E-059: the project does not exist
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** Alice; project `ghost` does not exist
- **When** she asks `fs.recalculate` for `search_index` on `/a.md` of `ghost`, then lists the jobs of `ghost`
- **Then** both answers are `ERR_PROJECT_NOT_FOUND`
- **And** no job started
- **Priority:** High

#### E2E-060: a running rebuild when the index mode becomes none
- **Category:** failure. **Scenario:** SC-010. **Requirements:** FR-NEW-015
- **Given** a `search_index` rebuild of `/notes.md` is running (the embedding service answers in 10 s)
- **When** the platform admin sets the index mode of `proj` to `none`, and the rebuild ends
- **Then** the job ends `superseded` and its entries are not kept
- **And** search finds nothing in `proj`
- **Priority:** High

#### E2E-061: a project is deleted while a rebuild runs
- **Category:** edge. **Scenario:** SC-010. **Requirements:** FR-NEW-018
- **Given** a rebuild of `/notes.md` is running in `proj`
- **When** the platform admin deletes `proj`, and the rebuild ends
- **Then** while `proj` is deleted, listing its jobs answers `ERR_PROJECT_NOT_FOUND` and the result of the rebuild is not kept; after the restore nothing of the rebuild is searchable
- **And** restoring `proj` later lists no job
- **Priority:** High

#### E2E-062: jobs of a restored project
- **Category:** journey. **Scenario:** SC-010. **Requirements:** FR-NEW-019
- **Given** `proj` deleted with 3 waiting jobs
- **When** the platform admin restores `proj`, then waits past the settle delay
- **Then** the job list of `proj` is empty
- **And** the files of `proj` are back and a later save schedules a new rebuild
- **Priority:** Medium

#### E2E-015: a kind of a later increment is refused
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** Alice, a member of `proj`
- **When** she asks for kind `conversion`
- **Then** the answer is `ERR_INVALID_ARGUMENT: kind must be 'search_index'`
- **And** no job started
- **Priority:** High

#### E2E-033: a move onto an existing file with jobs
- **Category:** edge. **Scenario:** SC-004. **Requirements:** FR-NEW-006
- **Given** `/b.md` has a waiting job; `/c.md` exists and has a waiting job and a finished job; a rebuild of `/b.md` is not running
- **When** Alice moves `/b.md` over `/c.md` (the move replaces it)
- **Then** the jobs that named the replaced `/c.md` are removed; one `waiting` job names `/c.md` with the time already waited by `/b.md`
- **And** none names `/b.md`
- **Priority:** Medium

#### E2E-063: a copy schedules its destination
- **Category:** journey. **Scenario:** SC-003. **Requirements:** FR-NEW-001
- **Given** `/a.md` indexed, settle delay 30 s
- **When** Alice copies `/a.md` to `/a2.md`
- **Then** the copy answers at once; one `waiting` job names `/a2.md` and none names `/a.md`
- **And** after 45 s search finds the content under both `/a.md` and `/a2.md`
- **Priority:** High

#### E2E-064: a copy in a project that does not index
- **Category:** failure. **Scenario:** SC-003. **Requirements:** FR-NEW-001
- **Given** project `plain` (index mode `none`) holds `/a.md`
- **When** Alice copies `/a.md` to `/a2.md` and waits past the settle delay
- **Then** no job is listed for `plain`
- **And** the copy answered successfully
- **Priority:** High

#### E2E-065: a restored project lists only new jobs
- **Category:** edge. **Scenario:** SC-010. **Requirements:** FR-NEW-019
- **Given** `proj` was deleted with 3 jobs and restored
- **When** Alice saves `/notes.md` once the project is back
- **Then** the job list holds exactly one job, for `/notes.md`
- **And** none of the old jobs reappears
- **Priority:** Medium

#### E2E-066: no rebuild runs for a deleted project
- **Category:** failure. **Scenario:** SC-010. **Requirements:** FR-NEW-018
- **Given** `proj` has 3 waiting jobs
- **When** the platform admin deletes `proj` and the settle delay passes
- **Then** no rebuild ran for any file of `proj`
- **And** the embedding service received no call for `proj`
- **Priority:** High

#### E2E-067: two restarts in a row
- **Category:** edge. **Scenario:** SC-005. **Requirements:** FR-NEW-007
- **Given** Alice saved `/notes.md`; the rebuild waits
- **When** the operator restarts the server twice, 5 s apart
- **Then** the job list shows `/notes.md` `waiting` with the full delay left after each restart
- **And** after the delay one rebuild runs and ends `done`
- **Priority:** Medium

#### E2E-068: a copy of a folder schedules every file under it
- **Category:** edge. **Scenario:** SC-003. **Requirements:** FR-NEW-001
- **Given** folder `/docs` holds `/docs/a.md` and `/docs/b.md`, both indexed; settle delay 30 s
- **When** Alice copies `/docs` to `/docs2`
- **Then** two `waiting` jobs name `/docs2/a.md` and `/docs2/b.md`, none names a path under `/docs`
- **And** after 45 s search finds the content under `/docs2`
- **Priority:** Medium

#### E2E-069: a rebuild by hand when search is not enabled
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** the server configuration has search disabled; Alice is a member of `proj`
- **When** she asks `fs.recalculate` for `search_index` on `/notes.md`
- **Then** the answer is `ERR_NOT_SUPPORTED: search is not enabled; set search.enabled: true in config`
- **And** no job started
- **Priority:** High

#### E2E-070: listing jobs when search is not enabled
- **Category:** edge. **Scenario:** SC-006. **Requirements:** FR-NEW-010
- **Given** the server configuration has search disabled
- **When** Alice lists the jobs of `proj`
- **Then** the list is empty
- **And** no error is returned
- **Priority:** Medium

#### E2E-071: an invalid settle delay with search disabled
- **Category:** failure. **Scenario:** SC-002. **Requirements:** FR-NEW-016
- **Given** the server configuration has search disabled and a settle delay of `-5`
- **When** the operator starts the server
- **Then** the start stops with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`
- **And** with the delay corrected the server starts
- **Priority:** High

#### E2E-072: a restart interrupts a rebuild while the embedding service is down
- **Category:** failure. **Scenario:** SC-005. **Requirements:** FR-NEW-017
- **Given** a rebuild of `/a.md` runs; the embedding service goes down; settle delay 3 s
- **When** the operator restarts the server and waits 15 s
- **Then** the first job is `failed` with `interrupted by a server restart` and the new job is `failed` with `embedding service unreachable`
- **And** the previous entries of `/a.md` are still searchable
- **Priority:** High

#### E2E-073: a rebuild that fails while Alice reads
- **Category:** failure. **Scenario:** SC-009. **Requirements:** FR-NEW-011, FR-NEW-012
- **Given** `/notes.md` indexed with `alpha`; Alice saved `beta`; the embedding service is down; settle delay 2 s
- **When** Alice searches `alpha` and `beta` before the rebuild, while it fails, and after it failed
- **Then** every search finds `alpha` and none finds `beta`
- **And** the job is `failed` with the reason `embedding service unreachable`
- **Priority:** High

#### E2E-074: a settle delay that is not a whole number
- **Category:** edge. **Scenario:** SC-002. **Requirements:** FR-NEW-016
- **Given** the server configuration has a settle delay of `2.5`, then of `abc`
- **When** the operator starts the server each time
- **Then** each start stops with `ERR_INVALID_ARGUMENT: settle delay must be a whole number of seconds, 0 or more`
- **And** a delay of `0` starts the server
- **Priority:** Medium

#### E2E-075: a move while the file is being rebuilt
- **Category:** failure. **Scenario:** SC-004. **Requirements:** FR-NEW-006
- **Given** settle delay 3 s; a rebuild of `/b.md` runs (the embedding service answers in 10 s)
- **When** Alice moves `/b.md` to `/c.md`, and the rebuild ends
- **Then** the running job ends `superseded` and names `/c.md`; one new job names `/c.md` and waits the full delay; a search finds nothing under `/b.md`
- **And** after the delay `/c.md` is searchable
- **Priority:** High

#### E2E-076: a rebuild by hand on a folder
- **Category:** failure. **Scenario:** SC-006. **Requirements:** FR-NEW-009
- **Given** `/docs` is a folder; Alice is a member
- **When** she asks `fs.recalculate` for `search_index` on `/docs`
- **Then** the answer is `ERR_INVALID_ARGUMENT: path must be a file`
- **And** no job started
- **Priority:** High

#### E2E-077: no mixed entries during a replacement
- **Category:** edge. **Scenario:** SC-009. **Requirements:** FR-NEW-011, FR-NEW-020
- **Given** `/big.md` indexed as 4 chunks of `old`; settle delay 0; the rebuild produces 6 chunks of `new` and takes 8 s
- **When** Alice searches `new` every second during the rebuild
- **Then** no answer holds both old and new chunks of `/big.md`
- **And** after the rebuild the search finds 6 chunks of `new` and none of `old`
- **Priority:** High

### 9.4 Modified tests
#### E2E-MOD-001: content searchable after the settle delay (was SPEC-0008/E2E-023)
- **Category:** journey. **Scenario:** SC-001. **Requirements:** FR-MOD-001
- **Validated before:** after a write, search status did not yet count the new chunks, and counted them after 5 seconds; the write answered well under the embedding delay of 2 seconds.
- **Validates now:** with settle delay 3 s and an embedding delay of 2 s, the write answers at once, search status does not yet count the new chunks at 1 s, and counts them at 14 s.
- **Given** an indexing project, settle delay 3 s, embedding service answering in 2 s
- **When** Alice writes a file of 2 chunks
- **Then** the write answers in under 1 s; at 1 s the status counts 0 new chunks; at 14 s it counts 2
- **And** the job list shows one `done` job
- **Priority:** Critical

#### E2E-MOD-002: a delay of 0 keeps the old timing (was SPEC-0008/E2E-023)
- **Category:** journey. **Scenario:** SC-002. **Requirements:** FR-MOD-001
- **Validated before:** as above, with no delay.
- **Validates now:** with settle delay 0, the behavior of SPEC-0008/E2E-023 holds.
- **Given** an indexing project, settle delay 0, embedding service answering in 4 s
- **When** Alice writes a file of 2 chunks
- **Then** the write answers in under 1 s; at 1 s search status does not yet count the new chunks; at 14 s it counts 2
- **And** no job is `waiting`
- **Priority:** High

### 9.5 Removed tests
None.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-MOD-001, FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-NEW-014 | E2E-001, E2E-MOD-001 | E2E-021 | E2E-009, E2E-018, E2E-049 |
| SC-002 | FR-MOD-001, FR-NEW-003, FR-NEW-004, FR-NEW-013, FR-NEW-014, FR-NEW-016 | E2E-002, E2E-023, E2E-053, E2E-MOD-002 | E2E-025, E2E-071 | E2E-029, E2E-052, E2E-074 |
| SC-003 | FR-NEW-001 | E2E-063 | E2E-064 | E2E-068 |
| SC-004 | FR-NEW-005, FR-NEW-006 | E2E-024, E2E-031, E2E-032 | E2E-010, E2E-030, E2E-075 | E2E-033 |
| SC-005 | FR-NEW-007, FR-NEW-017 | E2E-035 | E2E-034, E2E-072 | E2E-011, E2E-054, E2E-067 |
| SC-006 | FR-NEW-008, FR-NEW-009, FR-NEW-010, FR-NEW-012, FR-NEW-013 | E2E-004 | E2E-012, E2E-013, E2E-014, E2E-027, E2E-028, E2E-056, E2E-059, E2E-015, E2E-069, E2E-076 | E2E-016, E2E-017, E2E-019, E2E-020, E2E-026, E2E-039, E2E-057, E2E-070 |
| SC-007 | FR-MOD-001, FR-NEW-004, FR-NEW-005, FR-NEW-011, FR-NEW-012 | E2E-036 | E2E-007, E2E-042, E2E-051 | E2E-043 |
| SC-008 | FR-NEW-002, FR-NEW-011, FR-NEW-013 | E2E-008 | E2E-044 | E2E-045 |
| SC-009 | FR-NEW-010, FR-NEW-011, FR-NEW-012, FR-NEW-020 | E2E-005 | E2E-073 | E2E-040, E2E-041, E2E-077 |
| SC-010 | FR-NEW-015, FR-NEW-018, FR-NEW-019 | E2E-046, E2E-062 | E2E-047, E2E-060, E2E-066 | E2E-048, E2E-061, E2E-065 |

Tests per requirement (at least 3 each):
- FR-NEW-001: E2E-001, E2E-018, E2E-021, E2E-049, E2E-063, E2E-064, E2E-068 (7 tests)
- FR-NEW-002: E2E-001, E2E-008, E2E-009, E2E-045 (4 tests)
- FR-NEW-003: E2E-002, E2E-023, E2E-029 (3 tests)
- FR-NEW-004: E2E-001, E2E-007, E2E-023 (3 tests)
- FR-NEW-005: E2E-010, E2E-030, E2E-031, E2E-042 (4 tests)
- FR-NEW-006: E2E-024, E2E-032, E2E-033, E2E-075 (4 tests)
- FR-NEW-007: E2E-011, E2E-035, E2E-054, E2E-067 (4 tests)
- FR-NEW-008: E2E-004, E2E-017, E2E-026, E2E-039, E2E-056, E2E-057 (6 tests)
- FR-NEW-009: E2E-012, E2E-013, E2E-014, E2E-028, E2E-059, E2E-015, E2E-069, E2E-076 (8 tests)
- FR-NEW-010: E2E-004, E2E-005, E2E-016, E2E-019, E2E-020, E2E-027, E2E-070 (7 tests)
- FR-NEW-011: E2E-005, E2E-007, E2E-008, E2E-040, E2E-041, E2E-044, E2E-073, E2E-077 (8 tests)
- FR-NEW-012: E2E-007, E2E-036, E2E-043, E2E-051, E2E-056, E2E-073 (6 tests)
- FR-NEW-013: E2E-008, E2E-017, E2E-029, E2E-039, E2E-044, E2E-045, E2E-057 (7 tests)
- FR-NEW-014: E2E-001, E2E-002, E2E-052, E2E-053 (4 tests)
- FR-NEW-015: E2E-046, E2E-048, E2E-060 (3 tests)
- FR-NEW-016: E2E-025, E2E-071, E2E-074 (3 tests)
- FR-NEW-017: E2E-034, E2E-054, E2E-072 (3 tests)
- FR-NEW-018: E2E-047, E2E-061, E2E-066 (3 tests)
- FR-NEW-019: E2E-047, E2E-062, E2E-065 (3 tests)
- FR-NEW-020: E2E-005, E2E-041, E2E-077 (3 tests)
- FR-MOD-001: E2E-051, E2E-MOD-001, E2E-MOD-002 (3 tests)

## 11. Open questions
None that change observable behavior.

## 12. Glossary (MANDATORY)
| Term | Definition | Context |
|---|---|---|
| Derived data | what is built from a file; today its search entries | Derived data |
| Kind | which derived data: `search_index` now | Derived data |
| Settle delay | whole seconds a file must stay unchanged before its rebuild starts | Derived data |
| Quiet period | the time a file stays unchanged | Derived data |
| Rebuild | one recomputation of one kind for one file | Derived data |
| Job | the record of one rebuild, with a state | Derived data |
| Waiting, running, done, failed, superseded | states of a job: waiting for the delay; computing; finished well; finished with a reason; finished but its result was discarded because the file changed or went away | Derived data |
| Cause | `change` (a change started it) or `manual` (a person did) | Derived data |
| Save | a write of new content to a file | Derived data |
| Member | a person who belongs to a project | Derived data |
| Index mode | the project setting that says whether files are searchable and how | Search |
| Search entries | what search keeps of a file, replaced as a whole | Search |

## 13. Implementability gate (MANDATORY)
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 10 | 5 | NOT-IMPLEMENTABLE (what counts as a change, delete and move while running, delay 0 versus supersede, tool answers, refusal precedence, test timing, FR-MOD wording, index mode changes, test coverage, EARS shapes, no read only role) |
| 2 | 16 | 6 | NOT-IMPLEMENTABLE (simultaneous manual calls, mode change wipe, quota, refusal order, timing margins, listing order, companion scheduling, failure reasons, folders) |
| 3 | 11 | 5 | NOT-IMPLEMENTABLE (setting key, ambiguous "converted", inline conversion, git operations, quota payer, queued manual rebuild, move over a file, timing, EARS shapes) |
| 4 | 8 | 4 | NOT-IMPLEMENTABLE after the owner's cut of 2026-11-20 (conversion and git or archive triggers moved to BL-0176 and BL-0177): search disabled for the server, job identity across restarts of the wait, when entries follow a move, timing arithmetic, atomic replacement |
| 5 | 11 | 0 | NOT-IMPLEMENTABLE (timing arithmetic of nine tests, simultaneous call wording, EARS shapes, requirement tags, restart at delay 0, removed running jobs) |
| 6 | 6 | 0 | NOT-IMPLEMENTABLE (simultaneous call window, paging order, folder paths, move during a rebuild, three requirement shapes, tags) |
| 7 | 3 | 0 | verdict IMPLEMENTABLE; its three findings (a test contradicting the kept job rule, when a superseded job is listed on delete or move, one banned modal) are amended below |

**Amendments applied:** every finding of rounds 1 to 3 is merged or removed by the cut; tests regenerated from one source.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix (for /sdlc-design and the auditor, not for the product owner)
### 14.1 Current State evidence
| # | Claim (Section 2) | Evidence |
|---|---|---|
| 1 | every write spawns an immediate detached indexing task | `crates/core/src/search/indexer.rs:113,128,199-376` |
| 2 | no dedup, debounce, queue or status record; failures are logged only | `crates/core/src/search/indexer.rs:65-96,183` |
| 3 | detached tasks are not awaited at shutdown | `crates/core/src/app.rs:176-189`; `on_write` returns a handle that production callers drop, `crates/core/src/search/indexer.rs:113-128` |
| 4 | delete, move, copy and restore call the indexer hooks | `crates/core/src/tools/lifecycle.rs:70,110,145`, `crates/core/src/tools/trash.rs:217` |
| 5 | `search.index` indexes one path by hand; with search off it answers `ERR_NOT_SUPPORTED: search is not enabled; set search.enabled: true in config` | `crates/core/src/tools/search_semantic.rs:31-53` (`crates/core/src/tools/all.rs:163` only asserts it is registered) |
| 6 | tests: 12 in the indexer, auto index names in the search e2e | `crates/core/src/search/indexer.rs`, `crates/core/src/search/e2e.rs:572,593,722` |
| 7 | membership is the only authorization; an absent or deleted project is `ERR_PROJECT_NOT_FOUND`; a non member is refused as `ERR_FORBIDDEN: '<person>' is not a member of '<project>'` | `crates/core/src/storage/admin.rs:380-398` |
| 8 | a mode change wipes the project's entries and, when the new mode is active, starts a full rebuild | `crates/core/src/search/indexer.rs:65-100` |
Test command: `./test.sh` (`cargo test --workspace`), search only: `cargo test -p fsa-core --lib search::`; full gate `make check`.

### 14.2 Class sweep
Not applicable (`Security: n/a`).

### 14.3 Design inputs
- Waiting jobs must survive a restart: a persisted record per (volume, path, kind) is needed; no job table was found by the survey (confirm in design). Signal: E2E-011, E2E-034.
- Two writes to one path race in the indexer today; the coalescing rule must remove that race. Signal: E2E-001, E2E-029.
- A paged listing of 10 000 jobs in under 2 s needs an index on (project, end time). Signal: E2E-020.
- The key `search.settle_delay_seconds` and its boot validation follow the existing configuration precedence (FR-NEW-014, FR-NEW-016).
- New tools `fs.recalculate` and `fs.list_derived_jobs` change `TOOL_CONTRACT.txt` and `tool-contract-golden.json` (regenerate with `MCPFS_REWRITE_TOOL_CONTRACT=1` and review the diff); REST twins follow the "one implementation" rule of `AGENTS.md`.
- Existing tests that assume immediate indexing (`auto_index_write_then_query_finds_it`, SPEC-0008 E2E-023, the hook test in `indexer.rs`) need a settle delay of 0 or a wait; the design lists them.
- The relation between `search.index` (immediate, no job) and `fs.recalculate` (job) is for the design to keep clear; `search.index` is unchanged.
- A job answer needs a stable identifier and an RFC 3339 end time in UTC; project deletion and restoration must remove or ignore old jobs (FR-NEW-018, FR-NEW-019). Signal: E2E-047.
- The backends replace a path's entries by delete then insert, per backend, without atomicity (`search/bm25_pg.rs:56-90`, `search/mod.rs:252-262`; the sqlite backends to be confirmed). FR-NEW-011, FR-NEW-012 and FR-NEW-013 need a stage then swap of a path's entries across all backends, computed before the old entries are touched, so a failure or a superseded result leaves the old entries intact. Signal: E2E-007, E2E-041, E2E-044, E2E-051, E2E-056, E2E-060.
- `after_companion` (`indexer.rs:228-244`) and `tools/document.rs:93` also trigger indexing; they keep their present behavior and are not turned into jobs here (BL-0176).
- One scheduler for all waiting jobs, not one timer task per write.
- The index mode change wipe (`indexer.rs:65-100`) must coexist with the jobs (FR-NEW-015). Signal: E2E-046, E2E-048, E2E-060.
- The kind parameter stays open for later kinds (BL-0176, BL-0168).
