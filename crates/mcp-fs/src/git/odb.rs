//! Blob backed git object database.
//!
//! Git objects are stored in the volume's [`BlobBackend`] under the key
//! `git:{sha}` holding the canonical git object bytes `{type} {len}\0{payload}`,
//! with a `(hash, type, size)` row in [`SqliteGitDb`] used for `ForEach` and
//! short sha (prefix) lookups. Byte for byte the same layout as the C#
//! `Git/BlobBackedOdbBackend.cs`, so a volume written by either implementation is
//! readable by the other.
//!
//! # Deviation from the C#: no custom libgit2 ODB backend
//!
//! The C# registers a `LibGit2Sharp.OdbBackend` subclass on the repository
//! (`repo.ObjectDatabase.AddBackend(backend, priority: 5)`). `git2` (the libgit2
//! Rust bindings, same native library) does NOT expose that: `git2::Odb` only
//! offers `add_disk_alternate` and `add_new_mempack_backend`, and writing a real
//! `git_odb_backend` means hand rolling C function pointers plus manual
//! `git_odb_backend_malloc` lifetime management through `git2-sys`, which cannot
//! be done from safe Rust and would be untestable unsafe glue.
//!
//! So the plumbing differs while the stored bytes stay identical:
//!
//! * [`BlobObjectDb`] owns read/write/exists/prefix over the blob store plus the
//!   SQLite index. It is the source of truth and is what the `git.*` tools use.
//! * [`BlobObjectDb::export_to_repo`] hydrates the on disk bare repo's ODB from
//!   the blob store before libgit2 needs objects (pack building for a fetch, log,
//!   diff, blame).
//! * [`BlobObjectDb::import_from_repo`] copies anything libgit2 wrote on disk
//!   (an indexed incoming packfile on push, a commit created by a tool) back into
//!   the blob store and the index.
//!
//! Consequence: `state/git-repos/{project}/objects/` becomes a rebuildable cache
//! instead of staying empty. Deleting it loses nothing, the blob store still has
//! every object. That is the price of not writing unsafe FFI.

use crate::errors::{Result, ToolError, code};
use crate::git::db::{GitObjectRow, SqliteGitDb};
use crate::storage::BlobBackend;
use git2::{ObjectType, Oid};
use std::sync::Arc;

/// Blob store key for a git object. Git objects and file blobs share the bucket;
/// the `git:` prefix keeps them from ever colliding (a sha256 is plain hex).
pub fn blob_key(sha: &str) -> String {
    format!("git:{sha}")
}

/// Canonical git type name. Only the four real object types are storable.
pub fn object_type_name(kind: ObjectType) -> Result<&'static str> {
    match kind {
        ObjectType::Blob => Ok("blob"),
        ObjectType::Tree => Ok("tree"),
        ObjectType::Commit => Ok("commit"),
        ObjectType::Tag => Ok("tag"),
        other => Err(ToolError::invalid_argument(format!(
            "unknown object type {other:?}"
        ))),
    }
}

/// Inverse of [`object_type_name`].
pub fn parse_object_type(s: &str) -> Result<ObjectType> {
    match s {
        "blob" => Ok(ObjectType::Blob),
        "tree" => Ok(ObjectType::Tree),
        "commit" => Ok(ObjectType::Commit),
        "tag" => Ok(ObjectType::Tag),
        other => Err(ToolError::invalid_argument(format!(
            "unknown object type string '{other}'"
        ))),
    }
}

