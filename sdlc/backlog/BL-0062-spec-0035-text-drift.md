# BL-0062: SPEC-0035 text no longer matches what landed

Nature: DEBT (documentation of an implemented spec)
Origin: US-0040, US-0041, SPEC-0035 converge audit (2026-10-17)

Evidence:
- `Cargo.toml` pins `git2 = { version = "0.21", features = ["https", "ssh"] }`; the spec says `git2 = "0.21"`. git2 0.21 has `default = []`, so the bare form drops HTTPS and SSH (guard test `libgit2_is_built_with_the_https_and_ssh_transports`, `crates/core/src/git/remote.rs`).
- DR-005 lists 7 ids; `.cargo/audit.toml` and `deny.toml` also accept RUSTSEC-2026-0367/0368/0369 (zip, owner acceptance, until BL-0058).
- DR-007 and DEC-004 expect 2 yanked zip warnings; zip 2.4.2 and 3.0.0 were un-yanked, audit shows 0.
- DT-011 says only git2 and yoke-derive move in Cargo.lock; cargo also moved tempfile's getrandom 0.4.3 to 0.3.4 and added a serde_json dev-dependency for crates/mcp-fs.
- Section 8.4 lacks `dir_node_nests_by_path_segment` (`Oid::zero()` to `Oid::ZERO_SHA1`, deprecated in 0.21, same value).

Proposal: an as-built amendment note on SPEC-0035 (same route as BL-0054, the pipeline has no supersede step yet).
