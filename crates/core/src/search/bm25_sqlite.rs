//! Tantivy on-disk BM25 backend for the SQLite/dev deployment posture.
//!
//! Each volume gets its own Tantivy index directory under `{tantivy_dir}/{volume_id}/`.
//! The index is opened on first use and cached behind an `Arc<RwLock<...>>` for the
//! lifetime of the process. It is ephemeral local state: a pod restart or volume
//! remount empties it, and `search.status` reports `bm25_warm: false` so the caller
//! knows to re-index.
//!
//! All Tantivy I/O is executed inside `tokio::task::spawn_blocking` so it never
//! blocks the async executor.

use crate::errors::{Result, ToolError};
use crate::search::{IndexStats, SearchBackend, SearchResult};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::QueryParser;
use tantivy::schema::{FAST, STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyDocument, Term};

/// A cached, open Tantivy index for one volume.
struct VolumeIndex {
    index: Index,
    schema: Schema,
    /// Serializes the writers of this volume.
    ///
    /// Tantivy allows ONE `IndexWriter` per index and rejects a second with a
    /// lock error. Auto indexing makes overlapping writers the normal case (two
    /// writes a moment apart each spawn their own detached index task), and the
    /// loser would silently drop a document, so they queue here instead.
    write_lock: tokio::sync::Mutex<()>,
}

/// BM25 backend backed by one Tantivy directory per volume.
pub struct TantivyBm25Backend {
    tantivy_dir: PathBuf,
    /// Lazily opened, one entry per volume that has been accessed.
    cache: Arc<RwLock<HashMap<String, Arc<VolumeIndex>>>>,
}

impl TantivyBm25Backend {
    pub fn new(tantivy_dir: &str) -> Self {
        Self {
            tantivy_dir: PathBuf::from(tantivy_dir),
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Return (or open/create) the Tantivy index for one volume.
    fn get_or_open_volume(&self, volume_id: &str) -> Result<Arc<VolumeIndex>> {
        // Fast path: already cached.
        {
            let guard = self
                .cache
                .read()
                .map_err(|_| ToolError::internal("tantivy cache rwlock poisoned"))?;
            if let Some(vi) = guard.get(volume_id) {
                return Ok(vi.clone());
            }
        }

        // Slow path: open or create the index.
        let dir = self.tantivy_dir.join(volume_id);
        std::fs::create_dir_all(&dir)?;

        let mut builder = Schema::builder();
        // path is stored and indexed as a keyword (STRING) so we can delete by path term.
        builder.add_text_field("path", STRING | STORED);
        // chunk_idx is a fast u64 field, stored so we can include it in results.
        builder.add_u64_field("chunk_idx", FAST | STORED);
        // chunk_text is indexed (for BM25) and stored (for snippet retrieval).
        builder.add_text_field("chunk_text", TEXT | STORED);
        let schema = builder.build();

        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(&dir)
                .map_err(|e| ToolError::internal(format!("tantivy: open directory: {e}")))?,
            schema.clone(),
        )
        .map_err(|e| ToolError::internal(format!("tantivy: open or create index: {e}")))?;

        let vi = Arc::new(VolumeIndex { index, schema, write_lock: tokio::sync::Mutex::new(()) });

        let mut guard =
            self.cache.write().map_err(|_| ToolError::internal("tantivy cache rwlock poisoned"))?;
        // Another thread may have raced to insert the same volume; return whichever won.
        Ok(guard.entry(volume_id.to_string()).or_insert(vi).clone())
    }

    /// Path to the on-disk index directory for a volume.
    fn index_dir(&self, volume_id: &str) -> PathBuf {
        self.tantivy_dir.join(volume_id)
    }
}

#[async_trait]
impl SearchBackend for TantivyBm25Backend {
    async fn index_path(
        &self,
        volume_id: &str,
        path: &str,
        text: &str,
        chunk_size: usize,
        chunk_overlap: usize,
    ) -> Result<usize> {
        let backend = self.get_or_open_volume(volume_id)?;
        let chunks = crate::search::chunker::chunk(text, chunk_size, chunk_overlap);
        let n = chunks.len();
        let path_owned = path.to_string();
        let _writing = backend.write_lock.lock().await;

        tokio::task::spawn_blocking({
            let backend = backend.clone();
            move || {
                let path_field = backend
                    .schema
                    .get_field("path")
                    .map_err(|e| ToolError::internal(format!("tantivy: get path field: {e}")))?;
                let chunk_idx_field = backend.schema.get_field("chunk_idx").map_err(|e| {
                    ToolError::internal(format!("tantivy: get chunk_idx field: {e}"))
                })?;
                let chunk_text_field = backend.schema.get_field("chunk_text").map_err(|e| {
                    ToolError::internal(format!("tantivy: get chunk_text field: {e}"))
                })?;

                let mut writer: IndexWriter = backend
                    .index
                    .writer(50_000_000)
                    .map_err(|e| ToolError::internal(format!("tantivy: create writer: {e}")))?;

                // Idempotent: delete existing chunks for this path before inserting.
                writer.delete_term(Term::from_field_text(path_field, &path_owned));

                for (idx, chunk_text) in chunks.iter().enumerate() {
                    let mut doc = TantivyDocument::default();
                    doc.add_text(path_field, &path_owned);
                    doc.add_u64(chunk_idx_field, idx as u64);
                    doc.add_text(chunk_text_field, chunk_text);
                    writer
                        .add_document(doc)
                        .map_err(|e| ToolError::internal(format!("tantivy: add document: {e}")))?;
                }

                writer
                    .commit()
                    .map_err(|e| ToolError::internal(format!("tantivy: commit: {e}")))?;
                Ok(n)
            }
        })
        .await
        .map_err(|e| ToolError::internal(format!("tantivy task join: {e}")))?
    }

