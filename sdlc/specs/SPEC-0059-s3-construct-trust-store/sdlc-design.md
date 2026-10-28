# SPEC-0059: S3 blob store construction depends on the OS trust store, design

> Spec: sdlc/specs/SPEC-0059-s3-construct-trust-store/sdlc-spec.md
> Nature: BUG
> Depth: S
> Security: n/a
> Designed on: 2026-10-29
> Target tier: 2 (default: no `--tier`, no `Tier:` in `sdlc/constitution.md`, no `.spec.json`)

## 1. Stack and skills
| Element | Choice | Version | DEC |
|---|---|---|---|
| Language, runtime | Rust 2024, tokio (unchanged) | `Cargo.toml` | n/a |
| S3 client | `aws-sdk-s3` (unchanged) | 1.152.0 (`Cargo.lock`) | n/a |
| HTTP client builder | `aws-smithy-http-client`, promoted from transitive to direct | requirement `1.5`, lock stays 1.5.0 (crates.io latest 1.5.2) | DEC-002 |
| Root probe | `rustls-native-certs` 0.8 and `rustls` 0.23, promoted from transitive to direct | lock stays 0.8.4 and 0.23.46 (crates.io latest 0.8.5) | DEC-002 |

context7 does not index these crates (`resolve-library-id` returned only the Smithy IDL); versions come from `Cargo.lock` and the crates.io API. No crate is added to the build, only to the manifest. Skills loaded: `rust` (gate: `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, per `AGENTS.md` Quality gate).

## 2. Approaches
### Chosen: choose the HTTP client's trust store at construction, by endpoint scheme
`S3BlobStore::new` (`crates/core/src/storage/blob/s3.rs:21`) passes an explicit HTTP client to the SDK builder instead of letting `Client::from_conf` (`s3.rs:38`) build the default one. For `http://` the client never reads the OS store. For `https://` or an empty endpoint it reads the OS store through the SDK default, unless a one time probe finds no usable root, in which case it starts with an empty root set. Pros: fixes the root cause site; production behavior for a healthy host is unchanged. Cons: a new direct dependency set; a process wide probe adds one OS store read on the first `https` construction. Scope: `storage/blob/s3.rs`, manifests.
### Rejected: fix only the test
Give the unit test a custom HTTP client or set the variables in it. Hides BUG-001 for the test and leaves the debug build of the server panicking at `storage/mod.rs:172` (DEC-001).
### Rejected: build the SDK default client lazily at the first request
Moves the panic and the wait to the first `http://` request (DEC-001).
### Rejected: swallow the panic with `catch_unwind`
Does not stop the wait (BR-002) and would leave the lazy cell of the SDK poisoned for the process (DEC-002).

## 3. Architecture
One module. `S3BlobStore::new` keeps its signature and its static credentials and region (`s3.rs:22-32`). A private function `http_client(endpoint: &str) -> SharedHttpClient` returns the client given to `.http_client(..)` on the builder. A private `usable_native_roots() -> bool` is memoized in a `static OnceLock<bool>`; it calls `rustls_native_certs::load_native_certs()`, adds the certificates to a `rustls::RootCertStore` with `add_parsable_certificates`, and returns `valid > 0`. Callers: `build_blob_store` (`storage/mod.rs:172`) from `git/repo.rs:204`, `migrate.rs:287`, `storage/mod.rs:443,465`; all keep calling `S3BlobStore::new` unchanged.

## 4. Data model
None.

## 5. Contracts and interfaces
No tool, REST, config key or schema change (`tool-contract-golden.json` and `config/` untouched). `S3BlobStore::new(cfg: &BlobConfig, bucket: String) -> Result<Self>` unchanged. New private items in `s3.rs`: `fn http_client(endpoint: &str) -> SharedHttpClient`, `fn usable_native_roots() -> bool`, `static NATIVE_ROOTS_USABLE: OnceLock<bool>`. Errors: none new; a failed request on an `https` endpoint with no root surfaces through the existing storage error mapping of `put`, `get`, `exists`, `ensure_bucket`.

## 6. File structure impact
| Path | Action | Caused by | Story |
|---|---|---|---|
| `crates/core/src/storage/blob/s3.rs` | modify: `new`, `http_client`, `usable_native_roots`, tests BT-001, 010, 011, 012, 021 | BR-001 to BR-004 | US-0110 |
| `Cargo.toml` | modify: three entries in `[workspace.dependencies]` | DEC-002 | US-0110 |
| `crates/core/Cargo.toml` | modify: three `workspace = true` entries | DEC-002 | US-0110 |
| `Cargo.lock` | regenerated, expected diff: the three crates listed under `mcp-fs-core` dependencies only | DEC-002 | US-0110 |
| `.agent_docs/architecture.md` | modify: one dated entry on blob client TLS | documentation | US-0110 |

Affected existing tests: `constructs_with_path_style_and_bucket_name` (`s3.rs:237`) kept as is; the three `integration_*` tests stay `#[ignore]`; the unknown backend test (`storage/mod.rs:702`) kept. `README.md` and `AGENTS.md` need no change (no command, structure or contract moves).

## 7. Compatibility and risks
Healthy host: unchanged behavior for `https://` and empty endpoints (SDK default trust store, BR-004); `http://` endpoints skip an OS store read they never needed. Rollback: revert the commit; no stored data, bucket name or config key is touched. Dependencies: three promoted, none added to `Cargo.lock` packages. Risks: (1) the SDK's empty root path must not panic; `rustls_provider.rs:206-210` shows the `expect` only on the branch with additional server names, which we do not set, so the empty store goes through `with_root_certificates` (BT-011 proves it, red first). (2) `Builder::build_http` is `#[doc(hidden)]` (`client.rs:1099`), so the `http://` client uses the documented `tls_provider(..).tls_context(..).build_https()` with `TrustStore::empty()` instead (DEC-001). (3) the probe reads the OS store once more than the SDK does on `https`, about 300 ms on macOS (SDK comment `rustls_provider.rs:112-115`), once per process.

## 9. Test strategy
| Test | Level | Driver | Location | Story |
|---|---|---|---|---|
| BT-001 | functional e2e | child process of the lib test binary, empty `SSL_CERT_FILE` | `s3.rs` tests (`bt_0059_bt_001_*`) | US-0110 |
| BT-010 | functional e2e | child process, FIFO as `SSL_CERT_FILE`, 10 s kill | `s3.rs` tests | US-0110 |
| BT-011 | functional e2e | child process, `https://127.0.0.1:1`, empty store | `s3.rs` tests | US-0110 |
| BT-012 | functional e2e | child process plus `openssl s_server`, skipped when absent | `s3.rs` tests | US-0110 |
| BT-020 | functional e2e | `make test-e2e-full` (real MinIO) | `crates/core/tests/full_stack_e2e.rs` (existing) | US-0110 |
| BT-021 | unit | direct call | `storage/mod.rs:702` (existing) | US-0110 |

Mechanism: each child case is a `#[test] #[ignore]` function named `child_*` run by the parent through `std::process::Command::new(std::env::current_exe()?)` with `--ignored --exact <name> --nocapture` and the variables set, because the SDK loads the OS roots once per process (`rustls_provider.rs:117`). Test data is generated (empty file, empty directory, `mkfifo`, `openssl req -x509` into a temp dir). BT-001, BT-010 and BT-011 are RED before the fix (panic at `rustls_provider.rs:163`, a blocked read of the FIFO, the same panic). BT-012, BT-020 and BT-021 protect behavior that already holds and may pass at once: stated in the story.

## 10. Decisions log
- **DEC-001:** Fix at the construction site `S3BlobStore::new`: give the SDK builder an explicit HTTP client chosen by endpoint scheme; for `http://` use `aws_smithy_http_client::Builder::new().tls_provider(tls::Provider::Rustls(tls::rustls_provider::CryptoMode::AwsLc)).tls_context(TlsContext::builder().with_trust_store(TrustStore::empty()).build()?).build_https()`. **Obliged by:** SPEC-0059/BR-001, SPEC-0059/BR-002. **Rationale:** the root cause is the eager default client at `s3.rs:38`; `TrustStore::empty()` never reads the OS store (`client/tls.rs:108-113`) and, being the documented API, avoids the `#[doc(hidden)]` `build_http`. **Alternatives considered:** test only fix (leaves the server defect); lazy default client (moves the failure to the first request); `build_http` (hidden API, may vanish). **Source:** existing code. **Evidence:** `crates/core/src/storage/blob/s3.rs:21-38`, `aws-smithy-http-client-1.5.0/src/client/tls.rs:108-120`, `client.rs:938-950,1099-1100`.
- **DEC-002:** For `https://` and empty endpoints, probe the OS store once per process (`OnceLock<bool>`, `rustls_native_certs::load_native_certs` then `RootCertStore::add_parsable_certificates`); usable gives the SDK default trust store, unusable gives `TrustStore::empty()` on the same builder. Promote `aws-smithy-http-client` (features `rustls-aws-lc`, default features off), `rustls-native-certs` and `rustls` (default features off) from transitive to direct entries. **Obliged by:** SPEC-0059/BR-003, SPEC-0059/BR-004. **Rationale:** the SDK asserts on an empty root set only when native roots are enabled (`rustls_provider.rs:160-163`); probing first keeps verification on (BR-004) and lets the first request fail (BR-003). The crates are already in `Cargo.lock` at these versions, so the build gains no code; this is ASK ONLY FOR category 2 in form only, default applied because the recommendation stays inside the existing stack. **Alternatives considered:** `catch_unwind` (poisons the lazy cell, does not bound the wait); bundled `webpki-roots` (changes which CAs are trusted, breaks BR-004's "OS trust store"); disabling verification (forbidden by BR-004). **Source:** existing code. **Evidence:** `aws-smithy-http-client-1.5.0/src/client/tls/rustls_provider.rs:117-129,159-165,206-210`, `Cargo.lock:828-846,4977,4992`; versions: crates.io API 2026-10-29.

## 11. Stories
| Story | Title | Priority | Requirements | Depends on | min_tier | Closes spec |
|---|---|---|---|---|---|---|
| US-0110 | build the S3 blob store without depending on the OS trust store | P1 | BR-001, BR-002, BR-003, BR-004 | main | 2 | yes |
