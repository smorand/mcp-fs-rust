# SPEC-0086: Refuse a destination outside the project for every archive format, design

> Spec: sdlc/specs/SPEC-0086-refuse-destination-outside-the-project/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-09
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Destination rule | one in tree function `archive_single::resolve_destination` (segment walk, then `PosixPath::normpath`) | n/a | DEC-001 |
| Path normalizer of every other tool | `SafetyManager::normalize_path`, unchanged | n/a | DEC-003 |

No dependency change. Skills loaded: `rust` (no `unwrap` on fallible production paths, clippy `--all-features -D warnings`, fmt check).

## 2. Approaches

### Chosen: one segment walk for the destination, before the archive is read
A destination is refused when, walking its `/` separated segments from the root, a `..` is met at the root; otherwise it resolves with `PosixPath::normpath` of `/` plus the raw text. The same function serves every format (it replaces `archive_single::normalize_destination`, `archive_single.rs:123-130`), and for zip, 7z, RAR and the tar family it runs right after the multi volume check, before the stored archive is read (`archive.rs:178-205`). Pros: one rule, written once (the walk `tools::export::ensure_no_escape` and `ensure_entry_path_safe` already use, `export.rs:119-126`, `archive.rs:455-476`); nothing is read for a refused call. Cons: the planned destination helper of SPEC-0084 and SPEC-0085 disappears in the same step (DEC-002). Affected scope: `tools/archive.rs`, `tools/archive_single.rs`.

### Rejected: change `SafetyManager::normalize_path`
Refusing the climb in the shared normalizer would change every path option of every tool (writes, copies, moves, the HTTP plane) and their pinning tests (`safety.rs:263-265`, `posix.rs:115-118`); FR-NEW-006 and BL-0135 keep them unchanged.

### Rejected: keep `normalize_destination` and add an absolute check
`normalize_destination` tests `normpath(dest)` for a leading `..` on relative input only and then calls `normalize_path`, whose string prefix test refuses `..out` (`safety.rs:105`). Patching both halves keeps two rules where the spec wants one (FR-NEW-001, E2E-010).

## 3. Architecture

```
extract_archive (archive.rs)
  path, node lookup, folder check, detect_format, tar password, multi volume RAR   items 1 to 6
  Single(_) -> extract_single: name, resolve_destination, ...                     (bare order unchanged)
  others:  dest = destination.map(resolve_destination).unwrap_or(stem)            items 7, 8 (NEW place)
           read bytes, decode / list / declared check / bound ...                  item 9: every later refusal
           collision, admission, writes
```

`resolve_destination(raw)`: (1) a NUL character → `ERR_PATH_OUT_OF_BOUNDS` `path contains a NUL byte`; (2) walk `raw.split('/')`: `""` and `"."` stay, `".."` at depth 0 → `ERR_PATH_OUT_OF_BOUNDS` `path escapes the volume root: <raw>`, `".."` otherwise steps up, any other segment steps down; (3) return `PosixPath::normpath(&format!("/{raw}"))` (a leading `/` already present collapses). `SafetyManager::normalize_path` is not called for the destination.

## 4. Data model

None.

## 5. Contracts and interfaces

- `archive_single.rs`: `pub(crate) fn resolve_destination(raw: &str) -> Result<String>` replaces `pub(crate) fn normalize_destination(state: &AppState, dest: &str) -> Result<String>` (its two callers: `archive_single.rs:244`, `archive.rs:224`).
- Error contract: `ERR_PATH_OUT_OF_BOUNDS` `path escapes the volume root: <destination exactly as typed>`; `ERR_PATH_OUT_OF_BOUNDS` `path contains a NUL byte` (texts of `safety.rs:100-108`, unchanged).
- Answer and audit line: `destination` is the resolved folder (`finish_extraction`, `archive.rs:313-357`, unchanged).
- `archive.rs`: `planned_destination` (SPEC-0084 DEC-003) is removed; the bound of SPEC-0084 and the declared check of SPEC-0085 take the destination resolved before reading (always counted: a refused one never reaches them).
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json`: unchanged (`destination` keeps `Destination directory for the extracted entries.`, `archive.rs:107`).

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive_single.rs` | modify: `resolve_destination` replaces `normalize_destination` | FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001 | US-0194 |
| `crates/core/src/tools/archive.rs` | modify: every format uses `resolve_destination` (US-0194, at today's place); the call moves before `read_bytes` and `planned_destination` goes (US-0195); the comment at `:213-218` goes | FR-MOD-001, FR-MOD-002, FR-NEW-003 | US-0194, US-0195 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod archive_destination_e2e;` | test layout | US-0194 |
| `crates/core/src/tools/archive_destination_e2e.rs` | create: the `e2e_0086_*` tests, RAR ones in a nested `mod archive_rar` | every requirement | US-0194 to US-0197 |
| `crates/core/src/tools/archive_declared_e2e.rs` | modify: `e2e_0085_028_the_limit_wins_over_a_refused_destin` replaced by `e2e_0086_mod_001_the_destination_wins_over_the_declar` | FR-MOD-002 (E2E-MOD-001) | US-0195 |
| `crates/core/src/api/dataplane.rs` | modify: E2E-031, E2E-032, E2E-034 | FR-NEW-005 | US-0196 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-006) | US-0197 |

