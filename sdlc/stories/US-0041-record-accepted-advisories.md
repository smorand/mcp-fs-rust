# US-0041: Record the accepted advisories and drop the stale ignores

> Spec: SPEC-0035 (sdlc/specs/SPEC-0035-clear-warning-advisories/)
> Nature: DEBT
> Priority: P1
> Depends on: US-0040
> min_tier: 2
> Files touched: 4
> Closes spec: yes

## Objective
With git2 and yoke-derive fixed, record each remaining accepted advisory once in `.cargo/audit.toml` with its reason, move the rsa ignore off the Makefile flag, and drop the two stale git2 ignores from `deny.toml`, so `cargo audit` reports only the 2 yanked `zip` warnings that BL-0058 and BL-0059 track.

## Requirements covered
#### SPEC-0035/DR-005 [EARS-U]: Accepted advisories recorded once for cargo audit
> `.cargo/audit.toml` SHALL list, under `[advisories] ignore`, exactly RUSTSEC-2023-0071, RUSTSEC-2024-0384, RUSTSEC-2024-0436, RUSTSEC-2026-0192, RUSTSEC-2026-0253, RUSTSEC-2026-0002 and RUSTSEC-2024-0449, each with a one line reason comment naming the crate and why it stays (no upstream fix, or the backlog id that removes it), and the `security` target SHALL run `cargo audit` with no `--ignore` flag.
- **Old form:** `cargo audit --ignore RUSTSEC-2023-0071` (`Makefile:47`)
- **New form:** `cargo audit`

#### SPEC-0035/DR-006 [EARS-UB]: No stale cargo deny ignore
> `deny.toml` SHALL NOT list RUSTSEC-2026-0183 nor RUSTSEC-2026-0184, and SHALL keep RUSTSEC-2023-0071.

#### SPEC-0035/DR-007 [EARS-U]: Audit end state
> `cargo audit` SHALL exit 0 and report exactly 2 warnings, both of kind `yanked`, for `zip` 2.4.2 and `zip` 3.0.0; `cargo deny check` SHALL exit 0 with no `advisory-not-detected` warning.

#### SPEC-0035/DR-008 [EARS-U]: Behavior invariance
> The workspace SHALL produce, for every input covered by the existing test suite, byte identical observable output before and after this change.


## Independent Test
`cargo test -p mcp-fs --test security_gate` passes DT-012 (red after US-0040 alone: 9 warnings, not 2), DT-014 (red: `deny.toml` still lists both git2 ids) and DT-015 (red: `.cargo/audit.toml` absent); then the full gate DT-019.

## Design excerpt
### Decisions
- **(SPEC-0035) DEC-002:** Record acceptances in `.cargo/audit.toml`, the cargo audit configuration, and move the rsa ignore there so cargo audit has one source. **Alternatives considered:** more `--ignore` flags in the Makefile (rejected: no room for reasons, and split across two places); `deny.toml` only (rejected: cargo deny does not report these advisories under `unmaintained = "workspace"`, and listing them there triggers `advisory-not-detected`). **Implemented by:** DR-005. **Evidence:** probe in Section 2; `deny.toml:12-21`.
- **(SPEC-0035) DEC-003:** Advisories with a fix path (lru via aws-sdk-s3, unsafe-libyaml) are recorded as accepted with their backlog id as reason, so the audit output shows only new or untracked findings. **Alternatives considered:** leave them visible as reminders (rejected: twelve standing warnings are why a new one would not stand out). **Implemented by:** DR-005. **Evidence:** n/a.
- **(SPEC-0035) DEC-004:** The yanked `zip` warnings stay visible: cargo audit cannot ignore a yanked crate by advisory id, and they are tracked by BL-0058 and BL-0059. **Alternatives considered:** disable yanked checks (rejected: hides every future yank). **Implemented by:** DR-007. **Evidence:** n/a.
- **(SPEC-0035) DEC-007:** The new audit tests skip when `cargo-audit` or `cargo-deny` is not installed, like `make_security_passes` (`crates/mcp-fs/tests/security_gate.rs:22-26`). **Alternatives considered:** fail under `CI` (rejected: changes an existing test, out of scope). **Implemented by:** DR-005, DR-006, DR-007. **Evidence:** cited.
- **(SPEC-0035) DEC-008:** Two stories along the seam "code that must compile together" then "gate configuration". **Obliged by:** SPEC-0035/DR-001..DR-008. **Rationale:** the 9 git2 edits cannot be split (the workspace does not compile in between), but the audit configuration can follow: after US-0040, `cargo deny check` only warns `advisory-not-detected` on the two stale ignores and still reports `advisories ok` (Section 2 probe), and `cargo audit` with the Makefile flag still exits 0, so main stays green; each story stays within the tier 2 file budget (6 and 4 paths). Section 9's single step is kept as one story for its code part; the split does not change its order. **Alternatives considered:** one story of 9 paths (above every tier budget, and it would mix a code bump with gate policy); splitting the git2 edits (the build breaks in between). **Source:** design. **Evidence:** Section 2 probe.
- **(SPEC-0035) DEC-009:** Both stories at `min_tier: 2`. **Obliged by:** SPEC-0035/DR-001..DR-008. **Rationale:** decision density is low: the probe fixed every compile edit, and the story inlines them; the only judgment is crafting a raw `packed-refs` line for DT-017 and DT-018. **Alternatives considered:** `min_tier: 1` for US-0040 (6 paths sits at the top of the tier 2 budget; rejected because the edits are dictated, not designed). **Source:** design. **Evidence:** Section 2 probe.
### Assumptions (SPEC-0035)
- **Assumption:** the advisory database is read live; an advisory published after this lot makes DT-012 red for an unrelated crate. That is the intended signal (a new backlog entry), not a scope change.
- **Assumption:** DT-012 also goes red when `zip` is fixed upstream (the count drops below 2). Intended: it prompts updating the expectation in the lot that closes BL-0058 or BL-0059.
### Paths
| Path | Action |
|---|---|
| `.cargo/audit.toml` | new: `[advisories]` `ignore = [...]` with the 7 ids of DR-005, one `#` reason per line |
| `Makefile` | `:47` `cargo audit --ignore RUSTSEC-2023-0071` becomes `cargo audit` |
| `deny.toml` | remove `:19` RUSTSEC-2026-0183 and `:20` RUSTSEC-2026-0184; keep `:18` RUSTSEC-2023-0071 |
| `crates/mcp-fs/tests/security_gate.rs` | DT-012, DT-014, DT-015 |
### Existing patterns
- Reasons for `.cargo/audit.toml`, one per id: RUSTSEC-2023-0071 rsa Marvin timing side channel, JWT signing keys, no fix released; RUSTSEC-2024-0384 instant unmaintained, via datafusion 43; RUSTSEC-2024-0436 paste unmaintained, via datafusion 43; RUSTSEC-2026-0192 ttf-parser unmaintained, via pdf-extract 0.12.1, no newer release; RUSTSEC-2026-0253 lru unsound pop, via tantivy 0.26.2 (no fix) and aws-sdk-s3 (until BL-0060); RUSTSEC-2026-0002 lru unsound IterMut, via aws-sdk-s3 (until BL-0060); RUSTSEC-2024-0449 unsafe-libyaml unmaintained, via serde_yaml (until BL-0061).
- `make_security_passes` (`crates/mcp-fs/tests/security_gate.rs:21-30`) for running a tool and skipping when `cargo-audit` or `cargo-deny` is absent (DEC-007).
- `cargo audit --json`: count every entry of every array under `warnings`; expected exactly 2, kind `yanked`, packages `zip` 2.4.2 and 3.0.0.

