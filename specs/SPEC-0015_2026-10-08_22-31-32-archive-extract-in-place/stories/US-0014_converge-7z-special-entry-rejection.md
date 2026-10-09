---
Parent Spec: SPEC-0015
Spec ID: SPEC-0015
Status: ready
Depends On: US-0012
min_tier: 2
Files touched: crates/core/src/tools/archive.rs
---
# US-0014 [converge] 7z entries positively identified as non-regular are rejected

Converge round 1 gap: FR-NEW-013's 7z clause is not implemented. DEC-008 allows lower fidelity
(an entry that cannot be positively identified is a regular file), not zero detection. When a 7z
entry carries the unix extension flag (attributes & 0x8000) and its POSIX mode
((attributes >> 16) & 0xF000) is not regular (0x8000) nor directory (0x4000), the whole call fails
with `ERR_NOT_SUPPORTED` naming the entry and its type, nothing written. Test with a fixture whose
entry is marked symlink (0xA000) next to a benign entry; a 7z entry without the unix flag is still
extracted as a regular file.
