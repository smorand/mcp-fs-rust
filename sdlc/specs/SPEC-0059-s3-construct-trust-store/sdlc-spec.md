# S3 blob store construction depends on the OS trust store: Bug Specification

> Generated on: 2026-10-29
> Id: SPEC-0059
> Nature: BUG
> Depth: S
> Depth evidence: 1 module (`storage/blob/s3.rs`), 4 requirements, no public contract change, no data model change
> Status: implemented
> Verdict: authority exists
> Defect class: n/a (code violates a clear authority)
> Severity: Low
> From backlog: BL-0084
> Security: n/a
> CVSS: n/a
> Affected: every build where `S3BlobStore::new` runs with the OS trust store unreadable or empty (debug and test builds panic; release builds start with no root)
> Fixed in: unreleased

## 1. Defect

### BUG-001: constructing an S3 blob store panics, or blocks, when the OS trust store cannot be read
- **Observed behavior:** `S3BlobStore::new` ends with `Client::from_conf(builder.build())` (`crates/core/src/storage/blob/s3.rs:38`). No HTTP client is given to the builder (`s3.rs:29-37`), so the AWS SDK builds its default rustls client, which loads the operating system root certificates through a process wide lazy (`aws-smithy-http-client-1.5.0/src/client/tls/rustls_provider.rs:117-129`, `NATIVE_ROOTS`) and then asserts that at least one parsed (`rustls_provider.rs:162-163`, a `debug_assert!`). The construction happens even when `endpoint` starts with `http://`, where no certificate is ever used.
- **Expected per authority:** `AGENTS.md` section "Quality gate": "`cargo test --workspace` ... must all be clean before any commit". A unit test that constructs a store against `http://127.0.0.1:9000` (`s3.rs:224-243`) must pass whatever the host trust store holds.
- **Reproduction:** preconditions: debug test profile, macOS arm64, no MinIO running. Input: `cargo test -p mcp-fs-core --lib constructs_with_path_style_and_bucket_name` with `SSL_CERT_FILE=/tmp/empty.pem SSL_CERT_DIR=/tmp` (`/tmp/empty.pem` is an empty file). Expected: `ok`. Actual (run on 2026-10-29): `FAILED`, panic at `rustls_provider.rs:163` "TrustStore configured to enable native roots but no valid root certificates parsed!". Without the two variables the same test passes (`1 passed`, 1.56 s).
- **Root cause:** `s3.rs:38` builds the SDK default HTTP client, whose TLS setup requires a readable, non empty OS trust store (`rustls_provider.rs:159-165`, `:200`). On a loaded machine `rustls_native_certs` fails on macOS ("failed to load user/admin/system trust settings", Os code -36, `.implement/US-0089-verify.log:12045`) and the test hangs 60 s (`US-0089-verify.log:10581`) then panics (`US-0089-verify.log:13252-13253`). The 137 s gap between the two warnings (`US-0089-verify.log:12045-12046`) is the observed blocking; why the OS call blocks is `ASSUMED:` machine load, not reproduced.
- **Contributing factors:** the construction is eager, not at first request; `debug_assert!` makes the failure exist only in debug and test builds, so a release server silently starts with an empty root set; the full suite runs about 3000 tests in parallel (`US-0089-verify.log:13260`: 2960 passed, 315 s), which loads the host.
- **Impact:** a false red on `make check` (one occurrence, BL-0084). In production, a debug build of the server with `infra.blob.backend: minio` panics at the first project blob store (`storage/mod.rs:172`, also reached from `git/repo.rs:204`, `migrate.rs:287`, `storage/mod.rs:443,465`). Workaround: rerun the test.
- **Rollback posture:** a revert must leave existing buckets readable and writable on `http://` and `https://` endpoints; no stored data or config key changes.

## 2. Expected Behavior

#### BR-001 [EARS-E]: no panic on an empty trust store for a plain HTTP endpoint
> WHEN a blob store is built for backend `minio` or `s3` with an `endpoint` starting with `http://` and the OS trust store holds no usable certificate THE system SHALL return `Ok` from `S3BlobStore::new`.

- **Source:** restored from `AGENTS.md` "Quality gate" (clean `cargo test --workspace`).
- **Inputs:** `BlobConfig { backend: "minio", endpoint: "http://127.0.0.1:9000", region: "us-east-1", ... }`; env `SSL_CERT_FILE` naming an empty file.
- **Outputs:** `Ok(store)` with `store.bucket() == "mcpfs-unit"`.
- **Exact names:** `S3BlobStore::new`, `S3BlobStore::bucket`, env `SSL_CERT_FILE`, `SSL_CERT_DIR`.

