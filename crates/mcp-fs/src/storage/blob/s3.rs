//! S3 / MinIO blob backend. Port of the C# `Storage/MinioBlobStore.cs`.
//!
//! One bucket per volume (`{bucket_prefix}{project_id}`), object key = sha256, so a
//! bucket written by either implementation is readable by the other. Path-style
//! addressing is forced, which is what MinIO needs.

use crate::config::BlobConfig;
use crate::errors::{Result, ToolError};
use crate::storage::traits::BlobBackend;
use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;

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
            let s = e.to_string();
            if s.contains("NoSuchKey") || s.contains("NotFound") || s.contains("404") {
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
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(sha256)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let s = e.to_string();
                if s.contains("NotFound") || s.contains("NoSuchKey") || s.contains("404") {
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
        if self
            .client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
        {
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
                    Err(ToolError::internal(format!(
                        "s3 create bucket '{}': {e}",
                        self.bucket
                    )))
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
                    let _ = self
                        .client
                        .delete_object()
                        .bucket(&self.bucket)
                        .key(k)
                        .send()
                        .await;
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
}
