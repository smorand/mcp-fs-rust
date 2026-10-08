//! File-purge sweep (SPEC-0014 US-0005): soft-delete into the existing trash
//! every live file whose `atime` is older than its project's configured
//! `file_retention_days`.
//!
//! This module exposes a plain callable async function, nothing more: wiring it
//! to the background loop and the CLI is US-0007's job (see `Constraints` in the
//! story — `cli.rs` and `app.rs` are not touched here).

use crate::config::ServerConfig;
use crate::core::fs_ops::rename_with_collision_retry;
use crate::errors::{Result, ToolError};
use crate::git::GitRepoStore;
use crate::safety::SafetyManager;
use crate::storage::StoreManager;
use crate::storage::traits::AdminBackend;
use crate::storage::volume::VolumeClient;
use crate::util::now_unix;
use std::sync::Arc;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Soft-delete into `client`'s trash every live file older than its project's
/// configured `file_retention_days`. Returns the number of files purged.
///
/// A no-op (returns `Ok(0)`) whenever the project is not gated on, per
/// FR-NEW-007: `autopurge_enabled` false, `use_internal_purge` false, or
/// `file_retention_days` unset. Each file commits independently (DEC-013, no
/// locking): a failure soft-deleting one file is swallowed, never rolling back
/// or blocking any other file in the same sweep, because the next cycle picks
/// up anything missed.
pub async fn sweep_project_files(
    client: &VolumeClient,
    admin: &dyn AdminBackend,
    safety: &SafetyManager,
    project_id: &str,
) -> Result<usize> {
    sweep_project_files_at(now_unix(), client, admin, safety, project_id).await
}

/// Same as [`sweep_project_files`], but with an explicit `now` rather than
/// reading the wall clock, so a test can pin the staleness threshold to the
/// exact instant it seeded fixture data against (avoids wall-clock drift
/// between seeding and sweeping on a boundary test).
async fn sweep_project_files_at(
    now: f64,
    client: &VolumeClient,
    admin: &dyn AdminBackend,
    safety: &SafetyManager,
    project_id: &str,
) -> Result<usize> {
    let config = admin.get_purge_config(project_id).await?;
    if !config.autopurge_enabled || !config.use_internal_purge {
        return Ok(0);
    }
    let Some(days) = config.file_retention_days else {
        return Ok(0);
    };
    let threshold = now - (days as f64) * SECONDS_PER_DAY;
    let trash_root = format!("/{}", safety.config().trash_dir.trim_matches('/'));
    let stale = client.meta.stale_files(threshold, &trash_root).await?;

    let mut purged = 0usize;
    for node in stale {
        let dst = safety.trash_path(&node.path);
        let Some(slash) = dst.rfind('/') else { continue };
        let parent = &dst[..slash];
        // One commit per file: any failure here (mkdir, rename, or the
        // trash-entry insert) is swallowed so it never blocks or rolls back
        // the files before or after it (DEC-013). A failed insert additionally
        // renames the file back to its original path, so the rename and the
        // insert still appear atomic to an external observer even though they
        // are not one database transaction (SPEC-0011 US-0002's logged drift).
        if client.makedirs(parent, true).await.is_err() {
            continue;
        }
        let final_dst = match rename_with_collision_retry(client, &node.path, &dst).await {
            Ok(d) => d,
            Err(_) => continue,
        };
        if let Some(store) = &client.trash {
            let recorded = store
                .record_trash_entry(
                    &final_dst,
                    &node.path,
                    node.size,
                    &node.kind,
                    &crate::util::now_iso(),
                    None,
                )
                .await;
            if recorded.is_err() {
                let _ = client.rename(&final_dst, &node.path).await;
                continue;
            }
        }
        purged += 1;
    }
    Ok(purged)
}

/// Soft-delete `project_id` once it is stale, per FR-NEW-008: reference age is
/// `now - max(project.created_at, max(atime) over its live files)`, compared
/// strictly against `project_retention_days`. Returns whether THIS call
/// soft-deleted the project.
///
/// A no-op (returns `Ok(false)`) whenever the project is not gated on
/// (`autopurge_enabled`/`use_internal_purge` false, or `project_retention_days`
/// unset) or does not exist. The actual write goes through
/// [`AdminBackend::soft_delete_project`]'s conditional `UPDATE ... WHERE
/// deleted_at IS NULL` (DEC-013), so a project already soft-deleted is also a
/// no-op here, never re-stamping `deleted_at`.
pub async fn sweep_project(
    client: &VolumeClient,
    admin: &dyn AdminBackend,
    safety: &SafetyManager,
    project_id: &str,
) -> Result<bool> {
    sweep_project_at(now_unix(), client, admin, safety, project_id).await
}

/// Same as [`sweep_project`], but with an explicit `now`, for the same
/// boundary-test reason as [`sweep_project_files_at`].
async fn sweep_project_at(
    now: f64,
    client: &VolumeClient,
    admin: &dyn AdminBackend,
    safety: &SafetyManager,
    project_id: &str,
) -> Result<bool> {
    let config = admin.get_purge_config(project_id).await?;
    if !config.autopurge_enabled || !config.use_internal_purge {
        return Ok(false);
    }
    let Some(days) = config.project_retention_days else {
        return Ok(false);
    };
    let Some(project) = admin.get_project(project_id).await? else {
        return Ok(false);
    };

    let trash_root = format!("/{}", safety.config().trash_dir.trim_matches('/'));
    let nodes = client.meta.subtree("/").await?;
    let created_at = parse_iso_to_unix(&project.created_at)?;
    let reference = nodes
        .iter()
        .filter(|n| n.is_file() && !is_under(&n.path, &trash_root))
        .map(|n| n.atime)
        .fold(created_at, f64::max);

    let threshold = (days as f64) * SECONDS_PER_DAY;
    // Rounded to the nearest second: `created_at` round-trips through RFC 3339
    // text (microsecond granularity), which otherwise perturbs an exact-day
    // boundary by a fraction of a second that has no business meaning at a
    // retention granularity of whole days.
    let age = (now - reference).round();
    if age > threshold { admin.soft_delete_project(project_id).await } else { Ok(false) }
}

/// Aggregate counters for one orchestration cycle across every project
/// (US-0007: both the background loop and the CLI's global driver share this,
/// neither re-implements the gating `sweep_project_files`/`sweep_project`
/// already apply per project).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CycleSummary {
    pub files_purged: usize,
    pub projects_soft_deleted: usize,
    pub exports_swept: usize,
}

