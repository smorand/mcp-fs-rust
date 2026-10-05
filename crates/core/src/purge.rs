//! File-purge sweep (SPEC-0014 US-0005): soft-delete into the existing trash
//! every live file whose `atime` is older than its project's configured
//! `file_retention_days`.
//!
//! This module exposes a plain callable async function, nothing more: wiring it
//! to the background loop and the CLI is US-0007's job (see `Constraints` in the
//! story — `cli.rs` and `app.rs` are not touched here).

use crate::config::ServerConfig;
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
        // One commit per file: any failure here (mkdir or rename) is swallowed
        // so it never blocks or rolls back the files before or after it.
        if client.makedirs(parent, true).await.is_err() {
            continue;
        }
        if client.rename(&node.path, &dst).await.is_ok() {
            purged += 1;
        }
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
    }
    Ok(summary)
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
        let client = VolumeClient::new("proj", meta.clone(), blob);
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
}
