//! `fs.trash_list` (SPEC-0011 US-0004) and its shared backfill helper.
//!
//! Typed functions only: the `#[tool]` method in `mcp::server` authorizes and
//! normalizes, then calls straight through to [`trash_list`] below, exactly like
//! every other `fs.*` tool.

use crate::errors::{Result, ToolError};
use crate::state::AppState;
use crate::storage::VolumeClient;
use serde_json::{Value, json};

const SECONDS_PER_DAY: f64 = 86_400.0;

/// `fs.trash_list(mount_id, path_prefix, limit, offset)` (FR-NEW-007/008/011).
///
/// Time complexity: O(u) for the backfill, where `u` is the number of nodes
/// directly under the project's trash directory (one listing call, one
/// idempotent upsert per untracked node), plus O(limit) for the page actually
/// returned and one unpaged `COUNT(*)` for `total`. Space: O(u) for the trash
/// directory listing plus O(limit) for the returned page: the whole trash tree
/// is never loaded into memory beyond its immediate children.
pub(crate) async fn trash_list(
    state: &AppState,
    mount_id: &str,
    path_prefix: &str,
    limit: i64,
    offset: i64,
) -> Result<Value> {
    if limit < 0 || offset < 0 {
        return Err(ToolError::invalid_argument("limit and offset must both be >= 0"));
    }
    let client = state.stores.client(mount_id).await?;
    backfill_trash_entries(&client, &state.safety.config().trash_dir).await?;

    let store = client
        .trash
        .as_ref()
        .ok_or_else(|| ToolError::internal("volume has no trash store attached"))?;
    let (rows, total) = store.list_trash_entries_page(path_prefix, limit, offset).await?;
    let purge_config = state.admin.get_purge_config(mount_id).await?;
    let now = crate::util::now_unix();

    let entries: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let purge_in_days = purge_config.file_retention_days.map(|retention_days| {
                let deleted_unix = parse_iso(&r.deleted_at).unwrap_or(now);
                let days_elapsed = ((now - deleted_unix) / SECONDS_PER_DAY).floor() as i64;
                retention_days - days_elapsed
            });
            json!({
                "trash_path": r.trash_path,
                "original_path": r.original_path,
                "size": r.size,
                "kind": r.kind,
                "deleted_at": r.deleted_at,
                "deleted_by": r.deleted_by,
                "purge_in_days": purge_in_days,
            })
        })
        .collect();
    Ok(json!({"entries": entries, "total": total}))
}

/// Parses a `trash_entries.deleted_at` RFC 3339 timestamp (the format
/// [`crate::util::now_iso`] writes) into fractional Unix seconds.
fn parse_iso(iso: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(iso).ok().map(|dt| dt.timestamp_micros() as f64 / 1e6)
}

/// Backfill every untracked node directly under `trash_dir` into `trash_entries`
/// (FR-NEW-011, DEC-003). A standalone helper so US-0005's `fs.trash_restore`
/// can call the exact same reconstruction rather than re-implementing it.
///
/// Idempotent: each node is upserted through
/// [`crate::storage::meta::RelationalMetaStore::backfill_trash_entry`], which
/// only inserts when no row exists yet for that `trash_path`.
pub(crate) async fn backfill_trash_entries(client: &VolumeClient, trash_dir: &str) -> Result<()> {
    let Some(store) = client.trash.as_ref() else { return Ok(()) };
    let trash_root = format!("/{}", trash_dir.trim_matches('/'));
    if !client.is_dir(&trash_root).await? {
        return Ok(());
    }
    for node in client.list_dir(&trash_root).await? {
        let (original_path, deleted_at) = reconstruct_legacy_entry(&node.name);
        store
            .backfill_trash_entry(&node.path, &original_path, node.size, &node.kind, &deleted_at)
            .await?;
    }
    Ok(())
}

