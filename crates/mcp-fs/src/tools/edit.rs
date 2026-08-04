//! Edit family: `fs.edit`, `fs.multi_edit`, `fs.search_replace`,
//! `fs.insert_at_line`, `fs.apply_patch` (V4A).
//!
//! Port of the C# `Tools/EditTools.cs`. The first four delegate straight to the
//! engine. The V4A parser and applier live in `core::fs_ops` so the REST plane
//! shares them; this module only unpacks arguments. Historically they sat here,
//! which forced the dataplane to reach back through the tool registry. The
//! engine only exposes the primitives it drives (read, write, delete, rename).

use crate::core::fs_ops;
use crate::errors::{Result, ToolError};
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm, volume};
use serde_json::Value;

/// The `edits` items schema, byte for byte what the C# SDK generates from
/// `List<EditSpec>` (property order included, and no `required` list).
const EDIT_ITEMS: &str = r#"{"type":"object","properties":{"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}}}"#;

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.edit", "Replace a unique string; dry_run returns the diff.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_str("old_string", "Exact text to find; must be unique unless replace_all is set.")
            .req_str("new_string", "Replacement text substituted for old_string.")
            .opt_bool(
                "replace_all",
                false,
                "Replace every occurrence instead of requiring a unique match.",
            )
            .opt_bool("dry_run", false, "Return the diff without writing changes."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::edit_unique(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("old_string")?,
                &a.str("new_string")?,
                a.bool_or("replace_all", false),
                a.bool_or("dry_run", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.multi_edit", "Apply several edits atomically (all or nothing).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_obj_array(
                "edits",
                "Ordered edits (old_string, new_string, replace_all) applied atomically.",
                EDIT_ITEMS,
            )
            .opt_bool("dry_run", false, "Return the diff without writing changes."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let edits = edit_specs(&a)?;
            fs_ops::multi_edit(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &edits,
                a.bool_or("dry_run", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.search_replace", "Replace a multi-line block (optional fuzzy match).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_str("search_block", "Multi-line block of text to locate.")
            .req_str("replace_block", "Multi-line block that replaces search_block.")
            .opt_bool("fuzzy", false, "Allow whitespace tolerant (fuzzy) matching of search_block."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::search_replace(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("search_block")?,
                &a.str("replace_block")?,
                a.bool_or("fuzzy", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.insert_at_line", "Insert content before a 1-based line number.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_int("line", "1-based line number to insert content before.")
            .req_str("content", "Text content to insert."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::insert_at_line(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int("line")?,
                &a.str("content")?,
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.apply_patch", "Apply a multi-file V4A patch within one volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("patch_text", "Multi-file V4A patch text to apply within the volume."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            fs_ops::apply_patch(&client, &ctx.state.safety, &ctx.person, &mount, &a.str("patch_text")?).await
        }),
    );
}

/// Take `edits` as raw JSON: the engine reads `old_string` / `new_string` /
/// `replace_all` per entry with the same defaults the C# `EditSpec` has, so no
/// intermediate struct is needed.
fn edit_specs(a: &crate::mcp::Args) -> Result<Vec<Value>> {
    match a.raw("edits") {
        Some(Value::Array(v)) => Ok(v.clone()),
        Some(_) => Err(ToolError::invalid_argument("argument 'edits' must be an array of objects")),
        None => Err(ToolError::invalid_argument("missing required argument 'edits'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness};

    const NAMES: &[&str] = &[
        "fs.edit",
        "fs.multi_edit",
        "fs.search_replace",
        "fs.insert_at_line",
        "fs.apply_patch",
    ];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_edit_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.edit",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "old_string":{"description":"Exact text to find; must be unique unless replace_all is set.","type":"string"},
                 "new_string":{"description":"Replacement text substituted for old_string.","type":"string"},
                 "replace_all":{"description":"Replace every occurrence instead of requiring a unique match.","type":"boolean","default":false},
                 "dry_run":{"description":"Return the diff without writing changes.","type":"boolean","default":false}},
               "required":["mount_id","path","old_string","new_string"]}"#,
        );
        assert_description(register, "fs.edit", "Replace a unique string; dry_run returns the diff.");
    }

    /// The nested `edits` items schema is the part most likely to drift.
    #[test]
    fn fs_multi_edit_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.multi_edit",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "edits":{"description":"Ordered edits (old_string, new_string, replace_all) applied atomically.","type":"array",
                          "items":{"type":"object","properties":{"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}}}},
                 "dry_run":{"description":"Return the diff without writing changes.","type":"boolean","default":false}},
               "required":["mount_id","path","edits"]}"#,
        );
    }

    #[test]
    fn fs_apply_patch_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.apply_patch",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "patch_text":{"description":"Multi-file V4A patch text to apply within the volume.","type":"string"}},
               "required":["mount_id","patch_text"]}"#,
        );
    }

    #[tokio::test]
    async fn edit_applies_and_returns_a_diff() {
        let h = harness().await;
        h.seed("/a.txt", "hello\nworld\n").await;
        let r = h
            .call(
                "fs.edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/a.txt",
                                   "old_string": "world", "new_string": "rust"}),
            )
            .await
            .unwrap();
        assert_eq!(r["applied"], true);
        assert!(r["diff"].as_str().unwrap().contains("+rust"));
    }

    #[tokio::test]
    async fn edit_without_a_prior_read_is_blocked() {
        let h = harness().await;
        h.client().await.write_text_atomic("/g.txt", "x\n").await.unwrap();
        let err = h
            .call(
                "fs.edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/g.txt",
                                   "old_string": "x", "new_string": "y"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::EDIT_WITHOUT_PRIOR_READ);
    }

    #[tokio::test]
    async fn multi_edit_is_atomic_and_counts_edits() {
        let h = harness().await;
        h.seed("/m.txt", "one\ntwo\nthree\n").await;
        let r = h
            .call(
                "fs.multi_edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/m.txt", "edits": [
                    {"old_string": "one", "new_string": "1"},
                    {"old_string": "two", "new_string": "2"}]}),
            )
            .await
            .unwrap();
        assert_eq!(r["edits"], 2);
        assert_eq!(r["applied"], true);

        // A batch whose second edit cannot resolve writes nothing.
        let err = h
            .call(
                "fs.multi_edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/m.txt", "edits": [
                    {"old_string": "1", "new_string": "uno"},
                    {"old_string": "absent", "new_string": "x"}]}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_MATCH);
        let after = h.client().await.read_text("/m.txt").await.unwrap();
        assert_eq!(after, "1\n2\nthree\n");
    }

    #[tokio::test]
    async fn multi_edit_rejects_a_non_array_edits() {
        let h = harness().await;
        h.seed("/m.txt", "one\n").await;
        let err = h
            .call(
                "fs.multi_edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/m.txt", "edits": "nope"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn search_replace_and_insert_at_line_mutate_the_file() {
        let h = harness().await;
        h.seed("/s.txt", "alpha\nbeta\n").await;
        let sr = h
            .call(
                "fs.search_replace",
                serde_json::json!({"mount_id": MOUNT, "path": "/s.txt",
                                   "search_block": "beta", "replace_block": "gamma"}),
            )
            .await
            .unwrap();
        assert_eq!(sr["applied"], true);

        let ins = h
            .call(
                "fs.insert_at_line",
                serde_json::json!({"mount_id": MOUNT, "path": "/s.txt", "line": 1, "content": "zero"}),
            )
            .await
            .unwrap();
        assert_eq!(ins["line"], 1);
        assert_eq!(h.client().await.read_text("/s.txt").await.unwrap(), "zero\nalpha\ngamma\n");
    }

    #[tokio::test]
    async fn apply_patch_adds_updates_and_deletes() {
        let h = harness().await;
        h.seed("/keep.txt", "one\ntwo\n").await;
        h.seed("/gone.txt", "bye\n").await;
        let patch = "*** Begin Patch\n\
                     *** Add File: /new.txt\n\
                     +fresh\n\
                     *** Update File: /keep.txt\n\
                     @@\n\
                     -two\n\
                     +TWO\n\
                     *** Delete File: /gone.txt\n\
                     *** End Patch\n";
        let r = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": patch}))
            .await
            .unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], serde_json::json!({"path": "/new.txt", "op": "add"}));
        assert_eq!(files[1], serde_json::json!({"path": "/keep.txt", "op": "update"}));
        assert_eq!(files[2], serde_json::json!({"path": "/gone.txt", "op": "delete"}));

        let client = h.client().await;
        assert_eq!(client.read_text("/new.txt").await.unwrap(), "fresh");
        assert_eq!(client.read_text("/keep.txt").await.unwrap(), "one\nTWO\n");
        assert!(!client.exists("/gone.txt").await.unwrap());
    }

    #[tokio::test]
    async fn apply_patch_honours_move_to() {
        let h = harness().await;
        h.seed("/old.txt", "body\n").await;
        let patch = "*** Begin Patch\n\
                     *** Update File: /old.txt\n\
                     *** Move to: /moved.txt\n\
                     @@\n\
                     -body\n\
                     +new body\n\
                     *** End Patch\n";
        let r = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": patch}))
            .await
            .unwrap();
        assert_eq!(r["files"][0]["moved_to"], "/moved.txt");
        assert_eq!(h.client().await.read_text("/moved.txt").await.unwrap(), "new body\n");
    }

    #[tokio::test]
    async fn apply_patch_rejects_a_bad_envelope_and_a_stale_hunk() {
        let h = harness().await;
        let err = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);

        h.seed("/a.txt", "one\n").await;
        let stale = "*** Begin Patch\n*** Update File: /a.txt\n@@\n-absent\n+x\n*** End Patch\n";
        let err = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": stale}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_MATCH);
    }











}
