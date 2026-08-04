//! Listing family: `fs.list_dir`, `fs.tree`.
//!
//! Port of the C# `Tools/ListingTools.cs`. Both default `path` to the volume root,
//! so `mount_id` is the only required argument. The C# also wraps unexpected
//! `fs.tree` exceptions into `ERR_INTERNAL_ERROR`; the engine here only returns a
//! typed `ToolError`, so there is nothing to translate.

use crate::core::fs_ops;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm_or, volume};

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.list_dir", "Flat directory listing with kinds and optional sizes.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .opt_str("path", "/", "Absolute POSIX directory to list.")
            .opt_bool("include_hidden", false, "Include dotfiles (names starting with a period).")
            .opt_str("sort_by", "name", "Sort order: name or size.")
            .opt_bool("with_sizes", false, "Include size and mtime for each entry."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let path = norm_or(&ctx, &a, "path", "/")?;
            fs_ops::list_dir(
                &client,
                &path,
                a.bool_or("include_hidden", false),
                &a.str_or("sort_by", "name"),
                a.bool_or("with_sizes", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.tree", "Recursive JSON tree to a maximum depth.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .opt_str("path", "/", "Absolute POSIX directory to walk from.")
            .opt_int("max_depth", 3, "Maximum recursion depth to descend.")
            .opt_flexible_str_array(
                "exclude_patterns",
                "Glob patterns whose matches are pruned from the tree.",
            )
            .opt_bool("with_sizes", false, "Include size for each file node."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let path = norm_or(&ctx, &a, "path", "/")?;
            fs_ops::tree(
                &client,
                &path,
                a.int_or("max_depth", 3),
                &a.str_array("exclude_patterns"),
                a.bool_or("with_sizes", false),
            )
            .await
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness};
    use serde_json::json;

    const NAMES: &[&str] = &["fs.list_dir", "fs.tree"];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_list_dir_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.list_dir",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX directory to list.","type":"string","default":"/"},
                 "include_hidden":{"description":"Include dotfiles (names starting with a period).","type":"boolean","default":false},
                 "sort_by":{"description":"Sort order: name or size.","type":"string","default":"name"},
                 "with_sizes":{"description":"Include size and mtime for each entry.","type":"boolean","default":false}},
               "required":["mount_id"]}"#,
        );
        assert_description(
            register,
            "fs.list_dir",
            "Flat directory listing with kinds and optional sizes.",
        );
    }

    #[test]
    fn fs_tree_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.tree",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX directory to walk from.","type":"string","default":"/"},
                 "max_depth":{"description":"Maximum recursion depth to descend.","type":"integer","default":3},
                 "exclude_patterns":{"description":"Glob patterns whose matches are pruned from the tree.","default":null},
                 "with_sizes":{"description":"Include size for each file node.","type":"boolean","default":false}},
               "required":["mount_id"]}"#,
        );
    }

    #[tokio::test]
    async fn list_dir_hides_dotfiles_unless_asked() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        h.seed("/.hidden", "x\n").await;
        let plain = h.call("fs.list_dir", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(plain["path"], "/");
        assert_eq!(plain["total"], 1);
        assert_eq!(plain["entries"], json!([{"name": "a.txt", "kind": "file"}]));

        let hidden = h
            .call("fs.list_dir", json!({"mount_id": MOUNT, "include_hidden": true}))
            .await
            .unwrap();
        assert_eq!(hidden["total"], 2);
    }

    #[tokio::test]
    async fn list_dir_with_sizes_adds_size_and_mtime() {
        let h = harness().await;
        h.seed("/a.txt", "hello\n").await;
        let r = h
            .call("fs.list_dir", json!({"mount_id": MOUNT, "with_sizes": true}))
            .await
            .unwrap();
        assert_eq!(r["entries"][0]["size"], 6);
        assert!(r["entries"][0]["mtime"].is_number());
    }

    #[tokio::test]
    async fn tree_nests_children_and_respects_depth() {
        let h = harness().await;
        h.seed("/src/app.py", "x\n").await;
        let deep = h.call("fs.tree", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(deep["path"], "/");
        assert_eq!(deep["tree"][0]["name"], "src");
        assert_eq!(deep["tree"][0]["children"][0]["name"], "app.py");
        assert_eq!(deep["truncated"], false);

        let flat = h.call("fs.tree", json!({"mount_id": MOUNT, "max_depth": 0})).await.unwrap();
        assert!(flat["tree"][0].get("children").is_none());
    }

    #[tokio::test]
    async fn tree_prunes_excluded_names() {
        let h = harness().await;
        h.seed("/src/app.py", "x\n").await;
        h.seed("/vendor/lib.py", "x\n").await;
        let r = h
            .call("fs.tree", json!({"mount_id": MOUNT, "exclude_patterns": ["vendor"]}))
            .await
            .unwrap();
        let names: Vec<&str> =
            r["tree"].as_array().unwrap().iter().map(|n| n["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["src"]);
    }

    #[tokio::test]
    async fn listing_a_missing_directory_is_not_found() {
        let h = harness().await;
        let err = h.call("fs.list_dir", json!({"mount_id": MOUNT, "path": "/nope"})).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::NOT_FOUND);
    }
}