## Acceptance tests
> 100% must pass, run through `make check`.

| Test | Category | Requirement | Location | Given / When / Then | Red before because |
|---|---|---|---|---|---|
| SPEC-0035/DT-012 | security | DR-005, DR-007 | `security_gate.rs`, runs `cargo audit --json`, skips when not installed | exit 0; the JSON `warnings` hold exactly 2 entries, both kind `yanked`, packages `zip` 2.4.2 and `zip` 3.0.0 | 12 warnings, among them git2 RUSTSEC-2026-0183 and RUSTSEC-2026-0184 |
| SPEC-0035/DT-014 | security | DR-006, DR-007 | `security_gate.rs`: `deny.toml` text, then `cargo deny check` (skips when not installed) | `deny.toml` contains `RUSTSEC-2023-0071` and neither `RUSTSEC-2026-0183` nor `RUSTSEC-2026-0184`; `cargo deny check` exits 0 and its output has no `advisory-not-detected` | `deny.toml:19-20` list both git2 ids |
| SPEC-0035/DT-015 | structural | text of `.cargo/audit.toml` and `Makefile` | `crates/mcp-fs/tests/security_gate.rs` | US-0041 |
| SPEC-0035/DT-019 | guardrail | DR-008 | full suite | 8.1 green; `tool-contract-golden.json` unchanged | n/a |

## Constraints
- No code change; only `.cargo/audit.toml`, `Makefile`, `deny.toml` and the security gate tests.
- Never ignore a yanked `zip` warning, and never set `yanked` checks off (DEC-004).
- If the live advisory database now reports a new advisory for an unrelated crate, DT-012 goes red: record it as a new backlog entry and report, do not widen the ignore list.

## Definition of done
- [ ] RED evidence recorded for every new test before the change, in `.implement/US-0041-red.log`.
- [ ] Every acceptance test above green; `make check` green; main stays green after the squash.
- [ ] DEBT: the existing suite passes unmodified, exceptions: none (0).
- [ ] Non regression command: `MCPFS_TEST_PG_DSN=postgres://mcpfs:mcpfs@127.0.0.1:55432/mcpfs MCPFS_TEST_MSSQL_DSN='Server=tcp:127.0.0.1,51433;Database=master;User Id=sa;Password=mcpfs_Passw0rd;TrustServerCertificate=true' make check`
- [ ] Docs updated: none.
- [ ] Self review: full.
- [ ] Closes spec: the full acceptance set of SPEC-0035 green (DT-010..DT-019; DT-012 and DT-014 need `cargo-audit` and `cargo-deny`, installed here), and every DR-001..DR-008 cited by a done or landed story.
- [ ] One commit `US-0041: Record the accepted advisories and drop the stale ignores`, body `Spec: SPEC-0035` and the DR ids.

## Owner amendment (2026-10-17)
Also accept, in `.cargo/audit.toml`, RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369 (zip 2.4.2 and 3.0.0, published 2026-10-09/10, patched >=9.0.0), reason: only `ZipArchive::extract`/`ZipStreamReader::extract` are affected, never called (`tools/archive.rs:386` decodes entries itself and refuses symlinks at `:398`); removed by BL-0058 (zip 9). DR-005's list is therefore 10 ids. DT-012 still expects exactly the 2 yanked zip warnings and no vulnerability.
Update (2026-10-17, during implement): crates.io un-yanked zip 2.4.2 and 3.0.0 (API `yanked: false`), so `cargo audit` reports 0 warnings; DT-012 asserts no vulnerability and no warning, per the spec assumption on DT-012. `cargo deny check` also needed the 3 zip ids, added to `deny.toml` with the same reason.
