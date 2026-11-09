# SPEC-0084: Bounded decoding of the tar family on one xz decoder, design

> Spec: sdlc/specs/SPEC-0084-tar-family-bounded-decoding/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-09
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Capped reader, cap selection, memory hook | existing `tools/archive_single.rs` (`read_capped` `:92-108`, `Caps` `:166-199`, `caps` `:204-221`, `LAST_HIGH_WATER` `:56-61`), made `pub(crate)` and shared | n/a (in tree) | DEC-001, DEC-006 |
| `.tar.gz` decoder | `flate2::read::GzDecoder` (single member, unchanged) | `flate2 = "1"` (`Cargo.toml:82`), locked 1.1.10 | DEC-004 |
| `.tar.bz2` decoder | `bzip2::read::BzDecoder` (single stream, unchanged) | `bzip2 = "0.6"` (`Cargo.toml:85`), locked 0.6.1 | DEC-004 |
| `.tar.xz` decoder | the bare xz decoder `archive_single::decoder(SingleFormat::Xz, ..)` (liblzma stream decoder, `CONCATENATED \| TELL_UNSUPPORTED_CHECK`) | `liblzma = "0.4.9"` (`Cargo.toml:88`), locked 0.4.9 (`Cargo.lock`), already a direct dependency (SPEC-0082 DEC-002) | DEC-004 |
| Removed | `lzma-rs` | was `lzma-rs = "0.3"` (`Cargo.toml:87`, `crates/core/Cargo.toml:72`) | DEC-007 |
| Tar parser | `tar` | `tar = "0.4"` (`Cargo.toml:79`), locked 0.4.46, unchanged | n/a |

