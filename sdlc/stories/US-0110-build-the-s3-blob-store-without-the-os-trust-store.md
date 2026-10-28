# US-0110: build the S3 blob store without depending on the OS trust store

> Spec: SPEC-0059 (sdlc/specs/SPEC-0059-s3-construct-trust-store/)
> Nature: BUG
> Priority: P1
> Depends on: main
> min_tier: 2
> Files touched: 5
> Closes spec: yes

## Objective
`S3BlobStore::new` panics (debug and test builds) or waits when the operating system root certificates cannot be read, even for a plain `http://` endpoint where none is used. It must build without reading the OS store for `http://`, and never panic for `https://`, while keeping certificate verification on. This is the whole of SPEC-0059, so the story closes it.

## Requirements covered
- SPEC-0059/BR-001: WHEN a blob store is built for backend `minio` or `s3` with an `endpoint` starting with `http://` and the OS trust store holds no usable certificate THE system SHALL return `Ok` from `S3BlobStore::new`.
- SPEC-0059/BR-002: WHEN a blob store is built for an `endpoint` starting with `http://` THE system SHALL return without reading the OS trust store, so the call does not wait on it.
- SPEC-0059/BR-003: IF the `endpoint` starts with `https://` or is empty (AWS default) and the OS trust store holds no usable certificate THEN THE system SHALL return `Ok` from `S3BlobStore::new` and SHALL fail the first request with the existing storage error instead.
- SPEC-0059/BR-004: The system SHALL NOT accept an HTTPS server certificate that does not chain to a root of the OS trust store.

## Independent Test
`cargo test -p mcp-fs-core --lib storage::blob::s3` is green, and
`SSL_CERT_FILE=/tmp/empty.pem SSL_CERT_DIR=/tmp cargo test -p mcp-fs-core --lib constructs_with_path_style_and_bucket_name` (with `/tmp/empty.pem` an empty file) prints `1 passed` instead of the panic at `rustls_provider.rs:163`. `git diff Cargo.lock` lists only the three crates under `mcp-fs-core`.

## Design excerpt
### Decisions
- DEC-001 (SPEC-0059): Fix at `S3BlobStore::new`: give the SDK builder an explicit HTTP client by endpoint scheme; for `http://` use `aws_smithy_http_client::Builder::new().tls_provider(tls::Provider::Rustls(tls::rustls_provider::CryptoMode::AwsLc)).tls_context(TlsContext::builder().with_trust_store(TrustStore::empty()).build()?).build_https()`.
- DEC-002 (SPEC-0059): For `https://` and empty endpoints, probe the OS store once per process (`OnceLock<bool>`, `rustls_native_certs::load_native_certs` then `RootCertStore::add_parsable_certificates`); usable gives the SDK default trust store, unusable gives `TrustStore::empty()` on the same builder. Promote `aws-smithy-http-client` (features `rustls-aws-lc`, default features off), `rustls-native-certs` and `rustls` (default features off) to direct entries.
### Data model
None.
### Contracts
`S3BlobStore::new(cfg: &BlobConfig, bucket: String) -> Result<Self>` unchanged. New private items in `s3.rs`: `fn http_client(endpoint: &str) -> SharedHttpClient`, `fn usable_native_roots() -> bool`, `static NATIVE_ROOTS_USABLE: OnceLock<bool>`. The builder call is `aws_sdk_s3::config::Builder::http_client(..)`, inserted before `.build()` at `s3.rs:38`.
### Paths
| Path | Action |
|---|---|
| `crates/core/src/storage/blob/s3.rs` | modify |
| `Cargo.toml` | modify: three entries in `[workspace.dependencies]` after `aws-credential-types` (line 45) |
| `crates/core/Cargo.toml` | modify: three `{ workspace = true }` entries |
| `Cargo.lock` | regenerated |
| `.agent_docs/architecture.md` | modify: one dated entry (blob client TLS and why the endpoint scheme decides) |
### Existing patterns
Test modules sit at the bottom of the file under `#[cfg(test)] mod tests` with a `cfg()` helper (`s3.rs:221-235`). Dependencies are declared once in `Cargo.toml` `[workspace.dependencies]` and referenced as `dep = { workspace = true }` in `crates/core/Cargo.toml`. Opt in tests use `#[ignore = "..."]` (`s3.rs:245`).

## Acceptance tests
> 100% must pass, run through `make check` (or the project target). Never run test files ad hoc.

Child process mechanism (BT-001, 010, 011, 012): the parent test runs `std::env::current_exe()` with args `--ignored --exact storage::blob::s3::tests::<child_name> --nocapture` and the stated environment, and reads the exit status and stdout. The child is a `#[test] #[ignore]` function in the same module. The SDK loads the OS roots once per process, hence the child.

### SPEC-0059/BT-001: construct against http with an empty trust store
- Level / Driver / Location: functional e2e / child process / `s3.rs` tests, `bt_0059_bt_001_http_construct_with_an_empty_trust_store`
- Preconditions: an empty file `empty.pem` and an empty directory `certs/` in a `tempfile::tempdir()`.
- Given `SSL_CERT_FILE=<empty.pem>` and `SSL_CERT_DIR=<certs/>` / When the child runs the existing `constructs_with_path_style_and_bucket_name` / Then the child exit status is 0 and stdout contains `test storage::blob::s3::tests::constructs_with_path_style_and_bucket_name ... ok`.
- Red before the fix because: `rustls_provider.rs:163` panics, the child exits 101.

