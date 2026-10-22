//! Project storage quota (SPEC-0020): the usage read behind
//! `admin.get_project_quota` and its REST route, and [`admit`], the one
//! admission every write that adds bytes to a project goes through.
//!
//! Usage is the per volume `volume_usage` counter the node transactions keep
//! (DEC-002), so a read is one row, never a scan of the tree. The read takes the
//! volume's admission lock, the same lock enforcement holds across an
//! operation's writes, so a figure is never taken halfway through one.

use crate::errors::{Result, ToolError};
use crate::safety::SafetyManager;
use crate::storage::VolumeClient;
use crate::storage::meta::{FileUsageRow, FolderItems};

/// Proof that an operation was admitted. It holds the volume's admission lock,
/// so it must live until the operation's last write landed (DEC-002): another
/// writer admitted in between would check against a usage that is about to
/// grow and overshoot the limit.
#[must_use = "dropping the admission releases the lock before the write"]
pub(crate) struct Admission {
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

/// Admit an operation that changes `client`'s usage by `delta` bytes and
/// charges `session_bytes` to `person`'s session budget.
///
/// Order is DEC-004: the project limit first, the session charge only after it
/// passed, so a refusal charges nothing (FR-NEW-006). A `delta` of zero or less
/// always passes the project check (FR-NEW-004), and reaching the limit exactly
/// is allowed (FR-NEW-005). The check is one counter row read (O(1)).
pub(crate) async fn admit(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    session_bytes: i64,
    delta: i64,
) -> Result<Admission> {
    let guard = client.admission.clone().lock_owned().await;
    check_and_charge(client, safety, person, mount_id, session_bytes, delta).await?;
    Ok(Admission { _guard: guard })
}

/// [`admit`] for a write of `new_size` bytes to the single file `path`: the
/// delta is computed under the lock from the size `path` has right now, so a
/// concurrent writer cannot change it between the read and the check.
pub(crate) async fn admit_file(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    new_size: i64,
) -> Result<Admission> {
    let guard = client.admission.clone().lock_owned().await;
    let previous = match client.meta.get(path).await? {
        Some(node) if node.is_file() => node.size,
        _ => 0,
    };
    check_and_charge(client, safety, person, mount_id, new_size, new_size - previous).await?;
    Ok(Admission { _guard: guard })
}

/// [`admit`] for a Markdown companion of `md_len` bytes written to the
/// optional sibling `md`: the delta is the sibling's current size read
/// under the lock (SPEC-0020 FR-NEW-002, FR-NEW-014), so a concurrent write
/// to the sibling cannot land between that read and the check. `md` of
/// `None` means no sibling is written, so the delta is always 0.
/// `session_bytes` is the separate conversion-text weight charged to
/// `person`'s session budget (DEC-008).
pub(crate) async fn admit_sibling(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    md: Option<&str>,
    md_len: i64,
    session_bytes: i64,
) -> Result<Admission> {
    let guard = client.admission.clone().lock_owned().await;
    let delta = match md {
        Some(path) => {
            let previous = match client.meta.get(path).await? {
                Some(node) if node.is_file() => node.size,
                _ => 0,
            };
            md_len - previous
        }
        None => 0,
    };
    check_and_charge(client, safety, person, mount_id, session_bytes, delta).await?;
    Ok(Admission { _guard: guard })
}

/// The body of an admission; the caller holds the lock.
async fn check_and_charge(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    session_bytes: i64,
    delta: i64,
) -> Result<()> {
    if delta > 0
        && let Some(limit) = safety.project_limit(&client.project_id).await?
    {
        let used = client.meta.used_bytes().await?;
        if used + delta > limit {
            return Err(ToolError::project_quota_exceeded(limit, used, delta));
        }
    }
    safety.charge_write(person, mount_id, session_bytes)
}

/// [`admit`] for a multi file operation (SPEC-0020 FR-NEW-007): the net
/// change is the sum, over every `(path, new_size)` pair, of `new_size` minus
/// whatever file currently sits at `path` (0 if none), each previous size
/// read under the lock so a concurrent writer cannot change it between the
/// read and the check (DEC-003). The project limit is checked once for the
/// total, before any of the operation's writes (DEC-002). Used by a folder
/// copy (`session_bytes` 0: a folder copy charges nothing to the session
/// today, FR-NEW-006), by archive extraction (`session_bytes` the
/// already unchanged total declared size), and by every version control
/// operation that changes project files: a clone and
/// `git::merge::charge_and_apply` (`session_bytes` the written blobs, a
/// removed path listed with a new size of 0 so its bytes leave the usage).
///
/// `paths` may list the same destination more than once (a tar archive
/// allows duplicate entries; the extraction loop writes them in order, so
/// only the LAST occurrence's size is what lands on disk). Only that last
/// occurrence is counted: walking in reverse and skipping a path already
/// seen keeps the last one written per path before summing, so a duplicate
/// never double-charges the project limit.
///
/// Time: O(n) over `paths`, one `meta.get` per distinct path.
pub(crate) async fn admit_paths(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    session_bytes: i64,
    paths: &[(String, i64)],
) -> Result<Admission> {
    let guard = client.admission.clone().lock_owned().await;

    let mut seen = std::collections::HashSet::with_capacity(paths.len());
    let mut delta = 0i64;
    for (path, new_size) in paths.iter().rev() {
        if !seen.insert(path.as_str()) {
            continue;
        }
        let previous = match client.meta.get(path).await? {
            Some(node) if node.is_file() => node.size,
            _ => 0,
        };
        delta += new_size - previous;
    }
    check_and_charge(client, safety, person, mount_id, session_bytes, delta).await?;
    Ok(Admission { _guard: guard })
}

/// The bytes `client`'s project uses right now: every file's declared size,
/// trashed files included (FR-NEW-002). A volume whose counter row is missing
/// is backfilled from its nodes, under the lock.
pub(crate) async fn used_bytes(client: &VolumeClient) -> Result<i64> {
    let _admission = client.admission.lock().await;
    client.meta.used_bytes().await
}

/// A project's usage with its largest files (SPEC-0047).
pub(crate) struct DiskUsage {
    pub used_bytes: i64,
    pub file_count: i64,
    pub entries: Vec<FileUsageRow>,
}

/// The bytes `client`'s project uses and its `cap` largest files, largest
/// first (FR-NEW-001, FR-NEW-002), all taken at one moment.
pub(crate) async fn disk_usage(
    client: &VolumeClient,
    cap: usize,
    trash_dir: &str,
) -> Result<DiskUsage> {
    let store = client
        .trash
        .as_ref()
        .ok_or_else(|| ToolError::internal("volume has no relational store attached"))?;
    // Every write holds this lock until its last file landed (DEC-002), so a read
    // under it never sees half of an operation (FR-NEW-010, DEC-005). The counter
    // is read through `meta`, not `used_bytes`, which would lock a second time.
    let _admission = client.admission.lock().await;
    let used_bytes = client.meta.used_bytes().await?;
    let (entries, file_count) = store.largest_files(cap, trash_dir).await?;
    Ok(DiskUsage { used_bytes, file_count, entries })
}

/// What a folder read found (SPEC-0049). The tool turns `Missing` and `IsFile`
/// into their refusals.
pub(crate) enum FolderRead {
    Missing,
    IsFile,
    Found { used_bytes: i64, items: FolderItems },
}

/// The bytes the project uses and the direct items of `folder`, largest first.
/// The root `/` is always a folder; any other path must be a stored directory.
pub(crate) async fn folder_usage(
    client: &VolumeClient,
    folder: &str,
    cap: usize,
) -> Result<FolderRead> {
    let store = client
        .trash
        .as_ref()
        .ok_or_else(|| ToolError::internal("volume has no relational store attached"))?;
    let used_bytes = client.meta.used_bytes().await?;
    if folder != "/" {
        match client.meta.get(folder).await? {
            None => return Ok(FolderRead::Missing),
            Some(node) if node.is_file() => return Ok(FolderRead::IsFile),
            Some(_) => {}
        }
    }
    let items = store.folder_items(folder, cap).await?;
    Ok(FolderRead::Found { used_bytes, items })
}