/// Runs the FR-NEW-007 file sweep then the FR-NEW-008 project sweep for every
/// project the admin store knows about. A failure sweeping one project (volume
/// open, file sweep or project sweep) is logged and skipped, never aborting the
/// rest of the cycle, the same per-item isolation `sweep_project_files` already
/// applies within one project (DEC-013).
pub async fn run_cycle(
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    safety: &SafetyManager,
) -> Result<CycleSummary> {
    let mut summary = CycleSummary::default();
    for project in admin.list_all_projects().await? {
        let client = match stores.client(&project.id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    project = %project.id,
                    error = %e,
                    "purge cycle: cannot open volume, skipping this project"
                );
                continue;
            }
        };
        match sweep_project_files(&client, admin, safety, &project.id).await {
            Ok(n) => summary.files_purged += n,
            Err(e) => {
                tracing::warn!(project = %project.id, error = %e, "purge cycle: file sweep failed");
            }
        }
        match sweep_project(&client, admin, safety, &project.id).await {
            Ok(true) => summary.projects_soft_deleted += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(project = %project.id, error = %e, "purge cycle: project sweep failed");
            }
        }
        match sweep_project_exports(stores, &client, &project.id).await {
            Ok(n) => summary.exports_swept += n,
            Err(e) => {
                tracing::warn!(project = %project.id, error = %e, "purge cycle: export sweep failed");
            }
        }
    }
    Ok(summary)
}

/// Deletes every `export_links` row (and its blob) in `project_id` whose
/// `expires_at` has already passed, per FR-NEW-014 / DEC-013: the grace
/// period for a never-downloaded export is simply its own expiry. Returns the
/// number of rows swept.
///
/// Each row commits independently, mirroring `sweep_project_files`'s per-item
/// isolation (DEC-013): a failure deleting one row's blob is logged and the
/// row is still deleted (the cheaper orphan to clean up next cycle is a
/// missing blob, never an orphaned row), and a failure deleting the row
/// itself is logged and skipped without touching any other row.
async fn sweep_project_exports(
    stores: &StoreManager,
    client: &VolumeClient,
    project_id: &str,
) -> Result<usize> {
    let now = crate::tools::export::iso(chrono::Utc::now());
    let db = crate::storage::open_meta_db(stores.config(), stores.relational(), project_id).await?;
    let rows = {
        let mut tx = db.begin().await?;
        let rows =
            crate::storage::meta::select_expired_export_links(&mut *tx, project_id, &now).await?;
        tx.commit().await?;
        rows
    };

    let mut swept = 0usize;
    for row in rows {
        if let Err(e) = client.blob.delete(&format!("export:{}", row.token)).await {
            tracing::warn!(
                project = %project_id,
                token_prefix = &row.token[..row.token.len().min(8)],
                error = %e,
                "purge cycle: export blob delete failed"
            );
        }
        if delete_export_row(&db, &row.token).await {
            swept += 1;
        }
    }
    Ok(swept)
}

/// Deletes one `export_links` row unconditionally (an empty `now` matches any
/// `expires_at` against `delete_export_link_if_live`'s `expires_at>?`
/// predicate, the same trick `exports::reap_expired` already relies on to
/// avoid a second delete query in `storage::meta`). Logs and returns `false`
/// on failure rather than propagating, so this row never aborts the sweep.
async fn delete_export_row(
    db: &std::sync::Arc<dyn crate::storage::rel::RelationalDb>,
    token: &str,
) -> bool {
    let mut tx = match db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!(error = %e, "purge cycle: export row delete failed to begin transaction");
            return false;
        }
    };
    if let Err(e) = crate::storage::meta::delete_export_link_if_live(&mut *tx, token, "").await {
        tracing::warn!(error = %e, "purge cycle: export row delete failed");
        return false;
    }
    if let Err(e) = tx.commit().await {
        tracing::warn!(error = %e, "purge cycle: export row delete failed to commit");
        return false;
    }
    true
}

/// Permanently removes every soft-deleted project whose grace period has
/// elapsed (FR-NEW-012): `now - deleted_at > project_purge_grace_days` (global
/// config, DEC-008), strictly. Removal runs the same cascade FR-027's
/// `admin.delete_project` tool runs (volume teardown, then the git repo purge
/// when `git.enabled`, then the ACL row delete), unconditionally with respect
/// to that project's `use_internal_purge` (DEC-007: that flag only gates
/// whether the file/project sweeps run automatically, never whether an
/// already-soft-deleted project eventually gets permanently removed). Returns
/// the number of projects permanently removed this cycle.
///
/// A cascade failure on one project propagates immediately (FR-027's own
/// failure contract, unmodified by this caller), so a locked row on one
/// project never masks a problem by silently continuing to the next.
pub async fn sweep_grace_period(
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    config: &Arc<ServerConfig>,
) -> Result<usize> {
    sweep_grace_period_at(now_unix(), stores, admin, config).await
}

/// Same as [`sweep_grace_period`], but with an explicit `now`, for the same
/// boundary-test reason as [`sweep_project_files_at`]/[`sweep_project_at`].
async fn sweep_grace_period_at(
    now: f64,
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    config: &Arc<ServerConfig>,
) -> Result<usize> {
    let threshold = (config.safety.project_purge_grace_days as f64) * SECONDS_PER_DAY;
    let mut removed = 0usize;
    for (project_id, deleted_at) in admin.list_soft_deleted_projects().await? {
        let deleted_unix = parse_iso_to_unix(&deleted_at)?;
        let age = (now - deleted_unix).round();
        if age <= threshold {
            continue;
        }
        stores.teardown_volume(&project_id).await?;
        if config.git.enabled {
            let store = GitRepoStore::shared(config.clone(), stores.relational().clone());
            store.purge_repo(&project_id).await?;
        }
        admin.delete_project(&project_id).await?;
        removed += 1;
    }
    Ok(removed)
}

