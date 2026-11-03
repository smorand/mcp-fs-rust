# Tag publish error wording: functional specification

> Id: SPEC-0072
> Nature: FEAT
> Depth: S
> Depth evidence: FEAT / depth S: one behavior family (the text of two tag publish refusals), one module family, 3 anticipated requirements, no persisted change.
> Status: implemented
> Generated on: 2026-11-02
> Project: existing
> From backlog: BL-0104, BL-0106
> Depends on: none
> Security: n/a
> CVSS: n/a
> Affected: n/a
> Fixed in: n/a

## 1. Summary
A member who publishes a tag sees two confusing refusals today. When the remote cannot be reached the message talks about a fetch, and when the remote tag vanished while the member was publishing it the message says the tag "has moved to 0000000". This increment names the operation in the first case and says the tag does not exist in the second.

## 2. Current State
### 2.1 What users can do today
- A member can publish one tag (`git.remote_push_tag`) or every due tag (`git.remote_push_tags`) to a tracked remote.
- When the remote cannot be reached or drops the connection, the refusal reads `fetch from host '<host>' failed: <cause>`, whether the member was fetching or publishing. A branch push reads `push of branch '<branch>' failed: <cause>`, with no host.
- When the member confirmed replacing a remote tag and that tag was deleted before the push, the refusal reads `Remote tag '<tag>' has moved to 0000000 since you last saw it. Fetch, then confirm again.`
- Deleting a remote tag that is gone already reads `Remote tag '<tag>' does not exist.`

### 2.2 Specs governing this area
SPEC-0052 (publish, DEC-018/019), SPEC-0053 (EXC-001d, E2E-014: the `does not exist` text), SPEC-0054 (publish all). Deviation: this increment changes the connection failure wording on the publish routes only, and the vanished tag case. Those specs are not modified.

### 2.3 What is verified today
The two-tag unreachable cases are asserted with the old `fetch from host` text for publish all; for one tag only the failure code is compared with a branch push. No test covers a remote tag deleted between the listing and the push.

## 3. Scope
### 3.1 In scope
- The text of a connection failure while publishing one tag or all tags, at any step of the publish.
- The text when the remote tag to replace no longer exists at push time.
### 3.2 Out of scope, and why
- Wording of fetch, clone, pull and remote tag deletion failures: unchanged, they really are a fetch or already read well. Deleting a remote tag keeps `fetch from host`; a wording change there is not requested.
- Wording of the rate limited and credential expired refusals: unchanged.
### 3.3 Constraints imposed by the context
The failure code stays `ERR_INTERNAL_ERROR` for a connection failure and becomes `ERR_NOT_FOUND` for the vanished tag (owner decision, 2026-11-02).

## 5. Usage scenarios
### SC-001: publish while the remote is unreachable
**Actor:** a project member (Alice).
**Preconditions:** local tag `v1.0`; the tracked remote `origin` is down.
**Flow:**
1. Alice publishes `v1.0` → the refusal reads `push of tag 'v1.0' to host '<host>' failed: <cause>`.
2. Alice publishes all tags → the refusal reads `push of tags to host '<host>' failed: <cause>`.
**Postconditions:** the remote and the audit log are unchanged.
**Exceptions:**
- EXC-001a: the connection drops during the push → same wording, cause names the drop.
- EXC-001b: the remote answers `too many requests` → unchanged wording.

### SC-002: the remote tag vanished
**Actor:** Alice.
**Preconditions:** remote tag `v1.0` exists at `f6a7b8c`; Alice confirmed replacing it.
**Flow:** 1. Bob deletes the remote tag `v1.0` after Alice's confirmation and before the push → 2. Alice's publish is refused with `Remote tag 'v1.0' does not exist.`
**Postconditions:** the remote holds no `v1.0`; the audit log is unchanged.
**Exceptions:**
- EXC-002a: the remote tag moved instead of vanishing → `Remote tag 'v1.0' has moved to <short id> since you last saw it. Fetch, then confirm again.` (unchanged).

## 6. Functional requirements
### 6.1 New
#### FR-NEW-001 [EARS-E]: connection failure while publishing one tag
> WHEN the connection to the remote fails while a member publishes one tag THE Remote exchange context SHALL refuse with `push of tag '<tag>' to host '<host>' failed: <cause>`.
- **Business rules:** applies to the listing, the check of an annotated tag's message and the push; `<cause>` is the underlying text, with any secret removed as today; the failure code is unchanged.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-002 [EARS-E]: connection failure while publishing all tags
> WHEN the connection to the remote fails while a member publishes all tags THE Remote exchange context SHALL refuse with `push of tags to host '<host>' failed: <cause>`.
- **Business rules:** applies to every step of the publication (preview listing included); the preview that only counts due tags is a publish step and follows the same rule.
- **Scenario:** SC-001. **Priority:** Must

