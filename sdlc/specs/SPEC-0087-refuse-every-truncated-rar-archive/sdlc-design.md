# SPEC-0087: Refuse every truncated RAR archive, design

> Spec: sdlc/specs/SPEC-0087-refuse-every-truncated-rar-archive/sdlc-spec.md
> Nature: FEAT
> Depth: M
> Security: n/a
> Designed on: 2026-11-09
> Target tier: 2 (default: no `--tier`, `sdlc/constitution.md` has no `Tier:` field, no `.spec.json`); two stories need tier 1 (header decryption)

## 1. Stack and skills

| Element | Choice | Version | DEC |
|---|---|---|---|
| RAR reader | `unrar` (safe API only), unchanged | `unrar = "0.5.8"` (`Cargo.toml:95`), feature `rar` | DEC-001 |
| Completeness walk | in tree, `tools/archive_rar.rs` (`ensure_rar_complete` replaces `ensure_rar5_complete`, `:98-160`) | n/a | DEC-001, DEC-002 |
| AES block cipher | `aes` | `aes = "0.8"`, locked 0.8.4 (`Cargo.lock:31-32`), already compiled through `aes-gcm`, `sevenz-rust2`, `lopdf` | DEC-004 |
| CBC mode | `cbc` | `cbc = "0.1"`, locked 0.1.2 (`Cargo.lock:1276-1277`), already compiled through `sevenz-rust2`, `lopdf` | DEC-004 |
| HMAC (RAR5 key derivation) | `hmac` | `hmac = "0.13"`, locked 0.13.0 (`Cargo.lock:3073-3074`), already compiled through `zip`, `aws-sdk-s3`, `sqlx-postgres` | DEC-004 |
| SHA-256 for HMAC | `sha2` 0.11, renamed `sha2_v011` (the workspace `sha2 = "0.10"` cannot drive `hmac` 0.13, which needs `digest` 0.11) | `sha2 = "0.11"`, locked 0.11.0 (`Cargo.lock:5281-5282`), already compiled through `aws-sdk-s3`, `datafusion` | DEC-004 |
| SHA-1 (RAR4 key derivation) | `sha1` 0.10, existing direct dependency, feature `compress` enabled (exposes `sha1::compress`) | `sha1 = "0.10"` (`Cargo.toml:52`), locked 0.10.7 | DEC-005 |
| Header CRC | `flate2::Crc` (existing direct dependency) | `flate2 = "1"` | DEC-002 |

Dependency rule of this run: every crate above is already in `Cargo.lock` (the four new direct entries `aes`, `cbc`, `hmac`, `sha2` 0.11 are compiled today as transitive dependencies of the default build: `cargo tree --all-features -i cbc@0.1.2`, `-i hmac@0.13.0`, `-i aes@0.8.4`, `-i sha2@0.11.0`); no package is added to the lock file, only `mcp-fs-core`'s dependency list grows, and licences are unchanged (all `MIT OR Apache-2.0`, in `deny.toml`'s allow list). They are optional and enabled by the `rar` feature only. No ask was needed (ASK ONLY FOR category 2 is not triggered). Versions are read from `Cargo.lock`.

Skills loaded: `rust` (`unsafe_code` forbidden, no `unwrap` on fallible production paths, `spawn_blocking` for CPU work, clippy `--all-features -D warnings`, fmt check).

## 2. Approaches

### Chosen: our own header walk, decrypting encrypted headers with the right password
Extend today's RAR5 structural walk (`ensure_rar5_complete`, `archive_rar.rs:110-135`) into `ensure_rar_complete(bytes, password)`: find the marker exactly where UnRAR finds it, then walk every block of a RAR4 or RAR5 archive up to its end of archive block, refusing any shortfall with `archive is corrupt or not a valid rar file: unexpected end of archive`. Headers encrypted with the right password are decrypted by our code (RAR4: AES 128 CBC, RAR 3 key derivation from SHA-1 with an 8 byte salt per header; RAR5: AES 256 CBC, PBKDF2 HMAC SHA-256 with the encryption header's salt and iteration count, a 16 byte IV per header, and the password check value). Without a password or with an empty one the walk stops after the whole leading part and UnRAR answers the password question as today. Pros: complete answers through the safe API; header only work (no second decode of the data, spec 7.1); one walk for both versions. Cons: about 300 lines including two key derivations; KDF cost per call with a password (UnRAR pays it too). Affected scope: `tools/archive_rar.rs`, new `tools/archive_rar_crypt.rs`, manifests.

