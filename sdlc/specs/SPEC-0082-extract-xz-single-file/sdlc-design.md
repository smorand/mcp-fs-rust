# SPEC-0082: Extract a bare xz file, design

> Spec: sdlc/specs/SPEC-0082-extract-xz-single-file/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-08
> Target tier: 2 (default, no `--tier`, no constitution tier, no `.spec.json`)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| Xz reader | `liblzma::read::XzDecoder::new_stream` over `liblzma::stream::Stream::new_stream_decoder(u64::MAX, CONCATENATED \| TELL_UNSUPPORTED_CHECK)` | `liblzma = "0.4.9"` (crates.io max stable 0.4.9, checked 2026-11-08; context7 `/websites/rs_liblzma_liblzma`), new DIRECT dependency, default features; already locked at 0.4.9 through `datafusion` | DEC-002, DEC-003 |
| Single file flow, caps, admission, order of refusals | existing `tools/archive_single.rs` (SPEC-0081, SPEC-0083, landed) | n/a | DEC-001 |
| Tar family xz | unchanged: `lzma-rs` 0.3 (`archive.rs:742-746`) | `lzma-rs = "0.3"` (`Cargo.toml:87`) | DEC-009 |

Skills loaded: rust (edition 2024 workspace, `[workspace.dependencies]` single source of versions, `make check` gate, `cargo deny`/`cargo audit` in `make security`, no `unwrap` on fallible production paths). The project's own quality gate (AGENTS.md: `cargo test --workspace`, clippy `--all-features -D warnings`, `cargo fmt --check`) binds every story.

## 2. Approaches

### Chosen: add `SingleFormat::Xz` to the single file flow, decoded by liblzma
The flow already built for gzip and bzip2 (`extract_single`, `archive_single.rs:212-277`) owns the output name, destination, collision, caps, capped reading in 65 536 byte chunks (`archive_single.rs:14`, `:75-90`), admission, writes and the answer. This spec adds a variant, a table row, a decoder arm and the tests. Pros: about 25 production lines, one implementation of every rule; liblzma streams (stops mid block), decodes concatenated streams and validates stream padding itself. Cons: a C library (vendored xz source or the system liblzma) on the bare `.xz` path, where the other formats are pure Rust. Affected scope: `tools/archive.rs` (table row), `tools/archive_single.rs` (variant, decoder, tests), manifests.

### Rejected: keep `lzma-rs` and hand write a stream splitter plus a chunk header scan
Spec design issue of 2026-11-05 option (c): `lzma-rs` 0.3 holds a whole block in memory and fails on any byte after the first stream (`lzma-rs-0.3.0/src/decode/xz.rs:86-90`, `:226-285`). About 150 lines of format parsing to own. Rejected by erratum 1 of the spec (DEC-002).

### Rejected: `XzDecoder::new_multi_decoder`
The convenience constructor builds `Stream::new_auto_decoder(u64::MAX, LZMA_CONCATENATED)` (`liblzma-0.4.9/src/bufread.rs:185-188`): an auto decoder that also accepts legacy `.lzma` (LZMA alone) data, and that silently skips a check type it cannot verify. Measured: a `.lzma` file named `.xz` decodes to `hello` with it, which FR-NEW-008 ("is not xz") refuses (DEC-003).

## 3. Architecture

Unchanged from SPEC-0081 and SPEC-0083. `extract_archive` (`archive.rs:169`) refuses a password for every `Single(_)` format by name, then hands a `Single(format)` to `archive_single::extract_single` (`archive.rs:186-198`). In `extract_single` the order is: output name (`archive_single.rs:224`), destination normalization and check (`:226-231`), collision (`:233`), caps (`:235`, `caps` at `:187-205`), read the archive (`:236`), decode under the cap in `spawn_blocking` (`:239-241`), corrupt or limit refusal (`:244-251`), admission (`:257-259`), writes (`:261-265`), `finish_extraction` (`:267-276`).

