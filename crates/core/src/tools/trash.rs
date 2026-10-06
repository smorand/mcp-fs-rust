//! `fs.trash_list` (SPEC-0011 US-0004) and its shared backfill helper.
//!
//! Typed functions only: the `#[tool]` method in `mcp::server` authorizes and
//! normalizes, then calls straight through to [`trash_list`] below, exactly like
//! every other `fs.*` tool.

use crate::errors::{Result, ToolError};
use crate::state::AppState;
use crate::storage::VolumeClient;
#[cfg(test)]
use crate::tools::authorize_only;
#[cfg(test)]
use crate::tools::registry_support::{ToolRegistry, ToolSchema, handler};
use serde_json::{Value, json};

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Test-dispatch glue for `fs.trash_list`/`fs.trash_restore`, mirroring the real
/// production handlers (`McpServer::fs_trash_list`/`fs_trash_restore`) so the
/// golden-contract regeneration sees the same schema the live MCP surface serves.
#[cfg(test)]
pub(crate) fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new(
            "fs.trash_list",
            "List trashed files, paginated and filterable by original path prefix.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .opt_str(
            "path_prefix",
            "",
            "Only return trashed entries whose original path starts with this prefix.",
        )
        .opt_int("limit", 200, "Maximum number of entries to return.")
        .opt_int("offset", 0, "Number of entries to skip before collecting `limit` results.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            trash_list(
                &ctx.state,
                &mount,
                &a.str_or("path_prefix", ""),
                a.int_or("limit", 200),
                a.int_or("offset", 0),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new(
            "fs.trash_restore",
            "Restore a trashed file or directory subtree back to its original path, renaming to `_restoredN` on collision.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("trash_path", "The trashed entry's current path (as returned by `fs.trash_list`).")
        .read_only(false)
        .destructive(false)
        .idempotent(false)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let trash_path = a.str("trash_path")?;
            trash_restore(&ctx.state, &mount, &trash_path).await
        }),
    );
}

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

/// `fs.trash_restore(mount_id, trash_path)` (FR-NEW-009/010).
///
/// Time complexity: O(1) lookups/writes for the row plus O(k) collision
/// retries (k <= 51, same bound as [`crate::core::fs_ops::rename_with_
/// collision_retry`]) and O(subtree size) for the actual rename when
/// `trash_path` names a directory. Space: O(1) beyond the formatted
/// candidate path.
pub(crate) async fn trash_restore(
    state: &AppState,
    mount_id: &str,
    trash_path: &str,
) -> Result<Value> {
    if trash_path.trim().is_empty() {
        return Err(ToolError::invalid_argument("trash_path must not be empty"));
    }
    let client = state.stores.client(mount_id).await?;
    let store = client
        .trash
        .as_ref()
        .ok_or_else(|| ToolError::internal("volume has no trash store attached"))?;

    let mut entry = store.get_trash_entry(trash_path).await?;
    if entry.is_none() && client.exists(trash_path).await? {
        backfill_trash_entries(&client, &state.safety.config().trash_dir).await?;
        entry = store.get_trash_entry(trash_path).await?;
    }
    let entry = entry
        .ok_or_else(|| ToolError::not_found(format!("'{trash_path}' is not a trashed entry")))?;

    // The legacy backfill (FR-NEW-011) reconstructs `original_path` without a
    // leading slash; every other `original_path` already has one.
    let destination = if entry.original_path.starts_with('/') {
        entry.original_path.clone()
    } else {
        format!("/{}", entry.original_path)
    };
    let parent = &destination[..destination.rfind('/').unwrap_or(0)];
    if !parent.is_empty() {
        client.makedirs(parent, true).await?;
    }
    let restored_path = restore_with_collision_retry(&client, trash_path, &destination).await?;
    store.delete_trash_entry(trash_path).await?;

    tracing::info!(trash_path, restored_path = %restored_path, "fs.trash_restore");
    Ok(json!({"restored_path": restored_path, "trash_path": trash_path}))
}