### Rejected: raw `unrar_sys` calls to read the end of archive flag
The flag is reachable only through the C API (`dll.cpp:222-231`, `ERAR_END_ARCHIVE` whether or not the block was seen), which needs `unsafe`, forbidden workspace wide (SPEC-0080 DEC-006).

### Rejected: hold the requirements needing decryption
The spec flags them (14.3 signal: E2E-022, E2E-024, E2E-028, E2E-030, E2E-034, E2E-037). The investigation shows every primitive is already in `Cargo.lock` (section 1), so the design completes the spec instead of holding it.

## 3. Architecture

```
list_rar (archive_rar.rs, SPEC-0085 DEC-007), first step:
  ensure_rar_complete(bytes, password)
    marker = find_marker(bytes)            mirrors UnRAR Archive::IsArchive (archive.cpp:125-175)
      none, RAR 1.4, or a partial marker   -> Ok (UnRAR refuses it: today's corrupt prefix, FR-NEW-003)
    RAR4 (7 byte marker):  walk_v4(rest, password)
    RAR5 (8 byte marker):  walk_v5(rest, password)
      clear blocks: header, data size, next; end block -> Ok; any shortfall -> TRUNCATED
      encrypted headers (RAR4 main header flag 0x0080, RAR5 encryption header type 4):
        leading part must be whole, else TRUNCATED                                   FR-NEW-004
        password None or Some("")  -> Ok (UnRAR: password needed / incorrect)         FR-NEW-004, FR-NEW-009
        RAR5 check value present and mismatching -> incorrect password message        FR-NEW-005
        decrypt each header (archive_rar_crypt.rs); a header CRC mismatch -> Ok (UnRAR answers, today's message)
        shortfall -> TRUNCATED; end block -> Ok                                       FR-MOD-001
  then the UnRAR listing with entry rules, missing password gate, declared check (SPEC-0085), decode
```

`archive_rar_crypt.rs` (feature `rar`): `Rar3Key { key: [u8; 16], iv: [u8; 16] }` from `rar3_kdf(password, salt)`; `Rar5Keys { key: [u8; 32], psw_check: [u8; 8] }` from `rar5_kdf(password, salt, lg2_count)`; `decrypt_aes128_cbc(key, iv, &mut [u8])`, `decrypt_aes256_cbc(key, iv, &mut [u8])` over whole 16 byte blocks, no padding.

## 4. Data model

None persisted.

## 5. Contracts and interfaces