/// True when `path` is `root` itself or a strict descendant of it.
fn is_under(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// Parses `project.created_at` (RFC 3339, the format [`crate::util::now_iso`]
/// writes) into fractional Unix seconds, the same unit as `NodeRow::atime`.
fn parse_iso_to_unix(iso: &str) -> Result<f64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.timestamp_micros() as f64 / 1_000_000.0)
        .map_err(|e| ToolError::internal(format!("invalid created_at timestamp '{iso}': {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SafetyConfig;
    use crate::storage::admin::RelationalAdminStore;
    use crate::storage::blob::local::LocalBlobStore;
    use crate::storage::meta::RelationalMetaStore;
    use crate::storage::rel::{Dialect, Query, RelationalDb, RelationalTx, RowValues};
    use crate::storage::traits::{MetaBackend, NodeRow};
    use async_trait::async_trait;
    use std::sync::Arc;

    const DAY: f64 = SECONDS_PER_DAY;

    struct Fix {
        _dir: tempfile::TempDir,
        meta: Arc<RelationalMetaStore>,
        client: VolumeClient,
        admin: RelationalAdminStore,
        safety: SafetyManager,
    }

    async fn fixture() -> Fix {
        let dir = tempfile::tempdir().unwrap();
        let meta = Arc::new(RelationalMetaStore::in_memory("proj").await.unwrap());
        let blob = Arc::new(LocalBlobStore::new(dir.path(), "mcpfs-test"));
        let client = VolumeClient::new("proj", meta.clone(), blob).with_trash_store(meta.clone());
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        admin.create_project("proj", "owner@t.c").await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);
        Fix { _dir: dir, meta, client, admin, safety }
    }

    /// Writes `path`, then backdates its `atime` to `now_unix() - age_seconds`.
    async fn seed_stale_file(f: &Fix, path: &str, age_seconds: f64) {
        seed_stale_file_at(f, path, now_unix(), age_seconds).await;
    }

    /// Same as [`seed_stale_file`], but backdates relative to an explicit `now`
    /// rather than a fresh `now_unix()` call, so a boundary test can anchor
    /// both the seed and the sweep to the exact same instant.
    async fn seed_stale_file_at(f: &Fix, path: &str, now: f64, age_seconds: f64) {
        f.client.write_text_atomic(path, "data").await.unwrap();
        f.meta.set_atime_for_test(path, now - age_seconds).await.unwrap();
    }

    #[tokio::test]
    async fn e2e_new_024_happy_path_file_purged() {
        let f = fixture().await;
        seed_stale_file(&f, "/old.txt", 8.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 1);
        assert!(!f.client.exists("/old.txt").await.unwrap(), "no longer at its original path");

        let trash_root = format!("/{}", f.safety.config().trash_dir);
        let in_trash = f.client.meta.subtree(&trash_root).await.unwrap();
        assert!(
            in_trash.iter().any(|n| n.name.ends_with("old.txt")),
            "old.txt must be in the trash listing"
        );
    }

    #[tokio::test]
    async fn e2e_new_025_boundary_exactly_at_threshold_not_purged() {
        let f = fixture().await;
        let now = now_unix();
        seed_stale_file_at(&f, "/edge.txt", now, 7.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let purged =
            sweep_project_files_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0, "strict > required, not >=");
        assert!(f.client.exists("/edge.txt").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_new_026_boundary_just_past_threshold_purged() {
        let f = fixture().await;
        let now = now_unix();
        seed_stale_file_at(&f, "/past.txt", now, 7.0 * DAY + 1.0).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let purged =
            sweep_project_files_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 1);
        assert!(!f.client.exists("/past.txt").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_new_027_use_internal_purge_false_gates_the_sweep_off() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 10.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, false, Some(7), None).await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0);
        assert!(f.client.exists("/stale.txt").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_new_028_per_file_commit_not_all_or_nothing() {
        let f = fixture().await;
        seed_stale_file(&f, "/one.txt", 10.0 * DAY).await;
        seed_stale_file(&f, "/two.txt", 10.0 * DAY).await;
        seed_stale_file(&f, "/three.txt", 10.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let failing_meta = Arc::new(FailingRenameMeta {
            inner: f.meta.clone(),
            fail_path: "/two.txt".to_string(),
        });
        let flaky_client = VolumeClient {
            project_id: "proj".to_string(),
            meta: failing_meta,
            blob: f.client.blob.clone(),
            // SPEC-0011 US-0002 added this field; this mock has no real relational
            // store behind it, and the trash write is a no-op without one.
            trash: None,
        };

        let purged = sweep_project_files(&flaky_client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 2, "file 1 and file 3 purged, file 2's failure isolated");
        assert!(f.client.exists("/two.txt").await.unwrap(), "file 2 left in place");
        assert!(!f.client.exists("/one.txt").await.unwrap());
        assert!(!f.client.exists("/three.txt").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_new_029_idempotent_re_purge() {
        let f = fixture().await;
        seed_stale_file(&f, "/old.txt", 8.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let first = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(first, 1);
        let trash_root = format!("/{}", f.safety.config().trash_dir);
        let after_first = f.client.meta.subtree(&trash_root).await.unwrap().len();

        let second = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(second, 0, "re-purging an already-trashed file is a no-op");
        let after_second = f.client.meta.subtree(&trash_root).await.unwrap().len();
        assert_eq!(after_first, after_second, "no duplicate trash entry");
    }

    #[tokio::test]
    async fn e2e_new_030_empty_project() {
        let f = fixture().await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0);
    }

    #[tokio::test]
    async fn e2e_new_031_file_retention_days_none_skips_the_file_step() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 100.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0, "file step skipped independently of project_retention_days");
        assert!(f.client.exists("/stale.txt").await.unwrap());
    }

    #[tokio::test]
    async fn unconfigured_project_is_a_no_op() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 100.0 * DAY).await;
        // No seed_purge_config_for_test call: the project has never been configured.

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0);
        assert!(f.client.exists("/stale.txt").await.unwrap());
    }

    // ── SPEC-0011 US-0003: sweep writes the trash entry ─────────────────────

    /// Locates the one `trash_entries` row under `trash_root` whose
    /// `original_path` matches `original`, by scanning the trash subtree on
    /// disk (the sweep's destination stamp is wall-clock, so a test cannot
    /// predict it ahead of time).
    async fn find_trash_row_for(
        f: &Fix,
        trash_root: &str,
        original: &str,
    ) -> crate::storage::traits::TrashEntryRow {
        let store = f.client.trash.as_ref().unwrap();
        let in_trash = f.client.meta.subtree(trash_root).await.unwrap();
        for node in in_trash {
            if let Some(row) = store.get_trash_entry_for_test(&node.path).await.unwrap()
                && row.original_path == original
            {
                return row;
            }
        }
        panic!("no trash_entries row found for original_path '{original}'");
    }

    #[tokio::test]
    async fn e2e_new_403_sweep_writes_a_system_initiated_trash_entry() {
        let f = fixture().await;
        seed_stale_file(&f, "/old.txt", 3.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(1), None).await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 1);

        let trash_root = format!("/{}", f.safety.config().trash_dir);
        let row = find_trash_row_for(&f, &trash_root, "/old.txt").await;
        assert_eq!(row.original_path, "/old.txt");
        assert!(row.deleted_by.is_none(), "system-initiated delete, not a null string");
    }

    #[tokio::test]
    async fn e2e_new_440_sweep_created_entries_match_a_user_initiated_shape() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 5.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(1), None).await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 1);

        // TODO(US-0004/US-0005): once fs.trash_list/fs.trash_restore exist, extend
        // this into a full round trip instead of a direct DB query (E2E-NEW-440).
        let trash_root = format!("/{}", f.safety.config().trash_dir);
        let row = find_trash_row_for(&f, &trash_root, "/stale.txt").await;
        assert!(row.trash_path.ends_with("__stale.txt"), "got {}", row.trash_path);
        assert_eq!(row.size, 4, "matches the seeded \"data\" payload");
        assert_eq!(row.kind, "file");
        assert!(row.deleted_by.is_none());
    }

    #[tokio::test]
    async fn e2e_new_457_sweep_with_no_stale_files_writes_no_trash_entries() {
        let f = fixture().await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();
        let store = f.client.trash.as_ref().unwrap();
        let before = store.count_trash_entries_for_test().await.unwrap();

        let purged = sweep_project_files(&f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0);
        let after = store.count_trash_entries_for_test().await.unwrap();
        assert_eq!(before, after, "a no-op sweep writes nothing");
        assert_eq!(after, 0);
    }

    /// Delegates every [`MetaBackend`] method to `inner`, except `rename`,
    /// which always fails with `ERR_NO_CLOBBER`, counting its calls. Used in
    /// place of pre-occupying 51 wall-clock-stamped destinations (the sweep's
    /// destination, unlike `fs_ops::delete_path_at`'s deterministic seam, is
    /// stamped off `safety.trash_path`'s own `now_unix()` call, which a test
    /// cannot predict ahead of time): this reaches the same collision-
    /// exhaustion path deterministically, at the `MetaBackend::rename` level
    /// `rename_with_collision_retry` itself calls (E2E-NEW-447).
    struct AlwaysCollideMeta {
        inner: Arc<dyn MetaBackend>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl MetaBackend for AlwaysCollideMeta {
        async fn get(&self, path: &str) -> Result<Option<NodeRow>> {
            self.inner.get(path).await
        }
        async fn list_children(&self, parent: &str) -> Result<Vec<NodeRow>> {
            self.inner.list_children(parent).await
        }
        async fn subtree(&self, root: &str) -> Result<Vec<NodeRow>> {
            self.inner.subtree(root).await
        }
        async fn put_file(
            &self,
            path: &str,
            sha256: Option<&str>,
            size: i64,
            mode: i64,
        ) -> Result<crate::storage::traits::PutFileResult> {
            self.inner.put_file(path, sha256, size, mode).await
        }
        async fn delete_file(&self, path: &str) -> Result<Option<String>> {
            self.inner.delete_file(path).await
        }
        async fn remove_subtree(&self, path: &str) -> Result<Vec<String>> {
            self.inner.remove_subtree(path).await
        }
        async fn mkdirs(&self, path: &str, exist_ok: bool) -> Result<()> {
            self.inner.mkdirs(path, exist_ok).await
        }
        async fn mkdir(&self, path: &str) -> Result<()> {
            self.inner.mkdir(path).await
        }
        async fn rmdir(&self, path: &str) -> Result<()> {
            self.inner.rmdir(path).await
        }
        async fn rename(&self, _src: &str, _dst: &str) -> Result<()> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(crate::errors::ToolError::no_clobber("simulated collision"))
        }
        async fn touch_atime(&self, path: &str) -> Result<()> {
            self.inner.touch_atime(path).await
        }
        async fn touch_atime_mtime(&self, path: &str) -> Result<()> {
            self.inner.touch_atime_mtime(path).await
        }
        async fn stale_files(&self, before: f64, exclude_root: &str) -> Result<Vec<NodeRow>> {
            self.inner.stale_files(before, exclude_root).await
        }
    }

    #[tokio::test]
    async fn e2e_new_447_collision_exhaustion_inside_the_sweep_surfaces_the_same_error() {
        let f = fixture().await;
        seed_stale_file(&f, "/x", 8.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mock = AlwaysCollideMeta { inner: f.client.meta.clone(), calls: calls.clone() };
        let colliding_client = VolumeClient::new("proj", Arc::new(mock), f.client.blob.clone())
            .with_trash_store(f.meta.clone());

        let purged =
            sweep_project_files(&colliding_client, &f.admin, &f.safety, "proj").await.unwrap();
        assert_eq!(purged, 0, "the colliding file's failure is isolated, not propagated");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            51,
            "one bare attempt plus 50 suffixed retries, same exhaustion as E2E-NEW-405"
        );
        assert!(f.client.exists("/x").await.unwrap(), "stale file remains live after exhaustion");
    }

    /// Wraps a real [`RelationalDb`] so one `INSERT INTO trash_entries`
    /// transaction fails exactly once, for E2E-NEW-445: proving a failed
    /// `trash_entries` insert leaves the swept file live rather than
    /// orphaned in the trash directory with no tracking row.
    struct FailTrashInsertOnceDb {
        inner: Arc<dyn RelationalDb>,
        armed: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl RelationalDb for FailTrashInsertOnceDb {
        fn dialect(&self) -> Dialect {
            self.inner.dialect()
        }
        async fn execute(&self, query: &Query) -> Result<u64> {
            self.inner.execute(query).await
        }
        async fn query(&self, query: &Query) -> Result<Vec<RowValues>> {
            self.inner.query(query).await
        }
        async fn begin(&self) -> Result<Box<dyn RelationalTx>> {
            let tx = self.inner.begin().await?;
            Ok(Box::new(FailTrashInsertOnceTx { inner: tx, armed: self.armed.clone() }))
        }
        async fn migrate(&self, schema: &crate::storage::rel::SchemaSet) -> Result<()> {
            self.inner.migrate(schema).await
        }
    }

    struct FailTrashInsertOnceTx {
        inner: Box<dyn RelationalTx>,
        armed: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl RelationalTx for FailTrashInsertOnceTx {
        fn dialect(&self) -> Dialect {
            self.inner.dialect()
        }
        async fn execute(&mut self, query: &Query) -> Result<u64> {
            if query.sql.contains("INSERT INTO trash_entries")
                && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(ToolError::internal("simulated trash_entries insert failure"));
            }
            self.inner.execute(query).await
        }
        async fn query(&mut self, query: &Query) -> Result<Vec<RowValues>> {
            self.inner.query(query).await
        }
        async fn commit(self: Box<Self>) -> Result<()> {
            self.inner.commit().await
        }
    }

    #[tokio::test]
    async fn e2e_new_445_a_failed_trash_entries_insert_leaves_the_file_live() {
        use crate::storage::meta::RelationalMetaStore;
        use crate::storage::rel::SqliteRelationalDb;

        let dir = tempfile::tempdir().unwrap();
        let real_db: Arc<dyn RelationalDb> =
            Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let meta = Arc::new(RelationalMetaStore::open(real_db.clone(), "proj").await.unwrap());
        let blob = Arc::new(LocalBlobStore::new(dir.path(), "mcpfs-test"));
        let client = VolumeClient::new("proj", meta.clone(), blob);

        let failing_db: Arc<dyn RelationalDb> = Arc::new(FailTrashInsertOnceDb {
            inner: real_db,
            armed: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        });
        let failing_trash = Arc::new(RelationalMetaStore::open(failing_db, "proj").await.unwrap());
        let client = client.with_trash_store(failing_trash.clone());

        let admin = RelationalAdminStore::in_memory().await.unwrap();
        admin.create_project("proj", "owner@t.c").await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);

        client.write_text_atomic("/stale2.txt", "data").await.unwrap();
        meta.set_atime_for_test("/stale2.txt", now_unix() - 8.0 * DAY).await.unwrap();
        admin.seed_purge_config_for_test("proj", true, true, Some(1), None).await.unwrap();

        let purged = sweep_project_files(&client, &admin, &safety, "proj").await.unwrap();
        assert_eq!(purged, 0, "the per-file failure is isolated, matching purge.rs:26");
        assert!(client.exists("/stale2.txt").await.unwrap(), "the file remains live, not trashed");
        assert_eq!(
            failing_trash.count_trash_entries_for_test().await.unwrap(),
            0,
            "no orphaned trash_entries row either"
        );
    }

    /// Backdates `proj`'s `created_at` to `now - age_seconds`, so a project-sweep
    /// test can pin its age without racing the wall clock.
    async fn seed_project_age(f: &Fix, now: f64, age_seconds: f64) {
        let created_at = chrono::DateTime::from_timestamp(
            (now - age_seconds) as i64,
            (((now - age_seconds).fract()) * 1_000_000_000.0) as u32,
        )
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
        f.admin.seed_created_at_for_test("proj", &created_at).await.unwrap();
    }

    #[tokio::test]
    async fn e2e_new_032_happy_path_project_soft_deleted() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 50.0 * DAY).await;
        seed_stale_file_at(&f, "/live.txt", now, 45.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(deleted);
        assert!(f.admin.deleted_at_for_test("proj").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn e2e_new_033_boundary_exactly_at_threshold_not_soft_deleted() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 30.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(!deleted, "strict > required, not >=");
        assert!(f.admin.deleted_at_for_test("proj").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn e2e_new_034_boundary_just_past_threshold_soft_deleted() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 30.0 * DAY + 1.0).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn e2e_new_035_no_live_files_falls_back_to_created_at_alone() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 40.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn e2e_new_036_max_semantics_recent_activity_resets_staleness() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 200.0 * DAY).await;
        seed_stale_file_at(&f, "/recent.txt", now, 5.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(!deleted, "recent file activity resets the clock even for an old project");
    }

    #[tokio::test]
    async fn e2e_new_037_use_internal_purge_false_gates_the_project_sweep_off() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 100.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, false, None, Some(30)).await.unwrap();

        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(!deleted);
        assert!(f.admin.deleted_at_for_test("proj").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn e2e_new_038_repeated_trigger_does_not_advance_deleted_at() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 50.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let first = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(first);
        let first_value = f.admin.deleted_at_for_test("proj").await.unwrap().unwrap();

        let later = now + 60.0;
        let second = sweep_project_at(later, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(!second, "already soft-deleted, this trigger is a no-op");
        let second_value = f.admin.deleted_at_for_test("proj").await.unwrap().unwrap();
        assert_eq!(first_value, second_value, "deleted_at must not advance to the later `now`");
    }

    #[tokio::test]
    async fn e2e_new_039_soft_delete_never_deletes_data() {
        let f = fixture().await;
        let now = now_unix();
        seed_project_age(&f, now, 50.0 * DAY).await;
        seed_stale_file_at(&f, "/keep.txt", now, 50.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let before = f.client.meta.subtree("/").await.unwrap().len();
        let deleted = sweep_project_at(now, &f.client, &f.admin, &f.safety, "proj").await.unwrap();
        assert!(deleted);
        let after = f.client.meta.subtree("/").await.unwrap().len();
        assert_eq!(before, after, "soft-delete touches only project.deleted_at");
        assert!(f.client.exists("/keep.txt").await.unwrap());
    }

    #[tokio::test]
    async fn run_cycle_sweeps_every_project_and_aggregates_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut raw_config = crate::config::ServerConfig::default();
        raw_config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        raw_config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        let relational = std::sync::Arc::new(crate::storage::RelationalRegistry::new());
        let config = std::sync::Arc::new(raw_config);
        let stores = StoreManager::new(config, relational);
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);

        admin.create_project("p1", "owner@t.c").await.unwrap();
        admin.create_project("p2", "owner@t.c").await.unwrap();
        // `file_retention_days: Some(0)` so the write below is already stale by the
        // time `run_cycle` reads `now_unix()` again a moment later.
        admin.seed_purge_config_for_test("p1", true, true, Some(0), None).await.unwrap();
        admin.seed_purge_config_for_test("p2", true, true, None, Some(30)).await.unwrap();

        let c1 = stores.client("p1").await.unwrap();
        c1.write_text_atomic("/old.txt", "data").await.unwrap();

        let now = now_unix();
        let created_at = chrono::DateTime::from_timestamp((now - 40.0 * DAY) as i64, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
        admin.seed_created_at_for_test("p2", &created_at).await.unwrap();

        let summary = run_cycle(&stores, &admin, &safety).await.unwrap();
        assert_eq!(summary.files_purged, 1, "p1's stale file was swept");
        assert_eq!(summary.projects_soft_deleted, 1, "p2's staleness soft-deleted it");
        assert!(admin.deleted_at_for_test("p2").await.unwrap().is_some());
        assert!(admin.deleted_at_for_test("p1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn run_cycle_on_zero_projects_is_a_zero_count_no_op() {
        let relational = std::sync::Arc::new(crate::storage::RelationalRegistry::new());
        let config = std::sync::Arc::new(crate::config::ServerConfig::default());
        let stores = StoreManager::new(config, relational);
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);

        let summary = run_cycle(&stores, &admin, &safety).await.unwrap();
        assert_eq!(summary, CycleSummary::default());
    }

    #[tokio::test]
    async fn e2e_new_079_many_projects_bounded_time_no_skip_no_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let mut raw_config = crate::config::ServerConfig::default();
        raw_config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        raw_config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        let relational = std::sync::Arc::new(crate::storage::RelationalRegistry::new());
        let config = std::sync::Arc::new(raw_config);
        let stores = StoreManager::new(config, relational);
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);

        const N: usize = 50;
        for i in 0..N {
            let id = format!("proj{i}");
            admin.create_project(&id, "owner@t.c").await.unwrap();
            if i % 2 == 0 {
                admin.seed_purge_config_for_test(&id, true, true, Some(0), None).await.unwrap();
                let client = stores.client(&id).await.unwrap();
                client.write_text_atomic("/old.txt", "data").await.unwrap();
            } else {
                admin.seed_purge_config_for_test(&id, false, false, Some(0), None).await.unwrap();
            }
        }

        let start = std::time::Instant::now();
        let summary = run_cycle(&stores, &admin, &safety).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(summary.files_purged, N / 2, "exactly the stale half, no skip, no duplicate");
        assert!(elapsed < std::time::Duration::from_secs(10), "took {elapsed:?}");
    }

    /// Builds a `StoreManager` + admin store backed by a fresh tempdir, the same
    /// shape [`run_cycle`]'s tests use, so a grace-sweep test can call
    /// `sweep_grace_period` through its real signature (git disabled: the default
    /// `ServerConfig`'s `git.enabled` is `false`, so no test here touches
    /// `GitRepoStore`'s process-global shared instance).
    struct GraceFix {
        _dir: tempfile::TempDir,
        stores: StoreManager,
        admin: RelationalAdminStore,
        config: Arc<crate::config::ServerConfig>,
    }

    async fn grace_fixture() -> GraceFix {
        let dir = tempfile::tempdir().unwrap();
        let mut raw_config = crate::config::ServerConfig::default();
        raw_config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        raw_config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        raw_config.safety.project_purge_grace_days = 30;
        let relational = Arc::new(crate::storage::RelationalRegistry::new());
        let config = Arc::new(raw_config);
        let stores = StoreManager::new(config.clone(), relational);
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        GraceFix { _dir: dir, stores, admin, config }
    }

    /// Seeds `proj`, soft-deletes it, and backdates `deleted_at` to `now -
    /// age_seconds` by writing the raw ISO timestamp (the column `deleted_at`
    /// actually holds), so a boundary test can pin the age without racing the
    /// wall clock.
    async fn seed_soft_deleted(f: &GraceFix, project_id: &str, now: f64, age_seconds: f64) {
        f.admin.create_project(project_id, "owner@t.c").await.unwrap();
        let deleted_at = chrono::DateTime::from_timestamp(
            (now - age_seconds) as i64,
            (((now - age_seconds).fract()) * 1_000_000_000.0) as u32,
        )
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
        f.admin.seed_deleted_at_for_test(project_id, &deleted_at).await.unwrap();
    }

    #[tokio::test]
    async fn e2e_new_053_happy_path_direct_sweep() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 31.0 * DAY).await;

        let removed = sweep_grace_period_at(now, &f.stores, &f.admin, &f.config).await.unwrap();
        assert_eq!(removed, 1);
        assert!(f.admin.get_project("proj").await.unwrap().is_none(), "permanently removed");
    }

    #[tokio::test]
    async fn e2e_new_054_boundary_exactly_at_grace_period_not_removed() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 30.0 * DAY).await;

        let removed = sweep_grace_period_at(now, &f.stores, &f.admin, &f.config).await.unwrap();
        assert_eq!(removed, 0, "strict > required, not >=");
        assert!(f.admin.get_project("proj").await.unwrap().is_some(), "not yet removed");
    }

    #[tokio::test]
    async fn e2e_new_055_boundary_just_past_grace_period_removed() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 30.0 * DAY + 1.0).await;

        let removed = sweep_grace_period_at(now, &f.stores, &f.admin, &f.config).await.unwrap();
        assert_eq!(removed, 1);
        assert!(f.admin.get_project("proj").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn e2e_new_056_unconditional_on_use_internal_purge() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 31.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, false, None, None).await.unwrap();

        let removed = sweep_grace_period_at(now, &f.stores, &f.admin, &f.config).await.unwrap();
        assert_eq!(removed, 1, "use_internal_purge=false never gates the grace sweep off");
        assert!(f.admin.get_project("proj").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn e2e_new_057_reuses_fr_027_cascade_exactly() {
        let f = grace_fixture().await;
        let now = now_unix();

        // Baseline: a live project removed by hand, through the exact same
        // primitives FR-027's `admin.delete_project` tool runs (no soft-delete
        // involved at all, proving the cascade itself, independent of grace
        // evaluation).
        f.admin.create_project("baseline", "owner@t.c").await.unwrap();
        let baseline_client = f.stores.client("baseline").await.unwrap();
        baseline_client.write_text_atomic("/keep.txt", "data").await.unwrap();
        f.stores.teardown_volume("baseline").await.unwrap();
        f.admin.delete_project("baseline").await.unwrap();
        let baseline_gone = f.admin.get_project("baseline").await.unwrap().is_none();

        // Grace sweep: an equivalent soft-deleted project, past its grace period.
        seed_soft_deleted(&f, "swept", now, 31.0 * DAY).await;
        let swept_client = f.stores.client("swept").await.unwrap();
        swept_client.write_text_atomic("/keep.txt", "data").await.unwrap();

        let removed = sweep_grace_period_at(now, &f.stores, &f.admin, &f.config).await.unwrap();

        assert_eq!(removed, 1);
        assert!(baseline_gone, "baseline cascade actually removed the project");
        assert!(
            f.admin.get_project("swept").await.unwrap().is_none(),
            "same outcome via the sweep"
        );
        assert!(
            f.stores.client("swept").await.is_ok(),
            "teardown recreates an empty volume on next open, same as the baseline would"
        );
    }

    /// Delegates every [`AdminBackend`] method to `inner`, except `delete_project`,
    /// which fails whenever `project_id == fail_project` (E2E-NEW-080: simulates a
    /// locked child row partway through the cascade).
    struct FailingDeleteAdmin {
        inner: RelationalAdminStore,
        fail_project: String,
    }

    #[async_trait]
    impl AdminBackend for FailingDeleteAdmin {
        async fn connect(&self) -> Result<()> {
            self.inner.connect().await
        }
        async fn create_project(
            &self,
            project_id: &str,
            owner: &str,
        ) -> Result<crate::storage::traits::Project> {
            self.inner.create_project(project_id, owner).await
        }
        async fn delete_project(&self, project_id: &str) -> Result<()> {
            if project_id == self.fail_project {
                return Err(ToolError::internal(format!(
                    "simulated locked row for '{project_id}'"
                )));
            }
            self.inner.delete_project(project_id).await
        }
        async fn add_member(
            &self,
            project_id: &str,
            person: &str,
            added_by: &str,
        ) -> Result<crate::storage::traits::Member> {
            self.inner.add_member(project_id, person, added_by).await
        }
        async fn remove_member(&self, project_id: &str, person: &str) -> Result<()> {
            self.inner.remove_member(project_id, person).await
        }
        async fn get_project(
            &self,
            project_id: &str,
        ) -> Result<Option<crate::storage::traits::Project>> {
            self.inner.get_project(project_id).await
        }
        async fn list_projects_for(
            &self,
            person: &str,
        ) -> Result<Vec<crate::storage::traits::Project>> {
            self.inner.list_projects_for(person).await
        }
        async fn list_all_projects(&self) -> Result<Vec<crate::storage::traits::Project>> {
            self.inner.list_all_projects().await
        }
        async fn list_all_persons(&self) -> Result<Vec<String>> {
            self.inner.list_all_persons().await
        }
        async fn list_members(
            &self,
            project_id: &str,
        ) -> Result<Vec<crate::storage::traits::Member>> {
            self.inner.list_members(project_id).await
        }
        async fn is_member(&self, project_id: &str, person: &str) -> Result<bool> {
            self.inner.is_member(project_id, person).await
        }
        async fn require_member(&self, project_id: &str, person: &str) -> Result<()> {
            self.inner.require_member(project_id, person).await
        }
        async fn require_owner(
            &self,
            project_id: &str,
            person: &str,
        ) -> Result<crate::storage::traits::Project> {
            self.inner.require_owner(project_id, person).await
        }
        async fn set_index_mode(
            &self,
            project_id: &str,
            mode: crate::storage::traits::IndexMode,
        ) -> Result<()> {
            self.inner.set_index_mode(project_id, mode).await
        }
        async fn get_index_mode(
            &self,
            project_id: &str,
        ) -> Result<crate::storage::traits::IndexMode> {
            self.inner.get_index_mode(project_id).await
        }
        async fn get_purge_config(
            &self,
            project_id: &str,
        ) -> Result<crate::storage::traits::PurgeConfig> {
            self.inner.get_purge_config(project_id).await
        }
        async fn set_purge_config(
            &self,
            project_id: &str,
            config: crate::storage::traits::PurgeConfig,
        ) -> Result<()> {
            self.inner.set_purge_config(project_id, config).await
        }
        async fn soft_delete_project(&self, project_id: &str) -> Result<bool> {
            self.inner.soft_delete_project(project_id).await
        }
        async fn undelete_project(&self, project_id: &str) -> Result<()> {
            self.inner.undelete_project(project_id).await
        }
        async fn list_soft_deleted_projects(&self) -> Result<Vec<(String, String)>> {
            self.inner.list_soft_deleted_projects().await
        }
        async fn set_quota(&self, project_id: &str, quota_bytes: Option<i64>) -> Result<()> {
            self.inner.set_quota(project_id, quota_bytes).await
        }
        async fn get_quota(&self, project_id: &str) -> Result<Option<i64>> {
            self.inner.get_quota(project_id).await
        }
    }

    #[tokio::test]
    async fn e2e_new_080_cascade_failure_surfaces_like_any_fr_027_failure() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 31.0 * DAY).await;

        let failing = FailingDeleteAdmin { inner: f.admin, fail_project: "proj".to_string() };
        let err = sweep_grace_period_at(now, &f.stores, &failing, &f.config).await.unwrap_err();
        assert!(
            err.to_string().contains("simulated locked row"),
            "the failure surfaces, not swallowed"
        );

        // No partial state beyond FR-027's own cascade: the volume teardown that
        // ran before the failing row delete is not rolled back (same as an
        // unmodified `admin.delete_project` call failing at the same point would
        // leave behind), but the project row itself survives the failed delete.
        assert!(failing.inner.get_project("proj").await.unwrap().is_some());
    }

    /// Delegates every [`MetaBackend`] method to `inner`, except `rename`, which
    /// fails whenever `src == fail_path` (E2E-NEW-028: simulates a locked row).
    struct FailingRenameMeta {
        inner: Arc<dyn MetaBackend>,
        fail_path: String,
    }

    #[async_trait]
    impl MetaBackend for FailingRenameMeta {
        async fn get(&self, path: &str) -> Result<Option<NodeRow>> {
            self.inner.get(path).await
        }
        async fn list_children(&self, parent: &str) -> Result<Vec<NodeRow>> {
            self.inner.list_children(parent).await
        }
        async fn subtree(&self, root: &str) -> Result<Vec<NodeRow>> {
            self.inner.subtree(root).await
        }
        async fn put_file(
            &self,
            path: &str,
            sha256: Option<&str>,
            size: i64,
            mode: i64,
        ) -> Result<crate::storage::traits::PutFileResult> {
            self.inner.put_file(path, sha256, size, mode).await
        }
        async fn delete_file(&self, path: &str) -> Result<Option<String>> {
            self.inner.delete_file(path).await
        }
        async fn remove_subtree(&self, path: &str) -> Result<Vec<String>> {
            self.inner.remove_subtree(path).await
        }
        async fn mkdirs(&self, path: &str, exist_ok: bool) -> Result<()> {
            self.inner.mkdirs(path, exist_ok).await
        }
        async fn mkdir(&self, path: &str) -> Result<()> {
            self.inner.mkdir(path).await
        }
        async fn rmdir(&self, path: &str) -> Result<()> {
            self.inner.rmdir(path).await
        }
        async fn rename(&self, src: &str, dst: &str) -> Result<()> {
            if src == self.fail_path {
                return Err(crate::errors::ToolError::internal(format!(
                    "simulated locked row for '{src}'"
                )));
            }
            self.inner.rename(src, dst).await
        }
        async fn touch_atime(&self, path: &str) -> Result<()> {
            self.inner.touch_atime(path).await
        }
        async fn touch_atime_mtime(&self, path: &str) -> Result<()> {
            self.inner.touch_atime_mtime(path).await
        }
        async fn stale_files(&self, before: f64, exclude_root: &str) -> Result<Vec<NodeRow>> {
            self.inner.stale_files(before, exclude_root).await
        }
    }

    // ── export sweep (SPEC-0012 US-0004, FR-NEW-014) ───────────────────────────

    struct ExportFix {
        _dir: tempfile::TempDir,
        stores: StoreManager,
        admin: RelationalAdminStore,
        safety: SafetyManager,
    }

    async fn export_fixture() -> ExportFix {
        let dir = tempfile::tempdir().unwrap();
        let mut raw_config = crate::config::ServerConfig::default();
        raw_config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        raw_config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        let relational = Arc::new(crate::storage::RelationalRegistry::new());
        let config = Arc::new(raw_config);
        let stores = StoreManager::new(config, relational);
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);
        admin.create_project("proj", "owner@t.c").await.unwrap();
        ExportFix { _dir: dir, stores, admin, safety }
    }

    /// Inserts a live `export_links` row for `token`, with `expires_at` set to
    /// `now + offset_secs` (negative = already expired), and its blob at
    /// `export:{token}` carrying `bytes`.
    async fn seed_export(f: &ExportFix, token: &str, offset_secs: f64, bytes: &[u8]) {
        // Ensures the volume's schema (including `export_links`) exists before
        // the raw connection below writes to it.
        f.stores.client("proj").await.unwrap();
        let db = crate::storage::open_meta_db(f.stores.config(), f.stores.relational(), "proj")
            .await
            .unwrap();
        let mut tx = db.begin().await.unwrap();
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::milliseconds((offset_secs * 1000.0) as i64);
        crate::storage::meta::insert_export_link(
            &mut *tx,
            token,
            "proj",
            &crate::tools::export::iso(now),
            &crate::tools::export::iso(expires),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let client = f.stores.client("proj").await.unwrap();
        client.blob.put(&format!("export:{token}"), bytes).await.unwrap();
    }

    /// `0` or `1`: whether `token`'s `export_links` row still exists.
    async fn export_row_count(f: &ExportFix, token: &str) -> usize {
        let db = crate::storage::open_meta_db(f.stores.config(), f.stores.relational(), "proj")
            .await
            .unwrap();
        let mut tx = db.begin().await.unwrap();
        // Far-future cutoff: every row, expired or not, has `expires_at` in
        // the past relative to it, so this is an unconditional row count.
        let rows = crate::storage::meta::select_expired_export_links(
            &mut *tx,
            "proj",
            "9999-01-01T00:00:00Z",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        rows.iter().filter(|r| r.token == token).count()
    }

    async fn export_blob_present(f: &ExportFix, token: &str) -> bool {
        let client = f.stores.client("proj").await.unwrap();
        client.blob.exists(&format!("export:{token}")).await.unwrap()
    }

    /// E2E-NEW-031: happy path, the cycle returns success.
    #[tokio::test]
    async fn e2e_new_031_sweep_deletes_expired_never_downloaded_export() {
        let f = export_fixture().await;
        seed_export(&f, "T", -3600.0, b"zip bytes").await;
        let result = run_cycle(&f.stores, &f.admin, &f.safety).await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// E2E-NEW-032: the `export_links` row is gone after the sweep.
    #[tokio::test]
    async fn e2e_new_032_sweep_removes_the_export_link_row() {
        let f = export_fixture().await;
        seed_export(&f, "T", -3600.0, b"zip bytes").await;
        assert_eq!(export_row_count(&f, "T").await, 1, "row present before the sweep");
        let summary = run_cycle(&f.stores, &f.admin, &f.safety).await.unwrap();
        assert_eq!(export_row_count(&f, "T").await, 0);
        assert_eq!(summary.exports_swept, 1);
    }

    /// E2E-NEW-033: the export blob is gone after the sweep.
    #[tokio::test]
    async fn e2e_new_033_sweep_removes_the_export_blob() {
        let f = export_fixture().await;
        seed_export(&f, "T", -3600.0, b"zip bytes").await;
        assert!(export_blob_present(&f, "T").await, "blob present before the sweep");
        run_cycle(&f.stores, &f.admin, &f.safety).await.unwrap();
        assert!(!export_blob_present(&f, "T").await);
    }

    /// E2E-NEW-034: one row's blob already gone out-of-band must not stop the
    /// other row's row+blob from being swept, nor abort the cycle.
    #[tokio::test]
    async fn e2e_new_034_a_missing_blob_on_one_row_does_not_abort_the_sweep() {
        let f = export_fixture().await;
        seed_export(&f, "T1", -3600.0, b"t1 bytes").await;
        seed_export(&f, "T2", -3600.0, b"t2 bytes").await;
        let client = f.stores.client("proj").await.unwrap();
        client.blob.delete("export:T1").await.unwrap();

        let result = run_cycle(&f.stores, &f.admin, &f.safety).await;
        assert!(result.is_ok(), "{result:?}");
        let summary = result.unwrap();
        assert_eq!(summary.exports_swept, 2, "both rows removed regardless of T1's blob outcome");
        assert_eq!(export_row_count(&f, "T1").await, 0);
        assert_eq!(export_row_count(&f, "T2").await, 0);
        assert!(!export_blob_present(&f, "T2").await, "T2's blob was deleted by the sweep");
    }

    /// E2E-NEW-035: an unexpired row (and its blob) is left untouched.
    #[tokio::test]
    async fn e2e_new_035_unexpired_rows_are_left_untouched() {
        let f = export_fixture().await;
        seed_export(&f, "T_expired", -3600.0, b"expired bytes").await;
        seed_export(&f, "T_live", 4.0 * 60.0, b"live bytes").await;

        let summary = run_cycle(&f.stores, &f.admin, &f.safety).await.unwrap();
        assert_eq!(summary.exports_swept, 1);
        assert_eq!(export_row_count(&f, "T_expired").await, 0);
        assert!(!export_blob_present(&f, "T_expired").await);
        assert_eq!(export_row_count(&f, "T_live").await, 1, "the live row must survive");
        assert!(export_blob_present(&f, "T_live").await, "the live blob must survive");
        let client = f.stores.client("proj").await.unwrap();
        let bytes = client.blob.get("export:T_live", 0, None).await.unwrap();
        assert_eq!(bytes, b"live bytes");
    }

    /// E2E-NEW-036: zero `export_links` rows is a no-op, no panic.
    #[tokio::test]
    async fn e2e_new_036_zero_export_links_rows_is_a_no_op() {
        let f = export_fixture().await;
        let summary = run_cycle(&f.stores, &f.admin, &f.safety).await.unwrap();
        assert_eq!(summary.exports_swept, 0);
    }
}
