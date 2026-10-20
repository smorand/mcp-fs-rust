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

/// Largest accepted `context_lines`. Far beyond any file the diff size cap
/// admits, so it never limits a real comparison, yet it keeps the value small
/// enough that the line window arithmetic in the engine cannot overflow.
pub(crate) const MAX_CONTEXT_LINES: usize = 10_000_000;

/// Read the optional `context_lines`: absent or null means the default, a
/// whole number or a string of ASCII digits within range is taken as is, and
/// everything else gets the one message. The value is never echoed, so the
/// refusal reveals nothing.
fn parse_context_lines(raw: Option<&Value>) -> Result<usize> {
    let refused = || {
        ToolError::invalid_argument(format!(
            "'context_lines' must be a whole number from 0 to {MAX_CONTEXT_LINES}"
        ))
    };
    let n = match raw {
        None | Some(Value::Null) => return Ok(DEFAULT_CONTEXT),
        // `is_u64` rejects `-1`, `2.5`, `3.0`, `1e3` and `-0` (read as f64).
        Some(Value::Number(n)) if n.is_u64() => n.as_u64(),
        // Digits only: no sign, no space, no hex, as `str::parse` alone would allow `+3`.
        Some(Value::String(s)) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse::<u64>().ok()
        }
        Some(_) => None,
    };
    n.and_then(|n| usize::try_from(n).ok()).filter(|n| *n <= MAX_CONTEXT_LINES).ok_or_else(refused)
}

/// Compare `a` (old side) with `b` (new side).
///
/// `context_lines` is checked before anything else (SPEC-0046 DEC-001), so a
/// bad value is refused identically for every caller whatever the projects or
/// files. After it, the order of checks is fixed by DEC-004 (normalize both,
/// authorize and open both, then read both) so a refusal never depends on file
/// contents: a non member learns nothing about the other project's files.
///
/// Cost: O(n + m) memory for the two texts and their line slices, and the
/// Myers diff is O((n + m) d) time in lines, d the edit distance; it runs on
/// the blocking pool so a large comparison never stalls a request worker.
pub(crate) async fn diff_files(
    state: &AppState,
    person: &str,
    a: Side<'_>,
    b: Side<'_>,
    context_lines: Option<&Value>,
) -> Result<Value> {
    let context = parse_context_lines(context_lines)?;
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

    // Success only (DEC-006): both reads passed, so a refusal above never
    // touches. The bump is the sole record of the access; `record_read` is
    // deliberately absent so a comparison never satisfies read before edit.
    client_a.touch_atime(&path_a);
    client_b.touch_atime(&path_b);

    // Both are valid UTF-8 here, so string equality is byte equality (DEC-003).
    if old_text == new_text {
        return Ok(json!({"identical": true, "diff": ""}));
    }
    let diff = tokio::task::spawn_blocking(move || {
        unified_labeled(&old_text, &new_text, &label_a, &label_b, context)
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
            "Compare two text files (same or different project) and return a unified diff. Shows 3 lines of context by default; context_lines (0 to 10000000) chooses another number of unchanged lines around each change.",
        )
        .req_str("mount_id_a", "Project/volume id of the old side.")
        .req_str("path_a", "Absolute POSIX path of the old side within its volume.")
        .req_str("mount_id_b", "Project/volume id of the new side.")
        .req_str("path_b", "Absolute POSIX path of the new side within its volume.")
        .opt_any(
            "context_lines",
            "Optional. Number of unchanged lines shown around each change, a whole number from 0 to 10000000. Default 3.",
        )
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
                a.raw("context_lines"),
            )
            .await
        }),
    );
}