### SPEC-0059/BT-010: no wait on the OS store (http)
- Level / Driver / Location: functional e2e / child process / `s3.rs` tests, `bt_0059_bt_010_http_construct_does_not_wait_on_the_os_store`
- Preconditions: a FIFO `store.pem` created with `libc`-free `Command::new("mkfifo")` in a tempdir, never opened for writing.
- Given `SSL_CERT_FILE=<store.pem>` / When the child runs `constructs_with_path_style_and_bucket_name` and the parent waits at most 10 s (`try_wait` loop, then `kill`) / Then the child exits with status 0 before 10 s.
- Red before the fix because: the loader reads the FIFO and blocks, the parent kills it at 10 s and fails.
- Cleanup: the tempdir drops, the child is killed on timeout.

### SPEC-0059/BT-011: https with an empty store builds, then fails on use
- Level / Driver / Location: functional e2e / child process / `s3.rs` tests, `bt_0059_bt_011_https_with_an_empty_store_builds_then_fails_on_use`; child `child_https_empty_store`
- Preconditions: as BT-001.
- Given `SSL_CERT_FILE=<empty.pem>`, `SSL_CERT_DIR=<certs/>`, `endpoint: "https://127.0.0.1:1"` / When the child runs `S3BlobStore::new(..)` then `ensure_bucket().await` (tokio test) / Then `new` returns `Ok`, `ensure_bucket` returns `Err`, the child exits 0.
- Red before the fix because: the same panic at `rustls_provider.rs:163` during `new`.

### SPEC-0059/BT-012: an untrusted certificate is refused
- Level / Driver / Location: functional e2e / child process plus `openssl s_server` / `s3.rs` tests, `bt_0059_bt_012_an_untrusted_certificate_is_refused`; child `child_https_untrusted_cert`
- Preconditions: `openssl` on PATH, else the test prints `SKIPPED: openssl not found` and returns. In a tempdir: `openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1 -keyout k.pem -out c.pem`; a second self signed `other.pem` generated the same way; an `s_server` started with `openssl s_server -www -accept <free port> -cert c.pem -key k.pem`, its output captured; the port read from a bound then dropped `TcpListener`.
- Given `SSL_CERT_FILE=<other.pem>` (a valid PEM that does not contain `c.pem`) and `endpoint: "https://127.0.0.1:<port>"` / When the child runs `ensure_bucket().await` / Then it returns `Err`, the child exits 0, and the captured `s_server` output contains no line starting with `GET`.
- Red before the fix: not red (protects BR-004 against a fix that weakens TLS); it may pass at once.
- Cleanup: kill `s_server`, drop the tempdir.

### SPEC-0059/BT-020: integration tests keep their opt in contract
- Level / Driver / Location: functional e2e / `make test-e2e-full` (real MinIO) / existing `crates/core/tests/full_stack_e2e.rs`
- Given MinIO up (`docker-compose.test.yml`) / When `make test-e2e-full` runs / Then exit status 0; `integration_put_get_range_delete`, `integration_list_follows_pagination`, `integration_modified_reads_last_modified` remain `#[ignore]` and unedited.
- Passes at once: protects behavior that already holds. If Docker is unavailable, record the command and its output as not run, never as green.

### SPEC-0059/BT-021: unknown backend still refused
- Level / Driver / Location: unit / direct call / existing test at `crates/core/src/storage/mod.rs:702`, unedited
- Given backend `"nope"` / When `build_blob_store` runs / Then the error code is `ERR_INVALID_ARGUMENT` and the message contains `unknown blob backend 'nope'`.
- Passes at once.

## Constraints
- Files not to touch: `tool-contract-golden.json`, `TOOL_CONTRACT.txt`, `config/`, the three `integration_*` tests, `crates/core/src/storage/mod.rs`.
- Dependencies not to add: anything other than `aws-smithy-http-client`, `rustls-native-certs`, `rustls`; none may bring a new package into `Cargo.lock`.
- Patterns to avoid: `catch_unwind`, `webpki-roots`, any verifier that skips certificate checks, `unwrap`/`expect` on fallible paths, dashes as punctuation.
- Scope boundary: no change to credentials, region, path style, retries or timeouts of the S3 client.

## Definition of done
- [ ] RED evidence recorded for BT-001, BT-010 and BT-011 before the production code (command and output).
- [ ] Every acceptance test above green; `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` clean; main stays green after the squash.
- [ ] Non regression: `constructs_with_path_style_and_bucket_name` unedited and green with the host store; `git diff Cargo.lock` shows only the three crates under `mcp-fs-core`.
- [ ] Docs updated: `.agent_docs/architecture.md` dated entry (empty store, endpoint scheme, one process wide probe).
- [ ] Self review: does an `http://` build touch the OS store (it must not)? Is the probe memoized (one read per process)? Is verification still on for `https://`? Any `unwrap` left in production code? Any dash used as punctuation?
- [ ] Closes spec: every BT of SPEC-0059 green (BT-001, 010, 011, 012, 020, 021) and BR-001 to BR-004 each cited by this story.
- [ ] One commit `US-0110: build the S3 blob store without depending on the OS trust store`, body `Spec: SPEC-0059` and `BR-001, BR-002, BR-003, BR-004`.