#### BR-002 [EARS-E]: no wait on the OS trust store for a plain HTTP endpoint
> WHEN a blob store is built for an `endpoint` starting with `http://` THE system SHALL return without reading the OS trust store, so the call does not wait on it.

- **Source:** restored from `AGENTS.md` "Quality gate"; the 60 s hang at `US-0089-verify.log:10581`.
- **Inputs:** env `SSL_CERT_FILE` naming a named pipe (FIFO) that no process opens for writing; a read of it blocks forever.
- **Outputs:** `Ok(store)` within 10 s.

#### BR-003 [EARS-O]: no panic on an empty trust store for an HTTPS endpoint
> IF the `endpoint` starts with `https://` or is empty (AWS default) and the OS trust store holds no usable certificate THEN THE system SHALL return `Ok` from `S3BlobStore::new` and SHALL fail the first request with the existing storage error instead.

- **Source:** decided here: ASSUMED, matches today's release behavior (empty root set, `debug_assert!` compiled out), so no user visible change in production. Alternative (construction returns `Err`) rejected: it would turn a startup that works today into a refusal.
- **Inputs:** `endpoint: "https://s3.example.test"`, env as BR-001.
- **Outputs:** `Ok(store)`; `ensure_bucket()` returns `Err` (no root, handshake refused).

#### BR-004 [EARS-UB]: certificate verification stays on for HTTPS
> The system SHALL NOT accept an HTTPS server certificate that does not chain to a root of the OS trust store.

- **Source:** decided here: ASSUMED, guard against a fix that would silence BUG-001 by weakening TLS.
- **Inputs:** `endpoint: "https://127.0.0.1:<port>"` served with a self signed certificate absent from the trust store.
- **Outputs:** the first request returns `Err`; the server logs a completed TCP accept and no HTTP request line.

## 3. Edge Cases

| Case | Expected behavior | Requirement |
|------|-------------------|-------------|
| `endpoint` empty (real AWS) and a normal trust store | `Ok`; TLS verified against OS roots | BR-003, BR-004 |
| `SSL_CERT_FILE` unset, healthy host | `Ok` in under 5 s (today 1.56 s) | BR-001 |
| `http://` endpoint and a healthy trust store | `Ok`; behavior identical to today | BR-001 |
| `SSL_CERT_FILE` names a nonexistent path | `Ok` for `http://`; `Ok` for `https://` | BR-001, BR-003 |

## 4. Tests

The load dependent hang cannot be reproduced on demand. The environment variables `SSL_CERT_FILE` and `SSL_CERT_DIR` are honored by the loader on every platform (`rustls-native-certs-0.8.4/src/lib.rs:8,52-53`), and the loader runs once per process (`LazyLock`, `rustls_provider.rs:117`), so each test below runs its case in a child process of the test binary (`std::process::Command` on `std::env::current_exe()` with `--exact` and the variables set).

### 4.1 End to end reproduction (mandatory)
#### BT-001: construct against http with an empty trust store
- **Reproduces:** BUG-001
- **Driver:** child process of the lib test binary running `storage::blob::s3::tests::constructs_with_path_style_and_bucket_name`, env `SSL_CERT_FILE=<empty file>`, `SSL_CERT_DIR=<empty dir>`
- **Fails today because:** `rustls_provider.rs:163` panics on the empty root set during `s3.rs:38`.
- Given an empty PEM file and an empty directory / When the child runs the existing construct test / Then the child exits with status 0 and its stdout contains `test storage::blob::s3::tests::constructs_with_path_style_and_bucket_name ... ok`

### 4.2 New behavior
#### BT-010: no wait on the OS store (http)
- **Validates:** BR-002
- **Driver:** child process as BT-001, `SSL_CERT_FILE=<FIFO created with mkfifo, never opened>`
- Given the FIFO / When the child builds the store for `http://127.0.0.1:9000` / Then the child exits 0 within 10 s (the parent kills it at 10 s and fails the test). Red today: the child blocks reading the FIFO.

#### BT-011: https with an empty store builds, then fails on use
- **Validates:** BR-003
- **Driver:** child process as BT-001
- Given `endpoint: "https://127.0.0.1:1"` and an empty PEM file / When `S3BlobStore::new` runs then `ensure_bucket().await` / Then `new` returns `Ok` and `ensure_bucket` returns `Err`, the child exits 0. Red today: debug panic at `rustls_provider.rs:163`.

