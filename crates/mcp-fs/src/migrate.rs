//! Offline migration of relational state between two backends.
//!
//! `mcp-fs migrate --from a.yaml --to b.yaml` moves every row the server owns from
//! the engines one config names to the engines the other names. Without it,
//! switching `infra.meta.backend` to PostgreSQL would mean abandoning the existing
//! data, so the config flag would not be usable on a deployment that has any.
//!
//! # Why rows and not operations
//!
//! The copy is a row for row transfer at the [`RelationalDb`] level rather than a
//! replay through the store APIs. Replaying `put_file` would stamp fresh
//! timestamps, silently losing every `mtime` and `ctime`, and it would have to
//! reconstruct directory order. Copying rows preserves each value exactly, and the
//! schema is declared once per store so both sides agree on the columns whatever
//! the engine.
//!
//! # What is not migrated, and why
//!
//! * **Blob bytes** stay where they are. The blob store is configured separately
//!   (`infra.blob`) and is unaffected by a relational backend change, so copying
//!   it would be a different operation. Every `sha256` a node refers to is checked
//!   against the destination blob store and reported when missing, which is what
//!   turns a silent dangling reference into a visible one.
//! * **OAuth tokens** are session state, encrypted with `MCPFS_TOKEN_KEY`. They
//!   are deliberately skipped: a device flow re establishes them, and copying
//!   ciphertext across a deployment whose key may differ would produce rows that
//!   never decrypt.
//!
//! # This is offline
//!
//! Run it with the server stopped. It takes no locks against a live writer, so a
//! concurrent write during the copy would be missed.

use crate::config::ServerConfig;
use crate::errors::{Result, ToolError};
use crate::storage::rel::{Query, RelationalDb, SchemaSet, SqlValue};
use crate::storage::{RelationalRegistry, build_blob_store};
use std::collections::BTreeSet;

/// How many rows travel in one INSERT batch.
///
/// SQL Server caps a statement at 2100 parameters, and our widest table binds ten
/// columns, so 100 rows stays clear of that ceiling on every engine.
const BATCH_ROWS: usize = 100;

/// What one table copy did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableReport {
    pub table: String,
    pub rows: u64,
}

/// What the whole migration did, printed by the CLI and asserted by tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub tables: Vec<TableReport>,
    pub projects: Vec<String>,
    /// Shas referenced by a migrated node but absent from the destination blob
    /// store. Non empty means the destination is missing bytes, not rows.
    pub missing_blobs: Vec<String>,
}

impl MigrationReport {
    pub fn total_rows(&self) -> u64 {
        self.tables.iter().map(|t| t.rows).sum()
    }

    fn record(&mut self, table: &str, rows: u64) {
        // One entry per table, so a per project copy accumulates instead of
        // reporting the same table once per project.
        if let Some(existing) = self.tables.iter_mut().find(|t| t.table == table) {
            existing.rows += rows;
        } else {
            self.tables.push(TableReport { table: table.to_string(), rows });
        }
    }
}

/// Every column of a table, in declaration order, so both sides agree.
fn columns_of(schema: &SchemaSet, table: &str) -> Result<Vec<&'static str>> {
    schema
        .tables
        .iter()
        .find(|t| t.name == table)
        .map(|t| t.columns.iter().map(|c| c.name).collect())
        .ok_or_else(|| ToolError::internal(format!("no table '{table}' in the declared schema")))
}