No new dependency: every crate above is already a direct dependency of `mcp-fs-core`; one is removed. Versions are read from the manifests and `Cargo.lock` (the user's rule for this run); no new library version is introduced, so no context7 lookup was needed.

Skills loaded: `rust` (via `load_dynamic_context`, already present in the session). Effect: edition 2024 workspace, `[workspace.dependencies]` single source of versions, no `unwrap`/`expect` on fallible production paths, CPU work in `spawn_blocking`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` as the gate (AGENTS.md "Quality gate"), tests at the bottom of the module or in a `*_e2e.rs` test module.

## 2. Approaches

### Chosen: decode the compressed stream through the shared capped reader, then parse
`extract_archive` (`crates/core/src/tools/archive.rs:143-311`) reads the stored archive and today decodes it whole in `decode_tar` (`:720-793`, `read_to_end` per codec at `:727-749`). The chosen design sizes a decoding bound before reading (the lower of the project side and the session side, plus 1 048 576 bytes), decompresses with `archive_single::read_capped` (64 KiB chunks, stops the moment the output would pass the cap), and only then parses the decompressed bytes with `tar::Archive` exactly as today. `.tar.xz` uses the bare xz decoder of SPEC-0082. Pros: one capped reader and one cap rule for every compressed format; the parser and every entry rule stay as they are; about 60 production lines. Cons: the parse still copies each entry, so peak memory is up to twice the bound (spec 14.3 accepts it: FR-NEW-001 bounds the decompressed archive). Affected scope: `tools/archive.rs`, `tools/archive_single.rs`, `quota.rs`.

### Rejected: stream the tar parse over the decoder
`tar::Archive::new(decoder)` over a counting reader would hold one entry at a time. Rejected (DEC-001): the entry list must be complete before the destination, collision and admission checks (FR-NEW-005 items 6 to 9 come after the whole decode), so every entry is buffered anyway; a counting wrapper would add a second cap implementation next to `read_capped`.

### Rejected: bound the declared entry sizes
Counting only what the headers declare lets an archive of millions of empty entries, or megabytes after the end marker, exhaust memory (spec D-4, E2E-046). Rejected by FR-NEW-001 ("the sizes the entries declare play no part in the bound").

## 3. Architecture

```
extract_archive (archive.rs)
  membership (door), normalize path, node lookup, folder check        items 1 to 3 (unchanged)
  detect_format, password on tar refused                              item 4 (unchanged)
  Tar(None)            -> read bytes, decode_tar as today (no bound)  FR-NEW-011
  Tar(Gzip|Bzip2|Xz)   -> planned destination (DEC-003)
                          bound = pick_caps(project side, session side, 1 048 576)   (DEC-001, DEC-002)
                          read bytes
                          spawn_blocking: tar_decoder(c, bytes) -> read_capped(.., bound) -> parse
                            CapExceeded{high_water} -> hook, Caps::refusal               item 5
                            Io(e) -> corrupt("tar.gz|tar.bz2|tar.xz", e)                  item 5
                            parse errors, entry rules (unchanged)                         item 6
  destination refusal (unchanged place), collision, admit_paths, writes, finish_extraction  items 7 to 9
```

- `archive_single.rs` keeps the single file flow; its `caps` becomes a thin call to the shared `pick_caps` (framing 0), so the bare formats keep their behavior byte for byte (`archive_single.rs:204-221`).
- `quota.rs` gains `stored_under(client, folder)`: the sum of the sizes of the file nodes of `meta.subtree(folder)` (`storage/meta.rs:1868`, already selects a path and everything below it with `SubtreeMatch`).
- The memory hook `LAST_HIGH_WATER` stays the one thread local of `archive_single.rs:56-61`; the tar path sets it on the async thread after the blocking task returns, exactly where the single flow sets it (`:269`, `:275`).
- Bounded contexts: archive extraction (`tools/archive*.rs`) asks project storage (`quota.rs`, `safety.rs`) for the remaining storage, the stored size under a folder and the session allowance; it never charges during the bound computation (charging stays in `admit_paths`, `quota.rs:136-159`).

## 4. Data model

None. No persisted change; `meta.subtree` is an existing read.

## 5. Contracts and interfaces

- `pub(crate) const TAR_FRAMING: u64 = 1_048_576;` in `archive.rs` (FR-NEW-002).
- `archive_single.rs`: `pub(crate) struct Caps { pub(crate) limit: u64, pub(crate) which: CapKind }`, `pub(crate) enum CapKind { Project, Session }`, `pub(crate) async fn refusal(self, state, client) -> Result<ToolError>` (unchanged body: `requested` = `limit + 1`, `used` read at refusal time), new `pub(crate) fn pick_caps(project: Option<u64>, session: u64, framing: u64) -> Caps` (`Some(p) if p <= session` gives `Project` with `limit = p + framing`, else `Session` with `limit = session + framing`; saturating adds), `read_capped`, `CappedRead`, `ReadError`, `decoder` become `pub(crate)`.
- `quota.rs`: `pub(crate) async fn stored_under(client: &VolumeClient, folder: &str) -> Result<i64>`.
- `archive.rs`: `fn planned_destination(state, format, normalized, suffix_len, destination) -> Option<String>` (DEC-003); `fn tar_decoder<'a>(c: TarCompression, bytes: &'a [u8]) -> std::io::Result<Box<dyn Read + 'a>>`; `decode_tar` takes the already decompressed bytes and the format name.
- Error contract (unchanged codes and texts): bound on the project side `ERR_PROJECT_QUOTA_EXCEEDED` `project storage quota of <L> MB exceeded: <U> bytes used, <R> bytes requested` with `<R>` = bound + 1 (`ToolError::project_quota_exceeded`, `errors.rs:103-111`); bound on the session side `ERR_WRITE_QUOTA_EXCEEDED` `session write quota of <N> bytes exceeded`; corrupt `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid <format> file: <detail>` (`archive.rs:438-442`), `<format>` `tar.gz` / `tar.bz2` / `tar.xz`.
- `TOOL_CONTRACT.txt` and `tool-contract-golden.json`: unchanged (no parameter description changes).

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive.rs` | modify: xz arm on the liblzma decoder; bounded tar decode in `extract_archive`; `planned_destination`; `TAR_FRAMING`; later the `.txz` fixture of `e2e_new_002_six_formats_extract_default_destination` (`:1993-1995`) | FR-MOD-002, FR-NEW-001 to FR-NEW-005, FR-MOD-001 | US-0183, US-0184, US-0186 |
| `crates/core/src/tools/archive_single.rs` | modify: `decoder`, `read_capped`, `Caps`, `CapKind`, `ReadError`, `CappedRead` `pub(crate)`; `pick_caps`; the `.tar.xz` fixture of `tar_xz_is_a_tar` (`:2116-2121`) | FR-NEW-001 to FR-NEW-003, FR-MOD-002 | US-0183, US-0184, US-0186 |
| `crates/core/src/quota.rs` | modify: `stored_under` | FR-NEW-002 | US-0184 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod archive_testkit;` and `#[cfg(test)] mod archive_tar_bound_e2e;` | test layout (DEC-008) | US-0183 |
| `crates/core/src/tools/archive_testkit.rs` | create: shared test helpers of SPEC-0084 to SPEC-0087 | test layout (DEC-008) | US-0183, US-0184 |
| `crates/core/src/tools/archive_tar_bound_e2e.rs` | create: the `e2e_0084_*` tests | every requirement | US-0183 to US-0186 |
| `crates/core/src/api/dataplane.rs` | modify: `e2e_0084_007` (HTTP door) | FR-NEW-011 | US-0187 |
| `Cargo.toml`, `crates/core/Cargo.toml`, `Cargo.lock` | modify: drop `lzma-rs` | FR-MOD-002 (DEC-007) | US-0186 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-011) | US-0187 |