    async fn delete_path(&self, volume_id: &str, path: &str) -> Result<usize> {
        let backend = self.get_or_open_volume(volume_id)?;
        let path_owned = path.to_string();
        let _writing = backend.write_lock.lock().await;

        tokio::task::spawn_blocking({
            let backend = backend.clone();
            move || {
                let path_field = backend
                    .schema
                    .get_field("path")
                    .map_err(|e| ToolError::internal(format!("tantivy: get path field: {e}")))?;

                // Count before deleting so we can return the count.
                let reader = backend
                    .index
                    .reader_builder()
                    .reload_policy(ReloadPolicy::Manual)
                    .try_into()
                    .map_err(|e| ToolError::internal(format!("tantivy: build reader: {e}")))?;
                let searcher = reader.searcher();
                let term = Term::from_field_text(path_field, &path_owned);
                let term_query = tantivy::query::TermQuery::new(
                    term.clone(),
                    tantivy::schema::IndexRecordOption::Basic,
                );
                let count = searcher
                    .search(&term_query, &Count)
                    .map_err(|e| ToolError::internal(format!("tantivy: count: {e}")))?;

                let mut writer: IndexWriter = backend
                    .index
                    .writer(50_000_000)
                    .map_err(|e| ToolError::internal(format!("tantivy: create writer: {e}")))?;
                writer.delete_term(term);
                writer
                    .commit()
                    .map_err(|e| ToolError::internal(format!("tantivy: commit delete: {e}")))?;

                Ok(count)
            }
        })
        .await
        .map_err(|e| ToolError::internal(format!("tantivy task join: {e}")))?
    }