/// Copy every row of one table, optionally restricted to one volume.
///
/// The destination is emptied of that scope first, so a re run is idempotent
/// rather than producing duplicate key errors half way through.
async fn copy_table(
    src: &dyn RelationalDb,
    dst: &dyn RelationalDb,
    table: &str,
    columns: &[&'static str],
    volume: Option<&str>,
) -> Result<u64> {
    let col_list =
        columns.iter().map(|c| src.dialect().quote_ident(c)).collect::<Vec<_>>().join(", ");

    let (select, clear) = match volume {
        Some(_) => (
            format!("SELECT {col_list} FROM {table} WHERE volume_id=?1"),
            format!("DELETE FROM {table} WHERE volume_id=?1"),
        ),
        None => (format!("SELECT {col_list} FROM {table}"), format!("DELETE FROM {table}")),
    };

    let mut select_q = Query::new(select);
    let mut clear_q = Query::new(clear);
    if let Some(v) = volume {
        select_q = select_q.bind(v);
        clear_q = clear_q.bind(v);
    }

    let rows = src.query(&select_q).await?;

    // One transaction for the clear plus every batch: a partial table would leave
    // the destination with a subset that looks complete.
    let mut tx = dst.begin().await?;
    tx.execute(&clear_q).await?;

    let dst_cols =
        columns.iter().map(|c| dst.dialect().quote_ident(c)).collect::<Vec<_>>().join(", ");

    let mut written = 0u64;
    for chunk in rows.chunks(BATCH_ROWS) {
        let mut placeholders = Vec::with_capacity(chunk.len());
        let mut params: Vec<SqlValue> = Vec::with_capacity(chunk.len() * columns.len());
        let mut next = 1usize;
        for row in chunk {
            let mut group = Vec::with_capacity(columns.len());
            for i in 0..columns.len() {
                group.push(format!("?{next}"));
                next += 1;
                params.push(row.value(i)?.clone());
            }
            placeholders.push(format!("({})", group.join(", ")));
        }
        let sql = format!("INSERT INTO {table} ({dst_cols}) VALUES {}", placeholders.join(", "));
        let mut q = Query::new(sql);
        for p in params {
            q = q.bind(p);
        }
        written += tx.execute(&q).await?;
    }
    tx.commit().await?;
    Ok(written)
}

/// Count the rows of one table scope, for the post copy verification.
async fn count_rows(db: &dyn RelationalDb, table: &str, volume: Option<&str>) -> Result<i64> {
    let q = match volume {
        Some(v) => Query::new(format!("SELECT COUNT(*) FROM {table} WHERE volume_id=?1")).bind(v),
        None => Query::new(format!("SELECT COUNT(*) FROM {table}")),
    };
    db.query_opt(&q)
        .await?
        .ok_or_else(|| ToolError::internal(format!("COUNT(*) on {table} returned no row")))?
        .i64(0)
}

/// Every distinct sha a file node in one volume refers to.
async fn shas_of(db: &dyn RelationalDb, volume: &str) -> Result<BTreeSet<String>> {
    let rows = db
        .query(
            &Query::new("SELECT sha256 FROM nodes WHERE volume_id=?1 AND sha256 IS NOT NULL")
                .bind(volume),
        )
        .await?;
    let mut out = BTreeSet::new();
    for r in &rows {
        if let Some(s) = r.opt_text(0)? {
            out.insert(s);
        }
    }
    Ok(out)
}

/// Move every relational row from the `from` deployment to the `to` deployment.
pub async fn migrate(from: &ServerConfig, to: &ServerConfig) -> Result<MigrationReport> {
    let src_registry = RelationalRegistry::new();
    let dst_registry = RelationalRegistry::new();
    let mut report = MigrationReport::default();

    // ── the ACL registry, which also tells us which volumes exist ──────────────
    let src_admin_db = crate::storage::open_admin_db(from, &src_registry).await?;
    let dst_admin_db = crate::storage::open_admin_db(to, &dst_registry).await?;
    let admin_schema = crate::storage::admin::schema();
    // Both sides: the schema is applied on every normal store open, so this is the
    // same idempotent DDL. Without it a source that never opened a given store has
    // no table to read, and a project that never used git has no git tables.
    src_admin_db.migrate(&admin_schema).await?;
    dst_admin_db.migrate(&admin_schema).await?;

    // `project` before `project_member`: the member rows carry a foreign key onto
    // the project, so the other order fails on an engine that enforces it.
    for table in ["project", "project_member"] {
        let cols = columns_of(&admin_schema, table)?;
        let n = copy_table(&*src_admin_db, &*dst_admin_db, table, &cols, None).await?;
        report.record(table, n);

        let want = count_rows(&*src_admin_db, table, None).await?;
        let got = count_rows(&*dst_admin_db, table, None).await?;
        if want != got {
            return Err(ToolError::internal(format!(
                "migration of '{table}' is incomplete: source has {want} rows, destination {got}"
            )));
        }
    }

    let projects = crate::storage::build_admin_store(from, &src_registry).await?;
    projects.connect().await?;
    let project_ids: Vec<String> =
        projects.list_all_projects().await?.into_iter().map(|p| p.id).collect();

    // ── per volume state ──────────────────────────────────────────────────────
    let meta_schema = crate::storage::meta::schema();
    let git_schema = crate::git::db::schema();

    for project_id in &project_ids {
        report.projects.push(project_id.clone());

        let src_meta = crate::storage::open_meta_db(from, &src_registry, project_id).await?;
        let dst_meta = crate::storage::open_meta_db(to, &dst_registry, project_id).await?;
        src_meta.migrate(&meta_schema).await?;
        dst_meta.migrate(&meta_schema).await?;

        // `nodes` before `blob_refs` only for readability: they are independent.
        for table in ["nodes", "blob_refs"] {
            let cols = columns_of(&meta_schema, table)?;
            let n = copy_table(&*src_meta, &*dst_meta, table, &cols, Some(project_id)).await?;
            report.record(table, n);

            let want = count_rows(&*src_meta, table, Some(project_id)).await?;
            let got = count_rows(&*dst_meta, table, Some(project_id)).await?;
            if want != got {
                return Err(ToolError::internal(format!(
                    "migration of '{table}' for project '{project_id}' is incomplete: \
                     source has {want} rows, destination {got}"
                )));
            }
        }

        // The shas must survive exactly: a file whose content address changed would
        // read as a different file, which a row count cannot detect.
        let want_shas = shas_of(&*src_meta, project_id).await?;
        let got_shas = shas_of(&*dst_meta, project_id).await?;
        if want_shas != got_shas {
            return Err(ToolError::internal(format!(
                "migration changed the content addresses of project '{project_id}': \
                 {} shas before, {} after",
                want_shas.len(),
                got_shas.len()
            )));
        }

        // Bytes are not migrated, so a missing blob is reported rather than fixed.
        let dst_blobs = build_blob_store(to, project_id)?;
        for sha in &want_shas {
            if !dst_blobs.exists(sha).await? {
                report.missing_blobs.push(sha.clone());
            }
        }

        // The git index only exists once a project has used git.
        let src_git = crate::storage::open_git_db(from, &src_registry, project_id).await?;
        let dst_git = crate::storage::open_git_db(to, &dst_registry, project_id).await?;
        src_git.migrate(&git_schema).await?;
        dst_git.migrate(&git_schema).await?;
        for table in crate::git::db::TABLES {
            let cols = columns_of(&git_schema, table)?;
            let n = copy_table(&*src_git, &*dst_git, table, &cols, Some(project_id)).await?;
            report.record(table, n);
        }
    }

    Ok(report)
}

/// Open both configs, run the migration and print what happened.
pub async fn run(from: &ServerConfig, to: &ServerConfig) -> Result<MigrationReport> {
    let report = migrate(from, to).await?;

    // stderr: a caller piping stdout gets the machine readable summary only.
    eprintln!("migrated {} projects", report.projects.len());
    for t in &report.tables {
        eprintln!("  {:<16} {} rows", t.table, t.rows);
    }
    if !report.missing_blobs.is_empty() {
        eprintln!(
            "WARNING: {} content addresses have no bytes in the destination blob store.",
            report.missing_blobs.len()
        );
        eprintln!("         Relational rows moved, but infra.blob still points elsewhere.");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Two independent SQLite deployments under one temp dir. SQLite to SQLite is
    /// the only pairing that runs without a server, and it exercises every line of
    /// the copy: the engines are chosen from config exactly as in production.
    fn deployment(root: &std::path::Path, name: &str) -> ServerConfig {
        let mut c = ServerConfig::default();
        c.infra.meta.dir = root.join(name).join("volumes").display().to_string();
        c.infra.blob.dir = root.join(name).join("blobs").display().to_string();
        c.infra.admin.path = root.join(name).join("admin.db").display().to_string();
        c
    }

    #[tokio::test]
    async fn migrates_tree_sizes_shas_and_acls() {
        let tmp = tempfile::tempdir().unwrap();
        let from = deployment(tmp.path(), "src");
        let to = deployment(tmp.path(), "dst");

        // ── build a source deployment with real content ────────────────────────
        let src_reg = RelationalRegistry::new();
        let admin = crate::storage::build_admin_store(&from, &src_reg).await.unwrap();
        admin.connect().await.unwrap();
        admin.create_project("proj-a", "Owner@Example.com").await.unwrap();
        admin.add_member("proj-a", "Member@Example.com", "owner@example.com").await.unwrap();
        admin.create_project("proj-b", "other@example.com").await.unwrap();

        let stores = crate::storage::StoreManager::new(
            Arc::new(from.clone()),
            crate::storage::test_registry(),
        );
        let a = stores.client("proj-a").await.unwrap();
        a.write_text_atomic("/dir/one.txt", "hello").await.unwrap();
        a.write_text_atomic("/dir/two.txt", "world").await.unwrap();
        // Same bytes twice, so blob_refs carries a refcount above one.
        a.write_text_atomic("/dir/dup.txt", "hello").await.unwrap();
        let b = stores.client("proj-b").await.unwrap();
        b.write_text_atomic("/only-b.txt", "b").await.unwrap();

        let before_a = a.meta.subtree("/").await.unwrap();

        // ── migrate ───────────────────────────────────────────────────────────
        let report = migrate(&from, &to).await.unwrap();
        assert_eq!(report.projects.len(), 2, "both projects were seen");
        assert!(report.total_rows() > 0);

        // ── the destination must present the same volume ──────────────────────
        let dst_reg = RelationalRegistry::new();
        let dst_admin = crate::storage::build_admin_store(&to, &dst_reg).await.unwrap();
        dst_admin.connect().await.unwrap();

        let mut ids: Vec<String> =
            dst_admin.list_all_projects().await.unwrap().into_iter().map(|p| p.id).collect();
        ids.sort();
        assert_eq!(ids, ["proj-a", "proj-b"], "every project moved");

        let owner = dst_admin.get_project("proj-a").await.unwrap().expect("proj-a exists");
        assert_eq!(owner.owner, "owner@example.com", "the owner is preserved, lowercased");
        assert!(
            dst_admin.is_member("proj-a", "member@example.com").await.unwrap(),
            "membership moved, so the ACL still holds"
        );
        assert!(
            !dst_admin.is_member("proj-a", "stranger@example.com").await.unwrap(),
            "migration must not widen an ACL"
        );

        let dst_meta = crate::storage::build_meta_store(&to, &dst_reg, "proj-a").await.unwrap();
        let after_a = dst_meta.subtree("/").await.unwrap();
        assert_eq!(
            before_a, after_a,
            "every node matches: path, kind, size, mode, timestamps and sha"
        );

        // Isolation must survive the copy: proj-b's file cannot appear in proj-a.
        assert!(dst_meta.get("/only-b.txt").await.unwrap().is_none());
        let dst_b = crate::storage::build_meta_store(&to, &dst_reg, "proj-b").await.unwrap();
        assert!(dst_b.get("/only-b.txt").await.unwrap().is_some());
        assert!(dst_b.get("/dir/one.txt").await.unwrap().is_none());
    }

    /// Bytes are not copied, so a destination pointing at an empty blob store must
    /// say so rather than leaving a dangling reference for a reader to discover.
    #[tokio::test]
    async fn reports_content_addresses_with_no_bytes_at_the_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let from = deployment(tmp.path(), "src");
        let mut to = deployment(tmp.path(), "dst");
        // A different blob root: the rows move, the bytes do not.
        to.infra.blob.dir = tmp.path().join("elsewhere").display().to_string();

        let reg = RelationalRegistry::new();
        let admin = crate::storage::build_admin_store(&from, &reg).await.unwrap();
        admin.connect().await.unwrap();
        admin.create_project("proj", "o@e.com").await.unwrap();
        let stores = crate::storage::StoreManager::new(
            Arc::new(from.clone()),
            crate::storage::test_registry(),
        );
        stores.client("proj").await.unwrap().write_text_atomic("/f.txt", "x").await.unwrap();

        let report = migrate(&from, &to).await.unwrap();
        assert_eq!(report.missing_blobs.len(), 1, "the one sha has no bytes at the destination");
    }

    /// Running twice must not double the rows or fail on a duplicate key: an
    /// interrupted migration has to be safe to restart.
    #[tokio::test]
    async fn is_idempotent_when_run_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let from = deployment(tmp.path(), "src");
        let to = deployment(tmp.path(), "dst");

        let reg = RelationalRegistry::new();
        let admin = crate::storage::build_admin_store(&from, &reg).await.unwrap();
        admin.connect().await.unwrap();
        admin.create_project("proj", "o@e.com").await.unwrap();
        let stores = crate::storage::StoreManager::new(
            Arc::new(from.clone()),
            crate::storage::test_registry(),
        );
        let c = stores.client("proj").await.unwrap();
        c.write_text_atomic("/a.txt", "a").await.unwrap();
        c.write_text_atomic("/b.txt", "b").await.unwrap();

        let first = migrate(&from, &to).await.unwrap();
        let second = migrate(&from, &to).await.unwrap();
        assert_eq!(first.total_rows(), second.total_rows(), "a re run copies the same rows");

        let dst_reg = RelationalRegistry::new();
        let dst_meta = crate::storage::build_meta_store(&to, &dst_reg, "proj").await.unwrap();
        let nodes = dst_meta.subtree("/").await.unwrap();
        let files = nodes.iter().filter(|n| n.is_file()).count();
        assert_eq!(files, 2, "no duplicate rows after a second run");
    }

    #[tokio::test]
    async fn an_empty_deployment_migrates_to_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let from = deployment(tmp.path(), "src");
        let to = deployment(tmp.path(), "dst");
        let report = migrate(&from, &to).await.unwrap();
        assert!(report.projects.is_empty());
        assert_eq!(report.total_rows(), 0);
        assert!(report.missing_blobs.is_empty());
    }

    /// A file written after the migration must not disturb the copied rows, which
    /// is what proves the destination is a working deployment and not just a dump.
    #[tokio::test]
    async fn the_destination_is_writable_after_a_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let from = deployment(tmp.path(), "src");
        let mut to = deployment(tmp.path(), "dst");
        // The realistic move: the relational backend changes, the blob store does
        // not. Reading a migrated file needs its bytes, which only the shared blob
        // store has, because a migration copies rows and never bytes.
        to.infra.blob.dir = from.infra.blob.dir.clone();

        let reg = RelationalRegistry::new();
        let admin = crate::storage::build_admin_store(&from, &reg).await.unwrap();
        admin.connect().await.unwrap();
        admin.create_project("proj", "o@e.com").await.unwrap();
        crate::storage::StoreManager::new(Arc::new(from.clone()), crate::storage::test_registry())
            .client("proj")
            .await
            .unwrap()
            .write_text_atomic("/kept.txt", "kept")
            .await
            .unwrap();

        migrate(&from, &to).await.unwrap();

        let dst_stores = crate::storage::StoreManager::new(
            Arc::new(to.clone()),
            crate::storage::test_registry(),
        );
        let c = dst_stores.client("proj").await.unwrap();
        c.write_text_atomic("/new.txt", "new").await.unwrap();
        assert_eq!(c.read_text("/kept.txt").await.unwrap(), "kept");
        assert_eq!(c.read_text("/new.txt").await.unwrap(), "new");

        // The migrated node and the new one share a volume, so put_file had to
        // accept a tree it did not create.
        assert!(c.exists("/kept.txt").await.unwrap());
        let mut paths: Vec<String> = dst_stores
            .client("proj")
            .await
            .unwrap()
            .meta
            .subtree("/")
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.is_file())
            .map(|n| n.path)
            .collect();
        paths.sort();
        assert_eq!(paths, ["/kept.txt", "/new.txt"]);
    }
}