Affected existing tests: `e2e_new_002_six_formats_extract_default_destination` in `archive.rs:1968` (its `.txz` leg at `:1993-1995`) and `tar_xz_is_a_tar` helper behind `e2e_0082_005_tar_xz_is_still_a_tar`, `e2e_0082_006_txz_is_still_a_tar`, `e2e_0082_047_tar_xz_is_still_a_tar` (`archive_single.rs:2106-2145`) keep their assertions; only their fixture compressor changes from `lzma_rs::xz_compress` to `liblzma::write::XzEncoder` (US-0186). Every other existing tar test (success, corrupt gzip, unsafe paths, links, passwords, suffixes; spec 14.1 row 11) keeps its assertions. No existing test is removed.

Affected documentation: `README.md:283-320` (formats paragraph: `.tar.xz` no longer "pure Rust", bound on compressed tar), `AGENTS.md:37` stack line (`lzma-rs` gone, liblzma for every xz), `AGENTS.md:134-140` (`archive.rs` entry), `.agent_docs/tools.md:174-265` (archive section, order of refusals, a dated SPEC-0084 paragraph), `.agent_docs/testing.md:432-435` (the `e2e_0084_` filter, `archive_testkit.rs`, the hook).

## 7. Compatibility and risks

- Observable changes (declared by the spec): a compressed tar bomb is refused at the bound with the limit error instead of being decoded whole (and before a collision or a link entry it also holds); bytes after the end marker count; `.tar.xz` accepts several streams, padding, branch filters and SHA-256.
- Single file formats: `caps` keeps its exact rule (framing 0, tie to the project) through `pick_caps`; their 160 tests guard it (`cargo test -p mcp-fs-core --lib -- e2e_0081_ e2e_0082_ e2e_0083_`).
- Ordering inside this spec: US-0183 (decoder) before US-0184 (bound) because the bound reads the xz stream through the new decoder; US-0186 (drop `lzma-rs`) after US-0183 because production stops calling it there.
- Ordering across the run (decided once for SPEC-0084 to SPEC-0087, DEC-010): SPEC-0084, then SPEC-0085, then SPEC-0086, then SPEC-0087.
- Dependencies: `lzma-rs` removed (and the packages only it pulled leave `Cargo.lock`); nothing added. Licences unchanged.
- Rollback: revert the story commits; the manifests return with US-0186's revert.

## 9. Test strategy

