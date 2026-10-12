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

/// The bytes `client`'s project uses right now: every file's declared size,
/// trashed files included (FR-NEW-002). A volume whose counter row is missing
/// is backfilled from its nodes, under the lock.
pub(crate) async fn used_bytes(client: &VolumeClient) -> Result<i64> {
    let _admission = client.admission.lock().await;
    client.meta.used_bytes().await
}