#### BT-012: an untrusted certificate is refused
- **Validates:** BR-004
- **Driver:** child process as BT-001, plus `openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=127.0.0.1` and `openssl s_server -www -accept <port>` (test prints `SKIPPED: openssl not found` and passes when the binary is absent)
- Given a self signed certificate and `SSL_CERT_FILE` naming a valid PEM that does not contain it / When `ensure_bucket()` runs against `https://127.0.0.1:<port>` / Then it returns `Err`, and the `s_server` output contains no `GET` line.

### 4.3 Non regression
#### BT-020: the integration tests keep their opt in contract
- **Protects:** `integration_put_get_range_delete`, `integration_list_follows_pagination`, `integration_modified_reads_last_modified` (`s3.rs:245-308`) stay `#[ignore]` and, with MinIO on :9000, `make test-e2e-full` stays green.
- Given MinIO up (`docker-compose.test.yml`) / When `make test-e2e-full` runs / Then exit status 0.

#### BT-021: unknown backend still refused
- **Protects:** `build_blob_store(&c, "p")` with an unknown backend returns `ERR_INVALID_ARGUMENT` (`storage/mod.rs:702`).
- Given backend `"nope"` / When `build_blob_store` runs / Then the error code is `ERR_INVALID_ARGUMENT` and the message contains `unknown blob backend 'nope'`.

### 4.4 Existing tests asserting the current behavior
| Test file | Test name | Action | Reason |
|-----------|-----------|--------|--------|
| `crates/core/src/storage/blob/s3.rs` | `constructs_with_path_style_and_bucket_name` | Kept as is | it is the subject of BT-001; its assertion (`bucket() == "mcpfs-unit"`) stays valid |
| `crates/core/src/storage/blob/s3.rs` | `integration_*` (3) | Kept as is | ignored, see BT-020 |
| `crates/core/src/storage/mod.rs` | unknown backend test (`:702`) | Kept as is | see BT-021 |
| `crates/core/tests/full_stack_e2e.rs` | caller of `build_blob_store` (`:128`) | Kept as is | exercised by `make test-e2e-full` |

## 5. Existing Specs Affected
none. `SPEC-0020`, `SPEC-0041` and `SPEC-0043` mention S3 or MinIO but state no requirement on client construction.

## 6. Assumptions
- **Assumption:** the backlog hypothesis (an AWS credential or region provider probe) is wrong: credentials and region are static (`s3.rs:22-32`), no `aws-config` provider is called. The cause is the TLS trust store, evidenced by `US-0089-verify.log:12045,13252`. Shaped the whole document.
- **Assumption:** the long wait is the OS call blocking under load; not reproduced (BT-010 reproduces the wait with a FIFO, a stand in).
- **Assumption (BR-003, BR-004):** see their `Source:`.
- **Assumption:** the SDK lets a caller build the client without the OS roots for `http://`; this is for `/sdlc-design` to confirm (`TrustStore::with_native_roots`, `aws-smithy-http-client-1.5.0/src/client/tls.rs:119`). If it cannot, BR-001 and BR-002 fail and need a product decision.
- **Backlog entries created by this run:** none. BL-0084 is consumed.

## 7. Implementability Checklist

| Item | Result |
|---|---|
| 1 No technical decision | PASS (the `TrustStore` pointer in Section 6 is a feasibility note, not a decision) |
| 2 No buried requirement | PASS |
| 3 Observable names spelled | PASS (env vars, function names, `ERR_INVALID_ARGUMENT`, `endpoint` scheme) |
| 4 No forced choice | PASS (BR-003 fixes `Ok` then `Err` on first use) |
| 5 Tests specified | PASS |
| 6 Reproduction end to end | PASS (BT-001; the test process is the reporter's, as BL-0084 is a test failure) |
| 7ante Security misclassification | PASS (no advisory, injection or exposure; BR-004 is a guard) |
| 8 No hidden prerequisite | PASS (openssl optional, skip stated) |
| 9 EARS clean | PASS |
| 10 Right tool | PASS (4 requirements, S range) |
| 11 Length | PASS |
| 12 Claims cited | PASS |
| 13 Capability verified | PASS (writer and reader of the loader read: `rustls_provider.rs:117-129,159-165`) |
| 14 Verdict consistent | PASS (BR-001, BR-002 restore; BR-003, BR-004 decided as edge cases) |

| Verdict | IMPLEMENTABLE |
|---|---|
| Open F failures | 0 |
| A failures | amended in place: 0; moved to backlog: none |
