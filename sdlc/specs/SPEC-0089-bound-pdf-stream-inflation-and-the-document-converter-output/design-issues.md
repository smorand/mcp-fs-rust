# SPEC-0089 design issues

## 2026-11-10: E2E-003 fixture arithmetic (non blocking, designed through DEC-028)
- Spec: SPEC-0089, Requirement(s): FR-NEW-014, E2E-003
- Problem: the Given reads "a text mode stand in converter answering `# Deck` followed by 1 990 bytes of the letter `d`" (6 + 1 990 = 1 996 bytes) while the Then reads "the answer names `/docs/deck.md` with 2 000 bytes written".
- Evidence: `sdlc-spec.md` section 9.2, E2E-003.
- Needed: amend the Given to "followed by 1 994 letters `d`" (or the Then to 1 996) at the next `/sdlc-spec` touch of this spec.
- Recommendation: keep the asserted 2 000 and fix the Given; the design already sizes the fixture to 2 000 bytes (DEC-028). The spec was not held: the observable behaviour (bytes written equal the answer's length) admits one reading.
