//! Auto indexing: the side effects of a project's [`IndexMode`].
//!
//! One place knows how a mode maps to index operations, so the write tools, the
//! delete tool, the REST upload and `admin.set_index_mode` all behave the same.
//!
//! Two rules hold everywhere here:
//!
//! * **A write never waits for the index.** `on_write` and `on_delete` hand the
//!   work to a detached task and return immediately, so a slow or absent
//!   embedding endpoint costs a write nothing and fails it never. The index
//!   therefore lags the volume by the duration of one backend call, which is
//!   visible to a caller that writes and queries back to back.
//! * **An index failure is a log line, not an error.** The caller already
//!   committed bytes; refusing the write afterwards would be a lie.
//!
//! The search backend is process wide and serves whatever engine the server was
//! configured with (`search.mode`). A project's mode decides WHETHER its content
//! is indexed, not which engine indexes it.

use crate::core::fs_ops;
use crate::errors::Result;
use crate::search::SearchBackend;
use crate::state::AppState;
use crate::storage::VolumeClient;
use crate::storage::traits::IndexMode;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Chunk window used by auto indexing, matching the `search.index` tool defaults.
/// There is no per project chunk configuration in this iteration.
pub const AUTO_CHUNK_SIZE: usize = 1000;
/// Overlap between consecutive auto indexed chunks, matching `search.index`.
pub const AUTO_CHUNK_OVERLAP: usize = 100;

/// The side effect engine for one search backend.
///
/// Stateless and cheap to build: it borrows the backend out of the app state for
/// the duration of one call.
pub struct ProjectIndexer<'a> {
    backend: &'a Arc<dyn SearchBackend>,
}

impl<'a> ProjectIndexer<'a> {
    pub fn new(backend: &'a Arc<dyn SearchBackend>) -> Self {
        Self { backend }
    }

    /// Apply a mode change. Returns true when a background full index was started.
    ///
    /// The wipe is awaited, because `admin.set_index_mode` promises the index is
    /// empty when it answers. The rebuild is not: a large volume takes minutes and
    /// an admin call must not hold a request open for them.
    pub async fn on_mode_change(
        &self,
        volume_id: &str,
        old_mode: IndexMode,
        new_mode: IndexMode,
        client: Arc<VolumeClient>,
    ) -> Result<bool> {
        if old_mode == new_mode {
            // Same mode twice: wiping here would throw away a good index for nothing.
            return Ok(false);
        }

        // Every transition wipes, including none -> active: the volume may carry
        // chunks from an earlier mode, or from a manual `search.index` call.
        let wiped = self.backend.delete_all(volume_id).await?;
        tracing::info!(
            volume_id,
            old_mode = %old_mode,
            new_mode = %new_mode,
            wiped,
            "search index mode changed"
        );

        if !new_mode.is_active() {
            return Ok(false);
        }

        let backend = self.backend.clone();
        let vol = volume_id.to_string();
        tokio::task::spawn(async move {
            match full_index(&backend, &vol, &client).await {
                Ok(n) => {
                    tracing::info!(volume_id = %vol, chunks = n, "initial full index finished")
                }
                Err(e) => {
                    tracing::warn!(volume_id = %vol, error = %e, "initial full index failed")
                }
            }
        });
        Ok(true)
    }

    /// Index one file's new content, without waiting for it.
    ///
    /// The returned handle exists so a test can await the work; production callers
    /// drop it, which detaches the task.
    pub fn on_write(&self, volume_id: &str, path: &str, text: &str) -> JoinHandle<()> {
        let backend = self.backend.clone();
        let vol = volume_id.to_string();
        let p = path.to_string();
        let t = text.to_string();
        tokio::task::spawn(async move {
            if let Err(e) =
                backend.index_path(&vol, &p, &t, AUTO_CHUNK_SIZE, AUTO_CHUNK_OVERLAP).await
            {
                tracing::warn!(volume_id = %vol, path = %p, error = %e, "auto index write failed");
            }
        })
    }

