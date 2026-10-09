//! `fs.extract_archive` (SPEC-0015): extract an archive file in place inside
//! the volume.
//!
//! Typed function only: the `#[tool]` method in `mcp::server`
//! (`McpServer::fs_extract_archive`) and, once US-0011 lands, the REST route
//! both authorize, then call [`extract_archive`], so the two doors share one
//! implementation (`tools::export::export_zip` is the structural template).
//!
//! This story (US-0004) is the skeleton: the function normalizes `path`,
//! rejects a missing node and a directory node, and otherwise returns a
//! placeholder. Format detection, decode and the real result shape land in
//! later stories (US-0005 onward).

use crate::errors::{Result, ToolError};
use crate::state::AppState;
use serde_json::{Value, json};

/// `fs.extract_archive(mount_id, path, destination?, overwrite?, password?)`
/// (FR-NEW-001..004). The caller has already authorized `mount_id`
/// (FR-NEW-002 runs strictly before this, exactly as `export_zip` assumes).
///
/// Time: O(1) node lookups in this story (format detection and the actual
/// extraction walk are deferred to later stories). Space: O(1).
pub(crate) async fn extract_archive(
    state: &AppState,
    mount_id: &str,
    path: &str,
    _destination: Option<&str>,
    _overwrite: bool,
    _password: Option<&str>,
) -> Result<Value> {
    let normalized = state.safety.normalize_path(path)?;

    let client = state.stores.client(mount_id).await?;
    let node = client
        .meta
        .get(&normalized)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("'{path}' not found")))?;
    if node.is_dir() {
        return Err(ToolError::invalid_argument(format!(
            "'{path}' is a directory, not an archive file"
        )));
    }

    // Format detection and decode land in US-0005/US-0006; this skeleton has
    // nothing left to do once the preconditions above hold.
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::admin::test_support::Fixture;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "proj";

    async fn fixture() -> Fixture {
        let f = Fixture::with_config(|_| {}).await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/adir", true).await.unwrap();
        c.makedirs("/uploads/a/b", true).await.unwrap();
        f
    }

    async fn extract(f: &Fixture, path: &str) -> Result<Value> {
        extract_archive(&f.state, MOUNT, path, None, false, None).await
    }

    /// `path` points at a file that does not exist: `ERR_NOT_FOUND`.
    #[tokio::test]
    async fn missing_archive_is_not_found() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/missing.zip").await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(e.message.contains("/uploads/missing.zip"), "{}", e.message);
    }

    /// E2E-NEW-025: `path` points at a directory, not a file.
    #[tokio::test]
    async fn e2e_new_025_directory_path_is_invalid_argument() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/adir").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("'/uploads/adir' is a directory"), "{}", e.message);
    }

    /// E2E-NEW-059: `path` points at a directory nested two levels deep.
    #[tokio::test]
    async fn e2e_new_059_nested_directory_path_is_invalid_argument() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/a/b").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("'/uploads/a/b' is a directory"), "{}", e.message);
    }

    /// E2E-NEW-060: `path` is `/` itself, rejected the same way, not special
    /// cased.
    #[tokio::test]
    async fn e2e_new_060_volume_root_is_invalid_argument_like_any_directory() {
        let f = fixture().await;
        let e = extract(&f, "/").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("'/' is a directory"), "{}", e.message);
    }
}
