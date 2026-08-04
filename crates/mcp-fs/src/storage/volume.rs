//! One volume: metadata tree + content-addressed bytes, bound together.
//! Port of the C# `Storage/VolumeClient.cs`. This is the object every fs
//! operation works against.

use crate::errors::{Result, ToolError};
use crate::storage::traits::{BlobBackend, MODE_FILE, MetaBackend, NodeRow};
use crate::util::PosixPath;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub struct VolumeClient {
    pub project_id: String,
    pub meta: Arc<dyn MetaBackend>,
    pub blob: Arc<dyn BlobBackend>,
}

impl VolumeClient {
    pub fn new(
        project_id: impl Into<String>,
        meta: Arc<dyn MetaBackend>,
        blob: Arc<dyn BlobBackend>,
    ) -> Self {
        Self { project_id: project_id.into(), meta, blob }
    }

    pub fn sha256_hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        hex::encode(h.finalize())
    }

    // ── reads ────────────────────────────────────────────────────────────────

    pub async fn stat(&self, path: &str) -> Result<NodeRow> {
        self.meta
            .get(path)
            .await?
            .ok_or_else(|| ToolError::not_found(format!("'{path}' not found")))
    }

    pub async fn exists(&self, path: &str) -> Result<bool> {
        Ok(self.meta.get(path).await?.is_some())
    }

    pub async fn is_file(&self, path: &str) -> Result<bool> {
        Ok(self.meta.get(path).await?.is_some_and(|n| n.is_file()))
    }

    pub async fn is_dir(&self, path: &str) -> Result<bool> {
        Ok(self.meta.get(path).await?.is_some_and(|n| n.is_dir()))
    }

    /// Full byte content of a file. An empty file has no blob.
    pub async fn read_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let n = self.stat(path).await?;
        if n.is_dir() {
            return Err(ToolError::invalid_argument(format!("'{path}' is a directory")));
        }
        match n.sha256 {
            None => Ok(Vec::new()),
            Some(sha) => self.blob.get(&sha, 0, None).await,
        }
    }

    /// Byte range of a file.
    pub async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        let n = self.stat(path).await?;
        if n.is_dir() {
            return Err(ToolError::invalid_argument(format!("'{path}' is a directory")));
        }
        match n.sha256 {
            None => Ok(Vec::new()),
            Some(sha) => self.blob.get(&sha, offset, Some(length)).await,
        }
    }

    pub async fn read_text(&self, path: &str) -> Result<String> {
        let bytes = self.read_bytes(path).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub async fn list_dir(&self, path: &str) -> Result<Vec<NodeRow>> {
        let n = self.stat(path).await?;
        if !n.is_dir() {
            return Err(ToolError::invalid_argument(format!("'{path}' is not a directory")));
        }
        self.meta.list_children(path).await
    }

    /// Walk a subtree, yielding `(dir, subdirs, files)` per directory, like the
    /// C# `WalkAsync` (which mirrors Python's `os.walk`).
    pub async fn walk(&self, top: &str) -> Result<Vec<(String, Vec<String>, Vec<String>)>> {
        let nodes = self.meta.subtree(top).await?;
        let mut dirs: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> =
            std::collections::BTreeMap::new();
        // Every directory in the subtree gets an entry, so empty dirs still show up.
        for n in nodes.iter().filter(|n| n.is_dir()) {
            dirs.entry(n.path.clone()).or_default();
        }
        for n in nodes.iter() {
            if n.path == top {
                continue;
            }
            let Some(parent) = n.parent.clone() else { continue };
            let e = dirs.entry(parent).or_default();
            if n.is_dir() {
                e.0.push(n.name.clone());
            } else {
                e.1.push(n.name.clone());
            }
        }
        Ok(dirs.into_iter().map(|(d, (sd, f))| (d, sd, f)).collect())
    }

    // ── writes ───────────────────────────────────────────────────────────────

    /// Write bytes atomically: blob first (content-addressed), then the node.
    /// GCs a blob that lost its last reference.
    pub async fn write_bytes_atomic(&self, path: &str, data: &[u8]) -> Result<()> {
        let (sha, size) = if data.is_empty() {
            (None, 0i64)
        } else {
            let sha = Self::sha256_hex(data);
            self.blob.put(&sha, data).await?;
            (Some(sha), data.len() as i64)
        };
        let gc = self.meta.put_file(path, sha.as_deref(), size, MODE_FILE).await?;
        if let Some(dead) = gc {
            self.blob.delete(&dead).await?;
        }
        Ok(())
    }

    pub async fn write_text_atomic(&self, path: &str, text: &str) -> Result<()> {
        self.write_bytes_atomic(path, text.as_bytes()).await
    }

    pub async fn create_empty(&self, path: &str) -> Result<()> {
        self.write_bytes_atomic(path, &[]).await
    }

    pub async fn makedirs(&self, path: &str, exist_ok: bool) -> Result<()> {
        self.meta.mkdirs(path, exist_ok).await
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.meta.mkdir(path).await
    }

    /// Delete a file, GCing its blob when the last reference goes away.
    pub async fn delete_file(&self, path: &str) -> Result<()> {
        if let Some(dead) = self.meta.delete_file(path).await? {
            self.blob.delete(&dead).await?;
        }
        Ok(())
    }

    /// Delete a whole subtree, GCing every orphaned blob.
    pub async fn delete_tree(&self, path: &str) -> Result<()> {
        for dead in self.meta.remove_subtree(path).await? {
            self.blob.delete(&dead).await?;
        }
        Ok(())
    }

    pub async fn rename(&self, src: &str, dst: &str) -> Result<()> {
        self.meta.rename(src, dst).await
    }

    /// Copy one file: metadata only, the blob is shared (refcount incremented).
    pub async fn copy_file(&self, src: &str, dst: &str) -> Result<()> {
        let n = self.stat(src).await?;
        if n.is_dir() {
            return Err(ToolError::invalid_argument(format!("'{src}' is a directory")));
        }
        let gc = self
            .meta
            .put_file(dst, n.sha256.as_deref(), n.size, n.mode)
            .await?;
        if let Some(dead) = gc {
            self.blob.delete(&dead).await?;
        }
        Ok(())
    }

    /// Recursively copy a tree, sharing blobs.
    pub async fn copy_tree(&self, src: &str, dst: &str) -> Result<()> {
        let nodes = self.meta.subtree(src).await?;
        for n in nodes {
            let rel = &n.path[src.len()..];
            let target = if rel.is_empty() { dst.to_string() } else { format!("{dst}{rel}") };
            if n.is_dir() {
                self.meta.mkdirs(&target, true).await?;
            } else {
                if let Some(parent) = PosixPath::parent_of(&target)
                    && parent != "/" {
                        self.meta.mkdirs(&parent, true).await?;
                    }
                if let Some(dead) =
                    self.meta.put_file(&target, n.sha256.as_deref(), n.size, n.mode).await?
                {
                    self.blob.delete(&dead).await?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::blob::local::LocalBlobStore;
    use crate::storage::meta::SqliteMetaStore;

    fn vol() -> (tempfile::TempDir, VolumeClient) {
        let d = tempfile::tempdir().unwrap();
        let meta = Arc::new(SqliteMetaStore::in_memory().unwrap());
        let blob = Arc::new(LocalBlobStore::new(d.path(), "mcpfs-test"));
        (d, VolumeClient::new("test", meta, blob))
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let (_d, v) = vol();
        v.write_text_atomic("/a.txt", "hello").await.unwrap();
        assert_eq!(v.read_text("/a.txt").await.unwrap(), "hello");
        let n = v.stat("/a.txt").await.unwrap();
        assert_eq!(n.size, 5);
        assert_eq!(n.sha256, Some(VolumeClient::sha256_hex(b"hello")));
    }

    #[tokio::test]
    async fn sha256_is_standard() {
        // sha256("hello") well-known digest
        assert_eq!(
            VolumeClient::sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn empty_file_stores_no_blob() {
        let (_d, v) = vol();
        v.create_empty("/e.txt").await.unwrap();
        let n = v.stat("/e.txt").await.unwrap();
        assert_eq!(n.sha256, None);
        assert_eq!(n.size, 0);
        assert_eq!(v.read_bytes("/e.txt").await.unwrap(), Vec::<u8>::new());
    }

    #[tokio::test]
    async fn identical_content_shares_one_blob() {
        let (_d, v) = vol();
        v.write_text_atomic("/a.txt", "same").await.unwrap();
        v.write_text_atomic("/b.txt", "same").await.unwrap();
        let sha = VolumeClient::sha256_hex(b"same");

        // deleting one keeps the blob alive for the other
        v.delete_file("/a.txt").await.unwrap();
        assert!(v.blob.exists(&sha).await.unwrap(), "blob still referenced");
        assert_eq!(v.read_text("/b.txt").await.unwrap(), "same");

        // deleting the last reference GCs it
        v.delete_file("/b.txt").await.unwrap();
        assert!(!v.blob.exists(&sha).await.unwrap(), "blob GC'd at refcount 0");
    }

    #[tokio::test]
    async fn overwrite_gcs_the_replaced_blob() {
        let (_d, v) = vol();
        v.write_text_atomic("/a.txt", "old").await.unwrap();
        let old_sha = VolumeClient::sha256_hex(b"old");
        v.write_text_atomic("/a.txt", "new").await.unwrap();
        assert!(!v.blob.exists(&old_sha).await.unwrap(), "old blob GC'd");
        assert_eq!(v.read_text("/a.txt").await.unwrap(), "new");
    }

    #[tokio::test]
    async fn copy_is_metadata_only() {
        let (_d, v) = vol();
        v.write_text_atomic("/a.txt", "payload").await.unwrap();
        v.copy_file("/a.txt", "/b.txt").await.unwrap();
        let a = v.stat("/a.txt").await.unwrap();
        let b = v.stat("/b.txt").await.unwrap();
        assert_eq!(a.sha256, b.sha256, "same blob is shared");
        assert_eq!(v.read_text("/b.txt").await.unwrap(), "payload");

        // deleting the source keeps the copy readable
        v.delete_file("/a.txt").await.unwrap();
        assert_eq!(v.read_text("/b.txt").await.unwrap(), "payload");
    }

    #[tokio::test]
    async fn read_range_slices() {
        let (_d, v) = vol();
        v.write_text_atomic("/r.txt", "0123456789").await.unwrap();
        assert_eq!(v.read_range("/r.txt", 0, 4).await.unwrap(), b"0123");
        assert_eq!(v.read_range("/r.txt", 4, 3).await.unwrap(), b"456");
    }

    #[tokio::test]
    async fn delete_tree_removes_everything_and_gcs() {
        let (_d, v) = vol();
        v.write_text_atomic("/d/a.txt", "1").await.unwrap();
        v.write_text_atomic("/d/sub/b.txt", "2").await.unwrap();
        let sha1 = VolumeClient::sha256_hex(b"1");
        v.delete_tree("/d").await.unwrap();
        assert!(!v.exists("/d").await.unwrap());
        assert!(!v.exists("/d/sub/b.txt").await.unwrap());
        assert!(!v.blob.exists(&sha1).await.unwrap());
    }

    #[tokio::test]
    async fn copy_tree_recreates_structure() {
        let (_d, v) = vol();
        v.write_text_atomic("/s/a.txt", "1").await.unwrap();
        v.write_text_atomic("/s/sub/b.txt", "2").await.unwrap();
        v.copy_tree("/s", "/t").await.unwrap();
        assert_eq!(v.read_text("/t/a.txt").await.unwrap(), "1");
        assert_eq!(v.read_text("/t/sub/b.txt").await.unwrap(), "2");
        assert!(v.is_dir("/t/sub").await.unwrap());
    }

    #[tokio::test]
    async fn walk_reports_dirs_and_files() {
        let (_d, v) = vol();
        v.write_text_atomic("/w/a.txt", "1").await.unwrap();
        v.write_text_atomic("/w/sub/b.txt", "2").await.unwrap();
        let walked = v.walk("/w").await.unwrap();
        let root = walked.iter().find(|(d, _, _)| d == "/w").unwrap();
        assert_eq!(root.1, vec!["sub"]);
        assert_eq!(root.2, vec!["a.txt"]);
        let sub = walked.iter().find(|(d, _, _)| d == "/w/sub").unwrap();
        assert_eq!(sub.2, vec!["b.txt"]);
    }

    #[tokio::test]
    async fn reading_a_directory_is_an_error() {
        let (_d, v) = vol();
        v.mkdir("/d").await.unwrap();
        let e = v.read_bytes("/d").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn stat_missing_is_not_found() {
        let (_d, v) = vol();
        let e = v.stat("/nope").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
    }

    #[tokio::test]
    async fn rename_moves_file() {
        let (_d, v) = vol();
        v.write_text_atomic("/a.txt", "x").await.unwrap();
        v.rename("/a.txt", "/b.txt").await.unwrap();
        assert!(!v.exists("/a.txt").await.unwrap());
        assert_eq!(v.read_text("/b.txt").await.unwrap(), "x");
    }
}
