# US-0067: one moment, read only, hardening, docs, spec close

> Spec: SPEC-0047 (sdlc/specs/SPEC-0047-project-disk-usage-breakdown/)
> Nature: FEAT
> Priority: P2
> Depends on: US-0065, US-0066
> min_tier: 1
> Files touched: 7
> Closes spec: yes

## Objective
A read never shows half of a write: it holds the project's admission lock for its duration, so the used bytes, the file count and the list come from one moment. The remaining behaviors (read only, limit as set, boundaries of the cap, ordering edge cases, folders not listed) are asserted, the docs carry the real tool counts and the new tool and route, and the spec closes with its whole acceptance set.

## Requirements covered
- SPEC-0047/FR-NEW-002: The Administration context SHALL list the files from the largest to the smallest.
- SPEC-0047/FR-NEW-005: WHEN the project holds more files than are listed THE Administration context SHALL report the list as cut, and otherwise as complete.
- SPEC-0047/FR-NEW-008: The Administration context SHALL show the limit currently set for the project, no value when it is unlimited, whatever the usage.
- SPEC-0047/FR-NEW-009: The Administration context SHALL NOT change any file, the bytes used, the trash, or the reader's session write budget when it shows a breakdown.
- SPEC-0047/FR-NEW-010: The Administration context SHALL take the bytes used, the number of files and the list from the same moment.

## Independent Test
Run `make check`. Then `cargo test -p mcp-fs-core --lib tools::disk_usage_e2e`: all 35 tests of SPEC-0047 pass; E2E-032 holds the project's admission lock, starts a read, asserts the read has not finished, writes `/docs/new.txt` (30 000 bytes), releases the lock, and the read answers 6 files with `used_bytes` 1 005 500.

## Design excerpt
### Decisions
- DEC-005 (SPEC-0047): `quota::disk_usage` takes `client.admission` for the whole read, reads the counter with `client.meta.used_bytes()` (not `quota::used_bytes`, which locks again and would deadlock), then runs the count and the list in one transaction.
- DEC-010 (SPEC-0047): no index, no schema change; the 100 000 file bound of NFR 7.1 is met by the bounded top N (DEC-003).
### Data model
None.
### Contracts
No new surface. Docs to write: tool and route in `.agent_docs/tools.md` (admin section: heading `## admin (15)` becomes 16, one table row like `admin.get_project_quota` at line 288, served counts at lines 416 and 417 become 61, 110, 114 and the 135 row 136), `.agent_docs/api.md` (one row next to line 39), `.agent_docs/architecture.md` (a sentence next to lines 103 to 104 and the counts at line 264), `README.md` (lines 4, 335 area and 396: counts and the tool), `AGENTS.md` (lines 5, 85, 118, 159, 232 and the Overview: counts). Counts in these docs are stale (109) against the code: write the real figures after `cargo test` prints them (default 61, git 110, git and search 114).
### Paths
| Path | Action |
|---|---|
| `crates/core/src/quota.rs` | modify (lock in `disk_usage`) |
| `crates/core/src/tools/disk_usage_e2e.rs` | modify (add tests) |
| `README.md`, `AGENTS.md` | modify (counts, tool) |
| `.agent_docs/tools.md`, `.agent_docs/api.md`, `.agent_docs/architecture.md` | modify |
### Existing patterns
- Tool shape and gate: `crates/core/src/tools/admin.rs:647-676` (`get_project_quota`) and its `#[tool]` method `crates/core/src/mcp/server.rs:2918-2929`.
- Test harness: `crates/core/src/tools/quota_e2e.rs:60-139` (`World`, `as_`, `seed`, `ok_json`); a second harness for the router: `crates/core/src/tools/diff_files_e2e.rs:709-750`.
- Lock use: `crates/core/src/quota.rs:164-167`:
```rust
pub(crate) async fn used_bytes(client: &VolumeClient) -> Result<i64> {
    let _admission = client.admission.lock().await;
    client.meta.used_bytes().await
}
```
- Deterministic concurrency test (no sleeps): hold `let guard = client.admission.clone().lock_owned().await;`, spawn the read with `tokio::spawn`, `tokio::task::yield_now()` a few times and assert `!handle.is_finished()`, write the file through `client.write_bytes_atomic` (setup path, no admission), `drop(guard)`, await the handle.

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Shared setup unless stated (from the spec): project `Atlas` has a limit of 2 MB (2 097 152 bytes) and stores five files, all `Active`: `/video/demo.mp4` 600 000, `/data/export.csv` 250 000, `/docs/spec.docx` 120 000, `/docs/notes.txt` 4 000, `/readme.md` 1 500 bytes (975 500 in all). Olivia (`olivia@example.com`) owns Atlas, Mia (`mia@example.com`) is a member, Ada (`ada@example.com`) is a platform admin, Otto (`otto@example.com`) belongs to no project, Bram (`bram@example.com`) owns `Borealis`. Every session budget is 262 144 bytes. "The standard answer" means: project `Atlas`, limit 2, bytes used 975 500, 5 files, list complete, entries in the order of the table above, each `Active`. Refusal messages are asserted with `ends_with` (the tool prefixes the code, e.g. `ERR_FORBIDDEN: `).

