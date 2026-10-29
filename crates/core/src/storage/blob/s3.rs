//! S3 / MinIO blob backend. Port of the C# `Storage/MinioBlobStore.cs`.
//!
//! One bucket per volume (`{bucket_prefix}{project_id}`), object key = sha256. That
//! naming is a stable contract: an existing bucket must stay readable after an
//! upgrade. Path-style addressing is forced, which is what MinIO needs.

use crate::config::BlobConfig;
use crate::errors::{Result, ToolError};
use crate::storage::traits::{BlobBackend, BlobEntry};
use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, SharedHttpClient};
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_http_client::tls::rustls_provider::CryptoMode;
use aws_smithy_http_client::tls::{Provider, TlsContext, TrustStore};
use std::sync::OnceLock;

/// Whether the OS trust store yields at least one usable root, probed once per process.
static NATIVE_ROOTS_USABLE: OnceLock<bool> = OnceLock::new();

/// The SDK default client loads the OS roots eagerly and asserts that one parsed, so an
/// unreadable or empty store panics a debug build or stalls the call (SPEC-0059). Reading
/// them ourselves first lets an unusable store degrade to "trust nothing" instead.
fn usable_native_roots() -> bool {
    *NATIVE_ROOTS_USABLE.get_or_init(|| {
        let loaded = rustls_native_certs::load_native_certs();
        let mut roots = rustls::RootCertStore::empty();
        let (valid, _invalid) = roots.add_parsable_certificates(loaded.certs);
        if valid == 0 {
            tracing::warn!(
                errors = loaded.errors.len(),
                "no usable OS root certificate: https endpoints will fail"
            );
        }
        valid > 0
    })
}

/// The HTTP client for one endpoint. A plain `http://` endpoint never does TLS, so it must
/// not depend on the OS store; any other endpoint keeps verifying against it (BR-004).
fn http_client(endpoint: &str) -> Result<SharedHttpClient> {
    let plain = endpoint.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"));
    let trust = TrustStore::empty().with_native_roots(!plain && usable_native_roots());
    let tls = TlsContext::builder()
        .with_trust_store(trust)
        .build()
        .map_err(|e| ToolError::internal(format!("s3 tls context: {e}")))?;
    Ok(aws_smithy_http_client::Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
        .tls_context(tls)
        .build_https())
}

pub struct S3BlobStore {
    client: Client,
    bucket: String,
}

impl S3BlobStore {
    pub fn new(cfg: &BlobConfig, bucket: String) -> Result<Self> {
        let creds = Credentials::new(
            cfg.access_key.clone(),
            cfg.secret_key.clone(),
            None,
            None,
            "mcp-fs-config",
        );
        let mut builder = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region.clone()))
            .credentials_provider(creds)
            .http_client(http_client(&cfg.endpoint)?)
            // MinIO requires path-style addressing.
            .force_path_style(true);
        if !cfg.endpoint.is_empty() {
            builder = builder.endpoint_url(cfg.endpoint.clone());
        }
        Ok(Self { client: Client::from_conf(builder.build()), bucket })
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }
}