/// Reconstructs `(original_path, deleted_at)` from a trash node's file name of
/// the form `{epoch_ms}__{flattened_path}` (FR-NEW-011): strips the leading
/// `{epoch_ms}__` prefix and replaces remaining `__` with `/` for the path,
/// parses `epoch_ms` for the timestamp, falling back to now when unparseable.
///
/// Best-effort and ambiguous by design for pre-existing data only: a legacy
/// `original_path` that itself contained a literal `__` is indistinguishable
/// from one whose separator was `/` (both flatten identically).
fn reconstruct_legacy_entry(name: &str) -> (String, String) {
    let (epoch_part, rest) = name.split_once("__").unwrap_or(("", name));
    let original_path = rest.replace("__", "/");
    let deleted_at = epoch_part
        .parse::<i64>()
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Micros, false))
        .unwrap_or_else(crate::util::now_iso);
    (original_path, deleted_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::admin::test_support::Fixture;

    const OWNER: &str = "owner@test.com";

    async fn trash_delete(f: &Fixture, mount_id: &str, path: &str) {
        let client = f.state.stores.client(mount_id).await.unwrap();
        client.write_text_atomic(path, "x").await.unwrap();
        crate::core::fs_ops::delete_path(
            &client,
            &f.state.safety,
            OWNER,
            mount_id,
            path,
            false,
            true,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn orders_newest_first_and_reports_every_key() {
        let f = Fixture::new().await;
        f.seed_project("proj-list-1", OWNER).await;
        trash_delete(&f, "proj-list-1", "/a.txt").await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        trash_delete(&f, "proj-list-1", "/b.txt").await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        trash_delete(&f, "proj-list-1", "/c.txt").await;

        let r = trash_list(&f.state, "proj-list-1", "", 200, 0).await.unwrap();
        assert_eq!(r["total"], 3);
        let entries = r["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0]["original_path"].as_str().unwrap().ends_with("c.txt"));
        assert!(entries[1]["original_path"].as_str().unwrap().ends_with("b.txt"));
        assert!(entries[2]["original_path"].as_str().unwrap().ends_with("a.txt"));
        for key in ["trash_path", "original_path", "size", "kind", "deleted_at", "deleted_by"] {
            assert!(entries[0].get(key).is_some(), "missing key {key}");
        }
        assert!(entries[0].as_object().unwrap().contains_key("purge_in_days"));
    }

    #[tokio::test]
    async fn empty_trash_is_empty_not_an_error() {
        let f = Fixture::new().await;
        f.seed_project("proj-empty", OWNER).await;
        let r = trash_list(&f.state, "proj-empty", "", 200, 0).await.unwrap();
        assert_eq!(r, json!({"entries": [], "total": 0}));
    }

    #[tokio::test]
    async fn pagination_boundary() {
        let f = Fixture::new().await;
        f.seed_project("proj-page", OWNER).await;
        for n in 1..=5 {
            trash_delete(&f, "proj-page", &format!("/f{n}.txt")).await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let r = trash_list(&f.state, "proj-page", "", 2, 4).await.unwrap();
        assert_eq!(r["total"], 5);
        let entries = r["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0]["original_path"].as_str().unwrap().ends_with("f1.txt"));

        let r2 = trash_list(&f.state, "proj-page", "", 2, 5).await.unwrap();
        assert_eq!(r2["total"], 5);
        assert_eq!(r2["entries"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn path_prefix_matches_nothing() {
        let f = Fixture::new().await;
        f.seed_project("proj-list-1", OWNER).await;
        trash_delete(&f, "proj-list-1", "/data/a.txt").await;
        let r = trash_list(&f.state, "proj-list-1", "/nomatch/", 200, 0).await.unwrap();
        assert_eq!(r, json!({"entries": [], "total": 0}));
    }

    #[tokio::test]
    async fn path_prefix_filters_correctly() {
        let f = Fixture::new().await;
        f.seed_project("proj-list-1", OWNER).await;
        trash_delete(&f, "proj-list-1", "/data/a.txt").await;
        trash_delete(&f, "proj-list-1", "/other/b.txt").await;
        let r = trash_list(&f.state, "proj-list-1", "/data/", 200, 0).await.unwrap();
        let entries = r["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["original_path"], "/data/a.txt");
    }

    #[tokio::test]
    async fn backfills_a_legacy_untracked_trash_node() {
        let f = Fixture::new().await;
        f.seed_project("proj-backfill", OWNER).await;
        let client = f.state.stores.client("proj-backfill").await.unwrap();
        client
            .write_text_atomic("/.mcp_trash/1700000000000__legacy__report.txt", "x")
            .await
            .unwrap();

        let r = trash_list(&f.state, "proj-backfill", "", 200, 0).await.unwrap();
        let entries = r["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["original_path"], "legacy/report.txt");
        assert_eq!(entries[0]["deleted_by"], Value::Null);

        let store = client.trash.as_ref().unwrap();
        let row = store
            .get_trash_entry_for_test("/.mcp_trash/1700000000000__legacy__report.txt")
            .await
            .unwrap();
        assert!(row.is_some());
    }

    #[tokio::test]
    async fn backfill_is_idempotent() {
        let f = Fixture::new().await;
        f.seed_project("proj-backfill", OWNER).await;
        let client = f.state.stores.client("proj-backfill").await.unwrap();
        client
            .write_text_atomic("/.mcp_trash/1700000000000__legacy__report.txt", "x")
            .await
            .unwrap();

        trash_list(&f.state, "proj-backfill", "", 200, 0).await.unwrap();
        let r = trash_list(&f.state, "proj-backfill", "", 200, 0).await.unwrap();
        let entries = r["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        let store = client.trash.as_ref().unwrap();
        assert_eq!(store.count_trash_entries_for_test().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn purge_in_days_can_go_negative() {
        let f = Fixture::new().await;
        f.seed_project("proj-retention", OWNER).await;
        f.state
            .admin
            .set_purge_config(
                "proj-retention",
                crate::storage::traits::PurgeConfig {
                    file_retention_days: Some(3),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let client = f.state.stores.client("proj-retention").await.unwrap();
        client.write_text_atomic("/old.txt", "x").await.unwrap();
        let ten_days_ago = crate::util::now_unix() - 10.0 * SECONDS_PER_DAY;
        let deleted_at = chrono::DateTime::from_timestamp(ten_days_ago as i64, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
        client
            .trash
            .as_ref()
            .unwrap()
            .record_trash_entry("/.mcp_trash/old_entry", "/old.txt", 1, "file", &deleted_at, None)
            .await
            .unwrap();

        let r = trash_list(&f.state, "proj-retention", "", 200, 0).await.unwrap();
        assert_eq!(r["entries"][0]["purge_in_days"], -7);
    }

    #[tokio::test]
    async fn purge_in_days_is_null_without_retention_configured() {
        let f = Fixture::new().await;
        f.seed_project("proj-no-retention", OWNER).await;
        trash_delete(&f, "proj-no-retention", "/x.txt").await;
        let r = trash_list(&f.state, "proj-no-retention", "", 200, 0).await.unwrap();
        assert_eq!(r["entries"][0]["purge_in_days"], Value::Null);
    }

    #[tokio::test]
    async fn negative_limit_is_rejected() {
        let f = Fixture::new().await;
        f.seed_project("proj-list-1", OWNER).await;
        let e = trash_list(&f.state, "proj-list-1", "", -1, 0).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn negative_offset_is_rejected() {
        let f = Fixture::new().await;
        f.seed_project("proj-list-1", OWNER).await;
        let e = trash_list(&f.state, "proj-list-1", "", 10, -1).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn limit_zero_is_accepted_not_an_error() {
        let f = Fixture::new().await;
        f.seed_project("proj-list-1", OWNER).await;
        trash_delete(&f, "proj-list-1", "/a.txt").await;
        let r = trash_list(&f.state, "proj-list-1", "", 0, 0).await.unwrap();
        assert_eq!(r["entries"].as_array().unwrap().len(), 0);
        assert_eq!(r["total"], 1);
    }
}