### SPEC-0047/E2E-017: Reading changes nothing
- Level / Driver / Location: functional e2e / tool, then `McpServer::fs_write_bytes` for the session budget probe / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-009
- **Given** the shared setup, and Olivia has written 100 000 bytes this session (budget 262 144)
- **When** Olivia reads the breakdown of `Atlas` three times
- **Then** all three answers are identical
- **And** the quota read still shows 975 500 bytes used, the five files are unchanged, and Olivia can
  still write 162 144 bytes in this session (a write of 162 145 bytes is refused with
  `session write quota of 262144 bytes exceeded`)

### SPEC-0047/E2E-018: A project over its limit is readable
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-008, 009
- **Given** `Atlas` stores 3 files of 700 000, 500 000 and 300 000 bytes (1 500 000 in all), and Ada
  has lowered its limit to 1 MB
- **When** Olivia reads the breakdown
- **Then** limit 1, bytes used 1 500 000, 3 files in the order 700 000, 500 000, 300 000, complete
- **And** nothing was removed from the project

### SPEC-0047/E2E-021: An unlimited project
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-002. **Requirements:** FR-NEW-008
- **Given** `Atlas` has no limit and stores the five files of the shared setup
- **When** Ada reads the breakdown
- **Then** the limit is shown as no value (neither 0 nor text), bytes used 975 500, 5 entries in order

### SPEC-0047/E2E-024: A cap equal to the number of files
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads with a cap of 120
- **Then** 120 entries, the list is reported as complete

### SPEC-0047/E2E-025: A cap one below the number of files
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-004, 005
- **Given** `Atlas` stores 120 files of 1 000 bytes
- **When** Olivia reads with a cap of 119
- **Then** 119 entries, 120 files counted, the list is reported as cut

### SPEC-0047/E2E-026: The smallest and the largest accepted caps
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-001, 004, 005
- **Given** the shared setup
- **When** Olivia reads with a cap of 1, then with a cap of 500
- **Then** the cap of 1 shows only `/video/demo.mp4` (600 000), 5 files counted, the list cut
- **And** the cap of 500 shows the five entries, the list complete

### SPEC-0047/E2E-028: Empty files come last
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-002
- **Given** `Atlas` stores `/empty.txt` (0 bytes), `/a.txt` (10 bytes) and `/b.txt` (20 bytes)
- **When** Olivia reads the breakdown
- **Then** the order is `/b.txt`, `/a.txt`, `/empty.txt`, and `/empty.txt` shows size 0

### SPEC-0047/E2E-030: Accents and emoji stay intact
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** `Atlas` stores `/rapports/été 📊.xlsx` of 5 000 bytes
- **When** Olivia reads the breakdown
- **Then** the entry shows the path `/rapports/été 📊.xlsx` exactly, size 5 000

### SPEC-0047/E2E-032: A write during the read is seen entirely or not at all
- Level / Driver / Location: functional e2e / tool plus the volume client and its admission lock / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-004. **Requirements:** FR-NEW-009, 010
- **Given** the shared setup
- **When** Mia writes `/docs/new.txt` of 30 000 bytes while Olivia reads the breakdown
- **Then** the answer is either the standard answer (5 files, 975 500) or the answer with 6 files and
  bytes used 1 005 500 including `/docs/new.txt`; in both cases the entries add up to the bytes used
  and the file count matches
- **And** a second read afterwards shows 6 files, bytes used 1 005 500

### SPEC-0047/E2E-033: Folders are not listed
- Level / Driver / Location: functional e2e / `McpServer::admin_get_project_disk_usage` over real storage / `crates/core/src/tools/disk_usage_e2e.rs`
- **Scenario:** SC-001. **Requirements:** FR-NEW-003
- **Given** the shared setup, which holds the folders `/video`, `/data` and `/docs`
- **When** Olivia reads the breakdown
- **Then** no entry is a folder; the file count is 5

## Constraints
- Files not to touch: the tool function except for nothing; the route; the engine query.
- Dependencies not to add: none.
- Patterns to avoid: `std::thread::sleep` or `tokio::time::sleep` to order the test; holding the lock across the whole write path of the tool under test.
- Scope boundary: none left; this story closes SPEC-0047.

## Definition of done
- [ ] RED evidence recorded for every new test before the production code for E2E-032 only (it fails while the read does not wait for the lock); E2E-017, E2E-018, E2E-021, E2E-024, E2E-025, E2E-026, E2E-028, E2E-030 and E2E-033 describe behavior US-0064 already provides, so each gets a mutation check instead (break the behavior it asserts, see the test fail, revert), recorded in the commit body.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] Non regression: every test of US-0064, US-0065, US-0066 stays green; `admin.get_project_quota` unchanged.
- [ ] Docs updated: README.md (counts, tool and route), AGENTS.md (counts, structure line for `quota.rs`), .agent_docs/tools.md, .agent_docs/api.md, .agent_docs/architecture.md
- [ ] Self review: the answer shape matches DEC-004 exactly; no `unwrap`/`expect` on a fallible path; no dash used as punctuation in code, comments or output; every new function has a WHY comment, not a WHAT comment.
- [ ] Closes spec: the 35 acceptance tests of SPEC-0047 are green (E2E-001 to E2E-035), every FR-NEW-001 to FR-NEW-011 is cited by a story of this spec, status done or landed (US-0064 to US-0067), and the tool counts in code and docs agree.
- [ ] One commit `US-0067: one moment, read only, hardening, docs, spec close`, body `Spec: SPEC-0047` and the requirement ids.