    async fn delete_all(&self, volume_id: &str) -> Result<usize> {
        // Counted before the directory goes, because afterwards there is nothing
        // left to count.
        let count = self.stats(volume_id).await?.bm25_docs;

        // Behind the same lock as the writers, so an in flight index_path cannot
        // commit into a directory that is about to be removed.
        let volume = self.get_or_open_volume(volume_id)?;
        let _writing = volume.write_lock.lock().await;

        // Dropping the whole directory beats deleting every term: Tantivy would
        // keep the deleted segments until a merge, and the directory is always
        // rebuildable from the volume.
        {
            let mut guard = self
                .cache
                .write()
                .map_err(|_| ToolError::internal("tantivy cache rwlock poisoned"))?;
            // The cached Index still points at the removed directory, so it must
            // go with it or the next write would land in a ghost.
            guard.remove(volume_id);
        }
        let dir = self.index_dir(volume_id);
        if dir.exists() {
            let dir_owned = dir.clone();
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&dir_owned))
                .await
                .map_err(|e| ToolError::internal(format!("tantivy task join: {e}")))??;
        }
        Ok(count)
    }

    async fn query_bm25(
        &self,
        volume_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let backend = self.get_or_open_volume(volume_id)?;
        let query_owned = query.to_string();

        tokio::task::spawn_blocking(move || {
            let path_field = backend
                .schema
                .get_field("path")
                .map_err(|e| ToolError::internal(format!("tantivy: get path field: {e}")))?;
            let chunk_text_field = backend
                .schema
                .get_field("chunk_text")
                .map_err(|e| ToolError::internal(format!("tantivy: get chunk_text field: {e}")))?;

            let reader = backend
                .index
                .reader_builder()
                .reload_policy(ReloadPolicy::OnCommitWithDelay)
                .try_into()
                .map_err(|e| ToolError::internal(format!("tantivy: build reader: {e}")))?;
            let searcher = reader.searcher();
            let query_parser = QueryParser::for_index(&backend.index, vec![chunk_text_field]);
            let parsed = query_parser
                .parse_query(&query_owned)
                .map_err(|e| ToolError::internal(format!("tantivy: parse query: {e}")))?;

            let top_docs = searcher
                .search(&parsed, &TopDocs::with_limit(top_k).order_by_score())
                .map_err(|e| ToolError::internal(format!("tantivy: search: {e}")))?;

            let mut results = Vec::with_capacity(top_docs.len());
            for (score, doc_addr) in top_docs {
                let doc: TantivyDocument = searcher
                    .doc(doc_addr)
                    .map_err(|e| ToolError::internal(format!("tantivy: retrieve doc: {e}")))?;

                let path =
                    doc.get_first(path_field).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let chunk = doc
                    .get_first(chunk_text_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                results.push(SearchResult { path, score, chunk, rank: results.len() + 1 });
            }
            // Re-number ranks after collecting.
            for (i, r) in results.iter_mut().enumerate() {
                r.rank = i + 1;
            }
            Ok(results)
        })
        .await
        .map_err(|e| ToolError::internal(format!("tantivy task join: {e}")))?
    }

    async fn query_vector(
        &self,
        _volume_id: &str,
        _query: &str,
        _top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        Err(ToolError::not_supported(
            "vector search is not available in bm25 mode; use mode=rag or mode=both with the rag feature",
        ))
    }

    async fn stats(&self, volume_id: &str) -> Result<IndexStats> {
        let dir = self.index_dir(volume_id);
        let warm =
            dir.exists() && dir.read_dir().ok().map(|mut d| d.next().is_some()).unwrap_or(false);

        if !warm {
            return Ok(IndexStats {
                bm25_docs: 0,
                vector_chunks: 0,
                bm25_warm: false,
                mode: "bm25".into(),
            });
        }

        let backend = self.get_or_open_volume(volume_id)?;
        let count = tokio::task::spawn_blocking(move || {
            let reader = backend
                .index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()
                .map_err(|e| ToolError::internal(format!("tantivy: build reader: {e}")))?;
            let searcher = reader.searcher();
            Ok::<usize, ToolError>(searcher.num_docs() as usize)
        })
        .await
        .map_err(|e| ToolError::internal(format!("tantivy task join: {e}")))??;

        Ok(IndexStats { bm25_docs: count, vector_chunks: 0, bm25_warm: true, mode: "bm25".into() })
    }

    fn supported_modes(&self) -> Vec<&'static str> {
        vec!["bm25"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchBackend;

    async fn backend() -> (TantivyBm25Backend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let b = TantivyBm25Backend::new(dir.path().to_str().unwrap());
        (b, dir)
    }

    #[tokio::test]
    async fn index_and_query_returns_results() {
        let (b, _d) = backend().await;
        b.index_path("vol1", "/a.md", "the quick brown fox jumps over the lazy dog", 200, 0)
            .await
            .unwrap();
        let results = b.query_bm25("vol1", "fox", 10).await.unwrap();
        assert!(!results.is_empty(), "expected at least one result");
        assert_eq!(results[0].path, "/a.md");
    }

    #[tokio::test]
    async fn delete_removes_doc_from_results() {
        let (b, _d) = backend().await;
        b.index_path("vol1", "/a.md", "the quick brown fox jumps over the lazy dog", 200, 0)
            .await
            .unwrap();
        b.delete_path("vol1", "/a.md").await.unwrap();
        let results = b.query_bm25("vol1", "fox", 10).await.unwrap();
        assert!(results.is_empty(), "deleted doc must not appear in results");
    }

    #[tokio::test]
    async fn delete_all_removes_every_chunk_of_the_volume() {
        let (b, _d) = backend().await;
        b.index_path("vol1", "/a.md", "alpha content about foxes", 200, 0).await.unwrap();
        b.index_path("vol1", "/b.md", "beta content about foxes", 200, 0).await.unwrap();
        assert_eq!(b.stats("vol1").await.unwrap().bm25_docs, 2);

        let deleted = b.delete_all("vol1").await.unwrap();
        assert_eq!(deleted, 2, "delete_all must report what it removed");

        let s = b.stats("vol1").await.unwrap();
        assert_eq!(s.bm25_docs, 0);
        assert!(!s.bm25_warm, "the index directory is gone");
        assert!(b.query_bm25("vol1", "foxes", 10).await.unwrap().is_empty());

        // The volume must be usable again straight after the wipe.
        b.index_path("vol1", "/c.md", "gamma content about foxes", 200, 0).await.unwrap();
        assert_eq!(b.stats("vol1").await.unwrap().bm25_docs, 1);
    }

    /// Two writers on one Tantivy index collide on its lock file. Auto indexing
    /// makes that the normal case, and the loser used to drop its document.
    #[tokio::test]
    async fn concurrent_index_calls_on_one_volume_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let b = Arc::new(TantivyBm25Backend::new(dir.path().to_str().unwrap()));

        let mut tasks = Vec::new();
        for i in 0..5u32 {
            let b = b.clone();
            tasks.push(tokio::task::spawn(async move {
                b.index_path("vol1", &format!("/f{i}.md"), "shared marker text", 200, 0)
                    .await
                    .expect("a concurrent index call must not fail")
            }));
        }
        for t in tasks {
            t.await.expect("the task must not panic");
        }

        assert_eq!(
            b.stats("vol1").await.unwrap().bm25_docs,
            5,
            "every concurrent write must reach the index"
        );
    }

    #[tokio::test]
    async fn delete_all_is_scoped_to_one_volume() {
        let (b, _d) = backend().await;
        b.index_path("vol_a", "/x.md", "content a", 200, 0).await.unwrap();
        b.index_path("vol_b", "/x.md", "content b", 200, 0).await.unwrap();
        b.delete_all("vol_a").await.unwrap();
        assert_eq!(b.stats("vol_a").await.unwrap().bm25_docs, 0);
        assert_eq!(b.stats("vol_b").await.unwrap().bm25_docs, 1, "the other volume survives");
    }

    /// A wipe of a volume that was never indexed is a no op, not an error: the
    /// mode change path calls it unconditionally.
    #[tokio::test]
    async fn delete_all_on_an_empty_volume_is_zero() {
        let (b, _d) = backend().await;
        assert_eq!(b.delete_all("never-seen").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn stats_warm_after_indexing() {
        let (b, _d) = backend().await;
        let s0 = b.stats("vol1").await.unwrap();
        assert!(!s0.bm25_warm, "fresh volume should not be warm");

        b.index_path("vol1", "/a.md", "hello world", 200, 0).await.unwrap();
        let s1 = b.stats("vol1").await.unwrap();
        assert!(s1.bm25_warm);
        assert!(s1.bm25_docs > 0);
    }
}