Affected existing tests: kept with their assertions and named: `e2e_0080_061_destination_outside_the_project` (`archive_rar.rs:1283-1301`), the `../../x` tests of SPEC-0081 E2E-035, SPEC-0082 E2E-022, SPEC-0083 E2E-032 (`archive_single.rs:1096-1097`, `:1533-1534`, `:2271-2272`), `safety.rs:263-265`, `util/posix.rs:115-118`. Replaced: `e2e_0085_028_the_limit_wins_over_a_refused_destin` (US-0195). Any other existing test that changes is a story failure: stop and report it.

Affected documentation: `README.md:306-309` (destination rule), `AGENTS.md:137-140` (`archive_single.rs` entry: "destination outside the project"), `.agent_docs/tools.md:183-217` (destination rule for every format, order of refusals, the line "`../../x` is refused for `.rar`, the other formats clamp it" at `:217`), `.agent_docs/testing.md` (filter `e2e_0086_`).

## 7. Compatibility and risks

- Declared changes: a climbing destination is refused by every format, absolute or relative; `..out` is accepted; the destination refusal comes before anything the archive contains.
- Interplay: SPEC-0085 E2E-028 is reversed (E2E-MOD-001, US-0195); SPEC-0084 FR-NEW-005 item 7 and SPEC-0085 FR-NEW-009 item 9 move to item 8 of FR-MOD-002 here. SPEC-0087 adds no test pairing the two.
- Other tools: unchanged (`normalize_path` untouched).
- Rollback: revert the story commits.

## 9. Test strategy

Functional e2e through the typed engine call `extract_archive` after `f.state.authorize(MOUNT, person)` in `crates/core/src/tools/archive_destination_e2e.rs` (helpers from `archive_testkit.rs`); RAR tests in a nested `#[cfg(feature = "rar")] mod archive_rar` (SPEC-0085 DEC-011); E2E-031, E2E-032, E2E-034 through the HTTP door in `api/dataplane.rs`. Names `e2e_0086_<id>_<slug>`, filter `e2e_0086_`. "Nothing is written" reads `/x`, `/uploads/pack` and the folders the test names with `exists`, the stored bytes with `quota::used_bytes`, the session allowance with `session_remaining`, and the audit trail with `safety.audit` (no `extract_archive` line).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-002 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-003 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-004 | functional e2e | typed engine call | archive_destination_e2e.rs (nested `archive_rar`) | US-0197 |
| E2E-005 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-006 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-007 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-008 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-009 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-010 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-011 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-012 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-013 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-014 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-015 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-016 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-017 | functional e2e | typed engine call | archive_destination_e2e.rs (nested `archive_rar`) | US-0194 |
| E2E-018 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-019 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-020 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-021 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-022 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-023 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-024 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-025 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-026 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-027 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0196 |
| E2E-028 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0196 |
| E2E-029 | functional e2e | typed engine call | archive_destination_e2e.rs (nested `archive_rar`) | US-0196 |
| E2E-030 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-031 | functional e2e | HTTP door | api/dataplane.rs tests | US-0196 |
| E2E-032 | functional e2e | HTTP door | api/dataplane.rs tests | US-0196 |
| E2E-033 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0197 |
| E2E-034 | functional e2e | HTTP door | api/dataplane.rs tests | US-0196 |
| E2E-035 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0194 |
| E2E-036 | functional e2e | typed engine call | archive_destination_e2e.rs | US-0195 |
| E2E-MOD-001 | functional e2e | typed engine call | archive_declared_e2e.rs (nested `archive_rar`, replaces `e2e_0085_028_...`) | US-0195 |

Test data: generated with the testkit builders (`build_zip`, `build_sevenz_ext`, tar builder, gzip, bzip2 and xz encoders); RAR `/uploads/pack.rar` is the committed `crates/core/tests/fixtures/rar/v5.rar` (`a.txt` = `hello`); the RAR bomb is `rar5_store` of `zeros.bin`, 10 000 000 zero bytes.

## 10. Decisions log