```
extract_archive (archive.rs)
  detect_format: ".xz" -> Single(Xz)          (new row, after .tar.xz/.txz and .bz2)
  password? -> ERR_INVALID_ARGUMENT            (existing, archive.rs:169)
  extract_single (archive_single.rs)
    name, destination, collision, caps         (existing)
    spawn_blocking:
      decoder(Xz, bytes)?  -> XzDecoder::new_stream(bytes, Stream::new_stream_decoder(MAX, CONCATENATED | TELL_UNSUPPORTED_CHECK))
      read_capped(reader, cap)                 (existing, 64 KiB chunks, stops at cap)
    Io error -> corrupt("xz", e)               (existing mapping)
    admission, writes, answer, audit           (existing)
```

`decoder` becomes fallible because `Stream::new_stream_decoder` returns `Result` (DEC-004); the gzip and bzip2 arms wrap their reader in `Ok`.

## 4. Data model

None. No persisted change.

## 5. Contracts and interfaces

- Table: `(".xz", ArchiveFormat::Single(SingleFormat::Xz))` after `(".bz2", ...)` (`archive.rs:61`); `.tar.xz` and `.txz` stay at `archive.rs:49-50`, before it, so they keep matching first.
- `SingleFormat::Xz`, `label()` returns `"xz"`. Corrupt message: `archive is corrupt or not a valid xz file: <detail>` through `corrupt` (`archive.rs:438-442`). Observed details: `premature eof` (0 byte, truncated, text, junk after a stream), `lzma data error` (flipped byte, padding not a multiple of 4), `stream/file format not recognized` (a `.lzma` file), `liblzma not built with check support` (a check type liblzma does not know).
- `fn decoder(format: SingleFormat, bytes: &[u8]) -> std::io::Result<Box<dyn Read + '_>>`; new `ReadError::Init(std::io::Error)` mapped to `ToolError::internal("cannot start the xz decoder: <e>")` (DEC-004).
- Every other message, parameter and answer key: unchanged (SPEC-0081 design, section 5). `TOOL_CONTRACT.txt` and `tool-contract-golden.json` do not change: the `path` description of `fs.extract_archive` does not list formats.
- Manifests: `Cargo.toml` `[workspace.dependencies]` gains `liblzma = "0.4.9"` next to `lzma-rs = "0.3"` (`Cargo.toml:87`); `crates/core/Cargo.toml` gains `liblzma = { workspace = true }` next to `lzma-rs` (`crates/core/Cargo.toml:72`). Measured lockfile delta (throwaway worktree, `cargo metadata --offline`): one line, `"liblzma"` added to the `mcp-fs-core` dependency list; no package added or upgraded.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `Cargo.toml` | modify: `liblzma = "0.4.9"` | FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-013 (erratum 1) | US-0178 |
| `crates/core/Cargo.toml` | modify: `liblzma = { workspace = true }` | same | US-0178 |
| `Cargo.lock` | modify: one line (generated) | same | US-0178 |
| `crates/core/src/tools/archive.rs` | modify: table row, `detect_format_maps_every_suffix` rows (`archive.rs:1044-1063`) | FR-NEW-001, FR-MOD-001, FR-NEW-010 | US-0178 |
| `crates/core/src/tools/archive_single.rs` | modify: variant, label, fallible decoder, `ReadError::Init`, tests | FR-NEW-001 to FR-NEW-013 | US-0178 to US-0182 |
| `crates/core/src/api/dataplane.rs` | modify: HTTP door tests (answer, password, destination is a file) | FR-NEW-001, FR-NEW-009, FR-NEW-011 | US-0180, US-0181 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md` | modify: formats list, bare `.xz` paragraph, stack line | closing story | US-0182 |

Affected existing tests: `detect_format_maps_every_suffix` gains rows; `assert_bomb_bounded` (`archive_single.rs:822-833`) calls `decoder(...)`, which now returns `Result`: it unwraps it (test code). No existing test is removed. Affected documentation: as above.

## 7. Compatibility and risks

- Tar family untouched: `.tar.xz`/`.txz` keep `lzma_rs::xz_decompress` (`archive.rs:742-746`), so a second stream, padding, a BCJ filter or a SHA-256 check inside a tar archive is still refused there; moving it is BL-0132. Signal: E2E-005, E2E-006, E2E-047.
- Dependency (DEC-002): `liblzma` 0.4.9 and `liblzma-sys` 0.4.9 are already compiled in every build today (non optional `datafusion` -> `datafusion-datasource` -> `async-compression` -> `compression-codecs` -> `liblzma` with default features, `cargo tree -i liblzma -e features`). Declaring it directly with default features adds no package, no feature and no build step. Licences: both crates `MIT OR Apache-2.0`; the vendored xz source is 0BSD, both in `deny.toml:31`'s allow list. Baseline on 2026-11-08: `cargo deny check` gives `advisories ok, bans ok, licenses ok, sources ok`; `cargo audit` passes with 2 allowed warnings (`pin-utils` RUSTSEC-2026-0441, `arrow-data` RUSTSEC-2026-0442), neither about liblzma. One new exposure: `deny.toml:10` sets `unmaintained = "workspace"`, so an unmaintained advisory against liblzma would now fail `make security` where it only warned before; none exists today.
- Link mode (DEC-002): `liblzma-sys`'s build script probes pkg-config for a system liblzma at least 5.8.0 and links it dynamically, else compiles the vendored xz source statically (`liblzma-sys-0.4.9/build.rs:13-40`). In this repository's `target/` it built static (`cargo:rustc-link-lib=static=lzma`), and `otool -L target/release/mcp-fs` shows no lzma dylib; a standalone scratch crate on the same machine linked the Homebrew 5.8.4 dylib. This is today's behavior through datafusion and does not change; forcing the `static` feature would also change datafusion's build and is not required by the spec, so it is not done (recorded, no backlog item: no requirement asks for it).
- Memory: liblzma allocates the dictionary the stream header declares (8 MiB for xz preset 6, up to 1.5 GiB for a crafted header) but only touches the pages it writes; RSS stays bounded by the bytes decoded, which the cap bounds (DEC-006 measurements). A declared dictionary malloc cannot satisfy fails with `LZMA_MEM_ERROR`, an `Io` error, so the corrupt refusal.
- Ordering: US-0178 to US-0182 form one chain, every story editing `archive_single.rs` (one writer at a time). SPEC-0081 and SPEC-0083 stories are landed (`_queue.md`), so the first story depends on `main`.
- Rollback: revert the commits; the dependency line can go with them.

## 9. Test strategy

Drivers and locations as SPEC-0081 and SPEC-0083; test names `e2e_0082_<nnn>_<slug>`, filter `e2e_0082_`. Two design tests that pin DEC-003 and are not spec tests: `xz_0082_lzma_alone_file_is_corrupt`, `xz_0082_unknown_check_type_is_corrupt` (US-0179).

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-002 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-003 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-004 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-005 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-006 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-007 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-008 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-009 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-010 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-011 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-012 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-013 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-014 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-015 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-016 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-017 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-018 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-019 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-020 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-021 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-022 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-023 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-024 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-025 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-026 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-027 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-028 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-029 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-030 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-031 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-032 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-033 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-034 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-035 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-036 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-037 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0180 |
| E2E-038 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |
| E2E-039 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-040 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-041 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-042 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-043 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-044 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-045 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-046 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0180 |
| E2E-047 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-048 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0180 |
| E2E-049 | functional e2e | HTTP door: `Harness::new()` then `h.post(&u("extract-archive"), json!({...}))` | crates/core/src/api/dataplane.rs tests | US-0181 |
| E2E-050 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-051 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-052 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0179 |
| E2E-053 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-054 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-055 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0181 |
| E2E-056 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-057 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0178 |
| E2E-058 | functional e2e | typed engine call `extract_archive` (the function both doors call), after `f.state.authorize(MOUNT, person)` when the test is about membership | crates/core/src/tools/archive_single.rs tests | US-0182 |

Test data: all generated in the test with liblzma's own encoder (DEC-007); the `xz` command line tool is not used by any test.

## 10. Decisions log

- **DEC-001:** xz reuses the single file flow and rules of SPEC-0081 and SPEC-0083 (`extract_single`, `read_capped`, `caps`, `check_destination`, `check_collision`, admission, order). **Obliged by:** SPEC-0082/FR-NEW-004, FR-NEW-005, FR-NEW-006, FR-NEW-011, FR-NEW-012. **Rationale:** one engine per operation (AGENTS.md conventions); the spec's rules are those of the gzip increment word for word. **Alternatives considered:** a parallel xz flow. **Source:** existing code and SPEC-0083 DEC-001. **Evidence:** `crates/core/src/tools/archive_single.rs:212-277`, `sdlc/specs/SPEC-0083-extract-bz2-single-file/sdlc-design.md`.
- **DEC-002:** `liblzma` 0.4.9 becomes a direct dependency with its default features: `liblzma = "0.4.9"` in `[workspace.dependencies]`, `liblzma = { workspace = true }` in `crates/core/Cargo.toml`; `lzma-rs` stays for the tar family. **Obliged by:** SPEC-0082/FR-NEW-005, FR-NEW-006, FR-NEW-007, FR-NEW-013, and the spec's erratum 1 (a spec constraint: the spec names the library). **Rationale:** the only decoder that streams mid block, decodes concatenated streams and validates padding without custom parsing; already compiled in every build, lockfile delta one line, licences allowed, deny and audit baselines unaffected. Default features keep the current link behavior (system liblzma at least 5.8.0 through pkg-config, else vendored static). **Alternatives considered:** `lzma-rs` plus a hand written splitter (design issue option c); the `static` feature (changes datafusion's build too, no requirement asks for it); `xz2` (older fork, superseded by liblzma). **Source:** spec constraint (erratum 1, design-issues.md resolution 2026-11-05). **Evidence:** context7 `/websites/rs_liblzma_liblzma`; crates.io max stable `liblzma` 0.4.9, `liblzma-sys` 0.4.9; `Cargo.lock:3575-3592`; `cargo tree -p mcp-fs-core -i liblzma -e features`; `liblzma-sys-0.4.9/build.rs:13-40`; `cargo deny check` and `cargo audit` runs of 2026-11-08.
- **DEC-003:** the xz decoder is `liblzma::read::XzDecoder::new_stream(bytes, Stream::new_stream_decoder(u64::MAX, liblzma::stream::CONCATENATED | liblzma::stream::TELL_UNSUPPORTED_CHECK)?)`, read errors mapped to `corrupt("xz", e)`. **Obliged by:** SPEC-0082/FR-NEW-007, FR-NEW-008, FR-NEW-013, and FR-NEW-001 (E2E-015, E2E-056). **Rationale:** spike on liblzma 0.4.9 (2026-11-08, vendored or system liblzma 5.8.4), every input built with liblzma's encoder or `xz` 5.8.4: two streams give `hello world`; streams `a`, empty, `b` give `ab`; 4 or 8 zero bytes after a stream and 4 between two streams are accepted; 3 or 5 zero bytes after, and 3 or 6 between, fail with `lzma data error`; `JUNK` after a stream, a 0 byte file, 4 zero bytes alone, the text `not xz` and a file cut in half fail with `premature eof`; every one of the 72 single byte flips of the 16 byte CSV fixture fails; an x86 BCJ filter and a SHA-256 check decode to `hello`; a `.lzma` file fails with `stream/file format not recognized`; a reserved check type 0x02 fails with `liblzma not built with check support`. `new_stream_decoder` reads xz only (FR-NEW-008 "is not xz"); `TELL_UNSUPPORTED_CHECK` makes an unknown check type an error instead of decoding unverified (measured: without the flag the same file decodes to `hello`). Memory limit `u64::MAX`: see DEC-006. **Alternatives considered:** `new_multi_decoder` (auto decoder, accepts `.lzma`, decodes an unknown check unverified); `XzDecoder::new` (one stream; measured: ignores `JUNK`, 3 zero bytes and a second stream). **Source:** spike. **Evidence:** `liblzma-0.4.9/src/read.rs:149-163`, `src/bufread.rs:185-200`, `src/stream.rs:270-275`, `:379`, `:1320`.
- **DEC-004:** `decoder` returns `std::io::Result<Box<dyn Read + '_>>`; inside the blocking task `decoder(format, &bytes).map_err(ReadError::Init).and_then(|r| read_capped(r, cap))`; `ReadError::Init(e)` maps to `ToolError::internal(format!("cannot start the {label} decoder: {e}"))`. **Obliged by:** SPEC-0082/FR-NEW-008 (a corrupt refusal names a fault of the file, and a decoder that cannot start is a fault of the server). **Rationale:** `Stream::new_stream_decoder` returns `Result` (only memory or option errors); production code may not `unwrap` it (rust skill). **Alternatives considered:** `unwrap`/`expect` as `new_multi_decoder` does internally; mapping to the corrupt refusal. **Source:** design. **Evidence:** `liblzma-0.4.9/src/stream.rs:379`, `archive_single.rs:64-69`, `:239-251`.
- **DEC-005:** table row `(".xz", ArchiveFormat::Single(SingleFormat::Xz))` after `(".bz2", ...)`; `label()` is `"xz"`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-010, FR-MOD-001. **Rationale:** `detect_format` takes the first `ends_with` match in table order (`archive.rs:68-79`), so `.tar.xz` and `.txz` (rows `:49-50`) win over the plain `.xz`. **Alternatives considered:** none. **Source:** existing code. **Evidence:** `archive.rs:44-62`.
- **DEC-006:** no decoder memory limit; the bomb tests measure memory with the existing hooks: `LAST_HIGH_WATER` after the whole extraction (`assert_extraction_bounded`, `archive_single.rs:837-842`) and a direct `read_capped(decoder(Xz, &bomb)?, cap)` (`assert_bomb_bounded` pattern, `:822-833`), both asserting at most `cap + 65 536` bytes of output held and under 1 000 000. Process RSS is not asserted in a test: reading it needs `unsafe` (forbidden workspace wide) or spawning `ps`, which races with the parallel test threads. **Obliged by:** SPEC-0082/FR-NEW-005, FR-NEW-006 (E2E-009, E2E-010), NFR 7.2. **Rationale:** spike, release build, RSS read with `ps` before and during a capped read (cap 1 000 bytes) of 10 000 000 zero bytes: xz preset 6 file (8 MiB dictionary, 1 592 bytes) RSS delta 144 KB; liblzma encoder with 64 KiB, 8 MiB and 64 MiB dictionaries (1 584 bytes each) RSS delta 16 KB, 16 KB, 48 KB; the first `read` returns 65 536 bytes and the cap stops at `high_water` 0. liblzma touches dictionary pages only as it writes them, so memory follows the bytes decoded, which the cap bounds. A memory limit would refuse legitimate files (xz `-9` needs 65 MiB) for no spec requirement. **Alternatives considered:** `memlimit` of 64 or 256 MiB; an RSS assertion through `ps`. **Source:** spike. **Evidence:** spike output quoted in this entry (throwaway crate, not committed), `archive_single.rs:51-57`, `:822-842`.
- **DEC-007:** fixtures are generated in the test with liblzma's encoder: plain files `liblzma::write::XzEncoder::new(Vec::new(), 6)`; E2E-015 a `Stream::new_stream_encoder(&filters, Check::Crc64)` with `Filters::new()` then `.x86()` then `.lzma2(&LzmaOptions::new_preset(6)?)`; E2E-056 the same with `Check::Sha256` and only `.lzma2`; several streams are concatenated byte vectors; padding is appended zero bytes; the damaged checksum flips the byte at offset 24 of the CSV fixture (all 72 flips measured as refused); the bomb is preset 6 over 10 000 000 zero bytes (1 584 bytes, one block, 8 MiB dictionary); tar fixtures keep `lzma_rs::xz_compress` (`archive.rs:1991`). The design test for an unknown check encodes `hello` with `Check::Crc32`, sets the stream flags check byte to 0x02 at offset 7 and at `len - 3`, and recomputes the header CRC32 (bytes 6 to 7, stored at 8 to 11) and the footer CRC32 (bytes `len - 8` to `len - 3`, stored at `len - 12` to `len - 9`) with `flate2::Crc`. **Obliged by:** SPEC-0082/FR-NEW-001, FR-NEW-007, FR-NEW-008, FR-NEW-013 (fixtures "provided by the design", spec section 9). **Rationale:** no test depends on an installed `xz` binary (present here, 5.8.4, but not guaranteed on another machine); `flate2` is already a dependency. **Alternatives considered:** committed fixture files made with `xz`; `crc32fast` (not a direct dependency). **Source:** spike. **Evidence:** spike output; `Cargo.toml:82`.
- **DEC-008:** US-0178 wires `.xz` with `Stream::new_stream_decoder(u64::MAX, TELL_UNSUPPORTED_CHECK)` (one stream); US-0179 adds `CONCATENATED`, which turns the several streams, junk and odd padding tests from red to green; US-0180 to US-0182 add tests that characterize rules already built by SPEC-0081 (password, name, tar rows, collision, destination, order, caps) with mutation evidence instead of red evidence. **Obliged by:** n/a (no code impact): story planning. **Rationale:** measured: without `CONCATENATED` two streams give `hello ` and `JUNK` or 3 zero bytes after a stream are ignored, so US-0179's red evidence is real; the shared rules are green on arrival for xz, so proving they bite needs a mutation. **Alternatives considered:** one story of 58 tests; the full flags in US-0178 (US-0179 would then be all green). **Source:** design. **Evidence:** spike.
- **DEC-009:** the tar family keeps `lzma-rs`; its decode path (`archive.rs:742-746`) is not touched. **Obliged by:** SPEC-0082/FR-NEW-010, spec section 3.2. **Rationale:** the spec pins the tar behavior as unchanged; the move is BL-0132. **Alternatives considered:** moving tar xz to liblzma now. **Source:** spec constraint. **Evidence:** `sdlc/backlog/BL-0132-move-the-tar-xz-decoder-to-liblzma.md`.
- **DEC-010:** documentation as SPEC-0083 DEC-005: README formats list and single file paragraph gain `.xz` and say it is decoded by liblzma (C), not in pure Rust; AGENTS.md's `archive_single.rs` entry and stack line name xz and liblzma; `.agent_docs/tools.md` formats list and a dated SPEC-0082 paragraph (several streams, padding multiple of 4, BCJ and SHA-256 decode, `.lzma` refused). **Obliged by:** n/a (no code impact): project convention (AGENTS.md "keep README.md and AGENTS.md up to date"). **Rationale:** the README claims "all decoded in pure Rust" for the archive formats it lists, which a bare `.xz` would make wrong if listed there. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** `README.md:294-303`, `AGENTS.md:137-140`, `.agent_docs/tools.md:180-183`, `:220-240`.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0178 | extract a bare xz file next to the archive | P1 | SPEC-0082/FR-NEW-001, SPEC-0082/FR-NEW-002, SPEC-0082/FR-MOD-001 | main | 2 | no |
| US-0179 | decode several xz streams and padding, refuse corrupt input | P1 | SPEC-0082/FR-NEW-007, SPEC-0082/FR-NEW-008, SPEC-0082/FR-NEW-013 | US-0178 | 2 | no |
| US-0180 | xz password, output name, tar rows and the HTTP answer | P1 | SPEC-0082/FR-NEW-001, SPEC-0082/FR-NEW-003, SPEC-0082/FR-NEW-009, SPEC-0082/FR-NEW-010, SPEC-0082/FR-MOD-001 | US-0179 | 2 | no |
| US-0181 | check the xz collision, destination and order rules | P1 | SPEC-0082/FR-NEW-004, SPEC-0082/FR-NEW-011, SPEC-0082/FR-NEW-012 | US-0180 | 2 | no |
| US-0182 | check the xz storage and session limits and document the format | P1 | SPEC-0082/FR-NEW-005, SPEC-0082/FR-NEW-006 | US-0181 | 2 | yes |
