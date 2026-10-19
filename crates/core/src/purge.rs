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
use crate::search::SearchBackend;
use crate::storage::StoreManager;
use crate::storage::traits::TrashEntry;
use crate::storage::traits::{AdminBackend, BlobEntry};
use crate::storage::volume::VolumeClient;
use crate::util::now_unix;
use std::sync::Arc;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// An object must be strictly older than this before the orphan sweep may
/// delete it (SPEC-0034 DEC-003): the conservative side for a deleting job.
const ORPHAN_MIN_AGE_SECONDS: f64 = SECONDS_PER_DAY;

/// How long a superseded artifact set survives before the sweep reclaims it
/// (SPEC-0019 DEC-011): long enough that a reader mid-way through a prior
/// `fs.get_image`/`fs.get_table` call against the old set is not racing the
/// delete.
const ARTIFACT_SWEEP_GRACE_SECONDS: i64 = 600;

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
    search: Option<&Arc<dyn SearchBackend>>,
) -> Result<usize> {
    sweep_project_files_at(now_unix(), client, admin, safety, project_id, search).await
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
    search: Option<&Arc<dyn SearchBackend>>,
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
        // One commit per file: any failure here (mkdir or the joint rename and
        // trash entry write) is swallowed so it never blocks or rolls back the
        // files before or after it (DEC-013). The rename and the row share one
        // transaction (SPEC-0044), so a failure leaves the file where it was.
        if client.makedirs(parent, true).await.is_err() {
            continue;
        }
        let entry = client.trash.as_ref().map(|_| TrashEntry {
            original_path: node.path.clone(),
            size: node.size,
            kind: node.kind.clone(),
            deleted_at: crate::util::now_iso(),
            deleted_by: None,
        });
        if rename_with_collision_retry(client, &node.path, &dst, entry.as_ref()).await.is_err() {
            continue;
        }
        // DEC-002: the index follows the trash the same way `fs.delete` does,
        // but only when the project's mode is active; an index failure here is
        // logged, never allowed to undo or fail the move that already happened.
        if let Some(search) = search {
            match admin.get_index_mode(project_id).await {
                Ok(mode) if mode.is_active() => {
                    if let Err(e) = search.delete_path(project_id, &node.path).await {
                        tracing::warn!(
                            project = project_id,
                            path = %node.path,
                            error = %e,
                            "purge cycle: search delete_path failed"
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        project = project_id,
                        error = %e,
                        "purge cycle: could not read index mode"
                    );
                }
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
    pub artifact_sets_swept: usize,
    /// Orphan content blobs, export zips and temp files deleted by the sweep
    /// (SPEC-0034/DR-009), summed over every project swept this cycle.
    pub orphan_blobs_swept: usize,
    /// Sum of the listed sizes of `orphan_blobs_swept` (SPEC-0034/DR-009).
    pub orphan_bytes_swept: u64,
    /// Number of projects whose orphan sweep tripped the circuit breaker
    /// this cycle (SPEC-0034/DR-006, DR-009).
    pub orphan_breakers_tripped: usize,
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
    search: Option<&Arc<dyn SearchBackend>>,
) -> Result<CycleSummary> {
    run_cycle_at(now_unix(), stores, admin, safety, search).await
}

/// Same as [`run_cycle`], but with an explicit `now`, so the orphan sweep's
/// once-a-day gate (SPEC-0034/DR-007) can be driven by a test clock instead
/// of the wall clock (DEC-010).
async fn run_cycle_at(
    now: f64,
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    safety: &SafetyManager,
    search: Option<&Arc<dyn SearchBackend>>,
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
        match sweep_project_files(&client, admin, safety, &project.id, search).await {
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
        match sweep_project_artifacts(&client).await {
            Ok(n) => summary.artifact_sets_swept += n,
            Err(e) => {
                tracing::warn!(project = %project.id, error = %e, "purge cycle: artifact sweep failed");
            }
        }
        // At most once every 24 hours per project per process (SPEC-0034/DR-007);
        // a process that has not swept this project yet sweeps it now.
        let due =
            stores.last_orphan_sweep(&project.id).is_none_or(|last| now - last >= SECONDS_PER_DAY);
        if due {
            match sweep_project_orphans_at(now, stores, &client, &project.id).await {
                Ok(sweep) => {
                    summary.orphan_blobs_swept += sweep.blobs;
                    summary.orphan_bytes_swept += sweep.bytes;
                    if sweep.breaker_tripped {
                        summary.orphan_breakers_tripped += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(project = %project.id, error = %e, "purge cycle: orphan sweep failed");
                }
            }
            // Recorded whether the sweep succeeded or failed, so a failing
            // project is retried after 24 hours, not every cycle (DR-007).
            stores.record_orphan_sweep(&project.id, now);
        }
    }
    Ok(summary)
}

/// Deletes every stale artifact set of `client`'s volume (SPEC-0019 DEC-011)
/// and GCs the blobs that reached zero. A no-op for a client with no
/// relational store behind it (a test double, see `VolumeClient::trash`'s
/// doc). Returns the number of blobs GC'd.
async fn sweep_project_artifacts(client: &VolumeClient) -> Result<usize> {
    sweep_project_artifacts_at(now_unix() as i64, client).await
}

/// Same as [`sweep_project_artifacts`], but with an explicit `now`, for the
/// same boundary-test reason as [`sweep_project_files_at`].
async fn sweep_project_artifacts_at(now: i64, client: &VolumeClient) -> Result<usize> {
    let Some(store) = client.trash.as_deref() else { return Ok(0) };
    let (sets_swept, gc) =
        store.sweep_stale_artifact_sets(now, ARTIFACT_SWEEP_GRACE_SECONDS).await?;
    for sha in gc {
        if let Err(e) = client.blob.delete(&sha).await {
            tracing::warn!(error = %e, "purge cycle: artifact blob delete failed");
        }
    }
    Ok(sets_swept)
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

/// What one project's orphan sweep did (SPEC-0034/DR-003, DR-006).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OrphanSweep {
    /// Objects deleted: content blobs, export zips and temp files.
    pub(crate) blobs: usize,
    /// Sum of the listed sizes of those objects.
    pub(crate) bytes: u64,
    /// The circuit breaker held back this project's content blobs.
    pub(crate) breaker_tripped: bool,
}

/// Deletes the old unreferenced objects of `project_id`'s bucket
/// (SPEC-0034/DR-003..DR-006, DR-008): content blobs no `blob_refs` row holds,
/// export zips no `export_links` row holds, and local temp files, each strictly
/// older than 24 hours. A `git:` key or a key of any other form is never
/// touched.
///
/// The listing is diffed in memory against one bulk read of the referenced
/// hashes (DEC-001); each candidate is then re checked, age and reference,
/// right before its delete (DR-005), because a write can land between the
/// listing and the delete. Too many qualifying content blobs means the
/// reference read is not trustworthy, so none is deleted that run (DR-006).
///
/// A listing or bulk read failure is returned for the caller to log; a backend
/// that cannot list is skipped silently; a failure on one object is logged
/// and skips that object only (DR-008).
pub(crate) async fn sweep_project_orphans_at(
    now: f64,
    stores: &StoreManager,
    client: &VolumeClient,
    project_id: &str,
) -> Result<OrphanSweep> {
    let mut sweep = OrphanSweep::default();
    // A client with no relational store behind it has no reference to diff
    // against, and deleting without one would empty the bucket (DEC-007).
    let Some(store) = client.trash.as_deref() else { return Ok(sweep) };
    let listed = match client.blob.list().await {
        Ok(entries) => entries,
        Err(e) if e.code == crate::errors::code::NOT_SUPPORTED => return Ok(sweep),
        Err(e) => return Err(e),
    };
    let cutoff = now - ORPHAN_MIN_AGE_SECONDS;
    let referenced = store.referenced_hashes().await?;
    let db = crate::storage::open_meta_db(stores.config(), stores.relational(), project_id).await?;

    let content_listed = listed.iter().filter(|e| is_content_key(&e.key)).count();
    let mut content = Vec::new();
    let mut others = Vec::new();
    for entry in listed {
        if entry.modified >= cutoff {
            continue;
        }
        match orphan_kind(&entry) {
            Some(OrphanKind::Content) if !referenced.contains(&entry.key) => content.push(entry),
            // Export rows are few; the re check reads each one, so no bulk read.
            Some(OrphanKind::Export | OrphanKind::Temp) => others.push(entry),
            _ => {}
        }
    }

    let limit = (content_listed / 10).max(1);
    if content.len() > limit {
        tracing::warn!(
            project = %project_id,
            qualifying = content.len(),
            content_blobs = content_listed,
            "orphan sweep: breaker tripped, no content blob deleted"
        );
        sweep.breaker_tripped = true;
        content.clear();
    }

    let candidates: Vec<BlobEntry> = content.into_iter().chain(others).collect();
    if candidates.is_empty() {
        return Ok(sweep);
    }
    for entry in candidates {
        match still_unreferenced(store, &db, project_id, &entry).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(project = %project_id, key = %entry.key, error = %e, "orphan sweep: reference re check failed");
                continue;
            }
        }
        // The time is read per object, last thing before the delete: a re put
        // of existing bytes refreshes it (DR-002), and a second full listing
        // would miss a refresh landing after that listing but before this
        // candidate's turn (DR-005). Orchestrator decision: a per object time
        // read on the blob store rather than a re listing.
        match client.blob.modified(&entry).await {
            Ok(m) if m < cutoff => {}
            Ok(_) => continue,
            Err(e)
                if e.code == crate::errors::code::NOT_FOUND
                    || e.code == crate::errors::code::NOT_SUPPORTED =>
            {
                continue;
            }
            Err(e) => {
                tracing::warn!(project = %project_id, key = %entry.key, error = %e, "orphan sweep: modified time re read failed");
                continue;
            }
        }
        if let Err(e) = client.blob.delete_entry(&entry).await {
            tracing::warn!(project = %project_id, key = %entry.key, error = %e, "orphan sweep: delete failed");
            continue;
        }
        sweep.blobs += 1;
        sweep.bytes += entry.size;
    }
    Ok(sweep)
}

/// The three object forms the orphan sweep may delete (SPEC-0034/DR-003).
enum OrphanKind {
    Content,
    Export,
    Temp,
}

/// `None` for every key the sweep must never touch, `git:` keys included
/// (DR-004). A temp file only counts on the local store, the one backend that
/// reports a shard and writes temp names.
fn orphan_kind(entry: &BlobEntry) -> Option<OrphanKind> {
    if is_content_key(&entry.key) {
        return Some(OrphanKind::Content);
    }
    if entry.key.strip_prefix("export:").is_some_and(|token| !token.is_empty()) {
        return Some(OrphanKind::Export);
    }
    if entry.shard.is_some() && entry.key.starts_with(".tmp-") {
        return Some(OrphanKind::Temp);
    }
    None
}

/// Exactly 64 lowercase hex characters: the shape `sha256_hex` produces.
fn is_content_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The DR-005 reference re check, read fresh right before the delete.
async fn still_unreferenced(
    store: &crate::storage::meta::RelationalMetaStore,
    db: &Arc<dyn crate::storage::rel::RelationalDb>,
    project_id: &str,
    entry: &BlobEntry,
) -> Result<bool> {
    match orphan_kind(entry) {
        Some(OrphanKind::Content) => Ok(!store.blob_is_referenced(&entry.key).await?),
        Some(OrphanKind::Export) => {
            let token = entry.key.strip_prefix("export:").unwrap_or_default();
            let mut tx = db.begin().await?;
            let exists =
                crate::storage::meta::export_link_exists(&mut *tx, project_id, token).await?;
            tx.commit().await?;
            Ok(!exists)
        }
        Some(OrphanKind::Temp) => Ok(true),
        None => Ok(false),
    }
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
/// config, DEC-008), strictly. Removal is [`remove_project`], the very function
/// FR-027's `admin.delete_project` tool calls (volume teardown, search index
/// wipe when `search` is configured, git repo purge when `git.enabled`, then
/// the ACL row delete), unconditionally with respect
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
    search: Option<&Arc<dyn SearchBackend>>,
) -> Result<usize> {
    sweep_grace_period_at(now_unix(), stores, admin, config, search).await
}

/// Same as [`sweep_grace_period`], but with an explicit `now`, for the same
/// boundary-test reason as [`sweep_project_files_at`]/[`sweep_project_at`].
pub(crate) async fn sweep_grace_period_at(
    now: f64,
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    config: &Arc<ServerConfig>,
    search: Option<&Arc<dyn SearchBackend>>,
) -> Result<usize> {
    let threshold = (config.safety.project_purge_grace_days as f64) * SECONDS_PER_DAY;
    let mut removed = 0usize;
    for (project_id, deleted_at) in admin.list_soft_deleted_projects().await? {
        let deleted_unix = parse_iso_to_unix(&deleted_at)?;
        let age = (now - deleted_unix).round();
        if age <= threshold {
            continue;
        }
        remove_project(&project_id, stores, admin, config, search).await?;
        removed += 1;
    }
    Ok(removed)
}

/// Permanently removes a project (SPEC-0024 DEC-003), the one implementation
/// behind both `admin.delete_project` and the grace period purge, so neither
/// path can forget a store the other wipes. In order: the volume (every
/// metadata row and blob reference), the search index when search is
/// configured (Tantivy directory, sqlite-vec cache, PostgreSQL rows), the git
/// repository when git is enabled, and the project record last.
///
/// The record goes last so a failure at any step leaves the project listed and
/// deletable again: every step is idempotent, so a retry finishes the job
/// instead of a recreated project inheriting what an earlier step missed.
pub(crate) async fn remove_project(
    project_id: &str,
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    config: &Arc<ServerConfig>,
    search: Option<&Arc<dyn SearchBackend>>,
) -> Result<()> {
    remove_project_with_git(project_id, stores, admin, config, search, None).await
}

/// [`remove_project`] with the git store a caller already holds, so its in
/// process entry cache is the one purged (the test registry injects its own).
pub(crate) async fn remove_project_with_git(
    project_id: &str,
    stores: &StoreManager,
    admin: &dyn AdminBackend,
    config: &Arc<ServerConfig>,
    search: Option<&Arc<dyn SearchBackend>>,
    git: Option<Arc<GitRepoStore>>,
) -> Result<()> {
    stores.teardown_volume(project_id).await?;
    if let Some(search) = search {
        search.delete_all(project_id).await?;
    }
    if config.git.enabled {
        let store = git
            .unwrap_or_else(|| GitRepoStore::shared(config.clone(), stores.relational().clone()));
        store.purge_repo(project_id).await?;
    }
    admin.delete_project(project_id).await
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
        safety.attach_no_limits();
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

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
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

        let purged = sweep_project_files_at(now, &f.client, &f.admin, &f.safety, "proj", None)
            .await
            .unwrap();
        assert_eq!(purged, 0, "strict > required, not >=");
        assert!(f.client.exists("/edge.txt").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_new_026_boundary_just_past_threshold_purged() {
        let f = fixture().await;
        let now = now_unix();
        seed_stale_file_at(&f, "/past.txt", now, 7.0 * DAY + 1.0).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let purged = sweep_project_files_at(now, &f.client, &f.admin, &f.safety, "proj", None)
            .await
            .unwrap();
        assert_eq!(purged, 1);
        assert!(!f.client.exists("/past.txt").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_new_027_use_internal_purge_false_gates_the_sweep_off() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 10.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, false, Some(7), None).await.unwrap();

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
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
            admission: Arc::default(),
        };

        let purged =
            sweep_project_files(&flaky_client, &f.admin, &f.safety, "proj", None).await.unwrap();
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

        let first =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
        assert_eq!(first, 1);
        let trash_root = format!("/{}", f.safety.config().trash_dir);
        let after_first = f.client.meta.subtree(&trash_root).await.unwrap().len();

        let second =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
        assert_eq!(second, 0, "re-purging an already-trashed file is a no-op");
        let after_second = f.client.meta.subtree(&trash_root).await.unwrap().len();
        assert_eq!(after_first, after_second, "no duplicate trash entry");
    }

    #[tokio::test]
    async fn e2e_new_030_empty_project() {
        let f = fixture().await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(7), None).await.unwrap();

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
        assert_eq!(purged, 0);
    }

    #[tokio::test]
    async fn e2e_new_031_file_retention_days_none_skips_the_file_step() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 100.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, None, Some(30)).await.unwrap();

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
        assert_eq!(purged, 0, "file step skipped independently of project_retention_days");
        assert!(f.client.exists("/stale.txt").await.unwrap());
    }

    #[tokio::test]
    async fn unconfigured_project_is_a_no_op() {
        let f = fixture().await;
        seed_stale_file(&f, "/stale.txt", 100.0 * DAY).await;
        // No seed_purge_config_for_test call: the project has never been configured.

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
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

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
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

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
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

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
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
        async fn rename_into_trash(
            &self,
            _src: &str,
            _dst: &str,
            _entry: &crate::storage::traits::TrashEntry,
        ) -> Result<()> {
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
        async fn used_bytes(&self) -> Result<i64> {
            self.inner.used_bytes().await
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

        let purged = sweep_project_files(&colliding_client, &f.admin, &f.safety, "proj", None)
            .await
            .unwrap();
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
        /// Statement fragment that triggers the failure (`INSERT INTO trash_entries`
        /// for E2E-NEW-445, `UPDATE nodes` for SPEC-0044 DT-004).
        needle: &'static str,
        /// Matching statements to let through before the one that fails.
        pass: Arc<std::sync::atomic::AtomicUsize>,
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
            Ok(Box::new(FailTrashInsertOnceTx {
                inner: tx,
                armed: self.armed.clone(),
                needle: self.needle,
                pass: self.pass.clone(),
            }))
        }
        async fn migrate(&self, schema: &crate::storage::rel::SchemaSet) -> Result<()> {
            self.inner.migrate(schema).await
        }
    }

    struct FailTrashInsertOnceTx {
        inner: Box<dyn RelationalTx>,
        armed: Arc<std::sync::atomic::AtomicBool>,
        needle: &'static str,
        pass: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl RelationalTx for FailTrashInsertOnceTx {
        fn dialect(&self) -> Dialect {
            self.inner.dialect()
        }
        async fn execute(&mut self, query: &Query) -> Result<u64> {
            use std::sync::atomic::Ordering::SeqCst;
            if query.sql.contains(self.needle) && self.armed.load(SeqCst) {
                if self.pass.load(SeqCst) > 0 {
                    self.pass.fetch_sub(1, SeqCst);
                } else if self.armed.swap(false, SeqCst) {
                    return Err(ToolError::internal("simulated statement failure"));
                }
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
    async fn dt_010_a_failed_trash_entries_insert_leaves_the_file_live_and_unmoved() {
        let (_d, meta, client, armed) = faulty_store("INSERT INTO trash_entries", 0).await;
        let client = client.with_trash_store(meta.clone());
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        admin.create_project("proj", "owner@t.c").await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);
        safety.attach_no_limits();

        client.write_text_atomic("/stale2.txt", "data").await.unwrap();
        meta.set_atime_for_test("/stale2.txt", now_unix() - 8.0 * DAY).await.unwrap();
        admin.seed_purge_config_for_test("proj", true, true, Some(1), None).await.unwrap();
        let rev_before = meta.ensure_node_rev("/stale2.txt").await.unwrap();
        armed.store(true, std::sync::atomic::Ordering::SeqCst);

        let purged = sweep_project_files(&client, &admin, &safety, "proj", None).await.unwrap();
        assert_eq!(purged, 0, "the per-file failure is isolated, matching purge.rs:26");
        assert!(client.exists("/stale2.txt").await.unwrap(), "the file remains live, not trashed");
        assert_eq!(
            meta.ensure_node_rev("/stale2.txt").await.unwrap(),
            rev_before,
            "the file never moved: a rename back would have stamped a new revision"
        );
        let trash_root = format!("/{}", safety.config().trash_dir);
        let moved: Vec<_> = meta
            .subtree(&trash_root)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.kind != "dir")
            .collect();
        assert!(moved.is_empty(), "no file moved into the trash dir: {moved:?}");
        assert_eq!(meta.count_trash_entries_for_test().await.unwrap(), 0, "no orphaned row");
    }

    #[tokio::test]
    async fn dt_011_a_swept_file_gets_a_system_initiated_row_with_the_same_shape() {
        let f = fixture().await;
        seed_stale_file(&f, "/shape.txt", 4.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, true, Some(1), None).await.unwrap();

        let purged =
            sweep_project_files(&f.client, &f.admin, &f.safety, "proj", None).await.unwrap();
        assert_eq!(purged, 1);

        let trash_root = format!("/{}", f.safety.config().trash_dir);
        let row = find_trash_row_for(&f, &trash_root, "/shape.txt").await;
        assert_eq!((row.size, row.kind.as_str()), (4, "file"));
        assert!(row.deleted_by.is_none());
        assert!(f.meta.get("/shape.txt").await.unwrap().is_none(), "moved out of its place");
        assert!(f.meta.get(&row.trash_path).await.unwrap().is_some(), "node sits at the row's key");
    }

    // ── SPEC-0044 US-0052: rename_into_trash is one transaction ───────────────

    /// A store over a database that fails the `n`th-plus-one statement matching
    /// `needle` once armed, plus the handle that arms it and a client for seeding.
    async fn faulty_store(
        needle: &'static str,
        pass: usize,
    ) -> (
        tempfile::TempDir,
        Arc<RelationalMetaStore>,
        VolumeClient,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        use crate::storage::rel::SqliteRelationalDb;
        let dir = tempfile::tempdir().unwrap();
        let real: Arc<dyn RelationalDb> = Arc::new(SqliteRelationalDb::open_in_memory().unwrap());
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let db: Arc<dyn RelationalDb> = Arc::new(FailTrashInsertOnceDb {
            inner: real,
            armed: armed.clone(),
            needle,
            pass: Arc::new(std::sync::atomic::AtomicUsize::new(pass)),
        });
        let meta = Arc::new(RelationalMetaStore::open(db, "proj").await.unwrap());
        let blob = Arc::new(LocalBlobStore::new(dir.path(), "mcpfs-test"));
        let client = VolumeClient::new("proj", meta.clone(), blob);
        (dir, meta, client, armed)
    }

    fn entry_for(original: &str, size: i64, kind: &str) -> crate::storage::traits::TrashEntry {
        crate::storage::traits::TrashEntry {
            original_path: original.to_string(),
            size,
            kind: kind.to_string(),
            deleted_at: "2026-10-19T00:00:00Z".to_string(),
            deleted_by: Some("ada".to_string()),
        }
    }

    #[tokio::test]
    async fn dt_003_insert_failure_undoes_the_rename() {
        let (_d, meta, client, armed) = faulty_store("INSERT INTO trash_entries", 0).await;
        client.write_text_atomic("/a.txt", "x").await.unwrap();
        client.makedirs("/.mcp_trash", true).await.unwrap();
        armed.store(true, std::sync::atomic::Ordering::SeqCst);

        let r = meta
            .rename_into_trash("/a.txt", "/.mcp_trash/x", &entry_for("/a.txt", 1, "file"))
            .await;
        let e = r.unwrap_err();
        assert!(
            e.message.contains("simulated statement failure"),
            "the insert ran and failed: {e}"
        );
        assert!(client.exists("/a.txt").await.unwrap(), "src is still live");
        assert!(!client.exists("/.mcp_trash/x").await.unwrap(), "nothing at dst");
        assert_eq!(meta.count_trash_entries_for_test().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn dt_004_rename_failure_leaves_no_row() {
        // Let the parent's UPDATE through, fail the next descendant's.
        let (_d, meta, client, armed) = faulty_store("UPDATE nodes SET path", 1).await;
        for f in ["/d/1.txt", "/d/2.txt", "/d/3.txt"] {
            client.write_text_atomic(f, "x").await.unwrap();
        }
        client.makedirs("/.mcp_trash", true).await.unwrap();
        armed.store(true, std::sync::atomic::Ordering::SeqCst);

        let r = meta.rename_into_trash("/d", "/.mcp_trash/d", &entry_for("/d", 0, "dir")).await;
        let e = r.unwrap_err();
        assert!(
            e.message.contains("simulated statement failure"),
            "the UPDATE failed mid tree: {e}"
        );
        for p in ["/d", "/d/1.txt", "/d/2.txt", "/d/3.txt"] {
            assert!(client.exists(p).await.unwrap(), "{p} did not move");
        }
        assert!(!client.exists("/.mcp_trash/d").await.unwrap(), "no half moved tree");
        assert_eq!(meta.count_trash_entries_for_test().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn dt_005_success_is_visible_together() {
        let f = fixture().await;
        f.client.write_text_atomic("/a.txt", "x").await.unwrap();
        f.client.makedirs("/.mcp_trash", true).await.unwrap();
        let entry = entry_for("/a.txt", 1, "file");
        f.meta.rename_into_trash("/a.txt", "/.mcp_trash/x", &entry).await.unwrap();

        assert!(!f.client.exists("/a.txt").await.unwrap());
        assert!(f.client.exists("/.mcp_trash/x").await.unwrap());
        let row = f.meta.get_trash_entry_for_test("/.mcp_trash/x").await.unwrap().unwrap();
        assert_eq!(row.original_path, entry.original_path);
        assert_eq!((row.size, row.kind.as_str()), (1, "file"));
        assert_eq!(row.deleted_at, entry.deleted_at);
        assert_eq!(row.deleted_by.as_deref(), Some("ada"));
    }

    #[tokio::test]
    async fn dt_006_directory_move_writes_one_row() {
        let f = fixture().await;
        for p in ["/d/1.txt", "/d/2.txt", "/d/3.txt"] {
            f.client.write_text_atomic(p, "x").await.unwrap();
        }
        f.client.makedirs("/.mcp_trash", true).await.unwrap();
        f.meta.rename_into_trash("/d", "/.mcp_trash/d", &entry_for("/d", 0, "dir")).await.unwrap();

        for p in
            ["/.mcp_trash/d", "/.mcp_trash/d/1.txt", "/.mcp_trash/d/2.txt", "/.mcp_trash/d/3.txt"]
        {
            assert!(f.client.exists(p).await.unwrap(), "{p} moved");
        }
        assert!(!f.client.exists("/d").await.unwrap());
        assert_eq!(f.meta.count_trash_entries_for_test().await.unwrap(), 1);
        let row = f.meta.get_trash_entry_for_test("/.mcp_trash/d").await.unwrap().unwrap();
        assert_eq!(row.kind, "dir");
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
        safety.attach_no_limits();

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

        let summary = run_cycle(&stores, &admin, &safety, None).await.unwrap();
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
        safety.attach_no_limits();

        let summary = run_cycle(&stores, &admin, &safety, None).await.unwrap();
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
        safety.attach_no_limits();

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
        let summary = run_cycle(&stores, &admin, &safety, None).await.unwrap();
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

        let removed =
            sweep_grace_period_at(now, &f.stores, &f.admin, &f.config, None).await.unwrap();
        assert_eq!(removed, 1);
        assert!(f.admin.get_project("proj").await.unwrap().is_none(), "permanently removed");
    }

    #[tokio::test]
    async fn e2e_new_054_boundary_exactly_at_grace_period_not_removed() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 30.0 * DAY).await;

        let removed =
            sweep_grace_period_at(now, &f.stores, &f.admin, &f.config, None).await.unwrap();
        assert_eq!(removed, 0, "strict > required, not >=");
        assert!(f.admin.get_project("proj").await.unwrap().is_some(), "not yet removed");
    }

    #[tokio::test]
    async fn e2e_new_055_boundary_just_past_grace_period_removed() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 30.0 * DAY + 1.0).await;

        let removed =
            sweep_grace_period_at(now, &f.stores, &f.admin, &f.config, None).await.unwrap();
        assert_eq!(removed, 1);
        assert!(f.admin.get_project("proj").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn e2e_new_056_unconditional_on_use_internal_purge() {
        let f = grace_fixture().await;
        let now = now_unix();
        seed_soft_deleted(&f, "proj", now, 31.0 * DAY).await;
        f.admin.seed_purge_config_for_test("proj", true, false, None, None).await.unwrap();

        let removed =
            sweep_grace_period_at(now, &f.stores, &f.admin, &f.config, None).await.unwrap();
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

        let removed =
            sweep_grace_period_at(now, &f.stores, &f.admin, &f.config, None).await.unwrap();

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
        let err =
            sweep_grace_period_at(now, &f.stores, &failing, &f.config, None).await.unwrap_err();
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
        async fn rename_into_trash(
            &self,
            src: &str,
            dst: &str,
            entry: &crate::storage::traits::TrashEntry,
        ) -> Result<()> {
            if src == self.fail_path {
                return Err(crate::errors::ToolError::internal(format!(
                    "simulated locked row for '{src}'"
                )));
            }
            self.inner.rename_into_trash(src, dst, entry).await
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
        async fn used_bytes(&self) -> Result<i64> {
            self.inner.used_bytes().await
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
        safety.attach_no_limits();
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
        let result = run_cycle(&f.stores, &f.admin, &f.safety, None).await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// E2E-NEW-032: the `export_links` row is gone after the sweep.
    #[tokio::test]
    async fn e2e_new_032_sweep_removes_the_export_link_row() {
        let f = export_fixture().await;
        seed_export(&f, "T", -3600.0, b"zip bytes").await;
        assert_eq!(export_row_count(&f, "T").await, 1, "row present before the sweep");
        let summary = run_cycle(&f.stores, &f.admin, &f.safety, None).await.unwrap();
        assert_eq!(export_row_count(&f, "T").await, 0);
        assert_eq!(summary.exports_swept, 1);
    }

    /// E2E-NEW-033: the export blob is gone after the sweep.
    #[tokio::test]
    async fn e2e_new_033_sweep_removes_the_export_blob() {
        let f = export_fixture().await;
        seed_export(&f, "T", -3600.0, b"zip bytes").await;
        assert!(export_blob_present(&f, "T").await, "blob present before the sweep");
        run_cycle(&f.stores, &f.admin, &f.safety, None).await.unwrap();
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

        let result = run_cycle(&f.stores, &f.admin, &f.safety, None).await;
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

        let summary = run_cycle(&f.stores, &f.admin, &f.safety, None).await.unwrap();
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
        let summary = run_cycle(&f.stores, &f.admin, &f.safety, None).await.unwrap();
        assert_eq!(summary.exports_swept, 0);
    }

    // ── artifact sweep (SPEC-0019 US-0013, DEC-011) ─────────────────────────────

    use crate::storage::meta::{NewArtifactSet, NewDocImage};

    fn one_image(sha: &str) -> NewDocImage {
        NewDocImage {
            seq: 1,
            caption: "cap".into(),
            page: None,
            format: "png".into(),
            sha256: sha.into(),
            size: 3,
            line_start: 0,
            line_count: 1,
        }
    }

    /// Converts `path` in `proj` (writes the bytes, commits a one image set
    /// built from `node_rev`) and returns the image's sha, so a test can later
    /// assert its blob is gone.
    async fn convert(f: &ExportFix, path: &str, bytes: &[u8]) -> String {
        let client = f.stores.client("proj").await.unwrap();
        client.write_bytes_atomic(path, bytes).await.unwrap();
        let store = client.trash.clone().expect("a real relational store");
        let node_rev = store.ensure_node_rev(path).await.unwrap().expect("a rev");
        let tag = path.trim_start_matches('/').replace(['/', '.'], "-");
        let image_sha = format!("sha-img-{tag}");
        client.blob.put(&image_sha, b"pixels").await.unwrap();
        let set = NewArtifactSet {
            set_id: format!("set-{tag}"),
            path: path.to_string(),
            node_rev,
            text_sha: Some(format!("sha-text-{tag}")),
            text_len: 4,
            report: "1 images, 0 tables".into(),
            images: vec![one_image(&image_sha)],
            tables: Vec::new(),
        };
        client.blob.put(set.text_sha.as_deref().unwrap(), b"text").await.unwrap();
        store.commit_artifact_set(&set).await.unwrap();
        image_sha
    }

    /// Independent Test (US-0013): convert `/report.pdf`, move it, run
    /// `run_cycle`, and observe the stale set's blob references gone.
    #[tokio::test]
    async fn e2e_new_independent_run_cycle_sweeps_a_set_orphaned_by_a_move() {
        let f = export_fixture().await;
        let image_sha = convert(&f, "/report.pdf", b"%PDF-1.4").await;
        let client = f.stores.client("proj").await.unwrap();
        client.rename("/report.pdf", "/moved.pdf").await.unwrap();

        let summary = run_cycle(&f.stores, &f.admin, &f.safety, None).await.unwrap();

        assert!(summary.artifact_sets_swept >= 1, "{summary:?}");
        assert!(!client.blob.exists(&image_sha).await.unwrap(), "the orphaned image is GC'd");
        assert!(
            !client.blob.exists("sha-text-report-pdf").await.unwrap(),
            "the orphaned conversion text is GC'd"
        );

        let store = client.trash.clone().expect("a real relational store");
        assert_eq!(
            store.count_artifact_set_rows_for_test("set-report-pdf").await.unwrap(),
            0,
            "the swept set's doc_artifact_sets row is gone"
        );
        assert_eq!(
            store.count_doc_image_rows_for_test("set-report-pdf").await.unwrap(),
            0,
            "the swept set's doc_images rows are gone"
        );
        assert_eq!(
            store.blob_refcount_for_test(&image_sha).await.unwrap(),
            None,
            "the swept image's blob_refs row is gone"
        );
        assert_eq!(
            store.blob_refcount_for_test("sha-text-report-pdf").await.unwrap(),
            None,
            "the swept text's blob_refs row is gone"
        );
    }

    /// A superseded set inside its grace window is left alone; a document that
    /// never changed keeps its current set forever.
    #[tokio::test]
    async fn e2e_new_artifact_sweep_respects_the_grace_period_and_the_current_set() {
        let f = export_fixture().await;
        let old_sha = convert(&f, "/report.pdf", b"%PDF-1.4 v1").await;
        let client = f.stores.client("proj").await.unwrap();
        let store = client.trash.clone().unwrap();

        // Re-converting overwrites the sibling and node bytes, superseding the
        // first set rather than deleting it outright (DEC-002).
        client.write_bytes_atomic("/report.pdf", b"%PDF-1.4 v2").await.unwrap();
        let node_rev = store.ensure_node_rev("/report.pdf").await.unwrap().unwrap();
        let new_sha = "sha-img-v2";
        client.blob.put(new_sha, b"pixels2").await.unwrap();
        let current = NewArtifactSet {
            set_id: "set-v2".into(),
            path: "/report.pdf".into(),
            node_rev,
            text_sha: Some("sha-text-v2".into()),
            text_len: 4,
            report: "1 images, 0 tables".into(),
            images: vec![one_image(new_sha)],
            tables: Vec::new(),
        };
        client.blob.put("sha-text-v2", b"text2").await.unwrap();
        store.commit_artifact_set(&current).await.unwrap();

        // Too soon: the just superseded first set must survive this cycle.
        let summary = run_cycle(&f.stores, &f.admin, &f.safety, None).await.unwrap();
        assert_eq!(summary.artifact_sets_swept, 0, "inside the 10 minute grace period");
        assert!(client.blob.exists(&old_sha).await.unwrap(), "not yet GC'd");
        assert!(client.blob.exists(new_sha).await.unwrap(), "the current image is untouched");

        // Force the stale branch via the explicit clock, as every other sweep
        // in this module does for its boundary tests.
        let far_future = now_unix() + 700.0;
        let swept = sweep_project_artifacts_at(far_future as i64, &client).await.unwrap();
        assert!(swept >= 1, "the superseded set is reclaimed once past its grace period");
        assert!(!client.blob.exists(&old_sha).await.unwrap(), "the superseded image is GC'd");
        assert!(
            client.blob.exists(new_sha).await.unwrap(),
            "the current set of an unchanged document is never swept"
        );
    }

    // SPEC-0034 orphan sweep (US-0038). Every key lives in a real local blob
    // store under a temp dir, behind a real relational meta store; ages are set
    // with `File::set_modified` against an injected whole second `now`, so a
    // boundary age reads back exactly.

    const H25: f64 = 25.0 * 3600.0;
    const H1: f64 = 3600.0;
    /// Bounds the wait for the sweep's listing, so a sweep that never lists
    /// fails as an assertion rather than hanging.
    const PAUSE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

    struct OrphanFix {
        dir: tempfile::TempDir,
        stores: StoreManager,
        admin: RelationalAdminStore,
        safety: SafetyManager,
        now: f64,
    }

    async fn orphan_fixture() -> OrphanFix {
        let dir = tempfile::tempdir().unwrap();
        let mut raw_config = crate::config::ServerConfig::default();
        raw_config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        raw_config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        let stores = StoreManager::new(
            Arc::new(raw_config),
            Arc::new(crate::storage::RelationalRegistry::new()),
        );
        let admin = RelationalAdminStore::in_memory().await.unwrap();
        let safety = SafetyManager::new(SafetyConfig::default(), None);
        safety.attach_no_limits();
        OrphanFix { dir, stores, admin, safety, now: now_unix().floor() }
    }

    impl OrphanFix {
        async fn client(&self, project: &str) -> Arc<VolumeClient> {
            self.stores.client(project).await.unwrap()
        }

        /// Runs a full purge cycle at `now` over this fixture's real admin
        /// and safety stores (SPEC-0034 DT-022, DT-023, DT-025, DT-026).
        async fn run_cycle_at(&self, now: f64) -> Result<CycleSummary> {
            run_cycle_at(now, &self.stores, &self.admin, &self.safety, None).await
        }

        fn bucket_root(&self, project: &str) -> std::path::PathBuf {
            self.dir.path().join("blobs").join(self.stores.config().volume_bucket(project))
        }

        /// The one file named `key` anywhere in `project`'s bucket, if any.
        fn file_of(&self, project: &str, key: &str) -> Option<std::path::PathBuf> {
            let root = self.bucket_root(project);
            for shard in std::fs::read_dir(&root).ok()?.flatten() {
                let candidate = shard.path().join(key);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            None
        }

        fn present(&self, project: &str, key: &str) -> bool {
            self.file_of(project, key).is_some()
        }

        fn age(&self, project: &str, key: &str, age_seconds: f64) {
            let path = self.file_of(project, key).expect("seeded key");
            let when =
                std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(self.now - age_seconds);
            std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
        }

        /// Puts `bytes` under `key` and backdates it by `age_seconds`.
        async fn put(&self, project: &str, key: &str, bytes: &[u8], age_seconds: f64) {
            self.client(project).await.blob.put(key, bytes).await.unwrap();
            self.age(project, key, age_seconds);
        }

        /// Puts `bytes` under their own hash, returning it.
        async fn put_content(&self, project: &str, bytes: &[u8], age_seconds: f64) -> String {
            let sha = VolumeClient::sha256_hex(bytes);
            self.put(project, &sha, bytes, age_seconds).await;
            sha
        }

        /// A temp file written by hand in shard `ab`, where `put` leaves one.
        fn put_tmp(&self, project: &str, name: &str, bytes: &[u8], age_seconds: f64) {
            let shard = self.bucket_root(project).join("ab");
            std::fs::create_dir_all(&shard).unwrap();
            std::fs::write(shard.join(name), bytes).unwrap();
            self.age(project, name, age_seconds);
        }

        async fn add_refs(&self, project: &str, shas: &[String]) {
            let client = self.client(project).await;
            client.trash.as_ref().unwrap().add_blob_refs_for_test(shas).await.unwrap();
        }

        /// `count` referenced content blobs, each `age_seconds` old.
        async fn put_referenced(
            &self,
            project: &str,
            count: usize,
            age_seconds: f64,
        ) -> Vec<String> {
            let mut shas = Vec::with_capacity(count);
            for i in 0..count {
                shas.push(
                    self.put_content(project, format!("ref-{i}").as_bytes(), age_seconds).await,
                );
            }
            self.add_refs(project, &shas).await;
            shas
        }

        async fn add_export_row(&self, project: &str, token: &str) {
            self.client(project).await;
            let db = crate::storage::open_meta_db(
                self.stores.config(),
                self.stores.relational(),
                project,
            )
            .await
            .unwrap();
            let mut tx = db.begin().await.unwrap();
            let now = chrono::Utc::now();
            crate::storage::meta::insert_export_link(
                &mut *tx,
                token,
                project,
                &crate::tools::export::iso(now),
                &crate::tools::export::iso(now + chrono::Duration::days(7)),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }

        async fn sweep(&self, project: &str) -> Result<OrphanSweep> {
            let client = self.client(project).await;
            sweep_project_orphans_at(self.now, &self.stores, &client, project).await
        }
    }

    /// A blob double over the real local store: optionally pauses once right
    /// after its first listing, and optionally fails its `n`th `delete_entry`.
    struct SweepBlob {
        inner: Arc<dyn crate::storage::traits::BlobBackend>,
        pause_armed: std::sync::atomic::AtomicBool,
        listed: tokio::sync::Notify,
        resume: tokio::sync::Notify,
        deletes: std::sync::atomic::AtomicUsize,
        fail_delete: Option<usize>,
        delete_pause_armed: std::sync::atomic::AtomicBool,
        deleting: tokio::sync::Notify,
        first_deleted: std::sync::Mutex<Option<String>>,
    }

    impl SweepBlob {
        fn new(
            inner: Arc<dyn crate::storage::traits::BlobBackend>,
            pause: bool,
            fail_delete: Option<usize>,
        ) -> Self {
            Self {
                inner,
                pause_armed: std::sync::atomic::AtomicBool::new(pause),
                listed: tokio::sync::Notify::new(),
                resume: tokio::sync::Notify::new(),
                deletes: std::sync::atomic::AtomicUsize::new(0),
                fail_delete,
                delete_pause_armed: std::sync::atomic::AtomicBool::new(false),
                deleting: tokio::sync::Notify::new(),
                first_deleted: std::sync::Mutex::new(None),
            }
        }

        /// Also pause once right before the first `delete_entry`, recording
        /// its key: the window after every listing and before a later
        /// candidate's delete, whatever number of listings the sweep makes.
        fn with_delete_pause(self) -> Self {
            self.delete_pause_armed.store(true, std::sync::atomic::Ordering::SeqCst);
            self
        }
    }

    #[async_trait]
    impl crate::storage::traits::BlobBackend for SweepBlob {
        async fn put(&self, sha256: &str, data: &[u8]) -> Result<()> {
            self.inner.put(sha256, data).await
        }
        async fn get(&self, sha256: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
            self.inner.get(sha256, offset, length).await
        }
        async fn exists(&self, sha256: &str) -> Result<bool> {
            self.inner.exists(sha256).await
        }
        async fn delete(&self, sha256: &str) -> Result<()> {
            self.inner.delete(sha256).await
        }
        async fn ensure_bucket(&self) -> Result<()> {
            self.inner.ensure_bucket().await
        }
        async fn remove_bucket(&self) -> Result<()> {
            self.inner.remove_bucket().await
        }
        async fn list(&self) -> Result<Vec<crate::storage::traits::BlobEntry>> {
            let entries = self.inner.list().await;
            if self.pause_armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                self.listed.notify_one();
                self.resume.notified().await;
            }
            entries
        }
        async fn modified(&self, entry: &crate::storage::traits::BlobEntry) -> Result<f64> {
            self.inner.modified(entry).await
        }
        async fn delete_entry(&self, entry: &crate::storage::traits::BlobEntry) -> Result<()> {
            if self.delete_pause_armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                *self.first_deleted.lock().unwrap() = Some(entry.key.clone());
                self.deleting.notify_one();
                self.resume.notified().await;
            }
            let n = self.deletes.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if self.fail_delete == Some(n) {
                return Err(ToolError::internal("injected delete failure"));
            }
            self.inner.delete_entry(entry).await
        }
    }

    /// A client over `project`'s real stores whose blob backend is `blob`.
    async fn client_with_blob(f: &OrphanFix, project: &str, blob: Arc<SweepBlob>) -> VolumeClient {
        let real = f.client(project).await;
        VolumeClient::new(project, real.meta.clone(), blob)
            .with_trash_store(real.trash.clone().unwrap())
    }

    /// SPEC-0034/DT-014: exactly the old orphan, the rowless export zip and
    /// the old temp file go; every other key stays.
    #[tokio::test]
    async fn dt_014_sweep_deletes_exactly_the_qualifying_objects() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 20, H25).await;
        let a = f.put_content(p, b"referenced A", H25).await;
        f.add_refs(p, std::slice::from_ref(&a)).await;
        let o = f.put_content(p, b"orphan O", H25).await;
        let y = f.put_content(p, b"young Y", H1).await;
        let git = format!("git:{}", VolumeClient::sha256_hex(b"a git object"));
        f.put(p, &git, b"a git object", H25).await;
        f.put(p, "export:t1", b"zip one", H25).await;
        f.add_export_row(p, "t1").await;
        f.put(p, "export:t2", b"zip two", H25).await;
        f.put_tmp(p, ".tmp-x", b"partial", H25);
        f.put_tmp(p, ".tmp-y", b"partial", H1);
        f.put(p, "misc:k", b"other", H25).await;

        let swept = f.sweep(p).await.unwrap();

        for gone in [o.as_str(), "export:t2", ".tmp-x"] {
            assert!(!f.present(p, gone), "{gone} is swept");
        }
        for kept in [a.as_str(), y.as_str(), git.as_str(), "export:t1", ".tmp-y", "misc:k"] {
            assert!(f.present(p, kept), "{kept} is kept");
        }
        assert_eq!(swept.blobs, 3);
        assert!(!swept.breaker_tripped);
    }

    /// SPEC-0034/DT-015: "older than 24 hours" is strict.
    #[tokio::test]
    async fn dt_015_exactly_24_hours_is_kept_one_second_more_is_swept() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 20, H1).await;
        let at = f.put_content(p, b"exactly a day", DAY).await;
        let past = f.put_content(p, b"a day and a second", DAY + 1.0).await;

        f.sweep(p).await.unwrap();

        assert!(f.present(p, &at), "86 400 s old is kept");
        assert!(!f.present(p, &past), "86 401 s old is swept");
    }

    /// SPEC-0034/DT-016: only 64 lowercase hex characters make a content key.
    #[tokio::test]
    async fn dt_016_near_miss_content_keys_are_never_candidates() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 20, H1).await;
        let sha = VolumeClient::sha256_hex(b"near miss");
        let upper = sha.to_uppercase();
        let short = sha[..63].to_string();
        f.put(p, &upper, b"near miss", H25).await;
        f.put(p, &short, b"near miss", H25).await;
        let control = f.put_content(p, b"real orphan", H25).await;

        f.sweep(p).await.unwrap();

        assert!(f.present(p, &upper), "uppercase hex is not a content key");
        assert!(f.present(p, &short), "63 characters is not a content key");
        assert!(!f.present(p, &control), "the control orphan is swept");
    }

    /// SPEC-0034/DT-017: a file written between the listing and the delete
    /// keeps its bytes.
    #[tokio::test]
    async fn dt_017_a_write_during_the_sweep_keeps_its_blob() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 20, H1).await;
        let bytes = "orphan O, soon written";
        let o = f.put_content(p, bytes.as_bytes(), H25).await;
        let real = f.client(p).await;
        let double = Arc::new(SweepBlob::new(real.blob.clone(), true, None));
        let client = client_with_blob(&f, p, double.clone()).await;

        let sweep = sweep_project_orphans_at(f.now, &f.stores, &client, p);
        let writer = async {
            let listed = tokio::time::timeout(PAUSE_WAIT, double.listed.notified()).await;
            assert!(listed.is_ok(), "the sweep lists the bucket before deleting");
            real.write_text_atomic("/doc.txt", bytes).await.unwrap();
            double.resume.notify_one();
        };
        let (swept, ()) = tokio::join!(sweep, writer);

        swept.unwrap();
        assert!(f.present(p, &o), "the blob the new file references is kept");
        let read = real.read_text("/doc.txt").await.unwrap();
        assert_eq!(read, bytes);
    }

    /// SPEC-0034/DT-017b (DR-005): a re put refreshing an orphan's time after
    /// every listing, but before that orphan's own delete, keeps it, even with
    /// no reference added. A fresh time read per candidate is the only check
    /// that sees it.
    #[tokio::test]
    async fn dt_017b_a_time_refresh_just_before_the_delete_keeps_the_blob() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 20, H1).await;
        let a = f.put_content(p, b"orphan A", H25).await;
        let b = f.put_content(p, b"orphan B", H25).await;
        let real = f.client(p).await;
        let double = Arc::new(SweepBlob::new(real.blob.clone(), false, None).with_delete_pause());
        let client = client_with_blob(&f, p, double.clone()).await;

        let sweep = sweep_project_orphans_at(f.now, &f.stores, &client, p);
        let writer = async {
            let paused = tokio::time::timeout(PAUSE_WAIT, double.deleting.notified()).await;
            assert!(paused.is_ok(), "the sweep deletes a first orphan");
            let first = double.first_deleted.lock().unwrap().clone().unwrap();
            let (other, bytes): (&str, &[u8]) =
                if first == a { (&b, b"orphan B") } else { (&a, b"orphan A") };
            real.blob.put(other, bytes).await.unwrap();
            double.resume.notify_one();
            (first, other.to_string())
        };
        let (swept, (first, refreshed)) = tokio::join!(sweep, writer);

        swept.unwrap();
        assert!(!f.present(p, &first), "the untouched orphan is swept");
        assert!(f.present(p, &refreshed), "the orphan refreshed before its delete is kept");
    }

    /// SPEC-0034/DT-018: a reference added between the listing and the delete
    /// keeps the blob, even with no time refresh.
    #[tokio::test]
    async fn dt_018_a_reference_added_during_the_sweep_keeps_the_blob() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 20, H1).await;
        let o = f.put_content(p, b"orphan O", H25).await;
        let control = f.put_content(p, b"still an orphan", H25).await;
        let real = f.client(p).await;
        let double = Arc::new(SweepBlob::new(real.blob.clone(), true, None));
        let client = client_with_blob(&f, p, double.clone()).await;

        let sweep = sweep_project_orphans_at(f.now, &f.stores, &client, p);
        let referrer = async {
            let listed = tokio::time::timeout(PAUSE_WAIT, double.listed.notified()).await;
            assert!(listed.is_ok(), "the sweep lists the bucket before deleting");
            f.add_refs(p, std::slice::from_ref(&o)).await;
            double.resume.notify_one();
        };
        let (swept, ()) = tokio::join!(sweep, referrer);

        swept.unwrap();
        assert!(f.present(p, &o), "the newly referenced blob is kept");
        assert!(!f.present(p, &control), "the control orphan is swept");
    }

    /// SPEC-0034/DT-019: at the breaker limit, everything qualifying goes.
    #[tokio::test]
    async fn dt_019_orphans_at_the_limit_are_swept() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 18, H1).await;
        let o1 = f.put_content(p, b"orphan 1", H25).await;
        let o2 = f.put_content(p, b"orphan 2", H25).await;

        let swept = f.sweep(p).await.unwrap();

        assert!(!f.present(p, &o1) && !f.present(p, &o2), "both orphans are swept");
        assert!(!swept.breaker_tripped, "2 of 20 is at the limit, not over it");
        assert_eq!(swept.blobs, 2);
    }

    /// SPEC-0034/DT-020: over the limit, no content blob goes, the export zip
    /// and the temp file still do.
    #[tokio::test]
    async fn dt_020_over_the_limit_trips_the_breaker_for_content_only() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 17, H1).await;
        let mut orphans = Vec::new();
        for i in 0..3 {
            orphans.push(f.put_content(p, format!("orphan {i}").as_bytes(), H25).await);
        }
        f.put(p, "export:t2", b"zip two", H25).await;
        f.put_tmp(p, ".tmp-x", b"partial", H25);

        let swept = f.sweep(p).await.unwrap();

        assert!(swept.breaker_tripped, "3 of 20 is over the limit of 2");
        for o in &orphans {
            assert!(f.present(p, o), "{o} is held back by the breaker");
        }
        assert!(!f.present(p, "export:t2"), "the export zip is outside the breaker");
        assert!(!f.present(p, ".tmp-x"), "the temp file is outside the breaker");
        assert_eq!(swept.blobs, 2);
    }

    /// SPEC-0034/DT-021: the limit never drops below one.
    #[tokio::test]
    async fn dt_021_the_limit_is_at_least_one() {
        let f = orphan_fixture().await;
        let lone = f.put_content("p1", b"the only blob", H25).await;
        let pair =
            [f.put_content("p2", b"first", H25).await, f.put_content("p2", b"second", H25).await];

        let one = f.sweep("p1").await.unwrap();
        let two = f.sweep("p2").await.unwrap();

        assert!(!f.present("p1", &lone), "a lone orphan is within the limit of 1");
        assert!(!one.breaker_tripped);
        assert!(two.breaker_tripped, "2 orphans of 2 blobs exceed the limit of 1");
        for o in &pair {
            assert!(f.present("p2", o), "{o} is held back");
        }
    }

    /// SPEC-0034/DT-024: a failing delete skips that object only.
    #[tokio::test]
    async fn dt_024_a_failed_delete_skips_only_that_object() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.put_referenced(p, 27, H1).await;
        let mut orphans = Vec::new();
        for i in 0..3 {
            orphans.push(f.put_content(p, format!("orphan {i}").as_bytes(), H25).await);
        }
        let real = f.client(p).await;
        let double = Arc::new(SweepBlob::new(real.blob.clone(), false, Some(2)));
        let client = client_with_blob(&f, p, double).await;

        let swept = sweep_project_orphans_at(f.now, &f.stores, &client, p).await.unwrap();

        assert_eq!(swept.blobs, 2);
        assert!(!swept.breaker_tripped, "3 of 30 is at the limit");
        assert_eq!(orphans.iter().filter(|o| f.present(p, o)).count(), 1, "one orphan remains");
    }

    /// SPEC-0034/DT-027: the diff scales; exactly the orphans go.
    #[tokio::test]
    async fn dt_027_ten_thousand_referenced_blobs_and_five_orphans() {
        let f = orphan_fixture().await;
        let p = "proj";
        let referenced = f.put_referenced(p, 10_000, H25).await;
        let mut orphans = Vec::new();
        for i in 0..5 {
            orphans.push(f.put_content(p, format!("orphan {i}").as_bytes(), H25).await);
        }

        let swept = f.sweep(p).await.unwrap();

        assert_eq!(swept.blobs, 5);
        assert!(orphans.iter().all(|o| !f.present(p, o)), "every orphan is swept");
        assert!(referenced.iter().all(|r| f.present(p, r)), "every referenced blob is kept");
    }

    // ── orphan sweep wired into the purge cycle (US-0039) ──────────────────────

    /// A blob double whose `list` always fails; everything else forwards to
    /// `inner` (SPEC-0034/DT-023).
    struct FailListBlob {
        inner: Arc<dyn crate::storage::traits::BlobBackend>,
    }

    impl FailListBlob {
        fn new(inner: Arc<dyn crate::storage::traits::BlobBackend>) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl crate::storage::traits::BlobBackend for FailListBlob {
        async fn put(&self, sha256: &str, data: &[u8]) -> Result<()> {
            self.inner.put(sha256, data).await
        }
        async fn get(&self, sha256: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>> {
            self.inner.get(sha256, offset, length).await
        }
        async fn exists(&self, sha256: &str) -> Result<bool> {
            self.inner.exists(sha256).await
        }
        async fn delete(&self, sha256: &str) -> Result<()> {
            self.inner.delete(sha256).await
        }
        async fn ensure_bucket(&self) -> Result<()> {
            self.inner.ensure_bucket().await
        }
        async fn remove_bucket(&self) -> Result<()> {
            self.inner.remove_bucket().await
        }
        async fn list(&self) -> Result<Vec<crate::storage::traits::BlobEntry>> {
            Err(ToolError::internal("injected list failure"))
        }
        async fn modified(&self, entry: &crate::storage::traits::BlobEntry) -> Result<f64> {
            self.inner.modified(entry).await
        }
        async fn delete_entry(&self, entry: &crate::storage::traits::BlobEntry) -> Result<()> {
            self.inner.delete_entry(entry).await
        }
    }

    /// SPEC-0034/DT-022: the sweep runs on the first cycle, is gated for 24
    /// hours, runs again once past that window, and a fresh `StoreManager`
    /// over the same store has no memory of the schedule.
    #[tokio::test]
    async fn dt_022_schedule_gates_the_sweep_once_a_day_per_project() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.admin.create_project(p, "owner@t.c").await.unwrap();
        f.put_referenced(p, 20, H1).await;

        let o1 = f.put_content(p, b"orphan one", H25).await;
        let s1 = f.run_cycle_at(f.now).await.unwrap();
        assert_eq!(s1.orphan_blobs_swept, 1, "nothing scheduled yet, swept on the first cycle");
        assert!(!f.present(p, &o1));

        let o2 = f.put_content(p, b"orphan two", H25).await;
        let s2 = f.run_cycle_at(f.now + 23.0 * 3600.0).await.unwrap();
        assert_eq!(s2.orphan_blobs_swept, 0, "inside 24h of the last sweep, skipped");
        assert!(f.present(p, &o2), "kept: the sweep was skipped, not run and empty-handed");

        let s3 = f.run_cycle_at(f.now + 24.0 * 3600.0 + 60.0).await.unwrap();
        assert_eq!(s3.orphan_blobs_swept, 1, "past 24h since the last sweep, runs again");
        assert!(!f.present(p, &o2));

        let o3 = f.put_content(p, b"orphan three", H25).await;
        let fresh = StoreManager::new(f.stores.config().clone(), f.stores.relational().clone());
        let s4 = run_cycle_at(f.now + 24.0 * 3600.0 + 61.0, &fresh, &f.admin, &f.safety, None)
            .await
            .unwrap();
        assert_eq!(s4.orphan_blobs_swept, 1, "a fresh process sweeps on its first cycle");
        assert!(!f.present(p, &o3));
    }

    /// SPEC-0034/DT-023: a project whose backend fails to list is skipped and
    /// logged, but the cycle still succeeds and still sweeps every other
    /// project (its orphan sweep and its existing file sweep both).
    #[tokio::test]
    async fn dt_023_a_failing_project_is_isolated_from_the_rest_of_the_cycle() {
        let f = orphan_fixture().await;
        f.admin.create_project("p1", "owner@t.c").await.unwrap();
        f.admin.create_project("p2", "owner@t.c").await.unwrap();
        f.admin.seed_purge_config_for_test("p1", true, true, None, None).await.unwrap();
        f.admin.seed_purge_config_for_test("p2", true, true, Some(7), None).await.unwrap();

        let real1 = f.client("p1").await;
        let failing: Arc<dyn crate::storage::traits::BlobBackend> =
            Arc::new(FailListBlob::new(real1.blob.clone()));
        let client1 = VolumeClient::new("p1", real1.meta.clone(), failing)
            .with_trash_store(real1.trash.clone().unwrap());
        f.stores.insert_client_for_test("p1", Arc::new(client1)).await;

        let o = f.put_content("p2", b"orphan", H25).await;
        let c2 = f.client("p2").await;
        c2.write_text_atomic("/old.txt", "data").await.unwrap();
        c2.trash
            .as_ref()
            .unwrap()
            .set_atime_for_test("/old.txt", now_unix() - 8.0 * DAY)
            .await
            .unwrap();

        let result = f.run_cycle_at(f.now).await;
        assert!(result.is_ok(), "{result:?}");
        let summary = result.unwrap();
        assert_eq!(summary.files_purged, 1, "p2's stale file purged despite p1's failure");
        assert_eq!(summary.orphan_blobs_swept, 1, "p2's orphan swept despite p1's listing failure");
        assert!(!f.present("p2", &o));
    }

    /// SPEC-0034/DT-025: `CycleSummary` sums the orphan sweep's objects and
    /// bytes across the cycle (the DT-014 setup, trimmed to the three
    /// deleted keys with sizes that add to a distinctive total).
    #[tokio::test]
    async fn dt_025_cycle_summary_counts_blobs_and_bytes_swept() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.admin.create_project(p, "owner@t.c").await.unwrap();
        f.put_referenced(p, 20, H25).await;
        f.put_content(p, b"orphan1", H25).await; // O: 7 bytes
        f.put(p, "export:t2", b"elevenbytes", H25).await; // 11 bytes, no row
        f.put_tmp(p, ".tmp-x", b"fiveb", H25); // 5 bytes

        let summary = f.run_cycle_at(f.now).await.unwrap();

        assert_eq!(summary.orphan_blobs_swept, 3);
        assert_eq!(summary.orphan_bytes_swept, 23);
    }

    /// SPEC-0034/DT-026: an empty bucket is a zero-count no-op, cycle `Ok`.
    #[tokio::test]
    async fn dt_026_an_empty_bucket_is_a_zero_count_no_op() {
        let f = orphan_fixture().await;
        let p = "proj";
        f.admin.create_project(p, "owner@t.c").await.unwrap();
        f.client(p).await;

        let result = f.run_cycle_at(f.now).await;
        assert!(result.is_ok(), "{result:?}");
        let summary = result.unwrap();
        assert_eq!(summary.orphan_blobs_swept, 0);
        assert_eq!(summary.orphan_bytes_swept, 0);
        assert_eq!(summary.orphan_breakers_tripped, 0);
    }
}
