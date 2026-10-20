//! `fs.diff_files` (SPEC-0045): compare two text files, possibly in two
//! projects. Lives here rather than in `core::fs_ops` because every `fs_ops`
//! function works on one volume client, and this one needs two (DEC-001). The
//! MCP adapter and the REST handler both call `diff_files`, never a copy.

use crate::core::diff::{DEFAULT_CONTEXT, unified_labeled};
use crate::errors::{Result, ToolError};
use crate::state::AppState;
use serde_json::{Value, json};

/// One side of a comparison: the project and the raw (not yet normalized) path.
pub(crate) struct Side<'a> {
    pub mount_id: &'a str,
    pub path: &'a str,
}

/// Compare `a` (old side) with `b` (new side).
///
/// The order of checks is fixed by DEC-004 (normalize both, authorize and
/// open both, then read both) so a refusal never depends on file contents:
/// a non member learns nothing about the other project's files.
///
/// Cost: O(n + m) memory for the two texts and their line slices, and the
/// Myers diff is O((n + m) d) time in lines, d the edit distance; it runs on
/// the blocking pool so a large comparison never stalls a request worker.
pub(crate) async fn diff_files(
    state: &AppState,
    person: &str,
    a: Side<'_>,
    b: Side<'_>,
) -> Result<Value> {
    let path_a = state.safety.normalize_path(a.path)?;
    let path_b = state.safety.normalize_path(b.path)?;
    state.authorize(a.mount_id, person).await?;
    let client_a = state.stores.client(a.mount_id).await?;
    state.authorize(b.mount_id, person).await?;
    let client_b = state.stores.client(b.mount_id).await?;

    let label_a = format!("{}:{path_a}", a.mount_id);
    let label_b = format!("{}:{path_b}", b.mount_id);
    // Boot validation guarantees `Some`; a `None` here is a bug, not a default.
    let limit = state
        .safety
        .config()
        .max_diff_file_bytes
        .bytes()
        .ok_or_else(|| ToolError::internal("max_diff_file_bytes is not set"))?;
    // A is read completely before B is touched (FR-NEW-015).
    let old_text = client_a.read_text_checked(&label_a, &path_a, limit).await?;
    let new_text = client_b.read_text_checked(&label_b, &path_b, limit).await?;

    // Both are valid UTF-8 here, so string equality is byte equality (DEC-003).
    if old_text == new_text {
        return Ok(json!({"identical": true, "diff": ""}));
    }
    let diff = tokio::task::spawn_blocking(move || {
        unified_labeled(&old_text, &new_text, &label_a, &label_b, DEFAULT_CONTEXT)
    })
    .await
    .map_err(|e| ToolError::internal(format!("diff task failed: {e}")))?;
    Ok(json!({"identical": false, "diff": diff}))
}

/// Test registry glue, the shape `tools/contract_golden.rs` freezes.
#[cfg(test)]
pub(crate) fn register(reg: &mut crate::tools::registry_support::ToolRegistry) {
    use crate::tools::registry_support::{ToolSchema, handler};
    reg.add(
        ToolSchema::new(
            "fs.diff_files",
            "Compare two text files (same or different project) and return a unified diff with 3 lines of context.",
        )
        .req_str("mount_id_a", "Project/volume id of the old side.")
        .req_str("path_a", "Absolute POSIX path of the old side within its volume.")
        .req_str("mount_id_b", "Project/volume id of the new side.")
        .req_str("path_b", "Absolute POSIX path of the new side within its volume.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let (mount_a, path_a) = (a.str("mount_id_a")?, a.str("path_a")?);
            let (mount_b, path_b) = (a.str("mount_id_b")?, a.str("path_b")?);
            diff_files(
                &ctx.state,
                &ctx.person,
                Side { mount_id: &mount_a, path: &path_a },
                Side { mount_id: &mount_b, path: &path_b },
            )
            .await
        }),
    );
}