    /// Drop one file from the index, without waiting for it.
    pub fn on_delete(&self, volume_id: &str, path: &str) -> JoinHandle<()> {
        let backend = self.backend.clone();
        let vol = volume_id.to_string();
        let p = path.to_string();
        tokio::task::spawn(async move {
            if let Err(e) = backend.delete_path(&vol, &p).await {
                tracing::warn!(volume_id = %vol, path = %p, error = %e, "auto index delete failed");
            }
        })
    }
}

/// Index every file of a volume. Returns the number of chunks written.
///
/// Unreadable and non UTF-8 files are skipped, exactly as `search.index` skips
/// them: a volume full of binaries must not fail the pass.
pub async fn full_index(
    backend: &Arc<dyn SearchBackend>,
    volume_id: &str,
    client: &VolumeClient,
) -> Result<usize> {
    let files = fs_ops::iter_files(client, "/", &[]).await?;
    let mut chunks = 0usize;
    for (path, _mtime) in files {
        let Some(text) = read_utf8(client, &path).await else { continue };
        match backend.index_path(volume_id, &path, &text, AUTO_CHUNK_SIZE, AUTO_CHUNK_OVERLAP).await
        {
            Ok(n) => chunks += n,
            Err(e) => {
                tracing::warn!(volume_id, path = %path, error = %e, "full index skipped a file");
            }
        }
    }
    Ok(chunks)
}

/// A file's content as text, or `None` when it cannot be read as UTF-8.
///
/// `read_text` is lossy, so the bytes are decoded here instead: indexing the
/// replacement characters of a binary file is noise, not content.
async fn read_utf8(client: &VolumeClient, path: &str) -> Option<String> {
    let bytes = client.read_bytes(path).await.ok()?;
    String::from_utf8(bytes).ok()
}

// ── the hooks the tool handlers call ────────────────────────────────────────

/// The project's mode, or `None` when nothing should be indexed.
///
/// Answers `None` rather than an error when search is off or the mode cannot be
/// read: a write must not fail because of the index.
async fn active_backend<'s>(
    state: &'s AppState,
    volume_id: &str,
) -> Option<&'s Arc<dyn SearchBackend>> {
    let backend = state.search.as_ref()?;
    match state.admin.get_index_mode(volume_id).await {
        Ok(mode) if mode.is_active() => Some(backend),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(volume_id, error = %e, "cannot read the project index mode");
            None
        }
    }
}

/// Auto index a file whose new text the caller already holds.
pub async fn after_write(state: &AppState, volume_id: &str, path: &str, text: &str) {
    if let Some(backend) = active_backend(state, volume_id).await {
        drop(ProjectIndexer::new(backend).on_write(volume_id, path, text));
    }
}

/// Auto index a file whose new text the caller does not hold, by reading it back.
///
/// The read only happens when the project has an active mode, so a project that
/// indexes nothing pays nothing. A non UTF-8 file is skipped silently.
pub async fn after_write_reread(
    state: &AppState,
    volume_id: &str,
    path: &str,
    client: &VolumeClient,
) {
    if let Some(backend) = active_backend(state, volume_id).await
        && let Some(text) = read_utf8(client, path).await
    {
        drop(ProjectIndexer::new(backend).on_write(volume_id, path, &text));
    }
}

/// Auto remove one path from the index.
pub async fn after_delete(state: &AppState, volume_id: &str, path: &str) {
    if let Some(backend) = active_backend(state, volume_id).await {
        drop(ProjectIndexer::new(backend).on_delete(volume_id, path));
    }
}

/// Auto remove several paths from the index, for a recursive delete.
pub async fn after_delete_many(state: &AppState, volume_id: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    if let Some(backend) = active_backend(state, volume_id).await {
        let indexer = ProjectIndexer::new(backend);
        for path in paths {
            drop(indexer.on_delete(volume_id, path));
        }
    }
}