#[async_trait]
impl BlobBackend for S3BlobStore {
    async fn put(&self, sha256: &str, data: &[u8]) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(sha256)
            .body(ByteStream::from(data.to_vec()))
            .send()
            .await
            .map_err(|e| ToolError::internal(format!("s3 put '{sha256}': {e}")))?;
        Ok(())
    }

    async fn get(&self, sha256: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
        let mut req = self.client.get_object().bucket(&self.bucket).key(sha256);
        // Byte range, when a window was requested.
        if offset > 0 || length.is_some() {
            let range = match length {
                Some(len) if len > 0 => format!("bytes={}-{}", offset, offset + len - 1),
                _ => format!("bytes={offset}-"),
            };
            req = req.range(range);
        }
        let out = req.send().await.map_err(|e| {
            // Same classification as `exists`: the real HTTP status, not a
            // `Display` substring match, is what tells a genuine 404 apart from
            // every other failure the SDK folds into the same generic message.
            if e.raw_response().map(|r| r.status().as_u16()) == Some(404) {
                ToolError::not_found(format!("blob '{sha256}' not found"))
            } else {
                ToolError::internal(format!("s3 get '{sha256}': {e}"))
            }
        })?;
        let bytes = out
            .body
            .collect()
            .await
            .map_err(|e| ToolError::internal(format!("s3 read body: {e}")))?;
        Ok(bytes.into_bytes().to_vec())
    }

    async fn exists(&self, sha256: &str) -> Result<bool> {
        match self.client.head_object().bucket(&self.bucket).key(sha256).send().await {
            Ok(_) => Ok(true),
            Err(e) => {
                // A HEAD 404 carries no XML error body, so the SDK cannot classify
                // it into a named error variant the way a GET/PUT 404 can: `e`'s
                // `Display` renders as the generic "service error" with none of
                // "NotFound"/"NoSuchKey"/"404" as a substring, which silently
                // turned every ordinary cache miss into ERR_INTERNAL_ERROR against
                // MinIO. `raw_response()` reads the real HTTP status directly,
                // which is the only classification a bodyless HEAD response gives.
                if e.raw_response().map(|r| r.status().as_u16()) == Some(404) {
                    Ok(false)
                } else {
                    Err(ToolError::internal(format!("s3 head '{sha256}': {e}")))
                }
            }
        }
    }

    async fn delete(&self, sha256: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(sha256)
            .send()
            .await
            .map_err(|e| ToolError::internal(format!("s3 delete '{sha256}': {e}")))?;
        Ok(())
    }

    async fn ensure_bucket(&self) -> Result<()> {
        if self.client.head_bucket().bucket(&self.bucket).send().await.is_ok() {
            return Ok(());
        }
        match self.client.create_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(e) => {
                let s = e.to_string();
                // Racing creators, or an already-owned bucket, are both fine.
                if s.contains("BucketAlreadyOwnedByYou") || s.contains("BucketAlreadyExists") {
                    Ok(())
                } else {
                    Err(ToolError::internal(format!("s3 create bucket '{}': {e}", self.bucket)))
                }
            }
        }
    }

    async fn remove_bucket(&self) -> Result<()> {
        // Empty it first: S3 refuses to delete a non-empty bucket.
        let mut continuation: Option<String> = None;
        loop {
            let mut req = self.client.list_objects_v2().bucket(&self.bucket);
            if let Some(t) = &continuation {
                req = req.continuation_token(t);
            }
            let out = match req.send().await {
                Ok(o) => o,
                // A missing bucket is already in the desired state.
                Err(_) => return Ok(()),
            };
            for obj in out.contents() {
                if let Some(k) = obj.key() {
                    let _ = self.client.delete_object().bucket(&self.bucket).key(k).send().await;
                }
            }
            if out.is_truncated().unwrap_or(false) {
                continuation = out.next_continuation_token().map(str::to_string);
                if continuation.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        let _ = self.client.delete_bucket().bucket(&self.bucket).send().await;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<BlobEntry>> {
        let mut entries = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let mut req = self.client.list_objects_v2().bucket(&self.bucket);
            if let Some(t) = &continuation {
                req = req.continuation_token(t);
            }
            let out = req
                .send()
                .await
                .map_err(|e| ToolError::internal(format!("s3 list '{}': {e}", self.bucket)))?;
            for obj in out.contents() {
                let Some(key) = obj.key() else { continue };
                let modified = obj.last_modified().map(|d| d.as_secs_f64()).unwrap_or_default();
                entries.push(BlobEntry {
                    key: key.to_string(),
                    size: obj.size().unwrap_or_default().max(0) as u64,
                    modified,
                    shard: None,
                });
            }
            if out.is_truncated().unwrap_or(false) {
                continuation = out.next_continuation_token().map(str::to_string);
                if continuation.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(entries)
    }

    async fn modified(&self, entry: &BlobEntry) -> Result<f64> {
        let key = &entry.key;
        match self.client.head_object().bucket(&self.bucket).key(key).send().await {
            // This is the last read before an irreversible delete (SPEC-0034/DR-005):
            // a missing time must never default to the epoch, which would make a
            // just refreshed object look infinitely old. The sweep skips on error.
            Ok(out) => out.last_modified().map(|d| d.as_secs_f64()).ok_or_else(|| {
                ToolError::not_supported(format!("s3 head '{key}': no last modified time"))
            }),
            // Same raw status classification as `exists`: a HEAD 404 has no body.
            Err(e) if e.raw_response().map(|r| r.status().as_u16()) == Some(404) => {
                Err(ToolError::not_found(format!("blob '{key}' not found")))
            }
            Err(e) => Err(ToolError::internal(format!("s3 head '{key}': {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BlobConfig {
        BlobConfig {
            backend: "minio".into(),
            endpoint: "http://127.0.0.1:9000".into(),
            access_key: "admin".into(),
            secret_key: std::env::var("MCPFS_MINIO_SECRET_KEY").unwrap_or_default(),
            bucket_prefix: "mcpfs-".into(),
            region: "us-east-1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn constructs_with_path_style_and_bucket_name() {
        let s = S3BlobStore::new(&cfg(), "mcpfs-unit".to_string()).unwrap();
        assert_eq!(s.bucket(), "mcpfs-unit");
    }

    // SPEC-0059: the SDK loads the OS root certificates once per process, so each case
    // runs in a child process of this test binary with its own SSL_CERT_FILE.
    const MOD: &str = "storage::blob::s3::tests::";

    fn run_child(
        name: &str,
        envs: &[(&str, &std::ffi::OsStr)],
        limit: std::time::Duration,
    ) -> (Option<i32>, String) {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--include-ignored", "--exact", &format!("{MOD}{name}"), "--nocapture"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let start = std::time::Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if start.elapsed() > limit {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let out = child.wait_with_output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code(), text)
    }

    /// An empty PEM file and an empty directory: the OS store yields no certificate.
    fn empty_store() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let pem = dir.path().join("empty.pem");
        std::fs::write(&pem, b"").unwrap();
        let certs = dir.path().join("certs");
        std::fs::create_dir(&certs).unwrap();
        (dir, pem, certs)
    }

    const LONG: std::time::Duration = std::time::Duration::from_secs(120);

    #[test]
    fn bt_0059_bt_001_http_construct_with_an_empty_trust_store() {
        let (_d, pem, certs) = empty_store();
        let (code, out) = run_child(
            "constructs_with_path_style_and_bucket_name",
            &[("SSL_CERT_FILE", pem.as_os_str()), ("SSL_CERT_DIR", certs.as_os_str())],
            LONG,
        );
        assert_eq!(code, Some(0), "{out}");
        assert!(out.contains("constructs_with_path_style_and_bucket_name ... ok"), "{out}");
    }

    #[cfg(unix)]
    #[test]
    fn bt_0059_bt_010_http_construct_does_not_wait_on_the_os_store() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("store.pem");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let (code, out) = run_child(
            "constructs_with_path_style_and_bucket_name",
            &[("SSL_CERT_FILE", fifo.as_os_str())],
            std::time::Duration::from_secs(10),
        );
        assert_eq!(code, Some(0), "child blocked or failed: {out}");
        assert!(out.contains("constructs_with_path_style_and_bucket_name ... ok"), "{out}");
    }

    #[test]
    fn bt_0059_bt_011_https_with_an_empty_store_builds_then_fails_on_use() {
        let (_d, pem, certs) = empty_store();
        let (code, out) = run_child(
            "child_https_empty_store",
            &[("SSL_CERT_FILE", pem.as_os_str()), ("SSL_CERT_DIR", certs.as_os_str())],
            LONG,
        );
        assert_eq!(code, Some(0), "{out}");
        assert!(out.contains("child_https_empty_store ... ok"), "{out}");
    }

    #[tokio::test]
    #[ignore = "child process case of bt_0059_bt_011"]
    async fn child_https_empty_store() {
        let mut c = cfg();
        c.endpoint = "https://127.0.0.1:1".into();
        let s = S3BlobStore::new(&c, "mcpfs-unit".to_string()).unwrap();
        assert!(s.ensure_bucket().await.is_err());
    }

    fn openssl_pair(
        dir: &std::path::Path,
        name: &str,
    ) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let (key, crt) = (dir.join(format!("{name}-k.pem")), dir.join(format!("{name}-c.pem")));
        let ok = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1"])
            .args(["-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1"])
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&crt)
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?
            .success();
        ok.then_some((key, crt))
    }

    #[test]
    fn bt_0059_bt_012_an_untrusted_certificate_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (Some((key, crt)), Some((_, other))) =
            (openssl_pair(dir.path(), "srv"), openssl_pair(dir.path(), "other"))
        else {
            eprintln!("SKIPPED: openssl not found");
            return;
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut server = std::process::Command::new("openssl")
            .args(["s_server", "-www", "-accept", &port.to_string()])
            .arg("-cert")
            .arg(&crt)
            .arg("-key")
            .arg(&key)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // A refused connection would also read as "refused": prove the server listens first.
        let up = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        });
        assert!(up, "openssl s_server did not start on port {port}");
        let endpoint = format!("https://127.0.0.1:{port}");
        let (code, out) = run_child(
            "child_https_untrusted_cert",
            &[("SSL_CERT_FILE", other.as_os_str()), ("MCPFS_TEST_ENDPOINT", endpoint.as_ref())],
            LONG,
        );
        let _ = server.kill();
        let served = server.wait_with_output().unwrap();
        let served = format!(
            "{}{}",
            String::from_utf8_lossy(&served.stdout),
            String::from_utf8_lossy(&served.stderr)
        );
        assert!(out.contains("child_https_untrusted_cert ... ok"), "{out}");
        assert_eq!(code, Some(0), "{out}");
        assert!(
            !served.lines().any(|l| l.starts_with("GET")),
            "request reached the server: {served}"
        );
    }

    #[tokio::test]
    #[ignore = "child process case of bt_0059_bt_012"]
    async fn child_https_untrusted_cert() {
        let mut c = cfg();
        c.endpoint = std::env::var("MCPFS_TEST_ENDPOINT").unwrap();
        let s = S3BlobStore::new(&c, "mcpfs-unit".to_string()).unwrap();
        assert!(s.ensure_bucket().await.is_err());
    }

    /// Opt-in integration test: needs a live S3/MinIO on :9000 plus the secret in
    /// `MCPFS_MINIO_SECRET_KEY`. Ignored by default, like the C# MinIO tests.
    #[tokio::test]
    #[ignore = "requires a live MinIO/S3 on 127.0.0.1:9000 and MCPFS_MINIO_SECRET_KEY"]
    async fn integration_put_get_range_delete() {
        let bucket = format!("mcpfs-rust-it-{}", uuid::Uuid::new_v4().simple());
        let s = S3BlobStore::new(&cfg(), bucket).unwrap();
        s.ensure_bucket().await.unwrap();

        s.put("deadbeef", b"0123456789").await.unwrap();
        assert!(s.exists("deadbeef").await.unwrap());
        assert_eq!(s.get("deadbeef", 0, None).await.unwrap(), b"0123456789");
        assert_eq!(s.get("deadbeef", 4, Some(3)).await.unwrap(), b"456");

        s.delete("deadbeef").await.unwrap();
        assert!(!s.exists("deadbeef").await.unwrap());
        assert_eq!(
            s.get("deadbeef", 0, None).await.unwrap_err().code,
            crate::errors::code::NOT_FOUND
        );

        s.remove_bucket().await.unwrap();
    }

    /// SPEC-0034/DT-012: `list()` follows pagination (`list_objects_v2` pages at
    /// 1000 keys), returning every key with its correct size.
    #[tokio::test]
    #[ignore = "requires a live MinIO/S3 on 127.0.0.1:9000 and MCPFS_MINIO_SECRET_KEY"]
    async fn integration_list_follows_pagination() {
        let bucket = format!("mcpfs-rust-it-{}", uuid::Uuid::new_v4().simple());
        let s = S3BlobStore::new(&cfg(), bucket).unwrap();
        s.ensure_bucket().await.unwrap();

        const COUNT: usize = 1001;
        for i in 0..COUNT {
            let key = format!("key-{i:04}");
            s.put(&key, key.as_bytes()).await.unwrap();
        }

        let entries = s.list().await.unwrap();
        assert_eq!(entries.len(), COUNT);
        for e in &entries {
            assert_eq!(e.size, e.key.len() as u64);
        }

        s.remove_bucket().await.unwrap();
    }

    /// SPEC-0034/DR-005: `modified` is the object's `last_modified`, and a
    /// deleted object is not found.
    #[tokio::test]
    #[ignore = "requires a live MinIO/S3 on 127.0.0.1:9000 and MCPFS_MINIO_SECRET_KEY"]
    async fn integration_modified_reads_last_modified() {
        let bucket = format!("mcpfs-rust-it-{}", uuid::Uuid::new_v4().simple());
        let s = S3BlobStore::new(&cfg(), bucket).unwrap();
        s.ensure_bucket().await.unwrap();
        s.put("stat-me", b"x").await.unwrap();
        let entry = s.list().await.unwrap().into_iter().find(|e| e.key == "stat-me").unwrap();

        let read = s.modified(&entry).await.unwrap();
        assert!((read - entry.modified).abs() < 1.0, "head {read}, list {}", entry.modified);

        s.delete("stat-me").await.unwrap();
        assert_eq!(s.modified(&entry).await.unwrap_err().code, crate::errors::code::NOT_FOUND);
        s.remove_bucket().await.unwrap();
    }
}