Every acceptance test is a functional e2e test through the typed engine call `extract_archive` (the function both doors call, `mcp/server.rs:2370-2380`, `api/dataplane.rs:878-902`), after `f.state.authorize(MOUNT, person)` as both doors do, except E2E-007 which goes through the HTTP door (`Harness::new()` then `h.post(&u("extract-archive"), json!({...}))`, `api/dataplane.rs:1617-1720`). Test names follow the repository rule: `e2e_0084_<nnn>_<title slug cut to 36 characters>`; filter `e2e_0084_`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-002 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-003 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-004 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-005 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-006 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-007 | functional e2e | HTTP door | crates/core/src/api/dataplane.rs tests | US-0187 |
| E2E-008 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-009 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-010 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-011 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-012 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-013 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-014 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-015 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-016 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-017 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-018 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-019 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-020 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-021 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |
| E2E-022 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-023 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-024 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-025 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-026 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-027 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-028 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-029 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-030 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-031 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-032 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-033 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-034 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-035 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-036 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-037 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-038 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0183 |
| E2E-039 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-040 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-041 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-042 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0186 |
| E2E-043 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-044 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-045 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0185 |
| E2E-046 | functional e2e | typed engine call | crates/core/src/tools/archive_tar_bound_e2e.rs | US-0184 |

Test data: generated in the tests (DEC-009); no committed fixture file.

## 10. Decisions log

