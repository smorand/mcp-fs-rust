//! Project storage quota (SPEC-0020): the usage read behind
//! `admin.get_project_quota` and its REST route.
//!
//! Usage is the per volume `volume_usage` counter the node transactions keep
//! (DEC-002), so a read is one row, never a scan of the tree. The read takes the
//! volume's admission lock, the same lock enforcement holds across an
//! operation's writes, so a figure is never taken halfway through one.

use crate::errors::Result;
use crate::storage::VolumeClient;

/// The bytes `client`'s project uses right now: every file's declared size,
/// trashed files included (FR-NEW-002). A volume whose counter row is missing
/// is backfilled from its nodes, under the lock.
pub(crate) async fn used_bytes(client: &VolumeClient) -> Result<i64> {
    let _admission = client.admission.lock().await;
    client.meta.used_bytes().await
}