#### FR-NEW-003 [EARS-E]: the remote tag vanished
> WHEN the remote tag a member confirmed replacing no longer exists at push time THE Remote exchange context SHALL refuse with `Remote tag '<tag>' does not exist.`
- **Business rules:** the failure is `not found`; nothing is sent, nothing audited; a remote tag that moved keeps the `has moved to` text.
- **Scenario:** SC-002. **Priority:** Must

### 6.2 Modified
#### FR-MOD-001 [EARS-E]: publish connection wording (modifies SPEC-0052 publish wording)
> WHEN a connection fails during a tag publish THE Remote exchange context SHALL use FR-NEW-001 or FR-NEW-002 instead of `fetch from host '<host>' failed: <cause>`.
- **Before:** `fetch from host '<host>' failed: <cause>` (Section 2.1). **After:** FR-NEW-001 and FR-NEW-002. **Reason:** an operator reading a publish failure should not see the word fetch.

## 9. Acceptance tests
### 9.1 Summary
| Test | Action | Category | Scenario | Requirements | Priority |
|---|---|---|---|---|---|
| E2E-001 | New | happy | SC-001 | FR-NEW-001, FR-NEW-002 | High |
| E2E-002 | New | failure | SC-001 | FR-NEW-001 | Critical |
| E2E-003 | New | failure | SC-001 | FR-NEW-002 | Critical |
| E2E-004 | New | failure | SC-002 | FR-NEW-003 | Critical |
| E2E-005 | New | edge | SC-002 | FR-NEW-003 | High |
| E2E-006 | New | edge | SC-001 | FR-MOD-001 | High |
| E2E-007 | New | failure | SC-001 | FR-NEW-001 | Medium |
| E2E-008 | New | side effect | SC-001, SC-002 | FR-NEW-001, FR-NEW-003 | Medium |
| E2E-009 | New | failure | SC-001 | FR-NEW-001 | High |
| E2E-010 | New | failure | SC-002 | FR-NEW-003 | High |
| E2E-011 | New | failure | SC-001 | FR-NEW-002 | High |

Coverage: happy 1, failure 6, side effect 1, edge 2, state transition 0 (no transition changes), security 0 (not a security change). Happy to failure ratio 1:6.

### 9.2 New tests
#### E2E-001: publishing still works
- **Category:** happy. **Scenario:** SC-001. **Requirements:** FR-NEW-001, FR-NEW-002
- **Given** Alice, local tags `v1.0` and `v1.1` not yet on a reachable remote. **When** she publishes `v1.0`, then all tags. **Then** both succeed with the answers they gave before. **And** the remote holds both tags.
#### E2E-002: one tag, remote down
- **Given** Alice, local tag `v1.0`, a remote that does not exist. **When** she publishes `v1.0`. **Then** the refusal starts `push of tag 'v1.0' to host '<host>' failed: ` and its failure is `internal error`. **And** the text contains no `fetch`.
#### E2E-003: all tags, remote down
- **Given** Alice, tags `v1.0`, `v1.1` due, a remote that does not exist. **When** she publishes all tags. **Then** the refusal starts `push of tags to host '<host>' failed: `. **And** the audit log is unchanged.
#### E2E-004: remote tag vanished
- **Given** remote tag `v1.0` at `f6a7b8c`, Alice confirmed replacing it with a local `v1.0` at `a1b2c3d`. **When** the remote tag is deleted before her push goes out and her publish continues. **Then** the refusal is `Remote tag 'v1.0' does not exist.` and its failure is `not found`.
#### E2E-005: remote tag moved, not vanished
- **Edge.** **Given** the same, the remote tag is moved to `9e8d7c6` before the push. **Then** the refusal is `Remote tag 'v1.0' has moved to 9e8d7c6 since you last saw it. Fetch, then confirm again.`
#### E2E-006: other operations keep the fetch wording
- **Edge.** **Given** a remote that does not exist. **When** Alice fetches, pulls and deletes the remote tag `v1.0`. **Then** each refusal starts `fetch from host '<host>' failed: `.
#### E2E-007: the connection drops mid push
- **Given** a remote that closes the connection after the listing. **When** Alice publishes `v1.0`. **Then** the refusal starts `push of tag 'v1.0' to host '<host>' failed: ` and its text names the dropped connection.
#### E2E-008: nothing changes on refusal
- **Side effect.** **Given** E2E-002 and E2E-004 set ups. **Then** the remote refs and the audit log equal their state before the call. **And** the local tag `v1.0` still exists.
#### E2E-009: annotated tag message check fails to connect
- **Given** an annotated local tag `v1.0` with the message `Release 1.0`, a remote with `v1.0` that then goes down at the message check. **When** Alice publishes `v1.0` stating `f6a7b8c`. **Then** the refusal starts `push of tag 'v1.0' to host '<host>' failed: `.
#### E2E-010: vanished tag, publish confirmed twice
- **Given** the vanish of E2E-004. **When** Alice repeats the call. **Then** the repeat is not a refusal about a move; the tag publishes as a creation, answering as before (the remote holds no `v1.0`, so no confirmation is needed).
#### E2E-011: publish all, drop after the first tag
- **Given** two due tags and a remote that drops after the first. **When** Alice publishes all with the number 2. **Then** the refusal starts `push of tags to host '<host>' failed: ` and the remote holds only `v1.0`.