/// `{type} {len}\0{payload}`, the format libgit2 hashes to get the object id.
pub fn serialize(kind: ObjectType, payload: &[u8]) -> Result<Vec<u8>> {
    let header = format!("{} {}\0", object_type_name(kind)?, payload.len());
    let mut out = Vec::with_capacity(header.len() + payload.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Split a stored object back into its type and payload.
pub fn deserialize(raw: &[u8]) -> Result<(ObjectType, Vec<u8>)> {
    let nul = raw
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| ToolError::internal("corrupt git object: missing NUL"))?;
    let header = std::str::from_utf8(&raw[..nul])
        .map_err(|_| ToolError::internal("corrupt git object: non ascii header"))?;
    // The header is "{type} {len}"; the C# splits on the first space and ignores
    // the length, trusting the payload slice. Do the same.
    let type_name = header.split(' ').next().unwrap_or("");
    let kind = parse_object_type(type_name)?;
    Ok((kind, raw[nul + 1..].to_vec()))
}

/// The git object id of a payload, without storing anything.
pub fn object_id(kind: ObjectType, payload: &[u8]) -> Result<String> {
    Oid::hash_object(kind, payload)
        .map(|o| o.to_string())
        .map_err(|e| ToolError::internal(format!("git hash-object failed: {e}")))
}

/// Read/write access to one project's git objects.
pub struct BlobObjectDb {
    blobs: Arc<dyn BlobBackend>,
    index: Arc<SqliteGitDb>,
}

impl BlobObjectDb {
    pub fn new(blobs: Arc<dyn BlobBackend>, index: Arc<SqliteGitDb>) -> Self {
        Self { blobs, index }
    }

    pub fn index(&self) -> &Arc<SqliteGitDb> {
        &self.index
    }

    pub fn blobs(&self) -> &Arc<dyn BlobBackend> {
        &self.blobs
    }

    /// Store an object, returning its sha. Writes the blob first, then the index
    /// row: a crash in between leaves an unindexed but valid blob, never an index
    /// row pointing at nothing.
    pub async fn write(&self, kind: ObjectType, payload: &[u8]) -> Result<String> {
        let sha = object_id(kind, payload)?;
        let raw = serialize(kind, payload)?;
        self.blobs.put(&blob_key(&sha), &raw).await?;
        self.index
            .record_object(&sha, object_type_name(kind)?, payload.len() as i64)
            .await?;
        Ok(sha)
    }

    /// Store an object whose sha is already known (an object copied in from
    /// libgit2), skipping the rehash.
    pub async fn write_with_sha(
        &self,
        sha: &str,
        kind: ObjectType,
        payload: &[u8],
    ) -> Result<()> {
        let raw = serialize(kind, payload)?;
        self.blobs.put(&blob_key(sha), &raw).await?;
        self.index
            .record_object(sha, object_type_name(kind)?, payload.len() as i64)
            .await
    }

    /// `ERR_NOT_FOUND` when the object is absent.
    pub async fn read(&self, sha: &str) -> Result<(ObjectType, Vec<u8>)> {
        let key = blob_key(sha);
        if !self.blobs.exists(&key).await? {
            return Err(ToolError::not_found(format!("git object '{sha}' not found")));
        }
        let raw = self.blobs.get(&key, 0, None).await?;
        deserialize(&raw)
    }

    /// Type and payload length only, the C# `ReadHeader`.
    pub async fn read_header(&self, sha: &str) -> Result<(ObjectType, usize)> {
        let (kind, payload) = self.read(sha).await?;
        Ok((kind, payload.len()))
    }

    pub async fn exists(&self, sha: &str) -> Result<bool> {
        self.blobs.exists(&blob_key(sha)).await
    }

    /// Resolve a short sha. `ERR_NOT_FOUND` when nothing matches,
    /// `ERR_AMBIGUOUS_MATCH` when several do (the C# `GIT_EAMBIGUOUS`).
    pub async fn resolve_prefix(&self, prefix: &str) -> Result<String> {
        let matches = self.index.find_objects_by_prefix(prefix).await?;
        match matches.len() {
            0 => Err(ToolError::not_found(format!(
                "no git object matches '{prefix}'"
            ))),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => Err(ToolError::ambiguous_match(format!(
                "'{prefix}' matches {n} git objects"
            ))),
        }
    }

    /// [`Self::resolve_prefix`] then [`Self::read`], the C# `ReadPrefix`.
    pub async fn read_prefix(&self, prefix: &str) -> Result<(String, ObjectType, Vec<u8>)> {
        let sha = self.resolve_prefix(prefix).await?;
        let (kind, payload) = self.read(&sha).await?;
        Ok((sha, kind, payload))
    }

    /// Delete an object from the blob store. Not part of the C# surface (a git
    /// ODB never deletes), kept for volume teardown and tests.
    pub async fn delete(&self, sha: &str) -> Result<()> {
        self.blobs.delete(&blob_key(sha)).await
    }

    pub async fn list(&self) -> Result<Vec<GitObjectRow>> {
        self.index.list_objects().await
    }

    /// Copy every object libgit2 has on disk into the blob store and the index.
    /// Returns how many objects were newly stored. Existing objects are skipped:
    /// git content addressing guarantees identical bytes for the same sha.
    pub async fn import_from_repo(&self, repo: &git2::Repository) -> Result<usize> {
        // Collect first: the git2 Odb borrow cannot be held across an await.
        let pending: Vec<(String, ObjectType, Vec<u8>)> = {
            let odb = repo
                .odb()
                .map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
            let mut oids = Vec::new();
            odb.foreach(|oid| {
                oids.push(*oid);
                true
            })
            .map_err(|e| ToolError::internal(format!("odb foreach failed: {e}")))?;

            let mut out = Vec::with_capacity(oids.len());
            for oid in oids {
                let obj = match odb.read(oid) {
                    Ok(o) => o,
                    // A pack may reference an object we cannot read; skipping it
                    // matches the C# backend swallowing errors as GIT_ENOTFOUND.
                    Err(_) => continue,
                };
                if object_type_name(obj.kind()).is_err() {
                    continue;
                }
                out.push((oid.to_string(), obj.kind(), obj.data().to_vec()));
            }
            out
        };

        let mut stored = 0usize;
        for (sha, kind, payload) in pending {
            if self.exists(&sha).await? {
                // Still make sure the index knows about it, so prefix lookups work
                // after an index loss.
                if !self.index.object_exists(&sha).await? {
                    self.index
                        .record_object(&sha, object_type_name(kind)?, payload.len() as i64)
                        .await?;
                }
                continue;
            }
            self.write_with_sha(&sha, kind, &payload).await?;
            stored += 1;
        }
        Ok(stored)
    }

    /// Hydrate libgit2's on disk ODB from the blob store, so pack building, log,
    /// diff and blame can run. Returns how many objects were written to disk.
    pub async fn export_to_repo(&self, repo: &git2::Repository) -> Result<usize> {
        let rows = self.index.list_objects().await?;
        let mut payloads: Vec<(ObjectType, Vec<u8>)> = Vec::new();
        for row in rows {
            let oid = match Oid::from_str(&row.hash) {
                Ok(o) => o,
                Err(_) => continue, // a malformed index row must not break a fetch
            };
            {
                let odb = repo
                    .odb()
                    .map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
                if odb.exists(oid) {
                    continue;
                }
            }
            match self.read(&row.hash).await {
                Ok((kind, payload)) => payloads.push((kind, payload)),
                Err(e) if e.code == code::NOT_FOUND => continue,
                Err(e) => return Err(e),
            }
        }

        let odb = repo
            .odb()
            .map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
        let mut written = 0usize;
        for (kind, payload) in payloads {
            odb.write(kind, &payload)
                .map_err(|e| ToolError::internal(format!("odb write failed: {e}")))?;
            written += 1;
        }
        Ok(written)
    }
}

/// Write a commit straight into the blob backed store, for tests and for tools
/// that need a starting point. Returns `(commit sha, tree sha, blob sha)`.
pub async fn seed_commit(
    objects: &BlobObjectDb,
    file_name: &str,
    content: &[u8],
    message: &str,
) -> Result<(String, String, String)> {
    let blob_sha = objects.write(ObjectType::Blob, content).await?;
    // tree entry: "100644 {name}\0{20 raw sha bytes}"
    let mut tree = Vec::new();
    tree.extend_from_slice(format!("100644 {file_name}\0").as_bytes());
    tree.extend_from_slice(&hex::decode(&blob_sha).map_err(|e| {
        ToolError::internal(format!("blob sha is not hex: {e}"))
    })?);
    let tree_sha = objects.write(ObjectType::Tree, &tree).await?;

    let body = format!(
        "tree {tree_sha}\nauthor mcp-fs <mcp-fs@example.test> 1700000000 +0000\n\
         committer mcp-fs <mcp-fs@example.test> 1700000000 +0000\n\n{message}\n"
    );
    let commit_sha = objects.write(ObjectType::Commit, body.as_bytes()).await?;
    Ok((commit_sha, tree_sha, blob_sha))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::blob::local::LocalBlobStore;

    fn odb() -> (tempfile::TempDir, BlobObjectDb) {
        let d = tempfile::tempdir().unwrap();
        let blobs: Arc<dyn BlobBackend> =
            Arc::new(LocalBlobStore::new(d.path(), "mcpfs-git-test"));
        let index = Arc::new(SqliteGitDb::open_in_memory().unwrap());
        (d, BlobObjectDb::new(blobs, index))
    }

    #[test]
    fn blob_key_is_git_prefixed() {
        assert_eq!(blob_key("abc123"), "git:abc123");
        assert!(blob_key("x").starts_with("git:"));
        // a sha256 file blob key never starts with "git:", so no collision
        assert_ne!(blob_key("abc"), "abc");
    }

    #[test]
    fn type_names_round_trip() {
        for (name, kind) in [
            ("blob", ObjectType::Blob),
            ("tree", ObjectType::Tree),
            ("commit", ObjectType::Commit),
            ("tag", ObjectType::Tag),
        ] {
            assert_eq!(object_type_name(kind).unwrap(), name);
            assert_eq!(parse_object_type(name).unwrap(), kind);
        }
        assert!(parse_object_type("chicken").is_err());
        assert!(object_type_name(ObjectType::Any).is_err());
    }

    #[test]
    fn serialize_produces_the_git_header() {
        let raw = serialize(ObjectType::Blob, b"hello").unwrap();
        assert_eq!(raw, b"blob 5\0hello");
        let empty = serialize(ObjectType::Tree, b"").unwrap();
        assert_eq!(empty, b"tree 0\0");
    }

    #[test]
    fn serialize_deserialize_round_trip_for_each_type() {
        for kind in [
            ObjectType::Blob,
            ObjectType::Tree,
            ObjectType::Commit,
            ObjectType::Tag,
        ] {
            let payload = b"payload\0with\x01binary\xffbytes".to_vec();
            let raw = serialize(kind, &payload).unwrap();
            let (k2, p2) = deserialize(&raw).unwrap();
            assert_eq!(k2, kind);
            assert_eq!(p2, payload, "payload must survive embedded NUL bytes");
        }
    }

    #[test]
    fn deserialize_empty_payload() {
        let (kind, payload) = deserialize(b"commit 0\0").unwrap();
        assert_eq!(kind, ObjectType::Commit);
        assert!(payload.is_empty());
    }

    #[test]
    fn deserialize_rejects_corrupt_input() {
        let e = deserialize(b"no nul here").unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("missing NUL"));
        assert!(deserialize(b"weird 3\0abc").is_err());
    }

    #[test]
    fn object_id_matches_git_hash_object() {
        // `printf 'hello' | git hash-object --stdin`
        assert_eq!(
            object_id(ObjectType::Blob, b"hello").unwrap(),
            "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0"
        );
        // the well known empty blob
        assert_eq!(
            object_id(ObjectType::Blob, b"").unwrap(),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    #[tokio::test]
    async fn write_then_read_round_trip() {
        let (_d, o) = odb();
        let sha = o.write(ObjectType::Blob, b"hello").await.unwrap();
        assert_eq!(sha, "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0");
        assert!(o.exists(&sha).await.unwrap());
        let (kind, payload) = o.read(&sha).await.unwrap();
        assert_eq!(kind, ObjectType::Blob);
        assert_eq!(payload, b"hello");
        assert_eq!(o.read_header(&sha).await.unwrap(), (ObjectType::Blob, 5));
    }

    #[tokio::test]
    async fn write_stores_under_the_git_prefixed_key() {
        let (_d, o) = odb();
        let sha = o.write(ObjectType::Blob, b"x").await.unwrap();
        assert!(
            o.blobs().exists(&format!("git:{sha}")).await.unwrap(),
            "the blob must live under git:{{sha}}"
        );
        assert!(
            !o.blobs().exists(&sha).await.unwrap(),
            "and never under the bare sha"
        );
    }

    #[tokio::test]
    async fn write_records_the_index_row() {
        let (_d, o) = odb();
        let sha = o.write(ObjectType::Commit, b"tree x\n").await.unwrap();
        let row = o.index().get_object(&sha).await.unwrap().unwrap();
        assert_eq!(row.kind, "commit");
        assert_eq!(row.size, 7, "size is the payload length, header excluded");
    }

    #[tokio::test]
    async fn read_missing_is_not_found() {
        let (_d, o) = odb();
        let e = o.read("0000000000000000000000000000000000000000").await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(!o.exists("dead").await.unwrap());
    }

    #[tokio::test]
    async fn prefix_lookup_resolves_ambiguity() {
        let (_d, o) = odb();
        let sha = o.write(ObjectType::Blob, b"hello").await.unwrap();
        let (found, kind, payload) = o.read_prefix(&sha[..7]).await.unwrap();
        assert_eq!(found, sha);
        assert_eq!(kind, ObjectType::Blob);
        assert_eq!(payload, b"hello");

        // two objects sharing a fabricated prefix must report ambiguity
        o.index().record_object("aaaa1111", "blob", 1).await.unwrap();
        o.index().record_object("aaaa2222", "blob", 1).await.unwrap();
        let e = o.resolve_prefix("aaaa").await.unwrap_err();
        assert_eq!(e.code, code::AMBIGUOUS_MATCH);

        let e = o.resolve_prefix("zzzz").await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_removes_the_blob() {
        let (_d, o) = odb();
        let sha = o.write(ObjectType::Blob, b"gone").await.unwrap();
        o.delete(&sha).await.unwrap();
        assert!(!o.exists(&sha).await.unwrap());
    }

    #[tokio::test]
    async fn export_then_import_round_trips_through_libgit2() {
        let (_d, o) = odb();
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();

        let sha = o.write(ObjectType::Blob, b"exported bytes").await.unwrap();
        assert_eq!(o.export_to_repo(&repo).await.unwrap(), 1);
        // libgit2 can now see the object
        let oid = Oid::from_str(&sha).unwrap();
        assert!(repo.odb().unwrap().exists(oid));
        // exporting twice writes nothing new
        assert_eq!(o.export_to_repo(&repo).await.unwrap(), 0);

        // an object created directly by libgit2 comes back into the blob store
        let new_oid = repo.blob(b"written by libgit2").unwrap();
        assert_eq!(o.import_from_repo(&repo).await.unwrap(), 1);
        let (kind, payload) = o.read(&new_oid.to_string()).await.unwrap();
        assert_eq!(kind, ObjectType::Blob);
        assert_eq!(payload, b"written by libgit2");
        assert_eq!(o.list().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn seed_commit_writes_objects_libgit2_can_parse() {
        let (_d, o) = odb();
        let (commit, tree, blob) = seed_commit(&o, "readme.txt", b"file content\n", "init")
            .await
            .unwrap();

        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();
        assert_eq!(o.export_to_repo(&repo).await.unwrap(), 3);

        // libgit2 parsing the commit proves the hand built bytes are valid git
        let c = repo.find_commit(Oid::from_str(&commit).unwrap()).unwrap();
        assert_eq!(c.message().unwrap(), "init\n");
        assert_eq!(c.tree_id().to_string(), tree);
        assert_eq!(c.author().name().unwrap(), "mcp-fs");
        let t = c.tree().unwrap();
        let entry = t.get_name("readme.txt").unwrap();
        assert_eq!(entry.id().to_string(), blob);
        assert_eq!(entry.filemode(), 0o100644);
    }

    #[tokio::test]
    async fn import_reindexes_a_blob_missing_from_the_index() {
        let (_d, o) = odb();
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();
        let oid = repo.blob(b"orphan").unwrap();
        let sha = oid.to_string();

        // blob present, index row missing (a crash between the two writes)
        o.blobs()
            .put(&blob_key(&sha), &serialize(ObjectType::Blob, b"orphan").unwrap())
            .await
            .unwrap();
        assert!(!o.index().object_exists(&sha).await.unwrap());

        assert_eq!(o.import_from_repo(&repo).await.unwrap(), 0, "blob already stored");
        assert!(o.index().object_exists(&sha).await.unwrap(), "index repaired");
    }
}
