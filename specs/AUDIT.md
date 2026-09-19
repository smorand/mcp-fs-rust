# Cross-Spec Implementability Audit — S1 through S8

> Run on: 2026-09-18
> Scope: the eight retro-specifications in `specs/`
> Method: mechanical verification (`specs/audit.py`, `specs/check.py`) plus a targeted reading pass
> Verdict: **IMPLEMENTABLE-WITH-DRIFT**

## Why this replaces eight separate Phase 6 rounds

The `/spec-feat` contract asks for one fresh-context auditor sub-agent per specification, briefed with the file path and the audit contract only. Sub-agent execution was unavailable throughout this work: every spawn failed immediately with a provider budget error (`429 budget_exceeded`, team budget 750.0 exceeded), producing output files containing the prompt and the error and no work at all.

This audit is therefore run in-process, and it is weaker than the contract intends in exactly one respect: **it is not ignorant of the specs' authorship**. A fresh auditor's value comes from having to reconstruct intent from the file alone. That property is not reproduced here and this document does not pretend otherwise.

What it does reproduce, and in some respects exceeds, is the factual half the contract puts first: open every `file:LINE`, recount every count, hunt for claims about the code that are wrong. Those checks are mechanical, exhaustive rather than sampled, and re-runnable.

## Mechanical results

```
CITATIONS:     686 checked, 686 resolve to a real file and a real line
ID UNIQUENESS: no collisions across 8 specs
COVERAGE:      86/99 source modules claimed by at least one spec
CONSISTENCY:   8/8 specs pass the Step 4.5 gate
```

Per-spec citation density: foundation 254, engine 125, documents 64, git 61, storage 53, plane 52, integrations 42, rag 35.

Re-run with `python3 specs/audit.py` and `python3 specs/check.py specs/*.md`.

## Findings

Classified per the contract: **F** is a functional gap where two competent implementing agents would produce observably different results, and it blocks. **A** is alignment drift where the spec is right about the behaviour and wrong about the current code, and it does not block once registered.

### F findings: 1 found, 1 closed

**F-1 (G7, unresolved prerequisite) — `keys.rs` had no requirement.**
S1 SC-001 named the `mcp-fs keys` and `mcp-fs token` verbs and relied on their output, but no requirement said what they produce. Two implementing agents would have chosen different key file names, a different default directory, a different issuer and a different token lifetime, all of them user-observable and all of them load-bearing for authentication to work at all.

*Closed* by adding **S1 FR-044**, which fixes `.keys` as the default directory, `jwt.key` and `jwt.pub` as the file names, `web-a2a` as the issuer, `email` as the claim and 3600 seconds as the TTL, each cited at `keys.rs:22-32`, together with the observation that these are exactly the `auth.jwt` defaults so a fresh keypair works against a default config. Three tests added: E2E-115, E2E-116, E2E-117.

### A findings: 6 found, 6 amended in place

Every one was evident, meaning the correct value could be written from a citation opened during the audit and the correction changed no requirement's intent. None needed a Section 19 register entry.

| # | Finding | Evidence | Correction |
|---|---|---|---|
| A-1 | S1 cited `state.rs:63-66` for `authorize` five times; the file has 64 lines | `state.rs:61-63` | citation corrected in all five places |
| A-2 | S1 §8 and S4 E2E-431 gave the local blob layout as `{root}/{sha[..2]}/{sha}`, omitting the bucket component | `storage/blob/local.rs:4,24,33` | corrected to `{dir}/{bucket}/{sha[..2]}/{sha}` in both specs |
| A-3 | S5 §9.3 and TBD-504 claimed 17 editor tests, taken from `BACKLOG.md` and explicitly labelled `ASSUMED:` | 25 test functions in `tools/editor.rs` | corrected to 25; `BACKLOG.md` recorded as stale |
| A-4 | S4 described the serialized SQLite layer with no citation | `storage/sqlite.rs:1-6` | citation added to SC-401 and FR-414 |
| A-5 | S1 FR-024 named `util::normalize_identity` with no citation | `util/mod.rs:10` | citation added |
| A-6 | S2, S3 and S5 left the two MIME implementations as an open question across three TBDs | both tables hold 32 extensions with identical values | resolved to a stated fact in all three specs |

