//! Blob backed git object database.
//!
//! Git objects are stored in the volume's [`BlobBackend`] under the key
//! `git:{sha}` holding the canonical git object bytes `{type} {len}\0{payload}`,
//! with a `(hash, type, size)` row in [`RelationalGitDb`] used for `ForEach` and
//! short sha (prefix) lookups. The key layout is a stable on disk contract, so an
//! existing deployment's git objects stay readable across upgrades.
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
use crate::git::db::{GitObjectRow, RelationalGitDb};
use crate::storage::BlobBackend;
use git2::{ObjectType, Oid};
use std::collections::HashSet;
use std::sync::Arc;

/// Tree entry mode of a submodule commit.
const GITLINK_MODE: i32 = 0o160000;

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
        other => Err(ToolError::invalid_argument(format!("unknown object type {other:?}"))),
    }
}

/// Inverse of [`object_type_name`].
pub fn parse_object_type(s: &str) -> Result<ObjectType> {
    match s {
        "blob" => Ok(ObjectType::Blob),
        "tree" => Ok(ObjectType::Tree),
        "commit" => Ok(ObjectType::Commit),
        "tag" => Ok(ObjectType::Tag),
        other => Err(ToolError::invalid_argument(format!("unknown object type string '{other}'"))),
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
    index: Arc<RelationalGitDb>,
}

impl BlobObjectDb {
    pub fn new(blobs: Arc<dyn BlobBackend>, index: Arc<RelationalGitDb>) -> Self {
        Self { blobs, index }
    }

    pub fn index(&self) -> &Arc<RelationalGitDb> {
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
        self.index.record_object(&sha, object_type_name(kind)?, payload.len() as i64).await?;
        Ok(sha)
    }

    /// Store an object whose sha is already known (an object copied in from
    /// libgit2), skipping the rehash.
    pub async fn write_with_sha(&self, sha: &str, kind: ObjectType, payload: &[u8]) -> Result<()> {
        let raw = serialize(kind, payload)?;
        self.blobs.put(&blob_key(sha), &raw).await?;
        self.index.record_object(sha, object_type_name(kind)?, payload.len() as i64).await
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
            0 => Err(ToolError::not_found(format!("no git object matches '{prefix}'"))),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => Err(ToolError::ambiguous_match(format!("'{prefix}' matches {n} git objects"))),
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
            let odb =
                repo.odb().map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
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

    /// Copy exactly one object libgit2 holds into the blob store and the index.
    /// Returns 1 when it was newly stored, 0 when the store already had it or the
    /// object type is not storable. Callers that just created one object use this
    /// instead of [`Self::import_from_repo`], which walks and reads every object of
    /// the repository. An object libgit2 lacks is `ERR_NOT_FOUND`, so a caller never
    /// records a ref naming an object the store does not hold.
    pub async fn import_object(&self, repo: &git2::Repository, oid: Oid) -> Result<usize> {
        // Copy out before any await: the git2 Odb borrow cannot be held across one.
        let (kind, payload) = {
            let odb =
                repo.odb().map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
            let obj = odb.read(oid).map_err(|_| {
                ToolError::not_found(format!("git object '{oid}' not found in the repository"))
            })?;
            if object_type_name(obj.kind()).is_err() {
                return Ok(0);
            }
            (obj.kind(), obj.data().to_vec())
        };

        let sha = oid.to_string();
        if self.exists(&sha).await? {
            // Same index repair as import_from_repo, so prefix lookups work after an index loss.
            if !self.index.object_exists(&sha).await? {
                self.index
                    .record_object(&sha, object_type_name(kind)?, payload.len() as i64)
                    .await?;
            }
            return Ok(0);
        }
        self.write_with_sha(&sha, kind, &payload).await?;
        Ok(1)
    }

    /// Copy into the blob store and the index every object reachable from `tips`
    /// that the store lacks. Returns how many were stored.
    ///
    /// [`Self::import_from_repo`] reads and probes every object of the repository,
    /// so a bounded change (one commit) paid O(repository). This walks from the new
    /// tip and stops at any object the store holds, so it costs O(new objects): a
    /// tree is compared with its first parent's tree and only the changed entries
    /// are read or probed (one probe is one network request on S3).
    ///
    /// Stopping at a stored object is sound because of the closure invariant: an
    /// object is written only after everything it references that must be stored,
    /// so a stored object implies its whole closure is stored. Hence the two phase
    /// shape: every read and probe happens in the traversal, which fills a post
    /// order list (blobs, trees, parents, children); only then is the list written
    /// front to back. A crash leaves a closed prefix, and an object `repo` lacks
    /// fails the call before anything is written. A gitlink entry names a
    /// submodule commit that is never in the odb, so it is skipped.
    pub async fn import_reachable(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize> {
        let mut stack: Vec<Frame> =
            tips.iter().rev().map(|oid| Frame::Enter { oid: *oid, base: None }).collect();
        let mut seen: HashSet<Oid> = HashSet::new();
        let mut order: Vec<(String, ObjectType, Vec<u8>)> = Vec::new();

        while let Some(frame) = stack.pop() {
            let (oid, base) = match frame {
                Frame::Exit(object) => {
                    order.push(object);
                    continue;
                }
                Frame::Enter { oid, base } => (oid, base),
            };
            if base == Some(oid) || !seen.insert(oid) {
                continue;
            }
            let sha = oid.to_string();
            if self.exists(&sha).await? {
                continue;
            }
            // No git2 borrow survives `expand`, so the awaits above and below are safe.
            let Some((kind, payload, children)) = expand(repo, oid, base)? else {
                continue;
            };
            stack.push(Frame::Exit((sha, kind, payload)));
            stack.extend(children.into_iter().rev());
        }

        for (sha, kind, payload) in &order {
            self.write_with_sha(sha, *kind, payload).await?;
        }
        Ok(order.len())
    }

    /// Import the closure of the tips a pack names, so the cost follows the new
    /// objects and not the repository size. `import_reachable` is strict: a tip or
    /// parent libgit2 lacks fails it with `ERR_NOT_FOUND` before anything is
    /// written, and a client can cause that (a push naming an unknown tip). Only
    /// then does the whole walk run, so such a request keeps its per ref answer.
    /// Any other error (a failed blob write, an index error) is returned as is: the
    /// whole walk would hit the same failure after more work.
    pub async fn import_tips(&self, repo: &git2::Repository, tips: &[Oid]) -> Result<usize> {
        match self.import_reachable(repo, tips).await {
            Err(e) if e.code == code::NOT_FOUND => self.import_from_repo(repo).await,
            other => other,
        }
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
                let odb =
                    repo.odb().map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
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

        let odb = repo.odb().map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
        let mut written = 0usize;
        for (kind, payload) in payloads {
            odb.write(kind, &payload)
                .map_err(|e| ToolError::internal(format!("odb write failed: {e}")))?;
            written += 1;
        }
        Ok(written)
    }
}

/// An object read from the repository: its type, payload and the frames it queues.
type Expanded = (ObjectType, Vec<u8>, Vec<Frame>);

/// One step of the [`BlobObjectDb::import_reachable`] traversal.
enum Frame {
    /// Visit an object; `base` is the same-named tree of the first parent, if any.
    Enter { oid: Oid, base: Option<Oid> },
    /// All the object's dependencies are queued before it: record it for writing.
    Exit((String, ObjectType, Vec<u8>)),
}

fn git_err(what: &str, e: git2::Error) -> ToolError {
    ToolError::internal(format!("{what} failed: {e}"))
}

/// Read one object and list the objects it references that must be visited.
/// `None` when the type is not storable. Everything is copied out of the git2
/// borrows so the caller can await afterwards.
fn expand(repo: &git2::Repository, oid: Oid, base: Option<Oid>) -> Result<Option<Expanded>> {
    let odb = repo.odb().map_err(|e| git_err("odb open", e))?;
    let obj = odb.read(oid).map_err(|_| {
        ToolError::not_found(format!("git object '{oid}' not found in the repository"))
    })?;
    let kind = obj.kind();
    if object_type_name(kind).is_err() {
        return Ok(None);
    }
    let payload = obj.data().to_vec();
    let mut children = Vec::new();
    match kind {
        ObjectType::Commit => {
            let commit = repo.find_commit(oid).map_err(|e| git_err("commit parse", e))?;
            // The first parent's tree is the comparison base, when repo can read it.
            let base = commit
                .parent_id(0)
                .ok()
                .and_then(|p| repo.find_commit(p).ok())
                .map(|p| p.tree_id());
            children.push(Frame::Enter { oid: commit.tree_id(), base });
            children.extend(commit.parent_ids().map(|oid| Frame::Enter { oid, base: None }));
        }
        ObjectType::Tag => {
            let tag = repo.find_tag(oid).map_err(|e| git_err("tag parse", e))?;
            children.push(Frame::Enter { oid: tag.target_id(), base: None });
        }
        ObjectType::Tree => {
            let tree = repo.find_tree(oid).map_err(|e| git_err("tree parse", e))?;
            let base_tree = base.and_then(|b| repo.find_tree(b).ok());
            for entry in tree.iter() {
                if entry.filemode() == GITLINK_MODE {
                    continue;
                }
                let counterpart =
                    base_tree.as_ref().and_then(|t| t.get_name_bytes(entry.name_bytes()));
                if counterpart.as_ref().is_some_and(|c| c.id() == entry.id()) {
                    continue;
                }
                let base = counterpart
                    .filter(|c| c.kind() == Some(ObjectType::Tree))
                    .filter(|_| entry.kind() == Some(ObjectType::Tree))
                    .map(|c| c.id());
                children.push(Frame::Enter { oid: entry.id(), base });
            }
        }
        _ => {}
    }
    Ok(Some((kind, payload, children)))
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
    tree.extend_from_slice(
        &hex::decode(&blob_sha)
            .map_err(|e| ToolError::internal(format!("blob sha is not hex: {e}")))?,
    );
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

    use async_trait::async_trait;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Delegates to a local store and counts `exists` probes.
    struct CountingBlobs {
        inner: LocalBlobStore,
        exists_calls: AtomicUsize,
    }

    #[async_trait]
    impl BlobBackend for CountingBlobs {
        async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
            self.inner.put(key, data).await
        }
        async fn get(&self, key: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
            self.inner.get(key, offset, length).await
        }
        async fn exists(&self, key: &str) -> Result<bool> {
            self.exists_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key).await
        }
        async fn ensure_bucket(&self) -> Result<()> {
            self.inner.ensure_bucket().await
        }
        async fn remove_bucket(&self) -> Result<()> {
            self.inner.remove_bucket().await
        }
    }

    /// Delegates to a local store, but every `put` is counted and fails.
    struct FailingPutBlobs {
        inner: LocalBlobStore,
        puts: AtomicUsize,
    }

    #[async_trait]
    impl BlobBackend for FailingPutBlobs {
        async fn put(&self, _key: &str, _data: &[u8]) -> Result<()> {
            self.puts.fetch_add(1, Ordering::SeqCst);
            Err(ToolError::internal("disk full"))
        }
        async fn get(&self, key: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
            self.inner.get(key, offset, length).await
        }
        async fn exists(&self, key: &str) -> Result<bool> {
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key).await
        }
        async fn ensure_bucket(&self) -> Result<()> {
            self.inner.ensure_bucket().await
        }
        async fn remove_bucket(&self) -> Result<()> {
            self.inner.remove_bucket().await
        }
    }

    /// Delegates to a local store and records the key of every `put`, in order.
    struct RecordingBlobs {
        inner: LocalBlobStore,
        puts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl BlobBackend for RecordingBlobs {
        async fn put(&self, key: &str, data: &[u8]) -> Result<()> {
            self.puts.lock().unwrap().push(key.to_string());
            self.inner.put(key, data).await
        }
        async fn get(&self, key: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
            self.inner.get(key, offset, length).await
        }
        async fn exists(&self, key: &str) -> Result<bool> {
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key).await
        }
        async fn ensure_bucket(&self) -> Result<()> {
            self.inner.ensure_bucket().await
        }
        async fn remove_bucket(&self) -> Result<()> {
            self.inner.remove_bucket().await
        }
    }

    async fn odb_over(blobs: Arc<dyn BlobBackend>) -> BlobObjectDb {
        let index = Arc::new(RelationalGitDb::open_in_memory().await.unwrap());
        BlobObjectDb::new(blobs, index)
    }

    async fn counting_odb() -> (tempfile::TempDir, Arc<CountingBlobs>, BlobObjectDb) {
        let d = tempfile::tempdir().unwrap();
        let counting = Arc::new(CountingBlobs {
            inner: LocalBlobStore::new(d.path(), "mcpfs-git-test"),
            exists_calls: AtomicUsize::new(0),
        });
        let o = odb_over(counting.clone()).await;
        (d, counting, o)
    }

    fn bare() -> (tempfile::TempDir, git2::Repository) {
        let d = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(d.path()).unwrap();
        (d, repo)
    }

    fn sig() -> git2::Signature<'static> {
        git2::Signature::now("t", "t@t").unwrap()
    }

    fn commit_on(repo: &git2::Repository, tree: Oid, parents: &[Oid]) -> Oid {
        let tree = repo.find_tree(tree).unwrap();
        let parents: Vec<git2::Commit> =
            parents.iter().map(|p| repo.find_commit(*p).unwrap()).collect();
        let refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(None, &sig(), &sig(), "m", &tree, &refs).unwrap()
    }

    /// A flat tree of `name -> content` blobs.
    fn flat_tree(repo: &git2::Repository, files: &[(&str, &str)]) -> Oid {
        let mut tb = repo.treebuilder(None).unwrap();
        for (name, content) in files {
            tb.insert(name, repo.blob(content.as_bytes()).unwrap(), 0o100644).unwrap();
        }
        tb.write().unwrap()
    }

    /// First commit: `top` plus a directory `d` of 200 files. Second commit (child of the
    /// first): only `d/f007` changes.
    fn wide_history(repo: &git2::Repository) -> (Oid, Oid) {
        let mut dir = repo.treebuilder(None).unwrap();
        for i in 0..200 {
            let blob = repo.blob(format!("content {i}").as_bytes()).unwrap();
            dir.insert(format!("f{i:03}"), blob, 0o100644).unwrap();
        }
        let dir = dir.write().unwrap();
        let mut root = repo.treebuilder(None).unwrap();
        root.insert("top", repo.blob(b"top").unwrap(), 0o100644).unwrap();
        root.insert("d", dir, 0o040000).unwrap();
        let first = commit_on(repo, root.write().unwrap(), &[]);

        let dir_tree = repo.find_tree(dir).unwrap();
        let mut dir2 = repo.treebuilder(Some(&dir_tree)).unwrap();
        dir2.insert("f007", repo.blob(b"changed").unwrap(), 0o100644).unwrap();
        let dir2 = dir2.write().unwrap();
        let root_tree = repo.find_tree(root.write().unwrap()).unwrap();
        let mut root2 = repo.treebuilder(Some(&root_tree)).unwrap();
        root2.insert("d", dir2, 0o040000).unwrap();
        let second = commit_on(repo, root2.write().unwrap(), &[first]);
        (first, second)
    }

    async fn odb() -> (tempfile::TempDir, BlobObjectDb) {
        let d = tempfile::tempdir().unwrap();
        let blobs: Arc<dyn BlobBackend> = Arc::new(LocalBlobStore::new(d.path(), "mcpfs-git-test"));
        let index = Arc::new(RelationalGitDb::open_in_memory().await.unwrap());
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
        for kind in [ObjectType::Blob, ObjectType::Tree, ObjectType::Commit, ObjectType::Tag] {
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
        let (_d, o) = odb().await;
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
        let (_d, o) = odb().await;
        let sha = o.write(ObjectType::Blob, b"x").await.unwrap();
        assert!(
            o.blobs().exists(&format!("git:{sha}")).await.unwrap(),
            "the blob must live under git:{{sha}}"
        );
        assert!(!o.blobs().exists(&sha).await.unwrap(), "and never under the bare sha");
    }

    #[tokio::test]
    async fn write_records_the_index_row() {
        let (_d, o) = odb().await;
        let sha = o.write(ObjectType::Commit, b"tree x\n").await.unwrap();
        let row = o.index().get_object(&sha).await.unwrap().unwrap();
        assert_eq!(row.kind, "commit");
        assert_eq!(row.size, 7, "size is the payload length, header excluded");
    }

    #[tokio::test]
    async fn read_missing_is_not_found() {
        let (_d, o) = odb().await;
        let e = o.read("0000000000000000000000000000000000000000").await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(!o.exists("dead").await.unwrap());
    }

    #[tokio::test]
    async fn prefix_lookup_resolves_ambiguity() {
        let (_d, o) = odb().await;
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
        let (_d, o) = odb().await;
        let sha = o.write(ObjectType::Blob, b"gone").await.unwrap();
        o.delete(&sha).await.unwrap();
        assert!(!o.exists(&sha).await.unwrap());
    }

    #[tokio::test]
    async fn export_then_import_round_trips_through_libgit2() {
        let (_d, o) = odb().await;
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
        let (_d, o) = odb().await;
        let (commit, tree, blob) =
            seed_commit(&o, "readme.txt", b"file content\n", "init").await.unwrap();

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
        let (_d, o) = odb().await;
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

    #[tokio::test]
    async fn import_object_stores_exactly_one_object() {
        let (_d, o) = odb().await;
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();
        let oid = repo.blob(b"one").unwrap();

        assert_eq!(o.import_object(&repo, oid).await.unwrap(), 1);
        assert_eq!(o.import_object(&repo, oid).await.unwrap(), 0);
        let (kind, payload) = o.read(&oid.to_string()).await.unwrap();
        assert_eq!(kind, ObjectType::Blob);
        assert_eq!(payload, b"one");
        assert_eq!(o.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn import_object_leaves_other_objects_alone() {
        let (_d, o) = odb().await;
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();
        for i in 0..2000 {
            repo.blob(format!("blob {i}").as_bytes()).unwrap();
        }
        let target = repo.blob(b"target").unwrap();

        assert_eq!(o.import_object(&repo, target).await.unwrap(), 1);
        assert_eq!(o.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn import_object_repairs_a_missing_index_row() {
        let (_d, o) = odb().await;
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();
        let oid = repo.blob(b"orphan").unwrap();
        let sha = oid.to_string();
        o.blobs()
            .put(&blob_key(&sha), &serialize(ObjectType::Blob, b"orphan").unwrap())
            .await
            .unwrap();
        assert!(!o.index().object_exists(&sha).await.unwrap());

        assert_eq!(o.import_object(&repo, oid).await.unwrap(), 0, "blob already stored");
        assert!(o.index().object_exists(&sha).await.unwrap(), "index repaired");
    }

    #[tokio::test]
    async fn dt001_a_commit_tip_imports_exactly_its_three_objects() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        for i in 0..2000 {
            repo.blob(format!("blob {i}").as_bytes()).unwrap();
        }
        let commit = commit_on(&repo, flat_tree(&repo, &[("a.txt", "a")]), &[]);

        assert_eq!(o.import_reachable(&repo, &[commit]).await.unwrap(), 3);
        assert_eq!(o.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn dt002_a_second_commit_stores_only_what_changed() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let (first, second) = wide_history(&repo);
        assert_eq!(o.import_reachable(&repo, &[first]).await.unwrap(), 204);
        let before = o.list().await.unwrap().len();

        assert_eq!(o.import_reachable(&repo, &[second]).await.unwrap(), 4);
        assert_eq!(o.list().await.unwrap().len(), before + 4);
    }

    #[tokio::test]
    async fn dt003_the_store_probes_are_bounded() {
        let (_d, counting, o) = counting_odb().await;
        let (_r, repo) = bare();
        let (first, second) = wide_history(&repo);
        o.import_reachable(&repo, &[first]).await.unwrap();
        counting.exists_calls.store(0, Ordering::SeqCst);

        assert_eq!(o.import_reachable(&repo, &[second]).await.unwrap(), 4);
        let probes = counting.exists_calls.load(Ordering::SeqCst);
        assert!(probes <= 8, "{probes} store probes for 1 changed file in a 200 file directory");
    }

    #[tokio::test]
    async fn dt004_a_merge_commit_sharing_its_second_parents_tree() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let t1 = flat_tree(&repo, &[("a", "1")]);
        let t2 = flat_tree(&repo, &[("b", "2")]);
        let p1 = commit_on(&repo, t1, &[]);
        let p2 = commit_on(&repo, t2, &[]);
        o.import_reachable(&repo, &[p1, p2]).await.unwrap();
        let merge = commit_on(&repo, t2, &[p1, p2]);

        assert_eq!(o.import_reachable(&repo, &[merge]).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn dt005_a_root_commit_imports_all_its_entries() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let sub = flat_tree(&repo, &[("z.txt", "zzz")]);
        let mut root = repo.treebuilder(None).unwrap();
        root.insert("x.txt", repo.blob(b"xxx").unwrap(), 0o100644).unwrap();
        root.insert("y.txt", repo.blob(b"yyy").unwrap(), 0o100644).unwrap();
        root.insert("s", sub, 0o040000).unwrap();
        let commit = commit_on(&repo, root.write().unwrap(), &[]);

        assert_eq!(o.import_reachable(&repo, &[commit]).await.unwrap(), 6);
    }

    #[tokio::test]
    async fn dt006_a_gitlink_entry_is_skipped() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let sub = "2222222222222222222222222222222222222222";
        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert("sub", Oid::from_str(sub).unwrap(), 0o160000).unwrap();
        let commit = commit_on(&repo, tb.write().unwrap(), &[]);

        assert_eq!(o.import_reachable(&repo, &[commit]).await.unwrap(), 2);
        assert!(!o.exists(sub).await.unwrap());
    }

    #[tokio::test]
    async fn dt007_an_unknown_tip() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let unknown = Oid::from_str("1111111111111111111111111111111111111111").unwrap();

        let e = o.import_reachable(&repo, &[unknown]).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert_eq!(
            e.message,
            "git object '1111111111111111111111111111111111111111' not found in the repository"
        );
        assert!(o.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn dt008_an_annotated_tag_tip() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let commit = commit_on(&repo, flat_tree(&repo, &[("a.txt", "a")]), &[]);
        let target = repo.find_object(commit, None).unwrap();
        let tag = repo.tag("v1", &target, &sig(), "m", false).unwrap();

        assert_eq!(o.import_reachable(&repo, &[tag]).await.unwrap(), 4);
        assert!(o.exists(&tag.to_string()).await.unwrap());
        assert!(o.exists(&commit.to_string()).await.unwrap());
    }

    #[tokio::test]
    async fn dt009_a_tip_the_store_already_holds() {
        let (_d, counting, o) = counting_odb().await;
        let (_r, repo) = bare();
        let commit = commit_on(&repo, flat_tree(&repo, &[("a.txt", "a")]), &[]);
        assert_eq!(o.import_reachable(&repo, &[commit]).await.unwrap(), 3);
        counting.exists_calls.store(0, Ordering::SeqCst);

        assert_eq!(o.import_reachable(&repo, &[commit]).await.unwrap(), 0);
        assert_eq!(counting.exists_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dt010_dependencies_are_written_first() {
        let d = tempfile::tempdir().unwrap();
        let recording = Arc::new(RecordingBlobs {
            inner: LocalBlobStore::new(d.path(), "mcpfs-git-test"),
            puts: Mutex::new(Vec::new()),
        });
        let o = odb_over(recording.clone()).await;
        let (_r, repo) = bare();
        let blob = repo.blob(b"a").unwrap();
        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert("a.txt", blob, 0o100644).unwrap();
        let tree = tb.write().unwrap();
        let commit = commit_on(&repo, tree, &[]);

        o.import_reachable(&repo, &[commit]).await.unwrap();
        let puts = recording.puts.lock().unwrap().clone();
        assert_eq!(
            puts,
            vec![
                blob_key(&blob.to_string()),
                blob_key(&tree.to_string()),
                blob_key(&commit.to_string())
            ]
        );
    }

    /// 2 000 unimported blobs plus one commit (blob, tree, commit): 2 003 objects.
    fn commit_among_strays(repo: &git2::Repository) -> Oid {
        for i in 0..2000 {
            repo.blob(format!("blob {i}").as_bytes()).unwrap();
        }
        commit_on(repo, flat_tree(repo, &[("a.txt", "a")]), &[])
    }

    #[tokio::test]
    async fn us0123_dt001_one_new_commit_among_strays() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        let commit = commit_among_strays(&repo);

        assert_eq!(o.import_tips(&repo, &[commit]).await.unwrap(), 3);
        assert_eq!(o.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn us0123_dt002_an_unreadable_tip_falls_back_to_the_whole_walk() {
        let (_d, o) = odb().await;
        let (_r, repo) = bare();
        commit_among_strays(&repo);
        let unknown = Oid::from_str("1111111111111111111111111111111111111111").unwrap();

        let imported = o.import_tips(&repo, &[unknown]).await;
        assert_eq!(imported.as_ref().ok(), Some(&2003), "{imported:?}");
        assert_eq!(o.list().await.unwrap().len(), 2003);
    }

    #[tokio::test]
    async fn us0123_dt003_a_write_failure_is_not_retried_by_the_whole_walk() {
        let d = tempfile::tempdir().unwrap();
        let failing = Arc::new(FailingPutBlobs {
            inner: LocalBlobStore::new(d.path(), "mcpfs-git-test"),
            puts: AtomicUsize::new(0),
        });
        let o = odb_over(failing.clone()).await;
        let (_r, repo) = bare();
        let commit = commit_among_strays(&repo);

        let e = o.import_tips(&repo, &[commit]).await.unwrap_err();
        assert!(e.message.contains("disk full"), "{}", e.message);
        assert_eq!(failing.puts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn us0123_dt004_empty_tips() {
        let (_d, counting, o) = counting_odb().await;
        let (_r, repo) = bare();
        commit_among_strays(&repo);

        assert_eq!(o.import_tips(&repo, &[]).await.unwrap(), 0);
        assert_eq!(counting.exists_calls.load(Ordering::SeqCst), 0);
        assert!(o.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn import_object_on_an_unknown_id_is_not_found() {
        let (_d, o) = odb().await;
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(repo_dir.path()).unwrap();
        let unknown = Oid::from_str("1111111111111111111111111111111111111111").unwrap();

        let e = o.import_object(&repo, unknown).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert_eq!(
            e.message,
            "git object '1111111111111111111111111111111111111111' not found in the repository"
        );
        assert!(o.list().await.unwrap().is_empty());
    }
}