- `archive_rar.rs`: `fn ensure_rar_complete(bytes: &[u8], password: Option<&str>) -> Result<()>` (replaces `fn ensure_rar5_complete(bytes: &[u8]) -> Result<()>`, called at the start of `list_rar`); `const TRUNCATED: &str = "unexpected end of archive"` (existing, `:103`).
- RAR4 block layout read by the walk: `HEAD_CRC` u16, `HEAD_TYPE` u8, `HEAD_FLAGS` u16, `HEAD_SIZE` u16 (at least 7); when `HEAD_FLAGS & 0x8000` a u32 `ADD_SIZE` at offset 7 (the packed size for file `0x74` and service `0x7A` blocks), plus a u32 high part at offset 32 when `HEAD_FLAGS & 0x0100` on those two types; data size = `ADD_SIZE` (+ high part << 32); main header type `0x73`, its flag `0x0080` declares encrypted headers; end of archive type `0x7B`. Encrypted RAR4 header: 8 byte salt, then `ceil16(HEAD_SIZE)` bytes of AES 128 CBC ciphertext; the data that follows is not part of it.
- RAR5 layout: block = CRC32 u32, `HeadSize` vint, then `HeadSize` bytes (type vint, flags vint, extra size vint if `0x1`, data size vint if `0x2`, ...), then the data; encryption header type 4: `version` vint (0), `flags` vint (`0x1` = check value present), `lg2_count` u8 (at most 24, else corrupt), `salt` 16 bytes, then when flagged `psw_check` 8 bytes and its checksum 4 bytes (first 4 bytes of SHA-256 of `psw_check`); every later header is a 16 byte IV, then `ceil16(4 + vint length + HeadSize)` bytes of AES 256 CBC ciphertext; end of archive type 5.
- `archive_rar_crypt.rs`:
  - `pub(crate) fn rar3_kdf(password: &str, salt: &[u8; 8]) -> ([u8; 16], [u8; 16])`: raw = the password as UTF-16 little endian then the salt; SHA-1 state over `0x40000` rounds of `raw` (processed with UnRAR's `sha1_process_rar29` write back, `sha1.cpp:146-168`, through `sha1::compress`) then the 3 byte round number; at each `round % 0x4000 == 0` the IV byte `round / 0x4000` is the last byte of the digest of a copy of the state; key byte `i * 4 + j` = byte `j` (little endian) of digest word `i`, `i` and `j` in 0..4 (`crypt3.cpp`, `SetKey30`).
  - `pub(crate) fn rar5_kdf(password: &str, salt: &[u8; 16], lg2_count: u8) -> ([u8; 32], [u8; 8])`: PBKDF2 HMAC SHA-256 over the UTF-8 password, one block: `U1 = HMAC(pw, salt || 0x00000001)`, `F = U1`, then `count - 1` iterations give the key, 16 more the hash key value (not kept), 16 more the check value; `psw_check[i % 8] ^= value[i]` for `i` in 0..32 (`crypt5.cpp`, `pbkdf2`, `SetKey50`).
  - `pub(crate) fn aes128_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], buf: &mut [u8])`, `pub(crate) fn aes256_cbc_decrypt(key: &[u8; 32], iv: &[u8; 16], buf: &mut [u8])` (`buf.len()` a multiple of 16).
- Messages: every cut after a whole marker `ERR_INVALID_ARGUMENT` `archive is corrupt or not a valid rar file: unexpected end of archive`; a RAR5 check value mismatch `password_required_message(password)` (`incorrect password for this archive; call again with the correct 'password' parameter`, `archive.rs:675-686`); every other message unchanged.
- Manifests: `Cargo.toml` `[workspace.dependencies]`: `aes = "0.8"`, `cbc = "0.1"`, `hmac = "0.13"`, `sha2_v011 = { package = "sha2", version = "0.11" }`, and `sha1 = { version = "0.10", features = ["compress"] }`; `crates/core/Cargo.toml`: `aes`, `cbc`, `hmac`, `sha2_v011` with `optional = true`, and `rar = ["dep:unrar", "dep:aes", "dep:cbc", "dep:hmac", "dep:sha2_v011"]`.
- `TOOL_CONTRACT.txt`, `tool-contract-golden.json`: unchanged.

## 6. File structure impact

| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/tools/archive_rar.rs` | modify: `ensure_rar_complete` (marker search, RAR4 walk, RAR5 walk incl. encrypted headers); internal tests renamed or re pointed (DEC-009) | FR-NEW-001 to FR-NEW-005, FR-MOD-001 | US-0198, US-0201, US-0202 |
| `crates/core/src/tools/archive_rar_crypt.rs` | create: key derivations, AES CBC | FR-MOD-001 (right password), FR-NEW-005 | US-0201, US-0202 |
| `crates/core/src/tools/mod.rs` | modify: `#[cfg(test)] mod archive_rar_cut_e2e;` (US-0198), `#[cfg(feature = "rar")] pub(crate) mod archive_rar_crypt;` (US-0201) | test layout, FR-MOD-001 | US-0198, US-0201 |
| `crates/core/src/tools/archive_rar_cut_e2e.rs` | create: the `e2e_0087_*` tests | every requirement | US-0198 to US-0203 |
| `crates/core/tests/fixtures/rar/two4.rar`, `dir4.rar`, `solid4.rar`, `big4.rar`, `extras4.rar`, `hp5two.rar`, `hp4.rar`, `hp4long.rar`, `README.md` | create (archives), modify (README rows) | fixtures (DEC-006) | US-0198 to US-0202 |
| `Cargo.toml`, `crates/core/Cargo.toml`, `Cargo.lock` | modify (DEC-004, DEC-005) | FR-MOD-001 | US-0201, US-0202 |
| `crates/core/src/api/dataplane.rs` | modify: E2E-044, E2E-052, E2E-053 in a nested `mod archive_rar` | FR-NEW-008 | US-0204 |
| `README.md`, `AGENTS.md`, `.agent_docs/tools.md`, `.agent_docs/testing.md` | modify | closing story (DEC-010) | US-0204 |

Affected existing tests (DEC-009): `non_rar5_input_is_left_to_the_library` (`archive_rar.rs:754-759`) is replaced by `partial_markers_are_left_to_the_library` (empty input and `Rar!\x1a\x07\x01` still pass) and `rar4_marker_alone_is_truncated` (`Rar!\x1a\x07\x00` refused with `unexpected end of archive`); `every_strict_prefix_of_a_rar5_archive_is_corrupt` (`:740-752`), `huge_data_size_is_corrupt_not_a_panic` and the test at `:770` change only their call to `ensure_rar_complete(.., None)`, assertions unchanged. SPEC-0080 E2E-014, E2E-048, E2E-058, E2E-060, E2E-062 to E2E-065 keep their assertions (spec 9.4). Any other existing test that changes is a story failure: stop and report it.

Affected documentation: `.agent_docs/tools.md:208-210` (the RAR4 limitation sentence goes), `README.md` (truncated RAR refused for both versions), `AGENTS.md` (`archive_rar.rs` and new `archive_rar_crypt.rs` in the structure), `.agent_docs/testing.md:435` (fixtures, run time of the every length tests, filter `e2e_0087_`), `crates/core/tests/fixtures/rar/README.md`.

## 7. Compatibility and risks

- Declared changes: a cut RAR4 archive, or a cut archive with encrypted file names opened with the right password, is refused as corrupt; the marker alone or with its main header is refused; misleading messages on cuts become the corrupt one.
- Kept: password questions before the truncation for encrypted file names (missing, wrong, empty), BL-0136 (RAR4 wrong password reads as corrupt) unchanged.
- Run time: with a password, each call derives keys (RAR5: about 98 000 HMAC SHA-256 at `lg2_count` 15; RAR4: 262 144 SHA-1 updates); the debug test profile has no dependency optimization (`rg -n 'profile.dev.package' Cargo.toml` finds nothing), so E2E-034's password loop is reduced (DEC-007) and BL-0137 records the profile idea.
- Fixture availability: the archives measured by the spec are in `/tmp/bl131` today; RAR 6.24 and 7.23 trial binaries are in `/tmp/rarbin`. `/tmp` is not durable: a story that finds them gone remakes the archives with the commands of DEC-006 from the RARLAB trial builds and re measures every offset it uses.
- Rollback: revert the story commits; the manifests return with US-0201 and US-0202 reverted.

## 9. Test strategy

Functional e2e through the typed engine call `extract_archive` after `f.state.authorize(MOUNT, person)` in `crates/core/src/tools/archive_rar_cut_e2e.rs` (`#[cfg(test)]`; every test `#[cfg(feature = "rar")]` except E2E-043, `#[cfg(not(feature = "rar"))]`; the module path contains `archive_rar`, so `test.sh:7` runs it, SPEC-0085 DEC-011); E2E-044, E2E-052, E2E-053 through the HTTP door in `api/dataplane.rs`, nested `#[cfg(feature = "rar")] mod archive_rar`. Cuts are slices of the committed archives at run time (`&TWO4[..67]`), never committed files. The common data of spec 9.0 applies: project `atlas` (testkit `MOUNT`), password `s3cret-Ünï`. Names `e2e_0087_<nnn>_<slug>`, filter `e2e_0087_`.

| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| E2E-001 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-002 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-003 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0202 |
| E2E-004 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0201 |
| E2E-005 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-006 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-007 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-008 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-009 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-010 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-011 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-012 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-013 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-014 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-015 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-016 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-017 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-018 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-019 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-020 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-021 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-022 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0202 |
| E2E-023 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0202 |
| E2E-024 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0202 |
| E2E-025 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-026 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-027 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-028 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0201 |
| E2E-029 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0201 |
| E2E-030 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0201 |
| E2E-031 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-032 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-033 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-034 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-035 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-036 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-037 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0201 |
| E2E-038 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-039 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-040 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-041 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-042 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-043 | functional e2e | typed engine call (build without `rar`) | archive_rar_cut_e2e.rs | US-0199 |
| E2E-044 | functional e2e | HTTP door | api/dataplane.rs tests (nested `archive_rar`) | US-0204 |
| E2E-045 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-046 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0200 |
| E2E-047 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0198 |
| E2E-048 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0202 |
| E2E-049 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-050 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-051 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |
| E2E-052 | functional e2e | HTTP door | api/dataplane.rs tests (nested `archive_rar`) | US-0204 |
| E2E-053 | functional e2e | HTTP door | api/dataplane.rs tests (nested `archive_rar`) | US-0204 |
| E2E-054 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0199 |
| E2E-055 | functional e2e | typed engine call | archive_rar_cut_e2e.rs | US-0203 |

Test data: committed real archives (DEC-006), cut at run time; E2E-049's `bomb.rar` from the in tree builder (`rar5_store(&[RarEntry::declaring("big.bin", 10, &vec![7u8; 1_000_000])])`, the archive of SPEC-0080 E2E-012).

## 10. Decisions log

- **DEC-001:** `ensure_rar_complete(bytes, password)` replaces `ensure_rar5_complete` at the start of the RAR listing: it finds the marker as UnRAR does (the first 7 bytes, else the first `0x52` byte from offset 7 within the next `0x200000 - 16` bytes whose bytes form a signature; RAR 1.4 signature or no signature or a RAR5 signature without its 8th byte: return `Ok` and let UnRAR refuse), then walks the version's blocks; any block whose header or declared data runs past the end, or an archive ending after a whole block that is not the end of archive block, is refused with `corrupt("rar", TRUNCATED)`; bytes after the end block are ignored. **Obliged by:** SPEC-0087/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-MOD-001. **Rationale:** the safe API cannot tell an end block from an end of file (`unrar-0.5.8/src/open_archive.rs:452`, `dll.cpp:222-231`) and carries no packed size (`open_archive.rs:582-590`); mirroring UnRAR's marker search keeps the walk and the reader on the same archive (E2E-047). **Alternatives considered:** section 2. **Source:** spec 14.3 and existing code. **Evidence:** `crates/core/src/tools/archive_rar.rs:98-160`, `unrar_sys-0.5.8/vendor/unrar/archive.cpp:98-175`.
- **DEC-002:** the RAR4 walk reads the 7 byte base header of each block (CRC16, type, flags, size, size at least 7), the data size from `ADD_SIZE` when flag `0x8000` is set (plus the high 32 bits at header offset 32 for types `0x74` and `0x7A` when flag `0x0100` is set), stops at type `0x7B`, and treats every other type generically (comment, recovery record, service, sub blocks); a RAR4 header whose size is below 7, or a RAR5 vint that overflows, is refused as `TRUNCATED` or with the existing vint messages. Header CRCs of clear headers are not checked (UnRAR checks them); CRCs of decrypted headers are (DEC-003), with `flate2::Crc` (CRC16 = low 16 bits of the CRC32 of the header after its CRC field; RAR5 CRC32 of the header after its CRC field). **Obliged by:** SPEC-0087/FR-NEW-001 (every kind of block), FR-NEW-002. **Rationale:** the generic `LONG_BLOCK` rule covers the recovery record and comment of `extras4.rar` (P-1: a cut inside the recovery record is accepted today) without a table of block types. **Alternatives considered:** a per type table (misses unknown service blocks). **Source:** spec 14.3 "RAR4 block walk". **Evidence:** `unrar_sys-0.5.8/vendor/unrar/arcread.cpp`, spec probes P-1 and P-5.
- **DEC-003:** encrypted headers. The leading part (RAR4: marker and main header; RAR5: marker and encryption header) must be whole, else `TRUNCATED`. With `password` `None` or `Some("")` the walk returns `Ok` after the leading part and UnRAR answers (password needed, or incorrect for the empty password, `archive_rar.rs:162-169`, `:69-81`). With a non empty password: RAR5 derives the keys once per call and, when the check value flag is set and its checksum is valid, a mismatch returns `password_required_message(password)`; each header is decrypted (RAR5: 16 byte IV then the ciphertext; RAR4: 8 byte salt then the ciphertext, keys cached per salt within the call), its CRC checked, and a CRC mismatch returns `Ok` (UnRAR then gives today's answer: corrupt for RAR4, BL-0136; incorrect for RAR5); a shortfall anywhere returns `TRUNCATED`. **Obliged by:** SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007 item 4, FR-NEW-009, FR-MOD-001. **Rationale:** the password questions keep their place before the truncation; answering the wrong password ourselves only where RAR5 offers a check value matches what UnRAR says today (BadPassword from 62 bytes, P-3); deferring on a CRC mismatch keeps every wrong password answer byte for byte. **Alternatives considered:** deferring every password question to UnRAR after the walk (the walk cannot run without the key, so a cut archive with a wrong password would read as TRUNCATED, against FR-NEW-005). **Source:** design. **Evidence:** spec probes P-2 and P-3; `unrar_sys-0.5.8/vendor/unrar/crypt5.cpp`, `crypt3.cpp`.
- **DEC-004:** RAR5 crypto: `hmac` 0.13 with `sha2` 0.11 (renamed `sha2_v011`) for PBKDF2 HMAC SHA-256, one loop producing the key (count), the hash key value (count + 16, unused) and the check value (count + 32), as UnRAR's `pbkdf2` does; `lg2_count` above 24 (UnRAR's `CRYPT5_KDF_LG2_COUNT_MAX`) is corrupt; `aes` 0.8 and `cbc` 0.1 (`cbc::Decryptor<aes::Aes256>`, `decrypt_padded_mut::<NoPadding>`) for AES 256 CBC. All four are optional dependencies enabled by the `rar` feature, already in `Cargo.lock` (section 1). **Obliged by:** SPEC-0087/FR-MOD-001 (encrypted file names with the right password), FR-NEW-005. **Rationale:** the crates are the RustCrypto ones the tree already compiles (zip, 7z, S3 signing); a hand written HMAC or AES is avoided. `pbkdf2` 0.13 is not used because it would run the derivation three times for the three values. **Alternatives considered:** `pbkdf2` 0.13; `sha2` 0.10 with a hand written HMAC. **Source:** design. **Evidence:** `Cargo.lock:31-32`, `:1276-1277`, `:3073-3074`, `:5281-5282`; `cargo tree --all-features -i hmac@0.13.0` (zip, aws-sdk-s3, sqlx-postgres).
- **DEC-005:** RAR4 crypto: the RAR 3 key derivation of `crypt3.cpp` (`SetKey30`) with `sha1` 0.10's `compress` (feature `compress` enabled on the existing direct dependency; no new package): a small SHA-1 context (state, byte count, 64 byte buffer) reproduces `sha1_process_rar29`, which writes the 16 expanded schedule words back into the password buffer after each block processed straight from it (`sha1.cpp:146-168`; only passwords whose UTF-16 form plus the salt spans a whole block reach that path, so `s3cret-Ünï` does not, and `hp4long.rar` pins it); `aes` 0.8 and `cbc` 0.1 (`cbc::Decryptor<aes::Aes128>`) for AES 128 CBC. **Obliged by:** SPEC-0087/FR-MOD-001 (version 4 with encrypted file names and the right password, any password). **Rationale:** without the write back, a long password derives a different key than RAR and every cut would go undetected for it. **Alternatives considered:** the plain `Sha1` digest (wrong keys for long passwords); handing long passwords to UnRAR (spec FR-MOD-001 has no length exception). **Source:** UnRAR source. **Evidence:** `unrar_sys-0.5.8/vendor/unrar/sha1.cpp:146-168`, `crypt3.cpp` (`SetKey30`), `sha1-0.10.7/src/lib.rs:80-81`.
- **DEC-006:** fixtures: `two4.rar`, `big4.rar`, `dir4.rar`, `solid4.rar`, `extras4.rar`, `hp4.rar` (RAR 6.24) and `hp5two.rar` (RAR 7.23) are the archives the spec measured, copied from `/tmp/bl131/` (sizes 123, 3 116, 152, 133, 745, 220, 382 bytes, checked on 2026-11-09 with `wc -c`) into `crates/core/tests/fixtures/rar/` with a README row each (commands of spec 14.3); `hp4long.rar` is made once with `rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt` (RAR 6.24) for DEC-005's write back path. When `/tmp/bl131` is gone, remake each archive with its README command and re measure every offset the story's tests use (spec 14.3: RAR stamps times, sizes stay). **Obliged by:** SPEC-0087/FR-NEW-001 (spec 3.3, 9.0 archive table). **Rationale:** the offsets of spec 9.0 are those of these exact files. **Alternatives considered:** committing every cut (one archive covers every cut at run time). **Source:** spec 14.3, existing fixtures README. **Evidence:** `ls /tmp/bl131`, `crates/core/tests/fixtures/rar/README.md`.
- **DEC-007:** E2E-034's right password half runs, for each archive, every length up to the end of the leading part plus 1, every offset spec 9.0 names for that archive and its two neighbours, and the last 8 lengths; its no password half runs every length. Recorded in the test's doc comment. **Obliged by:** SPEC-0087/FR-MOD-001, FR-NEW-003, FR-NEW-004 (E2E-034). **Rationale:** spec 14.3 allows it when the full loop is too slow; every call with a password pays two key derivations (ours and UnRAR's) in an unoptimized debug build. **Alternatives considered:** the full loop (several minutes per run); optimizing dependencies in the dev profile (BL-0137). **Source:** spec 14.3 "Run time". **Evidence:** spec 14.3 probe timings (2.9 s, 7.1 s, 10.4 s in release).
- **DEC-008:** tests live in `crates/core/src/tools/archive_rar_cut_e2e.rs` (`#[cfg(test)] mod archive_rar_cut_e2e;`), each `#[cfg(feature = "rar")]` except E2E-043 (`#[cfg(not(feature = "rar"))]`, run by the default `cargo test --workspace`); the module path contains `archive_rar` for `test.sh:7`; the HTTP tests sit in a nested `#[cfg(feature = "rar")] mod archive_rar` of `api/dataplane.rs` tests. Helpers from `archive_testkit.rs` plus a module local `assert_never_recorded` twin (`archive_rar.rs:922-938`). **Obliged by:** n/a (no code impact): test layout. **Rationale:** SPEC-0085 DEC-011. **Source:** existing code. **Evidence:** `test.sh:7`, `crates/core/src/tools/archive.rs:905-933`.
- **DEC-009:** internal tests: `non_rar5_input_is_left_to_the_library` becomes `partial_markers_are_left_to_the_library` (`b""` and `b"Rar!\x1a\x07\x01"` still `Ok`) plus `rar4_marker_alone_is_truncated` (`b"Rar!\x1a\x07\x00"` refused, message `archive is corrupt or not a valid rar file: unexpected end of archive`); `every_strict_prefix_of_a_rar5_archive_is_corrupt`, `huge_data_size_is_corrupt_not_a_panic` and the end block test at `:770` only switch to `ensure_rar_complete(.., None)`. Design tests (not spec tests): `rar5_check_value_of_hidden_rar_matches` and `rar3_key_decrypts_the_first_header_of_hp4` (header CRC matches after decryption), `rar3_key_with_a_long_password_decrypts_hp4long`. **Obliged by:** SPEC-0087/FR-MOD-001, FR-NEW-001 (spec 14.3 "internal tests that pin the old limitation"). **Rationale:** the old test pins the limitation this spec removes; the design tests pin each key derivation on a real archive before the acceptance tests depend on it. **Alternatives considered:** published test vectors (none exist for the RAR 3 derivation). **Source:** spec 14.3. **Evidence:** `crates/core/src/tools/archive_rar.rs:740-800`.
- **DEC-010:** documentation in the closing story: `.agent_docs/tools.md` (truncation detected for both versions, encrypted file names included; the RAR4 limitation sentence at `:208-210` removed), `README.md`, `AGENTS.md` (`archive_rar_crypt.rs` entry, RAR header decryption with in tree RustCrypto crates), `.agent_docs/testing.md` (filter `e2e_0087_`, reduced loop, fixture provenance). **Obliged by:** n/a (no code impact): project convention. **Rationale:** `.agent_docs/tools.md:209` states the limitation this spec removes. **Alternatives considered:** none. **Source:** AGENTS.md. **Evidence:** as cited.
- **DEC-011:** no `unsafe`, no `unrar_sys` call. **Obliged by:** SPEC-0087/FR-MOD-001 (spec 14.3 constraint, SPEC-0080 DEC-006). **Rationale:** `unsafe_code` is forbidden workspace wide. **Alternatives considered:** section 2. **Source:** constitution of the workspace lints. **Evidence:** spec 14.3.
- **DEC-012:** stories US-0199, US-0200 and US-0203 add tests whose rule an earlier story built; each test green on arrival is proven by a mutation reasoned to change its observable outcome (listed per story); the two crypto stories are `min_tier` 1. **Obliged by:** n/a (no code impact): story planning. **Rationale:** precedent US-0172, US-0180 to US-0182; decision density of a binary format plus two key derivations. **Source:** design. **Evidence:** n/a.

## 11. Stories

| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0198 | refuse a cut RAR4 archive with one message | P1 | SPEC-0087/FR-NEW-001, FR-NEW-002, FR-NEW-003, FR-NEW-007, FR-MOD-001 | US-0197 | 2 | no |
| US-0199 | complete RAR archives unchanged, folders and solid cuts refused | P1 | SPEC-0087/FR-NEW-001, FR-NEW-007, FR-NEW-009, FR-MOD-001 | US-0198 | 2 | no |
| US-0200 | every block kind counts: stored data, recovery record, comment | P1 | SPEC-0087/FR-NEW-001, FR-NEW-006, FR-NEW-007, FR-MOD-001 | US-0199 | 2 | no |
| US-0201 | decrypt RAR5 headers to see a cut with the right password | P1 | SPEC-0087/FR-NEW-001, FR-NEW-005, FR-NEW-007, FR-MOD-001 | US-0200 | 1 | no |
| US-0202 | decrypt RAR4 headers to see a cut with the right password | P1 | SPEC-0087/FR-NEW-001, FR-NEW-006, FR-MOD-001 | US-0201 | 1 | no |
| US-0203 | password questions first, every cut of an encrypted names archive | P1 | SPEC-0087/FR-NEW-004, FR-NEW-005, FR-NEW-007, FR-NEW-009, FR-MOD-001 | US-0202 | 2 | no |
| US-0204 | cut RAR archives through the HTTP door and documentation | P2 | SPEC-0087/FR-NEW-008 | US-0203 | 2 | yes |