### 9.4 Modified tests
#### E2E-MOD-001: the publish all connection assertions (were the publish all connection checks of `e2e_tag_020`, `e2e_tag_022`, `e2e_tag_029`, `e2e_tag_050`, and the drop assertion at `git.rs:29528`)
- **Validated before:** the refusal starts `fetch from host '<host>' failed: `.
- **Validates now:** it starts `push of tags to host '<host>' failed: `, same cause text.
- **Given / When / Then:** as the originals; only the expected prefix changes.

## 10. Traceability
| Scenario | Requirements | Happy | Failure | Edge |
|---|---|---|---|---|
| SC-001 | FR-NEW-001, FR-NEW-002, FR-MOD-001 | E2E-001 | E2E-002, 003, 007, 009, 011 | E2E-006 |
| SC-002 | FR-NEW-003 | E2E-001 (publish after a vanish succeeds, E2E-010) | E2E-004, 010 | E2E-005 |

## 11. Open questions
None.

## 12. Glossary
| Term | Definition | Context |
|---|---|---|
| Publish | send local tags to a tracked remote | one tag or all tags |
| Due tag | a local tag the remote does not hold | publish all |
| Vanished tag | a remote tag deleted after the member confirmed replacing it | SC-002 |

## 13. Implementability gate
| Round | F | A | Verdict |
|---|---|---|---|
| 1 | 0 | 0 | IMPLEMENTABLE |

**Amendments applied:** none.
**No implementation detail outside Section 14:** PASS

## 14. Evidence appendix
### 14.1 Current State evidence
| Claim (Section 2) | Evidence |
|---|---|
| Connection failures read `fetch from host '<h>' failed: <e>`; 11 call sites | `crates/core/src/git/remote.rs:1568-1571`; recounted: 11 `connection_error(` call sites at `:806, :882, :896, :1100, :1115, :1144, :1166, :1169, :1322, :1369, :1389` plus the definition |
| Tag push connection failures reuse it | `crates/core/src/git/remote.rs:1115` |
| A branch push reads `push of branch '<b>' failed: <e>` | `crates/core/src/git/remote.rs:569` |
| The vanished tag reads `has moved to 0000000` (zero id substituted) | `crates/core/src/tools/git_tags.rs:719-725`, `moved` at `:1462-1467` |
| Deleting a gone remote tag reads `does not exist` | `crates/core/src/tools/git_tags.rs:1452-1454` |
| Existing tests assert the old text | `crates/core/src/tools/git.rs:29491-29495` (helper), callers `:29508, :29589, :29660, :30089`; the drop assertion at `:29528` |
| Only codes compared for one tag | `crates/core/src/tools/git.rs:26421-26433` |
Test command: `export MCPFS_TEST_PG_DSN="${MCPFS_TEST_PG_DSN/:5432\//:5433/}"; cargo test -p mcp-fs-core --lib git`

### 14.3 Design inputs
- The publish paths call the shared listing and probe functions in `remote.rs` that also serve fetch and delete; the wording must depend on the calling operation, not the callee (signal: E2E-006 fails if fetch changes).
- `publish_on_repo` substitutes the zero id when the tag is gone (`git_tags.rs:723`); signal: E2E-004 reads `0000000` before the change.
- Publish all lists at three places (`git_tags.rs:1028`, `:1234`, plus the push); all must carry FR-NEW-002; signal: E2E-003 and E2E-011.
- The delete route already maps an absent tag to `remote_tag_absent` (`git_tags.rs:1405-1409`, `:1440-1444`), the text FR-NEW-003 reuses; no change there.