/// Rename `src` back to `dst`, retrying at `dst_restored`, `dst_restored2`,
/// ... until a free destination is found (SPEC-0011 US-0005, FR-NEW-010,
/// DEC-002). Unlike [`crate::core::fs_ops::rename_with_collision_retry`]'s
/// `~N` suffix, this one appends after the full file name including its
/// extension, and never gives up: restore must never fail on a collision.
async fn restore_with_collision_retry(
    client: &VolumeClient,
    src: &str,
    dst: &str,
) -> Result<String> {
    match client.rename(src, dst).await {
        Ok(()) => return Ok(dst.to_string()),
        Err(e) if e.code == crate::errors::code::NO_CLOBBER => {}
        Err(e) => return Err(e),
    }
    let mut n: u64 = 1;
    loop {
        let candidate =
            if n == 1 { format!("{dst}_restored") } else { format!("{dst}_restored{n}") };
        match client.rename(src, &candidate).await {
            Ok(()) => return Ok(candidate),
            Err(e) if e.code == crate::errors::code::NO_CLOBBER => n += 1,
            Err(e) => return Err(e),
        }
    }
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

    // ── SPEC-0011 US-0005: fs.trash_restore engine tests ───────────────────

    /// E2E-NEW-417: restore to the original path.
    #[tokio::test]
    async fn e2e_new_417_restore_to_the_original_path() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        trash_delete(&f, "proj-restore-1", "/doc.txt").await;
        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        let r = trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        assert_eq!(r["restored_path"], "/doc.txt");
        assert_eq!(r["trash_path"], trash_path.as_str());

        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        assert!(client.exists("/doc.txt").await.unwrap());
        let store = client.trash.as_ref().unwrap();
        assert!(store.get_trash_entry_for_test(&trash_path).await.unwrap().is_none());
    }

    /// E2E-NEW-418: restore of an unknown trash path fails `ERR_NOT_FOUND`.
    #[tokio::test]
    async fn e2e_new_418_restore_of_an_unknown_trash_path_is_not_found() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        let e = trash_restore(&f.state, "proj-restore-1", "/.mcp_trash/999__nope.txt")
            .await
            .unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    /// E2E-NEW-420: restore collision renames to `_restored`.
    #[tokio::test]
    async fn e2e_new_420_restore_collision_renames_to_restored() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        trash_delete(&f, "proj-restore-1", "/a.txt").await;
        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        client.write_text_atomic("/a.txt", "new").await.unwrap();

        let r = trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        assert_eq!(r["restored_path"], "/a.txt_restored");
        assert!(client.exists("/a.txt").await.unwrap());
        assert!(client.exists("/a.txt_restored").await.unwrap());
        let store = client.trash.as_ref().unwrap();
        assert!(store.get_trash_entry_for_test(&trash_path).await.unwrap().is_none());
    }

    /// E2E-NEW-421: double collision renames to `_restored2`.
    #[tokio::test]
    async fn e2e_new_421_double_collision_renames_to_restored2() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        trash_delete(&f, "proj-restore-1", "/a.txt").await;
        let listed1 = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path1 = listed1["entries"][0]["trash_path"].as_str().unwrap().to_string();

        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        client.write_text_atomic("/a.txt", "new").await.unwrap();
        trash_restore(&f.state, "proj-restore-1", &trash_path1).await.unwrap();
        // Now /a.txt and /a.txt_restored both occupied; trash a second copy.
        trash_delete(&f, "proj-restore-1", "/a.txt").await;
        let listed2 = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path2 = listed2["entries"][0]["trash_path"].as_str().unwrap().to_string();
        client.write_text_atomic("/a.txt", "newer").await.unwrap();

        let r = trash_restore(&f.state, "proj-restore-1", &trash_path2).await.unwrap();
        assert_eq!(r["restored_path"], "/a.txt_restored2");
    }

    /// E2E-NEW-422: collision suffix appended after the extension.
    #[tokio::test]
    async fn e2e_new_422_collision_suffix_appended_after_the_extension() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        trash_delete(&f, "proj-restore-1", "/report.pdf").await;
        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        client.write_text_atomic("/report.pdf", "new").await.unwrap();

        let r = trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        assert_eq!(r["restored_path"], "/report.pdf_restored");
    }

    /// E2E-NEW-423: restoring a directory restores the whole subtree.
    #[tokio::test]
    async fn e2e_new_423_restoring_a_directory_restores_the_whole_subtree() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        client.write_text_atomic("/proj/x.txt", "x").await.unwrap();
        client.write_text_atomic("/proj/sub/y.txt", "y").await.unwrap();
        crate::core::fs_ops::delete_path(
            &client,
            &f.state.safety,
            OWNER,
            "proj-restore-1",
            "/proj",
            true,
            true,
        )
        .await
        .unwrap();

        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        assert_eq!(listed["total"], 1);
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        let store = client.trash.as_ref().unwrap();
        let before = store.count_trash_entries_for_test().await.unwrap();
        trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        let after = store.count_trash_entries_for_test().await.unwrap();
        assert_eq!(before - after, 1);

        assert!(client.is_dir("/proj").await.unwrap());
        assert!(client.exists("/proj/x.txt").await.unwrap());
        assert!(client.exists("/proj/sub/y.txt").await.unwrap());
    }

    /// E2E-NEW-424: restore backfills an untracked node inline.
    #[tokio::test]
    async fn e2e_new_424_restore_backfills_an_untracked_node_inline() {
        let f = Fixture::new().await;
        f.seed_project("proj-backfill-2", OWNER).await;
        let client = f.state.stores.client("proj-backfill-2").await.unwrap();
        client.write_text_atomic("/.mcp_trash/1700000000000__orphan.txt", "x").await.unwrap();

        let r = trash_restore(&f.state, "proj-backfill-2", "/.mcp_trash/1700000000000__orphan.txt")
            .await
            .unwrap();
        assert_eq!(r["restored_path"], "/orphan.txt");
        assert!(client.exists("/orphan.txt").await.unwrap());
        let store = client.trash.as_ref().unwrap();
        assert_eq!(store.count_trash_entries_for_test().await.unwrap(), 0);
    }

    /// E2E-NEW-425: restoring twice fails the second time.
    #[tokio::test]
    async fn e2e_new_425_restoring_twice_fails_the_second_time() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        trash_delete(&f, "proj-restore-1", "/doc.txt").await;
        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        let e = trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    /// E2E-NEW-426: restore deletes the row, verified independently.
    #[tokio::test]
    async fn e2e_new_426_restore_deletes_the_row_verified_independently() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        trash_delete(&f, "proj-restore-1", "/z.txt").await;
        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        let store = client.trash.as_ref().unwrap();
        assert!(store.get_trash_entry_for_test(&trash_path).await.unwrap().is_some());

        trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        assert!(store.get_trash_entry_for_test(&trash_path).await.unwrap().is_none());
    }

    /// E2E-NEW-437: restore with an empty `trash_path` fails `ERR_INVALID_ARGUMENT`.
    #[tokio::test]
    async fn e2e_new_437_restore_with_an_empty_trash_path_is_invalid_argument() {
        let f = Fixture::new().await;
        f.seed_project("proj-restore-1", OWNER).await;
        let e = trash_restore(&f.state, "proj-restore-1", "").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
    }

    /// E2E-NEW-439: restore recreates a missing ancestor directory.
    #[tokio::test]
    async fn e2e_new_439_restore_recreates_a_missing_ancestor_directory() {
        let f = Fixture::with_config(|c| c.safety.allow_hard_delete = true).await;
        f.seed_project("proj-restore-1", OWNER).await;
        let client = f.state.stores.client("proj-restore-1").await.unwrap();
        client.write_text_atomic("/parent/child/f.txt", "x").await.unwrap();
        crate::core::fs_ops::delete_path(
            &client,
            &f.state.safety,
            OWNER,
            "proj-restore-1",
            "/parent/child",
            true,
            true,
        )
        .await
        .unwrap();
        // Hard-delete the now-empty `/parent` ancestor out from under the trash entry.
        client.delete_tree("/parent").await.unwrap();
        assert!(!client.exists("/parent").await.unwrap());

        let listed = trash_list(&f.state, "proj-restore-1", "", 200, 0).await.unwrap();
        let trash_path = listed["entries"][0]["trash_path"].as_str().unwrap().to_string();

        trash_restore(&f.state, "proj-restore-1", &trash_path).await.unwrap();
        assert!(client.is_dir("/parent").await.unwrap());
        assert!(client.is_dir("/parent/child").await.unwrap());
    }
}
