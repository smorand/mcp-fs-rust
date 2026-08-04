//! Local filesystem blob backend: content-addressed files under a per-volume dir.
//! Port of the C# `Storage/LocalBlobStore.cs`.
//!
//! Layout: `{dir}/{bucket}/{sha[..2]}/{sha}`. The two character shard directory is
//! part of the on-disk contract: a volume written by the C# implementation must be
//! readable here and vice versa, so it is NOT an implementation detail.
//!
//! Known latent issue inherited from the C#: a git object key is `git:{sha}`, and a
//! colon is illegal in an NTFS filename, so the git subsystem on a Windows host needs
//! the S3 backend (or a key mapping landed in both implementations at once).

use crate::errors::{Result, ToolError};
use crate::storage::traits::BlobBackend;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct LocalBlobStore {
    root: PathBuf,
}

impl LocalBlobStore {
    /// `dir` is the configured blob dir; `bucket` scopes it to one volume.
    pub fn new(dir: impl AsRef<Path>, bucket: &str) -> Self {
        Self { root: dir.as_ref().join(bucket) }
    }

    /// `{root}/{first two chars}/{key}`. Keys shorter than two characters are kept
    /// flat rather than panicking; only fixed length hashes reach this in practice.
    fn path_for(&self, sha256: &str) -> PathBuf {
        if sha256.len() < 2 {
            return self.root.join(sha256);
        }
        self.root.join(&sha256[..2]).join(sha256)
    }
}

#[async_trait]
impl BlobBackend for LocalBlobStore {
    async fn put(&self, sha256: &str, data: &[u8]) -> Result<()> {
        let final_path = self.path_for(sha256);
        let dir = final_path.parent().unwrap_or(&self.root).to_path_buf();
        tokio::fs::create_dir_all(&dir).await?;
        // Content-addressed: an existing blob with this sha has identical bytes.
        if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
            return Ok(());
        }
        // Write to a temp name then rename, so a reader never sees a partial blob.
        let tmp = dir.join(format!(".tmp-{}-{}", sha256, uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, data).await?;
        match tokio::fs::rename(&tmp, &final_path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                // A concurrent writer may have won the race; that is fine.
                if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }

    async fn get(&self, sha256: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let path = self.path_for(sha256);
        let mut f = tokio::fs::File::open(&path).await.map_err(|_| {
            ToolError::not_found(format!("blob '{sha256}' not found"))
        })?;
        if offset > 0 {
            f.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        match length {
            None => {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).await?;
                Ok(buf)
            }
            Some(len) => {
                let mut buf = vec![0u8; len as usize];
                let n = f.read(&mut buf).await?;
                buf.truncate(n);
                Ok(buf)
            }
        }
    }

    async fn exists(&self, sha256: &str) -> Result<bool> {
        Ok(tokio::fs::try_exists(self.path_for(sha256)).await.unwrap_or(false))
    }

    async fn delete(&self, sha256: &str) -> Result<()> {
        match tokio::fs::remove_file(self.path_for(sha256)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn ensure_bucket(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.root).await?;
        Ok(())
    }

    async fn remove_bucket(&self) -> Result<()> {
        match tokio::fs::remove_dir_all(&self.root).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, LocalBlobStore) {
        let d = tempfile::tempdir().unwrap();
        let s = LocalBlobStore::new(d.path(), "mcpfs-test");
        (d, s)
    }

    #[tokio::test]
    async fn put_get_roundtrip() {
        let (_d, s) = store();
        s.ensure_bucket().await.unwrap();
        s.put("abc", b"hello world").await.unwrap();
        assert_eq!(s.get("abc", 0, None).await.unwrap(), b"hello world");
    }

    #[tokio::test]
    async fn range_reads() {
        let (_d, s) = store();
        s.put("r", b"0123456789").await.unwrap();
        assert_eq!(s.get("r", 0, Some(4)).await.unwrap(), b"0123");
        assert_eq!(s.get("r", 4, Some(3)).await.unwrap(), b"456");
        assert_eq!(s.get("r", 8, None).await.unwrap(), b"89");
        // a length past the end truncates instead of failing
        assert_eq!(s.get("r", 8, Some(100)).await.unwrap(), b"89");
    }

    #[tokio::test]
    async fn exists_and_delete() {
        let (_d, s) = store();
        assert!(!s.exists("x").await.unwrap());
        s.put("x", b"1").await.unwrap();
        assert!(s.exists("x").await.unwrap());
        s.delete("x").await.unwrap();
        assert!(!s.exists("x").await.unwrap());
        // deleting a missing blob is a no-op
        s.delete("x").await.unwrap();
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let (_d, s) = store();
        let e = s.get("nope", 0, None).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_is_idempotent() {
        let (_d, s) = store();
        s.put("same", b"data").await.unwrap();
        s.put("same", b"data").await.unwrap();
        assert_eq!(s.get("same", 0, None).await.unwrap(), b"data");
    }

    #[tokio::test]
    async fn remove_bucket_drops_everything() {
        let (_d, s) = store();
        s.put("a", b"1").await.unwrap();
        s.remove_bucket().await.unwrap();
        assert!(!s.exists("a").await.unwrap());
        // idempotent
        s.remove_bucket().await.unwrap();
    }

    #[tokio::test]
    async fn no_temp_files_left_behind() {
        let (d, s) = store();
        let sha = "abcdef0123";
        s.put(sha, b"1").await.unwrap();
        let shard = d.path().join("mcpfs-test").join("ab");
        let mut entries = tokio::fs::read_dir(&shard).await.unwrap();
        let mut names = Vec::new();
        while let Some(e) = entries.next_entry().await.unwrap() {
            names.push(e.file_name().to_string_lossy().to_string());
        }
        assert_eq!(names, vec![sha], "only the final blob should remain");
    }

    /// The two character shard directory is part of the on-disk contract shared with
    /// the C# implementation. A volume written by one must be readable by the other,
    /// so this layout is asserted explicitly.
    #[tokio::test]
    async fn on_disk_layout_is_sharded_by_first_two_chars() {
        let (d, s) = store();
        let sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        s.put(sha, b"hello").await.unwrap();

        let expected = d.path().join("mcpfs-test").join("2c").join(sha);
        assert!(expected.exists(), "expected sharded path {}", expected.display());
        assert!(
            !d.path().join("mcpfs-test").join(sha).exists(),
            "a flat layout would break compatibility with existing C# volumes"
        );
        assert_eq!(s.get(sha, 0, None).await.unwrap(), b"hello");
    }

    /// Blobs whose shard prefix collides still coexist.
    #[tokio::test]
    async fn blobs_sharing_a_shard_coexist() {
        let (_d, s) = store();
        s.put("ab1111", b"one").await.unwrap();
        s.put("ab2222", b"two").await.unwrap();
        assert_eq!(s.get("ab1111", 0, None).await.unwrap(), b"one");
        assert_eq!(s.get("ab2222", 0, None).await.unwrap(), b"two");
        s.delete("ab1111").await.unwrap();
        assert!(!s.exists("ab1111").await.unwrap());
        assert!(s.exists("ab2222").await.unwrap(), "sibling in the same shard survives");
    }
}
