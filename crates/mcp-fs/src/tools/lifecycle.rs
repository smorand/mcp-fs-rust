//! Lifecycle family: `fs.mkdir`, `fs.delete` (trash by default), `fs.move`,
//! `fs.copy`, `fs.list_allowed_roots`, `fs.audit_log`.
//!
//! Port of the C# `Tools/LifecycleTools.cs`. The last two never open a volume,
//! but they still take `mount_id` and still go through the membership gate.

use crate::core::fs_ops;
use crate::errors::Result;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolCtx, ToolRegistry, handler};
use crate::mcp::Args;
use crate::tools::{authorize_only, norm, volume};
use serde_json::{Value, json};

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.mkdir", "Create a directory (parents by default).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path of the directory to create.")
            .opt_bool("parents", true, "Create missing parent directories.")
            .opt_bool("exist_ok", true, "Succeed silently if the directory already exists."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::mkdir(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.bool_or("parents", true),
                a.bool_or("exist_ok", true),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.delete", "Delete a path (moves to trash by default).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path to delete.")
            .opt_bool("recursive", false, "Required to delete a non-empty directory.")
            .opt_bool("trash", true, "Move to trash instead of hard deleting."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::delete_path(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.bool_or("recursive", false),
                a.bool_or("trash", true),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.move", "Rename or relocate a path (no-clobber by default).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("source", "Absolute POSIX source path to move.")
            .req_str("destination", "Absolute POSIX destination path.")
            .opt_bool(
                "overwrite",
                false,
                "Allow overwriting an existing destination (default no-clobber).",
            ),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let src = norm(&ctx, &a, "source")?;
            let dst = norm(&ctx, &a, "destination")?;
            fs_ops::move_path(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &src,
                &dst,
                a.bool_or("overwrite", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.copy", "Copy a file or tree (no-clobber by default).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("source", "Absolute POSIX source path to copy.")
            .req_str("destination", "Absolute POSIX destination path.")
            .opt_bool(
                "overwrite",
                false,
                "Allow overwriting an existing destination (default no-clobber).",
            )
            .opt_bool("recursive", false, "Required to copy a directory tree."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let src = norm(&ctx, &a, "source")?;
            let dst = norm(&ctx, &a, "destination")?;
            fs_ops::copy_path(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &src,
                &dst,
                a.bool_or("overwrite", false),
                a.bool_or("recursive", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.list_allowed_roots", "List the volume roots the caller can access.")
            .req_str("mount_id", "Project/volume id the operation targets."),
        handler(|ctx, a| async move {
            let _mount = authorize_only(&ctx, &a).await?;
            list_allowed_roots(&ctx).await
        }),
    );

    reg.add(
        ToolSchema::new("fs.audit_log", "Recent mutations performed in this session.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .opt_nullable_num(
                "since",
                "Only return entries at or after this Unix timestamp (seconds).",
            )
            .opt_int("limit", 20, "Maximum number of recent entries to return."),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            Ok(audit_log(&ctx, &a, &mount))
        }),
    );
}

/// Every project the caller belongs to, not just `mount_id`: the tool answers
/// "where am I allowed to work", which is why it takes a mount only to authorize.
async fn list_allowed_roots(ctx: &ToolCtx) -> Result<Value> {
    let projects = ctx.state.admin.list_projects_for(&ctx.person).await?;
    let roots: Vec<Value> = projects
        .iter()
        .map(|p| json!({"mount_id": p.id, "root": "/", "owner": p.owner}))
        .collect();
    Ok(json!({"person": ctx.person, "roots": roots}))
}

/// The tail of the session audit log (kept in memory), oldest first. `since` filters by
/// timestamp, `limit` keeps the most recent N entries.
fn audit_log(ctx: &ToolCtx, a: &Args, mount: &str) -> Value {
    let mut entries = ctx.state.safety.audit(&ctx.person, mount);
    if let Some(since) = a.opt_num("since") {
        entries.retain(|e| e.timestamp >= since);
    }
    let limit = a.int_or("limit", 20);
    let skip = (entries.len() as i64 - limit).max(0) as usize;
    let recent: Vec<Value> = entries
        .iter()
        .skip(skip)
        .map(|e| json!({
            "timestamp": e.timestamp,
            "op": e.op,
            "path": e.path,
            "detail": e.detail,
        }))
        .collect();
    json!({"entries": recent})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{
        MOUNT, PERSON, assert_description, assert_family, assert_schema, harness, harness_with,
    };

    const NAMES: &[&str] = &[
        "fs.mkdir",
        "fs.delete",
        "fs.move",
        "fs.copy",
        "fs.list_allowed_roots",
        "fs.audit_log",
    ];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_delete_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.delete",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path to delete.","type":"string"},
                 "recursive":{"description":"Required to delete a non-empty directory.","type":"boolean","default":false},
                 "trash":{"description":"Move to trash instead of hard deleting.","type":"boolean","default":true}},
               "required":["mount_id","path"]}"#,
        );
        assert_description(register, "fs.delete", "Delete a path (moves to trash by default).");
    }

    /// `since` is the only nullable number in the whole surface.
    #[test]
    fn fs_audit_log_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.audit_log",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "since":{"description":"Only return entries at or after this Unix timestamp (seconds).","type":["number","null"],"default":null},
                 "limit":{"description":"Maximum number of recent entries to return.","type":"integer","default":20}},
               "required":["mount_id"]}"#,
        );
    }

    #[test]
    fn fs_copy_and_list_allowed_roots_schemas_match_the_contract() {
        assert_schema(
            register,
            "fs.copy",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "source":{"description":"Absolute POSIX source path to copy.","type":"string"},
                 "destination":{"description":"Absolute POSIX destination path.","type":"string"},
                 "overwrite":{"description":"Allow overwriting an existing destination (default no-clobber).","type":"boolean","default":false},
                 "recursive":{"description":"Required to copy a directory tree.","type":"boolean","default":false}},
               "required":["mount_id","source","destination"]}"#,
        );
        assert_schema(
            register,
            "fs.list_allowed_roots",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"}},
               "required":["mount_id"]}"#,
        );
    }

    #[tokio::test]
    async fn mkdir_creates_nested_directories() {
        let h = harness().await;
        let r = h.call("fs.mkdir", json!({"mount_id": MOUNT, "path": "/a/b/c"})).await.unwrap();
        assert_eq!(r, json!({"path": "/a/b/c", "created": true}));
        assert!(h.client().await.is_dir("/a/b").await.unwrap());
    }

    #[tokio::test]
    async fn mkdir_without_parents_and_without_exist_ok_clobbers() {
        let h = harness().await;
        h.call("fs.mkdir", json!({"mount_id": MOUNT, "path": "/d"})).await.unwrap();
        let err = h
            .call("fs.mkdir", json!({"mount_id": MOUNT, "path": "/d", "parents": false, "exist_ok": false}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn delete_moves_to_trash_by_default() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        let r = h.call("fs.delete", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(r["trashed"], true);
        let trash = r["trash_path"].as_str().unwrap();
        assert!(trash.starts_with("/.mcp_trash/"), "got {trash}");
        assert!(h.client().await.exists(trash).await.unwrap());
    }

    #[tokio::test]
    async fn a_hard_delete_needs_the_server_flag() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        let err = h
            .call("fs.delete", json!({"mount_id": MOUNT, "path": "/a.txt", "trash": false}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);

        let allowed = harness_with(|c| c.safety.allow_hard_delete = true).await;
        allowed.seed("/a.txt", "x\n").await;
        let r = allowed
            .call("fs.delete", json!({"mount_id": MOUNT, "path": "/a.txt", "trash": false}))
            .await
            .unwrap();
        assert_eq!(r["trashed"], false);
        assert_eq!(r["trash_path"], Value::Null);
    }

    #[tokio::test]
    async fn delete_a_directory_needs_recursive() {
        let h = harness().await;
        h.seed("/dir/a.txt", "x\n").await;
        let err = h.call("fs.delete", json!({"mount_id": MOUNT, "path": "/dir"})).await.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        let ok = h
            .call("fs.delete", json!({"mount_id": MOUNT, "path": "/dir", "recursive": true}))
            .await
            .unwrap();
        assert_eq!(ok["trashed"], true);
    }

    #[tokio::test]
    async fn move_and_copy_are_no_clobber_by_default() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        let copied = h
            .call("fs.copy", json!({"mount_id": MOUNT, "source": "/a.txt", "destination": "/b.txt"}))
            .await
            .unwrap();
        assert_eq!(copied, json!({"source": "/a.txt", "destination": "/b.txt"}));

        let err = h
            .call("fs.copy", json!({"mount_id": MOUNT, "source": "/a.txt", "destination": "/b.txt"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);

        let moved = h
            .call("fs.move", json!({"mount_id": MOUNT, "source": "/b.txt", "destination": "/c.txt"}))
            .await
            .unwrap();
        assert_eq!(moved["destination"], "/c.txt");
        assert!(!h.client().await.exists("/b.txt").await.unwrap());
    }

    #[tokio::test]
    async fn copy_a_tree_needs_recursive() {
        let h = harness().await;
        h.seed("/dir/a.txt", "x\n").await;
        let err = h
            .call("fs.copy", json!({"mount_id": MOUNT, "source": "/dir", "destination": "/copy"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        h.call(
            "fs.copy",
            json!({"mount_id": MOUNT, "source": "/dir", "destination": "/copy", "recursive": true}),
        )
        .await
        .unwrap();
        assert!(h.client().await.exists("/copy/a.txt").await.unwrap());
    }

    #[tokio::test]
    async fn list_allowed_roots_reports_every_project_of_the_caller() {
        let h = harness().await;
        let r = h.call("fs.list_allowed_roots", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(r["person"], PERSON);
        assert_eq!(r["roots"], json!([{"mount_id": MOUNT, "root": "/", "owner": PERSON}]));
    }

    #[tokio::test]
    async fn audit_log_reports_mutations_in_order_and_honours_limit() {
        let h = harness().await;
        h.call("fs.write", json!({"mount_id": MOUNT, "path": "/a.txt", "content": "one\n"}))
            .await
            .unwrap();
        h.call("fs.mkdir", json!({"mount_id": MOUNT, "path": "/d"})).await.unwrap();

        let all = h.call("fs.audit_log", json!({"mount_id": MOUNT})).await.unwrap();
        let entries = all["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["op"], "write");
        assert_eq!(entries[0]["detail"], "4 bytes");
        assert_eq!(entries[1]["op"], "mkdir");

        let last = h.call("fs.audit_log", json!({"mount_id": MOUNT, "limit": 1})).await.unwrap();
        assert_eq!(last["entries"].as_array().unwrap().len(), 1);
        assert_eq!(last["entries"][0]["op"], "mkdir");
    }

    #[tokio::test]
    async fn audit_log_since_filters_out_older_entries() {
        let h = harness().await;
        h.call("fs.mkdir", json!({"mount_id": MOUNT, "path": "/d"})).await.unwrap();
        let future = crate::util::now_unix() + 60.0;
        let r = h.call("fs.audit_log", json!({"mount_id": MOUNT, "since": future})).await.unwrap();
        assert_eq!(r["entries"], json!([]));
    }

    /// A read only session has nothing to report, and that is not an error.
    #[tokio::test]
    async fn audit_log_is_empty_for_a_fresh_session() {
        let h = harness().await;
        let r = h.call("fs.audit_log", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(r["entries"], json!([]));
    }
}