/// The paths a delete of `path` will remove from the index.
///
/// A directory delete takes every file underneath it, and each one has its own
/// index entries. Collected BEFORE the delete, because afterwards the tree is
/// gone. Returns just `path` when it is a file or cannot be walked.
pub async fn paths_under(
    state: &AppState,
    volume_id: &str,
    path: &str,
    client: &VolumeClient,
) -> Vec<String> {
    if active_backend(state, volume_id).await.is_none() {
        return Vec::new();
    }
    files_under(client, path).await
}

/// Every indexed path a move is about to invalidate: the source tree, which
/// moves away, and the destination tree, which `overwrite: true` destroys
/// outright. Both are enumerated BEFORE the rename, because afterwards neither
/// is there to list.
///
/// A file that lived only under the old destination has no counterpart in the
/// source, so nothing re-indexes it afterwards: without this it would keep an
/// index entry pointing at bytes the move deleted. Deduplicated, because a move
/// within one tree can list the same path on both sides.
pub async fn paths_displaced_by_move(
    state: &AppState,
    volume_id: &str,
    src: &str,
    dst: &str,
    client: &VolumeClient,
) -> Vec<String> {
    if active_backend(state, volume_id).await.is_none() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for path in files_under(client, src).await.into_iter().chain(files_under(client, dst).await) {
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

/// Follow a move in the index: forget the displaced paths, index the destination.
///
/// `sources` must have been collected with [`paths_displaced_by_move`] BEFORE
/// the move, for
/// the same reason a recursive delete collects first: once the rename has
/// happened there is no source tree left to walk, so its entries would stay in
/// the index forever with no path to enumerate them by. The destination side is
/// walked here instead of derived from `sources`, so a tree move needs no prefix
/// arithmetic to stay correct.
pub async fn after_move(
    state: &AppState,
    volume_id: &str,
    sources: &[String],
    destination: &str,
    client: &VolumeClient,
) {
    let Some(backend) = active_backend(state, volume_id).await else { return };
    let indexer = ProjectIndexer::new(backend);
    let clears: Vec<JoinHandle<()>> =
        sources.iter().map(|path| indexer.on_delete(volume_id, path)).collect();

    // The barrier. The cleared set and the re-indexed set OVERLAP: a file present
    // under both the source and the overwritten destination is cleared here and
    // written again just below, at the SAME path. `on_delete` and `on_write` both
    // detach, so without waiting the delete could land after the write and drop
    // the fresh chunks of a file that does exist. Awaiting is cheap: a clear is a
    // metadata-only backend delete, while the re-index below stays detached
    // because it costs an embedding round trip per file and must not sit on the
    // caller's write path.
    for clear in clears {
        if let Err(e) = clear.await {
            tracing::warn!(volume_id, error = %e, "an index clear task failed during a move");
        }
    }

    index_tree(backend, volume_id, destination, client).await;
}

/// Index the destination of a copy. The source keeps the entries it had, which
/// are still true: a copy leaves it untouched.
pub async fn after_copy(
    state: &AppState,
    volume_id: &str,
    destination: &str,
    client: &VolumeClient,
) {
    if let Some(backend) = active_backend(state, volume_id).await {
        index_tree(backend, volume_id, destination, client).await;
    }
}

/// Every file at or under `path`, or just `path` when it is a file or cannot be
/// walked. Directories carry no index entries of their own.
async fn files_under(client: &VolumeClient, path: &str) -> Vec<String> {
    if client.is_dir(path).await.unwrap_or(false) {
        return fs_ops::iter_files(client, path, &[])
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(p, _mtime)| p)
            .collect();
    }
    vec![path.to_string()]
}

/// Index every file that has just arrived at `root`, by re-reading it.
///
/// The counterpart of [`paths_under`]: that one enumerates before a removal,
/// this one after an arrival. Non UTF-8 files are skipped like everywhere else.
async fn index_tree(
    backend: &Arc<dyn SearchBackend>,
    volume_id: &str,
    root: &str,
    client: &VolumeClient,
) {
    let indexer = ProjectIndexer::new(backend);
    for path in files_under(client, root).await {
        let Some(text) = read_utf8(client, &path).await else { continue };
        drop(indexer.on_write(volume_id, &path, &text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ToolError;
    use crate::search::{IndexStats, SearchResult};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Records every call so a test can assert on what the indexer asked for.
    #[derive(Default)]
    struct RecordingBackend {
        calls: Mutex<Vec<String>>,
        /// When set, every index_path call fails, to prove errors stay contained.
        fail_index: bool,
        /// Latency added to every delete, recorded on completion. A real backend
        /// delete is a network round trip, so a test that needs the "delete lands
        /// after the write" interleaving has to make it reachable rather than
        /// hope the scheduler produces it.
        delete_delay: std::time::Duration,
    }

    impl RecordingBackend {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls mutex").clone()
        }

        fn record(&self, what: String) {
            self.calls.lock().expect("calls mutex").push(what);
        }
    }

    #[async_trait]
    impl SearchBackend for RecordingBackend {
        async fn index_path(
            &self,
            volume_id: &str,
            path: &str,
            text: &str,
            chunk_size: usize,
            chunk_overlap: usize,
        ) -> Result<usize> {
            self.record(format!("index {volume_id} {path} '{text}' {chunk_size}/{chunk_overlap}"));
            if self.fail_index {
                return Err(ToolError::internal("embedding endpoint is down"));
            }
            Ok(1)
        }

        async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
            tokio::time::sleep(self.delete_delay).await;
            self.record(format!("delete {volume_id} {path}"));
            Ok(1)
        }

        async fn delete_all(&self, volume_id: &str) -> Result<usize> {
            self.record(format!("delete_all {volume_id}"));
            Ok(3)
        }

        async fn query_bm25(&self, _v: &str, _q: &str, _k: usize) -> Result<Vec<SearchResult>> {
            Ok(Vec::new())
        }

        async fn query_vector(&self, _v: &str, _q: &str, _k: usize) -> Result<Vec<SearchResult>> {
            Ok(Vec::new())
        }

        async fn stats(&self, _volume_id: &str) -> Result<IndexStats> {
            Ok(IndexStats { bm25_docs: 0, vector_chunks: 0, bm25_warm: false, mode: "bm25".into() })
        }

        fn supported_modes(&self) -> Vec<&'static str> {
            vec!["bm25"]
        }
    }

    fn recording() -> (Arc<dyn SearchBackend>, Arc<RecordingBackend>) {
        with_delete_delay(std::time::Duration::ZERO)
    }

    fn with_delete_delay(
        delete_delay: std::time::Duration,
    ) -> (Arc<dyn SearchBackend>, Arc<RecordingBackend>) {
        let inner = Arc::new(RecordingBackend { delete_delay, ..Default::default() });
        (inner.clone() as Arc<dyn SearchBackend>, inner)
    }

    #[tokio::test]
    async fn on_write_hook_indexes_the_file_with_the_auto_chunk_window() {
        let (backend, rec) = recording();
        ProjectIndexer::new(&backend)
            .on_write("proj", "/a.md", "hello")
            .await
            .expect("the task must not panic");
        assert_eq!(rec.calls(), vec!["index proj /a.md 'hello' 1000/100"]);
    }

    #[tokio::test]
    async fn on_delete_hook_removes_the_path() {
        let (backend, rec) = recording();
        ProjectIndexer::new(&backend)
            .on_delete("proj", "/a.md")
            .await
            .expect("the task must not panic");
        assert_eq!(rec.calls(), vec!["delete proj /a.md"]);
    }

    /// A failing backend must not surface anywhere: the write already happened.
    #[tokio::test]
    async fn a_failing_index_call_is_contained_in_the_task() {
        let inner = Arc::new(RecordingBackend { fail_index: true, ..Default::default() });
        let backend = inner.clone() as Arc<dyn SearchBackend>;
        ProjectIndexer::new(&backend)
            .on_write("proj", "/a.md", "hello")
            .await
            .expect("a backend error must not panic the task");
        assert_eq!(inner.calls().len(), 1);
    }

    #[tokio::test]
    async fn on_mode_change_to_none_wipes_and_starts_nothing() {
        let (backend, rec) = recording();
        let client = client().await;
        let started = ProjectIndexer::new(&backend)
            .on_mode_change("proj", IndexMode::Bm25, IndexMode::None, client)
            .await
            .expect("the wipe must succeed");
        assert!(!started, "none must not start a reindex");
        assert_eq!(rec.calls(), vec!["delete_all proj"]);
    }

    #[tokio::test]
    async fn on_mode_change_to_an_active_mode_wipes_then_backfills() {
        let (backend, rec) = recording();
        let client = client().await;
        client.write_text_atomic("/a.md", "alpha").await.expect("seed");
        client.write_text_atomic("/b.md", "beta").await.expect("seed");

        let started = ProjectIndexer::new(&backend)
            .on_mode_change("proj", IndexMode::None, IndexMode::Bm25, client)
            .await
            .expect("the transition must succeed");
        assert!(started, "an active mode must start a reindex");

        // The backfill is a detached task, so wait for it rather than assume it ran.
        let calls = wait_for(&rec, 3).await;
        assert_eq!(calls[0], "delete_all proj", "the wipe comes first");
        assert!(calls.iter().any(|c| c.contains("index proj /a.md 'alpha'")), "{calls:?}");
        assert!(calls.iter().any(|c| c.contains("index proj /b.md 'beta'")), "{calls:?}");
    }

    #[tokio::test]
    async fn the_same_mode_twice_touches_nothing() {
        let (backend, rec) = recording();
        let client = client().await;
        let started = ProjectIndexer::new(&backend)
            .on_mode_change("proj", IndexMode::Both, IndexMode::Both, client)
            .await
            .expect("a no op must succeed");
        assert!(!started);
        assert!(rec.calls().is_empty(), "no wipe, no reindex: {:?}", rec.calls());
    }

    /// A binary file has no text to index, so the pass skips it instead of
    /// storing the replacement characters of a lossy decode.
    #[tokio::test]
    async fn full_index_skips_a_non_utf8_file() {
        let (backend, rec) = recording();
        let client = client().await;
        client.write_text_atomic("/good.md", "readable").await.expect("seed");
        client.write_bytes_atomic("/bad.bin", &[0xFF, 0xFE, 0x00]).await.expect("seed");

        full_index(&backend, "proj", &client).await.expect("the pass must succeed");
        let calls = rec.calls();
        assert_eq!(calls.len(), 1, "only the readable file is indexed: {calls:?}");
        assert!(calls[0].contains("/good.md"), "{calls:?}");
    }

    /// A file move must index the destination, by reading the bytes back.
    #[tokio::test]
    async fn index_tree_indexes_a_single_file_from_the_volume() {
        let (backend, rec) = recording();
        let client = client().await;
        client.write_text_atomic("/moved.md", "arrived").await.expect("seed");

        index_tree(&backend, "proj", "/moved.md", &client).await;
        let calls = wait_for(&rec, 1).await;
        assert_eq!(calls, vec!["index proj /moved.md 'arrived' 1000/100"]);
    }

    /// A tree move or copy must reach every file at every depth, not just the root.
    #[tokio::test]
    async fn index_tree_indexes_every_file_of_a_subtree() {
        let (backend, rec) = recording();
        let client = client().await;
        client.write_text_atomic("/dst/a.md", "alpha").await.expect("seed");
        client.write_text_atomic("/dst/deep/b.md", "beta").await.expect("seed");

        index_tree(&backend, "proj", "/dst", &client).await;
        let calls = wait_for(&rec, 2).await;
        assert!(calls.iter().any(|c| c.contains("index proj /dst/a.md 'alpha'")), "{calls:?}");
        assert!(calls.iter().any(|c| c.contains("index proj /dst/deep/b.md 'beta'")), "{calls:?}");
    }

    /// A binary that lands at the destination is skipped, exactly as a written one is.
    #[tokio::test]
    async fn index_tree_skips_a_non_utf8_file() {
        let (backend, rec) = recording();
        let client = client().await;
        client.write_text_atomic("/dst/good.md", "readable").await.expect("seed");
        client.write_bytes_atomic("/dst/bad.bin", &[0xFF, 0xFE, 0x00]).await.expect("seed");

        index_tree(&backend, "proj", "/dst", &client).await;
        let calls = wait_for(&rec, 1).await;
        assert_eq!(calls.len(), 1, "only the readable file is indexed: {calls:?}");
        assert!(calls[0].contains("/dst/good.md"), "{calls:?}");
    }

    /// A path that does not exist yields no index call, which is what a failed
    /// move or copy must leave behind.
    #[tokio::test]
    async fn index_tree_on_a_missing_path_indexes_nothing() {
        let (backend, rec) = recording();
        let client = client().await;
        index_tree(&backend, "proj", "/nowhere.md", &client).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(rec.calls().is_empty(), "{:?}", rec.calls());
    }

    /// The barrier in `after_move`, pinned: a path that is both cleared and
    /// re-indexed must see its delete STRICTLY before its index call. Both hooks
    /// detach, so an unordered version can land the delete last and drop the
    /// chunks of a file that still exists.
    #[tokio::test]
    async fn after_move_clears_a_path_before_reindexing_it() {
        let (state, client, rec) = indexing_state().await;
        client.write_text_atomic("/dst/a.md", "arrived").await.expect("seed");

        let displaced = vec!["/dst/a.md".to_string(), "/dst/gone.md".to_string()];
        after_move(&state, crate::tools::testkit::MOUNT, &displaced, "/dst", &client).await;

        let calls = wait_for(&rec, 3).await;
        let vol = crate::tools::testkit::MOUNT;
        let deleted = calls
            .iter()
            .position(|c| c == &format!("delete {vol} /dst/a.md"))
            .unwrap_or_else(|| panic!("the clear must have run: {calls:?}"));
        let indexed = calls
            .iter()
            .position(|c| c.starts_with(&format!("index {vol} /dst/a.md")))
            .unwrap_or_else(|| panic!("the re-index must have run: {calls:?}"));
        assert!(deleted < indexed, "the clear must precede the re-index: {calls:?}");
        assert!(
            calls.contains(&format!("delete {vol} /dst/gone.md")),
            "a path with no counterpart must still be cleared: {calls:?}"
        );
    }

    // ── fixtures ────────────────────────────────────────────────────────────

    /// An `AppState` whose search backend is the recorder and whose project has
    /// an active mode, so the `after_*` hooks actually fire.
    async fn indexing_state() -> (Arc<AppState>, Arc<VolumeClient>, Arc<RecordingBackend>) {
        let (backend, rec) = with_delete_delay(std::time::Duration::from_millis(150));
        let h = crate::tools::testkit::harness_with_search(|_| {}, Some(backend)).await;
        let state = h.state.clone();
        let client = h.client().await;
        state
            .admin
            .set_index_mode(crate::tools::testkit::MOUNT, IndexMode::Bm25)
            .await
            .expect("the mode must be settable");
        // The harness owns the temp dir; leaking it keeps the volume alive for the
        // length of the test, which is what a fixture is for.
        std::mem::forget(h);
        (state, client, rec)
    }

    /// A volume client over a temp dir, to drive `full_index` against real files.
    async fn client() -> Arc<VolumeClient> {
        let h = crate::tools::testkit::harness().await;
        let c = h.client().await;
        // The harness owns the temp dir; leaking it keeps the volume alive for
        // the length of the test, which is what a fixture is for.
        std::mem::forget(h);
        c
    }

    /// Poll until the recorder has at least `n` calls, then return them.
    /// Fails the test rather than hanging when the detached task never runs.
    async fn wait_for(rec: &Arc<RecordingBackend>, n: usize) -> Vec<String> {
        for _ in 0..200 {
            let calls = rec.calls();
            if calls.len() >= n {
                return calls;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {n} backend calls, saw {:?}", rec.calls());
    }
}
