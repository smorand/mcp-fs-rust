# SPEC-0077 design issues

## 2026-11-02: DR-003 and DT-011 are false against the current code (a perimeter hole)
- Spec: SPEC-0077, Requirement(s): DR-003, tests DT-010 and DT-011.
- Problem: DR-003 says non test code of `crates/core/src/tools/git.rs` "SHALL contain `.reference(` only inside `sync_disk_ref`", and DT-011 asserts `.reference(` appears exactly once. Recounted: `rg -n '\.reference\(' crates/core/src/tools/git.rs` finds 12 matches before the test module (`mod tests` at `:7767`): ten discarded writes, `sync_disk_ref` at `:2399`, and an eleventh, `:3716`, which is not discarded.
- Evidence: `crates/core/src/tools/git.rs:3716-3721`: `repo.reference(&format!("refs/heads/{branch_owned}"), oid, true, "mcp-fs git.remote_push").map_err(|e| git_err("sync local branch ref", e))?;` in `push_branch`. The comment above it says the push names the branch by ref, so a failed write must fail the push. Routing it through `sync_disk_ref` would turn a refusal into a warning (a behavior change), and leaving it makes DT-011 fail on arrival with the story unable to turn it green.
- Needed: the spec owner decides how DR-003 and DT-011 read. The perimeter and the ten sites of Section 2 are right.
- Recommendation: amend DR-003 to "SHALL NOT contain `let _ = repo.reference(`, and SHALL contain `.reference(` only inside `sync_disk_ref` and at the checked write of `push_branch`", and DT-011 to count exactly two `.reference(` calls with the second one followed by `.map_err(`. Add the site to Section 2 as "checked, unchanged" (11 matches in non test code). No other requirement moves.
