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
use crate::storage::traits::{BlobBackend, BlobEntry};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

// Test seam (SPEC-0038/DR-003): lets a test delete the blob between the
// moment `put` decides it exists and the refresh, which is otherwise a race
// no test can hit on demand. Thread local so parallel tests never fire each
// other's hook; compiled out of non test builds.
#[cfg(test)]
thread_local! {
    static BEFORE_REFRESH: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn before_refresh(path: &Path) {
    if let Some(hook) = BEFORE_REFRESH.with(std::cell::Cell::get) {
        hook(path);
    }
}

#[cfg(not(test))]
fn before_refresh(_: &Path) {}

#[async_trait]
impl BlobBackend for LocalBlobStore {
    async fn put(&self, sha256: &str, data: &[u8]) -> Result<()> {
        let final_path = self.path_for(sha256);
        let dir = final_path.parent().unwrap_or(&self.root).to_path_buf();
        tokio::fs::create_dir_all(&dir).await?;
        // Content-addressed: an existing blob with this sha has identical bytes.
        // Re-stored bytes still refresh the modified time (SPEC-0034/DR-002),
        // so an age check against it stays trustworthy. Opening directly, not
        // checking then opening: a blob deleted in between (orphan sweep, GC)
        // is simply written again below instead of failing the put (SPEC-0038).
        before_refresh(&final_path);
        let refresh_path = final_path.clone();
        let refreshed =
            tokio::task::spawn_blocking(move || match std::fs::File::open(&refresh_path) {
                Ok(f) => f.set_modified(SystemTime::now()).map(|()| true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e),
            })
            .await
            .map_err(|e| ToolError::internal(format!("join error refreshing mtime: {e}")))??;
        if refreshed {
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
        let mut f = tokio::fs::File::open(&path)
            .await
            .map_err(|_| ToolError::not_found(format!("blob '{sha256}' not found")))?;
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

    async fn list(&self) -> Result<Vec<BlobEntry>> {
        let mut entries = Vec::new();
        let mut shards = match tokio::fs::read_dir(&self.root).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        while let Some(shard_entry) = shards.next_entry().await? {
            if !shard_entry.file_type().await?.is_dir() {
                continue;
            }
            let shard = shard_entry.file_name().to_string_lossy().to_string();
            let mut files = tokio::fs::read_dir(shard_entry.path()).await?;
            while let Some(file_entry) = files.next_entry().await? {
                if !file_entry.file_type().await?.is_file() {
                    continue;
                }
                let metadata = file_entry.metadata().await?;
                let modified = metadata
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                entries.push(BlobEntry {
                    key: file_entry.file_name().to_string_lossy().to_string(),
                    size: metadata.len(),
                    modified,
                    shard: Some(shard.clone()),
                });
            }
        }
        Ok(entries)
    }

    async fn delete_entry(&self, entry: &BlobEntry) -> Result<()> {
        let path = match &entry.shard {
            Some(shard) => self.root.join(shard).join(&entry.key),
            None => self.root.join(&entry.key),
        };
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn modified(&self, entry: &BlobEntry) -> Result<f64> {
        let path = match &entry.shard {
            Some(shard) => self.root.join(shard).join(&entry.key),
            None => self.path_for(&entry.key),
        };
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ToolError::not_found(format!("blob '{}' not found", entry.key)));
            }
            Err(e) => return Err(e.into()),
        };
        // The last read before an irreversible delete (SPEC-0034/DR-005): a time
        // that cannot be expressed is an error the sweep skips, never the epoch.
        metadata.modified()?.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).map_err(
            |e| ToolError::internal(format!("blob '{}' time before epoch: {e}", entry.key)),
        )
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

    /// SPEC-0038/DT-001: a blob deleted after `put` decided it exists is
    /// rewritten instead of failing the write.
    #[tokio::test]
    async fn put_rewrites_a_blob_deleted_mid_call() {
        let (d, s) = store();
        s.put("racy", b"data").await.unwrap();
        BEFORE_REFRESH.with(|h| h.set(Some(|p: &Path| std::fs::remove_file(p).unwrap())));
        let r = s.put("racy", b"data").await;
        BEFORE_REFRESH.with(|h| h.set(None));
        r.unwrap();
        assert_eq!(s.get("racy", 0, None).await.unwrap(), b"data");
        assert!(std::fs::metadata(d.path().join("mcpfs-test").join("ra").join("racy")).is_ok());
    }

    /// SPEC-0038/DT-002: the refresh path keeps its bytes and leaves no temp file.
    #[tokio::test]
    async fn refresh_path_leaves_no_temp_file() {
        let (d, s) = store();
        s.put("kept", b"data").await.unwrap();
        let shard = d.path().join("mcpfs-test").join("ke");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3600);
        std::fs::File::open(shard.join("kept")).unwrap().set_modified(old).unwrap();

        s.put("kept", b"data").await.unwrap();

        let modified = std::fs::metadata(shard.join("kept")).unwrap().modified().unwrap();
        let age = std::time::SystemTime::now().duration_since(modified).unwrap_or_default();
        assert!(age.as_secs_f64() < 2.0, "not refreshed: {age:?}");
        assert_eq!(s.get("kept", 0, None).await.unwrap(), b"data");
        let tmp = std::fs::read_dir(&shard)
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".tmp-"));
        assert!(!tmp, "temp file left in {shard:?}");
    }

    /// SPEC-0038/DT-003: an open error other than not found is returned, never
    /// papered over by a rewrite.
    #[cfg(unix)]
    #[tokio::test]
    async fn put_does_not_rewrite_on_permission_denied() {
        use std::os::unix::fs::PermissionsExt;
        let (d, s) = store();
        s.put("locked", b"data").await.unwrap();
        let path = d.path().join("mcpfs-test").join("lo").join("locked");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root opens a 000 file anyway, so the refusal cannot be observed.
        if std::fs::File::open(&path).is_ok() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            return;
        }
        let r = s.put("locked", b"data").await;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(r.is_err(), "permission denied swallowed");
        assert_eq!(mode, 0o000, "blob was rewritten");
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

    /// The two character shard directory is part of this project's on-disk contract:
    /// an existing deployment's blobs must stay readable after an upgrade, so the
    /// layout is asserted explicitly rather than left to the implementation.
    #[tokio::test]
    async fn on_disk_layout_is_sharded_by_first_two_chars() {
        let (d, s) = store();
        let sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        s.put(sha, b"hello").await.unwrap();

        let expected = d.path().join("mcpfs-test").join("2c").join(sha);
        assert!(expected.exists(), "expected sharded path {}", expected.display());
        assert!(
            !d.path().join("mcpfs-test").join(sha).exists(),
            "a flat layout would orphan the blobs of every existing deployment"
        );
        assert_eq!(s.get(sha, 0, None).await.unwrap(), b"hello");
    }

    /// SPEC-0034/DT-010: `list()` enumerates every file under every shard,
    /// `.tmp-*` files included, and `delete_entry` removes exactly the listed
    /// file although a temp file's name does not map to its own shard.
    #[tokio::test]
    async fn list_enumerates_every_shard_entry_including_temp_files() {
        let (d, s) = store();
        assert_eq!(s.list().await.unwrap(), Vec::new());

        s.put("3bytes", b"abc").await.unwrap();
        s.put("5bytes", b"abcde").await.unwrap();
        let shard_dir = d.path().join("mcpfs-test").join("ab");
        tokio::fs::create_dir_all(&shard_dir).await.unwrap();
        tokio::fs::write(shard_dir.join(".tmp-ab-x"), b"partial").await.unwrap();

        let mut entries = s.list().await.unwrap();
        entries.sort_by_key(|e| e.size);
        assert_eq!(entries.len(), 3);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        for e in &entries {
            assert!((now - e.modified).abs() < 2.0, "{} not within 2s of now", e.key);
        }

        let sizes: Vec<u64> = entries.iter().map(|e| e.size).collect();
        assert_eq!(sizes, vec![3, 5, 7]);

        let tmp = entries.iter().find(|e| e.key == ".tmp-ab-x").expect("temp entry listed");
        assert_eq!(tmp.shard, Some("ab".to_string()));

        s.delete_entry(tmp).await.unwrap();
        assert!(!shard_dir.join(".tmp-ab-x").exists());
    }

    /// SPEC-0034/DT-013: `put` of bytes already present refreshes the modified
    /// time and leaves the bytes unchanged.
    #[tokio::test]
    async fn put_of_existing_bytes_refreshes_the_modified_time() {
        let (d, s) = store();
        s.put("existing", b"data").await.unwrap();
        let path = d.path().join("mcpfs-test").join("ex").join("existing");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3600);
        let f = std::fs::File::open(&path).unwrap();
        f.set_modified(old).unwrap();

        s.put("existing", b"data").await.unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let modified = metadata.modified().unwrap();
        let now = std::time::SystemTime::now();
        let age = now.duration_since(modified).unwrap_or_default();
        assert!(age.as_secs_f64() < 2.0, "modified time not refreshed: {age:?} old");
        assert_eq!(s.get("existing", 0, None).await.unwrap(), b"data");
    }

    /// SPEC-0034/DR-005: `modified` reads the file's own time, fresh, and
    /// reports a deleted entry as not found.
    #[tokio::test]
    async fn modified_reads_the_file_time_and_not_found_once_deleted() {
        let (d, s) = store();
        s.put("backdated", b"data").await.unwrap();
        let path = d.path().join("mcpfs-test").join("ba").join("backdated");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3600);
        std::fs::File::open(&path).unwrap().set_modified(old).unwrap();
        let entry = s.list().await.unwrap().into_iter().find(|e| e.key == "backdated").unwrap();

        let read = s.modified(&entry).await.unwrap();
        let expected = old.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        assert!((read - expected).abs() < 1.0, "read {read}, expected {expected}");

        s.delete_entry(&entry).await.unwrap();
        let e = s.modified(&entry).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
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