- **DEC-001:** one function `archive_single::resolve_destination(raw: &str) -> Result<String>`: NUL refused first (`path contains a NUL byte`), then a segment walk on `/` with a depth counter (`""` and `"."` stay, `".."` at depth 0 refused with `path escapes the volume root: <raw>`, `".."` otherwise up, any other segment down, dots inside a name included), then `PosixPath::normpath(&format!("/{raw}"))`; it never calls `SafetyManager::normalize_path`. **Obliged by:** SPEC-0086/FR-NEW-001, FR-NEW-002, FR-NEW-004, FR-MOD-001. **Rationale:** the walk is the repository's existing climb rule (`export.rs:119-126`, `archive.rs:455-476`); judging while walking refuses `a/../../x` and `/a/../../b` even though a clamp would land inside (spec D-6); `normpath` of a non climbing path stays under `/`, and skipping `normalize_path` drops its `starts_with("/..")` prefix test that refuses `..out` (`safety.rs:105`). Splitting on `/` only keeps a backslash a plain character (spec 3.2). **Alternatives considered:** section 2. **Source:** existing code. **Evidence:** `crates/core/src/tools/export.rs:119-126`, `crates/core/src/safety.rs:99-111`, `crates/core/src/util/posix.rs:48-55`.
- **DEC-002:** for zip, 7z, RAR, plain and compressed tar the destination is resolved right after the multi volume check and before `client.read_bytes` (`archive.rs:178-205`); `planned_destination` (SPEC-0084 DEC-003) is removed and the bound and the declared check use the resolved destination with its stored files always counted; the bare formats keep their place (`archive_single.rs:241-248`). **Obliged by:** SPEC-0086/FR-MOD-002, FR-NEW-003, FR-NEW-004. **Rationale:** after this move a refused destination never reaches decoding, so "stored files count for nothing when refused" (SPEC-0084 FR-NEW-002, SPEC-0085 FR-NEW-001) holds trivially. **Alternatives considered:** keeping the helper (dead branch). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:143-227`.
- **DEC-003:** `SafetyManager::normalize_path` and every other path option stay as they are. **Obliged by:** SPEC-0086/FR-NEW-006. **Rationale:** BL-0135 owns them. **Alternatives considered:** section 2. **Source:** spec. **Evidence:** `crates/core/src/safety.rs:263-265`, `crates/core/src/util/posix.rs:115-118`.
- **DEC-004:** US-0194 applies the strict rule at today's place (after decoding for the multi formats), US-0195 moves it before reading: the order tests of US-0195 are red against US-0194 and green after, and SPEC-0085 E2E-028 changes in the story that changes its outcome. **Obliged by:** n/a (no code impact): story planning. **Rationale:** each story ships red evidence for the behavior it adds. **Alternatives considered:** one story (16 plus tests, over the tier 2 budget). **Source:** design. **Evidence:** n/a.
- **DEC-005:** tests in `crates/core/src/tools/archive_destination_e2e.rs`; RAR ones (E2E-004, E2E-017, E2E-029) in a nested `#[cfg(feature = "rar")] mod archive_rar` (SPEC-0085 DEC-011, `test.sh:7`); E2E-004's legs that need no RAR are not split out (the spec writes one test). **Obliged by:** n/a (no code impact): test layout. **Rationale:** as SPEC-0085 DEC-011. **Source:** SPEC-0085 DEC-011. **Evidence:** `test.sh:7`.
- **DEC-006:** documentation in the closing story: `README.md` (destination rule), `AGENTS.md` (`archive_single.rs` entry), `.agent_docs/tools.md` (one destination rule for every format, order, `..out`, the old clamp sentence at `:217` removed), `.agent_docs/testing.md` (filter `e2e_0086_`). **Obliged by:** n/a (no code impact): project convention. **Rationale:** `.agent_docs/tools.md:217` says "the other formats clamp it". **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** as cited.
- **DEC-007:** tests whose rule an earlier story built are green on arrival, each proven by a mutation reasoned to change its observable outcome (listed per story). **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0180 to US-0182. **Source:** design. **Evidence:** n/a.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0194 | one destination rule: refuse a climb above the root for every format | P1 | SPEC-0086/FR-NEW-001, FR-NEW-003, FR-MOD-001 | US-0193 | 2 | no |
| US-0195 | check the destination before the archive is read | P1 | SPEC-0086/FR-MOD-002, FR-NEW-003, FR-NEW-004 | US-0194 | 2 | no |
| US-0196 | earlier refusals first and both doors alike | P2 | SPEC-0086/FR-MOD-002, FR-NEW-002, FR-NEW-005 | US-0195 | 2 | no |
| US-0197 | destinations inside the project unchanged and documentation | P2 | SPEC-0086/FR-NEW-002, FR-NEW-006 | US-0196 | 2 | yes |