**A-6 deserves its own paragraph**, because it is the finding that changed most. Three specs carried an open question about whether `core::fs_ops::mime_guess` and `docs::guess_mime` agree. The audit extracted both tables and compared them: 32 extensions each, identical keys, identical values, zero disagreements. So there is no live inconsistency. Two things remain true and are now stated rather than suspected: the duplication is a standing regression risk with nothing keeping the tables in step, and the module comment at `docs/mime.rs:2` claims that table serves `fs.read_bytes`, which the code contradicts. S5's E2E-548 is written to fail the moment the two diverge.

## Coverage gap analysis

86 of 99 source modules are claimed by at least one specification. The 13 unclaimed modules are accounted for rather than overlooked:

| Modules | Count | Status |
|---|---|---|
| `crates/agent/src/*` | 8 | Deliberately out of scope. The agent is an MCP client, not the server product; deferred as `BACKLOG.md` BL-002. |
| `api/mod.rs`, `core/mod.rs`, `git/oauth/mod.rs`, `storage/blob/mod.rs` | 4 | Pure module declarations: **0 substantive lines each**, only `mod` and `use` statements. Nothing to specify. |
| `main.rs` | 1 | 4 substantive lines; the entry point delegates to `cli.rs`, which S1 FR-001 and FR-004 specify. |


No production module with behaviour of its own is unspecified.

## What this audit did not do

Stated plainly, because an audit that overstates its reach is worse than none:

1. **It did not verify that every cited line *says* what the spec claims.** 686 citations resolve to real lines; a targeted reading pass checked the load-bearing ones, particularly every asserted count. A full semantic pass over all 686 is the work a fresh-context auditor would do.
2. **It did not simulate an ignorant reader.** The contract's central test, whether an agent with only the file can implement without guessing, cannot be self-administered. When sub-agents become available, running the real Phase 6 contract against each spec remains worthwhile, and the registered A findings and closed F finding mean it starts from a better baseline.
3. **It did not run the specified tests.** None of the 451 specified tests exists yet; §14 of each spec orders that work. The specs describe current behaviour and the mechanical checks confirm they describe it accurately, but only the test suite converts that into a maintained guarantee.

## Verdict

**IMPLEMENTABLE-WITH-DRIFT**, with zero unresolved F findings and zero unregistered A findings: all six A findings were evident and amended in place, which is what the contract prescribes in preference to a register entry.

Every count asserted about the code was recounted during this audit and is correct as written: 14 error codes, 45 always-on tools (35 `fs.*` + 10 `admin.*`), 38 REST routes, 14 git tools, 3 git tables, 10 tree-sitter grammars, 20 optional-family tools (5 web, 2 context7, 8 sqlite, 5 db), 4 search tools, 32 MIME extensions in each of two tables, 25 editor tests, 99 source modules.

## Corpus summary

| Spec | Scenarios | Requirements | Tests |
|---|---|---|---|
| S1 platform foundation | 10 | 44 | 117 |
| S2 filesystem engine | 12 | 34 | 90 |
| S3 REST data plane | 5 | 20 | 45 |
| S4 multi-backend storage | 5 | 27 | 42 |
| S5 documents | 6 | 27 | 48 |
| S6 git | 5 | 29 | 41 |
| S7 search and RAG | 5 | 21 | 36 |
| S8 optional integrations | 5 | 19 | 32 |
| **Total** | **53** | **221** | **451** |

## Follow-up work this audit identified

1. `BACKLOG.md` at the repository root states 17 editor tests; the count is 25. A documentation fix.
2. `docs/mime.rs:2` names `fs.read_bytes` as its caller; the caller is the REST download route. A comment fix.
3. The two MIME tables should be collapsed into one, or a test should pin their equality. S5 E2E-548 is specified to do the latter.
4. Run the real Phase 6 contract, one fresh sub-agent per spec, once the provider budget allows.