- **DEC-001:** the bound reuses the single file machinery: `read_capped` (64 KiB chunks, refuses as soon as the output would pass the cap, equality accepted) and one cap selector `pick_caps(project: Option<u64>, session: u64, framing: u64) -> Caps` that both the single flow (framing 0) and the tar flow (framing `TAR_FRAMING` = 1 048 576) call; `Caps::refusal` reports the project refusal with `requested` = cap + 1 and `used` read at refusal time, or the session refusal. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-002, FR-NEW-003. **Rationale:** one cap rule and one reader for every compressed format; `requested = limit + 1` is exactly FR-NEW-003's `<R>` = bound + 1 once the framing is inside the cap. **Alternatives considered:** a second counting reader in `archive.rs`; streaming the tar parse (rejected in section 2). **Source:** existing code. **Evidence:** `crates/core/src/tools/archive_single.rs:92-108`, `:166-221`.
- **DEC-002:** the project side is `quota::project_remaining_for(client, safety, stored)` where `stored = quota::stored_under(client, dest)` (sum of `size` over the file nodes of `meta.subtree(dest)`), for `overwrite` true and false; `stored = 0` when the planned destination is `None` (refused). No project limit: `None`, the session side alone sets the cap. The session side is `safety.session_remaining(person, mount_id)`. **Obliged by:** SPEC-0084/FR-NEW-002 (and its business rule on a refused destination). **Rationale:** `meta.subtree` already selects a folder and everything below it with the escaped `LIKE` plus exact match (`SubtreeMatch`, AGENTS.md conventions), so no new SQL; `project_remaining_for` already adds a replaced size and floors at 0. **Alternatives considered:** `folder_usage` (direct items only, needs a cap); a new SUM query (new SQL across two dialects for no gain). **Source:** existing code. **Evidence:** `crates/core/src/quota.rs:165-178`, `crates/core/src/storage/meta.rs:1868`, `crates/core/src/safety.rs:151-154`.
- **DEC-003:** `planned_destination(state, format, normalized, suffix_len, destination) -> Option<String>` resolves the destination before reading with today's rules (RAR: `archive_single::normalize_destination`; zip, 7z, tar: `state.safety.normalize_path`; none given: the archive path minus its suffix) and returns `None` instead of an error; it only sizes the bound. The destination refusal keeps its place after the entry checks (FR-NEW-005 item 7), computed as today at `archive.rs:219-227`. **Obliged by:** SPEC-0084/FR-NEW-002, FR-NEW-005. **Rationale:** the bound needs the destination before decoding while its refusal must not move (spec 14.3); SPEC-0086 later moves the refusal and removes this helper (SPEC-0086 DEC-002). **Alternatives considered:** moving the refusal now (changes FR-NEW-005 item 7, an observable order this spec keeps). **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:213-227`.
- **DEC-004:** decoders: `.tar.gz`/`.tgz` `flate2::read::GzDecoder`, `.tar.bz2`/`.tb2` `bzip2::read::BzDecoder` (single member and single stream, as today), `.tar.xz`/`.txz` `archive_single::decoder(SingleFormat::Xz, bytes)` (liblzma explicit stream decoder with `CONCATENATED | TELL_UNSUPPORTED_CHECK`, SPEC-0082 DEC-003); a plain `.tar` is not decompressed and gets no bound. Read errors map to `corrupt(<format>, e)`; `ReadError::Init` maps to `ToolError::internal("cannot start the tar.xz decoder: <e>")`. **Obliged by:** SPEC-0084/FR-MOD-002, FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-NEW-011 (spec 3.2: several gzip members or bzip2 streams stay out). **Rationale:** the bare xz decoder already implements every `.tar.xz` rule of the spec (measured in SPEC-0082 DEC-003: several streams, padding of a multiple of 4, BCJ, SHA-256 accepted; `.lzma`, unknown check, junk refused). **Alternatives considered:** `MultiGzDecoder`/`MultiBzDecoder` for the tar family (a behavior change the spec excludes); keeping `lzma-rs` (refuses padding, BCJ, SHA-256, several streams: the defect of BL-0132). **Source:** spec constraint and existing code. **Evidence:** `crates/core/src/tools/archive_single.rs:64-86`, `crates/core/src/tools/archive.rs:727-749`, `liblzma-0.4.9/src/stream.rs:1320`.
- **DEC-005:** the bounded tar decode runs in one `spawn_blocking` task that decompresses with `read_capped(tar_decoder(c, &bytes)?, bound.limit)` then parses with the existing entry loop; it returns either the entries and the high water, or `CapExceeded { high_water }`, or a `ToolError`. On `CapExceeded` the async side sets the hook and returns `caps.refusal(state, &client).await?`. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-003, FR-NEW-004, FR-NEW-005 (item 5: corrupt or bound, whichever decoding meets first). **Rationale:** the order "corrupt or bound, whichever comes first" falls out of one sequential read; nothing is written, charged or audited before `admit_paths` and `finish_extraction`, which a refusal never reaches (FR-NEW-004). **Alternatives considered:** two tasks (decompress, then parse): one more hop for nothing. **Source:** design. **Evidence:** `crates/core/src/tools/archive_single.rs:254-277`.
- **DEC-006:** "decompressed bytes held" is read through the one existing test hook `archive_single::LAST_HIGH_WATER` (`cfg(test)` thread local), set on the async thread after the blocking task returns, with `high_water` of the capped read (the bytes held when the read stopped, never above the cap) or the decompressed length on success; a call refused before decoding never sets it. Tests reset it before the call and read it with `held()` (0 when unset) or `held_some()` (fails when unset: the decoder never ran). **Obliged by:** SPEC-0084/FR-NEW-001, NFR 7.1 (E2E-008 to E2E-011, E2E-018, E2E-046). **Rationale:** one hook for every format (`.agent_docs/testing.md:432-434`); the tokio test runtime is single threaded, so the thread local belongs to one test; setting it on the async thread avoids the `spawn_blocking` thread mismatch that hides per thread captures (US-0175 commit note). **Alternatives considered:** a second hook in `archive.rs`; process RSS (needs `unsafe` or a `ps` race, SPEC-0082 DEC-006). **Source:** existing code. **Evidence:** `crates/core/src/tools/archive_single.rs:56-61`, `:862-866`.
- **DEC-007:** `lzma-rs` is removed from `Cargo.toml` `[workspace.dependencies]` and `crates/core/Cargo.toml`; the two test fixtures that used `lzma_rs::xz_compress` (`archive.rs:1993-1995`, `archive_single.rs:2116-2121`) use `liblzma::write::XzEncoder::new(Vec::new(), 6)`, assertions unchanged. **Obliged by:** SPEC-0084/FR-MOD-002 ("one xz rule"; production no longer calls `lzma-rs` after US-0183). **Rationale:** a production dependency used by tests only is dead weight in every build; the spec names the removal as a design decision (14.3). **Alternatives considered:** keeping it as a dev dependency (still two xz encoders in the tree); keeping it as is. **Source:** spec design input. **Evidence:** `rg -n 'lzma_rs|lzma-rs'` (5 hits: two manifests, `archive.rs:745`, `:1994`, `archive_single.rs:2120`).
- **DEC-008:** test layout: the `e2e_0084_*` tests live in a new test module `crates/core/src/tools/archive_tar_bound_e2e.rs` (`#[cfg(test)] mod archive_tar_bound_e2e;` in `tools/mod.rs`, precedent `quota_e2e.rs`, `diff_files_e2e.rs`); helpers shared by SPEC-0084 to SPEC-0087 live in `crates/core/src/tools/archive_testkit.rs` (`#[cfg(test)] pub(crate) mod archive_testkit;`): `MOUNT = "atlas"`, `OWNER = "mia@example.com"`, `fixture()`, `fixture_allowance(n)`, `seed`, `extract(f, path, destination, overwrite, password)` (authorizes then calls `extract_archive`), `read`, `exists`, `limit_remaining(f, n)` (exact bytes), `remaining_storage`, `allowance`, `held_reset`, `held`, `held_some`, `extraction_audit_lines(f, path)`, `project_refusal`, `session_refusal`, `corrupt_refusal`. The HTTP test stays with its harness in `api/dataplane.rs`. **Obliged by:** n/a (no code impact): test layout. **Rationale:** `archive.rs` (2 976 lines) and `archive_single.rs` (2 429 lines) already hold their own suites; four specs add about 190 tests sharing the same helpers. **Alternatives considered:** appending to `archive.rs` tests; one helper copy per module. **Source:** existing code. **Evidence:** `crates/core/src/tools/mod.rs:39-91`, `crates/core/src/tools/archive_single.rs:786-866`.
- **DEC-009:** fixtures are generated in the tests: tar bytes with `tar::Builder` and GNU headers (`archive.rs:1297-1311` pattern), gzip `flate2::write::GzEncoder`, bzip2 `bzip2::write::BzEncoder`, xz `liblzma::write::XzEncoder::new(_, 6)` (CRC64), x86 and SHA-256 and CRC32 variants through `liblzma::stream::Stream::new_stream_encoder` with `Filters` (SPEC-0082 DEC-007 recipe), legacy `.lzma` through `Stream::new_lzma_encoder(&LzmaOptions::new_preset(6)?)`, the reserved check type 3 by patching the stream flags byte at offset 7 and `len - 3` and recomputing both CRC32 with `flate2::Crc` (SPEC-0082 DEC-007 recipe); several streams are concatenations; the bomb is a tar holding `zeros.bin`, 10 000 000 zero bytes, declared as such; the exact size archive is the tar bytes of `a.txt` (2 048 bytes: header, one data block, two end blocks) extended with zero bytes to exactly 1 049 576 (E2E-014) or 1 049 577 (E2E-015) before compression; the lying entry is one GNU header declaring 1 000 000 000 followed by the 5 bytes `hello` and nothing else. **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-006 to FR-NEW-010 (spec section 9: fixtures built by the design). **Rationale:** no committed binary, no external tool; every recipe already proven in SPEC-0082. **Alternatives considered:** committed fixture files. **Source:** existing code and SPEC-0082 DEC-007. **Evidence:** `crates/core/src/tools/archive_single.rs:1700-1720`, `:2319-2322`.
- **DEC-010:** run order across SPEC-0084 to SPEC-0087, one global chain US-0183 to US-0204, each story depending on the previous one: SPEC-0084 (US-0183 to US-0187), SPEC-0085 (US-0188 to US-0193), SPEC-0086 (US-0194 to US-0197), SPEC-0087 (US-0198 to US-0204). **Obliged by:** n/a (no code impact): story order. **Rationale:** SPEC-0086 states it depends on SPEC-0084 and SPEC-0085 and its E2E-MOD-001 rewrites SPEC-0085 E2E-028, which must exist first; SPEC-0085 and SPEC-0084 both need the destination before decoding (this spec's DEC-003 helper, reused by SPEC-0085) and both rewrite the multi format pipeline of `extract_archive`, so one writer at a time; SPEC-0087 depends only on SPEC-0080 (built) and puts the truncation at item 5, before SPEC-0085's declared limits (its E2E-040 holds in either order), so it goes last: it has the highest risk (header decryption, tier 1 stories) and the other three never wait on it. **Alternatives considered:** SPEC-0087 before SPEC-0086 (equally valid, but `archive_rar.rs` would then change under SPEC-0085's RAR listing split and SPEC-0087's walk in alternation); parallel branches (several writers on `archive.rs`). **Source:** spec dependency statements. **Evidence:** SPEC-0086 front block `Depends on`, SPEC-0086 9.4, SPEC-0087 2.2, SPEC-0085 FR-NEW-009 item 5.
- **DEC-011:** documentation, in the closing story: `README.md` archive paragraph (`.tar.xz` decoded by liblzma, compressed tar decoding stops at the bound), `AGENTS.md` stack line (`lzma-rs` removed) and `archive.rs` entry, `.agent_docs/tools.md` archive section (order of refusals for compressed tar, a dated `(2026-11-09, SPEC-0084.)` paragraph), `.agent_docs/testing.md` (filter `e2e_0084_`, `archive_testkit.rs`, the hook). **Obliged by:** n/a (no code impact): project convention (AGENTS.md "keep README.md and AGENTS.md up to date"). **Rationale:** `README.md:295` says the tar family is "all decoded in pure Rust", false once `.tar.xz` uses liblzma. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `README.md:294-305`, `AGENTS.md:37`, `.agent_docs/tools.md:181-217`.
- **DEC-012:** E2E-024 carries one derived assertion beyond its spec text: the decompressed bytes held stay at or below 1 148 576 (the bound: 100 000 + 1 048 576). **Obliged by:** SPEC-0084/FR-NEW-001, FR-NEW-002. **Rationale:** its spec assertions (`ERR_PROJECT_QUOTA_EXCEEDED`, nothing written) are also produced by the later admission on declared sizes, so without it no mutation of the bound is observable (US-0181 and US-0172 commit notes: a listed mutation must really kill the test). **Alternatives considered:** recording E2E-024 as a guard with no mutation. **Source:** design. **Evidence:** `crates/core/src/tools/archive.rs:247-262`.
- **DEC-013:** stories US-0185 and US-0186 hold tests that turn green with an earlier story's code (green on arrival); each is proven by a mutation reasoned to change that test's observable outcome, listed in the story. **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182. **Alternatives considered:** fewer, larger stories over the tier 2 budget. **Source:** design. **Evidence:** `git log` of US-0180 to US-0182.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0183 | decode a .tar.xz with the bare xz decoder | P1 | SPEC-0084/FR-NEW-006, FR-NEW-007, FR-NEW-008, FR-NEW-009, FR-MOD-002 | main | 2 | no |
| US-0184 | stop a compressed tar bomb at the decoding bound | P1 | SPEC-0084/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-004, FR-MOD-001 | US-0183 | 2 | no |
| US-0185 | order the bound among the tar refusals and re extract near the limit | P1 | SPEC-0084/FR-NEW-002, FR-NEW-005, FR-NEW-010, FR-MOD-001 | US-0184 | 2 | no |
| US-0186 | one xz library and the other formats unchanged | P2 | SPEC-0084/FR-NEW-011, FR-MOD-002 | US-0185 | 2 | no |
| US-0187 | tar bound through the HTTP door and documentation | P2 | SPEC-0084/FR-NEW-011 | US-0186 | 2 | yes |
